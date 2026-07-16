//! Full-screen reverse-sync "merge cockpit" (R1–R9, R12, R16).
//!
//! This is the interactive layer that replaces the old re-printing
//! `inquire::Select` picker. It presents a left file-list pane beside a live
//! side-by-side diff, lets the developer set each file's direction with
//! explicit keys (`p` push / `l` pull / `u` undecided), and returns the chosen
//! [`Decision`]s to the caller — it does NOT write any files itself. The
//! destructive apply is performed by `reverse_sync::apply_decision` after
//! [`run_cockpit`] returns, so the cockpit stays free of filesystem side
//! effects beyond reading the two versions of each file for the diff.
//!
//! Interactive-merge (the per-hunk `m` overlay) is a LATER phase; this cockpit
//! deliberately offers only push / pull / undecided.
//!
//! ## Testability
//!
//! The rendering is factored into a pure [`draw`] that takes an [`App`] and a
//! `ratatui` [`Frame`], so it can be exercised with
//! [`ratatui::backend::TestBackend`] WITHOUT the event loop. The event loop
//! ([`event_loop`]) is kept thin. Terminal setup/teardown is guarded so a panic
//! never leaves the user's terminal in raw mode / the alternate screen.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::sync::merge::{default_decision, Decision, FileState};
use crate::sync::reverse_sync::DiffStatus;
use crate::tui::diffmodel::{self, ContentKind, DiffRow, RowTag, UnifiedRow, UnifiedTag};

/// Lines of unchanged context folded around each change (KD2 / R6).
const CONTEXT: usize = 3;

/// The result of running the cockpit: either an ordered set of decisions to
/// apply (only non-[`Decision::Undecided`] files), or a cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CockpitOutcome {
    /// Apply these `(rel, decision)` pairs (undecided files already filtered).
    Apply(Vec<(PathBuf, Decision)>),
    /// The user aborted — nothing should be written.
    Cancel,
}

/// True when BOTH stdin and stdout are a TTY. The cockpit needs a real terminal
/// (R16); a piped / CI / hook invocation must refuse to launch rather than fall
/// through to writing files.
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

// ── App state ─────────────────────────────────────────────────────────────

/// The prepared diff payload for one file, computed once at load time so
/// [`draw`] performs no filesystem I/O.
enum FileDiff {
    /// Both sides decoded as UTF-8 text; rendered side-by-side or unified
    /// depending on the terminal width at draw time.
    Text { local: String, main: String },
    /// A worktree-only file (absent in main): it will be created. `content`
    /// carries the local text when it decoded as UTF-8, else `None`.
    New { content: Option<String> },
    /// Either side is binary / non-UTF-8 (R9) — whole-file push/pull only.
    Binary { note: String },
    /// Either side is over the diff cap (R8) — whole-file push/pull only.
    TooLarge { note: String },
}

/// One reconcile candidate as shown in the cockpit.
struct FileEntry {
    /// Repo-relative path (identical on both sides).
    rel: PathBuf,
    /// How this file relates to main (new / differs).
    status: DiffStatus,
    /// The current reconcile direction (starts from [`default_decision`]).
    decision: Decision,
    /// The prepared diff view.
    diff: FileDiff,
    /// Worktree mtime, formatted as an unreliable hint (KD6).
    mtime_local: String,
    /// Main mtime, formatted as an unreliable hint (KD6).
    mtime_main: String,
}

/// Which overlay (if any) is active over the main cockpit view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The normal file-list + diff view.
    Normal,
    /// The centered help overlay.
    Help,
    /// The batched apply-confirmation overlay.
    Confirm,
}

/// The whole cockpit state. Holds no terminal handles or roots — everything the
/// renderer needs is precomputed into `files`.
struct App {
    files: Vec<FileEntry>,
    focused: usize,
    diff_scroll: u16,
    mode: Mode,
}

