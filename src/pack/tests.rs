use super::*;
use crate::workspace::superset_files;
use crate::tests::support::git_run;
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::process::ExitCode;
use tempfile::TempDir;

/// Fixed archive name written by pack before 0.3 — covered by the root-level
/// `ss-magic-*.tar.bz2` exclusion class; kept here as a regression fixture.
const LEGACY_PACK_FILE_NAME: &str = "ss-magic-files.tar.bz2";

fn exit_ok(code: ExitCode) -> bool {
    code == ExitCode::SUCCESS
}

/// Init a git repo with one commit so `cwd_repo_root` resolves.
fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", "main"], dir.path());
    crate::tests::support::neutralize_global_excludes(dir.path());
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

fn write_magic(root: &Path, patterns: &[&str]) {
    fs::create_dir_all(root.join(".superset")).unwrap();
    let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let cfg = superset_files::MagicConfig { files };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
    fs::write(root.join(".superset/magic.json"), body).unwrap();
}

fn write_file(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// Decompress `<root>/<derived archive name>` and return the set of file
/// entry paths (directories are represented by their contained files).
fn archive_entries(root: &Path) -> BTreeSet<String> {
    let f = fs::File::open(root.join(archive_file_name(root))).unwrap();
    let dec = bzip2::read::BzDecoder::new(f);
    let mut ar = tar::Archive::new(dec);
    let mut out = BTreeSet::new();
    for entry in ar.entries().unwrap() {
        let entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        // Record only file entries (skip bare directory headers).
        if entry.header().entry_type().is_file() {
            out.insert(path);
        }
    }
    out
}

/// Read a single file's bytes out of the archive.
fn archive_read(root: &Path, rel: &str) -> Option<String> {
    let f = fs::File::open(root.join(archive_file_name(root))).unwrap();
    let dec = bzip2::read::BzDecoder::new(f);
    let mut ar = tar::Archive::new(dec);
    for entry in ar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        if path == rel {
            let mut s = String::new();
            entry.read_to_string(&mut s).unwrap();
            return Some(s);
        }
    }
    None
}

// ── Happy paths ────────────────────────────────────────────────────────

#[test]
fn packs_literal_file_with_matching_bytes() {
    let repo = init_repo();
    write_magic(repo.path(), &[".env"]);
    write_file(repo.path(), ".env", "FOO=1\n");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code), "pack_core must succeed");
    assert!(
        repo.path().join(archive_file_name(repo.path())).is_file(),
        "archive must exist at git root"
    );
    assert_eq!(archive_read(repo.path(), ".env").as_deref(), Some("FOO=1\n"));
}

#[test]
fn packs_glob_matches_at_depth() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/.dev.vars"]);
    write_file(repo.path(), "apps/api/.dev.vars", "A=1\n");
    write_file(repo.path(), "apps/web/.dev.vars", "B=2\n");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(entries.contains("apps/api/.dev.vars"), "got {entries:?}");
    assert!(entries.contains("apps/web/.dev.vars"), "got {entries:?}");
}

#[test]
fn preserves_repo_relative_structure_not_flattened_or_absolute() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/.env"]);
    write_file(repo.path(), "a/b/c/.env", "DEEP=1\n");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(
        entries.contains("a/b/c/.env"),
        "path must be repo-relative and nested, got {entries:?}"
    );
    assert!(
        !entries.iter().any(|e| e.starts_with('/')),
        "no absolute paths in archive, got {entries:?}"
    );
}

#[test]
fn packs_matched_directory_recursively() {
    let repo = init_repo();
    write_magic(repo.path(), &["apps/api/config"]);
    write_file(repo.path(), "apps/api/config/a.toml", "a");
    write_file(repo.path(), "apps/api/config/sub/b.toml", "b");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(entries.contains("apps/api/config/a.toml"), "got {entries:?}");
    assert!(
        entries.contains("apps/api/config/sub/b.toml"),
        "got {entries:?}"
    );
}

/// A LEAF match under the tool's own `.superset/backups/` tree is never packed:
/// a broad glob that also matches a recovered secret copy must not archive it.
#[test]
fn excludes_backups_leaf_match_from_archive() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/.env"]);
    write_file(repo.path(), ".env", "REAL=1\n");
    // A recovered secret copy left under the backups tree by a reverse sync.
    write_file(
        repo.path(),
        ".superset/backups/20260101-000000/worktree/.env",
        "RECOVERED=1\n",
    );

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(entries.contains(".env"), "the real secret must pack: {entries:?}");
    assert!(
        !entries.iter().any(|e| e.contains(".superset/backups/")),
        "a backed-up secret copy must never be packed: {entries:?}"
    );
}

