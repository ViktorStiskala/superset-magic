//! Shared test-only helpers for every crate in the workspace.
//!
//! Compiled under `cfg(test)` for core's own suite and behind the `testutil`
//! feature for the binaries' suites (each enables it from its
//! `[dev-dependencies]` only, so no release build ever contains this module).
//! Every helper is `pub` because the callers sit in other crates.
//!
//! Centralizes the isolated-git invocation the per-module test suites
//! previously each redefined: author/committer identity is set via env vars
//! (so commits work on CI runners with no global git config), and
//! machine/system config (e.g. `commit.gpgsign`) is neutralized so commits
//! never block on a gpg agent.

use std::ffi::OsStr;
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
pub fn exit_code_to_u8(code: ExitCode) -> u8 {
    if code == ExitCode::SUCCESS {
        0
    } else {
        1
    }
}

/// Initialise a bare-ish main repo on `branch` with one initial commit.
pub fn init_main_repo(branch: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", branch], dir.path());
    // Disable automatic housekeeping in every fixture repo. `git commit` below
    // can otherwise spawn `gc --auto`, which repacks `.git/objects` in the
    // background; a test that walks or copies `.git` then sees `read_dir` yield
    // an entry that is gone by the time it is stat'd. That race only opens under
    // parallel load, so it surfaces as an intermittent failure of whichever test
    // happens to be reading `.git` at the time.
    git_run(&["config", "gc.auto", "0"], dir.path());
    neutralize_global_excludes(dir.path());
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

/// Write `magic.json` with the given patterns into `root/.superset/`.
pub fn write_magic(root: &Path, patterns: &[&str]) {
    fs::create_dir_all(root.join(".superset")).unwrap();
    let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let cfg = crate::superset_files::MagicConfig {
        files,
        ..Default::default()
    };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
    fs::write(root.join(".superset/magic.json"), body).unwrap();
}

/// Write a file at `root/rel` with the given body (creates parents).
pub fn write_file(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// Create a linked worktree from `main_dir` at a new temp path. Returns
/// `(worktree_tempdir, worktree_root_path)`. The branch name is arbitrary (no
/// test asserts on it).
pub fn make_worktree(main_dir: &Path) -> (TempDir, PathBuf) {
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

// ── Running one test in a child process ──────────────────────────────────────

/// Run one `#[ignore]`d test of this same test binary in a CHILD process,
/// with `env` set and `remove` unset in that child only, and return its
/// combined output. Panics unless the child ran exactly one test and it
/// passed.
///
/// This exists for tests whose subject is the process environment. Rust runs
/// tests on parallel threads, and `PATH` or a `GIT_*` variable set with
/// `std::env::set_var` is inherited by every `Command` any other thread spawns
/// during the window – a lock only serializes the tests that take it, not the
/// rest of the suite. A child process gets its own environment, so the
/// variable exists exactly where the test needs it and nowhere else.
///
/// `test_path` is the test's path WITHOUT the crate prefix, as libtest names
/// it (`git::discover::tests::child_x`); build it from `module_path!()` with
/// [`test_path_in_binary`]. The child test reads its inputs from the
/// variables passed here and must return early when they are absent, so an
/// `--include-ignored` run of the suite does not fail on the child half.
pub fn run_ignored_test_in_child(
    test_path: &str,
    env: &[(&str, &OsStr)],
    remove: &[&str],
) -> String {
    run_child(None, test_path, env, remove)
}

/// [`run_ignored_test_in_child`] with the child's working directory set to
/// `cwd`.
///
/// For a test whose subject is the process's OWN working directory – a
/// relative path resolved against it, say. `cargo test` starts every test
/// binary in its package root, which is `crates/<name>` in this workspace and
/// so holds no `docs/` or other repository-level directory a fixture might
/// need to exist there; and the cwd is process-global, so a test can no more
/// `set_current_dir` under a parallel suite than it can `set_var`. The child
/// must return early when the directory it expects is absent, for the same
/// `--include-ignored` reason as above.
pub fn run_ignored_test_in_child_from(
    cwd: &Path,
    test_path: &str,
    env: &[(&str, &OsStr)],
    remove: &[&str],
) -> String {
    run_child(Some(cwd), test_path, env, remove)
}

fn run_child(
    cwd: Option<&Path>,
    test_path: &str,
    env: &[(&str, &OsStr)],
    remove: &[&str],
) -> String {
    let exe = std::env::current_exe().expect("the test binary's own path");
    let mut cmd = Command::new(exe);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.args([
        "--exact",
        test_path,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]);
    for name in remove {
        cmd.env_remove(name);
    }
    for (name, value) in env {
        cmd.env(name, value);
    }
    let out = cmd.stdin(Stdio::null()).output().expect("spawn the test binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        out.status.success(),
        "child test `{test_path}` failed ({}):\n{combined}",
        out.status
    );
    // A mistyped path runs zero tests and still exits 0; that must not pass.
    assert!(
        stdout.contains("1 passed"),
        "child did not run exactly one test `{test_path}`:\n{combined}"
    );
    combined
}

/// The libtest name of `child` inside the module `module_path!()` names:
/// the crate prefix stripped, then `::child` appended.
pub fn test_path_in_binary(module_path: &str, child: &str) -> String {
    let without_crate = module_path.split_once("::").map_or("", |(_, rest)| rest);
    format!("{without_crate}::{child}")
}
