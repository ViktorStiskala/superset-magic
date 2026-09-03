//! `run_core` (the `--set` write), `parse_args` (the pure argv half), and the
//! read-only `--recommend` report — the heuristic on its own, the report over
//! real settings files and a real ledger, and the proof that none of it
//! writes.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::plugin::ledger::{Basis, Row, Tokens, LEDGER_FILE_NAME};
use crate::plugin::scratchpad::format_rfc3339;
use crate::tests::support::{exit_code_to_u8, init_main_repo};

fn fixture() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn read_local_settings(root: &std::path::Path) -> Value {
    let raw = fs::read_to_string(root.join(SETTINGS_LOCAL_REL)).unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn gitignore_contents(root: &std::path::Path) -> String {
    fs::read_to_string(root.join(".gitignore")).unwrap_or_default()
}

// ── `parse_args` — pure, no filesystem ──────────────────────────────────────

#[test]
fn set_with_a_value_in_bounds_parses() {
    let args = vec!["--set".to_string(), "200000".to_string()];
    assert!(matches!(parse_args(&args), ParsedArgs::Set(200_000)));
}

#[test]
fn set_below_the_lower_bound_is_an_error_naming_the_range() {
    let args = vec!["--set".to_string(), "1000".to_string()];
    match parse_args(&args) {
        ParsedArgs::Error(message) => {
            assert!(message.contains("100000"), "{message}");
            assert!(message.contains("1000000"), "{message}");
        }
        other => panic!("expected an error, got a {other:?}-shaped result"),
    }
}

#[test]
fn set_above_the_upper_bound_is_an_error() {
    let args = vec!["--set".to_string(), "5000000".to_string()];
    assert!(matches!(parse_args(&args), ParsedArgs::Error(_)));
}

/// Non-numeric and shorthand (`200k`) values are both rejected — R30 asks for
/// an absolute count, and this verb does not invent a second notation on top
/// of it.
#[test]
fn a_non_numeric_value_is_an_error_not_a_panic() {
    for bad in ["200k", "auto", "-100000", "200000.5", ""] {
        let args = vec!["--set".to_string(), bad.to_string()];
        assert!(
            matches!(parse_args(&args), ParsedArgs::Error(_)),
            "`{bad}` should be rejected"
        );
    }
}

/// `--recommend` takes an optional `--json` in either order; `--json` on its
/// own has nothing to apply to.
#[test]
fn recommend_parses_with_json_in_either_order_and_json_alone_is_an_error() {
    let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        parse_args(&args(&["--recommend"])),
        ParsedArgs::Recommend { json: false }
    );
    assert_eq!(
        parse_args(&args(&["--recommend", "--json"])),
        ParsedArgs::Recommend { json: true }
    );
    assert_eq!(
        parse_args(&args(&["--json", "--recommend"])),
        ParsedArgs::Recommend { json: true }
    );
    assert!(matches!(parse_args(&args(&["--json"])), ParsedArgs::Error(_)));
    assert!(matches!(
        parse_args(&args(&["--recommend", "--set", "200000"])),
        ParsedArgs::Error(_)
    ));
}

#[test]
fn missing_set_flag_and_bare_help_are_distinct() {
    assert!(matches!(parse_args(&[]), ParsedArgs::Error(_)));
    assert!(matches!(
        parse_args(&["--help".to_string()]),
        ParsedArgs::Help
    ));
    assert!(matches!(parse_args(&["-h".to_string()]), ParsedArgs::Help));
}

// ── `run_core` — the write, and the refusals ────────────────────────────────

/// A fresh repo (no `.claude/` at all) gains the local settings file, the
/// value, and the ignore rule in one step.
#[test]
fn fresh_repo_gains_file_value_and_ignore_rule_in_one_step() {
    let dir = fixture();
    let code = run_core(dir.path(), 200_000).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    let settings = read_local_settings(dir.path());
    assert_eq!(settings[WINDOW_KEY], json!(200_000));

    let gi = gitignore_contents(dir.path());
    assert!(
        gi.lines().any(|l| l == SETTINGS_LOCAL_REL || l == format!("/{SETTINGS_LOCAL_REL}")),
        "expected a rule covering {SETTINGS_LOCAL_REL} in:\n{gi}"
    );
}

