//! The plugin's private per-machine temporary root (R80) and the fd-lock
//! helper built on it (R81, AE61, KTD5).
//!
//! ## Why a temporary root at all
//!
//! Some coordination is inherently pre-repository: two Superset sessions on
//! the same machine can both start before the plugin binary is even
//! installed (AE61), and hook handlers on one event fire concurrently
//! against the same original tool input with no ordering guarantee (R81).
//! Neither has a worktree-scoped place to meet — the scratchpad tree
//! (`scratchpad.rs`) only exists once a repository and a session identity
//! are already resolved. R80 answers this with one fixed, machine-wide
//! location: `/tmp/ss-magic-plugin/<identifier>/`, falling back to
//! `$TMPDIR/ss-magic-plugin/<identifier>/` when `/tmp` cannot host a
//! writable private root (a sandbox that allowlists only `$TMPDIR`, for
//! instance).
//!
//! ## The identifier is a cross-language contract
//!
//! `<identifier>` is the first 16 hex characters of the SHA-256 of `$HOME` —
//! stable for one user on one machine, and derived the same way by two
//! independent implementations: this module in Rust, and the plugin's
//! `bootstrap.sh` in shell (`printf %s "$HOME" | shasum -a 256`). Neither
//! side may special-case an unset or empty `$HOME`: both read whatever the
//! environment hands them (possibly the empty string) and hash exactly that,
//! so the two sides agree on the resulting path without needing to agree on
//! how to detect "unset" first. [`crate::hashing::sha256`] carries the
//! from-scratch SHA-256 this relies on, and the reasoning for hand-rolling it
//! rather than reusing FNV-1a or adding a crate.
//!
//! ## A predictable path is not evidence of ownership
//!
//! `/tmp/ss-magic-plugin/<identifier>` is a guessable path on a shared
//! machine — another user, or something malicious, could have created it
//! first. So the resolved root is never trusted on the strength of merely
//! existing: [`resolve_root`] validates every component IT manages (the
//! `ss-magic-plugin` namespace directory and the `<identifier>` directory
//! beneath it — not `/tmp`/`$TMPDIR` themselves, which are system-owned and
//! outside anyone's control) by `lstat`ing each one as a real directory
//! (never a symlink), owned by this process's effective uid, mode exactly
//! `0700`. A component that fails validation makes the whole base
//! unusable — `/tmp` falls through to the `$TMPDIR` base, and if that also
//! fails, [`resolve_root`] returns `Err` rather than writing into a root
//! someone else controls. Nothing durable is ever kept here; a validation
//! failure just means the caller gets no coordination point this run.
//!
//! ## The lock helper
//!
//! [`with_lock`] and [`try_with_lock`] are the fd-lock pattern KTD5 already
//! established in `update/apply.rs`, applied to a file inside the validated
//! root: one lock file per protected concern, opened for read+write and
//! locked via `fd_lock::RwLock`. They differ from `update/apply.rs`'s update
//! lock in the one way R81 requires — that lock SKIPS on contention (an
//! update is optional, so a busy lock just means "don't bother this run"),
//! while [`with_lock`] BLOCKS until the other holder releases, because R81's
//! whole point is that concurrent handlers coordinate with each other rather
//! than one silently skipping work the other assumed would happen.
//! [`try_with_lock`] is the non-blocking sibling for a caller that, like the
//! update lock, would rather skip than wait.
//!
//! A lock taken this way needs no staleness reclaim of its own: `flock` (the
//! primitive `fd-lock` uses on Unix) is released by the kernel the instant
//! every file descriptor referencing it closes, including an abrupt process
//! death — the same guarantee `update/apply.rs`'s stale-lock TTL comment
//! documents as making a truly stuck lock "structurally impossible" even
//! without that module's own belt-and-suspenders mtime reclaim. Nothing here
//! reclaims a lock file by mtime for the same reason `apply_update`'s core
//! critical section doesn't need to: there is no way to hold `flock` and
//! also be unreachable.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use crate::hashing;

