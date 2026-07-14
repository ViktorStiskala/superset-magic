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
mod tests;