/// Covers AE15: a pre-existing window value survives the verb, with a report
/// instead of a write — the file's bytes (not just the value) are untouched.
#[test]
fn preexisting_window_survives_with_a_report_not_a_write() {
    let dir = fixture();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    let before = format!("{{\n  \"{WINDOW_KEY}\": 500000\n}}\n");
    fs::write(dir.path().join(SETTINGS_LOCAL_REL), &before).unwrap();

    let code = run_core(dir.path(), 300_000).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "a no-op report is still a success");

    let after = fs::read_to_string(dir.path().join(SETTINGS_LOCAL_REL)).unwrap();
    assert_eq!(after, before, "an existing window must not be touched at all");
}

/// The write is a load-modify-write over the whole settings object: unrelated
/// keys already in the local file (permissions, env, ...) survive alongside
/// the newly-inserted window.
#[test]
fn unrelated_keys_in_the_local_file_survive_the_write() {
    let dir = fixture();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    fs::write(
        dir.path().join(SETTINGS_LOCAL_REL),
        r#"{"permissions":{"allow":["Bash(git *)"]}}"#,
    )
    .unwrap();

    run_core(dir.path(), 300_000).unwrap();

    let settings = read_local_settings(dir.path());
    assert_eq!(settings[WINDOW_KEY], json!(300_000));
    assert_eq!(settings["permissions"]["allow"][0], json!("Bash(git *)"));
}

/// R31: the git-tracked `.claude/settings.json` is never written — its bytes
/// are identical before and after, even though it also happens to already
/// carry an `autoCompactWindow` (which only ever governs the tracked file,
/// not the local one this verb writes).
#[test]
fn tracked_settings_file_is_byte_identical_before_and_after() {
    let dir = fixture();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    let tracked_path = dir.path().join(".claude/settings.json");
    let tracked_before = r#"{"autoCompactWindow":300000,"otherKey":true}"#;
    fs::write(&tracked_path, tracked_before).unwrap();

    run_core(dir.path(), 200_000).unwrap();

    let tracked_after = fs::read(&tracked_path).unwrap();
    assert_eq!(tracked_after, tracked_before.as_bytes());
    // And the local file got the write the tracked one never sees.
    let local = read_local_settings(dir.path());
    assert_eq!(local[WINDOW_KEY], json!(200_000));
}

/// A local settings file that exists but is not valid JSON is refused rather
/// than rebuilt from nothing — this file is not ss-magic's own, and silently
/// discarding whatever a person or the harness already put there would be
/// exactly the kind of clobber R31 rules out.
#[test]
fn malformed_local_settings_json_refuses_rather_than_clobbering() {
    let dir = fixture();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    let before = "{ this is not json";
    fs::write(dir.path().join(SETTINGS_LOCAL_REL), before).unwrap();

    let code = run_core(dir.path(), 200_000).unwrap();
    assert_ne!(exit_code_to_u8(code), 0, "malformed JSON must not report success");

    let after = fs::read_to_string(dir.path().join(SETTINGS_LOCAL_REL)).unwrap();
    assert_eq!(after, before, "the malformed file must be left exactly as it was");
}

/// A local settings file whose top-level JSON value is valid but not an
/// object (here: an array) has nowhere to insert a key without discarding
/// what is there, so this refuses the same way malformed JSON does.
#[test]
fn non_object_top_level_value_refuses_rather_than_clobbering() {
    let dir = fixture();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    let before = "[1,2,3]";
    fs::write(dir.path().join(SETTINGS_LOCAL_REL), before).unwrap();

    let code = run_core(dir.path(), 200_000).unwrap();
    assert_ne!(exit_code_to_u8(code), 0);

    let after = fs::read_to_string(dir.path().join(SETTINGS_LOCAL_REL)).unwrap();
    assert_eq!(after, before);
}

/// Running twice on a fresh repo: the first call writes, the second sees its
/// own value already there and reports instead of writing again — this
/// verb's own write is not exempt from R31 just because it made the value
/// itself.
#[test]
fn running_twice_the_second_call_reports_instead_of_rewriting() {
    let dir = fixture();
    run_core(dir.path(), 200_000).unwrap();
    let after_first = fs::read_to_string(dir.path().join(SETTINGS_LOCAL_REL)).unwrap();

    let code = run_core(dir.path(), 900_000).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    let after_second = fs::read_to_string(dir.path().join(SETTINGS_LOCAL_REL)).unwrap();
    assert_eq!(after_second, after_first, "the second call must not rewrite the file");
}