impl App {
    /// Build the cockpit state, reading and classifying both versions of every
    /// offered file. This is the only place that touches the filesystem.
    fn new(worktree_root: &Path, main_root: &Path, offered: &[(PathBuf, DiffStatus)]) -> Result<App> {
        let mut files = Vec::with_capacity(offered.len());
        for (rel, status) in offered {
            files.push(load_entry(worktree_root, main_root, rel, *status)?);
        }
        Ok(App {
            files,
            focused: 0,
            diff_scroll: 0,
            mode: Mode::Normal,
        })
    }

    fn focus_next(&mut self) {
        if self.focused + 1 < self.files.len() {
            self.focused += 1;
            self.diff_scroll = 0;
        }
    }

    fn focus_prev(&mut self) {
        if self.focused > 0 {
            self.focused -= 1;
            self.diff_scroll = 0;
        }
    }

    fn scroll_down(&mut self, step: u16) {
        self.diff_scroll = self.diff_scroll.saturating_add(step).min(self.max_scroll());
    }

    fn scroll_up(&mut self, step: u16) {
        self.diff_scroll = self.diff_scroll.saturating_sub(step);
    }

    /// Upper bound for the diff scroll offset, from the focused file's line
    /// count (a generous bound; over-scroll just shows blank lines).
    fn max_scroll(&self) -> u16 {
        let lines = self
            .files
            .get(self.focused)
            .map(diff_line_count)
            .unwrap_or(0);
        lines.saturating_sub(1).min(u16::MAX as usize) as u16
    }

    fn set_decision(&mut self, decision: Decision) {
        if let Some(f) = self.files.get_mut(self.focused) {
            f.decision = decision;
        }
    }

    /// Pull is meaningless for a worktree-only file (main has nothing to pull),
    /// so `l` is a no-op there.
    fn set_pull(&mut self) {
        if let Some(f) = self.files.get_mut(self.focused) {
            if f.status != DiffStatus::WorktreeOnly {
                f.decision = Decision::Pull;
            }
        }
    }

    /// The decisions to hand back on apply: every file that is not undecided.
    fn decisions(&self) -> Vec<(PathBuf, Decision)> {
        self.files
            .iter()
            .filter(|f| !matches!(f.decision, Decision::Undecided))
            .map(|f| (f.rel.clone(), f.decision.clone()))
            .collect()
    }

    /// Existing targets that a decision would overwrite, paired with a
    /// human-readable direction — the batched-confirm list (R12).
    fn destructive_overwrites(&self) -> Vec<(PathBuf, &'static str)> {
        self.files
            .iter()
            .filter_map(|f| match &f.decision {
                // Push only overwrites when main already has the file.
                Decision::Push if f.status == DiffStatus::Differs => {
                    Some((f.rel.clone(), "worktree → main"))
                }
                // Pull always overwrites the (existing) worktree copy.
                Decision::Pull => Some((f.rel.clone(), "main → worktree")),
                Decision::Merge(_) => Some((f.rel.clone(), "merged → both")),
                _ => None,
            })
            .collect()
    }
}

/// Read + classify one candidate into a render-ready [`FileEntry`].
fn load_entry(
    worktree_root: &Path,
    main_root: &Path,
    rel: &Path,
    status: DiffStatus,
) -> Result<FileEntry> {
    let wt_path = worktree_root.join(rel);
    let wt_bytes =
        fs::read(&wt_path).with_context(|| format!("reading worktree file {}", wt_path.display()))?;
    let mtime_local = format_mtime(&wt_path);

    let (diff, mtime_main) = match status {
        DiffStatus::WorktreeOnly => {
            let content = match classify_content_owned(&wt_bytes) {
                ContentKind::Text(s) => Some(s),
                _ => None,
            };
            (FileDiff::New { content }, "—".to_string())
        }
        DiffStatus::Differs | DiffStatus::Identical => {
            let main_path = main_root.join(rel);
            let main_bytes = fs::read(&main_path).unwrap_or_default();
            let mtime_main = format_mtime(&main_path);
            (build_text_diff(&wt_bytes, &main_bytes), mtime_main)
        }
    };

    let decision = default_decision(file_state(status));
    Ok(FileEntry {
        rel: rel.to_path_buf(),
        status,
        decision,
        diff,
        mtime_local,
        mtime_main,
    })
}

