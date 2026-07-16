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
/// a decision badge, and the footer legend keys. The terminal is wide enough
/// (200 cols) that the diff PANE — only ~62% of the frame — still clears the
/// split threshold; a mere 120-col frame now renders unified.
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
    let out = buffer_text(&render(&app_with(files), 200, 30));

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

/// build_text_diff EOL-normalizes both sides at load: CRLF collapses to LF and
/// a missing trailing newline is added, so hunks reflect content only.
#[test]
fn build_text_diff_normalizes_eol_on_both_sides() {
    match build_text_diff(b"a\r\nX\r\nc", b"a\nY\nc\n") {
        FileDiff::Text { local, main } => {
            assert_eq!(local, "a\nX\nc\n");
            assert_eq!(main, "a\nY\nc\n");
        }
        other => panic!("expected FileDiff::Text, got a different variant: {:?}", std::mem::discriminant(&other)),
    }
}

/// A file whose sides differ ONLY by line endings / trailing newline renders
/// the explanatory notice instead of an empty diff pane.
#[test]
fn eol_only_difference_renders_notice_not_empty_diff() {
    // As build_text_diff would produce them: normalized-equal sides.
    let diff = build_text_diff(b"a\r\nb\r\n", b"a\nb");
    let files = vec![entry("dos.env", DiffStatus::Differs, Decision::Undecided, diff)];
    let out = buffer_text(&render(&app_with(files), 120, 30));
    assert!(
        out.contains("line endings"),
        "eol-only notice missing:\n{out}"
    );
}

/// Merging an EOL-only file: zero hunks, and accepting converges BOTH sides on
/// the normalized text — the in-tool path to make the phantom candidate go away.
#[test]
fn merge_on_eol_only_file_accepts_normalized_text() {
    let diff = build_text_diff(b"a\r\nb", b"a\nb\n");
    let mut app = app_with(vec![entry(
        "dos.env",
        DiffStatus::Differs,
        Decision::Undecided,
        diff,
    )]);
    app.try_open_merge();
    assert_eq!(app.mode, Mode::Merge, "eol-only text file can be merged");
    assert_eq!(app.merge.as_ref().unwrap().hunk_count(), 0, "no content hunks");
    app.accept_merge();
    match &app.files[0].decision {
        Decision::Merge(text) => assert_eq!(text, "a\nb\n"),
        other => panic!("expected Decision::Merge, got {other:?}"),
    }
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

/// The width→layout choice splits a wide diff pane and unifies a narrow one.
/// The argument is the diff-pane inner width, not the frame width.
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

/// Pull is a no-op when main's copy is UNREADABLE (present but permission/I/O
/// error): the diff pane shows pull disabled, and setting Pull would only fail
/// at apply time, so `l` leaves the decision unchanged (Finding 1). Distinct
/// from the worktree-only case: here `status == Differs`, so only the diff-side
/// guard can catch it.
#[test]
fn set_pull_is_noop_for_unreadable_main() {
    let mut app = app_with(vec![entry(
        "secret.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Unreadable {
            note: "main unreadable: permission denied — push only (pull/merge disabled)"
                .to_string(),
        },
    )]);
    app.set_pull();
    assert!(
        matches!(app.files[0].decision, Decision::Undecided),
        "pull must be a no-op for an unreadable-main file: {:?}",
        app.files[0].decision
    );
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

// ── handle_key dispatch (Finding 6) ────────────────────────────────────────

fn two_file_app() -> App {
    app_with(vec![
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
    ])
}

/// Normal-mode `Esc` yields `Cancel` (invariant 4) AND leaves every file's
/// decision untouched.
#[test]
fn handle_key_normal_esc_cancels_without_touching_decisions() {
    let mut app = two_file_app();
    let before: Vec<Decision> = app.files.iter().map(|f| f.decision.clone()).collect();

    let out = handle_key(&mut app, KeyCode::Esc, 10);

    assert_eq!(out, Some(CockpitOutcome::Cancel), "Esc must cancel");
    let after: Vec<Decision> = app.files.iter().map(|f| f.decision.clone()).collect();
    assert_eq!(before, after, "cancel must not change any decision");
}

/// `p` / `l` / `u` set the focused file's decision (and return `None`, staying
/// in the loop).
#[test]
fn handle_key_plu_set_focused_decision() {
    let mut app = app_with(vec![entry(
        "diff.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "x\n".to_string(),
            main: "y\n".to_string(),
        },
    )]);

    assert_eq!(handle_key(&mut app, KeyCode::Char('p'), 10), None);
    assert!(matches!(app.files[0].decision, Decision::Push));

    assert_eq!(handle_key(&mut app, KeyCode::Char('l'), 10), None);
    assert!(matches!(app.files[0].decision, Decision::Pull));

    assert_eq!(handle_key(&mut app, KeyCode::Char('u'), 10), None);
    assert!(matches!(app.files[0].decision, Decision::Undecided));
}