/// The ignore rule is idempotent: a repo whose `.gitignore` already covers
/// `.claude/settings.local.json` gains no duplicate line on a second run.
#[test]
fn ignore_rule_is_not_duplicated_on_a_second_run() {
    let dir = fixture();
    run_core(dir.path(), 200_000).unwrap();
    let gi_after_first = gitignore_contents(dir.path());

    // A second run on a DIFFERENT repo state (window already set) still goes
    // through the same gitignore check path via `run_core`; call it again to
    // confirm the rule is not appended twice.
    run_core(dir.path(), 200_000).unwrap();
    let gi_after_second = gitignore_contents(dir.path());
    assert_eq!(gi_after_second, gi_after_first);
}

// ── The write is atomic, not a bare `fs::write` ─────────────────────────────

/// `write_settings_object` must replace the file via a temp-file-then-rename,
/// not an in-place truncate — a rename swaps the directory entry for a new
/// inode, while a bare `fs::write` over an existing file keeps the same one.
/// Comparing the inode before and after a second write is a deterministic way
/// to tell the two apart without needing to catch an actual crash mid-write.
#[test]
fn the_settings_file_is_replaced_by_a_rename_not_an_in_place_truncate() {
    use std::os::unix::fs::MetadataExt;

    let dir = fixture();
    let path = dir.path().join(SETTINGS_LOCAL_REL);
    let mut settings = Map::new();
    settings.insert(WINDOW_KEY.to_string(), json!(200_000));
    write_settings_object(&path, &settings).unwrap();
    let ino_before = fs::metadata(&path).unwrap().ino();

    settings.insert("otherKey".to_string(), json!(true));
    write_settings_object(&path, &settings).unwrap();
    let ino_after = fs::metadata(&path).unwrap().ino();

    assert_ne!(
        ino_before, ino_after,
        "the settings file must be replaced via rename (new inode), not truncated in place"
    );
}

/// A rewrite preserves whatever mode the file already had, rather than
/// resetting it to whatever a fresh temp file happens to get by default.
#[test]
fn a_rewrite_preserves_the_files_existing_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fixture();
    let path = dir.path().join(SETTINGS_LOCAL_REL);
    let mut settings = Map::new();
    settings.insert(WINDOW_KEY.to_string(), json!(200_000));
    write_settings_object(&path, &settings).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    settings.insert("otherKey".to_string(), json!(true));
    write_settings_object(&path, &settings).unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640, "a rewrite must preserve the existing file's mode");
}


// ── The heuristic (R25) ───────────────────────────────────────────────────────

/// 1.25 x 80,000 is exactly 100,000, and rounding UP to the next 10,000 must
/// leave an exact multiple alone. Float arithmetic would make it
/// 100,000.00000001 and push it to 110,000, which is why the implementation
/// is integer-only; this pins that.
#[test]
fn an_exact_multiple_of_ten_thousand_is_not_rounded_up() {
    assert_eq!(recommend(&[80_000]).tokens, Some(100_000));
    assert_eq!(recommend(&[160_000]).tokens, Some(200_000));
    assert_eq!(recommend(&[400_000]).tokens, Some(500_000));
}

/// Anything past a multiple rounds up to the next one, never down — the
/// window is headroom, and rounding down would eat it.
#[test]
fn a_fraction_of_ten_thousand_rounds_up_never_down() {
    // AE8's arithmetic: 1.25 x 210,400 = 263,000 -> 270,000.
    assert_eq!(recommend(&[210_400]).tokens, Some(270_000));
    // 1.25 x 100,001 = 125,001.25 -> 130,000.
    assert_eq!(recommend(&[100_001]).tokens, Some(130_000));
    // One token past an exact multiple.
    assert_eq!(recommend(&[80_001]).tokens, Some(110_000));
}

/// The harness accepts 100,000-1,000,000 and nothing else; a recommendation
/// outside that would be refused by `--set` itself.
#[test]
fn the_recommendation_is_clamped_to_the_harness_range() {
    assert_eq!(recommend(&[900_000]).tokens, Some(1_000_000), "1.125M clamps to 1M");
    assert_eq!(recommend(&[5_000_000]).tokens, Some(1_000_000));
    assert_eq!(recommend(&[10]).tokens, Some(100_000), "13 rounds to 10k, clamps to 100k");
    assert_eq!(recommend(&[1]).tokens, Some(100_000));
}

