//! `write_atomically` is the crash-safety primitive every plugin writer now
//! shares, so the properties worth pinning down here are the ones the
//! consolidation promised its ten call sites: a reader never sees a partial
//! file, `mode` behaves exactly as documented in both directions, and a
//! failure at any step leaves the target and the directory clean rather than
//! littered with a stray temp file.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::*;

// ── Atomicity ─────────────────────────────────────────────────────────────────

/// The whole point of writing to a temp file and renaming it over the target:
/// a concurrent reader must always see either the fully-old or the fully-new
/// content, never a truncated or mixed one. A naive truncate-then-write would
/// fail this quickly; `rename`'s atomicity at the directory-entry level is
/// what a plain in-place write cannot offer.
#[test]
fn a_reader_never_sees_a_partially_written_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");

    let body_a = "a".repeat(200_000);
    let body_b = "b".repeat(200_000);
    fs::write(&target, &body_a).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reader_target = target.clone();
    let reader_stop = Arc::clone(&stop);
    let reader_a = body_a.clone();
    let reader_b = body_b.clone();
    let reader = std::thread::spawn(move || {
        while !reader_stop.load(Ordering::Relaxed) {
            // A transient I/O error here (e.g. a read racing the rename by a
            // hair) is not what this test is about; only a read that
            // *succeeds* with neither whole body is a failure.
            if let Ok(contents) = fs::read_to_string(&reader_target) {
                assert!(
                    contents == reader_a || contents == reader_b,
                    "read {} bytes that were neither fully old nor fully new",
                    contents.len()
                );
            }
        }
    });

    for i in 0..40 {
        let body = if i % 2 == 0 { &body_b } else { &body_a };
        write_atomically(&target, body, "tmp-", ".tmp", None, None, false).unwrap();
    }

    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    // The loop's last write (i = 39, odd) was body_a.
    assert_eq!(fs::read_to_string(&target).unwrap(), body_a);
}

// ── Mode ──────────────────────────────────────────────────────────────────────

/// `Some(mode)` chmods the replaced file to exactly that mode.
#[test]
fn mode_is_applied_exactly_when_requested() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");

    write_atomically(&target, "body", "tmp-", ".tmp", None, Some(0o640), false).unwrap();

    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

/// `None` does not chmod at all — the resulting mode is whatever a plain temp
/// file in that directory gets on its own, regardless of what the file being
/// replaced was set to. Proved by comparing two targets, one with no prior
/// file and one whose prior file had a deliberately unusual mode: if
/// `write_atomically` inherited or otherwise derived a mode from the old
/// file, the two outcomes would differ.
#[test]
fn mode_none_leaves_the_temp_files_own_permissions_alone() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh.txt");
    let replaced = dir.path().join("replaced.txt");
    fs::write(&replaced, "old").unwrap();
    fs::set_permissions(&replaced, fs::Permissions::from_mode(0o444)).unwrap();

    write_atomically(&fresh, "body", "tmp-", ".tmp", None, None, false).unwrap();
    write_atomically(&replaced, "body", "tmp-", ".tmp", None, None, false).unwrap();

    let fresh_mode = fs::metadata(&fresh).unwrap().permissions().mode() & 0o777;
    let replaced_mode = fs::metadata(&replaced).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        fresh_mode, replaced_mode,
        "a `None` mode must not be influenced by the file it replaced"
    );
    assert_ne!(
        replaced_mode, 0o444,
        "the replaced file's old mode must not survive the write"
    );
}

// ── Sync ──────────────────────────────────────────────────────────────────────

/// `sync: true` is best-effort and must not change the outcome on the happy
/// path — the content lands exactly as given either way. (Whether the fsync
/// syscall itself fired is not observable from a unit test; what is
/// observable, and what a regression here would break, is that asking for it
/// does not throw away or corrupt the write.)
#[test]
fn sync_true_still_writes_the_full_body() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");

    write_atomically(&target, "synced body", "tmp-", ".tmp", None, None, true).unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), "synced body");
}

// ── Failure cleanliness ───────────────────────────────────────────────────────

/// A failure partway through must not leave the temp file behind. Renaming a
/// file over an existing, non-empty-of-purpose directory fails at the OS
/// level (`EISDIR`), which forces `persist` itself to fail without needing to
/// break the create/write/chmod steps that come before it.
#[test]
fn a_failed_persist_does_not_leave_a_temp_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();

    let result = write_atomically(&target, "body", "tmp-", ".tmp", None, None, false);
    assert!(result.is_err(), "renaming a file over a directory must fail");
    drop(result); // run the failed NamedTempFile's Drop-based cleanup now.

    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        leftovers,
        vec![std::ffi::OsString::from("target")],
        "a failed persist must not leave its temp file behind: {leftovers:?}"
    );
}

/// Writing into a directory that does not exist fails cleanly — an error, not
/// a panic — and does not create the missing directory as a side effect of
/// trying.
#[test]
fn writing_into_a_missing_directory_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    let target = missing.join("target.txt");

    let result = write_atomically(&target, "body", "tmp-", ".tmp", None, None, false);

    let err = result.expect_err("a missing directory must not be silently created");
    assert!(
        err.to_string().contains("creating a temp file"),
        "expected a temp-file-creation error, got: {err}"
    );
    assert!(
        !missing.exists(),
        "a failed write must not create the missing directory"
    );
}
