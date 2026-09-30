use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::*;

/// The mode bits of `path`, masked to the permission bits alone.
fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

// ── identifier (R80's cross-language derivation) ──────────────────────────────

/// Pinned against `sha256("")`'s own pinned vector in `hashing::tests`
/// (`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`):
/// an unset or empty `$HOME` must still produce a well-defined identifier,
/// because `bootstrap.sh`'s shell side cannot special-case "unset" either —
/// it just hashes whatever `$HOME` expands to.
#[test]
fn identifier_of_empty_home_matches_known_vector() {
    assert_eq!(identifier(""), "e3b0c44298fc1c14");
}

#[test]
fn identifier_is_first_16_hex_chars_of_sha256() {
    let home = "/Users/example";
    assert_eq!(identifier(home), &hashing::sha256_hex(home.as_bytes())[..16]);
    assert_eq!(identifier(home).len(), 16);
}

#[test]
fn identifier_differs_for_different_home() {
    assert_ne!(identifier("/home/alice"), identifier("/home/bob"));
}

#[test]
fn identifier_is_deterministic() {
    assert_eq!(identifier("/home/alice"), identifier("/home/alice"));
}

// ── validate_component ────────────────────────────────────────────────────────

#[test]
fn validate_component_accepts_owner_only_dir_owned_by_euid() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("root");
    fs::create_dir(&target).unwrap();
    chmod(&target, 0o700);
    assert!(validate_component(&target, effective_uid()));
}

#[test]
fn validate_component_rejects_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!validate_component(&dir.path().join("nope"), effective_uid()));
}

#[test]
fn validate_component_rejects_wrong_mode() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("root");
    fs::create_dir(&target).unwrap();
    chmod(&target, 0o755);
    assert!(!validate_component(&target, effective_uid()));
}

#[test]
fn validate_component_rejects_symlink_even_to_a_valid_directory() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    chmod(&real, 0o700);
    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();
    assert!(!validate_component(&link, effective_uid()));
}

#[test]
fn validate_component_rejects_a_uid_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("root");
    fs::create_dir(&target).unwrap();
    chmod(&target, 0o700);
    // A real directory we just created is always owned by our own euid, so
    // any OTHER value is guaranteed to mismatch — this is the "foreign
    // owned" case without needing actual root/chown privileges in a test.
    let foreign = effective_uid().wrapping_add(1);
    assert!(!validate_component(&target, foreign));
}

// ── resolve_root_at (the injectable core) ──────────────────────────────────────

#[test]
fn resolve_root_creates_owner_only_root_and_namespace() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();

    let root = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();

    assert_eq!(root, primary.path().join(NAMESPACE_DIR).join(identifier("/home/alice")));
    assert_eq!(mode_of(&root), 0o700);
    assert_eq!(mode_of(root.parent().unwrap()), 0o700);
}

#[test]
fn resolve_root_is_idempotent_across_repeated_calls() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();

    let first = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();
    let second = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();
    assert_eq!(first, second);
}

/// The fallback fires when `/tmp` (here, `primary`) cannot host a writable
/// private root at all — e.g. a sandbox that only allowlists `$TMPDIR`.
#[test]
fn resolve_root_falls_back_when_primary_base_is_unwritable() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();
    chmod(primary.path(), 0o500); // read+execute only: mkdir inside it fails.

    let root = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();

    assert!(root.starts_with(fallback.path()));
    chmod(primary.path(), 0o700); // restore so the tempdir can clean itself up.
}

/// A `ss-magic-plugin` namespace directory planted as a symlink — even one
/// that resolves to a perfectly good directory — fails validation, because a
/// predictable path being present is not evidence this process created it.
#[test]
fn resolve_root_falls_back_when_namespace_dir_is_a_symlink() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();

    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), primary.path().join(NAMESPACE_DIR)).unwrap();

    let root = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();

    assert!(root.starts_with(fallback.path()));
}

/// Same as above, but the symlink is one level deeper: the namespace
/// directory is genuine, only the per-identifier directory beneath it is a
/// symlink.
#[test]
fn resolve_root_falls_back_when_identifier_dir_is_a_symlink() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();

    let namespace = primary.path().join(NAMESPACE_DIR);
    fs::create_dir(&namespace).unwrap();
    chmod(&namespace, 0o700);
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), namespace.join(identifier("/home/alice"))).unwrap();

    let root = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();

    assert!(root.starts_with(fallback.path()));
}

/// A pre-existing directory at the right shape but the wrong mode (0755
/// instead of 0700) is exactly as untrustworthy as a symlink or a foreign
/// owner — group/other access defeats the whole point of a private root.
#[test]
fn resolve_root_falls_back_when_existing_dir_has_wrong_mode() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();

    let namespace = primary.path().join(NAMESPACE_DIR);
    fs::create_dir(&namespace).unwrap();
    chmod(&namespace, 0o755);

    let root = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid).unwrap();

    assert!(root.starts_with(fallback.path()));
}

