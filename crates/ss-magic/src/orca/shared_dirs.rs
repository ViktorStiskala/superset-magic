//! Shared-directory detection: which dependency directories of the main
//! checkout Orca should link or clone-copy into a new worktree
//! (`worktree.sharedDirectories` in `orca.yaml`).
//!
//! These are exactly the trees ss-magic's own forward sync never copies
//! (`node_modules`, `.venv`), so Orca's native sharing is the only way a new
//! worktree gets them without a fresh install.
//!
//! The model has two layers (KTD7). `magic.json` stores glob PATTERNS under
//! `sharedDirectories` (`**/node_modules`), because the set of packages in a
//! monorepo changes and a pattern keeps up with it. Orca wants LITERAL
//! repo-relative directories, so each write expands the patterns against the
//! main checkout as it is right now:
//!
//! 1. [`walk_candidates`] walks the checkout BY NAME and records every
//!    directory of a known kind: `node_modules`, `.venv`, `venv`, `.cache`,
//!    `.turbo`, `.parcel-cache` and `.next/cache`. It never descends into a
//!    directory it recorded – which is why a `node_modules` nested inside
//!    another `node_modules` can never appear.
//! 2. The stored patterns are matched against those candidates. A pattern
//!    therefore SELECTS among the known kinds (`**/node_modules` takes all of
//!    them, `apps/*/node_modules` only some); it cannot make an arbitrary
//!    directory shareable. The kinds are a deliberate choice – Rust's `target/`
//!    was considered and left out – so they are a fixed list here, not something
//!    a pattern widens.
//! 3. One `git check-ignore` call keeps only the directories git positively
//!    reports as ignored. Orca refuses to share a directory that is not
//!    gitignored, and a tracked `venv/` is somebody's source, not a cache.
//! 4. The list is sorted and capped at Orca's limit of [`MAX_SHARED_DIRS`].

use std::path::{Path, PathBuf};

use globset::Glob;
use walkdir::WalkDir;

use crate::git;

/// Orca keeps only the first 100 `sharedDirectories` entries.
pub const MAX_SHARED_DIRS: usize = 100;

/// The pattern kinds offered by default, in the order the picker shows them:
/// `node_modules`, Python virtualenvs, and JS build caches. Rust's `target/` is
/// deliberately not among them.
pub const DEFAULT_PATTERNS: [&str; 7] = [
    "**/node_modules",
    "**/.venv",
    "**/venv",
    "**/.cache",
    "**/.turbo",
    "**/.parcel-cache",
    "**/.next/cache",
];

/// Directory names recorded wherever the walk meets them. `.next/cache` is the
/// one default kind that is a two-component shape, handled apart in the walk.
const KIND_NAMES: [&str; 6] = [
    "node_modules",
    ".venv",
    "venv",
    ".cache",
    ".turbo",
    ".parcel-cache",
];

/// The literal directories for `orca.yaml`, plus how many the cap left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    /// Repo-relative directories, sorted, at most [`MAX_SHARED_DIRS`].
    pub dirs: Vec<String>,
    /// Directories that matched and are ignored but did not fit under the cap.
    /// They will NOT be shared, and forward sync never copies dependency
    /// trees, so a worktree simply starts without them.
    pub dropped: usize,
}

/// Expand the stored `patterns` to the literal, gitignored directories they
/// match in `root` right now.
///
/// Fails closed: outside a git repository, or when git cannot answer, nothing
/// is positively ignored and the list is empty.
pub fn expand(root: &Path, patterns: &[String]) -> Expansion {
    let matchers: Vec<_> = patterns
        .iter()
        .filter_map(|pattern| Glob::new(pattern).ok())
        .map(|glob| glob.compile_matcher())
        .collect();
    let matched: Vec<PathBuf> = walk_candidates(root)
        .into_iter()
        .filter(|rel| matchers.iter().any(|matcher| matcher.is_match(rel)))
        .collect();
    cap(ignored_dirs(root, &matched))
}

/// Sort and cap an already-filtered directory list.
fn cap(mut dirs: Vec<String>) -> Expansion {
    dirs.sort();
    dirs.dedup();
    let dropped = dirs.len().saturating_sub(MAX_SHARED_DIRS);
    dirs.truncate(MAX_SHARED_DIRS);
    Expansion { dirs, dropped }
}

/// One row of the shared-directory picker: a pattern and the gitignored
/// directories it matches in the checkout today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatches {
    pub pattern: String,
    pub dirs: Vec<String>,
}

/// For each of `patterns`, the gitignored directories it matches in `root`,
/// sorted. One walk and one git call cover the whole list, so the picker can
/// show every default kind plus every stored pattern without a walk per row.
pub fn matches_by_pattern(root: &Path, patterns: &[String]) -> Vec<PatternMatches> {
    let candidates = walk_candidates(root);
    let mut ignored = ignored_dirs(root, &candidates);
    ignored.sort();
    patterns
        .iter()
        .map(|pattern| {
            let dirs = match Glob::new(pattern) {
                Ok(glob) => {
                    let matcher = glob.compile_matcher();
                    ignored
                        .iter()
                        .filter(|dir| matcher.is_match(Path::new(dir)))
                        .cloned()
                        .collect()
                }
                Err(_) => Vec::new(),
            };
            PatternMatches {
                pattern: pattern.clone(),
                dirs,
            }
        })
        .collect()
}

/// The default kinds that match at least one gitignored directory in `root` –
/// what a non-interactive init stores when `magic.json` has no
/// `sharedDirectories` key yet.
pub fn detected_default_patterns(root: &Path) -> Vec<String> {
    let defaults: Vec<String> = DEFAULT_PATTERNS.iter().map(|p| p.to_string()).collect();
    matches_by_pattern(root, &defaults)
        .into_iter()
        .filter(|m| !m.dirs.is_empty())
        .map(|m| m.pattern)
        .collect()
}