fn file_state(status: DiffStatus) -> FileState {
    match status {
        DiffStatus::WorktreeOnly => FileState::WorktreeOnly,
        DiffStatus::Differs | DiffStatus::Identical => FileState::ExistsBoth,
    }
}

/// Thin owned wrapper around [`diffmodel::classify_content`].
fn classify_content_owned(bytes: &[u8]) -> ContentKind {
    diffmodel::classify_content(bytes)
}

/// Choose the diff view for a two-sided file (R8/R9): binary or oversized on
/// either side degrades to a whole-file notice; otherwise a text diff.
fn build_text_diff(wt_bytes: &[u8], main_bytes: &[u8]) -> FileDiff {
    let wt = classify_content_owned(wt_bytes);
    let main = classify_content_owned(main_bytes);
    if matches!(wt, ContentKind::Binary) || matches!(main, ContentKind::Binary) {
        return FileDiff::Binary {
            note: binary_note(wt_bytes, main_bytes),
        };
    }
    match (wt, main) {
        (ContentKind::TooLarge(n), _) | (_, ContentKind::TooLarge(n)) => FileDiff::TooLarge {
            note: too_large_note(n),
        },
        (ContentKind::Text(local), ContentKind::Text(main)) => FileDiff::Text { local, main },
        // Unreachable: binary handled above, sizes handled above.
        _ => FileDiff::Binary {
            note: binary_note(wt_bytes, main_bytes),
        },
    }
}

fn binary_note(wt_bytes: &[u8], main_bytes: &[u8]) -> String {
    format!(
        "binary — differs (local {} bytes / main {} bytes / hash {:016x})",
        wt_bytes.len(),
        main_bytes.len(),
        cheap_hash(wt_bytes),
    )
}

fn too_large_note(n: u64) -> String {
    format!("diff too large to render ({n} bytes) — push/pull only")
}

/// A cheap, non-cryptographic content fingerprint for the binary notice — a
/// display hint only, never a security boundary.
fn cheap_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Format a file's mtime as a coarse "N ago" hint. Marked unreliable at the
/// call site (KD6); the exact value is never load-bearing.
fn format_mtime(path: &Path) -> String {
    let modified = match fs::metadata(path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return "unknown".to_string(),
    };
    match SystemTime::now().duration_since(modified) {
        Ok(d) => {
            let secs = d.as_secs();
            if secs < 60 {
                "just now".to_string()
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86_400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86_400)
            }
        }
        Err(_) => "in the future".to_string(),
    }
}

/// A generous line-count bound for the focused file's diff (scroll clamp only).
fn diff_line_count(f: &FileEntry) -> usize {
    match &f.diff {
        FileDiff::Text { local, main } => diffmodel::unified(local, main, CONTEXT).len() + 1,
        FileDiff::New { content } => content.as_ref().map(|c| c.lines().count()).unwrap_or(0) + 2,
        FileDiff::Binary { .. } | FileDiff::TooLarge { .. } => 1,
    }
}

// ── Pure helpers (unit-tested) ────────────────────────────────────────────

/// Whether the terminal is wide enough for the two-column split (R7). Thin
/// wrapper over [`diffmodel::should_split`] so the width→layout choice has a
/// single named seam.
fn use_split(frame_width: u16) -> bool {
    diffmodel::should_split(frame_width)
}

/// Badge label + color for a decision (R3): direction is shown with an
/// unambiguous arrow + words, never position alone.
fn badge_text(decision: &Decision) -> (String, Color) {
    match decision {
        Decision::Push => ("→ push to main".to_string(), Color::Green),
        Decision::Pull => ("← pull from main".to_string(), Color::Cyan),
        Decision::Undecided => ("? undecided".to_string(), Color::Yellow),
        Decision::Merge(_) => ("⇄ merge → both".to_string(), Color::Magenta),
    }
}

// ── Rendering (pure: no terminal, no I/O) ─────────────────────────────────

