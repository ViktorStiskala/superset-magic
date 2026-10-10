use super::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use ss_magic_core::testutil::{
    exit_code_to_u8, git_run, init_main_repo, make_worktree, write_file, write_magic,
};
use tempfile::TempDir;

/// A fresh main repository (one commit, global excludes neutralized) and its
/// canonical root, which is what `git::main_checkout_root` answers with.
fn repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

/// Commit everything currently in the working tree.
fn commit_all(root: &Path) {
    git_run(&["add", "-A"], root);
    git_run(&["commit", "-q", "-m", "fixture"], root);
}

/// `git status --porcelain` in `root`, untrimmed (an empty string means a
/// clean working tree, including no untracked files).
fn porcelain(root: &Path) -> String {
    let out = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

/// The shared `info/exclude` of the repository whose main checkout is `root`.
fn exclude_path(root: &Path) -> PathBuf {
    root.join(".git/info/exclude")
}

/// Every file under `root` outside `.git` (relative path and bytes, sorted),
/// plus the contents of `info/exclude`: the state a refused or failed install
/// must leave exactly as it found it.
fn snapshot(root: &Path) -> (Vec<(PathBuf, Vec<u8>)>, Option<String>) {
    let mut files: Vec<(PathBuf, Vec<u8>)> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let rel = e.path().strip_prefix(root).unwrap().to_path_buf();
            (rel, fs::read(e.path()).unwrap())
        })
        .collect();
    files.sort();
    (files, fs::read_to_string(exclude_path(root)).ok())
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn local_files(root: &Path) -> Vec<String> {
    superset_files::load_magic_local_json(root)
        .unwrap()
        .unwrap()
        .files
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const DEFAULTS: [&str; 2] = [".superset/magic.local.json", ".superset/config.local.json"];

// ── Fresh install ─────────────────────────────────────────────────────────────

/// AE1 (R1, R2, R3, R5): a fresh local install in a clean repository with a
/// committed `.gitignore` writes exactly the two local files, registers the
/// sync step in the `before` form, writes no committed-install file, leaves
/// `.gitignore` untouched, and leaves `git status --porcelain` empty.
#[test]
fn ae1_fresh_install_writes_only_ignored_local_files() {
    let (_dir, root) = repo();
    write_file(&root, ".gitignore", "target/\n");
    commit_all(&root);

    let code = run_local_init_noninteractive(&root, &strings(&[".env"])).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    assert_eq!(
        local_files(&root),
        strings(&[DEFAULTS[0], DEFAULTS[1], ".env"])
    );
    assert_eq!(
        read_json(&root.join(".superset/config.local.json")),
        serde_json::json!({"setup": {"before": ["ss-magic sync"]}})
    );
    for absent in ["magic.sh", "magic.json", "config.json"] {
        assert!(
            !root.join(".superset").join(absent).exists(),
            "a local install must not write .superset/{absent}"
        );
    }
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), "target/\n");

    let exclude = fs::read_to_string(exclude_path(&root)).unwrap();
    for rule in [
        "/.superset/magic.local.json",
        "/.superset/config.local.json",
        "/.superset/backups/",
        "/.superset/.magic/",
    ] {
        assert!(
            exclude.lines().any(|l| l == rule),
            "info/exclude must carry `{rule}`, got: {exclude:?}"
        );
    }
    assert_eq!(porcelain(&root), "", "a local install must leave git status empty");
}

/// AE2 (R3, R4): a committed `config.json` with its own setup is left
/// byte-identical, and the sync step goes into `config.local.json` only.
#[test]
fn ae2_committed_config_json_stays_byte_identical() {
    let (_dir, root) = repo();
    let committed = "{\n  \"setup\": [\"bun install\"]\n}\n";
    write_file(&root, ".superset/config.json", committed);
    commit_all(&root);

    let code = run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    assert_eq!(
        fs::read_to_string(root.join(".superset/config.json")).unwrap(),
        committed
    );
    let local = read_json(&root.join(".superset/config.local.json"));
    assert_eq!(local["setup"]["before"], serde_json::json!(["ss-magic sync"]));
    assert_eq!(porcelain(&root), "");
}