/// Three or more sessions are `high`, one or two `low`, none gives no number
/// and says so.
#[test]
fn confidence_follows_the_session_count() {
    let none = recommend(&[]);
    assert_eq!(none.tokens, None);
    assert_eq!(none.confidence, None);
    assert_eq!(none.sessions, 0);
    assert!(
        none.note.as_deref().unwrap().contains("no recorded sessions"),
        "{none:?}"
    );

    assert_eq!(recommend(&[150_000]).confidence, Some("low"));
    assert_eq!(recommend(&[150_000, 150_000]).confidence, Some("low"));
    assert_eq!(recommend(&[150_000, 150_000, 150_000]).confidence, Some("high"));
    let twenty: Vec<u64> = (0..20).map(|i| 100_000 + i).collect();
    assert_eq!(recommend(&twenty).confidence, Some("high"));
    assert_eq!(recommend(&twenty).sessions, 20);
}

/// The number rests on the LARGEST peak, wherever it sits in the list — not
/// the newest, not the mean — and the basis names both the count and the
/// peak so a reader can check the arithmetic.
#[test]
fn the_largest_peak_wins_and_the_basis_names_it() {
    let rec = recommend(&[143_000, 151_200, 96_000, 210_400, 180_000]);
    assert_eq!(rec.tokens, Some(270_000));
    assert_eq!(rec.max_peak_context_tokens, Some(210_400));
    assert_eq!(rec.sessions, 5);
    assert!(rec.basis.contains("5 "), "{}", rec.basis);
    assert!(rec.basis.contains("210400"), "{}", rec.basis);
    assert_eq!(rec.note, None);
}

// ── The report: fixtures ──────────────────────────────────────────────────────

/// A main checkout, canonical.
fn repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

/// A ledger row with only the fields the recommendation reads.
fn ledger_row(session_id: &str, root: Option<&str>, ts: u64, peak: Option<u64>) -> Row {
    Row {
        session_id: session_id.to_string(),
        at: format_rfc3339(ts),
        ts,
        root: root.map(str::to_string),
        also_roots: Vec::new(),
        branch: None,
        files: 1,
        bytes: 1,
        tokens: Tokens::default(),
        cost_usd: 0.0,
        basis: Basis::Table,
        harness_cost_usd: None,
        main_table_usd: 0.0,
        sub_table_usd: 0.0,
        price_table: None,
        unpriced_models: Vec::new(),
        peak_context_tokens: peak,
    }
}

/// A store holding exactly `rows` in its ledger.
fn store_with(rows: &[Row]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::new();
    for row in rows {
        body.push_str(&serde_json::to_string(row).unwrap());
        body.push('\n');
    }
    fs::write(dir.path().join(LEDGER_FILE_NAME), body).unwrap();
    dir
}

/// Sources pointing at a fake home directory's `settings.json`, with the
/// process environment reporting nothing.
fn sources_with_home(home: &Path) -> Sources {
    Sources {
        env_override: None,
        user_settings: Some(home.join("settings.json")),
        managed_settings: None,
    }
}