/// An ANCESTOR directory match (`.superset`) recurses into the tree but PRUNES
/// the `.superset/backups/` subtree — the regression guard for the pack blocker
/// where a bare `.superset` (or a broad `**`) pattern would walk the live tree
/// and archive recovered secrets.
#[test]
fn excludes_backups_subtree_from_ancestor_directory_match() {
    let repo = init_repo();
    write_magic(repo.path(), &[".superset"]);
    write_file(repo.path(), ".superset/config.json", "{}\n");
    write_file(
        repo.path(),
        ".superset/backups/20260101-000000/main/.env",
        "RECOVERED=1\n",
    );

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(
        entries.contains(".superset/config.json"),
        "non-backup .superset files still pack: {entries:?}"
    );
    assert!(
        !entries.iter().any(|e| e.contains(".superset/backups/")),
        "the backups subtree must be pruned from a directory match: {entries:?}"
    );
}

#[test]
fn add_events_emitted_per_entry() {
    let repo = init_repo();
    write_magic(repo.path(), &[".env", "config.toml"]);
    write_file(repo.path(), ".env", "x");
    write_file(repo.path(), "config.toml", "y");

    let mut events: Vec<PackEvent> = Vec::new();
    let code = pack_core(repo.path(), |e| events.push(e.clone())).unwrap();
    assert!(exit_ok(code));
    let adds = events
        .iter()
        .filter(|e| matches!(e, PackEvent::Add { .. }))
        .count();
    assert_eq!(adds, 2, "one Add event per entry, got {events:?}");
}

// ── Excludes inherited from match_paths ─────────────────────────────────

#[test]
fn excludes_node_modules_and_venv() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/.env"]);
    write_file(repo.path(), "apps/api/.env", "ok\n");
    write_file(repo.path(), "node_modules/pkg/.env", "drop\n");
    write_file(repo.path(), ".venv/lib/.env", "drop\n");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(entries.contains("apps/api/.env"), "got {entries:?}");
    assert!(
        !entries.iter().any(|e| e.contains("node_modules")),
        "node_modules must be excluded, got {entries:?}"
    );
    assert!(
        !entries.iter().any(|e| e.contains(".venv")),
        ".venv must be excluded, got {entries:?}"
    );
}

// ── Empty / error paths ─────────────────────────────────────────────────

#[test]
fn empty_files_writes_no_archive() {
    let repo = init_repo();
    write_magic(repo.path(), &[]);

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code), "empty files is success");
    assert!(
        !repo.path().join(archive_file_name(repo.path())).exists(),
        "no archive when files is empty"
    );
}

#[test]
fn no_matches_writes_no_archive() {
    let repo = init_repo();
    // Pattern is valid but nothing on disk matches.
    write_magic(repo.path(), &["**/.env"]);

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    assert!(
        !repo.path().join(archive_file_name(repo.path())).exists(),
        "no archive when nothing matches"
    );
}

#[test]
fn missing_magic_json_is_hard_error() {
    let repo = init_repo();
    // No magic.json written.
    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(!exit_ok(code), "must exit non-zero when magic.json absent");
}

#[test]
fn malformed_magic_json_is_hard_error() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join(".superset")).unwrap();
    fs::write(repo.path().join(".superset/magic.json"), "{bad json").unwrap();

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(!exit_ok(code), "must exit non-zero on malformed magic.json");
}

#[test]
fn outside_git_repo_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    // No git init.
    let code = pack_core(dir.path(), |_| {}).unwrap();
    assert!(!exit_ok(code), "must exit non-zero when not in a git repo");
}

// ── Symlink safety (must not dereference — mirrors apply.rs) ─────────────

