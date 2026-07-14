//! The pack engine: expand the sync patterns from the overlaid `magic.json`,
//! collect every matching file/directory from the current git repo root, and
//! write them — preserving repo-relative structure — into a single
//! `ss-magic-files.tar.bz2` at the git root.
//!
//! This reuses the existing seams verbatim:
//! - [`git::cwd_repo_root`] resolves the repo root (config source, match target,
//!   and archive destination are all this one root — see the plan's KTD1).
//! - [`superset_files::load_overlaid`] reads `magic.json` + `magic.local.json`.
//! - [`apply::match_paths`] expands the patterns with the same syntax checks,
//!   `DEFAULT_EXCLUDES`, and de-dupe that forward/reverse sync use.
//!
//! Compression is pure-Rust (bzip2 via `libbz2-rs-sys`), so no system libbz2 /
//! C toolchain is needed — consistent with the crate's hermetic-build posture.
//!
//! The control flow deliberately mirrors `main::sync_core`: resolve root →
//! probe `magic.json` → load overlaid config → empty-guard → do work.

use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use bzip2::write::BzEncoder;
use bzip2::Compression;
use tempfile::NamedTempFile;

use crate::sync::apply;
use crate::git;
use crate::tui::style;

/// Name of the archive written at the git root.
pub const PACK_FILE_NAME: &str = "ss-magic-files.tar.bz2";

/// Per-entry event emitted while building the archive. Callers convert these to
/// user-facing output (`main.rs`) or assertions (tests) — same closure-driven
/// design as [`apply::Event`].
#[derive(Debug, Clone)]
pub enum PackEvent {
    /// A file or directory was added to the archive at `rel`.
    Add { rel: PathBuf },
}