/// The namespace directory both this module and `bootstrap.sh` (U23) create
/// directly under a machine temp base. Fixed cross-language contract text —
/// see the module docs' "identifier is a cross-language contract" section.
// Not yet referenced outside this module and its tests: U23's bootstrap.sh
// matches this contract in shell rather than calling into Rust. The lock
// helpers below DO have an in-crate caller — `plugin::heartbeat` locks its
// own store with `with_lock` — but nothing outside this module needs the
// namespace name itself yet.
pub const NAMESPACE_DIR: &str = "ss-magic-plugin";

/// The lock file name for the one coordination point both implementations
/// share today: two sessions racing to install the pinned binary when
/// neither finds it yet (AE61). Lives directly inside [`resolve_root`]'s
/// result. `bootstrap.sh` locks this exact same relative path in shell; a
/// change here without a matching change there breaks that coordination.
#[allow(dead_code)]
pub const INSTALL_LOCK_NAME: &str = "install.lock";

/// Owner-only mode every component [`resolve_root`] manages must have, both
/// when created and when validated (R80).
const DIR_MODE: u32 = 0o700;

/// How many leading hex characters of the SHA-256 digest make up the
/// identifier (R80).
const IDENTIFIER_HEX_LEN: usize = 16;

/// [`resolve_root`] found no base — neither `/tmp` nor the `$TMPDIR`
/// fallback — that validates as a private, owner-only root. The caller must
/// refuse whatever it wanted the root for (R80: "the caller refuses ...
/// rather than writing into a root someone else controls"); for the
/// bootstrap that means exiting 0 anyway per R72, just without the
/// coordination this root would have offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoValidRoot;

impl std::fmt::Display for NoValidRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "neither /tmp nor the $TMPDIR fallback has a private, owner-only \
             {NAMESPACE_DIR} root that validates; refusing to use a temporary root"
        )
    }
}

impl std::error::Error for NoValidRoot {}

/// R80's per-machine identifier: the first 16 hex characters of the SHA-256
/// of `home` (exactly as it arrives — an unset `$HOME` and an empty `$HOME`
/// are indistinguishable here on purpose, matching `bootstrap.sh`'s shell
/// side, which cannot tell the difference either).
pub fn identifier(home: &str) -> String {
    let hex = hashing::sha256_hex(home.as_bytes());
    hex[..IDENTIFIER_HEX_LEN].to_string()
}

/// Resolve and validate the private per-machine temporary root for this
/// user, creating it (and its `ss-magic-plugin` namespace parent) if
/// missing.
///
/// Reads `$HOME` and `$TMPDIR` (via [`std::env::temp_dir`], which already
/// falls back to `/tmp` when `$TMPDIR` is unset) from the real environment
/// and this process's real effective uid. See [`resolve_root_at`] for the
/// injectable core this delegates to.
pub fn resolve_root() -> Result<PathBuf, NoValidRoot> {
    let home = std::env::var("HOME").unwrap_or_default();
    resolve_root_at(
        &home,
        Path::new("/tmp"),
        &std::env::temp_dir(),
        effective_uid(),
    )
}

/// The testable core of [`resolve_root`]: every environment fact it would
/// otherwise read is a parameter, so tests point `primary_base`/
/// `fallback_base` at tempdirs and inject `euid` instead of depending on the
/// developer's real `/tmp`, `$HOME`, or uid.
///
/// Tries `primary_base/ss-magic-plugin/<identifier>` first; if that base
/// fails to produce a validated root, tries the same shape under
/// `fallback_base`. `Err` only when neither does.
fn resolve_root_at(
    home: &str,
    primary_base: &Path,
    fallback_base: &Path,
    euid: u32,
) -> Result<PathBuf, NoValidRoot> {
    let id = identifier(home);
    ensure_and_validate(primary_base, &id, euid)
        .or_else(|| ensure_and_validate(fallback_base, &id, euid))
        .ok_or(NoValidRoot)
}

/// Ensure `base/ss-magic-plugin/<id>` exists and validate both it and its
/// `ss-magic-plugin` parent, in that order (the parent first, so a bad
/// parent is rejected before anything is created beneath it). `None` on any
/// failure — of either component, at either the create or the validate
/// step; the caller falls through to the next base rather than inspecting
/// why.
fn ensure_and_validate(base: &Path, id: &str, euid: u32) -> Option<PathBuf> {
    let namespace = base.join(NAMESPACE_DIR);
    ensure_dir(&namespace);
    if !validate_component(&namespace, euid) {
        return None;
    }

    let root = namespace.join(id);
    ensure_dir(&root);
    if !validate_component(&root, euid) {
        return None;
    }

    Some(root)
}

