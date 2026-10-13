use super::*;
use ss_magic_core::testutil::{git_run, init_main_repo, write_file};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A main checkout that ignores `.env` and `.worktreeinclude`, the way a
/// repository set up by this release does.
fn main_with_rule() -> tempfile::TempDir {
    let main = init_main_repo("main");
    write_file(main.path(), ".gitignore", ".env\n.worktreeinclude\n");
    git_run(&["add", ".gitignore"], main.path());
    git_run(&["commit", "-q", "-m", "ignore"], main.path());
    main
}

fn include_text(root: &Path) -> String {
    fs::read_to_string(root.join(worktreeinclude::FILE_NAME)).unwrap()
}

/// Make `<root>/.worktreeinclude` a symlink to `target`, which is taken
/// relative to `root` as a link written by hand would be.
#[cfg(unix)]
fn link_include_to(root: &Path, target: &str) {
    std::os::unix::fs::symlink(target, root.join(worktreeinclude::FILE_NAME)).unwrap();
}

/// Where `<root>/.worktreeinclude` points, read WITHOUT following it. `None`
/// when the entry is missing or is not a symlink any more – which is how a
/// writer that replaced the link with a regular file shows up.
#[cfg(unix)]
fn include_link_target(root: &Path) -> Option<PathBuf> {
    fs::read_link(root.join(worktreeinclude::FILE_NAME)).ok()
}

// --- containment ----------------------------------------------------------

#[test]
fn a_missing_or_plain_file_lands_inside() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(landing(dir.path(), ORCA_YAML).unwrap(), Landing::Inside);
    fs::write(dir.path().join(ORCA_YAML), "a: 1\n").unwrap();
    assert_eq!(landing(dir.path(), ORCA_YAML).unwrap(), Landing::Inside);
}

