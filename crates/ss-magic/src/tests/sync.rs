use crate::*;
use std::fs;

use ss_magic_core::testutil::{
    exit_code_to_u8, git_run, init_main_repo, make_worktree, write_file, write_magic,
};

// ── Test: patterns from overlaid config copy into the worktree ─────────

/// Literal file pattern copies from main into the worktree.
#[test]
fn sync_literal_file_copies_into_worktree() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "FOO=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must succeed");
    assert!(
        wt_root.join(".env").is_file(),
        ".env must be copied into worktree"
    );
    let body = fs::read_to_string(wt_root.join(".env")).unwrap();
    assert_eq!(body, "FOO=1\n");
}

/// Glob pattern (`**/.dev.vars`) copies matching files at any depth.
#[test]
fn sync_glob_pattern_copies_at_depth() {
    let main = init_main_repo("main");
    write_magic(main.path(), &["**/.dev.vars"]);
    write_file(main.path(), "apps/api/.dev.vars", "SECRET=x\n");
    write_file(main.path(), "apps/web/.dev.vars", "OTHER=y\n");

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);
    assert!(wt_root.join("apps/api/.dev.vars").is_file());
    assert!(wt_root.join("apps/web/.dev.vars").is_file());
}

/// `**` depth: pattern matches at 3+ nesting levels.
#[test]
fn sync_double_glob_matches_deep_paths() {
    let main = init_main_repo("main");
    write_magic(main.path(), &["**/.env"]);
    write_file(main.path(), "a/b/c/.env", "DEEP=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);
    assert!(wt_root.join("a/b/c/.env").is_file(), "deep path must copy");
}

/// node_modules and .venv matches are silently excluded; other files copy.
#[test]
fn sync_excludes_node_modules_and_venv() {
    let main = init_main_repo("main");
    write_magic(main.path(), &["**/.env"]);
    write_file(main.path(), "apps/api/.env", "ok\n");
    write_file(main.path(), "node_modules/pkg/.env", "drop\n");
    write_file(main.path(), ".venv/lib/.env", "drop\n");

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);
    assert!(wt_root.join("apps/api/.env").is_file());
    assert!(!wt_root.join("node_modules/pkg/.env").exists());
    assert!(!wt_root.join(".venv/lib/.env").exists());
}

/// magic.local.json overlay: patterns from both files are unioned.
#[test]
fn sync_uses_overlaid_config() {
    let main = init_main_repo("main");
    // magic.json has .env; magic.local.json adds .dev.vars
    write_magic(main.path(), &["**/.env"]);
    let local_body = r#"{"files":["**/.dev.vars"]}"#;
    fs::write(
        main.path().join(".superset/magic.local.json"),
        local_body,
    )
    .unwrap();
    write_file(main.path(), "apps/api/.env", "ENV=1\n");
    write_file(main.path(), "apps/api/.dev.vars", "VARS=2\n");

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);
    assert!(wt_root.join("apps/api/.env").is_file());
    assert!(wt_root.join("apps/api/.dev.vars").is_file());
}

/// Empty files list → success, nothing copied.
#[test]
fn sync_empty_files_succeeds_with_nothing_copied() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[]);

    let (_wt, wt_root) = make_worktree(main.path());
    let mut events: Vec<sync::apply::Event> = Vec::new();
    let code = sync_core(&wt_root, false, |e| events.push(e.clone())).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);
    assert!(events.is_empty(), "no events when files is empty");
}

// ── Failure-mode tests ─────────────────────────────────────────────────

/// No magic.json in main checkout → non-zero exit, error names the path.
#[test]
fn sync_no_magic_json_is_hard_error() {
    let main = init_main_repo("main");
    // Deliberately do NOT write magic.json.

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_ne!(exit_code_to_u8(code), 0, "must exit non-zero when magic.json absent");
}

