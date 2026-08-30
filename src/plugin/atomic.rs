//! The one atomic-write helper for every plugin module that must replace a
//! file's contents without a reader ever seeing a half-written one.
//!
//! The shape is always the same: create a temp file in the target's own
//! directory (so the final rename is same-filesystem and therefore atomic),
//! write the body, flush it, optionally chmod it, optionally fsync it, then
//! rename it over the target. A crash mid-write leaves the previous file
//! untouched instead of a truncated one.
//!
//! This used to live in `plugin::heartbeat` under a narrower signature (a
//! fixed `.jsonl` suffix and a hardcoded owner-only mode) and was shared with
//! `plugin::ledger`. Six more modules — the conclusion cache, bypass claims,
//! artifact expectations, the scratchpad session pointer, the CI workflow
//! writer, and the checklist document/pointer writers — had each hand-rolled
//! the identical `tempfile::Builder` -> `write_all` -> `flush` -> chmod ->
//! `persist` sequence with their own suffix/mode/sync choices. This is the
//! one copy all of them now call, generalized to the union of what they
//! needed; nothing here changes what any of those call sites actually did.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use anyhow::{Context, Result};

/// Replace `path`'s contents with `body` via a temp file created in the same
/// directory, then renamed over `path`.
///
/// - `prefix`/`suffix` name the temp file, which only ever matters when
///   somebody is looking at the directory mid-write. `suffix` may be empty.
/// - `what`, when `Some`, names the thing being written for the "writing"/
///   "flushing" error context (e.g. `Some("the bypass claim")` reads as
///   "writing the bypass claim"); `None` falls back to `path`'s own display,
///   which is what a caller with no better noun for it used before this was
///   consolidated.
/// - `mode`, when `Some`, chmods the temp file to that mode before the
///   rename, so the replaced file ends up with exactly the permissions the
///   caller asked for rather than `tempfile`'s default. `None` leaves the
///   temp file's mode alone — for a caller that does not manage permissions
///   here (either it does not care, or, like the checklist document, it
///   means to preserve whatever mode the existing file already had, and computes
///   that mode itself before calling in). A chmod failure names the mode that
///   was asked for, because that is what says whether the failure matters:
///   most callers here pass owner-only `0600` for a file under the state tree,
///   and a file that misses it is readable by everyone on the machine.
/// - `sync`, when true, fsyncs the temp file's contents to disk before the
///   rename (best-effort — a sync failure is ignored, same as every call site
///   already did), for a caller where surviving a crash right after the write
///   matters more than write latency.
pub(crate) fn write_atomically(
    path: &Path,
    body: &str,
    prefix: &str,
    suffix: &str,
    what: Option<&str>,
    mode: Option<u32>,
    sync: bool,
) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let label = what.map(str::to_string).unwrap_or_else(|| path.display().to_string());

    let mut tmp = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile_in(dir)
        .with_context(|| format!("creating a temp file in {}", dir.display()))?;
    tmp.write_all(body.as_bytes())
        .with_context(|| format!("writing {label}"))?;
    tmp.flush().with_context(|| format!("flushing {label}"))?;
    if let Some(mode) = mode {
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))
            // Naming the mode is what the four owner-only call sites said in
            // their own wording before this was consolidated ("setting
            // owner-only mode on …"): a chmod failure on `0600` means the
            // secret in the file may be world-readable, which is worth seeing
            // in the error. Spelling the octal rather than the word keeps one
            // message that also reads correctly for a caller passing an
            // explicit, deliberately non-restrictive mode.
            .with_context(|| format!("setting mode {mode:04o} on {}", path.display()))?;
    }
    if sync {
        tmp.as_file().sync_all().ok();
    }
    tmp.persist(path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