/// A symlink inside a matched directory pointing OUTSIDE the repo must be
/// stored as a symlink entry, never dereferenced — otherwise the target's
/// bytes (e.g. a secret) leak into the archive.
#[cfg(unix)]
#[test]
fn symlink_in_matched_dir_is_not_dereferenced() {
    use std::os::unix::fs::symlink;
    let repo = init_repo();
    write_magic(repo.path(), &["bundle"]);
    // A secret living outside the repo root.
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    fs::write(&secret, "TOPSECRET\n").unwrap();
    // A matched directory containing a real file and a symlink to the secret.
    write_file(repo.path(), "bundle/real.txt", "ok\n");
    symlink(&secret, repo.path().join("bundle/leak")).unwrap();

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    // The real file is packed as a normal file...
    assert!(archive_entries(repo.path()).contains("bundle/real.txt"));
    // ...and the secret bytes must NOT appear anywhere as file content.
    let f = fs::File::open(repo.path().join(archive_file_name(repo.path()))).unwrap();
    let mut ar = tar::Archive::new(bzip2::read::BzDecoder::new(f));
    for entry in ar.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap().to_string_lossy() == "bundle/leak" {
            assert!(
                entry.header().entry_type().is_symlink(),
                "leak must be a symlink entry, not a dereferenced file"
            );
        }
        if entry.header().entry_type().is_file() {
            let mut s = String::new();
            entry.read_to_string(&mut s).unwrap();
            assert!(!s.contains("TOPSECRET"), "secret bytes leaked into archive");
        }
    }
}

/// A broken symlink inside a matched directory must not abort the pack.
#[cfg(unix)]
#[test]
fn broken_symlink_in_matched_dir_does_not_abort() {
    use std::os::unix::fs::symlink;
    let repo = init_repo();
    write_magic(repo.path(), &["bundle"]);
    write_file(repo.path(), "bundle/real.txt", "ok\n");
    symlink("does/not/exist", repo.path().join("bundle/dangling")).unwrap();

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code), "broken symlink must not abort the pack");
    assert!(archive_entries(repo.path()).contains("bundle/real.txt"));
}

/// A top-level matched entry that is a symlink to a directory must be
/// stored as a symlink entry, NOT followed — otherwise `append_dir_all`
/// would walk the link target's tree and pull files outside the repo in.
#[cfg(unix)]
#[test]
fn top_level_symlink_dir_is_not_followed() {
    use std::os::unix::fs::symlink;
    let repo = init_repo();
    // An outside directory whose files must NOT end up in the archive.
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("sub")).unwrap();
    fs::write(outside.path().join("sub/outside_secret.txt"), "LEAK\n").unwrap();
    // A top-level match that is a symlink pointing at that outside dir.
    symlink(outside.path(), repo.path().join("linkdir")).unwrap();
    write_magic(repo.path(), &["linkdir", ".env"]);
    write_file(repo.path(), ".env", "x");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(entries.contains(".env"), "real match still packed, got {entries:?}");
    assert!(
        !entries.iter().any(|e| e.contains("outside_secret.txt")),
        "symlink target tree must not be archived, got {entries:?}"
    );
    // `linkdir` itself is stored as a symlink entry, not a directory.
    let f = fs::File::open(repo.path().join(archive_file_name(repo.path()))).unwrap();
    let mut ar = tar::Archive::new(bzip2::read::BzDecoder::new(f));
    let mut saw_symlink = false;
    for entry in ar.entries().unwrap() {
        let entry = entry.unwrap();
        if entry.path().unwrap().to_string_lossy() == "linkdir" {
            assert!(
                entry.header().entry_type().is_symlink(),
                "linkdir must be a symlink entry, not a followed directory"
            );
            saw_symlink = true;
        }
    }
    assert!(saw_symlink, "linkdir must appear in the archive as a symlink");
}

// ── Empty-archive guard (no clobber when nothing was added) ─────────────

/// When every entry is skipped (here: a path that vanished after
/// expansion), write_archive must add nothing AND leave any existing
/// archive untouched — never replace a good backup with an empty tarball.
#[test]
fn write_archive_skips_vanished_entry_and_preserves_existing() {
    let repo = init_repo();
    let out = repo.path().join(archive_file_name(repo.path()));
    // A pre-existing "good backup" that must survive intact.
    fs::write(&out, b"GOODBACKUP").unwrap();
    // A rel that does not exist on disk (vanished between match and pack).
    let rels = vec![PathBuf::from("gone.txt")];

    let n = write_archive(repo.path(), &rels, &out, &mut |_| {}).unwrap();
    assert_eq!(n, 0, "a vanished entry adds nothing");
    assert_eq!(
        fs::read(&out).unwrap(),
        b"GOODBACKUP",
        "existing archive must be preserved, not clobbered by an empty one"
    );
}

// ── Repo-root (`.`) guard ───────────────────────────────────────────────

