//! Menu-driven interactive mode (U10).
//!
//! Bare invocation routes here instead of the old `bootstrap_flow`/`apply_flow`.
//! The menu is location-aware: main checkout vs worktree determines which
//! operations are offered. Within the main checkout, [`migrate::detect_branch`]
//! picks the migration/init/edit-config variant, and a local (uncommitted)
//! install – a `magic.local.json` with no `magic.json` – gets its own
//! [`Branch::Local`] row set.
//!
//! ## Structure: testable routing vs interactive TUI
//!
//! [`operations_for`] is pure and unit-tested: given a location and branch it
//! returns the ordered `Vec<MenuOp>` without touching the filesystem or the
//! terminal. The actual [`run`] function calls it, builds the `inquire` menu,
//! and dispatches the selected handler — that layer is manual-smoke, consistent
//! with the repo's final-action/TUI convention.
//!
//! ## Esc / Ctrl-C safety
//!
//! The outer `Select::prompt()` returns an error when the user cancels.
//! `run` matches on `Err` (the `inquire` cancel path) and returns
//! `Ok(ExitCode::SUCCESS)` — the working tree is left untouched.

use std::fmt;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use inquire::Select;

use crate::git;
use crate::workspace::local_install;
use crate::workspace::migrate::{self, Branch};
use crate::sync::reverse_sync;
use crate::tui::style;
use crate::workspace::superset_files;

// ── Operation enum ────────────────────────────────────────────────────────────

/// An operation the user can select from the interactive menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuOp {
    /// Main-checkout, Branch::Migrate: migrate the old `setup.sh` layout.
    Migrate,
    /// Main-checkout, Branch::Init: first-time initialization of the magic layout.
    Init,
    /// Main-checkout, Branch::Init: first-time LOCAL install – patterns in the
    /// gitignored `magic.local.json`, `ss-magic sync` registered in Superset's
    /// gitignored `config.local.json`, no tracked file changed.
    InitLocal,
    /// Main-checkout, Branch::Normal: edit the committed `magic.json` patterns.
    EditConfig,
    /// Main-checkout, Branch::Local: edit a local install's `magic.local.json`
    /// patterns (the selection replaces the list).
    EditConfigLocal,
    /// Worktree: the unified interactive sync cockpit — reconcile every
    /// configured file against main in either direction (push / pull / merge /
    /// delete per file).
    Sync,
    /// Archive the configured files into `ss-magic-<repo>.tar.bz2` at the git
    /// root (name derived from the normalized `origin` remote, falling back to
    /// the primary worktree basename — see `pack::archive_file_name`). Offered
    /// wherever a pattern list exists (any worktree, or the main checkout on a
    /// Normal or Local branch).
    Pack,
}

impl fmt::Display for MenuOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            MenuOp::Migrate => "Migrate to the magic.json layout",
            MenuOp::Init => "Initialize ss-magic",
            MenuOp::InitLocal => "Initialize ss-magic locally",
            MenuOp::EditConfig => "Edit synced files (magic.json)",
            MenuOp::EditConfigLocal => "Edit synced files (magic.local.json, local install)",
            MenuOp::Sync => "Sync with main (interactive — push, pull, merge, or delete per file)",
            MenuOp::Pack => "Pack configured files into a tar.bz2 archive",
        };
        f.write_str(label)
    }
}

// ── Pure routing helper ───────────────────────────────────────────────────────

/// Location context (main checkout vs worktree) for [`operations_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// We are in the main checkout.
    Main,
    /// We are in a linked worktree.
    Worktree,
}

/// Pure helper: given a location and a branch decision (irrelevant for a
/// worktree), return the ordered list of [`MenuOp`]s to offer.
///
/// `Pack` is offered wherever a pattern list exists — any worktree, and the
/// main checkout on a `Normal` (committed `magic.json`) or `Local`
/// (`magic.local.json` only) branch. `Init`/`Migrate` branches have no pattern
/// list yet, so `Pack` would have nothing to archive there.
///
/// Truth table:
///
/// | Location  | Branch            | Ops                                    |
/// |-----------|-------------------|----------------------------------------|
/// | Worktree  | (any)             | `[Sync, Pack]`                         |
/// | Main      | `Branch::Migrate` | `[Migrate]`                            |
/// | Main      | `Branch::Init`    | `[Init, InitLocal]`                    |
/// | Main      | `Branch::Normal`  | `[EditConfig, Pack]`                   |
/// | Main      | `Branch::Local`   | `[EditConfigLocal, Pack]`              |
pub fn operations_for(location: Location, branch: Branch) -> Vec<MenuOp> {
    match location {
        Location::Worktree => vec![MenuOp::Sync, MenuOp::Pack],
        Location::Main => match branch {
            Branch::Migrate => vec![MenuOp::Migrate],
            Branch::Init => vec![MenuOp::Init, MenuOp::InitLocal],
            Branch::Normal => vec![MenuOp::EditConfig, MenuOp::Pack],
            Branch::Local => vec![MenuOp::EditConfigLocal, MenuOp::Pack],
        },
    }
}

/// The handler a selected [`MenuOp`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    /// `migrate::run_migrate`.
    Migrate,
    /// `migrate::run_init`, first committed install.
    CommittedInit,
    /// [`edit_config`], editing a committed install's `magic.json`.
    EditCommitted,
    /// `local_install::run_local_init` – first local install and its edit
    /// entry alike, since the interactive local flow replaces the list.
    LocalInstall,
    /// `reverse_sync::run`, the unified worktree cockpit.
    Sync,
    /// `run_pack_flow`.
    Pack,
}

