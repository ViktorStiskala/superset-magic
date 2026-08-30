//! The store's own behavior, tested without a hook envelope anywhere near it:
//! what a record holds, which one a stop takes, what expires, and which
//! declarations the verb refuses outright. `hook/subagent_stop/tests.rs`
//! covers what the handler does with what comes back from here.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use tempfile::TempDir;

use super::*;
use crate::tests::support::{git_run, init_main_repo};

/// 2026-08-30 12:00:00 UTC.
const NOW: u64 = 1_788_091_200;

/// A declaration directory, with no repository around it.
fn fixture() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let declarations = dir_in(&root.join(".superset/.magic"));
    fs::create_dir_all(&declarations).unwrap();
    (dir, declarations)
}

/// A repo whose `.gitignore` already covers the state tree — the ordinary
/// case, after `init`/`migrate` or `plugin enable` has run.
fn ignored_repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    fs::write(
        dir.path().join(".gitignore"),
        "target/\n.superset/.magic/\n",
    )
    .unwrap();
    git_run(&["add", ".gitignore"], dir.path());
    git_run(&["commit", "-q", "-m", "gitignore"], dir.path());
    let root = crate::git::cwd_repo_root(dir.path()).unwrap();
    (dir, root)
}

// ── Records ───────────────────────────────────────────────────────────────────

#[test]
fn a_record_keeps_the_file_it_names_and_when_it_was_declared() {
    let (_d, dir) = fixture();
    let target = PathBuf::from("/repo/docs/REPORT.md");

    let path = record(&dir, &target, "docs/REPORT.md", Some("the findings"), NOW).unwrap();
    let stored: Expectation = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(stored.path, "/repo/docs/REPORT.md");
    assert_eq!(stored.relative, "docs/REPORT.md");
    assert_eq!(stored.note.as_deref(), Some("the findings"));
    assert_eq!(stored.declared_epoch, NOW);
}

/// A declaration names a file, so declaring the same file twice leaves one
/// record — not two blocks waiting to happen.
#[test]
fn declaring_one_file_twice_leaves_one_record() {
    let (_d, dir) = fixture();
    let target = PathBuf::from("/repo/REPORT.md");

    record(&dir, &target, "REPORT.md", None, NOW).unwrap();
    record(&dir, &target, "REPORT.md", None, NOW + 5).unwrap();

    assert_eq!(candidates(&dir).len(), 1);
    let taken = take_oldest(&dir, NOW + 5).unwrap();
    assert_eq!(taken.declared_epoch, NOW + 5, "the later one replaced it");
    assert!(take_oldest(&dir, NOW + 5).is_none());
}

/// The note goes back to a model, so a runaway one is cut rather than passed
/// through whole.
#[test]
fn an_over_long_note_is_bounded() {
    let (_d, dir) = fixture();
    let target = PathBuf::from("/repo/REPORT.md");
    let note = "x".repeat(MAX_NOTE_LEN * 3);

    record(&dir, &target, "REPORT.md", Some(&note), NOW).unwrap();
    let stored = take_oldest(&dir, NOW).unwrap();

    let stored = stored.note.unwrap();
    assert!(stored.len() <= MAX_NOTE_LEN + 4, "{}", stored.len());
    assert!(stored.ends_with('…'));
}

// ── Taking ────────────────────────────────────────────────────────────────────

/// R51: nothing declared means nothing to take, which is what makes
/// `SubagentStop` inert by default.
#[test]
fn an_empty_directory_yields_nothing() {
    let (_d, dir) = fixture();
    assert!(take_oldest(&dir, NOW).is_none());
}

#[test]
fn a_missing_directory_yields_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(take_oldest(&dir.path().join("absent"), NOW).is_none());
}

/// Taking a record removes it, which is the whole block-once mechanism.
#[test]
fn a_record_is_taken_exactly_once() {
    let (_d, dir) = fixture();
    record(&dir, Path::new("/repo/A.md"), "A.md", None, NOW).unwrap();

    assert_eq!(take_oldest(&dir, NOW).unwrap().relative, "A.md");
    assert!(take_oldest(&dir, NOW).is_none(), "the record was consumed");
}

/// With several pending, the oldest declaration is the one a stop checks, and
/// each stop takes exactly one.
#[test]
fn records_are_taken_oldest_first_one_per_call() {
    let (_d, dir) = fixture();
    record(&dir, Path::new("/repo/B.md"), "B.md", None, NOW + 10).unwrap();
    record(&dir, Path::new("/repo/A.md"), "A.md", None, NOW).unwrap();
    record(&dir, Path::new("/repo/C.md"), "C.md", None, NOW + 20).unwrap();

    assert_eq!(take_oldest(&dir, NOW + 30).unwrap().relative, "A.md");
    assert_eq!(take_oldest(&dir, NOW + 30).unwrap().relative, "B.md");
    assert_eq!(take_oldest(&dir, NOW + 30).unwrap().relative, "C.md");
    assert!(take_oldest(&dir, NOW + 30).is_none());
}

