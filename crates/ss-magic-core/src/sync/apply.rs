//! The file-copy engine: expand glob/literal patterns and copy the matched
//! files from a source tree into a destination worktree. Used by `ss-magic
//! sync` (patterns from `magic.json`) and by reverse sync. Re-implements the
//! original `setup.sh`'s expansion semantics in Rust (no bash 4 + jq needed).
//!
//! Expansion semantics (carried over from the original `setup.sh`):
//!
//! - Reject absolute patterns and patterns containing a `..` segment.
//! - Literal patterns must exist; missing literal → counted skip.
//! - Glob patterns may match zero entries → logged, not counted.
//! - `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at any
//!   depth; logged, not counted.
//! - De-duplicate matches across patterns by relative path.
//! - Directories are copied recursively (mirrors `cp -R src/. dst/`).
//! - Summary line is OK when `skipped == 0`, WARN otherwise.
//!
//! [`run_filtered`] is the same engine with a [`PathFilter`]: a set of
//! repo-relative paths the copy must leave alone because something else already
//! put them in the destination. It is applied at final enumeration – on the
//! expanded match list AND inside the recursive directory copy – so a directory
//! match that is an ancestor of a filtered path still skips it.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobMatcher};
use walkdir::WalkDir;

use crate::sync::pattern::{self, SyntaxError};

/// Directory names that drop matches at any depth, matching `setup.sh`.
///
/// Public so a caller that expands a matched directory into its files itself
/// (the Orca snapshot in the CLI crate) can prune the same names during its own
/// walk instead of keeping a second copy of the list.
pub const DEFAULT_EXCLUDES: [&str; 2] = ["node_modules", ".venv"];

/// Result tally returned to the caller after `run`. Mirrors `setup.sh`'s
/// summary line.
#[derive(Debug, Default, Clone, Copy)]
pub struct Summary {
    pub copied: usize,
    pub skipped: usize,
    /// Paths a [`PathFilter`] kept out of the copy. Counted apart from
    /// `skipped` because a filtered path is not a problem – the caller asked for
    /// it to be left alone – so it must never turn the summary into a warning.
    pub filtered: usize,
}

/// A set of repo-relative paths a copy must leave alone.
///
/// An entry covers itself and, when it names a directory, everything beneath
/// it – the same "this path or anything inside it" shape as
/// `sync::under_excluded_tree`, but over a caller-supplied list instead of the
/// fixed excluded trees. Matching is by whole path components (`Path`
/// equality), never by string prefix, so an entry `config` covers
/// `config/a.json` and not `config.json` or `configs/a.json`.
///
/// The empty filter covers nothing, which is what makes [`run`] and
/// [`copy_dir_recursive`] plain delegations to their filtered forms.
#[derive(Debug, Default, Clone)]
pub struct PathFilter {
    paths: HashSet<PathBuf>,
}

impl PathFilter {
    /// The filter that covers nothing.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a filter from repo-relative paths. An empty path is dropped: it is
    /// an ancestor of every relative path, so keeping it would filter the whole
    /// tree on the strength of a blank line.
    pub fn new<I>(paths: I) -> Self
    where
        I: IntoIterator<Item = PathBuf>,
    {
        Self {
            paths: paths
                .into_iter()
                .filter(|p| !p.as_os_str().is_empty())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Whether `rel` is one of the filtered paths or lies beneath one.
    pub fn covers(&self, rel: &Path) -> bool {
        if self.paths.is_empty() {
            return false;
        }
        rel.ancestors().any(|a| self.paths.contains(a))
    }
}

/// Reason a pattern or path was skipped. `counts()` reports whether the
/// skip should bump the summary's `skipped` counter (matches `setup.sh`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    AbsolutePathRejected,
    ParentSegmentRejected,
    BadGlob(String),
    MissingLiteral,
    NoMatches,
    Excluded,
    NotAFileOrDir,
    CopyFailed(String),
    /// A [`PathFilter`] covers the path, so the copy left it alone. Not a
    /// problem and never counted in `Summary::skipped`; it is tallied in
    /// `Summary::filtered` instead.
    Filtered,
}

impl SkipReason {
    pub fn counts(&self) -> bool {
        matches!(
            self,
            SkipReason::AbsolutePathRejected
                | SkipReason::ParentSegmentRejected
                | SkipReason::BadGlob(_)
                | SkipReason::MissingLiteral
                | SkipReason::NotAFileOrDir
                | SkipReason::CopyFailed(_)
        )
    }

