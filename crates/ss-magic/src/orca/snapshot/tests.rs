use super::*;
use crate::orca::worktreeinclude::Document;
use ss_magic_core::testutil::{git_run, init_main_repo, write_file};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// The snapshot for an otherwise-empty `.worktreeinclude` and no shared
/// directories.
fn snapshot(root: &Path, patterns: &[&str]) -> Snapshot {
    compute(root, &strings(patterns), &[], Document::empty().budget())
}

/// AE5. A pattern matching an ignored file and a tracked one lists only the
/// ignored file; the tracked one stays ss-magic's own job.
#[test]
fn ae5_only_the_gitignored_match_is_listed() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\n");
    write_file(root, "apps/web/.env", "SECRET=1\n");
    write_file(root, "fixtures/.env", "FIXTURE=1\n");
    git_run(&["add", "-f", "fixtures/.env"], root);
    git_run(&["commit", "-q", "-m", "fixture"], root);

    let snap = snapshot(root, &["**/.env"]);

    assert_eq!(snap.entries, strings(&["apps/web/.env"]));
    assert_eq!(snap.dropped, 0);
    // Forward sync still copies the tracked one: it matches the pattern.
    let matched = apply::match_paths(root, &strings(&["**/.env"])).unwrap();
    assert!(matched.contains(&PathBuf::from("fixtures/.env")));
}

/// AE7. A pattern naming a gitignored directory lists the files in it, never
/// the directory itself.
#[test]
fn ae7_a_directory_match_is_listed_file_by_file() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "config/local/\n");
    write_file(root, "config/local/a.json", "{}");
    write_file(root, "config/local/b.json", "{}");

    let snap = snapshot(root, &["config/local"]);

    assert_eq!(
        snap.entries,
        strings(&["config/local/a.json", "config/local/b.json"])
    );
}

/// An untracked file that is NOT ignored never enters the snapshot (R10).
#[test]
fn an_untracked_file_that_is_not_ignored_is_left_out() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\n");
    write_file(root, ".env", "S=1\n");
    write_file(root, "notes.local", "n\n");

    assert_eq!(snapshot(root, &[".env", "notes.local"]).entries, strings(&[".env"]));
}

/// A bare `.superset` pattern is an ancestor of both state trees. The snapshot
/// lists the gitignored overlay and nothing under either tree.
#[test]
fn a_bare_superset_pattern_never_lists_the_backups_or_plugin_state() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(
        root,
        ".gitignore",
        ".superset/magic.local.json\n.superset/backups/\n.superset/.magic/\n",
    );
    write_file(root, ".superset/magic.json", "{\"files\":[]}\n");
    write_file(root, ".superset/magic.local.json", "{\"files\":[]}\n");
    write_file(root, ".superset/backups/20260101-000000/worktree/.env", "RECOVERED=1\n");
    write_file(root, ".superset/.magic/sessions/x/STATUS.md", "state\n");
    git_run(&["add", ".superset/magic.json"], root);
    git_run(&["commit", "-q", "-m", "magic"], root);

    let snap = snapshot(root, &[".superset"]);

    assert_eq!(snap.entries, strings(&[".superset/magic.local.json"]));
}

/// A `**` pattern matches everything; nothing under `.git`, an excluded tree or
/// a dependency directory may be listed.
#[test]
fn a_catch_all_pattern_lists_no_excluded_tree_and_no_dependency_tree() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(
        root,
        ".gitignore",
        ".env\nnode_modules/\n.venv/\n.superset/\n.scratchpad/\nbuild/\n",
    );
    write_file(root, ".env", "S=1\n");
    write_file(root, "node_modules/a/.env", "x");
    write_file(root, "apps/web/node_modules/b/.env", "x");
    write_file(root, "build/out/node_modules/c/.env", "x");
    write_file(root, "build/out/.venv/pyvenv.cfg", "x");
    write_file(root, "build/out/app.js", "x");
    write_file(root, ".superset/backups/20260101-000000/worktree/.env", "x");
    write_file(root, ".superset/.magic/cache/x", "x");
    write_file(root, ".scratchpad/notes.md", "x");

    let snap = snapshot(root, &["**"]);

    assert_eq!(snap.entries, strings(&[".env", "build/out/app.js"]));
}

/// A file beneath a directory Orca shares is left out: Orca links or clones
/// the whole directory anyway.
#[test]
fn a_file_under_a_shared_directory_is_left_out() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\n.cache/\n");
    write_file(root, ".env", "S=1\n");
    write_file(root, ".cache/.env", "S=1\n");
    write_file(root, "apps/web/.cache/.env", "S=1\n");
    let budget = Document::empty().budget();
    let patterns = strings(&["**/.env", ".env"]);

    let shared = compute(root, &patterns, &strings(&[".cache", "apps/web/.cache"]), budget);
    assert_eq!(shared.entries, strings(&[".env"]));

    // A directory the 100-entry cap cut from Orca's list is not shared, so the
    // file under it is listed after all.
    let one_cut = compute(root, &patterns, &strings(&[".cache"]), budget);
    assert_eq!(one_cut.entries, strings(&[".env", "apps/web/.cache/.env"]));
}