/// Render the whole cockpit into `frame` from `app`. Pure enough to drive with
/// a `TestBackend` (no event loop, no filesystem).
fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let split = use_split(area.width);

    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let body = rows[0];
    let footer = rows[1];

    let panes = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(body);
    render_file_list(frame, panes[0], app);
    render_diff(frame, panes[1], app, split);
    render_footer(frame, footer);

    match app.mode {
        Mode::Help => render_help(frame, area),
        Mode::Confirm => render_confirm(frame, area, app),
        Mode::Normal => {}
    }
}

fn render_file_list(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.files.iter().map(file_list_item).collect();
    let list = List::new(items)
        .block(Block::bordered().title(Line::from(" Files ".bold())))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    let mut state = ListState::default();
    state.select(Some(app.focused.min(app.files.len().saturating_sub(1))));
    frame.render_stateful_widget(list, area, &mut state);
}

fn file_list_item(f: &FileEntry) -> ListItem<'static> {
    let (badge, color) = badge_text(&f.decision);
    let line1 = Line::from(vec![
        Span::styled(badge, Style::new().fg(color).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw(f.rel.display().to_string()),
        Span::raw("  "),
        status_tag(f.status),
    ]);
    let line2 = Line::from(Span::styled(
        format!(
            "   mtime hint (unreliable): local {} · main {}",
            f.mtime_local, f.mtime_main
        ),
        Style::new().fg(Color::DarkGray),
    ));
    ListItem::new(vec![line1, line2])
}

fn status_tag(status: DiffStatus) -> Span<'static> {
    match status {
        DiffStatus::WorktreeOnly => Span::styled("(new)", Style::new().fg(Color::Green)),
        DiffStatus::Differs => Span::styled("(differs)", Style::new().fg(Color::Yellow)),
        DiffStatus::Identical => Span::styled("(identical)", Style::new().fg(Color::DarkGray)),
    }
}

fn render_diff(frame: &mut Frame, area: Rect, app: &App, split: bool) {
    let Some(f) = app.files.get(app.focused) else {
        return;
    };
    let block = Block::bordered().title(Line::from(format!(" {} ", f.rel.display()).bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match &f.diff {
        FileDiff::New { content } => render_new(frame, inner, content.as_deref(), app.diff_scroll),
        FileDiff::Binary { note } | FileDiff::TooLarge { note } => {
            render_notice(frame, inner, note)
        }
        FileDiff::Text { local, main } => {
            if split {
                render_split(frame, inner, local, main, app.diff_scroll);
            } else {
                render_unified(frame, inner, local, main, app.diff_scroll);
            }
        }
    }
}

fn render_new(frame: &mut Frame, area: Rect, content: Option<&str>, scroll: u16) {
    let mut lines = vec![
        Line::from(Span::styled(
            "new file — will be created in main",
            Style::new().fg(Color::Green).add_modifier(Modifier::ITALIC),
        )),
        Line::from(""),
    ];
    match content {
        Some(text) => {
            for l in text.lines() {
                lines.push(Line::from(Span::styled(
                    format!("+ {l}"),
                    Style::new().fg(Color::Green),
                )));
            }
        }
        None => lines.push(Line::from(Span::styled(
            "(binary or oversized new file — content not shown)",
            Style::new().fg(Color::DarkGray),
        ))),
    }
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn render_notice(frame: &mut Frame, area: Rect, note: &str) {
    frame.render_widget(
        Paragraph::new(note.to_string())
            .style(Style::new().fg(Color::Yellow))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_unified(frame: &mut Frame, area: Rect, local: &str, main: &str, scroll: u16) {
    let lines: Vec<Line> = diffmodel::unified(local, main, CONTEXT)
        .iter()
        .map(unified_row_line)
        .collect();
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn unified_row_line(row: &UnifiedRow) -> Line<'static> {
    if let UnifiedTag::Fold(n) = row.tag {
        return fold_line(n);
    }
    let (sign, color) = match row.tag {
        UnifiedTag::Delete => ("-", Some(Color::Red)),
        UnifiedTag::Insert => ("+", Some(Color::Green)),
        UnifiedTag::Context => (" ", None),
        UnifiedTag::Fold(_) => unreachable!("handled above"),
    };
    let mut spans = vec![
        Span::styled(
            format!("{} {} ", num(row.old_no), num(row.new_no)),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(format!("{sign} "), style_of(color, false)),
    ];
    push_segments(&mut spans, &row.segs, color);
    Line::from(spans)
}

fn render_split(frame: &mut Frame, area: Rect, local: &str, main: &str, scroll: u16) {
    let vchunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let titles = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vchunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(
            "Local file".bold().underlined().fg(Color::Cyan),
        )),
        titles[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(
            "Main branch".bold().underlined().fg(Color::Cyan),
        )),
        titles[1],
    );

    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vchunks[1]);
    let rows = diffmodel::side_by_side(local, main, CONTEXT);
    let (left, right) = side_lines(&rows);
    frame.render_widget(Paragraph::new(left).scroll((scroll, 0)), cols[0]);
    frame.render_widget(Paragraph::new(right).scroll((scroll, 0)), cols[1]);
}

/// Build the aligned left/right line vectors for the side-by-side view.
fn side_lines(rows: &[DiffRow]) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut left = Vec::with_capacity(rows.len());
    let mut right = Vec::with_capacity(rows.len());
    for row in rows {
        if let RowTag::Fold(n) = row.tag {
            left.push(fold_line(n));
            right.push(fold_line(n));
            continue;
        }
        let left_color = match row.tag {
            RowTag::Delete | RowTag::Replace => Some(Color::Red),
            _ => None,
        };
        let right_color = match row.tag {
            RowTag::Insert | RowTag::Replace => Some(Color::Green),
            _ => None,
        };
        left.push(side_cell(row.left_no, &row.left, left_color));
        right.push(side_cell(row.right_no, &row.right, right_color));
    }
    (left, right)
}

fn side_cell(
    line_no: Option<usize>,
    segs: &[crate::tui::diffmodel::Seg],
    color: Option<Color>,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{} ", num(line_no)),
        Style::new().fg(Color::DarkGray),
    )];
    push_segments(&mut spans, segs, color);
    Line::from(spans)
}