/// A dispatch that crashed before it ever spawned leaves a record nothing will
/// satisfy. It ages out, and is swept on the way past rather than left to
/// block an unrelated agent later.
#[test]
fn an_expired_record_is_swept_and_never_enforced() {
    let (_d, dir) = fixture();
    record(&dir, Path::new("/repo/stale.md"), "stale.md", None, NOW).unwrap();

    let later = NOW + MAX_AGE_SECS + 1;
    assert!(
        take_oldest(&dir, later).is_none(),
        "expired, so not enforced"
    );
    assert!(candidates(&dir).is_empty(), "and cleared out of the way");
}

/// An expired record sitting in front of a live one does not hide it: the
/// sweep continues past it in the same call.
#[test]
fn a_live_record_behind_an_expired_one_is_still_found() {
    let (_d, dir) = fixture();
    record(&dir, Path::new("/repo/stale.md"), "stale.md", None, NOW).unwrap();
    let later = NOW + MAX_AGE_SECS + 1;
    record(&dir, Path::new("/repo/live.md"), "live.md", None, later).unwrap();

    assert_eq!(take_oldest(&dir, later).unwrap().relative, "live.md");
    assert!(candidates(&dir).is_empty());
}

/// A record whose body is not a record cannot be enforced — there is no file
/// name in it to name — so it is discarded rather than trusted.
#[test]
fn an_unparseable_record_is_discarded() {
    let (_d, dir) = fixture();
    fs::write(dir.join("deadbeefdeadbeef.json"), "not json at all").unwrap();
    record(&dir, Path::new("/repo/live.md"), "live.md", None, NOW).unwrap();

    assert_eq!(take_oldest(&dir, NOW).unwrap().relative, "live.md");
    assert!(take_oldest(&dir, NOW).is_none());
}

/// The claim's landing files and half-written records both live in this
/// directory; neither may ever be mistaken for a declaration.
#[test]
fn temporary_files_in_the_directory_are_not_records() {
    let (_d, dir) = fixture();
    fs::write(dir.join(".taken-abc.tmp"), "{}").unwrap();
    fs::write(dir.join(".expect-abc.tmp"), "{}").unwrap();
    fs::write(dir.join("notes.txt"), "{}").unwrap();

    assert!(candidates(&dir).is_empty());
    assert!(take_oldest(&dir, NOW).is_none());
}

/// R48. Duplicate stop invocations racing for one declaration: exactly one may
/// come back with it, or the block would fire several times over. Driven with
/// a barrier, because a sequential version of this test passes even on a claim
/// that is not exclusive at all.
#[test]
fn concurrent_stops_race_for_one_declaration_and_exactly_one_wins() {
    const THREADS: usize = 8;

    for _ in 0..10 {
        let (_d, dir) = fixture();
        record(&dir, Path::new("/repo/REPORT.md"), "REPORT.md", None, NOW).unwrap();

        let barrier = Arc::new(Barrier::new(THREADS));
        let winners = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                let barrier = Arc::clone(&barrier);
                let winners = Arc::clone(&winners);
                let dir = dir.clone();
                scope.spawn(move || {
                    barrier.wait();
                    if take_oldest(&dir, NOW).is_some() {
                        winners.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });

        assert_eq!(winners.load(Ordering::SeqCst), 1);
    }
}

/// Two declarations and two concurrent stops check two records between them —
/// a caller that loses a race moves on to the next candidate rather than
/// giving up, so no declaration is silently skipped.
#[test]
fn two_concurrent_stops_take_two_of_two_declarations() {
    let (_d, dir) = fixture();
    record(&dir, Path::new("/repo/A.md"), "A.md", None, NOW).unwrap();
    record(&dir, Path::new("/repo/B.md"), "B.md", None, NOW + 1).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let taken = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let dir = dir.clone();
                scope.spawn(move || {
                    barrier.wait();
                    take_oldest(&dir, NOW + 1).map(|e| e.relative)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });

    let mut taken = taken;
    taken.sort();
    assert_eq!(taken, vec!["A.md".to_string(), "B.md".to_string()]);
}

// ── Checking the declared file ────────────────────────────────────────────────

#[test]
fn a_present_non_empty_file_keeps_the_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("REPORT.md");
    fs::write(&file, "findings").unwrap();

    assert_eq!(check(&file), None);
}

#[test]
fn a_missing_file_is_unmet() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(check(&dir.path().join("absent.md")), Some(Unmet::Missing));
}

/// An agent that creates the file and then runs out of room to write it is
/// exactly the loss this feature exists for, so an empty file is not "written".
#[test]
fn an_empty_file_is_unmet() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("REPORT.md");
    fs::write(&file, "").unwrap();

    assert_eq!(check(&file), Some(Unmet::Empty));
}