    pub fn label(&self) -> &str {
        match self {
            SkipReason::AbsolutePathRejected => "absolute path rejected",
            SkipReason::ParentSegmentRejected => "\"..\" not allowed",
            SkipReason::BadGlob(_) => "bad glob",
            SkipReason::MissingLiteral => "missing",
            SkipReason::NoMatches => "no matches",
            SkipReason::Excluded => "excluded",
            SkipReason::NotAFileOrDir => "not a file or dir",
            SkipReason::CopyFailed(_) => "copy failed",
            SkipReason::Filtered => "filtered",
        }
    }
}

/// Per-item event emitted while expanding and copying. Callers convert
/// these to user-facing output (`main.rs`) or assertions (tests).
#[derive(Debug, Clone)]
pub enum Event {
    Copy {
        rel: PathBuf,
    },
    Skip {
        reason: SkipReason,
        /// Label printed after the reason — pattern text for pattern-level
        /// skips, relative path for per-file skips.
        label: String,
    },
}

/// Apply `patterns` from `src` into `dest`, calling `on_event` for each
/// copy and skip. Returns the final `Summary`.
pub fn run<F>(src: &Path, dest: &Path, patterns: &[String], on_event: F) -> Result<Summary>
where
    F: FnMut(&Event),
{
    run_filtered(src, dest, patterns, &PathFilter::empty(), on_event)
}

/// [`run`], leaving every path `filter` covers untouched in `dest`.
///
/// The filter is applied where the file set is actually materialized, in two
/// places: on each expanded match, and on each entry of a matched directory's
/// recursive copy. The second is what handles a directory match that is an
/// ANCESTOR of a filtered path (pattern `config`, filter `config/a.json`) –
/// filtering only the match list would copy that file anyway. Every filtered
/// path emits one [`SkipReason::Filtered`] event and bumps `Summary::filtered`;
/// a filtered directory is reported once, not once per file beneath it.
pub fn run_filtered<F>(
    src: &Path,
    dest: &Path,
    patterns: &[String],
    filter: &PathFilter,
    mut on_event: F,
) -> Result<Summary>
where
    F: FnMut(&Event),
{
    let mut summary = Summary::default();

    let matches = {
        let mut counted = |ev: &Event| {
            if let Event::Skip { reason, .. } = ev {
                if reason.counts() {
                    summary.skipped += 1;
                }
            }
            on_event(ev);
        };
        expand_patterns(src, patterns, &mut counted)?
    };

    for rel in &matches {
        if filter.covers(rel) {
            on_event(&Event::Skip {
                reason: SkipReason::Filtered,
                label: rel.display().to_string(),
            });
            summary.filtered += 1;
            continue;
        }
        let src_path = src.join(rel);
        let dest_path = dest.join(rel);
        if let Some(parent) = dest_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                on_event(&Event::Skip {
                    reason: SkipReason::CopyFailed(e.to_string()),
                    label: rel.display().to_string(),
                });
                summary.skipped += 1;
                continue;
            }
        }
        let outcome: Result<()> = if src_path.is_dir() {
            let mut filtered_inside = 0;
            let outcome =
                copy_dir_recursive_filtered(src, &src_path, &dest_path, filter, &mut |inner| {
                    on_event(&Event::Skip {
                        reason: SkipReason::Filtered,
                        label: inner.display().to_string(),
                    });
                    filtered_inside += 1;
                });
            summary.filtered += filtered_inside;
            outcome
        } else if src_path.is_file() {
            fs::copy(&src_path, &dest_path).map(|_| ()).map_err(|e| {
                anyhow::Error::from(e).context(format!(
                    "copy {} → {}",
                    src_path.display(),
                    dest_path.display()
                ))
            })
        } else {
            on_event(&Event::Skip {
                reason: SkipReason::NotAFileOrDir,
                label: rel.display().to_string(),
            });
            summary.skipped += 1;
            continue;
        };
        match outcome {
            Ok(()) => {
                on_event(&Event::Copy { rel: rel.clone() });
                summary.copied += 1;
            }
            Err(err) => {
                on_event(&Event::Skip {
                    reason: SkipReason::CopyFailed(format!("{err:#}")),
                    label: rel.display().to_string(),
                });
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

/// Public matcher: expand `patterns` against `root` and return the deduped
/// list of relative paths that match, applying the SAME semantics as
/// [`run`]'s copy phase (syntax checks via `pattern::check_syntax`, glob/
/// literal expansion, `DEFAULT_EXCLUDES` dropped at any depth, de-dupe by
/// relative path) — but WITHOUT copying anything.
///
/// This is the seam reverse sync (U11) uses: `run` copies every match
/// unconditionally and offers no filtered-subset hook, so reverse sync needs
/// the raw matched paths to intersect with the untracked set and copy only
/// the user-selected subset itself. Skip events (bad glob, no-match, excluded)
/// are swallowed here — the caller only wants the resulting paths.
// consumed by U11 (reverse sync candidate computation)
#[allow(dead_code)]
pub fn match_paths(root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>> {
    let mut sink = |_: &Event| {};
    expand_patterns(root, patterns, &mut sink)
}

/// Expand `patterns` against `src`, emitting skip events through
/// `on_event`. Returns the deduped list of relative paths to copy.
fn expand_patterns<F>(src: &Path, patterns: &[String], on_event: &mut F) -> Result<Vec<PathBuf>>
where
    F: FnMut(&Event),
{
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut matches: Vec<PathBuf> = Vec::new();

    // Cache the source walk for any glob pattern that needs it. We walk
    // once, lazily, sharing across patterns.
    let mut all_files: Option<Vec<PathBuf>> = None;

    for pat in patterns {
        if let Err(err) = pattern::check_syntax(pat) {
            let reason = match err {
                SyntaxError::Empty | SyntaxError::AbsolutePath => SkipReason::AbsolutePathRejected,
                SyntaxError::ParentSegment => SkipReason::ParentSegmentRejected,
                SyntaxError::BadGlob(msg) => SkipReason::BadGlob(msg),
            };
            // Empty patterns are reported as absolute-path rejects only as
            // a fallback — `check_syntax` would have returned Empty, but
            // a real `setup_config.json` can't carry one through serde.
            // Map the rare case rather than add a SkipReason variant.
            on_event(&Event::Skip {
                reason,
                label: pat.clone(),
            });
            continue;
        }
        if pattern::has_glob_meta(pat) {
            // `pattern::check_syntax` above already verified the glob
            // compiles, so build_matcher can't fail here.
            let matcher = build_matcher(pat).expect("glob compiled in check_syntax");
            let entries = all_files.get_or_insert_with(|| walk_source(src));

            let mut raw_hit = false;
            for rel in entries.iter() {
                if !matcher.is_match(rel) {
                    continue;
                }
                raw_hit = true;
                if is_excluded(rel) {
                    on_event(&Event::Skip {
                        reason: SkipReason::Excluded,
                        label: rel.display().to_string(),
                    });
                    continue;
                }
                if seen.insert(rel.clone()) {
                    matches.push(rel.clone());
                }
            }
            if !raw_hit {
                on_event(&Event::Skip {
                    reason: SkipReason::NoMatches,
                    label: pat.clone(),
                });
            }
        } else {
            // Literal pattern.
            let abs = src.join(pat);
            if !abs.exists() {
                on_event(&Event::Skip {
                    reason: SkipReason::MissingLiteral,
                    label: pat.clone(),
                });
                continue;
            }
            let rel = PathBuf::from(pat);
            if is_excluded(&rel) {
                on_event(&Event::Skip {
                    reason: SkipReason::Excluded,
                    label: pat.clone(),
                });
                continue;
            }
            if seen.insert(rel.clone()) {
                matches.push(rel);
            }
        }
    }
    Ok(matches)
}

/// Walk `src` once and return every file/dir's path relative to `src`,
/// including directories so a pattern like `apps/*/config` can match them.
///
/// The excluded trees (`crate::sync::EXCLUDED_TREES` — the backups tree, the
/// plugin's `.magic` state, `.scratchpad`, `.git`) are PRUNED here rather than
/// filtered out of the result: `filter_entry` returning `false` for a directory
/// stops the walk from descending, so no pattern — however broad — can ever see
/// a path inside one of them.
fn walk_source(src: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let walker = WalkDir::new(src)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| match e.path().strip_prefix(src) {
            Ok(rel) => !crate::sync::under_excluded_tree(rel),
            // Not under `src` at all (can't happen for a plain walk) — keep it
            // rather than silently dropping an entry we can't classify.
            Err(_) => true,
        });
    for entry in walker.flatten() {
        if entry.depth() == 0 {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(src) {
            out.push(rel.to_path_buf());
        }
    }
    out
}

fn build_matcher(pattern: &str) -> Result<GlobMatcher> {
    Ok(Glob::new(pattern)
        .with_context(|| format!("compiling glob `{pattern}`"))?
        .compile_matcher())
}

/// Whether a match must be dropped, for either of the two independent reasons:
/// the legacy `DEFAULT_EXCLUDES` directory NAMES at any depth, or a whole
/// excluded tree (`crate::sync::EXCLUDED_TREES`) matched on its exact leading
/// components. Glob matches already arrive pre-pruned from [`walk_source`]; a
/// LITERAL pattern never passes through that walk, so this is where a literal
/// naming a file inside an excluded tree gets stopped.
fn is_excluded(rel: &Path) -> bool {
    if crate::sync::under_excluded_tree(rel) {
        return true;
    }
    rel.components().any(|c| match c {
        std::path::Component::Normal(name) => DEFAULT_EXCLUDES
            .iter()
            .any(|ex| name == std::ffi::OsStr::new(ex)),
        _ => false,
    })
}

/// Recursively copy `src` into `dst`. Crate-visible so the reverse-sync backup
/// pass can back up a directory target before a forward sync overwrites it.
///
/// `root` is the tree `src` lives under — the sync root, NOT `src` itself. This
/// walk re-enumerates the live filesystem from a directory that was already
/// chosen as a match, so it is its own enforcement point for the excluded trees
/// and cannot rely on any filter applied to the match list: a literal
/// `.superset` pattern is appended to the match set without ever passing through
/// [`walk_source`], and would otherwise copy `backups/` and `.magic/` wholesale.
/// The exclusion rules are written relative to the root (`.superset/.magic`), so
/// an entry's path is stripped against `root` — stripping against `src` would
/// yield `.magic/…` and match nothing. Excluded directories are pruned, so the
/// walk never descends into them.
pub fn copy_dir_recursive(root: &Path, src: &Path, dst: &Path) -> Result<()> {
    copy_dir_recursive_filtered(root, src, dst, &PathFilter::empty(), &mut |_| {})
}

/// [`copy_dir_recursive`], additionally pruning every entry `filter` covers.
///
/// The filter is keyed on root-relative paths, exactly like the excluded trees,
/// so it is checked against the same `strip_prefix(root)` result. A covered
/// entry is reported through `on_filtered` with its root-relative path and then
/// pruned: a covered directory is reported once and never descended into.
pub fn copy_dir_recursive_filtered(
    root: &Path,
    src: &Path,
    dst: &Path,
    filter: &PathFilter,
    on_filtered: &mut dyn FnMut(&Path),
) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("mkdir -p {}", dst.display()))?;
    let walker = WalkDir::new(src)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| match e.path().strip_prefix(root) {
            Ok(rel) => {
                if crate::sync::under_excluded_tree(rel) {
                    return false;
                }
                if filter.covers(rel) {
                    on_filtered(rel);
                    return false;
                }
                true
            }
            // Outside `root` entirely — keep it; the caller chose this source.
            Err(_) => true,
        });
    for entry in walker {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("mkdir -p {}", target.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("mkdir -p {}", parent.display()))?;
            }
            fs::copy(entry.path(), &target).with_context(|| {
                format!("copy {} → {}", entry.path().display(), target.display())
            })?;
        }
        // Skip symlinks and other special files — matches `cp -R`'s
        // pragmatic posture without leaking outside the source tree.
    }
    Ok(())
}

#[cfg(test)]
mod tests;
