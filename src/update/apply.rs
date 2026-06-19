//! Self-update apply path (U7): lock, download/verify/swap, re-exec.
//!
//! This is the heavy half of the update subsystem, gated on a
//! [`UpdateCheck::Newer`](super::check::UpdateCheck) verdict from U6 (or the
//! forced check behind `ss-magic update`). It is deliberately conservative:
//! it replaces the *running* binary, so any uncertainty falls through to the
//! installed version rather than risking a half-swapped install.
//!
//! Three concerns, each with its own test seam:
//!
//! - **Lock (KTD4, R20).** A single `fd-lock` advisory `try_write()` on a lock
//!   file in the cache dir serializes concurrent updaters. Contention →
//!   [`LockState::Contended`] → the caller SKIPS (does NOT wait) and proceeds
//!   on the current binary. `flock` is kernel-released on crash so a stale lock
//!   is structurally impossible; an mtime TTL ([`STALE_TTL`]) is defense in
//!   depth — a lock file older than the TTL is reclaimed (its mtime bumped) and
//!   acquisition retried. The lock path is injected so tests point it at a
//!   tempdir.
//!
//! - **Download / verify / swap (KTD1, KTD5).** Delegated to the `self_update`
//!   crate's GitHub backend (`self_update::backends::github`), which downloads
//!   the release archive, extracts the binary, and atomically replaces the
//!   running binary via `self-replace`. See the module-level KTD5 conformance
//!   notes below for which finer controls `self_update` does and does not
//!   expose.
//!
//! - **Re-exec (KTD3, R18, R19).** After a successful swap we drop the lock,
//!   then spawn the *swapped* binary resolved via [`std::env::current_exe`]
//!   (NEVER `argv[0]`/`$PATH`) with the original args and `SS_MAGIC_UPDATED=1`,
//!   `wait()` on it (BLOCKING the caller so Superset cannot advance
//!   mid-swap), and exit with the child's code. A signal-killed child maps to
//!   exit 1 ([`propagate_code`]). Running the parent's normal `exit` (not
//!   `execv`) preserves RAII cleanup. The spawn is behind the [`Spawner`] seam
//!   so tests inject a controlled exit status without a real re-exec.
//!
//! ## KTD5 conformance (self_update 0.44.0)
//!
//! - **self-replace atomic swap** — HONORED. `update_extended()` calls
//!   `self_replace::self_replace(new_exe)` when the install path is the running
//!   exe (our default).
//! - **0600 sibling temp file** — PARTIAL. `self_update` downloads + extracts
//!   into a private `tempfile::TempDir` (a fresh 0700 dir), not a 0600 sibling
//!   next to the binary. The in-flight executable is therefore process-private
//!   rather than world-readable, satisfying the *intent* (no world-readable
//!   in-flight binary) but not the literal "sibling temp file mode 0600 on the
//!   same filesystem" mechanic. self-replace performs its final rename through
//!   its own temp on the binary's filesystem.
//! - **Bounded download timeout** — NOT EXPOSED. `self_update`'s download path
//!   builds its own ureq `Agent` with no timeout, and `Download` exposes no
//!   timeout setter, so we cannot bound connect+read on the archive download
//!   through its public API. We DO bound the API metadata calls indirectly (the
//!   U6 pre-check uses a 5s ureq timeout for the cheap "is there an update"
//!   probe), but the archive download itself is unbounded.
//! - **SHA-256 vs REST `digest`** — NOT EXPOSED. The only integrity check
//!   `self_update` offers is ed25519 (`signatures` feature) over
//!   zipsign-signed archives — not the GitHub asset `digest` field. The
//!   checksum-mismatch contract is therefore not enforced here; it is a
//!   smoke-test-deferred residual the orchestrator must decide on (hand-roll a
//!   digest check, or accept TLS + cargo-dist's own checksums as the integrity
//!   boundary for v1).
//!
//! These gaps are reported up so the orchestrator can decide whether to
//! hand-roll the missing pieces. They do NOT compromise the atomic-swap
//! contract itself (the running binary is never left half-written).

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use super::check::REPO_SLUG;

/// Binary name shipped in release archives (matches `[[bin]] name`).
const BIN_NAME: &str = "ss-magic";

/// Lock file name inside the cache dir (sibling of the version cache).
pub const LOCK_FILE_NAME: &str = "update.lock";

