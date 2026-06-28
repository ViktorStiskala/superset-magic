//! Shared test-only helpers (declared `#[cfg(test)]` in `main.rs`, so this
//! compiles only under `cargo test`).
//!
//! Centralizes the isolated-git invocation the per-module test suites
//! previously each redefined: author/committer identity is set via env vars
//! (so commits work on CI runners with no global git config), and
//! machine/system config (e.g. `commit.gpgsign`) is neutralized so commits
//! never block on a gpg agent.

use std::path::Path;
use std::process::{Command, Stdio};

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
