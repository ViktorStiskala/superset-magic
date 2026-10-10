//! End-to-end tests for `run_reverse_sync_flow` (the `ss-magic reverse-sync`
//! handler in `main.rs`): root resolution, the main-checkout hard error, and
//! the happy-path bulk push into main via `sync::reverse_sync::run_bulk`.

use crate::*;
use std::fs;

use ss_magic_core::testutil::{
    exit_code_to_u8, git_run, init_main_repo, make_worktree, write_file, write_magic,
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

// ── Ignore-rule sink follows the install mode (KTD4) ────────────────────

/// The lines of the repository's shared `<git-common-dir>/info/exclude`, or
/// none when the file does not exist.
fn info_exclude_lines(root: &Path) -> Vec<String> {
    let path = git::git_common_dir(root).unwrap().join("info/exclude");
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// `git status --porcelain --untracked-files=all` in `root`, verbatim.
fn porcelain(root: &Path) -> String {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success(), "git status failed");
    String::from_utf8(out.stdout).unwrap()
}

/// A main checkout with a committed `.gitignore` (`target/`), so a test can
/// prove ss-magic left that tracked file byte-for-byte alone.
fn main_with_committed_gitignore() -> (tempfile::TempDir, PathBuf) {
    let main = init_main_repo("main");
    let main_root = main.path().canonicalize().unwrap();
    write_file(&main_root, ".gitignore", "target/\n");
    git_run(&["add", ".gitignore"], &main_root);
    git_run(&["commit", "-q", "-m", "gitignore"], &main_root);
    (main, main_root)
}

/// A local install in `main_root` for `patterns`, plus a linked worktree that
/// carries the install's `magic.local.json` (as its first forward sync would
/// leave it – reverse sync reads the pattern list from the worktree).
fn local_install_with_worktree(
    main_root: &Path,
    patterns: &[&str],
) -> (tempfile::TempDir, PathBuf) {
    let patterns: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let code =
        workspace::local_install::run_local_init_noninteractive(main_root, &patterns).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "local install must succeed");
    let (wt, wt_root) = make_worktree(main_root);
    // `.superset/` is untracked on a local install, so the fresh worktree has
    // no such directory yet.
    fs::create_dir_all(wt_root.join(".superset")).unwrap();
    fs::copy(
        main_root.join(".superset/magic.local.json"),
        wt_root.join(".superset/magic.local.json"),
    )
    .unwrap();
    (wt, wt_root)
}

/// AE5: bulk reverse sync on a LOCAL install puts the secret gate's rule in
/// the shared `info/exclude` as an anchored literal, never in main's tracked
/// `.gitignore`; the secret lands in main and main's `git status` stays empty.
#[test]
fn ae5_local_install_reverse_sync_excludes_secret_via_info_exclude() {
    let (_main, main_root) = main_with_committed_gitignore();
    let (_wt, wt_root) = local_install_with_worktree(&main_root, &["secret.key"]);
    write_file(&wt_root, "secret.key", "KEY=wt\n");

    let code = run_reverse_sync_flow(&wt_root, false).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "reverse sync must succeed");

    assert_eq!(
        fs::read_to_string(main_root.join("secret.key")).unwrap(),
        "KEY=wt\n",
        "the secret must land in main"
    );
    assert!(
        info_exclude_lines(&main_root).contains(&"/secret.key".to_string()),
        "info/exclude must gain the anchored `/secret.key` rule, got {:?}",
        info_exclude_lines(&main_root)
    );
    assert_eq!(
        fs::read_to_string(main_root.join(".gitignore")).unwrap(),
        "target/\n",
        "main's tracked .gitignore must be byte-identical"
    );
    assert_eq!(
        porcelain(&main_root),
        "",
        "main must stay clean: the secret is ignored through info/exclude"
    );
}

/// Regression guard for KTD4: on a COMMITTED install the secret gate's rule
/// still goes to main's `.gitignore` (the team-visible rule), not to
/// `info/exclude`.
#[test]
fn committed_install_reverse_sync_still_writes_gitignore() {
    let (_main, main_root) = main_with_committed_gitignore();
    write_magic(&main_root, &["secret.key"]);
    git_run(&["add", ".superset/magic.json"], &main_root);
    git_run(&["commit", "-q", "-m", "magic"], &main_root);
    let (_wt, wt_root) = make_worktree(&main_root);
    write_file(&wt_root, "secret.key", "KEY=wt\n");

    let code = run_reverse_sync_flow(&wt_root, false).unwrap();
    assert_eq!(exit_code_to_u8(code), 0, "reverse sync must succeed");

    let gitignore = fs::read_to_string(main_root.join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|l| l.contains("secret.key")),
        "a committed install keeps writing the secret rule to .gitignore, got {gitignore:?}"
    );
    assert!(
        !info_exclude_lines(&main_root)
            .iter()
            .any(|l| l.contains("secret.key")),
        "a committed install must not touch info/exclude for the secret"
    );
    assert!(git::is_ignored(&main_root, Path::new("secret.key")).unwrap());
}

/// The strict re-check survives the sink change: on a local install whose
/// tracked `.gitignore` NEGATES the secret (`!secret.key` outranks any
/// `info/exclude` rule), the push fails loudly and the secret never reaches
/// main.
#[test]
fn local_install_push_fails_when_tracked_negation_overrides_info_exclude() {
    let main = init_main_repo("main");
    let main_root = main.path().canonicalize().unwrap();
    write_file(&main_root, ".gitignore", "!secret.key\n");
    git_run(&["add", ".gitignore"], &main_root);
    git_run(&["commit", "-q", "-m", "negation"], &main_root);
    let (_wt, wt_root) = local_install_with_worktree(&main_root, &["secret.key"]);
    write_file(&wt_root, "secret.key", "KEY=wt\n");

    let code = run_reverse_sync_flow(&wt_root, false).unwrap();
    assert_ne!(
        exit_code_to_u8(code),
        0,
        "a secret git still does not ignore must fail the push"
    );
    assert!(
        !main_root.join("secret.key").exists(),
        "the un-ignorable secret must never land in main"
    );
    assert_eq!(
        fs::read_to_string(main_root.join(".gitignore")).unwrap(),
        "!secret.key\n",
        "main's tracked .gitignore must be untouched"
    );
}
