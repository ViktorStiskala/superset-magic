use super::*;
use ratatui::backend::TestBackend;

/// Reconstruct the rendered buffer as newline-joined text so we can assert on
/// visible content without touching the event loop.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let area = buf.area();
    let mut s = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(cell) = buf.cell((x, y)) {
                s.push_str(cell.symbol());
            }
        }
        s.push('\n');
    }
    s
}

fn entry(rel: &str, status: DiffStatus, decision: Decision, diff: FileDiff) -> FileEntry {
    FileEntry {
        rel: PathBuf::from(rel),
        status,
        decision,
        diff,
        mtime_local: "1m ago".to_string(),
        mtime_main: "2m ago".to_string(),
    }
}

fn app_with(files: Vec<FileEntry>) -> App {
    App {
        files,
        focused: 0,
        diff_scroll: 0,
        mode: Mode::Normal,
        merge: None,
        notice: None,
    }
}

fn render(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
}

/// A wide terminal renders the file names, the two side-by-side column titles,
/// a decision badge, and the footer legend keys.
#[test]
fn wide_render_shows_list_titles_badge_and_footer() {
    let files = vec![
        entry(
            "config.local.json",
            DiffStatus::Differs,
            Decision::Undecided,
            FileDiff::Text {
                local: "a\nb\nc\n".to_string(),
                main: "a\nB\nc\n".to_string(),
            },
        ),
        entry(
            "apps/api/.env",
            DiffStatus::WorktreeOnly,
            Decision::Push,
            FileDiff::New {
                content: Some("SECRET=1\n".to_string()),
            },
        ),
    ];
    let out = buffer_text(&render(&app_with(files), 120, 30));

    assert!(out.contains("config.local.json"), "file name missing:\n{out}");
    assert!(out.contains("apps/api/.env"), "file name missing:\n{out}");
    assert!(out.contains("Local file"), "split column title missing:\n{out}");
    assert!(out.contains("Main branch"), "split column title missing:\n{out}");
    assert!(out.contains("push to main"), "decision badge missing:\n{out}");
    // Footer legend keys.
    for key in ['p', 'l', 'u', '?'] {
        assert!(out.contains(key), "footer legend missing `{key}`:\n{out}");
    }
}

/// A narrow terminal falls back to the unified layout — the two column titles
/// are NOT both present (R7).
#[test]
fn narrow_render_uses_unified_layout() {
    let files = vec![entry(
        "config.local.json",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "a\nb\nc\n".to_string(),
            main: "a\nB\nc\n".to_string(),
        },
    )];
    let out = buffer_text(&render(&app_with(files), 80, 30));

    let both_titles = out.contains("Local file") && out.contains("Main branch");
    assert!(
        !both_titles,
        "80-col terminal must render the unified (single-column) layout:\n{out}"
    );
}

/// A binary file renders the "binary — differs" notice, not a diff (R9).
#[test]
fn binary_file_renders_notice_not_diff() {
    let files = vec![entry(
        "secret.bin",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Binary {
            note: "binary — differs (local 12 bytes / main 8 bytes / hash 00000000deadbeef)"
                .to_string(),
        },
    )];
    let out = buffer_text(&render(&app_with(files), 120, 30));
    assert!(out.contains("binary — differs"), "binary notice missing:\n{out}");
}

/// `is_interactive` is callable (its value depends on the test environment's
/// TTYs, so we only assert it does not panic).
#[test]
fn is_interactive_is_callable() {
    let _ = is_interactive();
}

/// The width→layout choice splits wide terminals and unifies narrow ones.
#[test]
fn use_split_thresholds() {
    assert!(use_split(120));
    assert!(!use_split(80));
}

/// Each decision maps to an unambiguous, direction-bearing badge label.
#[test]
fn badge_text_reflects_direction() {
    assert!(badge_text(&Decision::Push).0.contains("push to main"));
    assert!(badge_text(&Decision::Pull).0.contains("pull from main"));
    assert!(badge_text(&Decision::Undecided).0.contains("undecided"));
}

