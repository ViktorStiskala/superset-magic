//! Claim mechanics and the verb's exit codes.
//!
//! The claim half needs no git and no repository, which is the point: the Read
//! gate consumes claims on the hot path and must not spawn anything. Only
//! [`run_core`] touches a repository, because it bootstraps the state tree
//! through `scratchpad::ensure` to inherit that function's refusals.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use tempfile::TempDir;

use super::*;
use crate::tests::support::{git_run, init_main_repo};

/// 2026-08-30 12:00:00 UTC.
const NOW: u64 = 1_788_091_200;

/// A claim directory and a file to claim, with no repository around either.
fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let dir_path = dir_for_root(&root);
    let file = root.join("big.rs");
    fs::write(&file, "x").unwrap();
    (dir, dir_path, file)
}

#[test]
fn a_recorded_claim_is_consumed_exactly_once() {
    let (_d, claims, file) = fixture();
    record(&claims, &file, NOW).unwrap();

    assert!(consume(&claims, &file, NOW));
    assert!(!consume(&claims, &file, NOW), "a claim is one-shot");
}

#[test]
fn consuming_a_claim_that_was_never_recorded_is_false() {
    let (_d, claims, file) = fixture();
    assert!(!consume(&claims, &file, NOW));
}

/// The claim is per file, so recording one for `a` does not open the gate for
/// `b`.
#[test]
fn claims_are_keyed_per_file() {
    let (_d, claims, file) = fixture();
    let other = file.parent().unwrap().join("other.rs");
    fs::write(&other, "x").unwrap();

    record(&claims, &file, NOW).unwrap();
    assert!(!consume(&claims, &other, NOW));
    assert!(consume(&claims, &file, NOW));
}

/// Recording twice leaves one claim, not two — `record` replaces rather than
/// accumulating, so a person who runs the verb twice does not get two reads.
#[test]
fn recording_twice_still_leaves_one_claim() {
    let (_d, claims, file) = fixture();
    record(&claims, &file, NOW).unwrap();
    record(&claims, &file, NOW).unwrap();

    assert!(consume(&claims, &file, NOW));
    assert!(!consume(&claims, &file, NOW));
}

/// An expired claim is consumed — so it stops sitting in the tree waiting to
/// surprise somebody — but does not open the gate.
#[test]
fn an_expired_claim_is_cleared_without_opening_the_gate() {
    let (_d, claims, file) = fixture();
    let path = record(&claims, &file, NOW).unwrap();

    assert!(!consume(&claims, &file, NOW + MAX_AGE_SECS + 1));
    assert!(!path.exists(), "the expired claim should have been removed");

    // Exactly at the bound is still usable.
    record(&claims, &file, NOW).unwrap();
    assert!(consume(&claims, &file, NOW + MAX_AGE_SECS));
}

/// A claim whose body cannot be parsed is still a claim somebody deliberately
/// recorded, so it is honored rather than discarded on a technicality.
#[test]
fn a_claim_with_an_unreadable_body_is_still_honored() {
    let (_d, claims, file) = fixture();
    fs::create_dir_all(&claims).unwrap();
    fs::write(claim_path(&claims, &file), "not json at all").unwrap();

    assert!(consume(&claims, &file, NOW));
    assert!(!consume(&claims, &file, NOW));
}

/// The race the removal-is-the-claim design exists for: many readers, one
/// claim, exactly one winner. A check-then-delete would let several through.
#[test]
fn concurrent_consumers_race_for_one_claim_and_exactly_one_wins() {
    let (_d, claims, file) = fixture();
    record(&claims, &file, NOW).unwrap();

    const THREADS: usize = 8;
    let barrier = Arc::new(Barrier::new(THREADS));
    let winners = Arc::new(AtomicUsize::new(0));

    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            let barrier = Arc::clone(&barrier);
            let winners = Arc::clone(&winners);
            let claims = claims.clone();
            let file = file.clone();
            scope.spawn(move || {
                barrier.wait();
                if consume(&claims, &file, NOW) {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });

    assert_eq!(winners.load(Ordering::SeqCst), 1);
}

/// The claim file names the path it is for: the file name is a hash, so
/// without this a person looking at the directory could not tell what any of
/// it was about.
#[test]
fn a_claim_records_the_path_it_is_for() {
    let (_d, claims, file) = fixture();
    let path = record(&claims, &file, NOW).unwrap();
    let body = fs::read_to_string(&path).unwrap();

    assert!(body.contains(&file.to_string_lossy().into_owned()));
    assert!(body.contains(&NOW.to_string()));
}

/// Owner-only, like the rest of the state tree (R58).
#[test]
fn a_claim_is_written_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_d, claims, file) = fixture();
    let path = record(&claims, &file, NOW).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode was {mode:o}");
}

