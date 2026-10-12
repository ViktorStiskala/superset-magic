//! The `.worktreeinclude` snapshot: the files Orca should copy into a new
//! worktree itself, as literal repo-relative paths.
//!
//! The snapshot is "what forward sync would copy, filtered to what Orca would
//! copy" (KTD5). Orca's list takes no globs and only copies gitignored paths, so
//! the sync patterns are expanded against the main checkout as it is right now
//! and every match is checked with git. It is a per-machine snapshot: it lists
//! the files present in THIS main checkout and goes stale as files come and go,
//! which is why forward sync refreshes it and why it is never committed.
//!
//! Leaving a file out is always safe. Whatever the snapshot does not list,
//! ss-magic's own forward sync still copies when the worktree's setup script
//! runs – the snapshot only decides who does the copying. So every doubtful case
//! below resolves to "leave it out".

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::git;
use crate::orca::worktreeinclude::{self, Budget};
use crate::sync::apply::{self, PathFilter};

/// The managed section's entries, plus how many the budget left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Repo-relative FILE paths, sorted. Never a directory (R23).
    pub entries: Vec<String>,
    /// Files that qualified but did not fit in what is left of Orca's limits.
    /// ss-magic's own sync copies them.
    pub dropped: usize,
}

/// Compute the snapshot for `main_root`.
///
/// `patterns` are the overlaid sync patterns (`magic.json` plus
/// `magic.local.json`). `shared_dirs` is the directory list that made it into
/// `orca.yaml`; a file beneath one of them is left out, since Orca links or
/// clones the whole directory anyway. `hand_entries` are the person's own lines
/// in `.worktreeinclude`; a path they already list is not listed twice.
/// `budget` is what remains of Orca's limits for the managed section.
///
/// A file enters the snapshot only when git POSITIVELY reports it as ignored
/// and untracked (R10). Outside a repository, or when git cannot answer, the
/// snapshot is empty – and forward sync keeps copying everything itself.
pub fn compute(
    main_root: &Path,
    patterns: &[String],
    shared_dirs: &[String],
    hand_entries: &[String],
    budget: Budget,
) -> Snapshot {
    let candidates = candidate_files(main_root, patterns);
    let ignored = git::ignored_untracked(main_root, &candidates).unwrap_or_default();

    let shared = PathFilter::new(shared_dirs.iter().map(PathBuf::from));
    let already_listed: HashSet<&str> = hand_entries.iter().map(String::as_str).collect();
    let mut entries: Vec<String> = ignored
        .into_iter()
        .filter(|path| !shared.covers(Path::new(path)))
        .filter(|path| !already_listed.contains(path.as_str()))
        .collect();
    entries.sort();
    entries.dedup();

    let total = entries.len();
    let mut used_bytes = 0usize;
    let mut kept = 0usize;
    for entry in &entries {
        let cost = entry.len() + budget.per_entry_overhead;
        if kept >= budget.max_entries || used_bytes + cost > budget.max_bytes {
            break;
        }
        used_bytes += cost;
        kept += 1;
    }
    entries.truncate(kept);
    Snapshot {
        entries,
        dropped: total - kept,
    }
}

