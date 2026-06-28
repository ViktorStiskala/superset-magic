use std::env;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};

mod apply;
mod cli;
mod git;
mod gitignore;
mod menu;
mod migrate;
mod pattern;
mod repo_scan;
mod reverse_sync;
mod style;
mod superset_files;
mod ui;
mod update;
#[cfg(test)]
mod test_support;

use crate::apply::{Event, SkipReason};
use crate::cli::{Command, Parsed};

/// Pure gate-decision helper (U8, AE3). Returns `true` when the auto-update
/// daily-cache gate should fire for the given command and guard state.
///
/// Truth table:
///
/// | cmd            | guard_active | result |
/// |----------------|--------------|--------|
/// | `Bare`         | false        | true   |
/// | `Sync`         | false        | true   |
/// | `Update`       | false        | false  |
/// | `Bare`/`Sync`  | true         | false  |
/// | `Update`       | true         | false  |
///
/// `Command::Update` always bypasses the gate — it routes to the force path
/// (U7's `update_command`) which is NOT gated by the 24h cache. When the loop
/// guard is active (`SS_MAGIC_UPDATED` or `SS_MAGIC_NO_UPDATE` is set) the
/// gate never fires regardless of command, preventing re-exec loops (AE4).
pub fn should_run_update_gate(cmd: Command, guard_active: bool) -> bool {
    if guard_active {
        return false;
    }
    matches!(cmd, Command::Bare | Command::Sync)
}

fn run() -> Result<ExitCode> {
    style::init();
    // Composition order: style::init (above) → parse argv → [help check] →
    // [auto-update gate for Bare/Sync] → dispatch (menu / sync / update).
    // Parsing and the help response happen before the gate so `--help`
    // answers instantly without a network call.
    let args: Vec<String> = env::args().skip(1).collect();
    match cli::parse(&args) {
        Parsed::Help => {
            println!("{}", cli::usage());
            Ok(ExitCode::SUCCESS)
        }
        Parsed::Error(token) => {
            eprintln!(
                "{}",
                style::err(format!("error: unknown command `{token}`"))
            );
            eprintln!("{}", cli::usage());
            Ok(ExitCode::from(2))
        }
        // Non-interactive init (AN1): seed the layout from CLI patterns. Not
        // gated — one-time setup shouldn't depend on a network round-trip.
        Parsed::Init(patterns) => init_noninteractive(&patterns),
        Parsed::Command(cmd) => {
            // U8: run the daily-cache auto-update gate before any work for
            // `Bare` and `Sync`. On a "newer" verdict, `auto_update` swaps the
            // binary, re-execs, and terminates this process — the code below is
            // only reached when no update is needed. `Update` skips the gate
            // entirely and uses the force path in `update_flow`.
            if should_run_update_gate(cmd, update::apply::guard_active()) {
                update::auto_update();
            }
            dispatch(cmd)
        }
    }
}

/// Non-interactive `ss-magic init [PATTERN...]` (AN1): seed the magic.json
/// layout from CLI-supplied patterns without the TUI, so automation (CI,
/// Superset provisioning) can bootstrap a repo. Operates on the current
/// checkout root.
fn init_noninteractive(patterns: &[String]) -> Result<ExitCode> {
    let cwd = env::current_dir().context("getting current directory")?;
    match git::cwd_repo_root(&cwd) {
        Ok(repo_root) => migrate::run_init_noninteractive(&repo_root, patterns),
        Err(err) => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: `ss-magic init` must run inside a git repository: {err:#}"
                ))
            );
            Ok(ExitCode::from(1))
        }
    }
}

/// Route a parsed command to its handler. `Bare` routes to the
/// location-aware operation menu (U10); `Sync`/`Update` route to their
/// respective handlers.
fn dispatch(cmd: Command) -> Result<ExitCode> {
    let cwd = env::current_dir().context("getting current directory")?;
    match cmd {
        Command::Bare => menu::run(&cwd),
        Command::Sync => run_sync_flow(&cwd),
        Command::Update => update_flow(),
    }
}

