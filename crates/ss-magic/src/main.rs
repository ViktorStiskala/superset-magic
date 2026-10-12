use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};

mod cli;
mod orca;
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

use crate::sync::apply::{Event, PathFilter, SkipReason, Summary};
use crate::cli::{Command, Parsed};
use crate::orca::detect::EnvLookup;

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
/// | `Orca`                       | false        | true   |
/// | `Update`                     | false        | false  |
/// | `Bare`/`Sync`/`ReverseSync`/`Pack`/`Orca` | true | false |
/// | `Update`                     | true         | false  |
///
/// `Command::Update` always bypasses the gate — it routes to the force path
/// (U7's `update_command`) which is NOT gated by the 24h cache. When the loop
/// guard is active (`SS_MAGIC_UPDATED` or `SS_MAGIC_NO_UPDATE` is set) the
/// gate never fires regardless of command, preventing re-exec loops (AE4).
///
/// `Command::Pack`, `Command::ReverseSync` and `Command::Orca` are gated
/// alongside `Bare`/`Sync`: each is a non-interactive "do work" command, so
/// gating keeps their users self-updating.
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
        Command::Bare
            | Command::Sync { .. }
            | Command::ReverseSync { .. }
            | Command::Pack
            | Command::Orca
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
        Command::Sync {
            no_backup,
            no_orca_skip,
        } => run_sync_flow(
            &cwd,
            SyncOptions {
                no_backup,
                no_orca_skip,
            },
        ),
        Command::ReverseSync { no_backup } => run_reverse_sync_flow(&cwd, no_backup),
        Command::Pack => run_pack_flow(&cwd),
        Command::Orca => run_orca_flow(&cwd),
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
/// into `ss-magic-<repo>.tar.bz2` at the git root (name derived from the
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

/// Explicit Orca refresh: regenerate the main checkout's `orca.yaml` shared
/// directories and `.worktreeinclude` from `magic.json`. Handler for
/// `ss-magic orca` and the main-checkout menu's "Refresh Orca files" entry.
///
/// Runs from any directory of the repository – a worktree included – and always
/// writes into the MAIN checkout, since that is where Orca reads both files
/// from. This is also how a repository set up before Orca support gains the
/// files in one step.
pub fn run_orca_flow(cwd: &Path) -> Result<ExitCode> {
    tui::style::print_section("Refresh Orca files");
    match orca_core(cwd) {
        Ok(notices) => {
            orca::write::print_notices(&notices);
            Ok(ExitCode::SUCCESS)
        }
        Err(code) => Ok(code),
    }
}

/// Core of [`run_orca_flow`], returning the notices instead of printing them
/// so tests can read what happened. `Err` carries the exit code of a hard
/// error, already reported on stderr: the repository roots cannot be resolved,
/// or the main checkout has no (or a malformed) `.superset/magic.json` – in
/// which case nothing is written.
///
/// What the refresh does and does not touch:
///
/// - `worktree.sharedDirectories` is replaced by the expansion of the patterns
///   stored under `sharedDirectories` in the committed `magic.json`. When that
///   key is absent nobody has chosen yet, so the list is written empty and a
///   notice points at edit-config; `magic.json` itself is not changed here.
/// - `scripts.setup` of an EXISTING `orca.yaml` is left exactly as it is, so a
///   line someone removed on purpose stays removed (R25). Only when the refresh
///   creates the file does it include the `magic.sh sync` line.
/// - `.worktreeinclude` gets a fresh snapshot and, unless it is tracked, its
///   ignore rule – which is what lets the automatic end-of-sync refresh run in
///   a repository that never had the rule.
///
/// An `orca.yaml` that cannot be edited safely is reported and left alone; that
/// is a notice, not a failure.
fn orca_core(cwd: &Path) -> std::result::Result<Vec<orca::write::Notice>, ExitCode> {
    use orca::write::{Level, Notice, SetupPolicy};

    let (_cwd_root, main_root) = resolve_sync_roots(cwd)?;
    let cfg = load_magic_or_exit(&main_root)?;

    let shared = cfg.shared_directories.clone().unwrap_or_default();
    let plan = orca::write::plan(&main_root, &shared, SetupPolicy::OnlyWhenCreating);
    let mut notices = plan.apply(&main_root);
    if cfg.shared_directories.is_none() {
        notices.push(Notice {
            level: Level::Info,
            text: "magic.json has no `sharedDirectories` yet, so no directories are shared                    with Orca worktrees. Run `ss-magic` in the main checkout and pick                    \"Edit synced files\" to choose them."
                .to_string(),
        });
    }
    Ok(notices)
}