/// AE3 (R4): an existing `config.local.json` in the replace (array) form gains
/// the entry first, keeps every other key's value, and a second run does not
/// rewrite the file at all.
#[test]
fn ae3_existing_config_local_json_is_merged_then_left_alone() {
    let (_dir, root) = repo();
    write_file(
        &root,
        ".superset/config.local.json",
        r#"{"setup": ["./mine.sh"], "teardown": {"after": ["x"]}, "note": 1}"#,
    );

    run_local_init_noninteractive(&root, &[]).unwrap();
    let after_first = fs::read_to_string(root.join(".superset/config.local.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&after_first).unwrap();
    assert_eq!(value["setup"], serde_json::json!(["ss-magic sync", "./mine.sh"]));
    assert_eq!(value["teardown"], serde_json::json!({"after": ["x"]}));
    assert_eq!(value["note"], serde_json::json!(1));

    // Reformat the file by hand: a second run that rewrote it would restore
    // the pretty-printed shape, so unchanged bytes prove no write happened.
    let compact = serde_json::to_string(&value).unwrap();
    fs::write(root.join(".superset/config.local.json"), &compact).unwrap();
    run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".superset/config.local.json")).unwrap(),
        compact
    );
}

/// AE7 (R1): run from a linked worktree, the install lands in the MAIN
/// checkout, where Superset and sync read it, and writes nothing into the
/// worktree.
#[test]
fn ae7_run_from_linked_worktree_installs_into_main() {
    let (_dir, root) = repo();
    let (_wt, wt_root) = make_worktree(&root);

    let code = run_local_init_noninteractive(&wt_root, &strings(&[".env"])).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    assert!(root.join(".superset/magic.local.json").is_file());
    assert!(root.join(".superset/config.local.json").is_file());
    assert!(
        !wt_root.join(".superset").exists(),
        "the worktree must gain no .superset/ files"
    );
    assert_eq!(porcelain(&root), "");
    assert_eq!(porcelain(&wt_root), "");
}

// ── Refusals (R9) ─────────────────────────────────────────────────────────────

/// AE6: a checkout with a committed `magic.json` refuses with exit 1 and
/// changes no file (nor `info/exclude`).
#[test]
fn ae6_committed_magic_json_refuses_and_writes_nothing() {
    let (_dir, root) = repo();
    write_magic(&root, &[".env"]);
    commit_all(&root);
    let before = snapshot(&root);

    let code = run_local_init_noninteractive(&root, &strings(&[".env"])).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
    assert_eq!(snapshot(&root), before);

    let reason = refusal(&root).unwrap().expect("must refuse");
    assert!(reason.contains("magic.json"), "names the committed install: {reason}");
}

/// A `config.json` whose setup already runs the `magic.sh` wrapper is a
/// committed install even without a `magic.json` next to it.
#[test]
fn magic_marker_in_config_json_refuses() {
    let (_dir, root) = repo();
    write_file(
        &root,
        ".superset/config.json",
        r#"{"setup": ["./.superset/magic.sh sync"]}"#,
    );
    let before = snapshot(&root);

    let code = run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
    assert_eq!(snapshot(&root), before);
}

/// A `config.json` still running the retired `setup.sh` refuses, and the
/// message points at migration rather than at a local install.
#[test]
fn setup_sh_in_config_json_refuses_pointing_at_migration() {
    let (_dir, root) = repo();
    write_file(
        &root,
        ".superset/config.json",
        r#"{"setup": ["./.superset/setup.sh"]}"#,
    );
    let before = snapshot(&root);

    let reason = refusal(&root).unwrap().expect("must refuse");
    assert!(reason.contains("migrate"), "points at migration: {reason}");

    let code = run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
    assert_eq!(snapshot(&root), before);
}

/// A TRACKED `config.local.json` refuses: rewriting it would show up in
/// `git status`, which a local install must never cause.
#[test]
fn tracked_config_local_json_refuses() {
    let (_dir, root) = repo();
    write_file(&root, ".superset/config.local.json", "{}\n");
    commit_all(&root);
    let before = snapshot(&root);

    let code = run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
    assert_eq!(snapshot(&root), before);

    let reason = refusal(&root).unwrap().expect("must refuse");
    assert!(reason.contains(".superset/config.local.json"), "{reason}");
}

/// A tracked `magic.local.json` refuses too (it holds no `magic.json`, so the
/// install-mode check alone would read it as a local install).
#[test]
fn tracked_magic_local_json_refuses() {
    let (_dir, root) = repo();
    write_file(&root, ".superset/magic.local.json", "{\"files\": []}\n");
    commit_all(&root);
    let before = snapshot(&root);

    let code = run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
    assert_eq!(snapshot(&root), before);
}