/// A `.` pattern resolves to the repo root; it must be dropped rather than
/// packing the whole tree (and the in-progress temp archive) into itself.
#[test]
fn dot_pattern_resolving_to_root_is_dropped() {
    let repo = init_repo();
    write_magic(repo.path(), &[".", ".env"]);
    write_file(repo.path(), ".env", "x");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    // The real match is still packed...
    assert!(entries.contains(".env"), "got {entries:?}");
    // ...but nothing from the `.` (whole-tree) walk: no .git, no README, and
    // crucially no temp-archive bytes recursed in.
    assert!(
        !entries.iter().any(|e| e.starts_with(".git/") || e == "README.md"),
        "`.` must not pack the whole tree, got {entries:?}"
    );
    assert!(
        !entries.contains(&archive_file_name(repo.path())),
        "archive must not contain itself, got {entries:?}"
    );
}

// ── Self-exclusion (KTD3) ───────────────────────────────────────────────

#[test]
fn does_not_pack_the_archive_into_itself() {
    let repo = init_repo();
    // A broad pattern that would otherwise match a stale archive at root.
    write_magic(repo.path(), &["**/*.bz2", ".env"]);
    write_file(repo.path(), ".env", "x");
    // Simulate a leftover archive from a prior run.
    fs::write(repo.path().join(archive_file_name(repo.path())), "stale").unwrap();

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(
        !entries.contains(&archive_file_name(repo.path())),
        "archive must not contain itself, got {entries:?}"
    );
    assert!(entries.contains(".env"), "real match still packed, got {entries:?}");
}

// ── Legacy-name exclusion ───────────────────────────────────────────────