/// Non-interactive forward file copy: main checkout → current working tree.
/// Handler for `ss-magic sync` (the worktree menu now routes to the interactive
/// unified cockpit instead).
///
/// Resolves the main checkout root, verifies `.superset/magic.json` exists
/// there, loads the overlaid config (magic.json + magic.local.json), backs up
/// every worktree file about to be overwritten (under `<cwd>/.superset/backups/`,
/// unless `no_backup`), then runs the existing `sync::apply::run` engine into
/// `cwd`. No git/gh operations, no setup commands.
///
/// Hard errors (non-zero exit):
/// - Cannot resolve the main checkout root (not in a git repo, or git fails).
/// - `.superset/magic.json` absent in the resolved main root.
/// - Malformed `magic.json` or `magic.local.json` in the main root.
///
/// In an Orca worktree (see `orca::detect`) the copy leaves alone the paths
/// Orca already copied from the main checkout's `.worktreeinclude`, and a
/// successful copy is followed by a best-effort refresh of that file's managed
/// section. Outside an Orca worktree none of that happens.
pub fn run_sync_flow(cwd: &Path, opts: SyncOptions) -> Result<ExitCode> {
    sync_core(cwd, opts, &|name| env::var_os(name), print_event)
}

/// What `ss-magic sync` was asked to do besides the copy itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncOptions {
    /// `--no-backup`: skip the pre-overwrite backup pass.
    pub no_backup: bool,
    /// `--no-orca-skip`: in an Orca worktree, copy the `.worktreeinclude`
    /// paths as well instead of leaving them to Orca.
    pub no_orca_skip: bool,
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
///
/// `env` is how the Orca variables are read. Production passes the process
/// environment; tests pass a closure, so a test neither depends on the
/// variables of the terminal it happens to run in (which may itself be an Orca
/// terminal) nor has to set process-wide ones that parallel tests would see.
fn sync_core<F>(cwd: &Path, opts: SyncOptions, env: EnvLookup<'_>, on_event: F) -> Result<ExitCode>
where
    F: FnMut(&Event),
{
    // 1-2. Resolve the current repo root and the main checkout root.
    let (cwd_root, main_root) = match resolve_sync_roots(cwd) {
        Ok(roots) => roots,
        Err(code) => return Ok(code),
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

    // 5a. Orca: in an Orca worktree, the paths Orca already copied from the
    // main checkout's `.worktreeinclude` are left alone. The list is read HERE,
    // once, before anything is copied – and so before the refresh at the end
    // rewrites it. A path that refresh adds was not in the list Orca used, so
    // it must not be skipped on the strength of it (R16).
    let in_orca = orca::detect::is_orca_worktree(&cwd_root, &main_root, env);
    let filter = if in_orca && !opts.no_orca_skip {
        orca_skip_filter(&main_root, &cwd_root)
    } else {
        PathFilter::empty()
    };

    // 5b. Pre-copy backup pass: back up every worktree file the copy will
    // overwrite (under `<cwd>/.superset/backups/`, gitignored there) so a
    // mistaken forward sync is recoverable. Skipped by `--no-backup`. It runs
    // with the copy's own filter: a path left to Orca is not overwritten, so it
    // is not backed up either.
    if !opts.no_backup {
        if let Err(err) = sync::reverse_sync::backup_forward_targets(
            &main_root,
            &cwd_root,
            &cfg.files,
            &filter,
        ) {
            eprintln!("{}", tui::style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
    }

    // 6. Run the apply engine: main_root → cwd_root.
    let summary =
        match sync::apply::run_filtered(&main_root, &cwd_root, &cfg.files, &filter, on_event) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("{}", tui::style::err(format!("error: {err:#}")));
                return Ok(ExitCode::from(1));
            }
        };

    let line = sync_summary_line(&summary);
    println!();
    if summary.skipped == 0 {
        println!("{}", tui::style::ok(line));
    } else {
        println!("{}", tui::style::warn(line));
    }

    // 7. Orca: the copy succeeded, so bring the main checkout's snapshot up to
    // date for the NEXT worktree. Best-effort – whatever it reports, the exit
    // status of the sync stands (R26). It runs with `--no-orca-skip` too: that
    // flag is about what this run copies, not about keeping the list current.
    if in_orca {
        report_snapshot_refresh(&orca::write::refresh_snapshot(&main_root, &cfg.files));
    }

    Ok(ExitCode::SUCCESS)
}

/// The one-line result of a forward sync. Paths left to Orca are reported apart
/// from skips: they are not a problem and never turn the line into a warning.
fn sync_summary_line(summary: &Summary) -> String {
    let mut line = format!(
        "Sync done: copied {} files, skipped {} files",
        summary.copied, summary.skipped
    );
    if summary.filtered > 0 {
        line.push_str(&format!(
            ", left {} {} to Orca",
            summary.filtered,
            if summary.filtered == 1 { "path" } else { "paths" }
        ));
    }
    line
}

/// The paths this Orca worktree already has from Orca: every entry of the main
/// checkout's `.worktreeinclude` – the managed section and the hand-written
/// lines alike – that exists here. A file that cannot be read yields an empty
/// set plus a warning, so the sync copies everything itself rather than trust a
/// list it could not see.
fn orca_skip_filter(main_root: &Path, cwd_root: &Path) -> PathFilter {
    use orca::worktreeinclude::{self, Existing};
    match worktreeinclude::read(main_root) {
        Existing::Absent => PathFilter::empty(),
        Existing::Text(text) => {
            worktreeinclude::copied_by_orca(cwd_root, &worktreeinclude::entries(&text))
        }
        Existing::Unreadable(why) => {
            eprintln!(
                "{}",
                tui::style::warn(format!(
                    "Orca: the main checkout's `.worktreeinclude` was not used because {why};                      nothing is skipped."
                ))
            );
            PathFilter::empty()
        }
    }
}

/// Print what the end-of-sync snapshot refresh did. Never an error: a refresh
/// that could not run is a warning, and the sync has already succeeded.
fn report_snapshot_refresh(outcome: &orca::write::SnapshotRefresh) {
    use orca::write::SnapshotRefresh;
    let warning = match outcome {
        SnapshotRefresh::Written { listed, warning } => {
            println!(
                "{}",
                tui::style::info(format!(
                    "Orca: refreshed `.worktreeinclude` in the main checkout ({listed} {} listed).",
                    if *listed == 1 { "file" } else { "files" }
                ))
            );
            warning
        }
        SnapshotRefresh::Unchanged { warning } => warning,
        SnapshotRefresh::Skipped(why) => {
            eprintln!(
                "{}",
                tui::style::warn(format!("Orca: `.worktreeinclude` was not refreshed: {why}"))
            );
            &None
        }
    };
    if let Some(warning) = warning {
        eprintln!("{}", tui::style::warn(warning));
    }
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
            if matches!(reason, SkipReason::Filtered) {
                // Not a problem: Orca already put this path in the worktree, so
                // the copy left it alone. Worded for what happened rather than
                // with the engine's generic "filtered" label.
                println!("{}", tui::style::info(format!("Left to Orca: {label}")));
            } else if matches!(reason, SkipReason::Excluded) {
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
