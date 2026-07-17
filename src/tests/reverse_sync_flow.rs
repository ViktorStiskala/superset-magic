//! End-to-end tests for `run_reverse_sync_flow` (the `ss-magic reverse-sync`
//! handler in `main.rs`): root resolution, the main-checkout hard error, and
//! the happy-path bulk push into main via `sync::reverse_sync::run_bulk`.

use crate::*;
use crate::tests::support::git_run;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::tests::support::{
    exit_code_to_u8, init_main_repo, make_worktree, write_file, write_magic,
};

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
