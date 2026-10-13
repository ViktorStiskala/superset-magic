//! Is ss-magic running inside an Orca worktree?
//!
//! Orca behavior – skipping what Orca copied, refreshing main's snapshot –
//! must switch on only for the worktree Orca actually set up. Orca's variables
//! merely EXISTING is not that signal: a terminal opened by Orca keeps them
//! when the person changes into another repository's worktree, and running
//! there must behave exactly as it does today (AE3). So the test is whether one
//! of the variables NAMES the worktree ss-magic is running in (R15):
//!
//! - `ORCA_WORKTREE_PATH`, which Orca sets for setup-script runs, holds the
//!   worktree's absolute path.
//! - `ORCA_WORKTREE_ID`, which Orca sets in every terminal it opens, is
//!   `<repo id>::<absolute worktree path>`, optionally followed by
//!   `::workspace:<uuid>`.
//!
//! Either naming the current worktree root is enough. The main checkout is
//! never an Orca worktree for these purposes, even when a variable names it
//! (R24): there is nothing for Orca to have copied into it, and its
//! `.worktreeinclude` is the list being maintained, not a list to obey.
//!
//! The environment is passed in as a lookup (KTD9), the same shape
//! `git::discover::discover_with_env` uses. Tests call the function with a
//! closure instead of setting process-wide variables, which every other test's
//! child processes would inherit.

use std::ffi::OsString;
use std::path::Path;

/// Set by Orca for setup and archive script runs: the worktree being set up.
pub const WORKTREE_PATH_VAR: &str = "ORCA_WORKTREE_PATH";

/// Set by Orca in every terminal it opens: `<repo id>::<worktree path>`.
pub const WORKTREE_ID_VAR: &str = "ORCA_WORKTREE_ID";

/// The separator between the repository id and the path in a worktree id.
const ID_SEPARATOR: &str = "::";

/// The marker of the optional suffix a worktree id can carry after its path.
const WORKSPACE_SUFFIX: &str = "::workspace:";

/// How the environment is read: a variable's value, or `None` when unset.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// Whether `cwd_root` is an Orca worktree of the repository whose main
/// checkout is `main_root`.
///
/// Both roots and the path a variable names are canonicalized before they are
/// compared, so a symlinked spelling of the same directory (macOS's `/var` for
/// `/private/var`) still matches. Anything that cannot be resolved – a root
/// that no longer exists, a variable naming a deleted directory – is "not an
/// Orca worktree", which leaves forward sync behaving exactly as it does
/// outside Orca.
pub fn is_orca_worktree(cwd_root: &Path, main_root: &Path, env: EnvLookup<'_>) -> bool {
    let Ok(here) = cwd_root.canonicalize() else {
        return false;
    };
    match main_root.canonicalize() {
        Ok(main) if main != here => {}
        _ => return false,
    }
    let names_here = |candidate: &Path| {
        // A relative value would be resolved against this process's working
        // directory, which says nothing about what Orca meant.
        candidate.is_absolute() && candidate.canonicalize().is_ok_and(|path| path == here)
    };

    if env(WORKTREE_PATH_VAR).is_some_and(|value| names_here(Path::new(&value))) {
        return true;
    }
    env(WORKTREE_ID_VAR)
        .as_deref()
        .and_then(|id| id.to_str())
        .and_then(worktree_path_of_id)
        .is_some_and(|path| names_here(Path::new(path)))
}

/// The worktree path inside an `ORCA_WORKTREE_ID`: everything after the FIRST
/// `::` (the path itself may contain the separator), minus a trailing
/// `::workspace:<uuid>` when present. `None` for an id with no separator, which
/// is how Orca identifies a folder workspace that is not a git worktree.
fn worktree_path_of_id(id: &str) -> Option<&str> {
    let (_, rest) = id.split_once(ID_SEPARATOR)?;
    let path = match rest.rfind(WORKSPACE_SUFFIX) {
        Some(at) if is_uuid(&rest[at + WORKSPACE_SUFFIX.len()..]) => &rest[..at],
        _ => rest,
    };
    (!path.is_empty()).then_some(path)
}

/// Whether `text` has the shape Orca's own parser strips: 36 characters of
/// lowercase hex digits and hyphens.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text
            .chars()
            .all(|c| c == '-' || matches!(c, '0'..='9' | 'a'..='f'))
}

#[cfg(test)]
mod tests;
