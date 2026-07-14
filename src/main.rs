use std::env;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};

mod cli;
mod git;
mod pack;
mod sync;
mod tui;
mod update;
mod workspace;
#[cfg(test)]
mod tests;

use crate::sync::apply::{Event, SkipReason};
use crate::cli::{Command, Parsed};

/// Pure gate-decision helper (U8, AE3). Returns `true` when the auto-update
/// daily-cache gate should fire for the given command and guard state.
///
/// Truth table:
///
/// | cmd                 | guard_active | result |
/// |---------------------|--------------|--------|
/// | `Bare`              | false        | true   |
/// | `Sync`              | false        | true   |
/// | `Pack`              | false        | true   |
/// | `Update`            | false        | false  |
/// | `Bare`/`Sync`/`Pack`| true         | false  |
/// | `Update`            | true         | false  |
///
/// `Command::Update` always bypasses the gate — it routes to the force path
/// (U7's `update_command`) which is NOT gated by the 24h cache. When the loop
/// guard is active (`SS_MAGIC_UPDATED` or `SS_MAGIC_NO_UPDATE` is set) the
/// gate never fires regardless of command, preventing re-exec loops (AE4).
///
/// `Command::Pack` is gated alongside `Bare`/`Sync`: it is a non-interactive
/// "do work" command like `sync`, so gating keeps pack users self-updating.
pub fn should_run_update_gate(cmd: Command, guard_active: bool) -> bool {
    if guard_active {
        return false;
    }
    matches!(cmd, Command::Bare | Command::Sync | Command::Pack)
}

fn run() -> Result<ExitCode> {
    tui::style::init();
    // Composition order: tui::style::init (above) → parse argv → [help check] →
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
                tui::style::err(format!("error: unknown command `{token}`"))
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
        Ok(repo_root) => workspace::migrate::run_init_noninteractive(&repo_root, patterns),
        Err(err) => {
            eprintln!(
                "{}",
                tui::style::err(format!(
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
        Command::Bare => tui::menu::run(&cwd),
        Command::Sync => run_sync_flow(&cwd),
        Command::Pack => run_pack_flow(&cwd),
        Command::Update => update_flow(),
    }
}

/// Probe `<root>/.superset/magic.json` and load the overlaid config, printing a
/// styled error and returning the exit code on absence or malformation. Shared
/// by the forward-sync (`sync_core`) and pack (`pack::pack_core`) flows so the
/// "magic.json absent/malformed" error path lives in exactly one place.
///
/// `Ok(None)` from `load_overlaid` means the file vanished between the probe and
/// the load (a race) — reported the same as absent.
pub fn load_magic_or_exit(root: &Path) -> std::result::Result<workspace::superset_files::MagicConfig, ExitCode> {
    let magic_json_path = root.join(".superset/magic.json");
    let absent = || {
        eprintln!(
            "{}",
            tui::style::err(format!(
                "error: no `.superset/magic.json` in {}; expected {}",
                root.display(),
                magic_json_path.display()
            ))
        );
        ExitCode::from(1)
    };

    if !magic_json_path.is_file() {
        return Err(absent());
    }
    match workspace::superset_files::load_overlaid(root) {
        Ok(Some(cfg)) => Ok(cfg),
        Ok(None) => Err(absent()),
        Err(err) => {
            eprintln!("{}", tui::style::err(format!("error: {err:#}")));
            Err(ExitCode::from(1))
        }
    }
}

/// Non-interactive pack: archive the files defined by the overlaid `magic.json`
/// into `ss-magic-files.tar.bz2` at the git root. Handler for `ss-magic pack`
/// and the interactive menu's "Pack" operation. Delegates to `pack::pack_core`
/// with the stdout event printer.
pub fn run_pack_flow(cwd: &Path) -> Result<ExitCode> {
    pack::pack_core(cwd, print_pack_event)
}

fn print_pack_event(ev: &pack::PackEvent) {
    match ev {
        pack::PackEvent::Add { rel } => {
            println!("{}", tui::style::info(format!("Added: {}", rel.display())));
        }
        pack::PackEvent::Done { out_path, count } => {
            let name = out_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| out_path.display().to_string());
            // Prefer the canonical path for both display and clipboard so the
            // copied value works from anywhere, not just the repo root.
            let real = out_path
                .canonicalize()
                .unwrap_or_else(|_| out_path.clone());
            println!();
            println!(
                "{}",
                tui::style::ok(format!("Packed {count} entries → {}", real.display()))
            );
            println!(
                "{}",
                tui::style::info(format!(
                    "Extract into a repo root with: tar -xjvf {name} -C /path/to/repo"
                ))
            );
            if pack::copy_to_clipboard(&real.display().to_string()) {
                println!("{}", tui::style::info("full path copied to clipboard"));
            }
        }
    }
}

/// Non-interactive forward file copy: main checkout → current working tree.
/// Handler for `ss-magic sync` and the worktree menu's "Forward sync".
///
/// Resolves the main checkout root, verifies `.superset/magic.json` exists
/// there, loads the overlaid config (magic.json + magic.local.json), then
/// runs the existing `sync::apply::run` engine into `cwd`. No git/gh operations,
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
                tui::style::err(format!(
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
                tui::style::err(format!(
                    "error: cannot resolve main checkout root: {err:#}"
                ))
            );
            return Ok(ExitCode::from(1));
        }
    };

    // 3-4. Probe + load the overlaid magic.json (hard error on absent/malformed).
    let cfg = match load_magic_or_exit(&main_root) {
        Ok(c) => c,
        Err(code) => return Ok(code),
    };

    // 5. Empty files list → nothing to do, success.
    if cfg.files.is_empty() {
        println!(
            "{}",
            tui::style::info("magic.json `files` is empty — nothing to sync.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // 6. Run the apply engine: main_root → cwd_root.
    let summary = match sync::apply::run(&main_root, &cwd_root, &cfg.files, on_event) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{}", tui::style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
    };

    let line = format!(
        "Sync done: copied {} files, skipped {} files",
        summary.copied, summary.skipped
    );
    println!();
    if summary.skipped == 0 {
        println!("{}", tui::style::ok(line));
    } else {
        println!("{}", tui::style::warn(line));
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
    tui::style::print_section("Self-update");
    match update::update_command() {
        update::UpdateReport::Updated { version } => {
            println!("{}", tui::style::ok(format!("Updated to v{version}.")));
            Ok(ExitCode::SUCCESS)
        }
        update::UpdateReport::AlreadyLatest => {
            println!("{}", tui::style::info("Already on the latest release."));
            Ok(ExitCode::SUCCESS)
        }
        update::UpdateReport::Skipped => {
            println!(
                "{}",
                tui::style::warn(
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
            println!("{}", tui::style::info(format!("Copied: {}", rel.display())));
        }
        Event::Skip { reason, label } => {
            let line = format!("Skipped ({}): {label}", reason.label());
            if matches!(reason, SkipReason::Excluded) {
                println!("{}", tui::style::info(line));
            } else if matches!(reason, SkipReason::NoMatches) {
                // Default color, like setup.sh.
                println!("{line}");
            } else if reason.counts() {
                eprintln!("{}", tui::style::err(line));
            } else {
                eprintln!("{}", tui::style::warn(line));
            }
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", tui::style::err(format!("error: {err:#}")));
            ExitCode::from(1)
        }
    }
}
