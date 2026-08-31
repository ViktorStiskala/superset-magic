//! The one property that matters here is exclusivity under a genuine race, so
//! that is what [`concurrent_takers_race_and_exactly_one_wins`] measures — with
//! a barrier and N threads, never sequentially. Sequential calls show the
//! behavior we want even when the underlying syscall does not have it (an
//! `unlink`-based claim passes a sequential test and hands the same claim to
//! most of a concurrent burst), so a sequential test here would hide exactly
//! the bug this module exists to avoid.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use tempfile::TempDir;

use super::*;

/// A claim directory holding one claim at `token`.
fn fixture(body: &str) -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    fs::write(&token, body).unwrap();
    (dir, token)
}

#[test]
fn taking_a_claim_yields_what_it_recorded() {
    let (dir, token) = fixture("recorded body");

    let claimed = take(dir.path(), &token).expect("the claim is there to take");
    assert_eq!(claimed.text().as_deref(), Some("recorded body"));
}

#[test]
fn a_claim_is_taken_at_most_once() {
    let (dir, token) = fixture("body");

    let first = take(dir.path(), &token);
    assert!(first.is_some());
    assert!(take(dir.path(), &token).is_none(), "a claim is one-shot");
}

/// The claim file is gone once the winner drops it, so nothing can find it
/// again on a later pass.
#[test]
fn dropping_the_winner_retires_the_claim() {
    let (dir, token) = fixture("body");

    let claimed = take(dir.path(), &token).unwrap();
    let landing = claimed.landing.path().to_path_buf();
    drop(claimed);

    assert!(!token.exists(), "the claim was moved away");
    assert!(!landing.exists(), "and then deleted");
}

#[test]
fn taking_a_claim_that_is_not_there_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert!(take(dir.path(), &dir.path().join("absent")).is_none());
}

#[test]
fn taking_from_a_directory_that_does_not_exist_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-dir");
    assert!(take(&missing, &missing.join("token")).is_none());
}

/// The whole point of the module. Eight threads released together by a barrier
/// reach for one claim; exactly one may come back with it.
///
/// This is the shape that refuted the `unlink`-based claim, which handed the
/// same token to five to eight of these eight threads while passing every
/// sequential test above.
#[test]
fn concurrent_takers_race_and_exactly_one_wins() {
    const THREADS: usize = 8;
    // Repeated, because a race that resolves correctly once may not resolve
    // correctly always — a single trial of a scheduling-dependent property
    // proves very little.
    const TRIALS: usize = 25;

    for trial in 0..TRIALS {
        let (dir, token) = fixture("body");
        let barrier = Arc::new(Barrier::new(THREADS));
        let winners = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                let barrier = Arc::clone(&barrier);
                let winners = Arc::clone(&winners);
                let dir = dir.path().to_path_buf();
                let token = token.clone();
                scope.spawn(move || {
                    barrier.wait();
                    if take(&dir, &token).is_some() {
                        winners.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });

        assert_eq!(
            winners.load(Ordering::SeqCst),
            1,
            "trial {trial}: exactly one taker may win one claim"
        );
    }
}

/// Two claims in one directory are independent: a race for each has its own
/// single winner, and winning one never consumes the other.
#[test]
fn concurrent_takers_of_two_claims_each_have_one_winner() {
    const THREADS: usize = 8;

    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    fs::write(&a, "a").unwrap();
    fs::write(&b, "b").unwrap();

    let barrier = Arc::new(Barrier::new(THREADS * 2));
    let won_a = Arc::new(AtomicUsize::new(0));
    let won_b = Arc::new(AtomicUsize::new(0));

    std::thread::scope(|scope| {
        for (token, counter) in [(&a, &won_a), (&b, &won_b)] {
            for _ in 0..THREADS {
                let barrier = Arc::clone(&barrier);
                let counter = Arc::clone(counter);
                let root = dir.path().to_path_buf();
                let token = token.clone();
                scope.spawn(move || {
                    barrier.wait();
                    if let Some(claimed) = take(&root, &token) {
                        // Each claim carries its own body, so a winner that
                        // somehow took the other one would show up here.
                        assert_eq!(
                            claimed.text().as_deref(),
                            Some(token.file_name().unwrap().to_str().unwrap())
                        );
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        }
    });

    assert_eq!(won_a.load(Ordering::SeqCst), 1);
    assert_eq!(won_b.load(Ordering::SeqCst), 1);
}