/// A tracked local file under a different capitalization refuses: on a
/// case-insensitive filesystem writing the canonical path would rewrite it,
/// and a literal pathspec does not find it. The check is name-based, so this
/// runs the same on case-sensitive CI.
#[test]
fn tracked_local_file_in_another_case_refuses() {
    for rel in [".superset/CONFIG.LOCAL.JSON", ".Superset/magic.local.json"] {
        let (_dir, root) = repo();
        write_file(&root, rel, "{}\n");
        commit_all(&root);
        let before = snapshot(&root);
        let porcelain_before = porcelain(&root);

        let code = run_local_init_noninteractive(&root, &[]).unwrap();
        assert_eq!(exit_code_to_u8(code), 1, "{rel}");
        assert_eq!(snapshot(&root), before, "{rel}");
        assert_eq!(porcelain(&root), porcelain_before, "{rel}");

        let reason = refusal(&root).unwrap().expect("must refuse");
        assert!(reason.contains(rel), "names the index spelling: {reason}");
    }
}

/// An UNTRACKED symlink at either local file refuses: the JSON writers follow
/// a link onto its target, so `config.local.json -> config.json` would
/// otherwise rewrite the tracked `config.json` while reporting nothing to
/// commit.
#[cfg(unix)]
#[test]
fn symlinked_local_file_refuses_and_leaves_the_target_alone() {
    for rel in [".superset/config.local.json", ".superset/magic.local.json"] {
        let (_dir, root) = repo();
        write_file(&root, ".superset/config.json", r#"{"setup": ["bun install"]}"#);
        commit_all(&root);
        std::os::unix::fs::symlink("config.json", root.join(rel)).unwrap();
        let before = snapshot(&root);
        let porcelain_before = porcelain(&root);

        let code = run_local_init_noninteractive(&root, &strings(&[".env"])).unwrap();
        assert_eq!(exit_code_to_u8(code), 1, "{rel}");
        assert_eq!(snapshot(&root), before, "{rel}");
        assert_eq!(porcelain(&root), porcelain_before, "{rel}");

        let reason = refusal(&root).unwrap().expect("must refuse");
        assert!(reason.contains("symlink"), "{reason}");
    }
}

/// A symlinked `.superset` directory redirects both local files, so it
/// refuses as well.
#[cfg(unix)]
#[test]
fn symlinked_superset_dir_refuses() {
    let (_dir, root) = repo();
    let elsewhere = TempDir::new().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), root.join(".superset")).unwrap();
    let before = snapshot(&root);

    let code = run_local_init_noninteractive(&root, &[]).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
    assert_eq!(snapshot(&root), before);
    assert!(fs::read_dir(elsewhere.path()).unwrap().next().is_none());
}

/// A malformed `config.local.json` is an error naming the file, and nothing –
/// neither JSON file nor `info/exclude` – changes, because every input is
/// validated before the first write.
#[test]
fn malformed_config_local_json_errors_before_any_write() {
    let (_dir, root) = repo();
    write_file(&root, ".superset/config.local.json", "{ not json");
    let before = snapshot(&root);

    let err = run_local_init_noninteractive(&root, &strings(&[".env"])).unwrap_err();
    assert!(
        format!("{err:#}").contains("config.local.json"),
        "the error names the file: {err:#}"
    );
    assert_eq!(snapshot(&root), before);
    assert!(!root.join(".superset/magic.local.json").exists());
}

/// KTD7 write order: the `info/exclude` rules are written AND verified by git
/// before either JSON file. A committed `.gitignore` negation outranks
/// `info/exclude`, so the verification fails – and because it runs first, the
/// install errors with neither local file on disk and `git status` unchanged,
/// rather than leaving an unignored `magic.local.json` showing up as untracked.
fn assert_negation_refuses_before_json_writes(gitignore: &str) {
    let (_dir, root) = repo();
    write_file(&root, ".gitignore", gitignore);
    commit_all(&root);
    let before = porcelain(&root);

    let result = run_local_init_noninteractive(&root, &strings(&[".env"]));
    assert!(
        result.is_err(),
        "a tracked negation of `{gitignore}` must fail the install, got {result:?}"
    );
    assert!(!root.join(".superset/magic.local.json").exists());
    assert!(!root.join(".superset/config.local.json").exists());
    assert_eq!(porcelain(&root), before, "git status must not change");
}

