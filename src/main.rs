use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};

mod apply;
mod cli;
mod exec;
mod git;
mod gitignore;
mod pattern;
mod repo_detect;
mod repo_scan;
mod style;
mod superset_files;
mod ui;
mod update;

use crate::apply::{Event, SkipReason};
use crate::cli::{Command, Parsed};
use crate::git::Mode;
use crate::ui::FinalAction;

const COMMIT_MESSAGE: &str = "chore(superset): bootstrap workspace contract";

fn run() -> Result<ExitCode> {
    style::init();
    // Composition order: style::init (above) → parse argv → git::probe →
    // dispatch. Parsing happens before the git probe so `--help` answers
    // without touching the repo.
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
        Parsed::Command(cmd) => dispatch(cmd),
    }
}

/// Route a parsed command to its handler. `Bare` keeps the existing
/// location-auto behavior; `Sync`/`Update` route to placeholders that
/// downstream units (U4, U7) replace. Each handler runs after `git::probe`
/// only where it needs the probe result.
fn dispatch(cmd: Command) -> Result<ExitCode> {
    let cwd = env::current_dir().context("getting current directory")?;
    match cmd {
        Command::Bare => match git::probe(&cwd)? {
            Mode::Bootstrap { repo_root } => bootstrap_flow(&repo_root),
            Mode::Apply {
                cwd_root,
                main_checkout,
            } => apply_flow(&cwd_root, &main_checkout),
            Mode::Error(msg) => {
                eprintln!("{}", style::err(format!("error: {msg}")));
                Ok(ExitCode::from(1))
            }
        },
        Command::Sync => sync_flow(&cwd),
        Command::Update => update_flow(),
    }
}