/// Confirm mode: `n` and `Esc` return to Normal without applying; `y` yields
/// `Apply` carrying the decided files.
#[test]
fn handle_key_confirm_mode_flow() {
    let mut app = app_with(vec![entry(
        "diff.env",
        DiffStatus::Differs,
        Decision::Pull,
        FileDiff::Text {
            local: "x\n".to_string(),
            main: "y\n".to_string(),
        },
    )]);

    app.mode = Mode::Confirm;
    assert_eq!(handle_key(&mut app, KeyCode::Char('n'), 10), None);
    assert_eq!(app.mode, Mode::Normal, "n returns to Normal");

    app.mode = Mode::Confirm;
    assert_eq!(handle_key(&mut app, KeyCode::Esc, 10), None);
    assert_eq!(app.mode, Mode::Normal, "Esc returns to Normal");

    app.mode = Mode::Confirm;
    match handle_key(&mut app, KeyCode::Char('y'), 10) {
        Some(CockpitOutcome::Apply(d)) => {
            assert_eq!(d.len(), 1, "one decided file");
            assert_eq!(d[0].0, PathBuf::from("diff.env"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

/// Merge mode: `→` cycles the focused hunk's choice, `Enter` accepts (recording
/// a merge decision and returning to Normal).
#[test]
fn handle_key_merge_mode_choice_and_accept() {
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
    assert_eq!(app.mode, Mode::Merge);

    // Right cycles keep-local → keep-main.
    assert_eq!(handle_key(&mut app, KeyCode::Right, 10), None);
    assert_eq!(app.merge.as_ref().unwrap().choices[0], MergeChoice::Main);

    // Enter accepts → back to Normal with a Merge decision.
    assert_eq!(handle_key(&mut app, KeyCode::Enter, 10), None);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.merge.is_none(), "overlay cleared on accept");
    assert!(matches!(app.files[0].decision, Decision::Merge(_)));
}

/// Merge mode: `↑`/`↓` navigate between hunks (clamped at the ends).
#[test]
fn handle_key_merge_mode_hunk_navigation() {
    let mut app = app_with(vec![entry(
        "c.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "X\nb\nZ\n".to_string(),
            main: "A\nb\nC\n".to_string(),
        },
    )]);
    app.try_open_merge();
    assert_eq!(app.merge.as_ref().unwrap().hunk_count(), 2, "two hunks");
    assert_eq!(app.merge.as_ref().unwrap().hunk, 0);

    handle_key(&mut app, KeyCode::Down, 10);
    assert_eq!(app.merge.as_ref().unwrap().hunk, 1);
    handle_key(&mut app, KeyCode::Up, 10);
    assert_eq!(app.merge.as_ref().unwrap().hunk, 0);
}

/// Merge mode: `Esc` cancels the overlay, leaving the file's decision unchanged.
#[test]
fn handle_key_merge_mode_esc_leaves_decision_unchanged() {
    let mut app = app_with(vec![entry(
        "c.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "a\nX\nc\n".to_string(),
            main: "a\nY\nc\n".to_string(),
        },
    )]);
    app.try_open_merge();

    assert_eq!(handle_key(&mut app, KeyCode::Esc, 10), None);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.merge.is_none());
    assert!(
        matches!(app.files[0].decision, Decision::Undecided),
        "Esc keeps the pre-merge decision"
    );
}

/// A file whose main side is unreadable is a merge no-op: `try_open_merge` sets
/// a notice instead of entering the overlay (never merges from fabricated
/// content).
#[test]
fn unreadable_main_disables_merge() {
    let mut app = app_with(vec![entry(
        "secret.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Unreadable {
            note: "main unreadable: permission denied — push only (pull/merge disabled)"
                .to_string(),
        },
    )]);
    app.try_open_merge();
    assert_eq!(app.mode, Mode::Normal, "unreadable main must not enter merge");
    assert!(app.merge.is_none(), "no overlay for an unreadable file");
    assert!(app.notice.is_some(), "a merge-unavailable notice is set");

    // The notice renders in the diff pane.
    let out = buffer_text(&render(&app, 120, 30));
    assert!(out.contains("main unreadable"), "unreadable notice missing:\n{out}");
}
