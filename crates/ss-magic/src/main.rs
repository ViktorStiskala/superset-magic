use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};

mod cli;
mod pack;
mod sync;
mod tui;
mod update;
mod workspace;
#[cfg(test)]
mod tests;

// The shared plumbing lives in `ss-magic-core`; re-exported under the module
// names this crate used before the workspace split so every `crate::git::…`
// and `crate::hashing::…` path still resolves.
pub(crate) use ss_magic_core::{git, hashing};

use crate::sync::apply::{Event, SkipReason};
use crate::cli::{Command, Parsed};

/// Pure gate-decision helper (U8, AE3). Returns `true` when the auto-update
/// daily-cache gate should fire for the given command and guard state.
///
/// Truth table:
///
/// | cmd                          | guard_active | result |
/// |------------------------------|--------------|--------|
/// | `Bare`                       | false        | true   |
/// | `Sync`                       | false        | true   |
/// | `ReverseSync`                | false        | true   |
/// | `Pack`                       | false        | true   |
/// | `Update`                     | false        | false  |
/// | `Bare`/`Sync`/`ReverseSync`/`Pack` | true   | false  |
/// | `Update`                     | true         | false  |
///
/// `Command::Update` always bypasses the gate — it routes to the force path
/// (U7's `update_command`) which is NOT gated by the 24h cache. When the loop
/// guard is active (`SS_MAGIC_UPDATED` or `SS_MAGIC_NO_UPDATE` is set) the
/// gate never fires regardless of command, preventing re-exec loops (AE4).
///
/// `Command::Pack` and `Command::ReverseSync` are gated alongside `Bare`/`Sync`:
/// each is a non-interactive "do work" command, so gating keeps their users
/// self-updating.
///
/// Keep this an INCLUSION list. Inverting it to exclusions would mean every
/// future command self-updates unless someone remembers to opt it out, and the
/// list is short enough that naming each member costs nothing. The plugin used
/// to be the reason this mattered most — `ss-magic plugin …` had to reach its
/// verb tree without ever passing through here — and it is now a separate
/// binary that links no updater at all, so that concern is structural rather
/// than a rule this list has to keep.
pub fn should_run_update_gate(cmd: Command, guard_active: bool) -> bool {
    if guard_active {
        return false;
    }
    matches!(
        cmd,
        Command::Bare | Command::Sync { .. } | Command::ReverseSync { .. } | Command::Pack
    )
}

fn run() -> Result<ExitCode> {
    // Composition order: parse argv → style init → [help check] → [auto-update
    // gate for Bare/Sync] → dispatch (menu / sync / update). Parsing and the
    // help response happen before the gate so `--help` answers instantly
    // without a network call.
    //
    // Style is initialized AFTER the parse so `--help` and `--version` answer
    // without touching the terminal-detection path any earlier than needed. The
    // color decision lives in a `OnceLock`, so whichever call makes it first
    // wins for the whole process. The `inquire` prompt theme is a second step
    // on top of that decision (`tui::theme::install`), because the palette
    // lives in core where no prompt library is linked; only this binary drives
    // prompts, so only this binary installs the theme.
    let args: Vec<String> = env::args().skip(1).collect();
    let parsed = cli::parse(&args);
    tui::style::init();
    tui::theme::install();

    match parsed {
        // `--version`/`-V`: answer and stop. No network call, no dispatch —
        // a script shelling out to identify the binary must not trip an update
        // or open a menu it has no terminal for.
        Parsed::Version => {
            println!("{}", version_line());
            Ok(ExitCode::SUCCESS)
        }
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
        Parsed::Init(patterns) => init_noninteractive(&patterns, InitKind::Committed),
        // Non-interactive local install (`init --local`): same ungated
        // reasoning; it always lands in the MAIN checkout, also from a worktree.
        Parsed::InitLocal(patterns) => init_noninteractive(&patterns, InitKind::Local),
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

/// The single line `ss-magic --version` prints. Sourced from `Cargo.toml` at
/// compile time so it can never drift from the released version the self-update
/// path keys on.
pub fn version_line() -> String {
    format!("ss-magic {}", env!("CARGO_PKG_VERSION"))
}

/// Why the interactive menu cannot open, or `None` when it can.
///
/// The menu takes over the terminal for arrow-key selection, so both ends have
/// to be a real tty: with stdin redirected it would read a prompt answer out of
/// whatever is being piped in, and with stdout redirected it would write escape
/// sequences into a file and appear to hang. Split from the `IsTerminal` probe
/// so the decision is unit-testable without allocating a pty.
pub fn menu_blocked_reason(stdin_tty: bool, stdout_tty: bool) -> Option<&'static str> {
    match (stdin_tty, stdout_tty) {
        (true, true) => None,
        (false, true) => Some("stdin is not a terminal"),
        (true, false) => Some("stdout is not a terminal"),
        (false, false) => Some("neither stdin nor stdout is a terminal"),
    }
}

/// Which non-interactive init `ss-magic init` was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitKind {
    /// `init [PATTERN...]`: the committed `magic.json` layout.
    Committed,
    /// `init --local [PATTERN...]`: the local (uncommitted) install.
    Local,
}