/// Core pack flow, shared by `ss-magic pack` and the interactive menu.
///
/// Resolves the current repo root, verifies `.superset/magic.json` exists there,
/// loads the overlaid config, expands the patterns against that root, and writes
/// the matched files into `<root>/ss-magic-files.tar.bz2` with repo-relative
/// paths. Extracted as `pack_core` (taking an `on_event` closure) so tests can
/// collect events without side effects on stdout.
///
/// Hard errors (non-zero exit), paralleling `sync_core`:
/// - Cannot resolve the git repo root (not in a repo, or git fails).
/// - `.superset/magic.json` absent in the resolved root.
/// - Malformed `magic.json` or `magic.local.json`.
pub fn pack_core<F>(cwd: &Path, mut on_event: F) -> Result<ExitCode>
where
    F: FnMut(&PackEvent),
{
    // 1. Resolve the current repo root — config source, match target, and
    //    archive destination are all this one root (KTD1).
    let root = match git::cwd_repo_root(cwd) {
        Ok(r) => r,
        Err(err) => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: cannot resolve git repo root from {}: {err:#}",
                    cwd.display()
                ))
            );
            return Ok(ExitCode::from(1));
        }
    };

    // 2-3. Probe + load the overlaid magic.json (hard error on absent/malformed).
    //       Shared with `sync_core` via `crate::load_magic_or_exit`.
    let cfg = match crate::load_magic_or_exit(&root) {
        Ok(c) => c,
        Err(code) => return Ok(code),
    };

    // 4. Empty files list → nothing to do, success.
    if cfg.files.is_empty() {
        println!(
            "{}",
            style::info("magic.json `files` is empty — nothing to pack.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // 5. Expand patterns to matched relative paths (same semantics as sync).
    let mut rels = match apply::match_paths(&root, &cfg.files) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
    };

    // 6. Never pack the output archive into itself (KTD3): drop a match equal
    //    to the archive name at the repo root, so a broad user pattern like
    //    `*.bz2` can't recurse the archive. Also drop any match that resolves to
    //    the repo root itself (a `.` pattern): `append_dir_all(".", root)` would
    //    walk the whole tree live — including the in-progress temp archive and
    //    `.git` — corrupting the archive and bypassing the self-exclusion guard.
    rels.retain(|r| r != Path::new(PACK_FILE_NAME) && !is_repo_root_rel(r));

    // 7. Nothing left to archive after filtering → success, no archive written.
    if rels.is_empty() {
        println!(
            "{}",
            style::info("No files matched the config patterns — nothing to pack.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // 8. Build the archive at <root>/ss-magic-files.tar.bz2.
    let out_path = root.join(PACK_FILE_NAME);
    let count = match write_archive(&root, &rels, &out_path, &mut on_event) {
        Ok(n) => n,
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            return Ok(ExitCode::from(1));
        }
    };

    // Every match was a special/vanished entry — write_archive wrote nothing
    // and left any existing archive untouched. Don't claim "Packed 0 entries".
    if count == 0 {
        println!(
            "{}",
            style::info("No packable files remained — nothing to pack.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!();
    println!(
        "{}",
        style::ok(format!(
            "Packed {count} entries → {}",
            out_path.display()
        ))
    );
    Ok(ExitCode::SUCCESS)
}

/// Whether a relative match resolves to the repo root itself — i.e. every
/// component is a current-dir marker (`.`, `./`) or the path is empty. Such a
/// match (from a `.` pattern) must be dropped before archiving; see step 6.
fn is_repo_root_rel(rel: &Path) -> bool {
    rel.as_os_str().is_empty() || rel.components().all(|c| matches!(c, Component::CurDir))
}

/// Tar + bzip2-compress `rels` (repo-relative) from `root` into `out_path`,
/// writing to a temp file in `root` first and atomically renaming on success
/// (KTD3). Real directories are added recursively; a matched symlink is stored
/// as a single symlink entry (never followed); special files (sockets/fifos)
/// and entries that vanished after expansion are skipped. Returns the number of
/// entries added. When nothing was added (every match was special/vanished),
/// the temp file is discarded and `out_path` is left untouched — see the
/// `count == 0` guard — so a prior good archive is never replaced by an empty
/// one, and the caller gets `0`.
fn write_archive<F>(
    root: &Path,
    rels: &[PathBuf],
    out_path: &Path,
    on_event: &mut F,
) -> Result<usize>
where
    F: FnMut(&PackEvent),
{
    let tmp = NamedTempFile::new_in(root)
        .with_context(|| format!("creating temp archive in {}", root.display()))?;

    let mut count = 0usize;
    {
        // Default level (6) is the crate's documented speed/size balance —
        // right for a quick ad-hoc snapshot over what are usually small config
        // files, rather than paying level-9 CPU for marginal size gains.
        let enc = BzEncoder::new(tmp.as_file(), Compression::default());
        let mut builder = tar::Builder::new(enc);
        // Store symlinks as symlink entries instead of dereferencing them. The
        // default (`follow_symlinks(true)`) would embed the *target's* bytes —
        // a secret-leak vector if a matched dir holds a link to something like
        // `~/.aws/credentials`, and a hard abort on a broken link. This matches
        // `apply.rs`, which never follows symlinks out of the source tree.
        builder.follow_symlinks(false);

        for rel in rels {
            let abs = root.join(rel);
            // Classify by `symlink_metadata` (lstat — does NOT follow the
            // link). A matched entry that is itself a symlink is stored as a
            // single symlink entry and never followed: `Path::is_dir()` would
            // follow it, so a symlink to a directory would make
            // `append_dir_all` walk (and archive) the link's target tree,
            // pulling files outside the repo. `follow_symlinks(false)` only
            // governs symlinks encountered *during* a directory walk, not one
            // used as the walk root — this is the top-level analogue.
            let file_type = match std::fs::symlink_metadata(&abs) {
                Ok(m) => m.file_type(),
                // Vanished between expansion and packing — skip.
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                builder
                    .append_path_with_name(&abs, rel)
                    .with_context(|| format!("adding symlink {} to archive", rel.display()))?;
            } else if file_type.is_dir() {
                builder
                    .append_dir_all(rel, &abs)
                    .with_context(|| format!("adding directory {} to archive", rel.display()))?;
            } else if file_type.is_file() {
                builder
                    .append_path_with_name(&abs, rel)
                    .with_context(|| format!("adding file {} to archive", rel.display()))?;
            } else {
                // Socket / fifo / other special file — skip.
                continue;
            }
            on_event(&PackEvent::Add { rel: rel.clone() });
            count += 1;
        }

        // Finalize both layers: tar footer, then flush the bzip2 stream.
        let enc = builder.into_inner().context("finalizing tar stream")?;
        enc.finish().context("finalizing bzip2 stream")?;
    }

    // Nothing was actually archived (every match was a special/vanished
    // entry): drop the temp file and leave any existing archive untouched,
    // rather than replacing a prior good backup with an empty tarball.
    if count == 0 {
        return Ok(0);
    }

    // Ensure bytes hit disk before the rename, then persist atomically.
    tmp.as_file().sync_all().ok();
    tmp.persist(out_path)
        .with_context(|| format!("persisting archive to {}", out_path.display()))?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::superset_files;
    use crate::test_support::git_run;
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::Read;
    use std::process::ExitCode;
    use tempfile::TempDir;

    fn exit_ok(code: ExitCode) -> bool {
        code == ExitCode::SUCCESS
    }

    /// Init a git repo with one commit so `cwd_repo_root` resolves.
    fn init_repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_run(&["init", "-q", "-b", "main"], dir.path());
        crate::test_support::neutralize_global_excludes(dir.path());
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

    /// Decompress `<root>/ss-magic-files.tar.bz2` and return the set of file
    /// entry paths (directories are represented by their contained files).
    fn archive_entries(root: &Path) -> BTreeSet<String> {
        let f = fs::File::open(root.join(PACK_FILE_NAME)).unwrap();
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
        let f = fs::File::open(root.join(PACK_FILE_NAME)).unwrap();
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
            repo.path().join(PACK_FILE_NAME).is_file(),
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

    #[test]
    fn add_events_emitted_per_entry() {
        let repo = init_repo();
        write_magic(repo.path(), &[".env", "config.toml"]);
        write_file(repo.path(), ".env", "x");
        write_file(repo.path(), "config.toml", "y");

        let mut events: Vec<PackEvent> = Vec::new();
        let code = pack_core(repo.path(), |e| events.push(e.clone())).unwrap();
        assert!(exit_ok(code));
        assert_eq!(events.len(), 2, "one Add event per entry, got {events:?}");
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
            !repo.path().join(PACK_FILE_NAME).exists(),
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
            !repo.path().join(PACK_FILE_NAME).exists(),
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
        let f = fs::File::open(repo.path().join(PACK_FILE_NAME)).unwrap();
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
        let f = fs::File::open(repo.path().join(PACK_FILE_NAME)).unwrap();
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
        let out = repo.path().join(PACK_FILE_NAME);
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
            !entries.contains(PACK_FILE_NAME),
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
        fs::write(repo.path().join(PACK_FILE_NAME), "stale").unwrap();

        let code = pack_core(repo.path(), |_| {}).unwrap();
        assert!(exit_ok(code));
        let entries = archive_entries(repo.path());
        assert!(
            !entries.contains(PACK_FILE_NAME),
            "archive must not contain itself, got {entries:?}"
        );
        assert!(entries.contains(".env"), "real match still packed, got {entries:?}");
    }
}
