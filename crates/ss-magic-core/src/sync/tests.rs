//! Unit tests for the shared excluded-trees predicate. The behavioural
//! coverage — that each enumeration point actually applies it — lives with the
//! enumeration points themselves (`sync::apply`, `sync::reverse_sync`, `pack`).

use super::*;
use std::path::Path;

fn excluded(rel: &str) -> bool {
    under_excluded_tree(Path::new(rel))
}

#[test]
fn every_excluded_tree_root_and_its_contents_match() {
    for rel in [
        ".superset/backups",
        ".superset/backups/20260101-000000/main/.env",
        ".superset/.magic",
        ".superset/.magic/state.json",
        ".scratchpad",
        ".scratchpad/notes.md",
        ".git",
        ".git/objects/ab/cdef",
    ] {
        assert!(excluded(rel), "{rel} must be excluded");
    }
}

/// The tree root itself matching is what lets a `WalkDir::filter_entry` caller
/// prune the subtree instead of descending into it.
#[test]
fn tree_roots_match_so_a_walk_can_prune_them() {
    assert!(excluded(".superset/.magic"));
    assert!(excluded(".git"));
}

/// The contract files live directly under `.superset`, so widening any rule to
/// its first component would silently drop them from sync and pack.
#[test]
fn superset_itself_is_never_excluded() {
    for rel in [
        ".superset",
        ".superset/config.json",
        ".superset/magic.sh",
        ".superset/magic.json",
        ".superset/magic.local.json",
    ] {
        assert!(!excluded(rel), "{rel} must NOT be excluded");
    }
}

/// Matching is per component, never a string prefix and never a bare name.
#[test]
fn matching_is_per_component_not_prefix_or_bare_name() {
    for rel in [
        // Prefix look-alikes of an excluded component.
        ".superset/.magicked/keep.txt",
        ".superset/backupsfoo/keep.txt",
        ".scratchpadding/notes.md",
        ".github/workflows/ci.yml",
        // The right names in the wrong place: the rules are two-component
        // paths, so these need their `.superset` parent to match.
        ".magic/state.json",
        "backups/old.env",
        "vendor/.superset/.magic/state.json",
    ] {
        assert!(!excluded(rel), "{rel} must NOT be excluded");
    }
}

/// A `./`-prefixed path classifies the same as its bare form, and an empty
/// path (the walk root) is not itself an excluded tree.
#[test]
fn curdir_markers_are_skipped_and_the_root_is_not_excluded() {
    assert!(excluded("./.superset/.magic/state.json"));
    assert!(excluded(".superset/./backups/x"));
    assert!(!excluded(""));
    assert!(!excluded("."));
}
