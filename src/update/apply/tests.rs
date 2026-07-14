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