/// Every path under `dir` with its size and mtime — the "nothing was written"
/// snapshot. `.git/` is skipped for the reason `status`'s tests give: git's
/// own probes refresh the index behind a lock file that has nothing to do
/// with what is being asserted.
fn snapshot(dir: &Path) -> Vec<(String, u64, SystemTime)> {
    let mut out: Vec<(String, u64, SystemTime)> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != std::ffi::OsStr::new(".git"))
        .filter_map(Result::ok)
        .map(|e| {
            let meta = e.metadata().unwrap();
            (
                e.path().display().to_string(),
                meta.len(),
                meta.modified().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

// ── AE7: no sessions ──────────────────────────────────────────────────────────

/// With no row for this repository carrying a peak, the report still shows the
/// override findings and the windows, says there is nothing recorded, prints
/// the generic range guidance and the `--set` syntax — and never a made-up
/// number.
#[test]
fn ae7_no_recorded_sessions_degrades_to_guidance() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    let store = store_with(&[]);

    let report = recommend_report(
        &root,
        Some(&root),
        Some(store.path()),
        &sources_with_home(home.path()),
    );

    assert!(!report.override_.set);
    assert!(!report.windows.configured);
    assert_eq!(report.recommendation.tokens, None);
    assert!(report
        .recommendation
        .note
        .as_deref()
        .unwrap()
        .contains("no recorded sessions for this repository yet"));
    assert!(report.recommendation.basis.contains("100000"));
    assert!(report.recommendation.basis.contains("1000000"));
    assert_eq!(report.command.value, None);
    assert!(report.command.note.as_deref().unwrap().contains("--set <TOKENS>"));

    let text = render_text(&report);
    assert!(text.contains("no recorded sessions for this repository yet"), "{text}");
    assert!(text.contains("--set <TOKENS>"), "{text}");
}

// ── AE8: recorded sessions ────────────────────────────────────────────────────

/// Five rows for this repository with peaks 143,000 / 151,200 / 96,000 /
/// 210,400 / 180,000: 1.25 x 210,400 = 263,000, rounded up to 270,000, high
/// confidence, and the basis names five sessions and the peak.
#[test]
fn ae8_five_recorded_sessions_recommend_270k_with_high_confidence() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    let root_s = root.to_string_lossy().into_owned();
    let peaks = [143_000, 151_200, 96_000, 210_400, 180_000];
    let rows: Vec<Row> = peaks
        .iter()
        .enumerate()
        .map(|(i, peak)| ledger_row(&format!("s-{i}"), Some(&root_s), 1_000 + i as u64, Some(*peak)))
        .collect();
    let store = store_with(&rows);

    let report = recommend_report(
        &root,
        Some(&root),
        Some(store.path()),
        &sources_with_home(home.path()),
    );

    let rec = &report.recommendation;
    assert_eq!(rec.tokens, Some(270_000));
    assert_eq!(rec.confidence, Some("high"));
    assert_eq!(rec.sessions, 5);
    assert_eq!(rec.max_peak_context_tokens, Some(210_400));
    assert_eq!(
        report.command.value.as_deref(),
        Some("ss-magic plugin compact-window --set 270000")
    );
    let text = render_text(&report);
    assert!(text.contains("270000 tokens (confidence high)"), "{text}");
    assert!(text.contains("--set 270000"), "{text}");
}

/// A legacy row (no peak), a row from another repository and a row whose
/// worktree is gone are all left out of the population, so they neither
/// raise the number nor inflate the count.
#[test]
fn rows_that_do_not_qualify_are_left_out_of_the_population() {
    let (_dir, root) = repo();
    let (_other_dir, other) = repo();
    let home = tempfile::tempdir().unwrap();
    let root_s = root.to_string_lossy().into_owned();
    let other_s = other.to_string_lossy().into_owned();
    let store = store_with(&[
        ledger_row("s-mine", Some(&root_s), 1_000, Some(120_000)),
        ledger_row("s-legacy", Some(&root_s), 1_001, None),
        ledger_row("s-theirs", Some(&other_s), 1_002, Some(900_000)),
        ledger_row("s-gone", Some("/nonexistent/worktree/for/this/test"), 1_003, Some(900_000)),
    ]);

    let report = recommend_report(
        &root,
        Some(&root),
        Some(store.path()),
        &sources_with_home(home.path()),
    );

    assert_eq!(report.recommendation.sessions, 1);
    assert_eq!(report.recommendation.max_peak_context_tokens, Some(120_000));
    assert_eq!(report.recommendation.tokens, Some(150_000));
    assert_eq!(report.recommendation.confidence, Some("low"));
}

// ── AE9: the override ─────────────────────────────────────────────────────────

/// The user's global `settings.json` carries the override in its `env` block
/// and the process environment does not (the terminal case). The report
/// names that file as the location, advises removing the key by hand, says
/// this tool never edits that file, and still hands over the `--set` syntax.
#[test]
fn ae9_override_in_the_users_global_settings_is_named_and_left_alone() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    let user_settings = home.path().join("settings.json");
    let before = r#"{"env":{"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE":"60"},"otherKey":true}"#;
    fs::write(&user_settings, before).unwrap();
    let store = store_with(&[]);

    let report = recommend_report(
        &root,
        Some(&root),
        Some(store.path()),
        &sources_with_home(home.path()),
    );

    assert!(report.override_.set);
    assert_eq!(
        report.override_.locations,
        vec![OverrideLocation {
            location: user_settings.display().to_string(),
            value: "60".to_string(),
        }]
    );
    let note = &report.override_.note;
    assert!(note.contains("by hand"), "{note}");
    assert!(note.contains("never edits"), "{note}");
    assert!(report.command.note.as_deref().unwrap().contains("--set"));
    assert!(!report.windows.configured);

    let text = render_text(&report);
    assert!(text.contains(&user_settings.display().to_string()), "{text}");
    assert!(text.contains("by hand"), "{text}");
    assert!(text.contains("never edits"), "{text}");

    assert_eq!(
        fs::read_to_string(&user_settings).unwrap(),
        before,
        "the user's settings file must be byte-identical afterwards"
    );
}