/// Non-interactive `ss-magic init [--local] [PATTERN...]` (AN1): seed the
/// layout from CLI-supplied patterns without the TUI, so automation (CI,
/// Superset provisioning) can bootstrap a repo. A committed init operates on
/// the current checkout root; a local install resolves the main checkout from
/// it (`workspace::local_install`), because that is where Superset and sync
/// read a local install from.
fn init_noninteractive(patterns: &[String], kind: InitKind) -> Result<ExitCode> {
    let cwd = env::current_dir().context("getting current directory")?;
    match git::cwd_repo_root(&cwd) {
        Ok(repo_root) => match kind {
            InitKind::Committed => {
                workspace::migrate::run_init_noninteractive(&repo_root, patterns)
            }
            InitKind::Local => {
                workspace::local_install::run_local_init_noninteractive(&repo_root, patterns)
            }
        },
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
/// location-aware operation menu (U10) once both terminal ends check out;
/// `Sync`/`Update` route to their respective handlers.
fn dispatch(cmd: Command) -> Result<ExitCode> {
    let cwd = env::current_dir().context("getting current directory")?;
    match cmd {
        Command::Bare => {
            use std::io::IsTerminal;
            if let Some(reason) =
                menu_blocked_reason(std::io::stdin().is_terminal(), std::io::stdout().is_terminal())
            {
                eprintln!(
                    "{}",
                    tui::style::err(format!(
                        "error: cannot open the interactive menu — {reason}."
                    ))
                );
                eprintln!(
                    "{}",
                    tui::style::info(
                        "Run `ss-magic --help` for the non-interactive commands."
                    )
                );
                return Ok(ExitCode::from(2));
            }
            tui::menu::run(&cwd)
        }
        Command::Sync { no_backup } => run_sync_flow(&cwd, no_backup),
        Command::ReverseSync { no_backup } => run_reverse_sync_flow(&cwd, no_backup),
        Command::Pack => run_pack_flow(&cwd),
        Command::Update => update_flow(),
    }
}

/// The error printed when `root` has neither pattern file: names both
/// `magic.json` (a committed install) and `magic.local.json` (a local install)
/// and both ways to create one. Pure so the wording is testable without
/// capturing stderr.
pub fn no_sync_config_message(root: &Path) -> String {
    format!(
        "error: no `.superset/magic.json` or `.superset/magic.local.json` in {}; \
         run `ss-magic init` (committed) or `ss-magic init --local` (local install) \
         in the main checkout first",
        root.display()
    )
}

/// Load the sync pattern list for `root`, printing a styled error and
/// returning the exit code on absence or malformation. Shared by the
/// forward-sync (`sync_core`) and pack (`pack::pack_core`) flows so the
/// "no pattern list / malformed" error path lives in exactly one place.
///
/// Goes through `superset_files::load_sync_config` (KTD1 in the local-install
/// plan: the install mode is derived from which files exist): the overlay of
/// `magic.json` and `magic.local.json` on a committed install, and
/// `magic.local.json` alone on a local install, which has no `magic.json` at
/// all. `Ok(None)` – neither file – is the [`no_sync_config_message`] error.
pub fn load_magic_or_exit(root: &Path) -> std::result::Result<workspace::superset_files::MagicConfig, ExitCode> {
    match workspace::superset_files::load_sync_config(root) {
        Ok(Some(cfg)) => Ok(cfg),
        Ok(None) => {
            eprintln!("{}", tui::style::err(no_sync_config_message(root)));
            Err(ExitCode::from(1))
        }
        Err(err) => {
            eprintln!("{}", tui::style::err(format!("error: {err:#}")));
            Err(ExitCode::from(1))
        }
    }
}

/// Non-interactive pack: archive the files defined by the sync pattern list
/// (`load_magic_or_exit`: committed overlay, or a local install's
/// `magic.local.json`) into `ss-magic-<repo>.tar.bz2` at the git root (name derived from the
/// normalized origin remote). Handler for `ss-magic pack`
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
                tui::style::ok(format!("Packed {count} files → {}", real.display()))
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
/// Handler for `ss-magic sync` (the worktree menu now routes to the interactive
/// unified cockpit instead).
///
/// Resolves the main checkout root, loads its sync pattern list
/// (`load_magic_or_exit`: magic.json + magic.local.json on a committed install,
/// magic.local.json alone on a local one), backs up
/// every worktree file about to be overwritten (under `<cwd>/.superset/backups/`,
/// unless `no_backup`), then runs the existing `sync::apply::run` engine into
/// `cwd`. No git/gh operations, no setup commands.
///
/// Hard errors (non-zero exit):
/// - Cannot resolve the main checkout root (not in a git repo, or git fails).
/// - Neither `.superset/magic.json` nor `.superset/magic.local.json` in the
///   resolved main root.
/// - Malformed `magic.json` or `magic.local.json` in the main root.
pub fn run_sync_flow(cwd: &Path, no_backup: bool) -> Result<ExitCode> {
    sync_core(cwd, no_backup, print_event)
}

/// Resolve the current repo root and the main checkout root for the sync flows,
/// printing the same styled error and returning the exit code on failure. Shared
/// by `sync_core` (forward) and `run_reverse_sync_flow` (reverse).
fn resolve_sync_roots(cwd: &Path) -> std::result::Result<(PathBuf, PathBuf), ExitCode> {
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
            return Err(ExitCode::from(1));
        }
    };
    let main_root = match git::main_checkout_root(&cwd_root) {
        Ok(r) => r,
        Err(err) => {
            eprintln!(
                "{}",
                tui::style::err(format!("error: cannot resolve main checkout root: {err:#}"))
            );
            return Err(ExitCode::from(1));
        }
    };
    Ok((cwd_root, main_root))
}

