//! Full-screen reverse-sync "merge cockpit" (R1–R9, R12, R16).
//!
//! This is the interactive layer that replaces the old re-printing
//! `inquire::Select` picker. It presents a left file-list pane beside a live
//! side-by-side diff, lets the developer set each file's direction with
//! explicit keys (`p` push / `l` pull / `m` merge / `d` delete / `u`
//! undecided), and returns
//! the chosen [`Decision`]s to the caller — it does NOT write any files itself.
//! The destructive apply is performed by `reverse_sync::apply_decision` after
//! [`run_cockpit`] returns, so the cockpit stays free of filesystem side
//! effects beyond reading the two versions of each file for the diff.
//!
//! Pressing `m` on a differing text file opens the per-hunk merge overlay
//! ([`Mode::Merge`]): the user walks the file's hunks, cycling each between
//! keep-local / keep-main / keep-both, previews the assembled result live (via
//! [`merge_segments`] + [`assemble`]), and accepts it as a [`Decision::Merge`]
//! that the apply seam writes to BOTH sides. `m` is a no-op for binary /
//! oversized / worktree-only files, where interactive merge is unavailable (R9).
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

use crate::sync::merge::{
    assemble, default_decision, diff_count, merge_segments, Decision, FileState, MergeChoice,
    MergeSegment,
};
use crate::sync::reverse_sync::DiffStatus;
use crate::tui::diffmodel::{self, ContentKind, DiffRow, RowTag, UnifiedTag};

/// Lines of unchanged context folded around each change (KD2 / R6).
const CONTEXT: usize = 3;

/// Horizontal scroll step for `←`/`→` over long diff lines, in columns.
const H_SCROLL_STEP: u16 = 8;

/// Unified-view gutter width: two 4-wide line numbers + separators
/// (`"%4d %4d "`) plus the 2-column `± ` sign — fixed while the content
/// scrolls horizontally.
const UNIFIED_GUTTER: u16 = 12;

/// Diff-pane notice for a file whose sides are equal AFTER EOL normalization:
/// the bytes differ, but only by line endings / trailing newline.
const EOL_ONLY_NOTE: &str = "no content differences — the sides differ only by line endings \
(CRLF/LF) or a trailing newline. p/l overwrite the raw bytes; m (then Enter) converges both \
sides to the normalized text.";

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
    /// Both sides decoded as UTF-8 text, EOL-NORMALIZED at load time
    /// ([`diffmodel::normalize_eol`]: CRLF → LF, trailing newline ensured) so
    /// diff hunks and the merge overlay reflect content differences only;
    /// rendered side-by-side or unified depending on the terminal width at
    /// draw time. Equal normalized sides (an EOL-only difference — the raw
    /// bytes still differ, or the file would not be offered) render as an
    /// explanatory notice instead of an empty diff.
    Text { local: String, main: String },
    /// A worktree-only file (absent in main): it will be created. `content`
    /// carries the local text when it decoded as UTF-8, else `None`.
    New { content: Option<String> },
    /// Either side is binary / non-UTF-8 (R9) — whole-file push/pull only.
    Binary { note: String },
    /// Either side is over the diff cap (R8) — whole-file push/pull only.
    TooLarge { note: String },
    /// Main's copy could not be read (permissions / I/O — NOT missing). The
    /// error is surfaced verbatim; a diff/merge is NEVER built from a fabricated
    /// empty buffer, so interactive merge is unavailable for this file.
    Unreadable { note: String },
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
    /// The per-hunk interactive-merge overlay (state in [`App::merge`]).
    Merge,
}

/// Live state of the interactive per-hunk merge overlay (R10). Held apart from
/// [`Mode`] (which stays `Copy`) so the segment/choice buffers can be owned. The
/// `choices` vector is aligned to the `Diff` segments in order — one entry per
/// [`diff_count`] — and `hunk` indexes into it.
struct MergeOverlay {
    /// Index into [`App::files`] of the file being merged.
    file_idx: usize,
    /// The ordered `Equal`/`Diff` segments from [`merge_segments`].
    segments: Vec<MergeSegment>,
    /// One [`MergeChoice`] per `Diff` segment, in order (default `Local`).
    choices: Vec<MergeChoice>,
    /// The focused hunk — an index into `choices` / the `Diff` segments.
    hunk: usize,
    /// Vertical scroll offset for the assembled-preview pane.
    preview_scroll: u16,
}

impl MergeOverlay {
    /// Build the overlay for `file_idx`, computing hunks from `local` vs `main`
    /// and defaulting every choice to keep-local (nothing changes until the
    /// user cycles a hunk).
    fn build(file_idx: usize, local: &str, main: &str) -> MergeOverlay {
        let segments = merge_segments(local, main);
        let n = diff_count(&segments);
        MergeOverlay {
            file_idx,
            segments,
            choices: vec![MergeChoice::Local; n],
            hunk: 0,
            preview_scroll: 0,
        }
    }