#[test]
fn a_directory_where_a_file_was_declared_is_unmet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("REPORT.md");
    fs::create_dir(&path).unwrap();

    assert_eq!(check(&path), Some(Unmet::NotAFile));
}

// ── Resolving a declaration ───────────────────────────────────────────────────

#[test]
fn a_relative_declaration_resolves_against_the_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();

    let resolved = resolve_declared(&root, &root, "docs/REPORT.md").unwrap();
    assert_eq!(resolved, root.join("docs/REPORT.md"));
}

/// The declared file is ordinarily the thing that does not exist yet, and a
/// directory that does not exist yet is just as fine — the agent creates both.
#[test]
fn a_declaration_under_a_directory_that_does_not_exist_yet_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let resolved = resolve_declared(&root, &root, "out/reports/REPORT.md").unwrap();
    assert_eq!(resolved, root.join("out/reports/REPORT.md"));
}

/// R56. A declaration has to name somewhere the subagent and the hook can both
/// see, and a path that climbs out of the worktree is refused where a person
/// is looking at the error rather than silently at stop time.
#[test]
fn a_declaration_outside_the_worktree_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();

    let err = resolve_declared(&root, &root, "../elsewhere/REPORT.md").unwrap_err();
    assert!(err.contains("outside this worktree"), "{err}");

    let err = resolve_declared(&root, &root, "/etc/hosts").unwrap_err();
    assert!(err.contains("outside this worktree"), "{err}");
}

/// A `..` in the middle that comes back inside is fine — it is only leaving
/// that is refused.
#[test]
fn a_parent_segment_that_stays_inside_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let resolved = resolve_declared(&root, &root, "docs/../REPORT.md").unwrap();
    assert_eq!(resolved, root.join("REPORT.md"));
}

/// A symlinked parent could otherwise relocate a declaration out of the
/// worktree while the textual path still looks contained.
#[test]
fn a_declaration_through_a_symlink_out_of_the_worktree_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("repo");
    let outside = base.join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

    let err = resolve_declared(&root, &root, "escape/REPORT.md").unwrap_err();
    assert!(err.contains("outside this worktree"), "{err}");
}

#[test]
fn declaring_an_existing_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();

    let err = resolve_declared(&root, &root, "docs").unwrap_err();
    assert!(err.contains("has to be a single file"), "{err}");
}

#[test]
fn an_empty_declaration_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    assert!(resolve_declared(&root, &root, "   ").is_err());
}

// ── The verb ──────────────────────────────────────────────────────────────────

#[test]
fn the_verb_records_a_declaration_in_the_state_tree() {
    let (dir, root) = ignored_repo();

    let code = run_core(dir.path(), "docs/REPORT.md", Some("findings"), NOW).unwrap();
    assert_eq!(code, ExitCode::SUCCESS);

    let declarations = dir_in(&root.join(".superset/.magic"));
    let taken = take_oldest(&declarations, NOW).expect("a declaration was recorded");
    assert_eq!(taken.relative, "docs/REPORT.md");
    assert_eq!(taken.path, root.join("docs/REPORT.md").to_string_lossy());
    assert_eq!(taken.note.as_deref(), Some("findings"));
}

/// The verb inherits `scratchpad::ensure`'s refusals, so a repository that
/// never had the ignore rule written does not gain an untracked state file
/// from this.
#[test]
fn the_verb_refuses_when_the_state_tree_is_not_ignored() {
    let dir = init_main_repo("main");

    let code = run_core(dir.path(), "REPORT.md", None, NOW).unwrap();
    assert_eq!(code, ExitCode::from(1));
    assert!(!dir.path().join(".superset/.magic").exists());
}

#[test]
fn the_verb_refuses_a_declaration_outside_the_worktree() {
    let (dir, root) = ignored_repo();

    let code = run_core(dir.path(), "/etc/hosts", None, NOW).unwrap();
    assert_eq!(code, ExitCode::from(2));
    assert!(!dir_in(&root.join(".superset/.magic")).exists());
}

#[test]
fn the_verb_refuses_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let code = run_core(dir.path(), "REPORT.md", None, NOW).unwrap();
    assert_eq!(code, ExitCode::from(2));
}

#[test]
fn the_verb_needs_a_file() {
    assert_eq!(run(&[]).unwrap(), ExitCode::from(2));
}

#[test]
fn the_verb_rejects_an_unknown_flag_and_a_dangling_note() {
    assert_eq!(run(&["--nope".to_string()]).unwrap(), ExitCode::from(2));
    assert_eq!(
        run(&["a.md".to_string(), "--note".to_string()]).unwrap(),
        ExitCode::from(2)
    );
}

#[test]
fn the_verb_has_help() {
    assert_eq!(run(&["--help".to_string()]).unwrap(), ExitCode::SUCCESS);
}