/// Extracted core of `sync_flow` so tests can inject a no-op event handler
/// without side-effects on stdout/stderr.
fn sync_core<F>(cwd: &Path, no_backup: bool, on_event: F) -> Result<ExitCode>
where
    F: FnMut(&Event),
{
    // 1-2. Resolve the current repo root and the main checkout root.
    let (cwd_root, main_root) = match resolve_sync_roots(cwd) {
        Ok(roots) => roots,
        Err(code) => return Ok(code),
    };

    // 3-4. Load the sync pattern list (hard error on absent/malformed).
    let cfg = match load_magic_or_exit(&main_root) {
        Ok(c) => c,
        Err(code) => return Ok(code),
    };

    // 5. Empty files list → nothing to do, success.
    if cfg.files.is_empty() {
        println!(
            "{}",
            tui::style::info("the configured `files` list is empty — nothing to sync.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // 5b. Pre-copy backup pass: back up every worktree file the copy will
    // overwrite (under `<cwd>/.superset/backups/`, gitignored there) so a
    // mistaken forward sync is recoverable. Skipped by `--no-backup`.
    if !no_backup {
        if let Err(err) =
            sync::reverse_sync::backup_forward_targets(&main_root, &cwd_root, &cfg.files)
        {
            eprintln!("{}", tui::style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
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

/// Non-interactive reverse copy (current worktree → main). Handler for
/// `ss-magic reverse-sync`.
///
/// Resolves the current repo root and the main checkout root, hard-errors when
/// run FROM the main checkout (`cwd_root == main_root` — there is nothing to
/// push), then bulk-pushes every git-untracked candidate that differs from main
/// via `sync::reverse_sync::run_bulk` (pre-overwrite backups under main's
/// `.superset/backups/` unless `no_backup`, plus the gitignore-in-main secret
/// gate on every write).
pub fn run_reverse_sync_flow(cwd: &Path, no_backup: bool) -> Result<ExitCode> {
    let (cwd_root, main_root) = match resolve_sync_roots(cwd) {
        Ok(roots) => roots,
        Err(code) => return Ok(code),
    };
    if cwd_root == main_root {
        eprintln!(
            "{}",
            tui::style::err(
                "error: `ss-magic reverse-sync` must run from a worktree, not the main \
                 checkout — there is nothing to push."
            )
        );
        return Ok(ExitCode::from(1));
    }
    sync::reverse_sync::run_bulk(&cwd_root, &main_root, no_backup)
}

/// `ss-magic update`: force a self-update regardless of the 24h cache.
///
/// Routes straight to the forced apply path, which resolves the CLI line's
/// newest release from GitHub without the daily cache, runs the `self_update`
/// lock/download/swap pinned to that tag if it is newer, and reports the
/// resulting version, "already latest", or – when the release list could not
/// be fetched – that it could not check at all (the two are deliberately
/// distinct: offline is not "up to date"). Unlike the bare/sync auto-update
/// gate, this does not re-exec — the update itself is the requested work.
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
        update::UpdateReport::Unavailable => {
            println!(
                "{}",
                tui::style::warn("Could not check for a release; try again.")
            );
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