/// A directory owned by someone other than this process fails validation the
/// same way a symlink does — proven at the per-base [`ensure_and_validate`]
/// level (the unit `resolve_root_at` composes for each base in turn) rather
/// than end-to-end through `resolve_root_at`'s two-base fallback: a test has
/// no privilege to actually `chown` a real directory to a foreign uid, and
/// `resolve_root_at` takes ONE `euid` for the whole call — the same value a
/// real process's single effective uid always would — so there is no way to
/// make a freshly-created fallback validate against a DIFFERENT uid than a
/// pre-existing primary within one such call. Injecting a mismatched `euid`
/// here is the faithful substitute: a directory this test just created is
/// always really owned by the real euid, so comparing it against ANY other
/// value reproduces exactly what "foreign owned" looks like to
/// `validate_component`, without needing a second real user on the machine.
/// The fallback CONTROL FLOW itself (try primary, fall through on failure)
/// is already exercised by the symlink/mode/unwritable scenarios above; this
/// isolates the ownership half of what makes a base fail.
#[test]
fn ensure_and_validate_rejects_a_foreign_owned_dir_but_accepts_a_real_owner() {
    let base = tempfile::tempdir().unwrap();
    let real_euid = effective_uid();
    let foreign = real_euid.wrapping_add(1);
    let id = identifier("/home/alice");

    assert_eq!(ensure_and_validate(base.path(), &id, foreign), None);

    // The same on-disk directories `ensure_and_validate` just created (and
    // rejected) are genuinely owned by the real euid, so checking again
    // against the real value succeeds — isolating the uid comparison as the
    // only thing that changed between the two calls.
    assert_eq!(
        ensure_and_validate(base.path(), &id, real_euid),
        Some(base.path().join(NAMESPACE_DIR).join(&id))
    );
}

/// When NEITHER base validates, [`resolve_root_at`] refuses outright rather
/// than writing into (or trusting) a root it cannot vouch for.
#[test]
fn resolve_root_errs_when_neither_base_validates() {
    let primary = tempfile::tempdir().unwrap();
    let fallback = tempfile::tempdir().unwrap();
    let euid = effective_uid();
    chmod(primary.path(), 0o500);
    chmod(fallback.path(), 0o500);

    let result = resolve_root_at("/home/alice", primary.path(), fallback.path(), euid);

    assert_eq!(result, Err(NoValidRoot));
    chmod(primary.path(), 0o700);
    chmod(fallback.path(), 0o700);
}

// ── with_lock / try_with_lock ────────────────────────────────────────────────

#[test]
fn with_lock_runs_the_closure_and_returns_its_value() {
    let dir = tempfile::tempdir().unwrap();
    let value = with_lock(dir.path(), "test.lock", || 7).unwrap();
    assert_eq!(value, 7);
}

#[test]
fn try_with_lock_returns_some_when_uncontended() {
    let dir = tempfile::tempdir().unwrap();
    let value = try_with_lock(dir.path(), "test.lock", || 7).unwrap();
    assert_eq!(value, Some(7));
}

/// Two threads racing `with_lock` on the same file must never run their
/// critical sections at the same time — this is the whole reason R81 routes
/// concurrent handler coordination through this lock instead of an ordering
/// assumption. `fd-lock` uses `flock` on Unix, whose contention is tracked
/// per OPEN FILE DESCRIPTION rather than per process, so two threads that
/// each independently open the lock file (as `with_lock` does on every call)
/// genuinely contend with each other — this is exactly what makes the
/// in-process, two-thread shape of this test meaningful rather than a
/// same-process no-op.
#[test]
fn with_lock_serializes_two_concurrent_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let busy = Arc::new(AtomicBool::new(false));
    let overlapped = Arc::new(AtomicBool::new(false));

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let busy = Arc::clone(&busy);
            let overlapped = Arc::clone(&overlapped);
            std::thread::spawn(move || {
                with_lock(&root, "race.lock", || {
                    if busy.swap(true, Ordering::SeqCst) {
                        overlapped.store(true, Ordering::SeqCst);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                    busy.store(false, Ordering::SeqCst);
                })
                .unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    assert!(!overlapped.load(Ordering::SeqCst), "critical sections overlapped");
}

/// A live holder makes [`try_with_lock`] report contention immediately
/// (`None`, no waiting); once it releases, the exact same lock is free
/// again. The release-then-reacquire half of this is the same mechanism a
/// crashed holder's kernel-driven fd close relies on for the "a stale lock
/// from a dead process does not deadlock" property: `flock` cares only that
/// every file descriptor referencing the lock has closed, not that the
/// closing happened via a clean `Drop` versus a process dying — spawning an
/// actual second OS process to prove that literally would need a companion
/// test binary this crate doesn't otherwise ship, for a guarantee that is
/// the kernel's, not this module's, to keep (the same reasoning
/// `update/apply.rs`'s AE2 test leans on without re-deriving it).
#[test]
fn try_with_lock_sees_contention_then_the_release() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

    let holder_root = root.clone();
    let holder = std::thread::spawn(move || {
        with_lock(&holder_root, "contend.lock", || {
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    });

    held_rx.recv().unwrap(); // the holder thread now holds the lock.
    assert_eq!(try_with_lock(&root, "contend.lock", || 1).unwrap(), None);

    release_tx.send(()).unwrap();
    holder.join().unwrap();

    assert_eq!(try_with_lock(&root, "contend.lock", || 1).unwrap(), Some(1));
}