// ── The verb ─────────────────────────────────────────────────────────────────

/// A repository whose state tree git already ignores — the ordinary case,
/// after `ss-magic init` or `plugin enable`.
fn ignored_repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    fs::write(dir.path().join(".gitignore"), "target/\n.superset/.magic/\n").unwrap();
    git_run(&["add", ".gitignore"], dir.path());
    git_run(&["commit", "-q", "-m", "gitignore"], dir.path());
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

#[test]
fn the_verb_records_a_claim_the_gate_can_consume() {
    let (_dir, root) = ignored_repo();
    let file = root.join("big.rs");
    fs::write(&file, "x").unwrap();

    let code = run_core(&root, file.to_str().unwrap(), NOW).unwrap();
    assert_eq!(code, ExitCode::SUCCESS);

    let claims = dir_for_root(&root);
    assert!(consume(&claims, &file, NOW));
}

/// The claim is keyed on the resolved physical path, so reaching the file
/// through a symlink records the same claim the gate will look for.
#[test]
fn the_verb_resolves_symlinks_before_recording() {
    let (_dir, root) = ignored_repo();
    let file = root.join("big.rs");
    fs::write(&file, "x").unwrap();
    let link = root.join("link.rs");
    std::os::unix::fs::symlink(&file, &link).unwrap();

    run_core(&root, link.to_str().unwrap(), NOW).unwrap();
    assert!(consume(&dir_for_root(&root), &file, NOW));
}

#[test]
fn the_verb_refuses_a_path_that_does_not_resolve() {
    let (_dir, root) = ignored_repo();
    let code = run_core(&root, "nowhere/at/all.rs", NOW).unwrap();
    assert_eq!(code, ExitCode::from(2));
}

/// The state tree is bootstrapped through `scratchpad::ensure`, so the verb
/// inherits its refusals — above all the one that declines to write while git
/// does not ignore the tree, which is what keeps a claim from turning up as an
/// untracked file in the user's working copy.
#[test]
fn the_verb_writes_nothing_while_the_state_tree_is_not_ignored() {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    let file = root.join("big.rs");
    fs::write(&file, "x").unwrap();

    let code = run_core(&root, file.to_str().unwrap(), NOW).unwrap();
    assert_eq!(code, ExitCode::from(1));
    assert!(!dir_for_root(&root).join("..").join("conclusions").exists());
    assert!(!consume(&dir_for_root(&root), &file, NOW));
}

/// The caller's half of the `wrote_state` contract, and the actual exploit the
/// flag prevents.
///
/// `run_core` bootstraps through `scratchpad::ensure` and then writes its claim
/// into `report.state_root.join("bypass")` — it does not re-check containment,
/// because `!report.wrote_state` is supposed to have already stopped it. A
/// containment refusal raised late in `ensure` (R56) used to leave that flag
/// true, so this verb printed the refusal as a warning and wrote the claim
/// anyway, straight through the symlink `ensure` had just declined to follow.
///
/// The trap: the exit code is only half the story. Asserting `ExitCode::from(1)`
/// alone would be satisfied by a run that refused AFTER writing, so this also
/// asserts the escape directory is still empty — that is the file that must
/// never appear.
#[test]
fn the_verb_writes_no_claim_when_the_claim_directory_escapes_the_worktree() {
    let (_dir, root) = ignored_repo();
    let file = root.join("big.rs");
    fs::write(&file, "x").unwrap();

    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.join(crate::plugin::scratchpad::STATE_REL)).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir_for_root(&root)).unwrap();

    let code = run_core(&root, file.to_str().unwrap(), NOW).unwrap();

    assert!(
        fs::read_dir(outside.path()).unwrap().next().is_none(),
        "the verb wrote a bypass claim outside the worktree, into {} — \
         `scratchpad::ensure` refused the symlink and the verb wrote through \
         it anyway",
        outside.path().display()
    );
    assert_eq!(
        code,
        ExitCode::from(1),
        "the verb must stop on a containment refusal, not warn and continue"
    );
}

#[test]
fn the_verb_rejects_unknown_flags_and_extra_arguments() {
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(run(&args(&["--nope"])).unwrap(), ExitCode::from(2));
    assert_eq!(run(&args(&[])).unwrap(), ExitCode::from(2));
    assert_eq!(run(&args(&["a", "b"])).unwrap(), ExitCode::from(2));
    assert_eq!(run(&args(&["--help"])).unwrap(), ExitCode::SUCCESS);
}