    /// Number of decision points (differing hunks) in this merge.
    fn hunk_count(&self) -> usize {
        self.choices.len()
    }

    fn next_hunk(&mut self) {
        if self.hunk + 1 < self.hunk_count() {
            self.hunk += 1;
        }
    }

    fn prev_hunk(&mut self) {
        if self.hunk > 0 {
            self.hunk -= 1;
        }
    }

    /// Cycle the focused hunk's choice — forward keep-local → keep-main →
    /// keep-both → keep-local, or backward when `forward` is false.
    fn cycle_choice(&mut self, forward: bool) {
        if let Some(c) = self.choices.get_mut(self.hunk) {
            *c = if forward { next_choice(*c) } else { prev_choice(*c) };
        }
        // A different choice can shrink the assembled preview — keep the
        // scroll offset inside the new preview.
        self.preview_scroll = self.preview_scroll.min(self.max_preview_scroll());
    }

    /// Upper bound for the preview scroll offset, from the CURRENT assembled
    /// preview's line count (over-scroll would just show blank lines).
    fn max_preview_scroll(&self) -> u16 {
        self.preview()
            .lines()
            .count()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    fn scroll_preview_down(&mut self, step: u16) {
        self.preview_scroll = self
            .preview_scroll
            .saturating_add(step)
            .min(self.max_preview_scroll());
    }

    fn scroll_preview_up(&mut self, step: u16) {
        self.preview_scroll = self.preview_scroll.saturating_sub(step);
    }

    /// The (local, main) candidate texts of the `n`-th `Diff` segment, if any.
    fn diff_sides(&self, n: usize) -> Option<(&str, &str)> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                MergeSegment::Diff { local, main } => Some((local.as_str(), main.as_str())),
                MergeSegment::Equal(_) => None,
            })
            .nth(n)
    }

    /// The focused hunk's local-side text (empty when there is no hunk).
    fn focused_local(&self) -> &str {
        self.diff_sides(self.hunk).map(|(l, _)| l).unwrap_or("")
    }

    /// The focused hunk's main-side text (empty when there is no hunk).
    fn focused_main(&self) -> &str {
        self.diff_sides(self.hunk).map(|(_, m)| m).unwrap_or("")
    }

    /// The live assembled preview for the current choices (pure — testable
    /// without the event loop).
    fn preview(&self) -> String {
        merge_preview(&self.segments, &self.choices)
    }
}