/// Append styled spans for a line's segments, emphasizing changed spans (R6).
fn push_segments(
    spans: &mut Vec<Span<'static>>,
    segs: &[crate::tui::diffmodel::Seg],
    color: Option<Color>,
) {
    for seg in segs {
        spans.push(Span::styled(seg.text.clone(), style_of(color, seg.emphasized)));
    }
}

fn style_of(color: Option<Color>, emphasized: bool) -> Style {
    let mut style = Style::new();
    if let Some(c) = color {
        style = style.fg(c);
    }
    if emphasized {
        style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
    }
    style
}

fn fold_line(n: usize) -> Line<'static> {
    Line::from(Span::styled(
        format!("  ⋯ {n} unchanged lines ⋯"),
        Style::new().fg(Color::DarkGray),
    ))
}

/// A right-aligned 4-wide line number, or blanks when the row has none.
fn num(n: Option<usize>) -> String {
    match n {
        Some(n) => format!("{n:>4}"),
        None => "    ".to_string(),
    }
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let legend = "↑↓/jk move · PgUp/PgDn/Space/b scroll · p push · l pull · u undecided · Enter apply · ? help · Esc cancel";
    frame.render_widget(
        Paragraph::new(Line::from(legend)).style(Style::new().fg(Color::DarkGray)),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(64, 70, area);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from("Navigation".bold()),
        Line::from("  ↑/↓ or j/k     move between files"),
        Line::from("  PgUp / PgDn    scroll the diff"),
        Line::from("  Space / b      scroll diff down / up"),
        Line::from(""),
        Line::from("Decisions".bold()),
        Line::from(vec![
            Span::raw("  p              "),
            Span::styled("push worktree → main", Style::new().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("  l              "),
            Span::styled("pull main → worktree", Style::new().fg(Color::Cyan)),
        ]),
        Line::from("  u              mark undecided"),
        Line::from(""),
        Line::from("Apply".bold()),
        Line::from("  Enter          review & apply (confirm)"),
        Line::from("  Esc            cancel — nothing written"),
        Line::from("  ?              toggle this help"),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(Line::from(" Help ".bold()))),
        popup,
    );
}