/// R9. The snapshot lists every ignored match, whether or not the person also
/// wrote the path by hand outside the section; their line only costs budget.
#[test]
fn a_path_the_person_also_lists_by_hand_is_still_in_the_snapshot() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\n");
    write_file(root, ".env", "S=1\n");
    write_file(root, "apps/web/.env", "S=1\n");
    let doc = Document::parse(".env\n");

    let snap = compute(root, &strings(&["**/.env", ".env"]), &[], doc.budget());

    assert_eq!(snap.entries, strings(&[".env", "apps/web/.env"]));
    assert_eq!(
        worktreeinclude::entries(&doc.render(&snap.entries)),
        strings(&[".env", "apps/web/.env"]),
        "Orca reads the path once however many times the file names it"
    );
}

/// Outside a git repository nothing is positively ignored: the snapshot is
/// empty, without an error, and forward sync keeps copying everything itself.
#[test]
fn outside_a_repository_the_snapshot_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), ".env", "S=1\n");

    assert_eq!(snapshot(dir.path(), &[".env", "**/.env"]), Snapshot::default());
}

/// Paths git would abort the whole ignore query on – inside a submodule, or
/// beneath a symlinked directory – are dropped first, so every other ignored
/// match is still listed.
#[cfg(unix)]
#[test]
fn submodule_and_symlinked_paths_are_skipped_without_losing_the_rest() {
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
    write_file(root, ".gitignore", ".env\nlink\nreal/\n");
    write_file(root, ".env", "ROOT=1\n");
    write_file(root, "apps/api/.env", "API=1\n");
    write_file(root, "vendor/sub/.env", "SUB=1\n");
    write_file(root, "vendor/sub/node_modules/x/.env", "x");
    write_file(root, "real/.env", "REAL=1\n");
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    std::os::unix::fs::symlink(root.join(".env"), root.join("apps/api/.env.link")).unwrap();

    let snap = snapshot(
        root,
        &["**/.env", ".env", "link/.env", "vendor", "apps/api/.env.link"],
    );

    assert_eq!(
        snap.entries,
        strings(&[".env", "apps/api/.env", "real/.env"]),
        "the submodule, the symlinked directory and the symlink itself are left to ss-magic"
    );
}

/// 1,100 matching files plus 5 hand-written lines: 995 are listed and the
/// warning says 105 were left to ss-magic's own sync.
#[test]
fn more_files_than_orcas_limit_are_capped_after_counting_the_hand_lines() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "secrets/\n");
    let mut all = Vec::new();
    for i in 0..1100 {
        let rel = format!("secrets/s{i:04}.pem");
        write_file(root, &rel, "x");
        all.push(rel);
    }
    all.sort();
    let doc = Document::parse("# mine\nh/one\nh/two\nh/three\nh/four\nh/five\n");

    let snap = compute(root, &strings(&["secrets"]), &[], doc.budget());

    assert_eq!(snap.entries.len(), 995);
    assert_eq!(snap.entries, all[..995], "the first 995 in sorted order");
    assert_eq!(snap.dropped, 105);
    let warning = overflow_warning(&snap).unwrap();
    assert!(warning.contains("105 matching files were left out"), "{warning}");
    assert!(warning.contains("ss-magic's own sync"), "{warning}");

    // The rendered file stays inside both of Orca's limits.
    let rendered = doc.render(&snap.entries);
    assert_eq!(worktreeinclude::entries(&rendered).len(), worktreeinclude::MAX_ENTRIES);
    assert!(rendered.len() <= worktreeinclude::MAX_BYTES);
}

/// The byte limit binds before the entry limit when paths are long: the
/// rendered file must never pass 256 KiB, or Orca ignores it whole.
#[test]
fn the_byte_budget_caps_the_snapshot_when_paths_are_long() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "deep/\n");
    let long = "d".repeat(200);
    for i in 0..900 {
        write_file(root, &format!("deep/{long}/{long}/f{i:04}"), "x");
    }
    let doc = Document::empty();

    let snap = compute(root, &strings(&["deep"]), &[], doc.budget());

    assert!(snap.dropped > 0, "900 paths of ~410 bytes cannot fit in 256 KiB");
    assert_eq!(snap.entries.len() + snap.dropped, 900);
    let rendered = doc.render(&snap.entries);
    assert!(
        rendered.len() <= worktreeinclude::MAX_BYTES,
        "rendered {} bytes",
        rendered.len()
    );
    // And it is filled as far as it can be: one more entry would not fit.
    assert!(rendered.len() + 410 > worktreeinclude::MAX_BYTES);
}

/// A name Orca would misread as something else (a comment, a negation, a
/// glob, a trimmed path) is left out rather than written as a line that means
/// another path. A `#` that is not the line's first character is an ordinary
/// character to Orca, so such a path is listed.
#[test]
fn a_file_name_orca_would_misread_is_left_out() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "odd/\n\\#hash-first.env\n\\!bang-first.env\n");
    write_file(root, "#hash-first.env", "x");
    write_file(root, "!bang-first.env", "x");
    write_file(root, "odd/plain.env", "x");
    write_file(root, "odd/#hash-inside.env", "x");
    write_file(root, "odd/star*.env", "x");
    write_file(root, "odd/trailing-space ", "x");
    write_file(root, "odd/with space.env", "x");

    let snap = snapshot(root, &["odd", "#hash-first.env", "!bang-first.env"]);

    assert_eq!(
        snap.entries,
        strings(&["odd/#hash-inside.env", "odd/plain.env", "odd/with space.env"])
    );
}

#[test]
fn no_warning_when_everything_fits() {
    assert_eq!(overflow_warning(&Snapshot::default()), None);
}
