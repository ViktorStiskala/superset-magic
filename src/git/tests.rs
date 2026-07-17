use super::*;
use crate::tests::support::git_run;
use std::fs;
use tempfile::TempDir;

fn init_repo(initial_branch: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", initial_branch], dir.path());
    crate::tests::support::neutralize_global_excludes(dir.path());
    // empty commit so the branch exists as a ref
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

// ── Reverse-sync probes (U11) ───────────────────────────────────────────

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// `untracked_files` lists every untracked file — INCLUDING gitignored
/// ones — and excludes only tracked files. Including ignored files is the
/// behavior reverse sync depends on: the secrets it pushes are gitignored.
#[test]
fn untracked_files_lists_untracked_including_ignored() {
    let dir = init_repo("main");
    let root = dir.path();
    // tracked file (README.md committed by init_repo).
    // untracked, not ignored:
    write(root, "new.txt", "hi\n");
    // untracked AND gitignored (the secret-file shape reverse sync targets):
    write(root, ".gitignore", "**/.dev.vars\nignored.txt\n");
    write(root, "apps/api/.dev.vars", "SECRET=1\n");
    write(root, "ignored.txt", "nope\n");
    // The .gitignore itself is untracked here, so it would also show up.
    // Track it to keep the assertion focused on the data files.
    git_run(&["add", ".gitignore"], root);

    // Empty pathspecs → list the whole tree.
    let mut got: Vec<String> = untracked_files(root, &[])
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    got.sort();

    assert!(
        got.contains(&"apps/api/.dev.vars".to_string()),
        "gitignored untracked secret MUST be listed; got {got:?}"
    );
    assert!(
        got.contains(&"ignored.txt".to_string()),
        "gitignored untracked file MUST be listed; got {got:?}"
    );
    assert!(got.contains(&"new.txt".to_string()), "got {got:?}");
    assert!(
        !got.contains(&"README.md".to_string()),
        "tracked file must NOT be listed; got {got:?}"
    );
}

/// Pathspecs scope the probe: a gitignored secret reachable via an explicit
/// pathspec is listed (incl. one whose whole parent DIRECTORY is ignored),
/// while untracked files NOT named by a pathspec are omitted — so git never
/// has to walk unrelated ignored trees. Pins the perf-scoping behavior
/// reverse sync relies on.
#[test]
fn untracked_files_scopes_to_pathspecs_and_includes_ignored() {
    let dir = init_repo("main");
    let root = dir.path();
    // `secrets/` is gitignored as a whole DIRECTORY; `noise/` is unrelated.
    write(root, ".gitignore", "secrets/\nnoise/\n");
    git_run(&["add", ".gitignore"], root);
    write(root, "secrets/api.key", "KEY=1\n"); // gitignored via dir rule
    write(root, "noise/junk.bin", "x\n"); // untracked, not named below

    let got: Vec<String> = untracked_files(root, &["secrets/api.key"])
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    assert_eq!(
        got,
        vec!["secrets/api.key".to_string()],
        "pathspec must list the dir-ignored secret and nothing outside the pathspec; got {got:?}"
    );
}

/// `is_ignored` reports true for a gitignored path, false otherwise.
#[test]
fn is_ignored_reflects_gitignore() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "**/.dev.vars\n");
    assert!(is_ignored(root, Path::new("apps/api/.dev.vars")).unwrap());
    assert!(!is_ignored(root, Path::new("apps/api/config.json")).unwrap());
}

/// `check_ignore_pattern` returns the covering glob, not the literal path,
/// when a glob rule matches.
#[test]
fn check_ignore_pattern_returns_covering_glob() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "**/.dev.vars\n");
    let pat = check_ignore_pattern(root, Path::new("apps/api/.dev.vars")).unwrap();
    assert_eq!(pat, Some("**/.dev.vars".to_string()));
}

/// `check_ignore_pattern` returns the literal pattern when the rule is an
/// exact path entry.
#[test]
fn check_ignore_pattern_returns_literal_when_exact() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "secrets/api.key\n");
    let pat = check_ignore_pattern(root, Path::new("secrets/api.key")).unwrap();
    assert_eq!(pat, Some("secrets/api.key".to_string()));
}

/// `check_ignore_pattern` returns None when no rule covers the path.
#[test]
fn check_ignore_pattern_none_when_no_rule() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "node_modules/\n");
    let pat = check_ignore_pattern(root, Path::new("apps/api/.dev.vars")).unwrap();
    assert_eq!(pat, None);
}

/// A negation rule (`!…`) is reported as None, so callers fall back to the
/// literal path rather than copying a negation into main.
#[test]
fn check_ignore_pattern_negation_is_none() {
    let dir = init_repo("main");
    let root = dir.path();
    // Ignore everything, then un-ignore the secret — git check-ignore -v
    // reports the LAST matching rule, which is the negation.
    write(root, ".gitignore", "**/.dev.vars\n!apps/api/.dev.vars\n");
    let pat = check_ignore_pattern(root, Path::new("apps/api/.dev.vars")).unwrap();
    assert_eq!(pat, None, "negation rule must not be copied as a pattern");
}

/// Pure parser unit: pattern after the second colon, before the tab.
#[test]
fn parse_check_ignore_line_extracts_pattern() {
    let line = ".gitignore:1:**/.dev.vars\tapps/api/.dev.vars";
    assert_eq!(
        parse_check_ignore_line(line),
        Some("**/.dev.vars".to_string())
    );
}

#[test]
fn parse_check_ignore_line_handles_negation_and_empty() {
    assert_eq!(
        parse_check_ignore_line(".gitignore:2:!foo\tfoo"),
        None,
        "negation → None"
    );
    assert_eq!(parse_check_ignore_line(""), None);
}

// ── Unified sync (Task 5) ────────────────────────────────────────────────

/// `is_ignored_str` on a directory-shaped query (trailing slash) matches a
/// `foo/bar/` gitignore rule even though `foo/bar` does not exist on disk yet
/// — the whole point of the raw-pathname variant over `is_ignored`, which can
/// only query paths that already exist. The no-slash query for the same
/// (still-absent) path is treated as a file and MISSES the directory rule.
#[test]
fn is_ignored_str_dir_trailing_slash_matches_before_dir_exists() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "foo/bar/\n");
    // `foo/bar` does not exist on disk at all.
    assert!(
        is_ignored_str(root, "foo/bar/").unwrap(),
        "trailing-slash query must match the dir rule before the dir exists"
    );
    assert!(
        !is_ignored_str(root, "foo/bar").unwrap(),
        "no-slash query on an absent path must NOT match a directory-only rule"
    );
}

/// `tracked_files` returns only the git-TRACKED members of `pathspecs`: a
/// committed file is listed, a merely-present-on-disk untracked file is not
/// — scoped to the pathspecs passed, mirroring `untracked_files`'s contract.
#[test]
fn tracked_files_returns_committed_excludes_untracked() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, "tracked.txt", "hi\n");
    git_run(&["add", "tracked.txt"], root);
    git_run(&["commit", "-q", "-m", "add tracked.txt"], root);
    write(root, "untracked.txt", "nope\n");

    let got: Vec<String> = tracked_files(root, &["tracked.txt", "untracked.txt"])
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    assert!(
        got.contains(&"tracked.txt".to_string()),
        "committed file must be listed; got {got:?}"
    );
    assert!(
        !got.contains(&"untracked.txt".to_string()),
        "untracked file must NOT be listed; got {got:?}"
    );
}