/// The patterns the shared-directory picker offers, in row order: the default
/// kinds followed by any stored pattern that is not one of them (a pattern the
/// person added by hand earlier, which must stay visible to stay stored).
pub fn picker_patterns(stored: Option<&[String]>) -> Vec<String> {
    let mut options: Vec<String> = DEFAULT_PATTERNS.iter().map(|p| p.to_string()).collect();
    for pattern in stored.unwrap_or_default() {
        if !options.contains(pattern) {
            options.push(pattern.clone());
        }
    }
    options
}

/// Which of the picker's `rows` start checked, as indices into `rows`. It
/// depends on whether a choice was ever stored:
///
/// - `stored` is `Some` – exactly the stored patterns, so reopening the picker
///   shows the previous choice, including a deliberate "none".
/// - `stored` is `None` (never chosen) – every row that matches at least one
///   directory, so the first run offers what is actually there, preselected.
pub fn picker_preselected(stored: Option<&[String]>, rows: &[PatternMatches]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| match stored {
            Some(stored) => stored.contains(&row.pattern),
            None => !row.dirs.is_empty(),
        })
        .map(|(index, _)| index)
        .collect()
}

/// The patterns to store when nobody is asked – what the non-interactive init
/// and migrate write as `magic.json`'s `sharedDirectories`:
///
/// - the stored list when `magic.json` already has the key, an empty list
///   included, since that is a choice someone made;
/// - otherwise every default kind that matches something in `root`;
/// - and `None`, leaving the key ABSENT, when there is no stored choice and
///   nothing was detected. Writing `[]` there would record "chose none" on
///   behalf of nobody, and freeze it: a scripted init run in a fresh checkout,
///   before any dependencies are installed, would stop every later run from
///   detecting them.
pub fn stored_or_detected(root: &Path, stored: Option<&[String]>) -> Option<Vec<String>> {
    match stored {
        Some(stored) => Some(stored.to_vec()),
        None => Some(detected_default_patterns(root)).filter(|detected| !detected.is_empty()),
    }
}

/// Walk `root` by name and return every directory of a known kind, as
/// root-relative paths.
///
/// The walk never follows a symlink and never descends into:
///
/// - a directory it recorded – the reason a nested `node_modules` never shows
///   up, and what keeps the walk out of the largest trees in the checkout;
/// - the excluded trees (`.git`, `.superset/backups`, `.superset/.magic`,
///   `.scratchpad`);
/// - a directory holding a `.git` entry – a submodule or a nested repository.
///   Its contents belong to another repository, and git aborts a whole
///   `check-ignore` batch on a path inside a submodule.
///
/// A symlinked directory is neither recorded nor entered: sharing it would hand
/// Orca a path outside the checkout.
pub fn walk_candidates(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut walker = WalkDir::new(root).follow_links(false).into_iter();
    while let Some(entry) = walker.next() {
        // An unreadable directory is skipped, not fatal: detection only feeds
        // a list of optional optimizations.
        let Ok(entry) = entry else { continue };
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        if crate::sync::under_excluded_tree(rel) {
            walker.skip_current_dir();
            continue;
        }
        let name = entry.file_name();
        let is_named_kind = KIND_NAMES.iter().any(|kind| name == std::ffi::OsStr::new(kind));
        // `.next/cache`: a directory called `cache` directly inside `.next`.
        let is_next_cache = name == std::ffi::OsStr::new("cache")
            && rel
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|parent| parent == std::ffi::OsStr::new(".next"));
        if is_named_kind || is_next_cache {
            found.push(rel.to_path_buf());
            walker.skip_current_dir();
            continue;
        }
        if has_entry(entry.path(), ".git") {
            walker.skip_current_dir();
        }
    }
    found
}

/// Whether `dir` directly holds an entry called `name`, of any type. Checked
/// without following a symlink, so a dangling `.git` link still counts.
fn has_entry(dir: &Path, name: &str) -> bool {
    std::fs::symlink_metadata(dir.join(name)).is_ok()
}

/// The subset of `candidates` git positively reports as ignored and untracked,
/// as `/`-separated strings without a trailing slash.
///
/// Each directory is queried WITH a trailing slash, which forces git's
/// directory-only match so a `node_modules/` rule answers correctly. A candidate
/// whose path is not valid UTF-8 is dropped before the call – `orca.yaml` is
/// text, so it could not be written there anyway. Any git failure yields an
/// empty list.
pub fn ignored_dirs(root: &Path, candidates: &[PathBuf]) -> Vec<String> {
    let queries: Vec<String> = candidates
        .iter()
        .filter_map(|rel| rel.to_str())
        .map(|rel| format!("{rel}/"))
        .collect();
    match git::ignored_untracked(root, &queries) {
        Ok(ignored) => ignored
            .into_iter()
            .map(|dir| dir.trim_end_matches('/').to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// A human-readable warning for directories the cap left out, or `None` when
/// everything fit.
pub fn overflow_warning(expansion: &Expansion) -> Option<String> {
    (expansion.dropped > 0).then(|| {
        format!(
            "orca.yaml: Orca shares at most {MAX_SHARED_DIRS} directories, so {} matching \
             {} will not be shared with new worktrees (forward sync never copies dependency \
             trees). Narrow the `sharedDirectories` patterns in magic.json to choose which.",
            expansion.dropped,
            if expansion.dropped == 1 {
                "directory"
            } else {
                "directories"
            },
        )
    })
}

#[cfg(test)]
mod tests;
