//! Writing the Orca files into the main checkout.
//!
//! Two files, two writers that share one rule about where a write may land:
//!
//! - [`refresh_snapshot`] is the AUTOMATIC refresh that ends a forward sync in
//!   an Orca worktree (KTD11). It rewrites only the managed section of main's
//!   `.worktreeinclude`, only when git positively reports that file as ignored
//!   and untracked, and it reports rather than fails: a background setup script
//!   must never leave the shared main checkout with a modified tracked file, and
//!   must never turn a successful sync into a failed one.
//!
//! Every write goes through [`write_contained`], which refuses a target that
//! resolves outside the repository through a symlink (KTD13). The hazard is a
//! repository that commits `orca.yaml` or `.worktreeinclude` as a link to a file
//! the person owns elsewhere; a command run in that checkout would otherwise
//! follow the link and rewrite THAT file.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::git;
use crate::orca::worktreeinclude::{self, Document, Existing};
use crate::orca::{orca_yaml, snapshot};
use crate::workspace::superset_files;

/// The name of Orca's configuration file, at the root of the main checkout.
pub const ORCA_YAML: &str = "orca.yaml";

/// Where a write to `<root>/<rel>` would actually land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// Inside the repository: the file does not exist yet, or it exists and
    /// resolves inside – possibly through a symlink that stays in the tree,
    /// which the writer follows onto its target.
    Inside,
    /// The file resolves OUTSIDE the repository through a symlink, to this
    /// path. No writer may proceed.
    Outside(PathBuf),
}

/// Decide where `<root>/<rel>` really is before a writer follows the path.
///
/// Both sides are canonicalized, which resolves every symlink in one step and
/// keeps the comparison honest when the repository itself sits behind a link
/// (macOS's `/tmp`). `rel` names a file directly under `root`, so a file that
/// does not exist yet is created in `root` itself and nothing on the path can
/// redirect it. A dangling link reads as "does not exist"; the atomic writer
/// then replaces the link with a regular file, inside the repository. A root
/// that cannot be resolved is an error rather than a pass – the unknown answer
/// must be the refusing one.
pub fn landing(root: &Path, rel: &str) -> Result<Landing> {
    let real_root = root
        .canonicalize()
        .with_context(|| format!("resolving the repository root {}", root.display()))?;
    match root.join(rel).canonicalize() {
        Ok(real_file) if !real_file.starts_with(&real_root) => Ok(Landing::Outside(real_file)),
        _ => Ok(Landing::Inside),
    }
}

/// Replace `<root>/<rel>` with `text` atomically, unless that would write
/// outside the repository. Returns where the write landed, so the caller can
/// report an `Outside` refusal instead of treating it as a failure.
pub fn write_contained(root: &Path, rel: &str, text: &str) -> Result<Landing> {
    let landing = landing(root, rel)?;
    if landing == Landing::Inside {
        superset_files::write_atomically(&root.join(rel), text)?;
    }
    Ok(landing)
}

/// What the automatic snapshot refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotRefresh {
    /// The managed section was rewritten with `listed` files. `warning` is the
    /// Orca-limit notice when some files did not fit.
    Written {
        listed: usize,
        warning: Option<String>,
    },
    /// The file already says exactly this; nothing was written.
    Unchanged { warning: Option<String> },
    /// Nothing was written, for the reason given.
    Skipped(String),
}

/// Regenerate the managed section of `<main_root>/.worktreeinclude` from the
/// overlaid sync `patterns`, and touch nothing else – in particular never
/// `orca.yaml` (R12, AE6).
///
/// Best-effort by contract (R26): every reason not to write is a
/// [`SnapshotRefresh::Skipped`], never an error, so the forward sync that calls
/// this keeps its exit status. The write happens only when ALL of these hold:
///
/// - git positively reports `.worktreeinclude` as untracked (an empty
///   `ls-files` answer for it) AND as ignored. A tracked file would show up as
///   modified in the shared main checkout; an unignored one would show up as
///   untracked. Either would be a surprise left behind by a background script,
///   and "git could not be asked" counts as a no.
/// - the existing file, if any, can be read as UTF-8 text. Rebuilding an
///   unreadable file from an empty parse would drop the person's lines.
/// - the file does not resolve outside the repository through a symlink.
///
/// The files under a directory Orca already shares are left out of the
/// snapshot. "Already shares" is read from `orca.yaml` as it is on disk, since
/// that – not what `magic.json` would expand to today – is the list Orca uses
/// until someone runs the explicit refresh.
pub fn refresh_snapshot(main_root: &Path, patterns: &[String]) -> SnapshotRefresh {
    let name = worktreeinclude::FILE_NAME;
    match git::tracked_files(main_root, &[name]) {
        Ok(tracked) if tracked.is_empty() => {}
        Ok(_) => {
            return SnapshotRefresh::Skipped(format!(
                "the main checkout's `{name}` is tracked by git, and the automatic refresh never \
                 rewrites a tracked file. Run `ss-magic orca` there to update it."
            ));
        }
        Err(err) => {
            return SnapshotRefresh::Skipped(format!(
                "git could not say whether the main checkout's `{name}` is tracked ({err:#})"
            ));
        }
    }
    match git::is_ignored_str(main_root, name) {
        Ok(true) => {}
        Ok(false) => {
            return SnapshotRefresh::Skipped(format!(
                "the main checkout does not gitignore `{name}`. Run `ss-magic orca` there once to \
                 set it up."
            ));
        }
        Err(err) => {
            return SnapshotRefresh::Skipped(format!(
                "git could not say whether the main checkout ignores `{name}` ({err:#})"
            ));
        }
    }

    let existing = worktreeinclude::read(main_root);
    let document = match &existing {
        Existing::Absent => Document::empty(),
        Existing::Text(text) => Document::parse(text),
        Existing::Unreadable(why) => {
            return SnapshotRefresh::Skipped(format!(
                "the main checkout's `{name}` was left untouched because {why}"
            ));
        }
    };

    let shared_dirs = fs::read_to_string(main_root.join(ORCA_YAML))
        .map(|text| orca_yaml::shared_directories(&text))
        .unwrap_or_default();
    let snap = snapshot::compute(
        main_root,
        patterns,
        &shared_dirs,
        &document.hand_entries(),
        document.budget(),
    );
    let warning = snapshot::overflow_warning(&snap);
    let rendered = document.render(&snap.entries);
    if matches!(&existing, Existing::Text(text) if *text == rendered) {
        return SnapshotRefresh::Unchanged { warning };
    }

    match write_contained(main_root, name, &rendered) {
        Ok(Landing::Inside) => SnapshotRefresh::Written {
            listed: snap.entries.len(),
            warning,
        },
        Ok(Landing::Outside(target)) => SnapshotRefresh::Skipped(format!(
            "the main checkout's `{name}` is a symlink to {}, outside the repository",
            target.display()
        )),
        Err(err) => SnapshotRefresh::Skipped(format!("writing `{name}` failed ({err:#})")),
    }
}

#[cfg(test)]
mod tests;