fn render_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(70, 60, area);
    frame.render_widget(Clear, popup);

    let overwrites = app.destructive_overwrites();
    let decided = app.decisions().len();

    let mut lines = vec![Line::from("Apply changes?".bold()), Line::from("")];
    if overwrites.is_empty() {
        lines.push(Line::from(Span::styled(
            "No existing files will be overwritten.",
            Style::new().fg(Color::Green),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "These existing files will be OVERWRITTEN (a backup is taken first):",
            Style::new().fg(Color::Yellow),
        )));
        for (rel, dir) in &overwrites {
            lines.push(Line::from(vec![
                Span::raw(format!("  {}  ", rel.display())),
                Span::styled(*dir, Style::new().fg(Color::Yellow)),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!("{decided} file(s) will be written.")));
    lines.push(Line::from(""));
    lines.push(Line::from(
        "y = apply · n / Esc = back (default: No)".bold(),
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(Line::from(" Confirm apply ".bold()))),
        popup,
    );
}

/// A centered rect `percent_x` × `percent_y` of `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

// ── Event loop + terminal lifecycle ───────────────────────────────────────

/// Run the cockpit to completion, restoring the terminal on every exit path
/// (normal return, error, or panic).
pub fn run_cockpit(
    worktree_root: &Path,
    main_root: &Path,
    offered: &[(PathBuf, DiffStatus)],
) -> Result<CockpitOutcome> {
    let mut app = App::new(worktree_root, main_root, offered)?;

    install_panic_hook();
    enable_raw_mode().context("enabling terminal raw mode")?;
    io::stdout()
        .execute(EnterAlternateScreen)
        .context("entering the alternate screen")?;
    // Restores the terminal on scope exit, INCLUDING during unwinding.
    let _guard = TerminalGuard;

    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).context("creating the terminal")?;
    event_loop(&mut terminal, &mut app)
}

/// The thin event loop: draw, read one key, mutate state, repeat.
fn event_loop<B>(terminal: &mut Terminal<B>, app: &mut App) -> Result<CockpitOutcome>
where
    B: ratatui::backend::Backend,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    loop {
        terminal
            .draw(|frame| draw(frame, app))
            .context("drawing the cockpit")?;

        let page = terminal
            .size()
            .map(|s| s.height.saturating_sub(4).max(1))
            .unwrap_or(10);

        let Event::Key(key) = event::read().context("reading a terminal event")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match app.mode {
            Mode::Help => {
                if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
                    app.mode = Mode::Normal;
                }
            }
            Mode::Confirm => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    return Ok(CockpitOutcome::Apply(app.decisions()));
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.mode = Mode::Normal,
                _ => {}
            },
            Mode::Normal => match key.code {
                KeyCode::Esc => return Ok(CockpitOutcome::Cancel),
                KeyCode::Up | KeyCode::Char('k') => app.focus_prev(),
                KeyCode::Down | KeyCode::Char('j') => app.focus_next(),
                KeyCode::PageDown | KeyCode::Char(' ') => app.scroll_down(page),
                KeyCode::PageUp | KeyCode::Char('b') => app.scroll_up(page),
                KeyCode::Char('p') => app.set_decision(Decision::Push),
                KeyCode::Char('l') => app.set_pull(),
                KeyCode::Char('u') => app.set_decision(Decision::Undecided),
                KeyCode::Char('?') => app.mode = Mode::Help,
                KeyCode::Enter => app.mode = Mode::Confirm,
                _ => {}
            },
        }
    }
}

/// Restore the terminal: leave raw mode and the alternate screen. Best-effort —
/// errors are swallowed because this runs on teardown / panic paths.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = io::stdout().execute(LeaveAlternateScreen);
}

/// RAII guard that restores the terminal when dropped (normal, error, or
/// unwinding-panic exit).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Install a panic hook that restores the terminal BEFORE the default hook
/// prints the panic message, so a panic never leaves the user wedged in raw
/// mode / the alternate screen with a garbled message.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

#[cfg(test)]
mod tests;