/// Non-interactive forward file copy: main checkout → current working tree.
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
fn sync_flow(cwd: &Path) -> Result<ExitCode> {
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

fn bootstrap_flow(repo_root: &Path) -> Result<ExitCode> {
    let existing = superset_files::load_existing(repo_root)?;
    let banner = if existing.superset_dir_present {
        "Bootstrap mode (edit)"
    } else {
        "Bootstrap mode"
    };
    style::print_section(banner);
    println!(
        "{}",
        style::info(format!("Repo root: {}", repo_root.display()))
    );

    let existing_files: Vec<String> = existing
        .setup_config_json
        .as_ref()
        .map(|c| c.files.clone())
        .unwrap_or_default();

    let existing_unknown =
        superset_files::existing_unknown_entries(&existing_files, &repo_scan::OPTIONS);

    // Merge the four preconfigured options with any existing custom patterns
    // so the user can deselect a previously-saved custom to remove it.
    let mut options: Vec<String> = repo_scan::OPTIONS.iter().map(|s| s.to_string()).collect();
    options.extend(existing_unknown.iter().cloned());

    // Preselect logic: filesystem hits or existing-in-config for the four
    // preconfigured patterns; ALL existing customs (they're already in
    // setup_config.json).
    let pattern_strs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let fs_match = repo_scan::matches_for_patterns(repo_root, &pattern_strs)?;
    let mut preselected: Vec<usize> = Vec::new();
    for (i, option) in options.iter().enumerate() {
        let in_existing = existing_files.iter().any(|p| p == option);
        if i < repo_scan::OPTIONS.len() {
            if fs_match[i] || in_existing {
                preselected.push(i);
            }
        } else {
            preselected.push(i);
        }
    }

    // ---- Setup-commands picker inputs. The known-options set is fixed
    // ---- and rendered every run; existing custom entries from
    // ---- config.json's `setup` array surface above the sentinels so
    // ---- the user can deselect them to drop them.
    let existing_setup: Vec<String> = existing
        .config_json
        .as_ref()
        .map(|c| c.setup.clone())
        .unwrap_or_default();

    let existing_unknown_setup =
        superset_files::existing_unknown_entries(&existing_setup, &repo_detect::OPTIONS);

    let mut cmd_options: Vec<String> =
        repo_detect::OPTIONS.iter().map(|s| s.to_string()).collect();
    cmd_options.extend(existing_unknown_setup.iter().cloned());

    let mut cmd_detected = repo_detect::detect_for_options(repo_root)?;
    cmd_detected.extend(std::iter::repeat_n(false, existing_unknown_setup.len()));

    let mut cmd_preselected: Vec<usize> = Vec::new();
    for (i, option) in cmd_options.iter().enumerate() {
        let in_existing = existing_setup.iter().any(|c| c == option);
        if cmd_detected[i] || in_existing {
            cmd_preselected.push(i);
        }
    }

    // ---- Capture all decisions from the user. Nothing is written to
    // ---- repo_root during this section; an early exit (Ctrl-C, Esc)
    // ---- leaves the working tree untouched.
    let chosen = ui::pick_patterns(&options, &preselected, repo_root)?;
    let chosen_commands = ui::pick_setup_commands(&cmd_options, &cmd_preselected, &cmd_detected)?;
    let envrc_choice = superset_files::should_offer_envrc(repo_root) && ui::confirm_envrc()?;
    let action = ui::pick_final_action()?;

    // ---- Stage writes into a tempdir. Drop on early return cleans up.
    let staging = tempfile::Builder::new()
        .prefix("superset-setup-")
        .tempdir()
        .context("creating staging tempdir")?;
    let stage_root = staging.path();
    superset_files::write_setup_sh(stage_root)?;
    superset_files::write_setup_config_json(stage_root, &chosen)?;
    let merged_config =
        superset_files::merge_setup_into_config(existing.config_json.as_ref(), chosen_commands);
    superset_files::write_config_json(stage_root, &merged_config)?;
    if envrc_choice {
        superset_files::write_envrc(stage_root)?;
    }

    // Byte-equality vs the pre-existing on-disk config.json drives the
    // "unchanged" info line. Read once from the stage rather than recomputing
    // the pretty-print to guarantee we compare exactly what'll be copied.
    let staged_config_path = stage_root.join(".superset/config.json");
    let staged_config_body = fs::read_to_string(&staged_config_path).with_context(|| {
        format!("reading staged config {}", staged_config_path.display())
    })?;
    let real_config_path = repo_root.join(".superset/config.json");
    let config_unchanged = fs::read_to_string(&real_config_path)
        .ok()
        .is_some_and(|raw| raw == staged_config_body);

    // ---- Materialize: copy the staged files into repo_root. From here
    // ---- on, the working tree has been touched.
    let report = superset_files::copy_into_repo(stage_root, repo_root)?;

    println!();
    println!("{}", style::ok("Wrote .superset/setup.sh"));
    if !existing.superset_dir_present {
        println!("{}", style::ok("Wrote .superset/config.json"));
    } else if config_unchanged {
        println!(
            "{}",
            style::info("Setup commands unchanged — config.json rewritten with no changes")
        );
    } else {
        println!("{}", style::ok("Updated .superset/config.json"));
    }
    println!("{}", style::ok("Wrote .superset/setup_config.json"));
    if report.wrote_envrc {
        println!("{}", style::ok("Wrote .envrc"));
    }

    execute_final_action(repo_root, action)
}

fn execute_final_action(repo_root: &Path, action: FinalAction) -> Result<ExitCode> {
    match action {
        FinalAction::Done => {
            println!(
                "{}",
                style::info("Done. Changes are on disk; run `git status` to review.")
            );
            Ok(ExitCode::SUCCESS)
        }
        FinalAction::CommitPushMain => {
            git::stage_paths(repo_root, &[".superset", ".envrc"])?;
            if git::nothing_to_commit(repo_root)? {
                println!(
                    "{}",
                    style::info("Nothing to commit — files already match what is tracked.")
                );
                return Ok(ExitCode::SUCCESS);
            }
            git::commit(repo_root, COMMIT_MESSAGE)?;
            let main_branch = git::main_branch_name(repo_root)?;
            git::push(repo_root, "origin", &main_branch)?;
            println!("{}", style::ok(format!("Pushed to origin/{main_branch}")));
            Ok(ExitCode::SUCCESS)
        }
        FinalAction::FeatureBranchPR => {
            git::stage_paths(repo_root, &[".superset", ".envrc"])?;
            if git::nothing_to_commit(repo_root)? {
                println!(
                    "{}",
                    style::info(
                        "Nothing to commit — files already match what is tracked. No branch created."
                    )
                );
                return Ok(ExitCode::SUCCESS);
            }
            let suffix = git::timestamp_branch_suffix()?;
            let branch = format!("chore/superset-setup-{suffix}");
            git::create_branch(repo_root, &branch)?;
            git::commit(repo_root, COMMIT_MESSAGE)?;
            git::push_upstream(repo_root, "origin", &branch)?;
            if !git::gh_available() {
                println!(
                    "{}",
                    style::warn(format!(
                        "`gh` not found in PATH; branch `{branch}` pushed. Open the PR manually."
                    ))
                );
                return Ok(ExitCode::SUCCESS);
            }
            let main_branch = git::main_branch_name(repo_root)?;
            match git::pr_create(repo_root, &main_branch) {
                Ok(url) => {
                    println!("{}", style::ok(format!("PR opened: {url}")));
                }
                Err(err) => {
                    eprintln!(
                        "{}",
                        style::warn(format!(
                            "{err:#}\nBranch `{branch}` is pushed; open the PR manually."
                        ))
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn apply_flow(cwd_root: &Path, main_checkout: &Path) -> Result<ExitCode> {
    style::print_section("Apply Superset config");
    println!(
        "{}",
        style::info(format!("Source: {}", main_checkout.display()))
    );
    println!("{}", style::info(format!("Dest:   {}", cwd_root.display())));

    let cfg = apply::load_main_config(main_checkout)?;
    if cfg.files.is_empty() {
        println!(
            "{}",
            style::info("setup_config.json `files` is empty — nothing to do.")
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!();
    println!("Configured patterns:");
    ui::print_pattern_list(&cfg.files);
    println!();

    if !ui::confirm_apply(main_checkout, cwd_root)? {
        println!("{}", style::info("Skipped — nothing copied."));
        return Ok(ExitCode::SUCCESS);
    }

    let summary = apply::run(main_checkout, cwd_root, &cfg.files, print_event)?;

    let line = format!(
        "File setup done: copied: {} files, skipped {} files",
        summary.copied, summary.skipped
    );
    println!();
    if summary.skipped == 0 {
        println!("{}", style::ok(line));
    } else {
        println!("{}", style::warn(line));
    }

    run_setup_step(cwd_root, main_checkout)?;

    Ok(ExitCode::SUCCESS)
}

/// Picker-output `setup` array consumer side. Reads the main checkout's
/// `config.json`, prints a description of what will execute, asks the
/// user to confirm, and runs the commands (or the `setup.sh` fallback).
/// File copy has already completed at this point; declining or failing
/// the setup step does not roll it back.
fn run_setup_step(workspace_root: &Path, main_checkout: &Path) -> Result<()> {
    let main_config = match superset_files::load_config(main_checkout) {
        Ok(opt) => opt,
        Err(err) => {
            println!();
            println!(
                "{}",
                style::warn(format!(
                    "Could not read .superset/config.json in main checkout: {err:#}\nFile copy completed; skipping setup execution."
                ))
            );
            return Ok(());
        }
    };

    let setup_sh_path = main_checkout.join(".superset/setup.sh");
    let setup_sh_present = setup_sh_path.is_file();

    enum Plan {
        RunCommands(Vec<String>),
        RunSetupSh(PathBuf),
        Skip(&'static str),
    }

    let plan = match main_config {
        Some(cfg) if !cfg.setup.is_empty() => Plan::RunCommands(cfg.setup),
        Some(_) if setup_sh_present => Plan::RunSetupSh(setup_sh_path),
        Some(_) => Plan::Skip("No setup commands configured."),
        None if setup_sh_present => Plan::RunSetupSh(setup_sh_path),
        None => Plan::Skip(
            "No .superset/config.json or .superset/setup.sh in main checkout; nothing to run.",
        ),
    };

    if let Plan::Skip(msg) = plan {
        println!();
        println!("{}", style::info(msg));
        return Ok(());
    }

    println!();
    style::print_section("Setup commands");

    // Print bullets (readable) + the exact shell invocation (the contract).
    let (bullets, invocation) = match &plan {
        Plan::RunCommands(cmds) => (cmds.clone(), exec::invocation_preview(cmds)),
        Plan::RunSetupSh(p) => {
            let line = format!("bash {}", p.display());
            (vec![line.clone()], line)
        }
        Plan::Skip(_) => unreachable!(),
    };

    ui::print_pattern_list(&bullets);
    println!();
    println!(
        "{}",
        style::info(format!("Will run as: {invocation}"))
    );
    println!(
        "{}",
        style::info(format!("Working directory: {}", workspace_root.display()))
    );
    println!(
        "{}",
        style::info(
            "File copy is already complete. Declining leaves files in place; commands will not run."
        )
    );
    println!(
        "{}",
        style::info("Env vars exposed to commands: SUPERSET_ROOT_PATH, SUPERSET_WORKSPACE_PATH")
    );
    println!();

    if !ui::confirm_run_setup_commands()? {
        println!(
            "{}",
            style::info("Skipped setup commands. Files are in place; run setup manually when ready.")
        );
        return Ok(());
    }

    println!();
    let status = match plan {
        Plan::RunCommands(cmds) => {
            exec::run(workspace_root, main_checkout, &cmds, print_exec_event)?
        }
        Plan::RunSetupSh(p) => {
            exec::run_setup_sh(workspace_root, main_checkout, &p, print_exec_event)?
        }
        Plan::Skip(_) => unreachable!(),
    };

    if status.success() {
        println!();
        println!("{}", style::ok("Setup complete."));
        Ok(())
    } else {
        bail!(
            "Setup failed (exit {}). The file copy completed and is not rolled back. \
Fix the issue, then either run the setup commands directly or re-run \
`superset-setup` and decline the file-copy step.",
            exec::format_exit(status)
        );
    }
}

fn print_exec_event(ev: &exec::Event) {
    match ev {
        exec::Event::Begin { display } => {
            println!("{}", style::info(format!("Running: {display}")));
        }
        exec::Event::Complete { status } => {
            // Eager non-zero surface; the `run_setup_step` caller adds the
            // longer recovery message when it bails. Success path stays
            // quiet so the caller's "Setup complete." reads cleanly.
            if !status.success() {
                println!(
                    "{}",
                    style::warn(format!("Setup exit: {}", exec::format_exit(*status)))
                );
            }
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
    use std::fs;
    use std::process::{Command, Stdio};
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

    // ── Git helpers (mirroring git.rs test helpers) ────────────────────────

    fn git_run(args: &[&str], cwd: &Path) {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            // Disable GPG signing and isolate from global git config so tests
            // don't depend on machine-level settings (e.g. commit.gpgsign=true).
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "user.email")
            .env("GIT_CONFIG_VALUE_0", "test@example.com")
            .env("GIT_CONFIG_KEY_1", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_1", "false")
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

    /// Initialise a bare-ish main repo with one initial commit.
    fn init_main_repo(branch: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_run(&["init", "-q", "-b", branch], dir.path());
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