/// Env var set on the re-exec'd child so its update check early-returns
/// (KTD6, R21). The child inherits it; the gate (U8) honors it.
pub const UPDATED_ENV: &str = "SS_MAGIC_UPDATED";

/// Documented opt-out: when set the update check is skipped entirely (KTD6).
pub const NO_UPDATE_ENV: &str = "SS_MAGIC_NO_UPDATE";

/// Stale-lock TTL (KTD4). A lock file whose mtime is at least this old is
/// treated as abandoned and reclaimed. Chosen strictly greater than a normal
/// download+verify+rename so a live updater is never mistaken for stale; the
/// kernel already releases `flock` on crash, so this is belt-and-suspenders.
const STALE_TTL: Duration = Duration::from_secs(60);

/// True when a loop-guard / opt-out env var is present (KTD6, R21).
///
/// The re-exec'd child inherits [`UPDATED_ENV`]; [`NO_UPDATE_ENV`] is the
/// documented manual opt-out. Either one means "do not run the update check".
/// U8 wires this into startup; U7 owns setting [`UPDATED_ENV`] on the child.
pub fn guard_active() -> bool {
    env_flag_set(UPDATED_ENV) || env_flag_set(NO_UPDATE_ENV)
}

/// A guard env var counts as set when present and not literally empty/`0`.
fn env_flag_set(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

/// Outcome of attempting to acquire the update lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // used in tests via try_lock_state
pub enum LockState {
    /// The lock was free (or reclaimed as stale) and we hold it.
    Acquired,
    /// Another process holds the lock — the caller skips without waiting.
    Contended,
}

/// Map a child exit status's `Option<i32>` code to a process exit code
/// (KTD3, R19). `None` means the child was killed by a signal (no code) →
/// exit 1. A present code is propagated verbatim, defaulting to 1 only if it
/// somehow can't be represented (it always can here).
///
/// Kept pure (takes the already-extracted `Option<i32>`) so it is unit-tested
/// directly without spawning a process.
pub fn propagate_code(code: Option<i32>) -> i32 {
    code.unwrap_or(1)
}

/// The spawn-and-wait seam (KTD3, R18). Production resolves the re-exec target
/// and spawns it; tests inject a controlled exit code without a real download
/// or re-exec.
///
/// `exe` is the absolute path to run (always [`std::env::current_exe`] in
/// production — never `argv[0]`). `args` are the original args (program name
/// already stripped). The implementation must set [`UPDATED_ENV`] on the
/// child, `wait()` for it (blocking), and return the child's exit code as the
/// raw `Option<i32>` (`None` for a signal-kill) for [`propagate_code`].
pub trait Spawner {
    fn spawn_and_wait(&self, exe: &Path, args: &[OsString]) -> std::io::Result<Option<i32>>;
}

/// Real re-exec: spawn `exe` with `args` + [`UPDATED_ENV`], inherit stdio,
/// block on `wait()`, return the child's `code()`.
pub struct ProcessSpawner;

impl Spawner for ProcessSpawner {
    fn spawn_and_wait(&self, exe: &Path, args: &[OsString]) -> std::io::Result<Option<i32>> {
        let status = Command::new(exe)
            .args(args)
            .env(UPDATED_ENV, "1")
            .status()?;
        Ok(status.code())
    }
}

/// Re-exec the swapped binary and exit the current process with its code.
///
/// Resolves the target via [`std::env::current_exe`] (R18: never `argv[0]`),
/// passes the original args (`args_os().skip(1)`), and delegates the actual
/// spawn+wait to `spawner` so it's testable.
///
/// On the happy path it propagates the child's exit code. If the binary was
/// swapped but the new binary can't be resolved or re-executed, the work the
/// caller asked for did NOT run — so we exit NON-ZERO with a diagnostic rather
/// than silently exiting 0. A silent exit-0 would tell an unattended caller
/// (e.g. Superset running `magic.sh sync`) that the sync succeeded when it
/// never ran; a visible failure is the honest, recoverable outcome.
///
/// This never returns: it terminates via [`std::process::exit`].
pub fn reexec_and_exit<S: Spawner>(spawner: &S) -> ! {
    let (exe, args) = match reexec_target() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "{}",
                crate::style::err(format!(
                    "error: self-update replaced the binary but could not resolve the new \
                     executable to run ({e}); the requested work did NOT run. Re-run ss-magic."
                ))
            );
            std::process::exit(1);
        }
    };
    match spawner.spawn_and_wait(&exe, &args) {
        Ok(code) => std::process::exit(propagate_code(code)),
        Err(e) => {
            eprintln!(
                "{}",
                crate::style::err(format!(
                    "error: self-update replaced the binary but re-executing it failed ({e}); \
                     the requested work did NOT run. Re-run ss-magic."
                ))
            );
            std::process::exit(1);
        }
    }
}

