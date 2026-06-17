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
use crate::migrate::{self, Branch};
use crate::reverse_sync;
use crate::style;
use crate::superset_files;

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
    /// Worktree: non-interactive forward copy (main → this worktree).
    ForwardSync,
    /// Worktree: push git-untracked files back to main.
    ReverseSync,
}

impl fmt::Display for MenuOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            MenuOp::Migrate => "Migrate to the magic.json layout",
            MenuOp::Init => "Initialize ss-magic",
            MenuOp::EditConfig => "Edit synced files (magic.json)",
            MenuOp::ForwardSync => "Forward sync (copy files from main to this worktree)",
            MenuOp::ReverseSync => "Reverse sync (push untracked files to main)",
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
/// Truth table:
///
/// | Location  | Branch            | Ops                                    |
/// |-----------|-------------------|----------------------------------------|
/// | Worktree  | (any)             | `[ForwardSync, ReverseSync]`           |
/// | Main      | `Branch::Migrate` | `[Migrate]`                            |
/// | Main      | `Branch::Init`    | `[Init]`                               |
/// | Main      | `Branch::Normal`  | `[EditConfig]`                         |
pub fn operations_for(location: Location, branch: Branch) -> Vec<MenuOp> {
    match location {
        Location::Worktree => vec![MenuOp::ForwardSync, MenuOp::ReverseSync],
        Location::Main => match branch {
            Branch::Migrate => vec![MenuOp::Migrate],
            Branch::Init => vec![MenuOp::Init],
            Branch::Normal => vec![MenuOp::EditConfig],
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
            MenuOp::ForwardSync => forward_sync_in_worktree(cwd),
            MenuOp::ReverseSync => reverse_sync::run(&cwd_root, &main_root),
            _ => unreachable!("worktree only offers ForwardSync/ReverseSync"),
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
            _ => unreachable!("main checkout only offers Migrate/Init/EditConfig"),
        })
    }
}

/// Render the [`Select`] menu and dispatch to the handler, returning
/// `Ok(SUCCESS)` on Esc/Ctrl-C (cancel is inert).
fn dispatch_menu<F>(ops: Vec<MenuOp>, mut handler: F) -> Result<ExitCode>
where
    F: FnMut(MenuOp) -> Result<ExitCode>,
{
    let len = ops.len();
    let selected = Select::new("What would you like to do?", ops)
        .with_starting_cursor(0_usize.min(len.saturating_sub(1)))
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

/// Forward sync from inside a worktree: resolves the main checkout root and
/// runs `sync_core` (the U4 path, shared with `ss-magic sync`).
fn forward_sync_in_worktree(cwd: &Path) -> Result<ExitCode> {
    // Re-use the public sync_core via the module path. `sync_core` lives in
    // `main.rs` and is not re-exported; call the top-level `sync_flow` instead,
    // which wraps it with `print_event` — exactly what the non-interactive
    // `sync` subcommand does.
    //
    // Because `menu.rs` cannot call `crate::sync_core` directly (it is a
    // private fn in `main.rs`), we delegate to `crate::sync_flow_for_cwd`,
    // which is a thin re-export added in main.rs for this purpose.
    crate::run_sync_flow(cwd)
}

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
mod tests {
    use super::*;

    // ── operations_for: location gating ──────────────────────────────────────

    /// Worktree always gets ForwardSync + ReverseSync, regardless of the
    /// branch value passed (branch is irrelevant for a worktree).
    #[test]
    fn worktree_ops_are_forward_and_reverse_sync() {
        for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
            let ops = operations_for(Location::Worktree, branch);
            assert_eq!(
                ops,
                vec![MenuOp::ForwardSync, MenuOp::ReverseSync],
                "worktree branch={branch:?} must offer ForwardSync + ReverseSync"
            );
        }
    }

    /// Main checkout never offers ForwardSync or ReverseSync.
    #[test]
    fn main_checkout_ops_never_include_worktree_ops() {
        for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
            let ops = operations_for(Location::Main, branch);
            assert!(
                !ops.contains(&MenuOp::ForwardSync),
                "main checkout must not offer ForwardSync; branch={branch:?}"
            );
            assert!(
                !ops.contains(&MenuOp::ReverseSync),
                "main checkout must not offer ReverseSync; branch={branch:?}"
            );
        }
    }

    // ── operations_for: main-checkout branch → op mapping ────────────────────

    /// Branch::Migrate → exactly [Migrate].
    #[test]
    fn migrate_branch_offers_migrate_op() {
        let ops = operations_for(Location::Main, Branch::Migrate);
        assert_eq!(ops, vec![MenuOp::Migrate]);
    }

    /// Branch::Init → exactly [Init].
    #[test]
    fn init_branch_offers_init_op() {
        let ops = operations_for(Location::Main, Branch::Init);
        assert_eq!(ops, vec![MenuOp::Init]);
    }

    /// Branch::Normal → exactly [EditConfig].
    #[test]
    fn normal_branch_offers_edit_config_op() {
        let ops = operations_for(Location::Main, Branch::Normal);
        assert_eq!(ops, vec![MenuOp::EditConfig]);
    }

    // ── Invariant: every op belongs to exactly one location ──────────────────

    /// Main-checkout ops are a subset of {Migrate, Init, EditConfig}.
    #[test]
    fn main_checkout_ops_are_main_only() {
        let main_ops: std::collections::HashSet<MenuOp> =
            [Branch::Migrate, Branch::Init, Branch::Normal]
                .iter()
                .flat_map(|&b| operations_for(Location::Main, b))
                .collect();
        let worktree_ops: std::collections::HashSet<MenuOp> =
            operations_for(Location::Worktree, Branch::Init)
                .into_iter()
                .collect();
        // The two sets must be disjoint.
        let overlap: Vec<_> = main_ops.intersection(&worktree_ops).collect();
        assert!(
            overlap.is_empty(),
            "main-only and worktree-only ops must not overlap; overlap={overlap:?}"
        );
    }
}