/// A file committed as a link to something outside the repository is never
/// written through; one that links to a file inside the repository is.
#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_repository_is_refused_and_one_inside_is_written_through() {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("settings.yaml");
    fs::write(&victim, "mine: true\n").unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(&victim, repo.path().join(ORCA_YAML)).unwrap();

    let outcome = write_contained(repo.path(), ORCA_YAML, "scripts: {}\n").unwrap();

    assert_eq!(outcome, Landing::Outside(victim.canonicalize().unwrap()));
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "mine: true\n",
        "the file outside the repository must be byte-identical"
    );

    // An in-repo link: the write goes to its target and the link stays a link.
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join("config")).unwrap();
    fs::write(repo.path().join("config/orca.yaml"), "old: 1\n").unwrap();
    std::os::unix::fs::symlink("config/orca.yaml", repo.path().join(ORCA_YAML)).unwrap();

    let outcome = write_contained(repo.path(), ORCA_YAML, "new: 1\n").unwrap();

    assert_eq!(outcome, Landing::Inside);
    assert_eq!(fs::read_to_string(repo.path().join("config/orca.yaml")).unwrap(), "new: 1\n");
    assert!(fs::symlink_metadata(repo.path().join(ORCA_YAML))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn an_unresolvable_root_is_an_error_not_a_pass() {
    let dir = tempfile::tempdir().unwrap();
    assert!(landing(&dir.path().join("gone"), ORCA_YAML).is_err());
}

// --- the automatic snapshot refresh ----------------------------------------

#[test]
fn the_refresh_writes_the_section_when_the_file_is_ignored_and_untracked() {
    let main = main_with_rule();
    write_file(main.path(), ".env", "S=1\n");
    write_file(main.path(), "apps/web/.env", "S=1\n");

    let outcome = refresh_snapshot(main.path(), &strings(&["**/.env", ".env"]));

    assert_eq!(
        outcome,
        SnapshotRefresh::Written {
            listed: 2,
            warning: None
        }
    );
    assert_eq!(
        worktreeinclude::entries(&include_text(main.path())),
        strings(&[".env", "apps/web/.env"])
    );
    // Nothing to do the second time: not one byte is rewritten.
    assert_eq!(
        refresh_snapshot(main.path(), &strings(&["**/.env", ".env"])),
        SnapshotRefresh::Unchanged { warning: None }
    );
}

#[test]
fn the_refresh_keeps_hand_written_lines() {
    let main = main_with_rule();
    write_file(main.path(), ".env", "S=1\n");
    write_file(main.path(), worktreeinclude::FILE_NAME, "# mine\nsecrets/local.pem\n");

    refresh_snapshot(main.path(), &strings(&[".env"]));

    let text = include_text(main.path());
    assert!(text.starts_with("# mine\nsecrets/local.pem\n"), "{text}");
    assert_eq!(
        worktreeinclude::entries(&text),
        strings(&["secrets/local.pem", ".env"])
    );
}

/// R26. Without the ignore rule nothing is written: the file would show up as
/// untracked in the shared main checkout.
#[test]
fn the_refresh_is_skipped_when_the_file_is_not_gitignored() {
    let main = init_main_repo("main");
    write_file(main.path(), ".gitignore", ".env\n");
    write_file(main.path(), ".env", "S=1\n");

    let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

    let SnapshotRefresh::Skipped(why) = outcome else {
        panic!("expected a skip, got {outcome:?}");
    };
    assert!(why.contains("does not gitignore"), "{why}");
    assert!(!why.contains("  "), "one tidy sentence: {why:?}");
    assert!(why.contains("ss-magic orca"), "the notice says how to set it up: {why}");
    assert!(!main.path().join(worktreeinclude::FILE_NAME).exists());
}

/// R26. A tracked file is never rewritten by the automatic refresh, even when
/// an ignore rule also names it.
#[test]
fn the_refresh_is_skipped_when_the_file_is_tracked() {
    let main = main_with_rule();
    write_file(main.path(), ".env", "S=1\n");
    write_file(main.path(), worktreeinclude::FILE_NAME, "committed/by-hand\n");
    git_run(&["add", "-f", worktreeinclude::FILE_NAME], main.path());
    git_run(&["commit", "-q", "-m", "track it"], main.path());

    let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

    let SnapshotRefresh::Skipped(why) = outcome else {
        panic!("expected a skip, got {outcome:?}");
    };
    assert!(why.contains("tracked by git"), "{why}");
    assert!(!why.contains("  "), "one tidy sentence: {why:?}");
    assert_eq!(include_text(main.path()), "committed/by-hand\n");
}

/// R26. The automatic refresh must never leave a tracked file modified in the
/// shared main checkout. Git is asked about the NAME `.worktreeinclude` only,
/// and a name that is an untracked, ignored symlink passes both of those
/// questions while a write through it lands on the link's target – here a
/// committed file. So the refresh does not write through a symlink at all.
#[cfg(unix)]
#[test]
fn the_refresh_never_writes_a_tracked_file_through_a_symlink() {
    let main = main_with_rule();
    let committed = "# shared by the team\nvendor/licence.key\n";
    write_file(main.path(), "shared.include", committed);
    git_run(&["add", "shared.include"], main.path());
    git_run(&["commit", "-q", "-m", "a tracked list"], main.path());
    link_include_to(main.path(), "shared.include");
    // An ignored secret the patterns match, so the snapshot is not empty and
    // the rendered file differs from what the link's target holds.
    write_file(main.path(), ".env", "S=1\n");
    // The two answers git gives about the name are exactly the ones that let
    // the refresh write: the link itself is neither tracked nor unignored.
    let name = worktreeinclude::FILE_NAME;
    assert!(git::tracked_files(main.path(), &[name]).unwrap().is_empty());
    assert!(git::is_ignored_str(main.path(), name).unwrap());

    let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

    let SnapshotRefresh::Skipped(why) = outcome else {
        panic!("expected a skip, got {outcome:?}");
    };
    assert!(why.contains("is a symlink"), "{why}");
    assert!(why.contains("ss-magic orca"), "the notice says what does update it: {why}");
    assert!(!why.contains("  "), "one tidy sentence: {why:?}");
    assert_eq!(
        fs::read_to_string(main.path().join("shared.include")).unwrap(),
        committed,
        "the tracked file the link points at must be byte-identical"
    );
    assert_eq!(
        include_link_target(main.path()),
        Some(PathBuf::from("shared.include")),
        "the link itself is left as it was"
    );
    assert_eq!(
        git::status_porcelain(main.path(), &["."]).unwrap(),
        Vec::new(),
        "git must see nothing modified and nothing new in the main checkout"
    );
}

/// The refresh does not judge where a link leads: one that points at an
/// untracked, ignored file inside the repository is not written through
/// either, so the person's own file behind it keeps its bytes.
#[cfg(unix)]
#[test]
fn the_refresh_never_writes_an_untracked_file_through_a_symlink() {
    let main = main_with_rule();
    write_file(main.path(), ".gitignore", ".env\n.worktreeinclude\nlocal.include\n");
    git_run(&["commit", "-q", "-am", "ignore the local list"], main.path());
    let by_hand = "# mine\nsecrets/local.pem\n";
    write_file(main.path(), "local.include", by_hand);
    link_include_to(main.path(), "local.include");
    write_file(main.path(), ".env", "S=1\n");

    let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

    assert!(
        matches!(&outcome, SnapshotRefresh::Skipped(why) if why.contains("is a symlink")),
        "{outcome:?}"
    );
    assert_eq!(fs::read_to_string(main.path().join("local.include")).unwrap(), by_hand);
    assert_eq!(
        include_link_target(main.path()),
        Some(PathBuf::from("local.include"))
    );
}

/// A link that leaves the repository is turned away by the same check, before
/// the file behind it is even read – the containment check at the write is
/// only the second line of defence for it.
#[cfg(unix)]
#[test]
fn the_refresh_never_writes_through_a_symlink_out_of_the_repository() {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("mine.include");
    fs::write(&victim, "# elsewhere\n").unwrap();
    let main = main_with_rule();
    link_include_to(main.path(), victim.to_str().unwrap());
    write_file(main.path(), ".env", "S=1\n");

    let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

    // The reason is the link check's own, not the containment check's "is a
    // symlink to <path>, outside the repository".
    assert!(
        matches!(&outcome, SnapshotRefresh::Skipped(why) if why.contains("never writes through one")),
        "{outcome:?}"
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), "# elsewhere\n");
    assert_eq!(include_link_target(main.path()), Some(victim));
}

