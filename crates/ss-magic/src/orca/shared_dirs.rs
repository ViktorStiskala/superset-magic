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
//! 1. [`walk_candidates`] walks the checkout and records every directory that is
//!    one of the known kinds or matches a stored pattern, never descending into
//!    a directory it recorded – which is why a `node_modules` nested inside
//!    another `node_modules` can never appear.
//! 2. The stored patterns are matched against those candidates.
//! 3. One `git check-ignore` call keeps only the directories git positively
//!    reports as ignored. Orca refuses to share a directory that is not
//!    gitignored, and a tracked `venv/` is somebody's source, not a cache.
//! 4. The list is sorted and capped at Orca's limit of [`MAX_SHARED_DIRS`].

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
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

/// The standard marker a tool leaves in a directory that holds only
/// regenerable cache data (<https://bford.info/cachedir/>). Cargo writes one
/// into every `target/`.
const CACHEDIR_TAG: &str = "CACHEDIR.TAG";

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
    let Some(set) = glob_set(patterns) else {
        return Expansion::default();
    };
    let matched: Vec<PathBuf> = walk_candidates(root, patterns)
        .into_iter()
        .filter(|rel| set.is_match(rel))
        .collect();
    cap(ignored_dirs(root, &matched))
}

/// Sort-and-cap applied to an already-filtered directory list.
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
    let candidates = walk_candidates(root, patterns);
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

/// Compile `patterns` into one matcher, skipping any that is not a valid glob.
/// `None` when no pattern is usable, which callers read as "matches nothing".
fn glob_set(patterns: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
            any = true;
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}

/// Walk `root` and return every directory that is a known kind or matches one
/// of `patterns`, as root-relative paths.
///
/// The walk never follows a symlink and never descends into:
///
/// - a directory it recorded – the reason a nested `node_modules` never shows
///   up, and what keeps the walk out of the largest trees in the checkout;
/// - the excluded trees (`.git`, `.superset/backups`, `.superset/.magic`,
///   `.scratchpad`);
/// - a directory holding a `.git` entry – a submodule or a nested repository.
///   Its contents belong to another repository, and git aborts a whole
///   `check-ignore` batch on a path inside a submodule;
/// - a directory tagged with `CACHEDIR.TAG` (a Rust `target/`, for one). It is
///   regenerable build output that can hold hundreds of thousands of entries
///   and never a directory worth sharing;
/// - a `.next` directory, beyond checking it for the `cache` child that is the
///   one part of it worth sharing.
///
/// A symlinked directory is neither recorded nor entered: sharing it would hand
/// Orca a path outside the checkout.
pub fn walk_candidates(root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let custom = glob_set(patterns);
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
        let is_kind = KIND_NAMES.iter().any(|kind| name == std::ffi::OsStr::new(kind));
        if is_kind || custom.as_ref().is_some_and(|set| set.is_match(rel)) {
            found.push(rel.to_path_buf());
            walker.skip_current_dir();
            continue;
        }
        if name == std::ffi::OsStr::new(".next") {
            let cache = entry.path().join("cache");
            if std::fs::symlink_metadata(&cache).is_ok_and(|meta| meta.is_dir()) {
                found.push(rel.join("cache"));
            }
            walker.skip_current_dir();
            continue;
        }
        if has_entry(entry.path(), ".git") || has_entry(entry.path(), CACHEDIR_TAG) {
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