/// Malformed magic.json → non-zero exit.
#[test]
fn sync_malformed_magic_json_is_hard_error() {
    let main = init_main_repo("main");
    fs::create_dir_all(main.path().join(".superset")).unwrap();
    fs::write(main.path().join(".superset/magic.json"), "{bad json").unwrap();

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "must exit non-zero on malformed magic.json"
    );
}

/// Malformed magic.local.json → non-zero exit (no silent fallback).
#[test]
fn sync_malformed_magic_local_json_is_hard_error() {
    let main = init_main_repo("main");
    write_magic(main.path(), &["**/.env"]);
    fs::write(
        main.path().join(".superset/magic.local.json"),
        "{not json",
    )
    .unwrap();

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "must exit non-zero on malformed magic.local.json"
    );
}

/// When cwd is not inside any git repository, sync_core must exit non-zero.
#[test]
fn sync_outside_git_repo_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    // No git init — not a repo.
    let code = sync_core(dir.path(), false, |_| {}).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "must exit non-zero when not in a git repo"
    );
}

// ── Pre-copy backup pass (unified sync, Task 5) ─────────────────────────

/// Forward sync backs up the worktree's pre-overwrite bytes under
/// `.superset/backups/<ts>/worktree/<rel>` before copying main's version in,
/// unless `--no-backup` is set.
#[test]
fn sync_backs_up_overwritten_worktree_file_by_default() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "NEW=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    write_file(&wt_root, ".env", "OLD=0\n");

    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must succeed");
    assert_eq!(
        fs::read_to_string(wt_root.join(".env")).unwrap(),
        "NEW=1\n",
        "worktree must end up with main's bytes"
    );

    let backups_dir = wt_root.join(".superset/backups");
    let batch = fs::read_dir(&backups_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .expect("one backup batch dir must exist under .superset/backups");
    // Forward sync overwrites the worktree side, so the backup lands under the
    // shared `<ts>/worktree/<rel>` namespace (`backup_rel_path`).
    let backed_up = fs::read_to_string(batch.join("worktree").join(".env")).unwrap();
    assert_eq!(
        backed_up, "OLD=0\n",
        "the backup must hold the pre-overwrite (OLD) bytes"
    );
}

/// `--no-backup` (`no_backup: true`) skips the pre-copy backup pass entirely —
/// no `.superset/backups` dir is created at all.
#[test]
fn sync_no_backup_flag_skips_backup() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "NEW=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    write_file(&wt_root, ".env", "OLD=0\n");

    let code = sync_core(&wt_root, true, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must succeed");
    assert_eq!(
        fs::read_to_string(wt_root.join(".env")).unwrap(),
        "NEW=1\n",
        "worktree must still end up with main's bytes"
    );
    assert!(
        !wt_root.join(".superset/backups").exists(),
        "no_backup must skip the backup pass, so no backups dir is created"
    );
}

/// A file matched by `magic.json` but absent from the worktree is a fresh
/// create — nothing to lose, so no backup is written for it.
#[test]
fn sync_new_file_creates_no_backup() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "FOO=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    // Worktree deliberately has no `.env` yet.

    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must succeed");
    assert!(
        wt_root.join(".env").is_file(),
        ".env must be created in the worktree"
    );
    assert!(
        !wt_root.join(".superset/backups").exists(),
        "a fresh create has nothing to back up, so no backups dir is created"
    );
}

/// After a default (backing-up) sync, `.superset/backups` is gitignored in
/// the worktree so a mistaken overwrite's recovery copy is never committed.
#[test]
fn sync_backup_dir_is_gitignored_in_worktree() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "NEW=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    write_file(&wt_root, ".env", "OLD=0\n");

    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must succeed");
    assert!(
        git::is_ignored(&wt_root, Path::new(".superset/backups")).unwrap(),
        ".superset/backups must be gitignored in the worktree after a backing-up sync"
    );
}

// ── Local install (no magic.json) ──────────────────────────────────────