/// Non-interactive forward file copy: main checkout → current working tree.
/// Handler for `ss-magic sync` and the worktree menu's "Forward sync".
///
/// Resolves the main checkout root, verifies `.superset/magic.json` exists
/// there, loads the overlaid config (magic.json + magic.local.json), then
/// runs the existing `apply::run` engine into `cwd`. No git/gh operations,
/// no setup commands.
///
/// Hard errors (non-zero exit):
/// - Cannot resolve the main checkout root (not in a git repo, or git fails).
/// - `.superset/magic.json` absent in the resolved main root.
/// - Malformed `magic.json` or `magic.local.json` in the main root.
pub fn run_sync_flow(cwd: &Path) -> Result<ExitCode> {
    sync_core(cwd, print_event)
}

/// Extracted core of `sync_flow` so tests can inject a no-op event handler
/// without side-effects on stdout/stderr.
fn sync_core<F>(cwd: &Path, on_event: F) -> Result<ExitCode>
where
    F: FnMut(&Event),
{
    // 1. Resolve the current repo root (the working tree cwd belongs to).
    let cwd_root = match git::cwd_repo_root(cwd) {
        Ok(r) => r,
        Err(err) => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: cannot resolve git repo root from {}: {err:#}",
                    cwd.display()
                ))
            );
            return Ok(ExitCode::from(1));
        }
    };

    // 2. Resolve the main checkout root (parent of git-common-dir).
    let main_root = match git::main_checkout_root(&cwd_root) {
        Ok(r) => r,
        Err(err) => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: cannot resolve main checkout root: {err:#}"
                ))
            );
            return Ok(ExitCode::from(1));
        }
    };

    // 3. Probe for magic.json — hard error when absent.
    let magic_json_path = main_root.join(".superset/magic.json");
    if !magic_json_path.is_file() {
        eprintln!(
            "{}",
            style::err(format!(
                "error: no `.superset/magic.json` in {}; expected {}",
                main_root.display(),
                magic_json_path.display()
            ))
        );
        return Ok(ExitCode::from(1));
    }

    // 4. Load overlaid config — propagates parse errors as non-zero exit.
    let cfg = match superset_files::load_overlaid(&main_root) {
        Ok(Some(c)) => c,
        Ok(None) => {
            // load_overlaid returns None when magic.json is absent; we probed
            // above so this branch means the probe and the load raced — treat
            // it the same as absent.
            eprintln!(
                "{}",
                style::err(format!(
                    "error: no `.superset/magic.json` in {}; expected {}",
                    main_root.display(),
                    magic_json_path.display()
                ))
            );
            return Ok(ExitCode::from(1));
        }
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
    };

    // 5. Empty files list → nothing to do, success.
    if cfg.files.is_empty() {
        println!(
            "{}",
            style::info("magic.json `files` is empty — nothing to sync.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // 6. Run the apply engine: main_root → cwd_root.
    let summary = match apply::run(&main_root, &cwd_root, &cfg.files, on_event) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
    };

    let line = format!(
        "Sync done: copied {} files, skipped {} files",
        summary.copied, summary.skipped
    );
    println!();
    if summary.skipped == 0 {
        println!("{}", style::ok(line));
    } else {
        println!("{}", style::warn(line));
    }

    Ok(ExitCode::SUCCESS)
}

/// `ss-magic update` (R4): force a self-update regardless of the 24h cache.
///
/// Routes straight to the forced apply path (U7), which bypasses the daily
/// cache, runs the `self_update` lock/download/swap if a newer release exists,
/// and reports the resulting version or "already latest". Unlike the bare/sync
/// auto-update gate (U8), this does not re-exec — the update itself is the
/// requested work.
fn update_flow() -> Result<ExitCode> {
    style::print_section("Self-update");
    match update::update_command() {
        update::UpdateReport::Updated { version } => {
            println!("{}", style::ok(format!("Updated to v{version}.")));
            Ok(ExitCode::SUCCESS)
        }
        update::UpdateReport::AlreadyLatest => {
            println!("{}", style::info("Already on the latest release."));
            Ok(ExitCode::SUCCESS)
        }
        update::UpdateReport::Skipped => {
            println!(
                "{}",
                style::warn(
                    "Another update is already in progress; skipped. Try again in a moment."
                )
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn print_event(ev: &Event) {
    match ev {
        Event::Copy { rel } => {
            println!("{}", style::info(format!("Copied: {}", rel.display())));
        }
        Event::Skip { reason, label } => {
            let line = format!("Skipped ({}): {label}", reason.label());
            if matches!(reason, SkipReason::Excluded) {
                println!("{}", style::info(line));
            } else if matches!(reason, SkipReason::NoMatches) {
                // Default color, like setup.sh.
                println!("{line}");
            } else if reason.counts() {
                eprintln!("{}", style::err(line));
            } else {
                eprintln!("{}", style::warn(line));
            }
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use crate::test_support::git_run;
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
        crate::test_support::neutralize_global_excludes(dir.path());
        fs::write(dir.path().join("README.md"), "hi").unwrap();
        git_run(&["add", "."], dir.path());
        git_run(&["commit", "-q", "-m", "init"], dir.path());
        dir
    }

    /// Write `magic.json` with the given patterns into `root/.superset/`.
    fn write_magic(root: &Path, patterns: &[&str]) {
        fs::create_dir_all(root.join(".superset")).unwrap();
        let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let cfg = superset_files::MagicConfig { files };
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
        let mut events: Vec<apply::Event> = Vec::new();
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
}

/// U8 gate-decision tests: `should_run_update_gate` truth table (AE3).
///
/// These are pure unit tests over the decision helper only — they do not
/// perform network calls, lock files, or re-exec. The actual block-in-wait
/// and exit-with-child-code behavior is seam-tested in U7 (update/apply.rs).
#[cfg(test)]
mod update_gate_tests {
    use super::*;

    // ── Gate fires for Bare / Sync when guard is inactive ───────────────────

    /// AE3 (wiring): Bare command + no guard → gate fires.
    #[test]
    fn ae3_bare_no_guard_gate_fires() {
        assert!(
            should_run_update_gate(Command::Bare, false),
            "Bare + guard inactive → gate must fire"
        );
    }

    /// Sync command + no guard → gate fires.
    #[test]
    fn sync_no_guard_gate_fires() {
        assert!(
            should_run_update_gate(Command::Sync, false),
            "Sync + guard inactive → gate must fire"
        );
    }

    // ── Update bypasses the gate regardless of guard state ──────────────────

    /// Update + no guard → gate does NOT fire (uses its own force path).
    #[test]
    fn update_no_guard_gate_does_not_fire() {
        assert!(
            !should_run_update_gate(Command::Update, false),
            "Update must bypass the daily-cache gate (uses force path)"
        );
    }

    /// Update + guard active → gate does NOT fire.
    #[test]
    fn update_guard_active_gate_does_not_fire() {
        assert!(
            !should_run_update_gate(Command::Update, true),
            "Update + guard active → gate must not fire"
        );
    }

    // ── Guard active short-circuits the gate for all commands ───────────────

    /// AE4 (no loop): re-exec'd child has SS_MAGIC_UPDATED=1 → guard active →
    /// gate does not fire, preventing infinite re-exec loops.
    #[test]
    fn ae4_bare_guard_active_gate_does_not_fire() {
        assert!(
            !should_run_update_gate(Command::Bare, true),
            "Bare + guard active → gate must not fire (loop prevention)"
        );
    }

    /// Sync + guard active → gate does NOT fire.
    #[test]
    fn sync_guard_active_gate_does_not_fire() {
        assert!(
            !should_run_update_gate(Command::Sync, true),
            "Sync + guard active (SS_MAGIC_NO_UPDATE) → gate must not fire"
        );
    }
}