/// Apply collects only non-undecided decisions.
#[test]
fn decisions_excludes_undecided() {
    let files = vec![
        entry(
            "a.env",
            DiffStatus::WorktreeOnly,
            Decision::Push,
            FileDiff::New { content: None },
        ),
        entry(
            "b.env",
            DiffStatus::Differs,
            Decision::Undecided,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
        entry(
            "c.env",
            DiffStatus::Differs,
            Decision::Pull,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
    ];
    let decisions = app_with(files).decisions();
    assert_eq!(decisions.len(), 2, "undecided must be excluded: {decisions:?}");
    assert!(!decisions.iter().any(|(p, _)| p == Path::new("b.env")));
}

/// Only existing-target overwrites are listed as destructive: a worktree-only
/// push (a create) is not, a pull and a differing push are.
#[test]
fn destructive_overwrites_lists_only_existing_targets() {
    let files = vec![
        entry(
            "new.env",
            DiffStatus::WorktreeOnly,
            Decision::Push,
            FileDiff::New { content: None },
        ),
        entry(
            "over.env",
            DiffStatus::Differs,
            Decision::Push,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
        entry(
            "pull.env",
            DiffStatus::Differs,
            Decision::Pull,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
    ];
    let over = app_with(files).destructive_overwrites();
    let paths: Vec<String> = over.iter().map(|(p, _)| p.display().to_string()).collect();
    assert!(paths.contains(&"over.env".to_string()), "{paths:?}");
    assert!(paths.contains(&"pull.env".to_string()), "{paths:?}");
    assert!(!paths.contains(&"new.env".to_string()), "create is not destructive: {paths:?}");
}

/// Pull is a no-op on a worktree-only file (main has nothing to pull).
#[test]
fn set_pull_is_noop_for_worktree_only() {
    let mut app = app_with(vec![entry(
        "new.env",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New { content: None },
    )]);
    app.set_pull();
    assert!(matches!(app.files[0].decision, Decision::Push));
}

// ── Interactive merge overlay (Phase 4) ───────────────────────────────────

/// Opening the merge overlay for a one-hunk file (local "X" vs main "Y") shows
/// both hunk sides and the live assembled preview.
#[test]
fn merge_overlay_renders_both_sides_and_preview() {
    let mut app = app_with(vec![entry(
        "config.local.json",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "a\nX\nc\n".to_string(),
            main: "a\nY\nc\n".to_string(),
        },
    )]);
    app.try_open_merge();
    assert_eq!(app.mode, Mode::Merge, "m must enter the merge overlay");

    let out = buffer_text(&render(&app, 120, 40));
    assert!(out.contains("Local file"), "local side label missing:\n{out}");
    assert!(out.contains("Main branch"), "main side label missing:\n{out}");
    assert!(out.contains('X'), "local hunk text missing:\n{out}");
    assert!(out.contains('Y'), "main hunk text missing:\n{out}");
    assert!(
        out.contains("Assembled preview"),
        "preview pane missing:\n{out}"
    );
}

/// The overlay's assembled preview tracks the focused hunk's choice: Main yields
/// the main side (== `merge::assemble` with `[Main]`); Both yields local then
/// main, in order.
#[test]
fn merge_overlay_preview_tracks_choice() {
    let local = "a\nX\nc\n";
    let main = "a\nY\nc\n";
    let mut overlay = MergeOverlay::build(0, local, main);
    assert_eq!(overlay.hunk_count(), 1, "exactly one differing hunk");

    overlay.choices[0] = MergeChoice::Main;
    let segs = merge_segments(local, main);
    assert_eq!(overlay.preview(), assemble(&segs, &[MergeChoice::Main]));
    assert_eq!(overlay.preview(), main, "keep-main yields the main text");

    overlay.choices[0] = MergeChoice::Both;
    let both = overlay.preview();
    assert_eq!(both, "a\nX\nY\nc\n", "keep-both interleaves local then main");
    let x = both.find('X').expect("local present in keep-both");
    let y = both.find('Y').expect("main present in keep-both");
    assert!(x < y, "keep-both must place local before main: {both:?}");
}

/// `m` is a no-op on a binary file and on a worktree-only (new) file — neither
/// can be merged, so the overlay never opens and a notice is set instead.
#[test]
fn merge_key_is_noop_for_binary_and_new_files() {
    let mut binary = app_with(vec![entry(
        "secret.bin",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Binary {
            note: "binary — differs".to_string(),
        },
    )]);
    binary.try_open_merge();
    assert_eq!(binary.mode, Mode::Normal, "binary must not enter merge");
    assert!(binary.merge.is_none(), "no overlay for a binary file");
    assert!(binary.notice.is_some(), "a merge-unavailable notice is set");

    let mut new = app_with(vec![entry(
        "new.env",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New {
            content: Some("SECRET=1\n".to_string()),
        },
    )]);
    new.try_open_merge();
    assert_eq!(new.mode, Mode::Normal, "worktree-only must not enter merge");
    assert!(new.merge.is_none(), "no overlay for a worktree-only file");
}

/// Accepting the overlay records a [`Decision::Merge`] carrying the assembled
/// bytes and returns to the normal view with the merge badge.
#[test]
fn accept_merge_sets_merge_decision_and_badge() {
    let mut app = app_with(vec![entry(
        "config.local.json",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "a\nX\nc\n".to_string(),
            main: "a\nY\nc\n".to_string(),
        },
    )]);
    app.try_open_merge();
    // Default choice is keep-local ⇒ assembled == the local text.
    app.accept_merge();

    assert_eq!(app.mode, Mode::Normal, "accept returns to the cockpit");
    assert!(app.merge.is_none(), "overlay state is cleared on accept");
    match &app.files[0].decision {
        Decision::Merge(text) => assert_eq!(text, "a\nX\nc\n"),
        other => panic!("expected Decision::Merge, got {other:?}"),
    }
    assert!(
        badge_text(&app.files[0].decision).0.contains("merge"),
        "badge reflects the merge decision"
    );
}