/// A link to nothing and a link to a directory are skipped for the same
/// reason. Neither is replaced by a regular file, and nothing is created where
/// the dangling link points.
#[cfg(unix)]
#[test]
fn the_refresh_skips_a_dangling_link_and_a_link_to_a_directory() {
    for target in ["no-such-file", "apps"] {
        let main = main_with_rule();
        fs::create_dir(main.path().join("apps")).unwrap();
        link_include_to(main.path(), target);
        write_file(main.path(), ".env", "S=1\n");

        let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

        assert!(
            matches!(&outcome, SnapshotRefresh::Skipped(why) if why.contains("is a symlink")),
            "a link to `{target}`: {outcome:?}"
        );
        assert_eq!(
            include_link_target(main.path()),
            Some(PathBuf::from(target)),
            "the link to `{target}` is still a link"
        );
        assert!(!main.path().join("no-such-file").exists());
    }
}

/// When the entry cannot even be inspected, nobody knows whether it is a link,
/// so the answer is the refusing one. A root that is a regular file makes the
/// lookup fail with something other than "not found".
#[cfg(unix)]
#[test]
fn the_refresh_is_skipped_when_the_entry_cannot_be_inspected() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_directory = dir.path().join("plain-file");
    fs::write(&not_a_directory, "x").unwrap();

    let outcome = refresh_snapshot(&not_a_directory, &strings(&[".env"]));

    assert!(
        matches!(&outcome, SnapshotRefresh::Skipped(why) if why.contains("could not be inspected")),
        "{outcome:?}"
    );
}

/// Outside a repository git cannot vouch for anything, so nothing is written.
#[test]
fn the_refresh_is_skipped_when_git_cannot_be_asked() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), ".env", "S=1\n");

    assert!(matches!(
        refresh_snapshot(dir.path(), &strings(&[".env"])),
        SnapshotRefresh::Skipped(_)
    ));
    assert!(!dir.path().join(worktreeinclude::FILE_NAME).exists());
}

/// A file that is not text is never rebuilt from an empty parse.
#[test]
fn the_refresh_never_rewrites_a_file_it_cannot_read() {
    let main = main_with_rule();
    write_file(main.path(), ".env", "S=1\n");
    let bytes = b"caf\xe9/secret.pem\n";
    fs::write(main.path().join(worktreeinclude::FILE_NAME), bytes).unwrap();

    let outcome = refresh_snapshot(main.path(), &strings(&[".env"]));

    assert!(matches!(&outcome, SnapshotRefresh::Skipped(why) if why.contains("UTF-8")));
    assert!(matches!(&outcome, SnapshotRefresh::Skipped(why) if !why.contains("  ")));
    assert_eq!(fs::read(main.path().join(worktreeinclude::FILE_NAME)).unwrap(), bytes);
}

/// The files under a directory Orca already shares – per `orca.yaml` as it is
/// on disk – are left out, and `orca.yaml` itself is never touched.
#[test]
fn the_refresh_reads_orca_yaml_for_shared_directories_and_never_writes_it() {
    let main = main_with_rule();
    write_file(main.path(), ".gitignore", ".env\n.worktreeinclude\n.cache/\n");
    write_file(main.path(), ".env", "S=1\n");
    write_file(main.path(), ".cache/.env", "S=1\n");
    write_file(main.path(), "tools/.cache/.env", "S=1\n");
    let yaml = "# by hand\nworktree:\n  sharedDirectories:\n    - .cache\n";
    write_file(main.path(), ORCA_YAML, yaml);

    refresh_snapshot(main.path(), &strings(&["**/.env", ".env"]));

    assert_eq!(
        worktreeinclude::entries(&include_text(main.path())),
        strings(&[".env", "tools/.cache/.env"]),
        "`.cache/.env` is under a shared directory; `tools/.cache` is not shared"
    );
    assert_eq!(fs::read_to_string(main.path().join(ORCA_YAML)).unwrap(), yaml);
}