/// Every file the sync patterns match in `main_root`, as `/`-separated
/// repo-relative strings, before git is asked anything.
///
/// The patterns are expanded by the same matcher forward sync uses
/// (`apply::match_paths`), which already drops the excluded trees and any match
/// with a `node_modules` / `.venv` component. A matched DIRECTORY is then walked
/// into its files here, because the snapshot lists files only. That walk prunes:
///
/// - the excluded trees, so a bare `.superset` pattern never lists a recovered
///   secret under `.superset/backups/` or the plugin's `.superset/.magic/` state;
/// - `node_modules` / `.venv` at any depth, so a broad `**` never lists a
///   dependency tree file by file;
/// - any directory holding a `.git` entry (a submodule or nested repository).
///
/// Left out as well, each for a reason that would otherwise cost the whole
/// answer or produce a line Orca misreads:
///
/// - a symlink, and anything below a symlinked directory or inside a directory
///   holding a `.git` entry – git aborts the entire `check-ignore` batch on such
///   a path (KTD6), so they must be gone before the call;
/// - a path that is not valid UTF-8, or that cannot be written as one line Orca
///   reads back unchanged ([`worktreeinclude::is_listable`]).
fn candidate_files(main_root: &Path, patterns: &[String]) -> Vec<String> {
    let Ok(matches) = apply::match_paths(main_root, patterns) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for rel in matches {
        let Ok(meta) = fs::symlink_metadata(main_root.join(&rel)) else {
            continue;
        };
        if meta.is_file() {
            files.push(rel);
        } else if meta.is_dir() {
            walk_files(main_root, &rel, &mut files);
        }
        // A symlink or a special file is left to ss-magic's own sync.
    }

    let mut blocked_dirs: HashMap<PathBuf, bool> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for rel in files {
        if has_blocked_ancestor(main_root, &rel, &mut blocked_dirs) {
            continue;
        }
        let Some(text) = rel.to_str() else { continue };
        if !worktreeinclude::is_listable(text) {
            continue;
        }
        if seen.insert(text.to_string()) {
            out.push(text.to_string());
        }
    }
    out
}

/// Collect the regular files under the matched directory `rel` into `files`,
/// as `main_root`-relative paths, never following a symlink.
fn walk_files(main_root: &Path, rel: &Path, files: &mut Vec<PathBuf>) {
    let mut walker = WalkDir::new(main_root.join(rel))
        .follow_links(false)
        .into_iter();
    while let Some(entry) = walker.next() {
        // An unreadable directory only makes the snapshot smaller.
        let Ok(entry) = entry else { continue };
        let Ok(entry_rel) = entry.path().strip_prefix(main_root) else {
            continue;
        };
        let file_type = entry.file_type();
        if file_type.is_dir() {
            let name = entry.file_name();
            let default_excluded = apply::DEFAULT_EXCLUDES
                .iter()
                .any(|excluded| name == std::ffi::OsStr::new(excluded));
            if crate::sync::under_excluded_tree(entry_rel)
                || default_excluded
                || fs::symlink_metadata(entry.path().join(".git")).is_ok()
            {
                walker.skip_current_dir();
            }
        } else if file_type.is_file() {
            files.push(entry_rel.to_path_buf());
        }
    }
}

/// Whether some directory between `main_root` and the file `rel` is a symlink
/// or holds a `.git` entry. `cache` remembers the verdict per directory, so a
/// thousand files in one directory cost one pair of `lstat`s.
fn has_blocked_ancestor(
    main_root: &Path,
    rel: &Path,
    cache: &mut HashMap<PathBuf, bool>,
) -> bool {
    rel.ancestors()
        .skip(1)
        .filter(|dir| !dir.as_os_str().is_empty())
        .any(|dir| {
            *cache.entry(dir.to_path_buf()).or_insert_with(|| {
                let abs = main_root.join(dir);
                // Unknown counts as blocked: a directory that cannot be
                // examined is not one to vouch for.
                let is_symlink = fs::symlink_metadata(&abs)
                    .map(|meta| meta.file_type().is_symlink())
                    .unwrap_or(true);
                is_symlink || fs::symlink_metadata(abs.join(".git")).is_ok()
            })
        })
}

/// A human-readable warning for files the budget left out, or `None` when
/// everything fit.
pub fn overflow_warning(snapshot: &Snapshot) -> Option<String> {
    (snapshot.dropped > 0).then(|| {
        format!(
            ".worktreeinclude: Orca reads at most {} entries ({} KiB), so {} matching {} left \
             out; ss-magic's own sync copies {} when a worktree is set up.",
            worktreeinclude::MAX_ENTRIES,
            worktreeinclude::MAX_BYTES / 1024,
            snapshot.dropped,
            if snapshot.dropped == 1 {
                "file was"
            } else {
                "files were"
            },
            if snapshot.dropped == 1 { "it" } else { "them" },
        )
    })
}

#[cfg(test)]
mod tests;