/// The whole cockpit state. Holds no terminal handles or roots — everything the
/// renderer needs is precomputed into `files`.
struct App {
    files: Vec<FileEntry>,
    focused: usize,
    diff_scroll: u16,
    /// Horizontal offset of the diff CONTENT (the line-number gutters stay
    /// fixed) — `←`/`→`, reset when the focus moves to another file.
    diff_hscroll: u16,
    mode: Mode,
    /// The active merge overlay's state (`Some` iff `mode == Mode::Merge`).
    merge: Option<MergeOverlay>,
    /// A transient footer notice (e.g. "merge unavailable"), cleared on the
    /// next keypress in [`Mode::Normal`].
    notice: Option<String>,
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
            diff_hscroll: 0,
            mode: Mode::Normal,
            merge: None,
            notice: None,
        })
    }

    fn focus_next(&mut self) {
        if self.focused + 1 < self.files.len() {
            self.focused += 1;
            self.diff_scroll = 0;
            self.diff_hscroll = 0;
        }
    }

    fn focus_prev(&mut self) {
        if self.focused > 0 {
            self.focused -= 1;
            self.diff_scroll = 0;
            self.diff_hscroll = 0;
        }
    }

    fn scroll_down(&mut self, step: u16) {
        self.diff_scroll = self.diff_scroll.saturating_add(step).min(self.max_scroll());
    }

    fn scroll_up(&mut self, step: u16) {
        self.diff_scroll = self.diff_scroll.saturating_sub(step);
    }

    fn scroll_right(&mut self, step: u16) {
        self.diff_hscroll = self
            .diff_hscroll
            .saturating_add(step)
            .min(self.max_hscroll());
    }

    fn scroll_left(&mut self, step: u16) {
        self.diff_hscroll = self.diff_hscroll.saturating_sub(step);
    }

    /// Upper bound for the horizontal scroll offset, from the focused file's
    /// longest content line (over-scroll would just show blank columns).
    fn max_hscroll(&self) -> u16 {
        self.files
            .get(self.focused)
            .map(|f| max_content_width(&f.diff))
            .unwrap_or(0)
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
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

    /// Pull requires a readable, existing main copy to read from. A worktree-only
    /// file (main absent) or one whose main copy is [`FileDiff::Unreadable`]
    /// (present but permission/I/O error) has none, so `l` is a no-op for both —
    /// mirroring the diff pane, which already shows pull disabled. Setting Pull
    /// there would only fail at apply time.
    fn set_pull(&mut self) {
        if let Some(f) = self.files.get_mut(self.focused) {
            let main_readable = f.status != DiffStatus::WorktreeOnly
                && !matches!(f.diff, FileDiff::Unreadable { .. });
            if main_readable {
                f.decision = Decision::Pull;
            }
        }
    }

    /// Attempt to open the interactive merge overlay for the focused file.
    /// Only a file that DIFFERS and is text on both sides can be merged (R9,
    /// R10); for binary / oversized / worktree-only files this is a no-op that
    /// sets a transient "merge unavailable" notice instead of entering the mode.
    fn try_open_merge(&mut self) {
        let overlay = match self.files.get(self.focused) {
            Some(f) => match (&f.diff, f.status) {
                (FileDiff::Text { local, main }, DiffStatus::Differs) => {
                    Some(MergeOverlay::build(self.focused, local, main))
                }
                _ => None,
            },
            None => None,
        };
        match overlay {
            Some(overlay) => {
                self.merge = Some(overlay);
                self.mode = Mode::Merge;
            }
            None => {
                self.notice =
                    Some("merge unavailable here — only differing text files can be merged".into());
            }
        }
    }

    /// Accept the active merge: set the focused file's decision to
    /// [`Decision::Merge`] carrying the assembled bytes, then return to the
    /// normal view. A cancel (`Esc`) instead drops the overlay untouched.
    fn accept_merge(&mut self) {
        if let Some(overlay) = self.merge.take() {
            let assembled = merge_preview(&overlay.segments, &overlay.choices);
            if let Some(f) = self.files.get_mut(overlay.file_idx) {
                f.decision = Decision::Merge(assembled);
            }
        }
        self.mode = Mode::Normal;
    }

    /// The decisions to hand back on apply: every file that is not undecided.
    fn decisions(&self) -> Vec<(PathBuf, Decision)> {
        self.files
            .iter()
            .filter(|f| !matches!(f.decision, Decision::Undecided))
            .map(|f| (f.rel.clone(), f.decision.clone()))
            .collect()
    }

    /// Existing files a decision would overwrite OR delete, paired with a
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
                // Delete removes every existing side.
                Decision::Delete if f.status == DiffStatus::WorktreeOnly => {
                    Some((f.rel.clone(), "delete (worktree copy)"))
                }
                Decision::Delete => Some((f.rel.clone(), "delete (worktree + main)")),
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
    let mtime_local = format_mtime(&wt_path);

    let (diff, mtime_main) = match status {
        DiffStatus::WorktreeOnly => (build_new(&wt_path)?, "—".to_string()),
        DiffStatus::Differs | DiffStatus::Identical => {
            let main_path = main_root.join(rel);
            let mtime_main = format_mtime(&main_path);
            (build_two_sided(&wt_path, &main_path)?, mtime_main)
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

/// Build the [`FileDiff::New`] view for a worktree-only file, consulting the
/// file's size via metadata BEFORE any full read (R8): an oversized new file is
/// shown as "content not shown" without paying for the read.
fn build_new(wt_path: &Path) -> Result<FileDiff> {
    let wt_len = fs::metadata(wt_path)
        .with_context(|| format!("reading worktree file {}", wt_path.display()))?
        .len();
    let content = if wt_len > diffmodel::MAX_DIFF_BYTES {
        None
    } else {
        let wt_bytes = fs::read(wt_path)
            .with_context(|| format!("reading worktree file {}", wt_path.display()))?;
        match diffmodel::classify_content(&wt_bytes) {
            ContentKind::Text(s) => Some(s),
            _ => None,
        }
    };
    Ok(FileDiff::New { content })
}

/// Build the two-sided diff view for a file present on both sides.
///
/// Consults both sides' sizes via `fs::metadata` FIRST (R8): if either exceeds
/// [`diffmodel::MAX_DIFF_BYTES`], a [`FileDiff::TooLarge`] is built from the
/// metadata length WITHOUT reading either file in full. A main-side read error
/// that is NOT "missing" (permissions / I/O) surfaces as [`FileDiff::Unreadable`]
/// — main's bytes are never fabricated as empty, so no diff/merge is driven from
/// fabricated content. The worktree side propagates its error (it is the secret
/// being pushed).
fn build_two_sided(wt_path: &Path, main_path: &Path) -> Result<FileDiff> {
    let wt_len = fs::metadata(wt_path)
        .with_context(|| format!("reading worktree file {}", wt_path.display()))?
        .len();
    let main_len = match fs::metadata(main_path) {
        Ok(m) => m.len(),
        Err(e) => return Ok(main_unreadable(&e)),
    };
    if wt_len > diffmodel::MAX_DIFF_BYTES || main_len > diffmodel::MAX_DIFF_BYTES {
        return Ok(FileDiff::TooLarge {
            note: too_large_note(wt_len.max(main_len)),
        });
    }

    let wt_bytes = fs::read(wt_path)
        .with_context(|| format!("reading worktree file {}", wt_path.display()))?;
    let main_bytes = match fs::read(main_path) {
        Ok(b) => b,
        Err(e) => return Ok(main_unreadable(&e)),
    };
    Ok(build_text_diff(&wt_bytes, &main_bytes))
}

/// The [`FileDiff::Unreadable`] notice for a main-side read failure.
fn main_unreadable(err: &io::Error) -> FileDiff {
    FileDiff::Unreadable {
        note: format!("main unreadable: {err} — push only (pull/merge disabled)"),
    }
}

fn file_state(status: DiffStatus) -> FileState {
    match status {
        DiffStatus::WorktreeOnly => FileState::WorktreeOnly,
        DiffStatus::Differs | DiffStatus::Identical => FileState::ExistsBoth,
    }
}

/// Choose the diff view for a two-sided file (R8/R9): binary or oversized on
/// either side degrades to a whole-file notice; otherwise a text diff.
fn build_text_diff(wt_bytes: &[u8], main_bytes: &[u8]) -> FileDiff {
    let wt = diffmodel::classify_content(wt_bytes);
    let main = diffmodel::classify_content(main_bytes);
    if matches!(wt, ContentKind::Binary) || matches!(main, ContentKind::Binary) {
        return FileDiff::Binary {
            note: binary_note(wt_bytes, main_bytes),
        };
    }
    match (wt, main) {
        (ContentKind::TooLarge(n), _) | (_, ContentKind::TooLarge(n)) => FileDiff::TooLarge {
            note: too_large_note(n),
        },
        // Normalize EOLs once at load: every downstream consumer (diff pane,
        // merge overlay, line counts) sees content-only differences. Raw bytes
        // on disk are untouched — push/pull still copy them verbatim.
        (ContentKind::Text(local), ContentKind::Text(main)) => FileDiff::Text {
            local: diffmodel::normalize_eol(&local),
            main: diffmodel::normalize_eol(&main),
        },
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
        FileDiff::Binary { .. } | FileDiff::TooLarge { .. } | FileDiff::Unreadable { .. } => 1,
    }
}

// ── Pure helpers (unit-tested) ────────────────────────────────────────────

/// Whether the diff PANE (not the whole frame) is wide enough for the two-column
/// split (R7). The argument is the diff pane's inner width — the width that
/// actually has to hold two columns — so the choice matches what the user sees.
/// Thin wrapper over [`diffmodel::should_split`] so the width→layout choice has
/// a single named seam.
fn use_split(diff_pane_inner_width: u16) -> bool {
    diffmodel::should_split(diff_pane_inner_width)
}

/// Badge label + color for a decision (R3): direction is shown with an
/// unambiguous arrow + words, never position alone. `status` disambiguates a
/// delete's sides — a worktree-only file has no main copy to remove, and the
/// badge must say exactly what the batched confirm will say.
fn badge_text(decision: &Decision, status: DiffStatus) -> (String, Color) {
    match decision {
        Decision::Push => ("→ push to main".to_string(), Color::Green),
        Decision::Pull => ("← pull from main".to_string(), Color::Cyan),
        Decision::Undecided => ("? undecided".to_string(), Color::Yellow),
        Decision::Merge(_) => ("⇄ merge (assembled)".to_string(), Color::Magenta),
        Decision::Delete if status == DiffStatus::WorktreeOnly => {
            ("✗ delete (worktree copy)".to_string(), Color::Red)
        }
        Decision::Delete => ("✗ delete (worktree + main)".to_string(), Color::Red),
    }
}

/// The assembled merge preview for `segments` under `choices` — a thin, pure
/// delegate to [`assemble`] so the overlay's live preview is testable without
/// the event loop.
fn merge_preview(segments: &[MergeSegment], choices: &[MergeChoice]) -> String {
    assemble(segments, choices)
}

/// Cycle a merge choice forward: keep-local → keep-main → keep-both → keep-local.
fn next_choice(c: MergeChoice) -> MergeChoice {
    match c {
        MergeChoice::Local => MergeChoice::Main,
        MergeChoice::Main => MergeChoice::Both,
        MergeChoice::Both => MergeChoice::Local,
    }
}

/// Cycle a merge choice backward (the inverse of [`next_choice`]).
fn prev_choice(c: MergeChoice) -> MergeChoice {
    match c {
        MergeChoice::Local => MergeChoice::Both,
        MergeChoice::Main => MergeChoice::Local,
        MergeChoice::Both => MergeChoice::Main,
    }
}

/// Human label for a merge choice, shown in the overlay header + hunk list.
fn choice_label(c: MergeChoice) -> &'static str {
    match c {
        MergeChoice::Local => "keep-local",
        MergeChoice::Main => "keep-main",
        MergeChoice::Both => "keep-both (local, then main)",
    }
}

// ── Rendering (pure: no terminal, no I/O) ─────────────────────────────────

/// Render the whole cockpit into `frame` from `app`. Pure enough to drive with
/// a `TestBackend` (no event loop, no filesystem).
fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let body = rows[0];
    let footer = rows[1];

    let panes = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(body);
    // The split decision is based on the DIFF PANE's inner width, not the frame:
    // only ~62% of the frame is the diff pane, and it is halved again per column,
    // so a frame-width test wildly overestimates each column's real width.
    // `render_diff` wraps the pane in a one-cell border, so the inner width is
    // the pane width minus 2.
    let diff_pane_inner_width = panes[1].width.saturating_sub(2);
    let split = use_split(diff_pane_inner_width);
    render_file_list(frame, panes[0], app);
    render_diff(frame, panes[1], app, split);
    render_footer(frame, footer, app.notice.as_deref());

    match app.mode {
        Mode::Help => render_help(frame, area),
        Mode::Confirm => render_confirm(frame, area, app),
        Mode::Merge => render_merge(frame, area, app),
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
    let (badge, color) = badge_text(&f.decision, f.status);
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

    // Long-line awareness: when any content line extends past the visible
    // width (a trailing comment, a long value), the change may live in the
    // clipped tail — the title must say so, and `←`/`→` scroll to it. A pane
    // that silently clips the only differing column hides the change entirely.
    let inner_width = area.width.saturating_sub(2);
    let visible = visible_content_width(&f.diff, split, inner_width);
    let scrollable = matches!(&f.diff, FileDiff::Text { local, main } if local != main)
        || matches!(&f.diff, FileDiff::New { content: Some(_) });
    let clipped = scrollable
        && max_content_width(&f.diff) > visible as usize + app.diff_hscroll as usize;
    let title = if app.diff_hscroll > 0 {
        format!(" {} · → col {} ", f.rel.display(), app.diff_hscroll)
    } else if clipped {
        format!(" {} · lines continue → (←/→ scrolls) ", f.rel.display())
    } else {
        format!(" {} ", f.rel.display())
    };
    let block = Block::bordered().title(Line::from(title.bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match &f.diff {
        FileDiff::New { content } => render_new(
            frame,
            inner,
            content.as_deref(),
            app.diff_scroll,
            app.diff_hscroll,
        ),
        FileDiff::Binary { note }
        | FileDiff::TooLarge { note }
        | FileDiff::Unreadable { note } => render_notice(frame, inner, note),
        // Equal after EOL normalization: the raw bytes differ only by line
        // endings / trailing newline — say so instead of drawing an empty diff.
        FileDiff::Text { local, main } if local == main => {
            render_notice(frame, inner, EOL_ONLY_NOTE)
        }
        FileDiff::Text { local, main } => {
            if split {
                render_split(frame, inner, local, main, app.diff_scroll, app.diff_hscroll);
            } else {
                render_unified(frame, inner, local, main, app.diff_scroll, app.diff_hscroll);
            }
        }
    }
}

/// The widest content line (in chars) of the focused file's diff view — the
/// clamp for horizontal scrolling and the "lines continue →" hint. Char count
/// approximates display width; it is only a bound/hint, never a layout.
fn max_content_width(diff: &FileDiff) -> usize {
    let longest = |text: &str| text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    match diff {
        FileDiff::Text { local, main } => longest(local).max(longest(main)),
        FileDiff::New { content } => content.as_deref().map(longest).unwrap_or(0),
        _ => 0,
    }
}

/// How many columns of CONTENT (after the fixed gutter) the current view
/// shows for `diff` at the pane's `inner_width`.
fn visible_content_width(diff: &FileDiff, split: bool, inner_width: u16) -> u16 {
    match diff {
        FileDiff::Text { .. } if split => {
            (inner_width / 2).saturating_sub(diffmodel::SPLIT_GUTTER)
        }
        FileDiff::Text { .. } => inner_width.saturating_sub(UNIFIED_GUTTER),
        // The `+ ` prefix scrolls with the content in the new-file view.
        _ => inner_width,
    }
}

fn render_new(frame: &mut Frame, area: Rect, content: Option<&str>, scroll: u16, hscroll: u16) {
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
    frame.render_widget(Paragraph::new(lines).scroll((scroll, hscroll)), area);
}

fn render_notice(frame: &mut Frame, area: Rect, note: &str) {
    frame.render_widget(
        Paragraph::new(note.to_string())
            .style(Style::new().fg(Color::Yellow))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_unified(frame: &mut Frame, area: Rect, local: &str, main: &str, scroll: u16, hscroll: u16) {
    let rows = diffmodel::unified(local, main, CONTEXT);
    let mut nums: Vec<Line> = Vec::with_capacity(rows.len());
    let mut content: Vec<Line> = Vec::with_capacity(rows.len());
    for row in &rows {
        if let UnifiedTag::Fold(n) = row.tag {
            nums.push(Line::from(""));
            content.push(fold_line(n));
            continue;
        }
        let (sign, color) = match row.tag {
            UnifiedTag::Delete => ("-", Some(Color::Red)),
            UnifiedTag::Insert => ("+", Some(Color::Green)),
            UnifiedTag::Context => (" ", None),
            UnifiedTag::Fold(_) => unreachable!("handled above"),
        };
        nums.push(Line::from(vec![
            Span::styled(
                format!("{} {} ", num(row.old_no), num(row.new_no)),
                Style::new().fg(Color::DarkGray),
            ),
            Span::styled(format!("{sign} "), style_of(color, false)),
        ]));
        let mut spans = Vec::new();
        push_segments(&mut spans, &row.segs, color);
        content.push(Line::from(spans));
    }
    render_gutter_and_content(frame, area, UNIFIED_GUTTER, nums, content, scroll, hscroll);
}

fn render_split(
    frame: &mut Frame,
    area: Rect,
    local: &str,
    main: &str,
    scroll: u16,
    hscroll: u16,
) {
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
    let (left, right) = side_columns(&rows);
    for (area, side) in [(cols[0], left), (cols[1], right)] {
        render_gutter_and_content(
            frame,
            area,
            diffmodel::SPLIT_GUTTER,
            side.nums,
            side.content,
            scroll,
            hscroll,
        );
    }
}

/// Render one diff column as a FIXED line-number gutter beside horizontally
/// scrollable content — `←`/`→` shift the content while the numbers (and the
/// unified `±` signs) stay put, so orientation survives the scroll.
fn render_gutter_and_content(
    frame: &mut Frame,
    area: Rect,
    gutter: u16,
    nums: Vec<Line<'static>>,
    content: Vec<Line<'static>>,
    scroll: u16,
    hscroll: u16,
) {
    let parts = Layout::horizontal([Constraint::Length(gutter), Constraint::Min(0)]).split(area);
    frame.render_widget(Paragraph::new(nums).scroll((scroll, 0)), parts[0]);
    frame.render_widget(Paragraph::new(content).scroll((scroll, hscroll)), parts[1]);
}

/// One side of the side-by-side view, gutter and content held apart so the
/// content can scroll horizontally under a fixed gutter.
struct SideColumns {
    nums: Vec<Line<'static>>,
    content: Vec<Line<'static>>,
}

/// Build the aligned left/right column pairs for the side-by-side view.
fn side_columns(rows: &[DiffRow]) -> (SideColumns, SideColumns) {
    let mut left = SideColumns {
        nums: Vec::with_capacity(rows.len()),
        content: Vec::with_capacity(rows.len()),
    };
    let mut right = SideColumns {
        nums: Vec::with_capacity(rows.len()),
        content: Vec::with_capacity(rows.len()),
    };
    for row in rows {
        if let RowTag::Fold(n) = row.tag {
            for side in [&mut left, &mut right] {
                side.nums.push(Line::from(""));
                side.content.push(fold_line(n));
            }
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
        push_side_cell(&mut left, row.left_no, &row.left, left_color);
        push_side_cell(&mut right, row.right_no, &row.right, right_color);
    }
    (left, right)
}

fn push_side_cell(
    side: &mut SideColumns,
    line_no: Option<usize>,
    segs: &[crate::tui::diffmodel::Seg],
    color: Option<Color>,
) {
    side.nums.push(Line::from(Span::styled(
        format!("{} ", num(line_no)),
        Style::new().fg(Color::DarkGray),
    )));
    let mut spans = Vec::new();
    push_segments(&mut spans, segs, color);
    side.content.push(Line::from(spans));
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

/// The persistent key legend (R5).
const FOOTER_LEGEND: &str = "↑↓/jk move · PgUp/PgDn/Space/b scroll · p push · l pull · m merge · d delete · u undecided · Enter apply · ? help · Esc cancel";

/// Render the footer: the transient `notice` (bold yellow) when present, else
/// the persistent key legend.
fn render_footer(frame: &mut Frame, area: Rect, notice: Option<&str>) {
    let para = match notice {
        Some(n) => Paragraph::new(Line::from(n.to_string()))
            .style(Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        None => Paragraph::new(Line::from(FOOTER_LEGEND)).style(Style::new().fg(Color::DarkGray)),
    };
    frame.render_widget(para, area);
}

fn render_help(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("Navigation".bold()),
        Line::from("  ↑/↓ or j/k         move between files"),
        Line::from("  PgUp/PgDn/Space/b  scroll the diff · ←/→ long lines"),
        Line::from(""),
        Line::from("Decisions".bold()),
        Line::from(vec![
            Span::raw("  p                  "),
            Span::styled("push worktree → main", Style::new().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("  l                  "),
            Span::styled("pull main → worktree", Style::new().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("  m                  "),
            Span::styled(
                "interactive merge (differing text files)",
                Style::new().fg(Color::Magenta),
            ),
        ]),
        Line::from(vec![
            Span::raw("  d                  "),
            Span::styled(
                "delete from both sides (backed up first)",
                Style::new().fg(Color::Red),
            ),
        ]),
        Line::from("  u                  mark undecided"),
        Line::from(""),
        Line::from("Merge overlay".bold()),
        Line::from("  ↑/↓ or j/k         move between hunks"),
        Line::from("  ←/→ or h/l         cycle keep-local / keep-main / keep-both"),
        Line::from("  PgUp/PgDn/Space/b  scroll the assembled preview"),
        Line::from("  Enter / Esc        accept (written to BOTH sides) / cancel"),
        Line::from(""),
        Line::from("Apply & safety".bold()),
        Line::from("  Enter              review & apply (one batched confirm, default No)"),
        Line::from("  Esc                cancel — nothing written · ? toggles this help"),
        Line::from(Span::styled(
            "  Backups first: .superset/backups/<timestamp>/ (10 newest kept)",
            Style::new().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Diffs/merges are EOL-normalized (CRLF→LF); push/pull copy raw bytes",
            Style::new().fg(Color::DarkGray),
        )),
    ];
    // Size the popup to its content, clamped to the frame — a fixed
    // percentage of the frame silently clipped the tail of the help (the
    // safety facts!) on common terminal sizes like 80×24. 22 content lines
    // + 2 border rows fit a 24-row terminal exactly.
    let w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 2;
    let h = lines.len() as u16 + 2;
    let popup = centered_rect_abs(w, h, area);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(Line::from(" Help ".bold()))),
        popup,
    );
}

/// A centered rect of absolute `width` × `height`, each clamped to `area`.
fn centered_rect_abs(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect::new(x, y, w, h)
}

fn render_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(70, 60, area);
    frame.render_widget(Clear, popup);

    let overwrites = app.destructive_overwrites();
    let decided = app.decisions().len();

    let mut lines = vec![Line::from("Apply changes?".bold()), Line::from("")];
    if overwrites.is_empty() {
        lines.push(Line::from(Span::styled(
            "No existing files will be overwritten or deleted.",
            Style::new().fg(Color::Green),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "These existing files will be OVERWRITTEN or DELETED (a backup is taken first):",
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

/// Render the per-hunk interactive-merge overlay (R10): the focused hunk's two
/// sides (clearly labeled), the list of every hunk's current choice, and a live
/// assembled preview built from [`merge_preview`].
fn render_merge(frame: &mut Frame, area: Rect, app: &App) {
    let Some(m) = app.merge.as_ref() else {
        return;
    };
    let rel = app
        .files
        .get(m.file_idx)
        .map(|f| f.rel.display().to_string())
        .unwrap_or_default();

    let popup = centered_rect(88, 90, area);
    frame.render_widget(Clear, popup);
    let block = Block::bordered().title(Line::from(format!(" Merge {rel} ").bold()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::vertical([
        Constraint::Length(1),      // header (hunk N/M + choice)
        Constraint::Percentage(24), // local side
        Constraint::Percentage(24), // main side
        Constraint::Percentage(18), // hunk list
        Constraint::Min(2),         // assembled preview
        Constraint::Length(1),      // footer legend
    ])
    .split(inner);

    render_merge_header(frame, chunks[0], m);
    render_merge_side(
        frame,
        chunks[1],
        "Local file (keep-local)",
        m.focused_local(),
        Color::Cyan,
    );
    render_merge_side(
        frame,
        chunks[2],
        "Main branch (keep-main)",
        m.focused_main(),
        Color::Green,
    );
    render_merge_hunk_list(frame, chunks[3], m);
    render_merge_preview(frame, chunks[4], m);
    frame.render_widget(
        Paragraph::new(Line::from(
            "↑↓/jk hunk · ←→/hl choice · PgUp/PgDn/Space/b scroll preview · Enter accept · Esc cancel",
        ))
        .style(Style::new().fg(Color::DarkGray)),
        chunks[5],
    );
}

fn render_merge_header(frame: &mut Frame, area: Rect, m: &MergeOverlay) {
    let text = if m.hunk_count() == 0 {
        "No differing hunks — Enter accepts the shared text".to_string()
    } else {
        format!(
            "Hunk {}/{}    choice: {}",
            m.hunk + 1,
            m.hunk_count(),
            choice_label(m.choices[m.hunk]),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(text.bold())),
        area,
    );
}

/// Render one labeled side of the focused hunk; an empty side (a pure
/// insert/delete) is shown as an explicit placeholder rather than blank.
fn render_merge_side(frame: &mut Frame, area: Rect, label: &str, text: &str, color: Color) {
    let block = Block::bordered().title(Line::from(label.bold().fg(color)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    if text.is_empty() {
        lines.push(Line::from(Span::styled(
            "(nothing on this side)",
            Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
    } else {
        for l in text.lines() {
            lines.push(Line::from(Span::styled(l.to_string(), Style::new().fg(color))));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_merge_hunk_list(frame: &mut Frame, area: Rect, m: &MergeOverlay) {
    let block = Block::bordered().title(Line::from(" Hunks ".bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = (0..m.hunk_count())
        .map(|i| {
            let focused = i == m.hunk;
            let marker = if focused { "› " } else { "  " };
            let style = if focused {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::DarkGray)
            };
            Line::from(Span::styled(
                format!("{marker}hunk {}: {}", i + 1, choice_label(m.choices[i])),
                style,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_merge_preview(frame: &mut Frame, area: Rect, m: &MergeOverlay) {
    let block = Block::bordered().title(Line::from(" Assembled preview ".bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let preview = m.preview();
    let lines: Vec<Line> = preview
        .lines()
        .map(|l| Line::from(l.to_string()))
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((m.preview_scroll, 0))
            .wrap(Wrap { trim: false }),
        inner,
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
    // Construct the RAII guard the instant raw mode is on, BEFORE entering the
    // alternate screen: its Drop restores the terminal on EVERY later exit path
    // (normal return, a `?` error from EnterAlternateScreen or terminal setup,
    // or an unwinding panic) — a guard built after EnterAlternateScreen would
    // leak raw mode if that step (or `Terminal::new`) failed.
    let _guard = TerminalGuard;
    io::stdout()
        .execute(EnterAlternateScreen)
        .context("entering the alternate screen")?;

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

        if let Some(outcome) = handle_key(app, key.code, page) {
            return Ok(outcome);
        }
    }
}

/// Pure key dispatch: mutate `app` for the pressed `code` in the current mode
/// and return `Some(outcome)` when the loop should exit (apply or cancel), or
/// `None` to keep going. Factored out of [`event_loop`] so the whole key surface
/// — including the invariant-4 `Esc → Cancel` arm — is unit-testable with
/// synthetic [`KeyCode`]s, no terminal or `event::read` required. `page` is the
/// diff scroll step (terminal height − chrome).
fn handle_key(app: &mut App, code: KeyCode, page: u16) -> Option<CockpitOutcome> {
    match app.mode {
        Mode::Help => {
            if matches!(code, KeyCode::Char('?') | KeyCode::Esc) {
                app.mode = Mode::Normal;
            }
            None
        }
        Mode::Confirm => match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(CockpitOutcome::Apply(app.decisions())),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.mode = Mode::Normal;
                None
            }
            _ => None,
        },
        Mode::Merge => {
            match code {
                // Esc leaves the file's decision unchanged (F2).
                KeyCode::Esc => {
                    app.merge = None;
                    app.mode = Mode::Normal;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(m) = app.merge.as_mut() {
                        m.prev_hunk();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(m) = app.merge.as_mut() {
                        m.next_hunk();
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if let Some(m) = app.merge.as_mut() {
                        m.cycle_choice(true);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if let Some(m) = app.merge.as_mut() {
                        m.cycle_choice(false);
                    }
                }
                // The assembled preview can outgrow its pane — scroll it with
                // the same keys the normal view uses for the diff.
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    if let Some(m) = app.merge.as_mut() {
                        m.scroll_preview_down(page);
                    }
                }
                KeyCode::PageUp | KeyCode::Char('b') => {
                    if let Some(m) = app.merge.as_mut() {
                        m.scroll_preview_up(page);
                    }
                }
                KeyCode::Enter => app.accept_merge(),
                _ => {}
            }
            None
        }
        Mode::Normal => {
            // Any keypress clears a transient notice from the previous action;
            // a no-op `m` below may then set a fresh one.
            app.notice = None;
            match code {
                KeyCode::Esc => return Some(CockpitOutcome::Cancel),
                KeyCode::Up | KeyCode::Char('k') => app.focus_prev(),
                KeyCode::Down | KeyCode::Char('j') => app.focus_next(),
                KeyCode::PageDown | KeyCode::Char(' ') => app.scroll_down(page),
                KeyCode::PageUp | KeyCode::Char('b') => app.scroll_up(page),
                // Long lines: shift the diff CONTENT horizontally so a change
                // past the pane's right edge (e.g. a trailing comment) is
                // reachable — the pane title hints when lines are clipped.
                KeyCode::Right => app.scroll_right(H_SCROLL_STEP),
                KeyCode::Left => app.scroll_left(H_SCROLL_STEP),
                KeyCode::Char('p') => app.set_decision(Decision::Push),
                KeyCode::Char('l') => app.set_pull(),
                KeyCode::Char('m') => app.try_open_merge(),
                KeyCode::Char('d') => app.set_decision(Decision::Delete),
                KeyCode::Char('u') => app.set_decision(Decision::Undecided),
                KeyCode::Char('?') => app.mode = Mode::Help,
                KeyCode::Enter => app.mode = Mode::Confirm,
                _ => {}
            }
            None
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