/// A tracked `.gitignore` re-including `magic.local.json` fails at its own
/// (first) rule, before any JSON file is written.
#[test]
fn ktd7_tracked_negation_of_magic_local_json_errors_before_json_writes() {
    assert_negation_refuses_before_json_writes("!/.superset/magic.local.json\n");
}

/// A tracked `.gitignore` re-including the backups tree fails at the THIRD
/// rule, after the two file rules succeeded – still before any JSON file is
/// written, which is what pins the rules loop ahead of the writes.
#[test]
fn ktd7_tracked_negation_of_backups_dir_errors_before_json_writes() {
    assert_negation_refuses_before_json_writes("!/.superset/backups/\n");
}

// ── Pattern-list semantics ────────────────────────────────────────────────────

/// Non-interactive re-runs APPEND: a new pattern lands once, after the
/// existing custom patterns, and `_comment` survives every rewrite.
#[test]
fn noninteractive_rerun_appends_once_and_keeps_custom_and_comment() {
    let (_dir, root) = repo();
    write_file(
        &root,
        ".superset/magic.local.json",
        r#"{"_comment": "keep me", "files": [".superset/magic.local.json", ".superset/config.local.json", "custom/**"]}"#,
    );

    run_local_init_noninteractive(&root, &strings(&[".env"])).unwrap();
    run_local_init_noninteractive(&root, &strings(&[".env", "extra"])).unwrap();

    assert_eq!(
        local_files(&root),
        strings(&[DEFAULTS[0], DEFAULTS[1], "custom/**", ".env", "extra"])
    );
    let raw = read_json(&root.join(".superset/magic.local.json"));
    assert_eq!(raw["_comment"], serde_json::json!("keep me"));
}

/// The interactive entry REPLACES the list with the selection, so a pattern
/// the user deselected is gone; the defaults stay first and `_comment`
/// survives. Drives the same `prepare` → `selection_files` → `commit` path the
/// interactive entry runs after its picker.
#[test]
fn interactive_selection_replaces_list_and_keeps_comment() {
    let (_dir, root) = repo();
    write_file(
        &root,
        ".superset/magic.local.json",
        r#"{"_comment": "keep me", "files": [".superset/magic.local.json", ".superset/config.local.json", ".env", "old/**"]}"#,
    );

    let prepared = prepare(&root).unwrap();
    commit(&root, prepared, selection_files(&strings(&[".env"]))).unwrap();

    assert_eq!(local_files(&root), strings(&[DEFAULTS[0], DEFAULTS[1], ".env"]));
    let raw = read_json(&root.join(".superset/magic.local.json"));
    assert_eq!(raw["_comment"], serde_json::json!("keep me"));
    assert_eq!(porcelain(&root), "");
}

/// A fresh non-interactive list is the defaults then the patterns, deduped
/// against both.
#[test]
fn appended_files_fresh_is_defaults_then_patterns() {
    assert_eq!(
        appended_files(&[], &strings(&[".env", DEFAULTS[1], ".env"])),
        strings(&[DEFAULTS[0], DEFAULTS[1], ".env"])
    );
}

/// The picker never offers the defaults (they are always written), so a
/// custom row for them cannot appear and deselecting cannot drop them.
#[test]
fn without_local_defaults_drops_only_the_defaults() {
    assert_eq!(
        without_local_defaults(&strings(&[DEFAULTS[0], ".env", DEFAULTS[1], "x/**"])),
        strings(&[".env", "x/**"])
    );
}

// ── KTD10 PATH advisory ──────────────────────────────────────────────────────

#[test]
fn resolves_on_path_finds_an_executable() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("ss-magic");
    fs::write(&bin, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let empty = tempfile::tempdir().unwrap();
    let path = std::env::join_paths([empty.path(), dir.path()]).unwrap();
    assert!(resolves_on_path(Some(&path), "ss-magic"));
}

#[test]
fn resolves_on_path_rejects_missing_or_non_executable() {
    let dir = tempfile::tempdir().unwrap();
    let path = std::env::join_paths([dir.path()]).unwrap();
    assert!(!resolves_on_path(Some(&path), "ss-magic"), "absent");
    assert!(!resolves_on_path(None, "ss-magic"), "PATH unset");

    #[cfg(unix)]
    {
        fs::write(dir.path().join("ss-magic"), "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("ss-magic"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(!resolves_on_path(Some(&path), "ss-magic"), "not executable");
    }
}
