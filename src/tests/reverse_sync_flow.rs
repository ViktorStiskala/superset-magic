//! End-to-end tests for `run_reverse_sync_flow` (the `ss-magic reverse-sync`
//! handler in `main.rs`): root resolution, the main-checkout hard error, and
//! the happy-path bulk push into main via `sync::reverse_sync::run_bulk`.

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
            "feature/reverse-sync-flow-test",
            wt_path.to_str().unwrap(),
        ],
        main_dir,
    );
    let wt_root = wt_path.canonicalize().unwrap();
    (wt, wt_root)
}

/// An untracked worktree file matching `magic.json` that differs from main's
/// copy must be bulk-pushed into main, gitignored there, with a success exit.
#[test]
fn run_reverse_sync_flow_pushes_untracked_candidate_into_main() {
    let main = init_main_repo("main");
    let (_wt, wt_root) = make_worktree(main.path());

    // Reverse sync's candidate computation reads magic.json from the
    // WORKTREE (not main) — unlike forward sync.
    write_magic(&wt_root, &["**/.dev.vars"]);
    // Main already has a differing copy at the same path.
    write_file(main.path(), "apps/api/.dev.vars", "MAIN=1\n");
    // The worktree's copy is untracked (never `git add`ed) and differs.
    write_file(&wt_root, "apps/api/.dev.vars", "SECRET=wt\n");

    let code = run_reverse_sync_flow(&wt_root, false).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "reverse sync must succeed");

    let pushed = fs::read_to_string(main.path().join("apps/api/.dev.vars")).unwrap();
    assert_eq!(
        pushed, "SECRET=wt\n",
        "main must gain the worktree's bytes"
    );

    assert!(
        git::is_ignored(main.path(), Path::new("apps/api/.dev.vars")).unwrap(),
        "the pushed untracked secret must be gitignored in main"
    );
}

/// Calling `ss-magic reverse-sync` FROM the main checkout (cwd_root ==
/// main_root) must hard-error and touch nothing — there is no worktree to
/// push from.
#[test]
fn reverse_sync_flow_from_main_checkout_is_hard_error() {
    let main = init_main_repo("main");
    write_magic(main.path(), &["**/.dev.vars"]);
    write_file(main.path(), "apps/api/.dev.vars", "MAIN=1\n");

    let code = run_reverse_sync_flow(main.path(), false).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "must exit non-zero when run from the main checkout"
    );
    assert!(
        !main.path().join(".superset/backups").exists(),
        "must write nothing (no backups dir) when refused"
    );
    assert_eq!(
        fs::read_to_string(main.path().join("apps/api/.dev.vars")).unwrap(),
        "MAIN=1\n",
        "main's file must be untouched when refused"
    );
}

/// When cwd is not inside any git repository, `run_reverse_sync_flow` must
/// exit non-zero.
#[test]
fn reverse_sync_flow_outside_git_repo_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    // No git init — not a repo.
    let code = run_reverse_sync_flow(dir.path(), false).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "must exit non-zero when not in a git repo"
    );
}