/// The process environment is a location like any other — it is what a hook
/// sees — and a value found in more than one place is listed in every place.
#[test]
fn the_override_is_found_in_the_process_environment_and_in_every_file_that_sets_it() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join("settings.json"),
        r#"{"env":{"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE":60}}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(
        root.join(SETTINGS_PROJECT_REL),
        r#"{"env":{"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE":"45"}}"#,
    )
    .unwrap();
    let mut sources = sources_with_home(home.path());
    sources.env_override = Some("60".to_string());

    let report = detect_override(&root, &sources);

    assert!(report.set);
    let locations: Vec<(&str, &str)> = report
        .locations
        .iter()
        .map(|l| (l.location.as_str(), l.value.as_str()))
        .collect();
    assert_eq!(locations[0], ("process environment", "60"));
    assert_eq!(locations.len(), 3, "{locations:?}");
    assert!(locations[1].0.ends_with("settings.json"));
    assert_eq!(locations[1].1, "60", "a JSON number renders as its digits");
    assert!(locations[2].0.ends_with(SETTINGS_PROJECT_REL));
    assert_eq!(locations[2].1, "45");
}

/// When nothing sets it, the report says so and lists every place it looked,
/// so "not set" is a finding rather than an absence.
#[test]
fn an_absent_override_lists_where_the_search_went() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    let mut sources = sources_with_home(home.path());
    sources.managed_settings = Some(PathBuf::from("/nonexistent/managed-settings.json"));

    let report = detect_override(&root, &sources);

    assert!(!report.set);
    assert!(report.locations.is_empty());
    assert!(report.note.contains("not set"), "{}", report.note);
    assert_eq!(report.searched.len(), 5, "{:?}", report.searched);
    assert_eq!(report.searched[0], "process environment");
    assert!(report.searched.iter().any(|s| s.contains(SETTINGS_LOCAL_REL)));
    assert!(report.searched.iter().any(|s| s.contains("managed-settings.json")));
}

// ── The windows ───────────────────────────────────────────────────────────────

