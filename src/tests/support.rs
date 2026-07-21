//! Shared test-only helpers (declared `#[cfg(test)]` in `main.rs`, so this
//! compiles only under `cargo test`).
//!
//! Centralizes the isolated-git invocation the per-module test suites
//! previously each redefined: author/committer identity is set via env vars
//! (so commits work on CI runners with no global git config), and
//! machine/system config (e.g. `commit.gpgsign`) is neutralized so commits
//! never block on a gpg agent.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use tempfile::TempDir;

/// Neutralize the developer's GLOBAL git ignore for a freshly-initialized test
/// repo by pointing its local `core.excludesFile` at an empty source
/// (`/dev/null`).
///
/// Without this, a global excludes file (commonly `~/.config/git/ignore`
/// listing `.env` / `.dev.vars`) leaks into `git check-ignore` and `git
/// ls-files` and silently breaks the reverse-sync / gitignore tests, which
/// assume each test repo's own `.gitignore` is the *only* ignore source.
/// `core.excludesFile` lives in the shared (common) config, so calling this on
/// the main repo also covers any linked worktree created from it. Call right
/// after `git init`. Uses `/dev/null` as the empty source — Unix-only, like the
/// rest of this test suite (shell `git`, `chmod 0755`, `magic.sh`).
pub fn neutralize_global_excludes(repo_root: &Path) {
    git_run(
        &["config", "--local", "core.excludesFile", "/dev/null"],
        repo_root,
    );
}

/// Run `git <args>` in `cwd` with an isolated identity + config and assert it
/// succeeds. On failure the panic message carries git's stderr.
pub fn git_run(args: &[&str], cwd: &Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        // Isolate from machine-level git config (e.g. commit.gpgsign=true) so
        // commits don't intermittently fail on a slow/absent gpg agent.
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .stdout(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed in {}:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── Crate-root sync-flow test fixtures (shared by `tests::sync` +
//    `tests::reverse_sync_flow`) ──────────────────────────────────────────────

/// Convert an [`ExitCode`] to a u8 for assertions (`ExitCode` has no
/// `From<ExitCode> for u8`): `SUCCESS` → 0, anything else → 1 (these flows only
/// ever return 0 or 1).
pub(crate) fn exit_code_to_u8(code: ExitCode) -> u8 {
    if code == ExitCode::SUCCESS {
        0
    } else {
        1
    }
}

/// Initialise a bare-ish main repo on `branch` with one initial commit.
pub(crate) fn init_main_repo(branch: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", branch], dir.path());
    neutralize_global_excludes(dir.path());
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

/// Write `magic.json` with the given patterns into `root/.superset/`.
pub(crate) fn write_magic(root: &Path, patterns: &[&str]) {
    fs::create_dir_all(root.join(".superset")).unwrap();
    let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let cfg = crate::workspace::superset_files::MagicConfig { files };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
    fs::write(root.join(".superset/magic.json"), body).unwrap();
}

/// Write a file at `root/rel` with the given body (creates parents).
pub(crate) fn write_file(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// Create a linked worktree from `main_dir` at a new temp path. Returns
/// `(worktree_tempdir, worktree_root_path)`. The branch name is arbitrary (no
/// test asserts on it).
pub(crate) fn make_worktree(main_dir: &Path) -> (TempDir, PathBuf) {
    let wt = tempfile::tempdir().unwrap();
    let wt_path = wt.path().join("wt");
    git_run(
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/sync-flow-test",
            wt_path.to_str().unwrap(),
        ],
        main_dir,
    );
    let wt_root = wt_path.canonicalize().unwrap();
    (wt, wt_root)
}
