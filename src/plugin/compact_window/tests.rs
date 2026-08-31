//! `run_core` (the filesystem half) and `parse_args` (the pure argv half),
//! exercised separately per the module's own split.

use std::fs;

use serde_json::json;

use super::*;
use crate::tests::support::exit_code_to_u8;

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