/// Best-effort, single-level `mkdir` at owner-only mode. Whatever the
/// outcome — created, already existed (as a directory, a symlink, or
/// anything else), or failed outright because `base` itself is read-only —
/// [`validate_component`] is what actually decides whether `path` is usable;
/// this step only tries to make the common case (nothing there yet) exist.
fn ensure_dir(path: &Path) {
    let _ = DirBuilder::new().mode(DIR_MODE).create(path);
}

/// R80's validation: `path` must `lstat` as a real directory (a symlink —
/// even one pointing at a perfectly good directory — fails this), owned by
/// `euid`, at exactly mode `0700`. Any of the three failing, including the
/// path not existing at all, is "not usable" — there is no partial credit.
fn validate_component(path: &Path, euid: u32) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    meta.is_dir() && meta.uid() == euid && (meta.mode() & 0o777) == DIR_MODE
}

/// This process's effective uid, via a direct call to the platform's own
/// `geteuid`. Not shelled out to `id -u`: this sits on the validation path
/// for every lock attempt, and a subprocess there is both slower and one
/// more thing that can fail (`id` missing or behaving unexpectedly) for a
/// fact the process already knows about itself. `geteuid` takes no
/// arguments, cannot fail, and has no side effects, and every platform this
/// binary targets (Linux, macOS) already links the libc that defines it as
/// part of the Rust standard library's own runtime — so this needs no new
/// crate dependency, just the raw C declaration.
fn effective_uid() -> u32 {
    // SAFETY: `geteuid()` is a pure, argument-free POSIX call with no
    // preconditions and no failure mode.
    unsafe { geteuid() }
}

extern "C" {
    fn geteuid() -> u32;
}

/// Open (creating if absent) the lock file at `path` for `fd_lock`. Mirrors
/// `update/apply.rs::open_lock_file`; the file's contents are never read —
/// only the fd-level advisory lock matters.
fn open_lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Run `f` while holding an exclusive advisory lock on `root.join(name)`,
/// blocking until any other holder releases it.
///
/// This is R81's coordination primitive: concurrent handlers wait for each
/// other here instead of relying on an ordering assumption that hook events
/// don't actually provide. `root` should be [`resolve_root`]'s result (or
/// another already-validated root); this function does not itself validate
/// anything about `root`; it just opens/creates the one file inside it.
///
/// Also used off the temporary root: `plugin::heartbeat` locks its own
/// machine-level store with this rather than introducing a second locking
/// scheme (KTD5 allows exactly one).
pub fn with_lock<T>(root: &Path, name: &str, f: impl FnOnce() -> T) -> io::Result<T> {
    let file = open_lock_file(&root.join(name))?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.write()?;
    Ok(f())
}

/// Non-blocking sibling of [`with_lock`]: `Ok(None)` immediately when
/// another holder has `root.join(name)` locked right now, rather than
/// waiting. The one-shot-claim half of KTD5's fd-lock pattern (mirroring
/// `update/apply.rs`'s `try_write`), for a caller that would rather skip
/// than block — [`with_lock`] is the one AE61 and R81 actually call for.
#[allow(dead_code)]
pub fn try_with_lock<T>(root: &Path, name: &str, f: impl FnOnce() -> T) -> io::Result<Option<T>> {
    let file = open_lock_file(&root.join(name))?;
    let mut lock = fd_lock::RwLock::new(file);
    // Bound to a local rather than matched directly in tail position: the
    // `Err` arm's guard-less result still borrows `lock`, which would
    // otherwise have to outlive `lock` itself at the end of the function
    // (the same shape `update/apply.rs::try_lock_state_at` works around).
    let outcome = match lock.try_write() {
        Ok(_guard) => Some(f()),
        Err(_) => None,
    };
    Ok(outcome)
}

#[cfg(test)]
mod tests;
