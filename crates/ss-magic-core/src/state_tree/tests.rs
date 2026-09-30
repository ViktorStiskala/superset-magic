use std::fs;
use std::path::Path;

use super::*;
use crate::sync::EXCLUDED_TREES;
use crate::testutil::{git_run, init_main_repo};

/// The state tree has one spelling. The sync-exclusion rule in
/// `sync::EXCLUDED_TREES` and the gitignore rule written here must name the
/// same directory, or a rename on one side would leave the other guarding a
/// path nothing uses.
#[test]
fn state_rel_matches_the_excluded_trees_entry() {
    let components: Vec<&str> = STATE_REL.split('/').collect();
    assert!(
        EXCLUDED_TREES.iter().any(|tree| *tree == components.as_slice()),
        "{STATE_REL} is not one of {EXCLUDED_TREES:?}"
    );
    assert_eq!(
        Path::new(STATE_REL).components().count(),
        2,
        "the state tree sits directly under .superset"
    );
}

/// A repository with no rule gains exactly one, at the git root, and git then
/// reports the tree ignored.
#[test]
fn writes_the_rule_once_at_the_root() {
    let repo = init_main_repo("main");
    let root = repo.path();

    ensure_state_ignored(root).unwrap();
    let body = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        body.lines().any(|l| l == ".superset/.magic/"),
        "expected an anchored dir rule, got:\n{body}"
    );
    assert!(crate::git::is_ignored_no_index_str(root, ".superset/.magic/").unwrap());

    // Idempotent: a second call appends nothing.
    ensure_state_ignored(root).unwrap();
    let again = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(body, again);
}

/// A rule that already covers the tree – here a broader `.superset/` glob – is
/// honored: nothing is appended, because git already ignores the path.
#[test]
fn respects_an_existing_covering_rule() {
    let repo = init_main_repo("main");
    let root = repo.path();
    fs::write(root.join(".gitignore"), ".superset/\n").unwrap();
    git_run(&["add", ".gitignore"], root);
    git_run(&["commit", "-q", "-m", "ignore"], root);

    ensure_state_ignored(root).unwrap();
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), ".superset/\n");
}