/// Pure helper: the handler for `op` at `location`, or `None` when that op is
/// never offered there. Split from the dispatch closures so a test can prove
/// every op [`operations_for`] offers has a handler, i.e. no selection can
/// reach the dispatcher's `unreachable!` arm.
pub fn handler_for(location: Location, op: MenuOp) -> Option<Handler> {
    match (location, op) {
        (Location::Worktree, MenuOp::Sync) => Some(Handler::Sync),
        (_, MenuOp::Pack) => Some(Handler::Pack),
        (Location::Main, MenuOp::Migrate) => Some(Handler::Migrate),
        (Location::Main, MenuOp::Init) => Some(Handler::CommittedInit),
        (Location::Main, MenuOp::EditConfig) => Some(Handler::EditCommitted),
        (Location::Main, MenuOp::InitLocal | MenuOp::EditConfigLocal) => {
            Some(Handler::LocalInstall)
        }
        _ => None,
    }
}

// ── Interactive entry point ───────────────────────────────────────────────────

/// Interactive menu entry point for `Command::Bare`.
///
/// Determines the location (main checkout vs worktree) and, for the main
/// checkout, reads `config.json` to decide which operations to offer.
/// Presents a `Select` prompt; Esc/Ctrl-C is inert (returns `Ok(SUCCESS)`).
pub fn run(cwd: &Path) -> Result<ExitCode> {
    // 1. Resolve the cwd repo root and location.
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

    let is_wt = git::is_worktree(&cwd_root).unwrap_or(false);

    if is_wt {
        // Worktree path: resolve main checkout root for handlers.
        let main_root = match git::main_checkout_root(&cwd_root) {
            Ok(r) => r,
            Err(err) => {
                eprintln!(
                    "{}",
                    style::err(format!("error: cannot resolve main checkout root: {err:#}"))
                );
                return Ok(ExitCode::from(1));
            }
        };

        let ops = operations_for(Location::Worktree, Branch::Init); // branch unused for worktree
        dispatch_menu(ops, |op| match handler_for(Location::Worktree, op) {
            Some(Handler::Sync) => reverse_sync::run(&cwd_root, &main_root),
            Some(Handler::Pack) => crate::run_pack_flow(cwd),
            _ => unreachable!("worktree only offers Sync/Pack"),
        })
    } else {
        // Main checkout path: read config.json to detect the branch.
        let config = match superset_files::load_config(&cwd_root) {
            Ok(opt) => opt,
            Err(err) => {
                // Malformed config.json → hard error naming the path (KTD8).
                eprintln!(
                    "{}",
                    style::err(format!("error: {err:#}"))
                );
                return Ok(ExitCode::from(1));
            }
        };

        let mode = superset_files::install_mode(&cwd_root);
        let branch = migrate::detect_branch(config.as_ref(), mode);
        let ops = operations_for(Location::Main, branch);
        // R13: a committed install beside a `config.local.json` that still
        // registers `ss-magic sync` runs the sync twice; say so up front.
        migrate::print_duplicate_sync_entry_warning(&cwd_root);

        let repo_root = cwd_root.clone();
        dispatch_menu(ops, move |op| match handler_for(Location::Main, op) {
            Some(Handler::Migrate) => migrate::run_migrate(&repo_root, config.as_ref().unwrap()),
            Some(Handler::CommittedInit) => migrate::run_init(&repo_root, config.as_ref()),
            Some(Handler::EditCommitted) => edit_config(&repo_root, config.as_ref()),
            Some(Handler::LocalInstall) => local_install::run_local_init(&repo_root),
            Some(Handler::Pack) => crate::run_pack_flow(&repo_root),
            _ => unreachable!("main checkout only offers Migrate/Init/EditConfig/local/Pack"),
        })
    }
}

/// Render the [`Select`] menu and dispatch to the handler, returning
/// `Ok(SUCCESS)` on Esc/Ctrl-C (cancel is inert).
fn dispatch_menu<F>(ops: Vec<MenuOp>, mut handler: F) -> Result<ExitCode>
where
    F: FnMut(MenuOp) -> Result<ExitCode>,
{
    let selected = Select::new("What would you like to do?", ops)
        .with_starting_cursor(0)
        .with_help_message("↑↓ navigate · enter to select · Esc to cancel")
        .prompt();

    match selected {
        Ok(op) => handler(op),
        Err(_) => {
            // Esc / Ctrl-C: leave the tree untouched.
            println!("{}", style::info("Cancelled — nothing changed."));
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ── Operation handlers ────────────────────────────────────────────────────────

/// Edit-config flow for `Branch::Normal` (already migrated).
///
/// Reuses `migrate::run_init` as the idempotent edit-config path: it opens the
/// pattern picker, lets the user adjust the `magic.json` files list, then runs
/// the finishing-action prompt. When called against an already-initialized repo,
/// `run_init` writes a new `magic.json` with the chosen patterns, which is
/// exactly the "edit synced files" semantic needed here. No separate function
/// is needed because `run_init` is already idempotent for the Normal case.
fn edit_config(repo_root: &Path, existing: Option<&superset_files::Config>) -> Result<ExitCode> {
    migrate::run_init(repo_root, existing)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
