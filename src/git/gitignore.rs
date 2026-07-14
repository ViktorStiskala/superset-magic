//! Helpers for managing `.gitignore` at a git repository root.
//!
//! All helpers follow the convention: never reorder or rewrite existing
//! content; only append when the exact line is missing; create the file
//! if absent.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::git;

/// Append `line` to `<git_root>/.gitignore` if no EXACT line match already
/// exists.  Creates `.gitignore` if the file is absent.
///
/// - Existing content is NEVER reordered or rewritten.
/// - The appended entry is placed on its own line.
/// - A single trailing newline is preserved: if the file's last byte is
///   already `\n` the entry is appended directly; otherwise a newline is
///   inserted before it.
// consumed by U9 (bootstrap) and U11 (reverse sync)
#[allow(dead_code)]
pub fn ensure_entry(git_root: &Path, line: &str) -> Result<()> {
    let path = git_root.join(".gitignore");

    if path.exists() {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        // Exact line match: any line in the file equals `line` after stripping
        // the trailing newline.
        let already_present = contents.lines().any(|l| l == line);
        if already_present {
            return Ok(());
        }

        // Append: ensure the new entry starts on its own line.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {} for append", path.display()))?;

        // If the file is non-empty and its last byte is not a newline,
        // emit one first so the entry starts on its own line.
        if !contents.is_empty() && !contents.ends_with('\n') {
            file.write_all(b"\n")
                .with_context(|| format!("writing {}", path.display()))?;
        }
        writeln!(file, "{}", line).with_context(|| format!("writing {}", path.display()))?;
    } else {
        // File absent — create it with just the entry + trailing newline.
        fs::write(&path, format!("{}\n", line))
            .with_context(|| format!("creating {}", path.display()))?;
    }

    Ok(())
}

/// Resolve the `.gitignore` rule in the working tree rooted at `worktree_root`
/// that COVERS `rel`, for copying into another checkout's `.gitignore`.
///
/// Returns `Ok(Some(pattern))` when a rule matches (the bare pattern text,
/// e.g. `**/.dev.vars`), and `Ok(None)` when NO rule covers `rel`. The caller
/// (reverse sync, U11) appends the returned pattern via [`ensure_entry`] when
/// `Some`, and falls back to the literal relative path when `None` — so a
/// reverse-synced secret lands gitignored in main even if the worktree relied
/// on an inherited glob.
///
/// Delegates to `git check-ignore -v --no-index` (via [`git::check_ignore_pattern`])
/// rather than hand-parsing `.gitignore` files, so nested `.gitignore`s,
/// inherited rules, and git's full match semantics are respected. A negation
/// rule (`!…`) is reported as `None` by the underlying probe.
// consumed by reverse sync (gitignore safety), reachable via the worktree menu
pub fn find_covering_rule(worktree_root: &Path, rel: &Path) -> Result<Option<String>> {
    git::check_ignore_pattern(worktree_root, rel)
}

#[cfg(test)]
mod tests;