/// Each file's value is read as it is; `configured` is true when either has
/// one.
#[test]
fn window_values_are_read_from_both_files_and_either_counts_as_configured() {
    let (_dir, root) = repo();
    assert!(!window_configured(&root));

    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(root.join(SETTINGS_PROJECT_REL), r#"{"autoCompactWindow":300000}"#).unwrap();
    assert!(window_configured(&root), "the tracked file alone configures a window");
    let project = read_window(&root, SETTINGS_PROJECT_REL);
    assert_eq!(project.value, Some(json!(300_000)));
    assert_eq!(project.note, None);
    let local = read_window(&root, SETTINGS_LOCAL_REL);
    assert_eq!(local.value, None);
    assert!(local.note.as_deref().unwrap().contains("not set"), "{local:?}");

    fs::write(root.join(SETTINGS_LOCAL_REL), r#"{"autoCompactWindow":"200k"}"#).unwrap();
    let local = read_window(&root, SETTINGS_LOCAL_REL);
    assert_eq!(local.value, Some(json!("200k")), "the value is reported as the file has it");
}

/// A settings file that is not valid JSON, or not an object, is reported as
/// such — a note, never a failure, and never read as "configured".
#[test]
fn a_malformed_settings_file_is_a_note_not_a_failure() {
    let (_dir, root) = repo();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(root.join(SETTINGS_LOCAL_REL), "{ not json").unwrap();
    fs::write(root.join(SETTINGS_PROJECT_REL), "[1,2]").unwrap();

    let local = read_window(&root, SETTINGS_LOCAL_REL);
    assert_eq!(local.value, None);
    assert!(local.note.as_deref().unwrap().contains("not valid JSON"), "{local:?}");
    let project = read_window(&root, SETTINGS_PROJECT_REL);
    assert_eq!(project.value, None);
    assert!(project.note.as_deref().unwrap().contains("not a JSON object"), "{project:?}");
    assert!(!window_configured(&root));

    // And the override search over the same broken files is a non-finding.
    let home = tempfile::tempdir().unwrap();
    assert!(!detect_override(&root, &sources_with_home(home.path())).set);
}

// ── `--json` ──────────────────────────────────────────────────────────────────

/// The machine-readable shape keeps `status`'s rule: a `null` always has a
/// non-`null` `note` beside it. Checked on the report with the most nulls in
/// it — nothing recorded, nothing configured, nothing set.
#[test]
fn json_carries_a_note_beside_every_null() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    let store = store_with(&[]);
    let report = recommend_report(
        &root,
        Some(&root),
        Some(store.path()),
        &sources_with_home(home.path()),
    );
    let json = serde_json::to_value(&report).unwrap();

    assert_eq!(json["schema"], json!(RECOMMEND_SCHEMA_VERSION));
    for (value, note) in [
        (&json["recommendation"]["tokens"], &json["recommendation"]["note"]),
        (&json["command"]["value"], &json["command"]["note"]),
        (&json["windows"]["local"]["value"], &json["windows"]["local"]["note"]),
        (&json["windows"]["project"]["value"], &json["windows"]["project"]["note"]),
    ] {
        assert!(value.is_null());
        assert!(note.is_string(), "a null must carry a note: {json:#}");
    }
    assert!(json["override"]["note"].is_string());
    assert!(json["override"]["searched"].as_array().unwrap().len() >= 3);
}

// ── R28: the report never writes ──────────────────────────────────────────────

/// The whole read-only half — the report, its text and its JSON — leaves the
/// repository tree, the fake home directory and the store byte-for-byte as
/// they were: same paths, same sizes, same mtimes. In particular
/// `.claude/settings.local.json` is neither created nor modified, and the
/// override key stays exactly where it was.
#[test]
fn recommend_never_creates_or_modifies_anything() {
    let (_dir, root) = repo();
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join("settings.json"),
        r#"{"env":{"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE":"60"}}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(root.join(SETTINGS_PROJECT_REL), r#"{"permissions":{}}"#).unwrap();
    let root_s = root.to_string_lossy().into_owned();
    let store = store_with(&[ledger_row("s-1", Some(&root_s), 1_000, Some(150_000))]);

    let repo_before = snapshot(&root);
    let home_before = snapshot(home.path());
    let store_before = snapshot(store.path());
    assert!(!root.join(SETTINGS_LOCAL_REL).exists());

    let report = recommend_report(
        &root,
        Some(&root),
        Some(store.path()),
        &sources_with_home(home.path()),
    );
    let _ = render_text(&report);
    let _ = serde_json::to_string_pretty(&report).unwrap();

    assert_eq!(snapshot(&root), repo_before, "the repository tree changed");
    assert_eq!(snapshot(home.path()), home_before, "the home directory changed");
    assert_eq!(snapshot(store.path()), store_before, "the store changed");
    assert!(
        !root.join(SETTINGS_LOCAL_REL).exists(),
        "--recommend must never create the local settings file"
    );
    assert!(
        fs::read_to_string(home.path().join("settings.json"))
            .unwrap()
            .contains(OVERRIDE_ENV),
        "the override is the person's to remove, never this tool's"
    );
    assert!(report.override_.set && report.recommendation.tokens.is_some());
}

// ── The `enable` tip (R27) ────────────────────────────────────────────────────

/// One line pointing at `--recommend` when no window is configured; nothing
/// at all once one is, in either file.
#[test]
fn the_enable_tip_is_present_only_when_no_window_is_configured() {
    let (_dir, root) = repo();
    let tip = enable_tip(&root).expect("no window configured: the tip is due");
    assert!(tip.contains("compact-window --recommend"), "{tip}");
    assert!(tip.to_lowercase().contains("nothing is written"), "{tip}");

    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(root.join(SETTINGS_LOCAL_REL), r#"{"autoCompactWindow":200000}"#).unwrap();
    assert_eq!(enable_tip(&root), None, "a configured window silences the tip");

    fs::remove_file(root.join(SETTINGS_LOCAL_REL)).unwrap();
    fs::write(root.join(SETTINGS_PROJECT_REL), r#"{"autoCompactWindow":300000}"#).unwrap();
    assert_eq!(enable_tip(&root), None, "the tracked file counts too");
}