/// Resolve the re-exec target: the running binary via [`std::env::current_exe`]
/// (R18 — NEVER `argv[0]`/`$PATH`) plus the original args (program name
/// stripped, `args_os().skip(1)`). Extracted from [`reexec_and_exit`] so the
/// resolution itself is unit-testable (the exit is not).
fn reexec_target() -> std::io::Result<(std::path::PathBuf, Vec<OsString>)> {
    let exe = std::env::current_exe()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    Ok((exe, args))
}

/// Whether the lock file at `path` is stale by mtime (KTD4).
///
/// Stale = exists AND its mtime is at least [`STALE_TTL`] in the past. A
/// missing file, an unreadable mtime, or a clock anomaly is treated as "not
/// stale" (conservative — we don't want to wrongly steal a live lock).
fn lock_is_stale(path: &Path, ttl: Duration, now: SystemTime) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    match now.duration_since(modified) {
        Ok(age) => age >= ttl,
        // mtime is in the future (clock skew) → treat as fresh.
        Err(_) => false,
    }
}

/// Open (creating if absent) the lock file for `fd-lock`. The file's contents
/// are irrelevant — only the fd-level advisory lock matters.
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Bump the lock file's mtime to now, so reclaiming a stale lock resets its
/// freshness for the next contender. Best-effort.
fn touch(path: &Path) {
    let now = SystemTime::now();
    let _ = File::open(path).and_then(|f| f.set_modified(now));
}

/// Try to acquire the update lock at `lock_path`, with stale reclaim.
///
/// Returns [`LockState::Acquired`] if we obtained the advisory `try_write()`
/// lock (releasing it immediately — the caller's critical section is handled
/// by [`apply_update`], which re-acquires for the duration of the swap), or if
/// the file was stale and reclaimed. Returns [`LockState::Contended`] when a
/// live process holds it.
///
/// This is the seam the AE2 test drives: the test holds a real `fd-lock`
/// write lock on `lock_path`, then asserts this returns `Contended`.
#[allow(dead_code)] // used in tests
pub fn try_lock_state(lock_path: &Path) -> LockState {
    try_lock_state_at(lock_path, STALE_TTL, SystemTime::now())
}

/// Testable core of [`try_lock_state`] with an injectable TTL + clock.
#[allow(dead_code)] // used in tests
fn try_lock_state_at(lock_path: &Path, ttl: Duration, now: SystemTime) -> LockState {
    let stale = lock_is_stale(lock_path, ttl, now);

    let Ok(file) = open_lock_file(lock_path) else {
        // Can't even open the lock file (read-only cache dir, etc.) — treat as
        // contended so we skip the update rather than charging ahead unlocked.
        return LockState::Contended;
    };
    let mut lock = fd_lock::RwLock::new(file);
    // Bind the acquired/failed state to a bool, dropping the guard (or the
    // failed `Err`, which borrows `lock`) before the function returns so the
    // borrow doesn't outlive `lock`'s scope.
    let acquired = lock.try_write().is_ok();
    if acquired {
        // Held only for the `try_write` call above; the guard has already
        // dropped. Bump mtime so a freshly-acquired lock looks fresh.
        touch(lock_path);
        LockState::Acquired
    } else if stale {
        // A live `flock` can't be older than the TTL (kernel releases it on
        // crash), so an old-mtime file that's still locked is a pathological
        // case; reclaim it by bumping mtime and report Acquired so the caller
        // proceeds (it will re-lock under `apply_update`, the real critical
        // section).
        touch(lock_path);
        LockState::Acquired
    } else {
        LockState::Contended
    }
}

