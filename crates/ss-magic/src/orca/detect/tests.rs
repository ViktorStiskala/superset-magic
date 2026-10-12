use super::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// A lookup over a fixed set of variables – the injected environment.
fn env_of(vars: &[(&str, String)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), OsString::from(value)))
        .collect();
    move |name: &str| map.get(name).cloned()
}

/// A main checkout and a worktree beside it, as two plain directories: the
/// detection never asks git anything.
fn roots() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    let worktree = dir.path().join("worktree");
    fs::create_dir_all(&main).unwrap();
    fs::create_dir_all(&worktree).unwrap();
    (dir, main, worktree)
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

#[test]
fn no_orca_variables_means_not_an_orca_worktree() {
    let (_dir, main, worktree) = roots();
    assert!(!is_orca_worktree(&worktree, &main, &env_of(&[])));
}

#[test]
fn worktree_path_naming_this_worktree_is_detected() {
    let (_dir, main, worktree) = roots();
    let env = env_of(&[(WORKTREE_PATH_VAR, text(&worktree))]);
    assert!(is_orca_worktree(&worktree, &main, &env));
}

/// The variable may spell the directory through a symlink (macOS reports a
/// temp dir as `/var/…` while its real path is `/private/var/…`). Both sides
/// are canonicalized, so the alias still names this worktree.
#[cfg(unix)]
#[test]
fn worktree_path_through_a_symlinked_alias_is_detected() {
    let (dir, main, worktree) = roots();
    let alias_parent = dir.path().join("alias");
    std::os::unix::fs::symlink(dir.path(), &alias_parent).unwrap();
    let aliased = alias_parent.join("worktree");
    assert_ne!(aliased, worktree.canonicalize().unwrap(), "fixture sanity");

    let env = env_of(&[(WORKTREE_PATH_VAR, text(&aliased))]);
    assert!(is_orca_worktree(&worktree, &main, &env));
    // And the other way round: ss-magic handed the aliased spelling of its root.
    let env = env_of(&[(WORKTREE_PATH_VAR, text(&worktree))]);
    assert!(is_orca_worktree(&aliased, &main, &env));
}

#[test]
fn worktree_id_alone_naming_this_worktree_is_detected() {
    let (_dir, main, worktree) = roots();
    let id = format!("4519ac23-3e21-4b10-ae38-470f6dd00e05::{}", text(&worktree));
    assert!(is_orca_worktree(&worktree, &main, &env_of(&[(WORKTREE_ID_VAR, id)])));
}

#[test]
fn worktree_id_with_a_workspace_suffix_is_detected() {
    let (_dir, main, worktree) = roots();
    let id = format!(
        "4519ac23-3e21-4b10-ae38-470f6dd00e05::{}::workspace:0b8f5a52-9c1e-4d3a-8f77-1a2b3c4d5e6f",
        text(&worktree)
    );
    assert!(is_orca_worktree(&worktree, &main, &env_of(&[(WORKTREE_ID_VAR, id)])));
}

/// AE3. An Orca terminal (its id names worktree A) that changed directory into
/// another worktree B must not switch Orca behavior on in B.
#[test]
fn ae3_variables_naming_another_worktree_are_not_detected() {
    let (dir, main, worktree_b) = roots();
    let worktree_a = dir.path().join("worktree-a");
    fs::create_dir_all(&worktree_a).unwrap();
    let env = env_of(&[
        (WORKTREE_ID_VAR, format!("repo-id::{}", text(&worktree_a))),
        (WORKTREE_PATH_VAR, text(&worktree_a)),
    ]);

    assert!(!is_orca_worktree(&worktree_b, &main, &env));
    assert!(is_orca_worktree(&worktree_a, &main, &env), "fixture sanity");
}

/// AE8 / R24. The main checkout is never an Orca worktree, even when a
/// variable names it.
#[test]
fn ae8_the_main_checkout_is_never_an_orca_worktree() {
    let (_dir, main, _worktree) = roots();
    let env = env_of(&[
        (WORKTREE_ID_VAR, format!("repo-id::{}", text(&main))),
        (WORKTREE_PATH_VAR, text(&main)),
    ]);
    assert!(!is_orca_worktree(&main, &main, &env));
}

#[test]
fn either_variable_is_enough_when_the_other_names_something_else() {
    let (dir, main, worktree) = roots();
    let other = dir.path().join("other");
    fs::create_dir_all(&other).unwrap();

    let env = env_of(&[
        (WORKTREE_PATH_VAR, text(&other)),
        (WORKTREE_ID_VAR, format!("repo-id::{}", text(&worktree))),
    ]);
    assert!(is_orca_worktree(&worktree, &main, &env));

    let env = env_of(&[
        (WORKTREE_PATH_VAR, text(&worktree)),
        (WORKTREE_ID_VAR, format!("repo-id::{}", text(&other))),
    ]);
    assert!(is_orca_worktree(&worktree, &main, &env));
}

#[test]
fn values_that_name_nothing_usable_are_not_detected() {
    let (dir, main, worktree) = roots();
    let gone = dir.path().join("deleted-worktree");
    for (name, value) in [
        (WORKTREE_PATH_VAR, String::new()),
        (WORKTREE_PATH_VAR, "worktree".to_string()),
        (WORKTREE_PATH_VAR, text(&gone)),
        // A folder workspace id has no separator at all.
        (WORKTREE_ID_VAR, "4519ac23-3e21-4b10-ae38-470f6dd00e05".to_string()),
        (WORKTREE_ID_VAR, "repo-id::".to_string()),
        (WORKTREE_ID_VAR, format!("repo-id::{}", text(&gone))),
        (WORKTREE_ID_VAR, text(&worktree)),
    ] {
        assert!(
            !is_orca_worktree(&worktree, &main, &env_of(&[(name, value.clone())])),
            "{name}={value:?} must not be detected"
        );
    }
    // A root that cannot be resolved is not an Orca worktree either.
    let env = env_of(&[(WORKTREE_PATH_VAR, text(&gone))]);
    assert!(!is_orca_worktree(&gone, &main, &env));
}

#[test]
fn the_path_is_everything_after_the_first_separator() {
    assert_eq!(worktree_path_of_id("id::/a/b"), Some("/a/b"));
    assert_eq!(worktree_path_of_id("id::/a::b/c"), Some("/a::b/c"));
    assert_eq!(
        worktree_path_of_id("id::/a/b::workspace:0b8f5a52-9c1e-4d3a-8f77-1a2b3c4d5e6f"),
        Some("/a/b")
    );
    // Not the suffix shape: it stays part of the path.
    assert_eq!(
        worktree_path_of_id("id::/a/b::workspace:not-a-uuid"),
        Some("/a/b::workspace:not-a-uuid")
    );
    assert_eq!(worktree_path_of_id("no-separator"), None);
    assert_eq!(worktree_path_of_id("id::"), None);
}
