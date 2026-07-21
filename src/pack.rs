//! The pack engine: expand the sync patterns from the overlaid `magic.json`,
//! collect every matching file/directory from the current git repo root, and
//! write them — preserving repo-relative structure — into a single
//! `ss-magic-<repo>.tar.bz2` at the git root (name derived by
//! [`archive_file_name`]).
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

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{ExitCode, Stdio};

use anyhow::{Context, Result};
use bzip2::write::BzEncoder;
use bzip2::Compression;
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use crate::sync::apply;
use crate::git;
use crate::tui::style;

/// Archive file name for `root`: `ss-magic-<stem>.tar.bz2`.
///
/// The stem is derived from the `origin` remote when one is configured
/// (normalized so every URL form of the same repo yields the same name), and
/// falls back to the primary (main) worktree directory's basename otherwise —
/// e.g. `ss-magic-viktorstiskala_upx-cz.tar.bz2` for any origin form of
/// `github.com/ViktorStiskala/upx.cz`, or `ss-magic-upx-cz.tar.bz2` for an
/// origin-less checkout at `.../upx.cz`. A last-resort `files` stem preserves
/// the legacy name shape when neither source yields usable characters.
pub fn archive_file_name(root: &Path) -> String {
    let stem = git::origin_url(root)
        .ok()
        .flatten()
        .and_then(|url| stem_from_origin(&url))
        .or_else(|| {
            let main_root = git::main_checkout_root(root).unwrap_or_else(|_| root.to_path_buf());
            main_root
                .file_name()
                .map(|n| sanitize_segment(&n.to_string_lossy()))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "files".to_string());
    format!("ss-magic-{stem}.tar.bz2")
}

/// Normalize a git remote URL into a filename stem.
///
/// Strips the scheme (`https://`, `ssh://`, `git://`, …), userinfo, host, and
/// port; drops a trailing `.git`; lowercases; maps every non-alphanumeric run
/// inside a path segment to a single `-`; joins path segments with `_`. The
/// scp-like form (`git@host:owner/repo`) and a scheme form of the same remote
/// produce identical stems, and nested paths (GitLab groups) keep every
/// segment: `gitlab.com/group/sub/repo` → `group_sub_repo`. A local-path
/// origin — bare (`/srv/git/repo.git`) or `file://` — contributes only its
/// final segment, so local directory hierarchies never leak into the archive
/// name. Returns `None` when nothing usable remains.
fn stem_from_origin(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    // Split off the scheme, then the host: with a scheme the host ends at the
    // first `/`; without one, the scp-like `[user@]host:path` uses the first
    // `:`. A `file://` or bare local path has no host — only the basename
    // identifies the repo (the rest is local filesystem hierarchy).
    let path = if let Some(rest) = url.strip_prefix("file://") {
        final_segment(rest)
    } else if let Some(i) = url.find("://") {
        url[i + 3..].split_once('/')?.1
    } else if let Some((_host, path)) = url.split_once(':') {
        path
    } else {
        final_segment(url)
    };
    let trimmed = path.trim_matches('/');
    let path = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    let segments: Vec<String> = path
        .split('/')
        .map(sanitize_segment)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("_"))
}

/// Final `/`-separated segment of a path-like string (the whole string when
/// it contains no `/`).
fn final_segment(s: &str) -> &str {
    s.rfind('/').map_or(s, |i| &s[i + 1..])
}

/// Lowercase `seg` and collapse every run of non-alphanumeric characters
/// (dots, spaces, unicode, …) into a single `-`, trimming the edges — so a
/// repo named `upx.cz` becomes `upx-cz`. `_` is reserved as the segment
/// joiner, so it too maps to `-` here.
fn sanitize_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    let mut pending_dash = false;
    for ch in seg.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/// Copy `text` to the system clipboard by piping it to the first available
/// clipboard tool (`pbcopy` on macOS; `wl-copy`/`xclip`/`xsel` on Linux).
/// Returns whether a tool accepted the text. Deliberately *not* called from
/// `pack_core` — it is a rendering-layer side effect (`main.rs`), so the pure
/// engine and its tests never touch the user's clipboard.
pub fn copy_to_clipboard(text: &str) -> bool {
    const TOOLS: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (tool, args) in TOOLS {
        let child = std::process::Command::new(tool)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { continue }; // tool not installed
        let Some(mut stdin) = child.stdin.take() else {
            let _ = child.kill();
            continue;
        };
        let wrote = stdin.write_all(text.as_bytes()).is_ok();
        drop(stdin); // close the pipe so the tool can exit
        match child.wait() {
            Ok(status) if wrote && status.success() => return true,
            _ => continue,
        }
    }
    false
}

/// Per-entry event emitted while building the archive. Callers convert these to
/// user-facing output (`main.rs`) or assertions (tests) — same closure-driven
/// design as [`apply::Event`].
#[derive(Debug, Clone)]
pub enum PackEvent {
    /// A file or directory was added to the archive at `rel`.
    Add { rel: PathBuf },
    /// The archive was written and persisted: `count` entries at `out_path`.
    /// The rendering layer owns the summary line, the `tar` extraction hint,
    /// and the clipboard copy of the archive's real path.
    Done { out_path: PathBuf, count: usize },
}

