use super::*;
use ss_magic_core::testutil::{git_run, init_main_repo, write_file};
use std::fs;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn defaults() -> Vec<String> {
    DEFAULT_PATTERNS.iter().map(|p| p.to_string()).collect()
}

fn rels(paths: Vec<PathBuf>) -> Vec<String> {
    let mut out: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The R6 fixture: root and per-package `node_modules`, one nested inside
/// another, a virtualenv, a JS build cache and a symlinked `node_modules`.
#[cfg(unix)]
#[test]
fn detects_dependency_directories_but_never_a_nested_or_symlinked_one() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n.venv/\n.next/\n");
    write_file(root, "node_modules/left-pad/index.js", "x");
    write_file(root, "apps/web/node_modules/react/index.js", "x");
    write_file(root, "apps/web/node_modules/x/node_modules/y/index.js", "x");
    write_file(root, ".venv/bin/python", "x");
    write_file(root, "apps/web/.next/cache/webpack/a.pack", "x");
    write_file(root, "apps/web/.next/server/page.js", "x");
    write_file(root, "apps/web/src/index.ts", "x");
    fs::create_dir_all(root.join("apps/linked")).unwrap();
    std::os::unix::fs::symlink(root.join("node_modules"), root.join("apps/linked/node_modules"))
        .unwrap();

    let expansion = expand(root, &defaults());

    assert_eq!(
        expansion.dirs,
        strings(&[
            ".venv",
            "apps/web/.next/cache",
            "apps/web/node_modules",
            "node_modules",
        ])
    );
    assert_eq!(expansion.dropped, 0);
    assert_eq!(overflow_warning(&expansion), None);
}

#[test]
fn a_directory_that_is_not_gitignored_is_not_shared() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n");
    write_file(root, "node_modules/a/index.js", "x");
    // A `venv/` nobody ignored, and one that is committed: both are source to
    // git, so neither may be handed to Orca as a shareable cache.
    write_file(root, "venv/pyvenv.cfg", "x");
    write_file(root, "tools/.cache/kept.txt", "x");
    git_run(&["add", "-f", "tools/.cache/kept.txt"], root);
    git_run(&["commit", "-q", "-m", "tracked cache"], root);

    assert_eq!(expand(root, &defaults()).dirs, strings(&["node_modules"]));
}

#[test]
fn only_the_stored_patterns_are_expanded() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n.venv/\n");
    write_file(root, "node_modules/a/index.js", "x");
    write_file(root, "apps/api/node_modules/a/index.js", "x");
    write_file(root, ".venv/pyvenv.cfg", "x");

    assert_eq!(
        expand(root, &strings(&["**/node_modules"])).dirs,
        strings(&["apps/api/node_modules", "node_modules"])
    );
    assert_eq!(expand(root, &strings(&["**/.venv"])).dirs, strings(&[".venv"]));
    assert!(expand(root, &[]).dirs.is_empty(), "no patterns, no directories");
    assert!(expand(root, &strings(&["[unclosed"])).dirs.is_empty());
}

/// A pattern the person added by hand is honored too: the walk records what it
/// matches, not only the default kinds.
#[test]
fn a_custom_pattern_is_matched_by_the_walk() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "dist/\nvendor/bundle/\n");
    write_file(root, "packages/ui/dist/index.js", "x");
    write_file(root, "packages/ui/dist/nested/dist/deep.js", "x");
    write_file(root, "vendor/bundle/gems/a.rb", "x");

    let expansion = expand(root, &strings(&["**/dist", "vendor/bundle"]));

    assert_eq!(expansion.dirs, strings(&["packages/ui/dist", "vendor/bundle"]));
}

/// 120 matching directories: the first 100 in sorted order are kept and the
/// warning says how many will not be shared.
#[test]
fn more_than_orcas_limit_keeps_the_first_hundred_sorted_and_warns() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n");
    let mut all = Vec::new();
    for i in 0..120 {
        let dir = format!("packages/p{i:03}/node_modules");
        write_file(root, &format!("{dir}/a.js"), "x");
        all.push(dir);
    }
    all.sort();

    let expansion = expand(root, &strings(&["**/node_modules"]));

    assert_eq!(expansion.dirs.len(), MAX_SHARED_DIRS);
    assert_eq!(expansion.dirs, all[..100]);
    assert_eq!(expansion.dropped, 20);
    let warning = overflow_warning(&expansion).unwrap();
    assert!(
        warning.contains("20 matching directories will not be shared"),
        "{warning}"
    );
}