/// A stale pre-0.3 `ss-magic-files.tar.bz2` at the root must not be swept
/// into the new (derived-name) archive by a broad pattern.
#[test]
fn does_not_pack_a_stale_legacy_archive() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/*.bz2", ".env"]);
    write_file(repo.path(), ".env", "x");
    fs::write(repo.path().join(LEGACY_PACK_FILE_NAME), "stale").unwrap();

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(
        !entries.contains(LEGACY_PACK_FILE_NAME),
        "legacy archive must be excluded, got {entries:?}"
    );
}

// ── Done event ──────────────────────────────────────────────────────────

/// A successful pack emits exactly one `Done` event carrying the archive
/// path and the entry count — the seam the rendering layer (summary line,
/// tar hint, clipboard copy) hangs off.
#[test]
fn done_event_carries_out_path_and_count() {
    let repo = init_repo();
    write_magic(repo.path(), &[".env", ".dev.vars"]);
    write_file(repo.path(), ".env", "a");
    write_file(repo.path(), ".dev.vars", "b");

    let mut done: Vec<(PathBuf, usize)> = Vec::new();
    let code = pack_core(repo.path(), |ev| {
        if let PackEvent::Done { out_path, count } = ev {
            done.push((out_path.clone(), *count));
        }
    })
    .unwrap();
    assert!(exit_ok(code));
    assert_eq!(done.len(), 1, "exactly one Done event");
    let (out_path, count) = &done[0];
    assert_eq!(*count, 2);
    assert!(out_path.is_file(), "Done.out_path must exist on disk");
    assert_eq!(
        out_path.file_name().unwrap().to_string_lossy(),
        archive_file_name(repo.path()),
    );
}

// ── Archive naming: origin normalization ────────────────────────────────

/// Every URL form of the same remote must normalize to the same stem
/// (example from the spec: github.com/ViktorStiskala/upx.cz).
#[test]
fn origin_forms_normalize_identically() {
    let expect = Some("viktorstiskala_upx-cz".to_string());
    for url in [
        "https://github.com/ViktorStiskala/upx.cz.git",
        "https://github.com/ViktorStiskala/upx.cz",
        "git@github.com:ViktorStiskala/upx.cz.git",
        "ssh://git@github.com/ViktorStiskala/upx.cz.git",
        "ssh://git@github.com:22/ViktorStiskala/upx.cz",
        "git://github.com/ViktorStiskala/upx.cz.git",
        "https://github.com/ViktorStiskala/upx.cz/",
    ] {
        assert_eq!(stem_from_origin(url), expect, "url: {url}");
    }
}

/// Nested paths (GitLab groups) keep every segment, joined with `_`.
#[test]
fn nested_group_origin_keeps_all_segments() {
    assert_eq!(
        stem_from_origin("https://gitlab.com/group/sub.group/repo.git"),
        Some("group_sub-group_repo".to_string())
    );
    assert_eq!(
        stem_from_origin("git@gitlab.com:group/sub/repo.git"),
        Some("group_sub_repo".to_string())
    );
}

/// A local-path origin contributes only its final segment; junk-only input
/// yields None.
#[test]
fn local_and_degenerate_origins() {
    assert_eq!(
        stem_from_origin("/srv/git/upx.cz.git"),
        Some("upx-cz".to_string())
    );
    assert_eq!(stem_from_origin(""), None);
    assert_eq!(stem_from_origin("https://github.com/"), None);
}

/// Segment sanitization: lowercase, non-alphanumeric runs collapse to one
/// `-`, edges trimmed, `_` reserved for the joiner.
#[test]
fn sanitize_segment_rules() {
    assert_eq!(sanitize_segment("ViktorStiskala"), "viktorstiskala");
    assert_eq!(sanitize_segment("upx.cz"), "upx-cz");
    assert_eq!(sanitize_segment("my_repo..name"), "my-repo-name");
    assert_eq!(sanitize_segment(".env"), "env");
    assert_eq!(sanitize_segment("---"), "");
}

/// With an origin configured, the archive name comes from the normalized
/// remote; without one it falls back to the primary worktree basename.
#[test]
fn archive_name_prefers_origin_and_falls_back_to_basename() {
    let repo = init_repo();
    let base = sanitize_segment(
        &repo.path().canonicalize().unwrap().file_name().unwrap().to_string_lossy(),
    );
    assert_eq!(
        archive_file_name(repo.path()),
        format!("ss-magic-{base}.tar.bz2"),
        "no origin -> primary worktree basename"
    );

    git_run(
        &["remote", "add", "origin", "git@github.com:ViktorStiskala/upx.cz.git"],
        repo.path(),
    );
    assert_eq!(
        archive_file_name(repo.path()),
        "ss-magic-viktorstiskala_upx-cz.tar.bz2",
        "origin present -> normalized remote"
    );
}

/// An archive left under a *previous* derived name (the origin changed
/// between packs) must not be swept into the new archive by a broad pattern
/// — same KTD3 class as the current-name and legacy-name guards.
#[test]
fn does_not_pack_an_archive_from_a_previous_origin() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/*.bz2", ".env"]);
    write_file(repo.path(), ".env", "x");
    git_run(
        &["remote", "add", "origin", "git@github.com:alice/old-name.git"],
        repo.path(),
    );
    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let old_archive = archive_file_name(repo.path());
    assert!(repo.path().join(&old_archive).is_file());

    // Origin changes; the next pack derives a different name.
    git_run(
        &["remote", "set-url", "origin", "git@github.com:bob/new-name.git"],
        repo.path(),
    );
    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let new_archive = archive_file_name(repo.path());
    assert_ne!(old_archive, new_archive);
    let entries = archive_entries(repo.path());
    assert!(
        !entries.contains(&old_archive),
        "previous-origin archive must be excluded, got {entries:?}"
    );
    assert!(entries.contains(".env"), "real match still packed");
}

/// Deeper ss-magic-*.tar.bz2 files are not pack outputs and stay packable;
/// only root-level ones are excluded.
#[test]
fn nested_ss_magic_archives_are_still_packable() {
    let repo = init_repo();
    write_magic(repo.path(), &["**/*.bz2"]);
    write_file(repo.path(), "backups/ss-magic-old.tar.bz2", "nested");

    let code = pack_core(repo.path(), |_| {}).unwrap();
    assert!(exit_ok(code));
    let entries = archive_entries(repo.path());
    assert!(
        entries.contains("backups/ss-magic-old.tar.bz2"),
        "nested archive is user data, got {entries:?}"
    );
}

/// `file://` origins are local paths: only the final segment names the repo,
/// identical to the equivalent bare path — local directory hierarchy must
/// never leak into the archive name.
#[test]
fn file_scheme_origin_uses_only_the_final_segment() {
    assert_eq!(
        stem_from_origin("file:///srv/git/upx.cz.git"),
        Some("upx-cz".to_string())
    );
    assert_eq!(
        stem_from_origin("file:///srv/git/upx.cz.git"),
        stem_from_origin("/srv/git/upx.cz.git"),
    );
    assert_eq!(
        stem_from_origin("file:///Users/alice/client/repo.git"),
        Some("repo".to_string()),
        "local hierarchy must not leak into the name"
    );
}
