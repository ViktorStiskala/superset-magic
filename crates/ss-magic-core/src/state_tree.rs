//! The plugin's per-worktree state tree, as the ONE place its path is spelled.
//!
//! `.superset/.magic/` holds everything the Claude Code plugin keeps for a
//! worktree – session scratchpads, the conclusion cache, the one-shot stores.
//! It is machine-local and gitignored, and the plugin's hooks refuse to write
//! any of it while git does not report the tree ignored. Two different
//! programs therefore need the same two facts: the path, and how to add the
//! gitignore rule for it.
//!
//! - The `ss-magic` CLI writes the rule EAGERLY from `init` and `migrate`
//!   (`workspace/migrate.rs::ensure_bootstrap_gitignores`), so a repository is
//!   protected before any plugin state exists.
//! - The plugin's `enable` and `config set plugin.enabled true` write it
//!   LAZILY, for a repository that was initialized before the plugin existed.
//! - No hook ever writes it – a hook that could edit `.gitignore` would be a
//!   hook that dirties the user's working tree behind their back.
//!
//! Before the workspace split the CLI reached this function through the
//! plugin's `scratchpad` module, the one place CLI code depended on plugin
//! code. Owning it here removes that reverse dependency; `scratchpad`
//! re-exports both items so the plugin's callers are unchanged.
//!
//! The same path also appears in [`crate::sync::EXCLUDED_TREES`], where it
//! keeps the tree out of every sync and pack walk. That is the sync-exclusion
//! control; this module is the gitignore side. A test pins the two spellings
//! equal so they cannot drift apart.

use std::path::Path;

use anyhow::Result;

use crate::git::gitignore::{self, PathKind};

/// Repo-relative path of the plugin's state tree.
pub const STATE_REL: &str = ".superset/.magic";

/// Ensure git ignores `.superset/.magic/` under `root`, adding a `Dir` rule
/// only when it does not already. Idempotent, and lands in the closest
/// EXISTING `.gitignore` among the path's ancestors — the repository root
/// ordinarily, but `.superset/.gitignore` in a repo that carries one.
///
/// The ONE place this rule is written; see the module doc for who calls it
/// and when. Git-tolerant like every `ensure_path_ignored` caller: a git
/// failure reads as "not ignored" and the literal rule is appended.
pub fn ensure_state_ignored(root: &Path) -> Result<()> {
    gitignore::ensure_path_ignored(root, root, Path::new(STATE_REL), PathKind::Dir)?;
    Ok(())
}

#[cfg(test)]
mod tests;