/// Core pack flow, shared by `ss-magic pack` and the interactive menu.
///
/// Resolves the current repo root, verifies `.superset/magic.json` exists there,
/// loads the overlaid config, expands the patterns against that root, and writes
/// the matched files into `<root>/ss-magic-<repo>.tar.bz2` (see [`archive_file_name`]) with repo-relative
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

    // 6. Never pack a pack archive into itself (KTD3): drop every root-level
    //    match shaped `ss-magic-*.tar.bz2`, so a broad user pattern like
    //    `*.bz2` can't recurse the output — covering the current derived name,
    //    the pre-0.3 fixed `ss-magic-files.tar.bz2`, AND archives left under a
    //    *previous* derived name (the origin remote can change between packs;
    //    a stale snapshot of secrets must never nest into a new archive). Also
    //    drop any match that resolves to the repo root itself (a `.` pattern):
    //    `append_dir_all(".", root)` would walk the whole tree live — including
    //    the in-progress temp archive and `.git` — corrupting the archive and
    //    bypassing the self-exclusion guard. And drop any LEAF match under the
    //    tool's own `.superset/backups/` tree so a recovered secret copy is
    //    never packed. (An ancestor DIRECTORY match — e.g. a bare `.superset`
    //    pattern — is handled separately in `write_archive`, whose directory
    //    walk prunes the backups subtree; `under_backups_dir` needs both the
    //    `.superset` and `backups` components, so it cannot catch the ancestor.)
    let file_name = archive_file_name(&root);
    rels.retain(|r| {
        !is_pack_archive_rel(r)
            && !is_repo_root_rel(r)
            && !crate::sync::reverse_sync::under_backups_dir(r)
    });

    // 7. Nothing left to archive after filtering → success, no archive written.
    if rels.is_empty() {
        println!(
            "{}",
            style::info("No files matched the config patterns — nothing to pack.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // 8. Build the archive at <root>/<derived name> (see `archive_file_name`).
    let out_path = root.join(&file_name);
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

    // Summary rendering (count line, tar hint, clipboard copy) is the
    // caller's job — tests collect this event without side effects.
    on_event(&PackEvent::Done { out_path, count });
    Ok(ExitCode::SUCCESS)
}

/// Whether a relative match is a pack archive at the repo root — any
/// root-level `ss-magic-*.tar.bz2`, which covers the current derived name,
/// the legacy fixed name, and archives produced under a previous derived name
/// (origin remotes change). Deeper matches (`sub/ss-magic-x.tar.bz2`) are not
/// pack outputs and stay packable.
fn is_pack_archive_rel(rel: &Path) -> bool {
    let root_level = rel
        .parent()
        .is_none_or(|p| p.as_os_str().is_empty());
    root_level
        && rel
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("ss-magic-") && n.ends_with(".tar.bz2"))
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
                // NOT a blind `append_dir_all`: a directory match that is an
                // ANCESTOR of `.superset/backups` (a literal `.superset`
                // pattern, or a broad glob like `**` that matches the bare
                // `.superset` component) would otherwise walk the live tree and
                // pack every recovered secret under `.superset/backups/…`. The
                // guarded walk prunes that subtree no matter how the dir match
                // reached `rels` — the flat `under_backups_dir` retain filter
                // (step 6) only catches leaf matches, not ancestor dirs.
                append_dir_excluding_backups(&mut builder, root, rel, &abs)
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

/// Recursively add the directory match `rel` (rooted at `abs`) to `builder`,
/// EXCLUDING any descendant under the tool's own `.superset/backups/` tree so a
/// recovered secret is never packed. Mirrors `append_dir_all`'s recursive walk
/// but prunes the backups subtree (keyed on each entry's `root`-relative path,
/// via [`crate::sync::reverse_sync::under_backups_dir`]) and never follows
/// symlinks (a symlink is stored as a single symlink entry, matching the
/// top-level classification and `apply.rs`). Entry names are `root`-relative so
/// the archive keeps the same layout `append_dir_all` produced.
fn append_dir_excluding_backups<W: Write>(
    builder: &mut tar::Builder<W>,
    root: &Path,
    rel: &Path,
    abs: &Path,
) -> Result<()> {
    let walker = WalkDir::new(abs)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| match e.path().strip_prefix(root) {
            // Prune the backups subtree wherever it appears under the match.
            Ok(r) => !crate::sync::reverse_sync::under_backups_dir(r),
            Err(_) => true,
        });
    for entry in walker {
        let entry = entry.with_context(|| format!("walking {} for the archive", abs.display()))?;
        let path = entry.path();
        // Archive under the entry's repo-relative name (falls back to `rel` for
        // the walk root, which strips to exactly `rel`).
        let name = path.strip_prefix(root).unwrap_or(rel);
        let ft = entry.file_type();
        if ft.is_dir() {
            builder
                .append_dir(name, path)
                .with_context(|| format!("adding dir {} to archive", name.display()))?;
        } else if ft.is_symlink() || ft.is_file() {
            builder
                .append_path_with_name(path, name)
                .with_context(|| format!("adding {} to archive", name.display()))?;
        }
        // Special files (socket / fifo) are skipped, as at the top level.
    }
    Ok(())
}

#[cfg(test)]
mod tests;
