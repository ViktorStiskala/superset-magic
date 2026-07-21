//! Menu-driven interactive mode (U10).
//!
//! Bare invocation routes here instead of the old `bootstrap_flow`/`apply_flow`.
//! The menu is location-aware: main checkout vs worktree determines which
//! operations are offered. Within the main checkout, [`migrate::detect_branch`]
//! picks the migration/init/edit-config variant.
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
    /// Main-checkout, Branch::Normal: edit the committed `magic.json` patterns.
    EditConfig,
    /// Worktree: the unified interactive sync cockpit — reconcile every
    /// configured file against main in either direction (push / pull / merge /
    /// delete per file).
    Sync,
    /// Archive the configured files into `ss-magic-<repo>.tar.bz2` at the git
    /// root (name derived from the normalized `origin` remote, falling back to
    /// the primary worktree basename — see `pack::archive_file_name`). Offered
    /// wherever an initialized `magic.json` exists (any worktree, or the main
    /// checkout on a Normal branch).
    Pack,
}

impl fmt::Display for MenuOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            MenuOp::Migrate => "Migrate to the magic.json layout",
            MenuOp::Init => "Initialize ss-magic",
            MenuOp::EditConfig => "Edit synced files (magic.json)",
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
/// `Pack` is offered wherever an initialized `magic.json` exists — any worktree,
/// and the main checkout on a `Normal` branch. `Init`/`Migrate` branches have no
/// `magic.json` yet, so `Pack` would have nothing to archive there.
///
/// Truth table:
///
/// | Location  | Branch            | Ops                                    |
/// |-----------|-------------------|----------------------------------------|
/// | Worktree  | (any)             | `[Sync, Pack]`                         |
/// | Main      | `Branch::Migrate` | `[Migrate]`                            |
/// | Main      | `Branch::Init`    | `[Init]`                               |
/// | Main      | `Branch::Normal`  | `[EditConfig, Pack]`                   |
pub fn operations_for(location: Location, branch: Branch) -> Vec<MenuOp> {
    match location {
        Location::Worktree => vec![MenuOp::Sync, MenuOp::Pack],
        Location::Main => match branch {
            Branch::Migrate => vec![MenuOp::Migrate],
            Branch::Init => vec![MenuOp::Init],
            Branch::Normal => vec![MenuOp::EditConfig, MenuOp::Pack],
        },
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
        dispatch_menu(ops, |op| match op {
            MenuOp::Sync => reverse_sync::run(&cwd_root, &main_root),
            MenuOp::Pack => crate::run_pack_flow(cwd),
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

        let branch = migrate::detect_branch(config.as_ref());
        let ops = operations_for(Location::Main, branch);

        let repo_root = cwd_root.clone();
        dispatch_menu(ops, move |op| match op {
            MenuOp::Migrate => migrate::run_migrate(&repo_root, config.as_ref().unwrap()),
            MenuOp::Init => migrate::run_init(&repo_root, config.as_ref()),
            MenuOp::EditConfig => edit_config(&repo_root, config.as_ref()),
            MenuOp::Pack => crate::run_pack_flow(&repo_root),
            _ => unreachable!("main checkout only offers Migrate/Init/EditConfig/Pack"),
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
