//! Scan the working tree to decide which patterns should be preselected
//! in the bootstrap MultiSelect, and which ones should be flagged as
//! "no current matches" in the UI.

use std::path::Path;

use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use walkdir::WalkDir;

/// The four patterns offered in bootstrap mode by default. Order is
/// load-bearing — preselect bitmaps and prompt strings index into this
/// array.
pub const OPTIONS: [&str; 4] = [".env", "**/.env", ".env.local", "**/.dev.vars"];

const SKIP_DIRS: [&str; 4] = ["node_modules", ".venv", ".git", "target"];

/// For each pattern in `patterns`, `true` when at least one file under
/// `root` matches it (skipping the directories in [`SKIP_DIRS`]).
/// Returns a vector aligned to `patterns`.
pub fn matches_for_patterns(root: &Path, patterns: &[&str]) -> Result<Vec<bool>> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("compiling glob `{pattern}`"))?);
    }
    let set = builder.build().context("building globset")?;

    let mut hits = vec![false; patterns.len()];

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(skip_excluded);

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            // Permission errors or transient races shouldn't abort a scan
            // that only feeds defaults — log nothing, just skip.
            Err(_) => continue,
        };
        if hits.iter().all(|b| *b) {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for idx in set.matches(rel) {
            hits[idx] = true;
        }
    }

    Ok(hits)
}

/// True when `pattern` matches at least one file under `root` (respecting
/// [`SKIP_DIRS`]).
pub fn pattern_matches_any(root: &Path, pattern: &str) -> Result<bool> {
    Ok(matches_for_patterns(root, &[pattern])?[0])
}

fn skip_excluded(entry: &walkdir::DirEntry) -> bool {
    // Always descend into the user-provided root.
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name();
    !SKIP_DIRS.iter().any(|s| name == std::ffi::OsStr::new(s))
}

#[cfg(test)]
mod tests;
