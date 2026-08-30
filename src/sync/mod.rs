//! The sync engine: glob expansion, forward copy, reverse sync, and the
//! shared pattern/working-tree helpers behind both directions.

use std::ffi::OsStr;
use std::path::{Component, Path};

pub(crate) mod apply;
pub(crate) mod merge;
pub(crate) mod pattern;
pub(crate) mod repo_scan;
pub(crate) mod reverse_sync;

/// Whole directory trees that no enumeration layer may ever yield, each written
/// as its exact sequence of path components relative to a tree root:
///
/// - `.superset/backups` — the tool's own timestamped copies of files a sync
///   overwrote. They are recovered secrets; re-offering or packing one would
///   leak the very bytes the backup exists to protect.
/// - `.superset/.magic` — the Claude plugin's local state. It is gitignored and
///   machine-local, so it must never be committed, pushed into main, or packed.
/// - `.scratchpad` — a scratch tree ss-magic does not own but must never push
///   into the shared main checkout.
/// - `.git` — git's own object store; walking it is pointless and copying it
///   would corrupt a checkout.
///
/// Matching is component-by-component against the HEAD of a candidate path, so
/// the rule is the exact path and never a string prefix or a bare name: a
/// sibling `.superset/.magicked/` stays includable, a root-level `.magic` file
/// is untouched (the rule needs the `.superset` parent), and — crucially —
/// `.superset` itself is never excluded, since widening the rule would drop the
/// contract files (`config.json`, `magic.sh`, `magic.json`) out of sync and pack
/// entirely.
///
/// Distinct from `apply::DEFAULT_EXCLUDES`, which drops a match whose path
/// contains one of a few directory NAMES (`node_modules`, `.venv`) at ANY depth.
pub(crate) const EXCLUDED_TREES: [&[&str]; 4] = [
    &[".superset", "backups"],
    &[".superset", ".magic"],
    &[".scratchpad"],
    &[".git"],
];

/// Whether `rel` IS one of [`EXCLUDED_TREES`] or lives inside one.
///
/// This is the predicate every point of FINAL enumeration applies — the glob
/// walk ([`apply::walk_source`]), the forward copy walk
/// ([`apply::copy_dir_recursive`]), reverse sync's candidate computation, and
/// pack's recursive directory walk. Filtering an upstream match list is not
/// enough on its own: any step that re-walks the live filesystem after the match
/// set is decided would otherwise re-discover the excluded tree through an
/// ancestor directory match (a bare `.superset` pattern, or a broad `**`).
///
/// Returning `true` for the tree root itself is what lets a
/// `WalkDir::filter_entry` caller prune the whole subtree instead of descending
/// into it.
pub(crate) fn under_excluded_tree(rel: &Path) -> bool {
    EXCLUDED_TREES
        .iter()
        .any(|tree| starts_with_components(rel, tree))
}

/// Whether `rel`'s leading path components are exactly `tree`.
///
/// Comparison is per COMPONENT, not per byte, so `.superset/.magicked` does not
/// match the rule `.superset/.magic`. Current-directory markers are skipped so a
/// `./x`-shaped relative path classifies the same as `x`; anything that is not a
/// plain named component (a root, a `..`) fails the match — such paths are
/// rejected upstream anyway.
fn starts_with_components(rel: &Path, tree: &[&str]) -> bool {
    let mut components = rel
        .components()
        .filter(|c| !matches!(c, Component::CurDir));
    for want in tree {
        match components.next() {
            Some(Component::Normal(name)) if name == OsStr::new(*want) => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests;