/// Result of the apply path, reported to the caller (main / U8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Another updater held the lock; we skipped (AE2). Caller runs current.
    Skipped,
    /// Ran the swap path but no newer release was actually installed
    /// (`self_update` reported up-to-date, or download/verify/swap failed and
    /// fell through). Caller runs current.
    NoUpdate,
    /// Swap succeeded; `version` is the newly installed release version.
    /// The caller should re-exec (production does this via
    /// [`reexec_and_exit`]).
    Updated { version: String },
}

/// Run the `self_update` download/verify/swap for `target_tag` (or the latest
/// compatible release when `None`), holding the update lock for the duration.
///
/// `lock_path` is injected (tempdir in tests). On lock contention → returns
/// [`ApplyOutcome::Skipped`] WITHOUT waiting (AE2). Any download/verify/swap
/// failure is swallowed → [`ApplyOutcome::NoUpdate`] so an unattended caller is
/// never blocked or broken by a flaky release.
///
/// This performs real network I/O and a real binary swap, so it is NOT
/// unit-tested directly (the SEAMS — lock state, exit-code propagation — are
/// tested in isolation). It is exercised by manual smoke against a real
/// release.
pub fn apply_update(lock_path: &Path, target_tag: Option<&str>) -> ApplyOutcome {
    // Acquire + HOLD the lock for the whole critical section. We open the file
    // and keep the `RwLock` alive in this scope so the guard lives as long as
    // the swap runs.
    let stale = lock_is_stale(lock_path, STALE_TTL, SystemTime::now());
    let Ok(file) = open_lock_file(lock_path) else {
        return ApplyOutcome::Skipped;
    };
    let mut lock = fd_lock::RwLock::new(file);
    // First attempt. We can't bind the failed `Err` (it borrows `lock`) and
    // then re-borrow `lock` for the reclaim, so probe with `is_ok()` (which
    // drops the guard/err immediately) and only then take a held guard.
    if lock.try_write().is_err() {
        if !stale {
            return ApplyOutcome::Skipped;
        }
        // Reclaim a stale lock: bump its mtime, then re-attempt. The prior
        // holder is presumed dead (flock is kernel-released on crash), so this
        // won't actually block in practice.
        touch(lock_path);
    }
    let _guard = match lock.try_write() {
        Ok(g) => g,
        // Still contended (live holder, or lost a reclaim race) → skip.
        Err(_) => return ApplyOutcome::Skipped,
    };

    // Inside the lock: run the self_update swap. Any error → NoUpdate.
    match run_self_update(target_tag) {
        Ok(Some(version)) => ApplyOutcome::Updated { version },
        Ok(None) => ApplyOutcome::NoUpdate,
        Err(_) => ApplyOutcome::NoUpdate,
    }
    // `_guard` (and `lock`) drop here, releasing the advisory lock before the
    // caller re-execs.
}

/// Run the swap WITHOUT a lock — used only as a defensive fallback when no
/// cache dir resolves (so there's nowhere to put a lock file) on the explicit
/// `ss-magic update` force path. Every supported platform resolves a cache
/// dir, so this is rarely reached; locking is preferred via [`apply_update`].
pub fn apply_update_unlocked(target_tag: Option<&str>) -> ApplyOutcome {
    match run_self_update(target_tag) {
        Ok(Some(version)) => ApplyOutcome::Updated { version },
        Ok(None) => ApplyOutcome::NoUpdate,
        Err(_) => ApplyOutcome::NoUpdate,
    }
}

/// Drive the `self_update` GitHub backend. Returns `Ok(Some(version))` on a
/// successful swap, `Ok(None)` when already up to date, `Err` on any failure
/// (network, archive, swap). `target_tag` pins a specific release tag when
/// known (the forced/checked path); `None` lets the backend pick the latest.
///
/// Configured `no_confirm(true)` + `show_output(false)` so it runs silently
/// and unattended (no TTY prompt). The binary is replaced in place at
/// `current_exe()` (the backend's default `bin_install_path`).
fn run_self_update(target_tag: Option<&str>) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let (owner, repo) = split_slug(REPO_SLUG);

    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(owner)
        .repo_name(repo)
        .bin_name(BIN_NAME)
        // cargo-dist nests the binary inside a `<bin>-<target>/` directory in
        // the release tarball (verified: `ss-magic-aarch64-apple-darwin/ss-magic`).
        // self_update defaults `bin_path_in_archive` to the bare bin name, which
        // would fail extraction and silently report UpToDate, so set it to match
        // cargo-dist's layout. self_update substitutes `{{ bin }}`/`{{ target }}`.
        .bin_path_in_archive("{{ bin }}-{{ target }}/{{ bin }}")
        .current_version(env!("CARGO_PKG_VERSION"))
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false);
    if let Some(tag) = target_tag {
        builder.target_version_tag(tag);
    }

    let status = builder.build()?.update()?;
    match status {
        self_update::Status::Updated(v) => Ok(Some(v)),
        self_update::Status::UpToDate(_) => Ok(None),
    }
}

