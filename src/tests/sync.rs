use crate::*;
use crate::tests::support::git_run;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Convert `ExitCode` to u8 for assertions.
/// `ExitCode` doesn't implement `From<ExitCode> for u8`; this helper
/// works by matching against known constants.
fn exit_code_to_u8(code: ExitCode) -> u8 {
    if code == ExitCode::SUCCESS {
        0
    } else {
        // Any non-SUCCESS code is treated as non-zero. For tests that
        // assert `!= 0` this is sufficient; we only ever return 0 or 1.
        1
    }
}

/// Initialise a bare-ish main repo with one initial commit.
fn init_main_repo(branch: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", branch], dir.path());
    crate::tests::support::neutralize_global_excludes(dir.path());
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

/// Write `magic.json` with the given patterns into `root/.superset/`.
fn write_magic(root: &Path, patterns: &[&str]) {
    fs::create_dir_all(root.join(".superset")).unwrap();
    let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let cfg = workspace::superset_files::MagicConfig { files };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
    fs::write(root.join(".superset/magic.json"), body).unwrap();
}

/// Write a file at `root/rel_path` with the given body (creates parents).
fn write_file(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// Create a linked worktree from `main_dir` at a new temp path.
/// Returns `(worktree_dir, worktree_root_path)`.
fn make_worktree(main_dir: &Path) -> (TempDir, PathBuf) {
    let wt = tempfile::tempdir().unwrap();
    let wt_path = wt.path().join("wt");
    git_run(
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/sync-test",
            wt_path.to_str().unwrap(),
        ],
        main_dir,
    );
    let wt_root = wt_path.canonicalize().unwrap();
    (wt, wt_root)
}

// ── Test: patterns from overlaid config copy into the worktree ─────────

/// Literal file pattern copies from main into the worktree.
#[test]
fn sync_literal_file_copies_into_worktree() {
    let main = init_main_repo("main");
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "FOO=1\n");

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(&wt_root, |e| events.push(e.clone())).unwrap();
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
    let code = sync_core(&wt_root, |_| {}).unwrap();
    assert_ne!(exit_code_to_u8(code), 0, "must exit non-zero when magic.json absent");
}

/// Malformed magic.json → non-zero exit.
#[test]
fn sync_malformed_magic_json_is_hard_error() {
    let main = init_main_repo("main");
    fs::create_dir_all(main.path().join(".superset")).unwrap();
    fs::write(main.path().join(".superset/magic.json"), "{bad json").unwrap();

    let (_wt, wt_root) = make_worktree(main.path());
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(&wt_root, |_| {}).unwrap();
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
    let code = sync_core(dir.path(), |_| {}).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "must exit non-zero when not in a git repo"
    );
}