/// A submodule's directories belong to another repository, and a path inside
/// one would make git abort the whole ignore query – losing the answer for
/// every other directory as well.
#[test]
fn a_submodule_is_not_entered_and_does_not_cost_the_other_answers() {
    let sub = init_main_repo("main");
    let main = init_main_repo("main");
    let root = main.path();
    git_run(
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            sub.path().to_str().unwrap(),
            "vendor/sub",
        ],
        root,
    );
    write_file(root, ".gitignore", "node_modules/\n");
    write_file(root, "node_modules/a/index.js", "x");
    write_file(root, "vendor/sub/node_modules/b/index.js", "x");

    assert_eq!(
        rels(walk_candidates(root, &[])),
        strings(&["node_modules"]),
        "the walk stops at a directory holding a `.git` entry"
    );
    assert_eq!(expand(root, &defaults()).dirs, strings(&["node_modules"]));
}

/// A directory tagged as a cache (a Rust `target/`) is build output: the walk
/// stays out of it, and the excluded trees are never entered either.
#[test]
fn the_walk_stays_out_of_tagged_cache_directories_and_excluded_trees() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_file(root, "target/CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n");
    write_file(root, "target/debug/node_modules/x.js", "x");
    write_file(root, ".git/node_modules/x.js", "x");
    write_file(root, ".superset/backups/20260101-000000/worktree/node_modules/x.js", "x");
    write_file(root, ".superset/.magic/.cache/x", "x");
    write_file(root, ".scratchpad/node_modules/x.js", "x");
    write_file(root, "src/node_modules/x.js", "x");

    assert_eq!(rels(walk_candidates(root, &[])), strings(&["src/node_modules"]));
}

/// Outside a git repository nothing is positively ignored, so nothing is
/// shared – the candidates exist, the answer is still empty.
#[test]
fn outside_a_repository_the_list_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "node_modules/a/index.js", "x");

    assert_eq!(rels(walk_candidates(dir.path(), &[])), strings(&["node_modules"]));
    assert_eq!(expand(dir.path(), &defaults()), Expansion::default());
    assert!(detected_default_patterns(dir.path()).is_empty());
}

#[test]
fn matches_are_reported_per_pattern_for_the_picker() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n.turbo/\n");
    write_file(root, "node_modules/a/index.js", "x");
    write_file(root, "apps/web/node_modules/a/index.js", "x");
    write_file(root, ".turbo/cache/a", "x");

    let rows = matches_by_pattern(root, &defaults());

    assert_eq!(rows.len(), DEFAULT_PATTERNS.len());
    assert_eq!(rows[0].pattern, "**/node_modules");
    assert_eq!(rows[0].dirs, strings(&["apps/web/node_modules", "node_modules"]));
    let turbo = rows.iter().find(|row| row.pattern == "**/.turbo").unwrap();
    assert_eq!(turbo.dirs, strings(&[".turbo"]));
    let venv = rows.iter().find(|row| row.pattern == "**/.venv").unwrap();
    assert!(venv.dirs.is_empty());

    assert_eq!(
        detected_default_patterns(root),
        strings(&["**/node_modules", "**/.turbo"]),
        "only kinds that match something, in the picker's order"
    );
}

// --- the picker's options ------------------------------------------------------

fn row(pattern: &str, dirs: &[&str]) -> PatternMatches {
    PatternMatches {
        pattern: pattern.to_string(),
        dirs: strings(dirs),
    }
}

#[test]
fn the_picker_offers_the_default_kinds_then_stored_custom_patterns() {
    assert_eq!(picker_patterns(None), defaults());

    let stored = strings(&["**/dist", "**/node_modules", "vendor/bundle"]);
    let mut expected = defaults();
    expected.extend(strings(&["**/dist", "vendor/bundle"]));
    assert_eq!(picker_patterns(Some(&stored)), expected);
}

/// Never chosen: what matches something starts checked. Chosen before: exactly
/// the stored choice starts checked, whatever matches – including "none".
#[test]
fn the_picker_preselects_matches_at_first_and_the_stored_choice_afterwards() {
    let rows = vec![
        row("**/node_modules", &["node_modules"]),
        row("**/.venv", &[]),
        row("**/.turbo", &[".turbo"]),
        row("**/dist", &[]),
    ];

    assert_eq!(picker_preselected(None, &rows), vec![0, 2]);

    let stored = strings(&["**/.venv", "**/dist"]);
    assert_eq!(picker_preselected(Some(&stored), &rows), vec![1, 3]);

    assert!(picker_preselected(Some(&[]), &rows).is_empty());
}

#[test]
fn without_a_stored_choice_the_detected_kinds_are_stored() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n.venv/\n");
    write_file(root, "node_modules/a/index.js", "x");
    write_file(root, ".venv/pyvenv.cfg", "x");

    assert_eq!(
        stored_or_detected(root, None),
        strings(&["**/node_modules", "**/.venv"])
    );
    assert_eq!(stored_or_detected(root, Some(&[])), Vec::<String>::new());
    assert_eq!(
        stored_or_detected(root, Some(&strings(&["**/dist"]))),
        strings(&["**/dist"])
    );
}