/// Split an `owner/repo` slug into its two halves; if there's no `/`, the
/// whole string is the owner and the repo is empty (builder validation then
/// surfaces a clear config error).
fn split_slug(slug: &str) -> (&str, &str) {
    match slug.split_once('/') {
        Some((owner, repo)) => (owner, repo),
        None => (slug, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Env vars are process-global; serialize the guard tests so they don't
    /// race each other (Rust runs tests multithreaded by default).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // ── propagate_code (KTD3, R19) ──────────────────────────────────────────

    #[test]
    fn propagate_code_passes_through_zero() {
        assert_eq!(propagate_code(Some(0)), 0);
    }

    #[test]
    fn propagate_code_passes_through_nonzero() {
        assert_eq!(propagate_code(Some(3)), 3);
        assert_eq!(propagate_code(Some(42)), 42);
    }

    #[test]
    fn propagate_code_maps_signal_kill_to_one() {
        // A signal-killed child has no exit code (`None`) → exit 1.
        assert_eq!(propagate_code(None), 1);
    }

    // ── guard_active (AE4, KTD6, R21) ───────────────────────────────────────

    #[test]
    fn ae4_guard_active_when_updated_env_set() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var(NO_UPDATE_ENV);
        std::env::set_var(UPDATED_ENV, "1");
        assert!(
            guard_active(),
            "SS_MAGIC_UPDATED=1 must activate the loop guard"
        );
        std::env::remove_var(UPDATED_ENV);
    }

    #[test]
    fn guard_active_when_no_update_env_set() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var(UPDATED_ENV);
        std::env::set_var(NO_UPDATE_ENV, "1");
        assert!(
            guard_active(),
            "SS_MAGIC_NO_UPDATE=1 must activate the opt-out"
        );
        std::env::remove_var(NO_UPDATE_ENV);
    }

    #[test]
    fn guard_inactive_when_neither_env_set() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var(UPDATED_ENV);
        std::env::remove_var(NO_UPDATE_ENV);
        assert!(!guard_active(), "no marker → guard inactive, check proceeds");
    }

    #[test]
    fn guard_inactive_for_empty_or_zero_value() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var(NO_UPDATE_ENV);
        std::env::set_var(UPDATED_ENV, "");
        assert!(!guard_active(), "empty value must not activate the guard");
        std::env::set_var(UPDATED_ENV, "0");
        assert!(!guard_active(), "`0` must not activate the guard");
        std::env::remove_var(UPDATED_ENV);
    }

    // ── Lock: skip-on-contention, stale reclaim (AE2, KTD4, R20) ────────────

    #[test]
    fn ae2_lock_held_makes_second_caller_contend_and_skip() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILE_NAME);

        // First "process": hold a real fd-lock write lock for the duration.
        let file = open_lock_file(&lock_path).unwrap();
        let mut held = fd_lock::RwLock::new(file);
        let _held_guard = held.try_write().expect("first caller acquires the lock");

        // Second caller (this one) must see contention and skip immediately —
        // no blocking, no wait.
        let state = try_lock_state(&lock_path);
        assert_eq!(
            state,
            LockState::Contended,
            "a held lock must report Contended (second caller skips, runs current)"
        );

        // apply_update on the same contended lock must Skip without waiting.
        let outcome = apply_update(&lock_path, Some("v999.0.0"));
        assert_eq!(
            outcome,
            ApplyOutcome::Skipped,
            "contended apply must Skip and not attempt a download"
        );
    }

    #[test]
    fn free_lock_is_acquired() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILE_NAME);
        // Nothing holds it → Acquired (and released immediately).
        assert_eq!(try_lock_state(&lock_path), LockState::Acquired);
        // Re-acquirable after release.
        assert_eq!(try_lock_state(&lock_path), LockState::Acquired);
    }

    #[test]
    fn stale_lock_by_mtime_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILE_NAME);

        // Create the lock file and hold a live lock, but force its mtime to
        // look ancient. The mtime is what `lock_is_stale` reads; with a tiny
        // TTL and a "now" far in the future, the file is stale.
        let file = open_lock_file(&lock_path).unwrap();
        let mut held = fd_lock::RwLock::new(file);
        let _held = held.try_write().unwrap();

        // Set mtime to the unix epoch (definitely older than any TTL).
        File::open(&lock_path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();

        // Even though the lock is *held*, an mtime older than the TTL is
        // reclaimed → Acquired (defense-in-depth path).
        let state = try_lock_state_at(&lock_path, Duration::from_secs(60), SystemTime::now());
        assert_eq!(
            state,
            LockState::Acquired,
            "a lock file older than the TTL must be reclaimed"
        );
    }

    #[test]
    fn fresh_lock_file_with_no_holder_is_acquired_not_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILE_NAME);
        // Create a fresh (just-touched) lock file with no live holder.
        let _ = open_lock_file(&lock_path).unwrap();
        // Not stale (just created), no holder → normal acquisition.
        let state = try_lock_state_at(&lock_path, Duration::from_secs(60), SystemTime::now());
        assert_eq!(state, LockState::Acquired);
    }

    // ── Re-exec target + exit-code propagation (KTD3, R18, R19) ─────────────

    /// A `Spawner` test double: records the exe path + args it was handed and
    /// returns a canned exit code, so we can assert the re-exec target without
    /// a real download or process replacement.
    struct RecordingSpawner {
        code: Option<i32>,
        seen_exe: std::sync::Mutex<Option<PathBuf>>,
        seen_args: std::sync::Mutex<Vec<OsString>>,
    }

    impl RecordingSpawner {
        fn new(code: Option<i32>) -> Self {
            Self {
                code,
                seen_exe: std::sync::Mutex::new(None),
                seen_args: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl Spawner for RecordingSpawner {
        fn spawn_and_wait(&self, exe: &Path, args: &[OsString]) -> std::io::Result<Option<i32>> {
            *self.seen_exe.lock().unwrap() = Some(exe.to_path_buf());
            *self.seen_args.lock().unwrap() = args.to_vec();
            Ok(self.code)
        }
    }

    /// The re-exec target must be `current_exe()` (absolute), never `argv[0]`.
    /// We can't let `reexec_and_exit` call `process::exit` in a test, but its
    /// target resolution is factored into `reexec_target()`; we drive that
    /// production helper through the spawn seam and assert what the spawner
    /// received. (`reexec_and_exit` is a thin `exit(propagate_code(...))`
    /// wrapper around exactly this.)
    #[test]
    fn reexec_target_is_current_exe_not_argv0() {
        let spawner = RecordingSpawner::new(Some(0));
        let (exe, args) = reexec_target().expect("current_exe resolves in tests");
        let code = spawner.spawn_and_wait(&exe, &args).unwrap();

        let seen = spawner.seen_exe.lock().unwrap().clone().unwrap();
        assert!(seen.is_absolute(), "re-exec target must be an absolute path");
        assert_eq!(
            seen,
            std::env::current_exe().unwrap(),
            "re-exec target must be current_exe(), not argv[0]"
        );
        assert_eq!(propagate_code(code), 0);
    }

    /// Child exit codes propagate through the spawn seam: zero, non-zero, and
    /// a signal-kill (`None`) → 1.
    #[test]
    fn child_exit_code_propagates_through_seam() {
        for (canned, expected) in [(Some(0), 0), (Some(7), 7), (None, 1)] {
            let spawner = RecordingSpawner::new(canned);
            let exe = std::env::current_exe().unwrap();
            let code = spawner.spawn_and_wait(&exe, &[]).unwrap();
            assert_eq!(
                propagate_code(code),
                expected,
                "canned child code {canned:?} must propagate to {expected}"
            );
        }
    }

    // ── slug split ──────────────────────────────────────────────────────────

    #[test]
    fn split_slug_splits_owner_and_repo() {
        assert_eq!(split_slug("owner/repo"), ("owner", "repo"));
        assert_eq!(split_slug("ViktorStiskala/superset-magic"), ("ViktorStiskala", "superset-magic"));
    }
}