/// AE4: a local install in main (only `magic.local.json`, no `magic.json`) is
/// a valid sync source. `sync_core` in a fresh worktree copies the configured
/// `.env` AND both local files, so the worktree carries the install too.
#[test]
fn ae4_sync_from_a_local_install_copies_patterns_and_local_files() {
    let main = init_main_repo("main");
    let main_root = main.path().canonicalize().unwrap();
    write_file(&main_root, ".env", "FOO=1\n");
    let code = workspace::local_install::run_local_init_noninteractive(
        &main_root,
        &[".env".to_string()],
    )
    .unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "local install must succeed");
    assert!(!main_root.join(".superset/magic.json").exists());

    let (_wt, wt_root) = make_worktree(&main_root);
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must accept a local install");
    assert_eq!(fs::read_to_string(wt_root.join(".env")).unwrap(), "FOO=1\n");
    for rel in [".superset/magic.local.json", ".superset/config.local.json"] {
        assert_eq!(
            fs::read(wt_root.join(rel)).unwrap(),
            fs::read(main_root.join(rel)).unwrap(),
            "{rel} must be copied into the worktree"
        );
    }
}

/// Neither `magic.json` nor `magic.local.json` in main: still exit 1.
#[test]
fn sync_without_any_pattern_file_exits_1() {
    let main = init_main_repo("main");
    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 1);
}

/// The no-config error names both files a checkout can be configured by, and
/// both ways to create one.
#[test]
fn no_sync_config_message_names_both_pattern_files() {
    let msg = no_sync_config_message(std::path::Path::new("/repo"));
    assert!(msg.contains(".superset/magic.json"), "msg: {msg}");
    assert!(msg.contains(".superset/magic.local.json"), "msg: {msg}");
    assert!(msg.contains("ss-magic init --local"), "msg: {msg}");
    assert!(msg.contains("/repo"), "msg: {msg}");
}

/// KTD4 on the forward path: on a local install whose `info/exclude` lost the
/// backups rule, a backing-up `ss-magic sync` re-adds `/.superset/backups/` to
/// `info/exclude` and leaves the worktree's tracked `.gitignore` alone.
#[test]
fn forward_sync_on_local_install_readds_backups_rule_to_info_exclude() {
    let main = init_main_repo("main");
    let main_root = main.path().canonicalize().unwrap();
    write_file(&main_root, ".gitignore", "target/\n");
    git_run(&["add", ".gitignore"], &main_root);
    git_run(&["commit", "-q", "-m", "gitignore"], &main_root);
    write_file(&main_root, ".env", "NEW=1\n");
    let code = workspace::local_install::run_local_init_noninteractive(
        &main_root,
        &[".env".to_string()],
    )
    .unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "local install must succeed");

    // Drop the backups rule the install wrote.
    let exclude = git::git_common_dir(&main_root).unwrap().join("info/exclude");
    let kept: String = fs::read_to_string(&exclude)
        .unwrap()
        .lines()
        .filter(|l| *l != "/.superset/backups/")
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&exclude, kept).unwrap();

    let (_wt, wt_root) = make_worktree(&main_root);
    write_file(&wt_root, ".env", "OLD=0\n");
    assert!(!git::is_ignored(&wt_root, Path::new(".superset/backups")).unwrap());

    let code = sync_core(&wt_root, false, |_| {}).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "sync_core must succeed");

    let lines: Vec<String> = fs::read_to_string(&exclude)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(
        lines.contains(&"/.superset/backups/".to_string()),
        "the backups rule must be re-added to info/exclude, got {lines:?}"
    );
    assert_eq!(
        fs::read_to_string(wt_root.join(".gitignore")).unwrap(),
        "target/\n",
        "the worktree's tracked .gitignore must be untouched"
    );
    assert!(git::is_ignored(&wt_root, Path::new(".superset/backups")).unwrap());
}
