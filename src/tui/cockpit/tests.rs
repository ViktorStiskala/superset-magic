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

/// The foreground color carried by the first span of `line` that sets one — for
/// asserting per-row diff colors without the event loop.
fn line_fg(line: &Line) -> Option<Color> {
    line.spans.iter().find_map(|s| s.style.fg)
}

/// True when some rendered cell shows `sym` with foreground `fg` (`Cell.fg` is a
/// plain `Color`, defaulting to `Color::Reset` for unstyled cells).
fn any_cell_is(terminal: &Terminal<TestBackend>, sym: &str, fg: Color) -> bool {
    !cell_columns(terminal, sym, fg).is_empty()
}

/// The x column of every `sym`/`fg` cell (one entry per matching cell, sorted,
/// NOT deduped) — the caller derives both the cell count and the set of unique
/// columns, e.g. to assert the split divider is one continuous vertical rule.
fn cell_columns(terminal: &Terminal<TestBackend>, sym: &str, fg: Color) -> Vec<u16> {
    let buf = terminal.backend().buffer();
    let area = buf.area();
    let mut xs = Vec::new();
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(cell) = buf.cell((x, y)) {
                if cell.symbol() == sym && cell.fg == fg {
                    xs.push(x);
                }
            }
        }
    }
    xs.sort_unstable();
    xs
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
        diff_hscroll: 0,
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
    // Footer legend entries — full key+word substrings, since single chars
    // like 'd' would vacuously match badges and file names.
    for entry in ["p push", "l pull", "m merge", "d delete", "u undecided", "? help"] {
        assert!(out.contains(entry), "footer legend missing `{entry}`:\n{out}");
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

/// A read failure on a one-sided "will be created" file (worktree-only or
/// main-only) degrades to `FileDiff::Unreadable` — surfaced, cockpit stays open —
/// instead of propagating an error that would abort `App::new` for the whole
/// session, mirroring `build_two_sided`'s main-side handling.
#[test]
fn one_sided_read_error_degrades_to_unreadable_not_abort() {
    let missing = Path::new("/nonexistent-ss-magic-test-xyz/does/not/exist.env");
    match build_new(missing) {
        FileDiff::Unreadable { .. } => {}
        _ => panic!("build_new on an unreadable path must degrade to Unreadable"),
    }
    match build_main_only(missing) {
        FileDiff::Unreadable { .. } => {}
        _ => panic!("build_main_only on an unreadable path must degrade to Unreadable"),
    }
}

/// A two-sided (Differs) file whose WORKTREE side fails to read degrades to
/// `FileDiff::Unreadable` (cockpit stays open) instead of propagating an error
/// that would abort `App::new` — symmetric with the main-side handling.
#[test]
fn two_sided_worktree_read_error_degrades_to_unreadable_not_abort() {
    let missing_wt = Path::new("/nonexistent-ss-magic-test-xyz/wt.env");
    let missing_main = Path::new("/nonexistent-ss-magic-test-xyz/main.env");
    match build_two_sided(missing_wt, missing_main) {
        Ok(FileDiff::Unreadable { note }) => {
            assert!(note.contains("worktree unreadable"), "note: {note}")
        }
        _ => panic!("a worktree read error must degrade to a worktree Unreadable notice"),
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

/// Each decision maps to an unambiguous, direction-bearing badge label; the
/// delete badge names exactly the sides that will be removed (mirroring the
/// batched confirm's wording), so a worktree-only delete never claims a main
/// copy that does not exist.
#[test]
fn badge_text_reflects_direction() {
    let differs = DiffStatus::Differs;
    assert!(badge_text(&Decision::Push, differs).0.contains("push to main"));
    assert!(badge_text(&Decision::Pull, differs).0.contains("pull from main"));
    assert!(badge_text(&Decision::Undecided, differs).0.contains("undecided"));
    assert_eq!(
        badge_text(&Decision::Delete, differs).0,
        "✗ delete (worktree + main)"
    );
    assert_eq!(
        badge_text(&Decision::Delete, DiffStatus::WorktreeOnly).0,
        "✗ delete (worktree copy)"
    );
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

/// A delete is ALWAYS destructive: both a differing and a worktree-only file
/// marked delete appear in the confirm list, labeled with the sides removed.
#[test]
fn destructive_overwrites_lists_deletes_with_side_labels() {
    let files = vec![
        entry(
            "gone.env",
            DiffStatus::Differs,
            Decision::Delete,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
        entry(
            "gone-new.env",
            DiffStatus::WorktreeOnly,
            Decision::Delete,
            FileDiff::New { content: None },
        ),
    ];
    let over = app_with(files).destructive_overwrites();
    assert_eq!(over.len(), 2, "both deletes are destructive: {over:?}");
    let label_of = |rel: &str| {
        over.iter()
            .find(|(p, _)| p == Path::new(rel))
            .map(|(_, l)| *l)
            .unwrap_or_else(|| panic!("{rel} missing from {over:?}"))
    };
    assert_eq!(label_of("gone.env"), "delete (worktree + main)");
    assert_eq!(label_of("gone-new.env"), "delete (worktree copy)");
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

// ── Long lines: horizontal scroll + clipped-tail hint ──────────────────────

/// The smoke-caught bug: a line whose ONLY change sits past the pane's right
/// edge (a trailing comment at col ~80) rendered as two identical-looking
/// sides with no hint that anything was clipped. The pane title must flag
/// clipped lines, and `→` must scroll the content (fixed gutter) until the
/// changed tail is visible.
#[test]
fn long_line_change_past_pane_edge_is_flagged_and_reachable() {
    let prefix = format!(
        "# pulse_product_name: \"Spiral\"{}# used in report titles",
        " ".repeat(26)
    );
    let mut app = app_with(vec![
        entry(
            "config.local.yaml",
            DiffStatus::Differs,
            Decision::Undecided,
            FileDiff::Text {
                local: format!("{prefix}\n"),
                main: format!("{prefix}EXTRA\n"),
            },
        ),
        entry(
            "short.env",
            DiffStatus::Differs,
            Decision::Undecided,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
    ]);

    // 100 cols → the diff pane falls back to unified; its content area is far
    // narrower than the ~84-char lines, so the changed tail starts clipped.
    let out = buffer_text(&render(&app, 100, 30));
    assert!(
        !out.contains("EXTRA"),
        "precondition: the changed tail must start beyond the pane edge:\n{out}"
    );
    assert!(
        out.contains("lines continue"),
        "clipped lines must be flagged in the pane title:\n{out}"
    );

    // Scroll right until the tail is in view; the title shows the offset and
    // the line-number gutter stays put.
    for _ in 0..5 {
        handle_key(&mut app, KeyCode::Right, 10);
    }
    assert_eq!(app.diff_hscroll, 40);
    let out = buffer_text(&render(&app, 100, 30));
    assert!(
        out.contains("EXTRA"),
        "the changed tail must be reachable via → scrolling:\n{out}"
    );
    assert!(out.contains("→ col 40"), "title must show the offset:\n{out}");
    assert!(out.contains("   1"), "line-number gutter must stay fixed:\n{out}");

    // Moving to another file resets the offset; short lines get no hint.
    handle_key(&mut app, KeyCode::Down, 10);
    assert_eq!(app.diff_hscroll, 0, "focus move must reset the h-scroll");
    let out = buffer_text(&render(&app, 100, 30));
    assert!(!out.contains("lines continue"), "short lines need no hint:\n{out}");
    assert!(!out.contains("→ col"), "no offset shown at col 0:\n{out}");
}

/// `←`/`→` clamp: left saturates at 0, right at the longest content line; a
/// file with no scrollable content (binary) never scrolls.
#[test]
fn handle_key_horizontal_scroll_clamps() {
    let mut app = app_with(vec![entry(
        "wide.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: format!("{}\n", "L".repeat(30)),
            main: format!("{}\n", "M".repeat(20)),
        },
    )]);
    assert_eq!(handle_key(&mut app, KeyCode::Left, 10), None);
    assert_eq!(app.diff_hscroll, 0, "left saturates at 0");
    for _ in 0..10 {
        handle_key(&mut app, KeyCode::Right, 10);
    }
    assert_eq!(
        app.diff_hscroll, 29,
        "right clamps to the longest content line - 1"
    );

    let mut binary = app_with(vec![entry(
        "secret.bin",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Binary {
            note: "binary — differs".to_string(),
        },
    )]);
    handle_key(&mut binary, KeyCode::Right, 10);
    assert_eq!(binary.diff_hscroll, 0, "a binary notice never h-scrolls");
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
        badge_text(&app.files[0].decision, app.files[0].status)
            .0
            .contains("merge"),
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

    assert_eq!(handle_key(&mut app, KeyCode::Char('d'), 10), None);
    assert!(matches!(app.files[0].decision, Decision::Delete));

    assert_eq!(handle_key(&mut app, KeyCode::Char('u'), 10), None);
    assert!(matches!(app.files[0].decision, Decision::Undecided));
}

/// Confirm mode: `Esc` returns to Normal without applying; `Enter` yields
/// `Apply` carrying the decided files; the old `y`/`n` keys are no longer bound
/// (they are inert no-ops that leave the confirm open).
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

    // y/n are no longer bound: they are no-ops and the confirm stays open.
    app.mode = Mode::Confirm;
    assert_eq!(handle_key(&mut app, KeyCode::Char('y'), 10), None);
    assert_eq!(app.mode, Mode::Confirm, "y is inert, confirm stays open");
    assert_eq!(handle_key(&mut app, KeyCode::Char('n'), 10), None);
    assert_eq!(app.mode, Mode::Confirm, "n is inert, confirm stays open");

    // Esc backs out to Normal without applying.
    assert_eq!(handle_key(&mut app, KeyCode::Esc, 10), None);
    assert_eq!(app.mode, Mode::Normal, "Esc returns to Normal");

    // Enter applies, carrying the decided files.
    app.mode = Mode::Confirm;
    match handle_key(&mut app, KeyCode::Enter, 10) {
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

/// Merge mode: PgDn/PgUp (and Space/b) scroll the assembled preview, clamped to
/// its line count; cycling a choice re-clamps a scroll that outgrew the new,
/// shorter preview.
#[test]
fn handle_key_merge_mode_scrolls_preview_clamped() {
    // 30 shared lines + one hunk → a preview tall enough to scroll.
    let shared: String = (0..30).map(|i| format!("line{i}\n")).collect();
    let mut app = app_with(vec![entry(
        "long.env",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: format!("{shared}X\n"),
            main: format!("{shared}Y\n"),
        },
    )]);
    app.try_open_merge();
    assert_eq!(app.merge.as_ref().unwrap().preview_scroll, 0);

    // Page down by 10 → offset 10; again → clamped at the last line (30).
    handle_key(&mut app, KeyCode::PageDown, 10);
    assert_eq!(app.merge.as_ref().unwrap().preview_scroll, 10);
    handle_key(&mut app, KeyCode::Char(' '), 10);
    handle_key(&mut app, KeyCode::PageDown, 100);
    assert_eq!(
        app.merge.as_ref().unwrap().preview_scroll,
        30,
        "scroll clamps to the preview's last line"
    );

    // Page up scrolls back (Char('b') too) and saturates at 0.
    handle_key(&mut app, KeyCode::PageUp, 10);
    assert_eq!(app.merge.as_ref().unwrap().preview_scroll, 20);
    handle_key(&mut app, KeyCode::Char('b'), 100);
    assert_eq!(app.merge.as_ref().unwrap().preview_scroll, 0);

    // A choice change re-clamps: scroll to the bottom of a keep-both preview,
    // then shrink it back to keep-local — the offset must follow it down.
    handle_key(&mut app, KeyCode::Right, 10); // keep-main
    handle_key(&mut app, KeyCode::Right, 10); // keep-both (32 lines)
    handle_key(&mut app, KeyCode::PageDown, 100);
    assert_eq!(app.merge.as_ref().unwrap().preview_scroll, 31);
    handle_key(&mut app, KeyCode::Right, 10); // back to keep-local (31 lines)
    assert_eq!(
        app.merge.as_ref().unwrap().preview_scroll,
        30,
        "cycling a choice must re-clamp the preview scroll"
    );
}

/// The help overlay documents every decision key, the preview scroll, and the
/// backup location + retention.
#[test]
fn help_overlay_documents_keys_and_backups() {
    let mut app = two_file_app();
    app.mode = Mode::Help;
    let out = buffer_text(&render(&app, 120, 40));
    assert!(out.contains("delete from both sides"), "d key missing:\n{out}");
    assert!(out.contains("assembled preview"), "preview scroll missing:\n{out}");
    assert!(out.contains(".superset/backups/"), "backup path missing:\n{out}");
    assert!(out.contains("10 newest"), "retention missing:\n{out}");
}

/// The help popup is sized to its content, so even an 80×24 terminal shows the
/// FULL help — including the trailing safety facts, which a fixed-percentage
/// popup used to clip silently.
#[test]
fn help_overlay_fits_an_80x24_terminal() {
    let mut app = two_file_app();
    app.mode = Mode::Help;
    let out = buffer_text(&render(&app, 80, 24));
    assert!(out.contains("Navigation"), "help top missing:\n{out}");
    assert!(
        out.contains(".superset/backups/"),
        "backup path (tail of the help) clipped:\n{out}"
    );
    assert!(out.contains("10 newest"), "retention (tail) clipped:\n{out}");
    assert!(
        out.contains("EOL-normalized"),
        "last help line clipped:\n{out}"
    );
}

/// The batched-confirm overlay renders the overwrite-or-delete warning and the
/// per-file side labels for deletes; with nothing destructive it says so.
#[test]
fn confirm_overlay_renders_delete_labels_and_clean_case() {
    let mut app = app_with(vec![
        entry(
            "gone.env",
            DiffStatus::Differs,
            Decision::Delete,
            FileDiff::Text {
                local: "x\n".to_string(),
                main: "y\n".to_string(),
            },
        ),
        entry(
            "gone-new.env",
            DiffStatus::WorktreeOnly,
            Decision::Delete,
            FileDiff::New { content: None },
        ),
    ]);
    app.mode = Mode::Confirm;
    let out = buffer_text(&render(&app, 120, 30));
    assert!(
        out.contains("OVERWRITTEN or DELETED"),
        "destructive warning missing:\n{out}"
    );
    assert!(
        out.contains("delete (worktree + main)"),
        "differs delete label missing:\n{out}"
    );
    assert!(
        out.contains("delete (worktree copy)"),
        "worktree-only delete label missing:\n{out}"
    );

    // Nothing destructive: a worktree-only push is a plain create.
    let mut clean = app_with(vec![entry(
        "new.env",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New { content: None },
    )]);
    clean.mode = Mode::Confirm;
    let out = buffer_text(&render(&clean, 120, 30));
    assert!(
        out.contains("No existing files will be overwritten or deleted."),
        "clean confirm wording missing:\n{out}"
    );
}

/// A destructive list too long for the terminal truncates with an EXPLICIT
/// "… and N more" marker — never silently — and the count and Enter/Esc prompt
/// stay visible, even at 80×20.
#[test]
fn confirm_overlay_truncates_long_lists_with_explicit_marker() {
    let files: Vec<FileEntry> = (0..20)
        .map(|i| {
            entry(
                &format!("file{i:02}.env"),
                DiffStatus::Differs,
                Decision::Pull,
                FileDiff::Text {
                    local: "x\n".to_string(),
                    main: "y\n".to_string(),
                },
            )
        })
        .collect();
    let mut app = app_with(files);
    app.mode = Mode::Confirm;
    let out = buffer_text(&render(&app, 80, 20));
    assert!(out.contains("file00.env"), "leading entries listed:\n{out}");
    assert!(
        !out.contains("file19.env"),
        "overflow entries must not render past the popup:\n{out}"
    );
    assert!(
        out.contains("… and 10 more"),
        "explicit truncation marker missing:\n{out}"
    );
    assert!(
        out.contains("20 file(s) will be written."),
        "count must stay visible:\n{out}"
    );
    assert!(
        out.contains("Enter = apply"),
        "prompt must stay visible:\n{out}"
    );
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

// ── Task 1: file-list wrapping ────────────────────────────────────────────

/// A path that fits the pane width is a single chunk (no spurious extra line).
#[test]
fn wrap_hard_returns_one_chunk_when_it_fits() {
    assert_eq!(wrap_hard("short.env", 26), vec!["short.env".to_string()]);
}

/// A long, whitespace-free path hard-wraps at the char width (47 chars → 26+21).
#[test]
fn wrap_hard_splits_long_token_at_char_width() {
    assert_eq!(
        wrap_hard("services/billing/invoice-templates/monthly.tmpl", 26),
        vec![
            "services/billing/invoice-t".to_string(),
            "emplates/monthly.tmpl".to_string(),
        ]
    );
}

/// Empty string → one blank line (stable min height); width 0 → one char per
/// chunk instead of panicking on `chunks(0)`.
#[test]
fn wrap_hard_handles_empty_string_and_zero_width() {
    assert_eq!(wrap_hard("", 10), vec![String::new()]);
    assert_eq!(
        wrap_hard("abc", 0),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

/// The file-list content width is the pane width minus the border (2) and the
/// reserved highlight symbol (2).
#[test]
fn file_list_content_width_subtracts_border_and_highlight_symbol() {
    assert_eq!(file_list_content_width(Rect::new(0, 0, 30, 30)), 26);
    assert_eq!(file_list_content_width(Rect::new(0, 0, 46, 30)), 42);
    // Underflow saturates to 0, never wraps around.
    assert_eq!(file_list_content_width(Rect::new(0, 0, 2, 30)), 0);
}

/// A long path in a narrow Files pane wraps across rows instead of clipping —
/// both halves of the split path are present in the rendered buffer.
#[test]
fn narrow_pane_file_list_wraps_long_path_instead_of_clipping() {
    let files = vec![entry(
        "services/billing/invoice-templates/monthly.tmpl",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New { content: None },
    )];
    let out = buffer_text(&render(&app_with(files), 80, 30));
    assert!(
        out.contains("services/billing/invoice-t"),
        "first wrapped chunk missing:\n{out}"
    );
    assert!(
        out.contains("emplates/monthly.tmpl"),
        "second wrapped chunk missing:\n{out}"
    );
}

// ── Task 2: split divider + local-green / main-red recolor ────────────────

fn seg1(text: &str) -> Vec<diffmodel::Seg> {
    vec![diffmodel::Seg {
        text: text.to_string(),
        emphasized: false,
    }]
}

/// side_columns colors a local-only line (Delete) green on the left, a main-only
/// line (Insert) red on the right, and a Replace row green-left / red-right —
/// the main = base, local = working copy model.
#[test]
fn side_columns_colors_local_green_and_main_red() {
    let rows = vec![
        DiffRow {
            left_no: Some(1),
            left: seg1("local_add"),
            right_no: None,
            right: Vec::new(),
            tag: RowTag::Delete,
        },
        DiffRow {
            left_no: None,
            left: Vec::new(),
            right_no: Some(1),
            right: seg1("main_add"),
            tag: RowTag::Insert,
        },
        DiffRow {
            left_no: Some(2),
            left: seg1("L"),
            right_no: Some(2),
            right: seg1("M"),
            tag: RowTag::Replace,
        },
    ];
    let (left, right) = side_columns(&rows);
    assert_eq!(line_fg(&left.content[0]), Some(Color::Green), "local-only green");
    assert_eq!(line_fg(&right.content[1]), Some(Color::Red), "main-only red");
    assert_eq!(line_fg(&left.content[2]), Some(Color::Green), "replace local green");
    assert_eq!(line_fg(&right.content[2]), Some(Color::Red), "replace main red");
}

/// The unified view (narrow pane) colors a local addition `+` green and a
/// main-side line `-` red, matching the split view.
#[test]
fn unified_view_signs_and_colors_follow_local_addition_convention() {
    let files = vec![entry(
        "config.local.json",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "a\nLOCAL_ONLY\nc\n".to_string(),
            main: "a\nMAIN_ONLY\nc\n".to_string(),
        },
    )];
    let t = render(&app_with(files), 80, 30);
    assert!(any_cell_is(&t, "+", Color::Green), "local addition '+' must be green");
    assert!(any_cell_is(&t, "-", Color::Red), "main-side '-' must be red");
}

/// After the `unified(main, local)` arg-swap, the gutter still prints the LOCAL
/// line number before the main one (only sign/color meaning flipped).
#[test]
fn render_unified_keeps_local_number_column_first_after_arg_swap() {
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_unified(
                frame,
                area,
                "L1\ncommon1\ncommon2\ncommon3\n",
                "common1\ncommon2\ncommon3\n",
                0,
                0,
            );
        })
        .unwrap();
    let out = buffer_text(&terminal);
    // common1 is local line 2, main line 1 → gutter "   2    1" (local first).
    assert!(
        out.contains("   2    1"),
        "local number must print before main number:\n{out}"
    );
}

/// The split view draws one continuous faint (DarkGray) vertical divider between
/// the Local and Main columns.
#[test]
fn split_view_renders_divider_between_local_and_main_columns() {
    let files = vec![entry(
        "config.local.json",
        DiffStatus::Differs,
        Decision::Undecided,
        FileDiff::Text {
            local: "a\nb\nc\n".to_string(),
            main: "a\nB\nc\n".to_string(),
        },
    )];
    let t = render(&app_with(files), 200, 30);
    let cols = cell_columns(&t, "│", Color::DarkGray);
    assert!(cols.len() >= 2, "expected a multi-row divider, got {cols:?}");
    let mut unique = cols.clone();
    unique.dedup();
    assert_eq!(
        unique.len(),
        1,
        "divider must be one continuous column, got columns {unique:?}"
    );
}

// ── Task 4: new-file view header + numbered gutter ────────────────────────

/// A worktree-only file shows the green header notice and 1-based numbered `+`
/// content rows (line numbers in a fixed gutter).
#[test]
fn new_file_view_shows_header_and_numbered_gutter() {
    let files = vec![entry(
        "apps/api/.env",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New {
            content: Some("FIRST=1\nSECOND=2\n".to_string()),
        },
    )];
    let out = buffer_text(&render(&app_with(files), 120, 24));
    assert!(
        out.contains("new file — will be created in main"),
        "green header notice missing:\n{out}"
    );
    assert!(out.contains("1 + FIRST=1"), "numbered first line missing:\n{out}");
    assert!(out.contains("2 + SECOND=2"), "numbered second line missing:\n{out}");
}

/// A binary/oversized new file (content `None`) still shows the header, then the
/// placeholder instead of a numbered gutter.
#[test]
fn new_file_view_without_content_shows_placeholder_not_gutter() {
    let files = vec![entry(
        "apps/api/blob.bin",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New { content: None },
    )];
    let out = buffer_text(&render(&app_with(files), 120, 24));
    assert!(
        out.contains("new file — will be created in main"),
        "green header notice missing:\n{out}"
    );
    assert!(
        out.contains("(binary or oversized new file — content not shown)"),
        "placeholder missing:\n{out}"
    );
}

/// diff_line_count for a new file counts only the numbered content rows (the
/// header row is fixed, not scrolled); a content-less new file is 1.
#[test]
fn diff_line_count_new_file_excludes_header_rows() {
    let with_content = entry(
        "a.env",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New {
            content: Some("a\nb\n".to_string()),
        },
    );
    assert_eq!(diff_line_count(&with_content), 2);
    let no_content = entry(
        "b.bin",
        DiffStatus::WorktreeOnly,
        Decision::Push,
        FileDiff::New { content: None },
    );
    assert_eq!(diff_line_count(&no_content), 1);
}

// ── Task 5: MainOnly cockpit view + the set_push gate ─────────────────────

/// The delete badge names the main copy exactly (mirroring the batched
/// confirm's wording); pull is the natural, always-available direction for a
/// main-only file.
#[test]
fn badge_text_main_only_delete_and_pull() {
    assert_eq!(
        badge_text(&Decision::Delete, DiffStatus::MainOnly).0,
        "✗ delete (main copy)"
    );
    assert!(badge_text(&Decision::Pull, DiffStatus::MainOnly)
        .0
        .contains("pull from main"));
}

/// A main-only file renders the Cyan "will be created in this worktree"
/// header and the same 1-based numbered `+` gutter as a worktree-only file
/// (mirrored from main's content instead of the worktree's).
#[test]
fn main_only_file_renders_numbered_pull_notice() {
    let files = vec![entry(
        "config.main.json",
        DiffStatus::MainOnly,
        Decision::Pull,
        FileDiff::MainOnly {
            content: Some("K=V\n".to_string()),
        },
    )];
    let out = buffer_text(&render(&app_with(files), 120, 24));
    assert!(
        out.contains("main only"),
        "main-only header notice missing:\n{out}"
    );
    assert!(out.contains("1 + K=V"), "numbered content line missing:\n{out}");
}

/// `p` is a no-op on a main-only file (no worktree copy to push) and sets a
/// transient notice instead of a decision; `l` (pull) is unaffected and still
/// works, since main IS present and readable.
#[test]
fn set_push_is_noop_for_main_only_and_pull_allowed() {
    let mut app = app_with(vec![entry(
        "config.main.json",
        DiffStatus::MainOnly,
        Decision::Undecided,
        FileDiff::MainOnly {
            content: Some("K=V\n".to_string()),
        },
    )]);
    app.set_push();
    assert!(
        matches!(app.files[0].decision, Decision::Undecided),
        "push must be a no-op for a main-only file: {:?}",
        app.files[0].decision
    );
    assert!(app.notice.is_some(), "a push-unavailable notice is set");

    app.set_pull();
    assert!(
        matches!(app.files[0].decision, Decision::Pull),
        "pull must still be allowed for a main-only file"
    );
}

/// destructive_overwrites: a main-only PULL only CREATES the worktree file
/// (not destructive, so excluded); a main-only DELETE removes main's only
/// copy and is listed as such.
#[test]
fn destructive_overwrites_main_only_pull_is_create_delete_is_main_copy() {
    let pulled = app_with(vec![entry(
        "config.main.json",
        DiffStatus::MainOnly,
        Decision::Pull,
        FileDiff::MainOnly { content: None },
    )])
    .destructive_overwrites();
    assert!(
        pulled.is_empty(),
        "a main-only pull creates the worktree file, not destructive: {pulled:?}"
    );

    let deleted = app_with(vec![entry(
        "config.main.json",
        DiffStatus::MainOnly,
        Decision::Delete,
        FileDiff::MainOnly { content: None },
    )])
    .destructive_overwrites();
    assert_eq!(deleted.len(), 1, "a main-only delete is destructive: {deleted:?}");
    assert_eq!(deleted[0].1, "delete (main copy)");
}

/// Regression for the switch from `set_decision(Decision::Push)` to the
/// gated `set_push()`: `p` on an ordinary Differs file must still push it —
/// the MainOnly guard must not swallow the common case.
#[test]
fn handle_key_p_still_pushes_a_differs_file_after_set_push_switch() {
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
    assert!(app.notice.is_none(), "no notice for a normal push");
}
