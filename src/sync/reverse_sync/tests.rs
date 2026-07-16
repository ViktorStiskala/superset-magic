use super::*;
use crate::sync::merge::Decision;
use crate::tests::support::git_run;
use tempfile::TempDir;

fn init_main_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", "main"], dir.path());
    crate::tests::support::neutralize_global_excludes(dir.path());
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

fn make_worktree(main_dir: &Path) -> (TempDir, PathBuf) {
    let wt = tempfile::tempdir().unwrap();
    let wt_path = wt.path().join("wt");
    git_run(
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/rs-test",
            wt_path.to_str().unwrap(),
        ],
        main_dir,
    );
    let wt_root = wt_path.canonicalize().unwrap();
    (wt, wt_root)
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn write_magic(root: &Path, patterns: &[&str]) {
    fs::create_dir_all(root.join(".superset")).unwrap();
    let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let cfg = superset_files::MagicConfig { files };
    let body = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
    fs::write(root.join(".superset/magic.json"), body).unwrap();
}

fn rels(v: &[PathBuf]) -> Vec<String> {
    let mut s: Vec<String> = v.iter().map(|p| p.to_string_lossy().to_string()).collect();
    s.sort();
    s
}

// ── compute_candidates ──────────────────────────────────────────────────

/// AE9 (candidate side): untracked `apps/api/.dev.vars` IS a candidate;
/// tracked `magic.json` is NOT (tracked files reach main via merge).
#[test]
fn ae9_untracked_is_candidate_tracked_is_not() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // magic.json (tracked once committed) matches itself via a pattern,
    // plus an untracked secret.
    write_magic(&wt, &["**/.dev.vars", ".superset/magic.json"]);
    // Track magic.json so it's NOT untracked.
    git_run(&["add", ".superset/magic.json"], &wt);
    // Untracked secret matching the pattern.
    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        cands.contains(&"apps/api/.dev.vars".to_string()),
        "untracked secret must be a candidate; got {cands:?}"
    );
    assert!(
        !cands.contains(&".superset/magic.json".to_string()),
        "tracked magic.json must NOT be a candidate; got {cands:?}"
    );
}

/// magic.local.json (gitignored ⇒ untracked) flows through the same path
/// with no special-casing.
#[test]
fn magic_local_json_is_candidate_when_matched_and_untracked() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write_magic(&wt, &[".superset/magic.local.json"]);
    // magic.local.json present and untracked (gitignore not even needed —
    // it's simply not added).
    write(&wt, ".superset/magic.local.json", "{\"files\":[]}\n");

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        cands.contains(&".superset/magic.local.json".to_string()),
        "magic.local.json must be a candidate; got {cands:?}"
    );
}

/// No magic.json in the worktree → empty candidate set (nothing configured).
#[test]
fn no_magic_json_yields_no_candidates() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");
    let cands = compute_candidates(&wt).unwrap();
    assert!(cands.is_empty(), "got {cands:?}");
}

/// A tracked file matching the pattern is excluded even when its content
/// differs from HEAD (modified-but-tracked still goes via merge).
#[test]
fn modified_tracked_file_is_not_a_candidate() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    write_magic(&wt, &["tracked.env"]);
    write(&wt, "tracked.env", "ORIG=1\n");
    git_run(&["add", "tracked.env"], &wt);
    git_run(&["commit", "-q", "-m", "add tracked.env"], &wt);
    // Modify it without committing.
    write(&wt, "tracked.env", "CHANGED=1\n");

    let cands = compute_candidates(&wt).unwrap();
    assert!(
        !rels(&cands).contains(&"tracked.env".to_string()),
        "modified-but-tracked file must NOT be a candidate; got {cands:?}"
    );
}

/// Regression: a GITIGNORED untracked secret that matches the patterns MUST
/// be a candidate. The whole point of reverse sync is pushing gitignored
/// secrets; a probe carrying `--exclude-standard` dropped exactly these, so
/// the candidate set was always empty in a real repo. Covers AE9 (candidate
/// side) for the realistic gitignored shape.
#[test]
fn gitignored_secret_is_a_candidate() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // Worktree gitignores the secret via a glob (tracked .gitignore).
    write(&wt, ".gitignore", "**/.dev.vars\n");
    git_run(&["add", ".gitignore"], &wt);
    // magic.json matches the secret; the secret is present and gitignored.
    write_magic(&wt, &["**/.dev.vars"]);
    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

    // Sanity: the secret really is ignored in the worktree.
    assert!(
        git::is_ignored(&wt, Path::new("apps/api/.dev.vars")).unwrap(),
        "test precondition: secret must be gitignored in the worktree"
    );

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        cands.contains(&"apps/api/.dev.vars".to_string()),
        "gitignored untracked secret MUST be a candidate; got {cands:?}"
    );
}

/// The gitignored `.superset/magic.local.json` (the canonical bootstrap
/// shape — it is gitignored, not merely un-added) MUST be a candidate when
/// matched and untracked. Same defect class as the secret above.
#[test]
fn gitignored_magic_local_json_is_a_candidate() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write(&wt, ".gitignore", ".superset/magic.local.json\n");
    git_run(&["add", ".gitignore"], &wt);
    write_magic(&wt, &[".superset/magic.local.json"]);
    write(&wt, ".superset/magic.local.json", "{\"files\":[]}\n");

    assert!(
        git::is_ignored(&wt, Path::new(".superset/magic.local.json")).unwrap(),
        "test precondition: magic.local.json must be gitignored in the worktree"
    );

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        cands.contains(&".superset/magic.local.json".to_string()),
        "gitignored magic.local.json MUST be a candidate; got {cands:?}"
    );
}

/// Guard for the broadened probe: a file that matches a pattern AND is
/// gitignored but is also TRACKED (force-added past .gitignore) is still
/// excluded — tracked files reach main via merge. Confirms the fix did not
/// start pulling tracked files into the candidate set.
#[test]
fn modified_tracked_secret_is_not_a_candidate() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write(&wt, ".gitignore", "**/.dev.vars\n");
    git_run(&["add", ".gitignore"], &wt);
    write_magic(&wt, &["**/.dev.vars"]);
    write(&wt, "apps/api/.dev.vars", "ORIG=1\n");
    // Precondition: the secret is genuinely gitignored, so a non-vacuous
    // "not a candidate" below is owed to its TRACKED state, not to the
    // gitignore/pattern setup silently failing.
    assert!(
        git::is_ignored(&wt, Path::new("apps/api/.dev.vars")).unwrap(),
        "precondition: secret must be gitignored before force-adding"
    );
    // Force-add past the gitignore so the secret is TRACKED, then commit.
    git_run(&["add", "-f", "apps/api/.dev.vars"], &wt);
    git_run(&["commit", "-q", "-m", "track secret"], &wt);
    // Modify it without committing — still tracked.
    write(&wt, "apps/api/.dev.vars", "CHANGED=1\n");

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        !cands.contains(&"apps/api/.dev.vars".to_string()),
        "tracked (even gitignored, even modified) file must NOT be a candidate; got {cands:?}"
    );
}

/// A secret whose entire parent DIRECTORY is gitignored (e.g. `secrets/`) —
/// the realistic "ignored secrets dir" shape — is still a candidate.
#[test]
fn secret_in_gitignored_directory_is_a_candidate() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write(&wt, ".gitignore", "secrets/\n");
    git_run(&["add", ".gitignore"], &wt);
    write_magic(&wt, &["secrets/api.key"]);
    write(&wt, "secrets/api.key", "KEY=1\n");

    assert!(
        git::is_ignored(&wt, Path::new("secrets/api.key")).unwrap(),
        "precondition: secret must be gitignored via the directory rule"
    );

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        cands.contains(&"secrets/api.key".to_string()),
        "secret inside a gitignored directory MUST be a candidate; got {cands:?}"
    );
}

/// DEFAULT_EXCLUDES (`node_modules`) are dropped by the matcher, so an
/// untracked secret under `node_modules/` never becomes a candidate even
/// though `git ls-files --others` would see it. Locks the two-layer
/// protection: matcher exclude + pathspec-scoped intersection.
#[test]
fn node_modules_secret_is_not_a_candidate() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write(&wt, ".gitignore", "**/.dev.vars\n");
    git_run(&["add", ".gitignore"], &wt);
    write_magic(&wt, &["**/.dev.vars"]);
    write(&wt, "apps/api/.dev.vars", "REAL=1\n");
    write(&wt, "node_modules/pkg/.dev.vars", "VENDORED=1\n");

    let cands = rels(&compute_candidates(&wt).unwrap());
    assert!(
        cands.contains(&"apps/api/.dev.vars".to_string()),
        "real secret must be a candidate; got {cands:?}"
    );
    assert!(
        !cands.contains(&"node_modules/pkg/.dev.vars".to_string()),
        "node_modules secret must be excluded by DEFAULT_EXCLUDES; got {cands:?}"
    );
}

// ── classify (diff-aware) ────────────────────────────────────────────────

/// Worktree-only file → WorktreeOnly; identical file → Identical (hidden);
/// differing file → Differs.
#[test]
fn classify_distinguishes_new_identical_and_differing() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // worktree-only
    write(&wt, "new.env", "NEW=1\n");
    // identical in both
    write(&wt, "same.env", "SAME=1\n");
    write(main.path(), "same.env", "SAME=1\n");
    // differing
    write(&wt, "diff.env", "WT=1\n");
    write(main.path(), "diff.env", "MAIN=1\n");

    assert_eq!(
        classify(main.path(), &wt, Path::new("new.env")).unwrap(),
        DiffStatus::WorktreeOnly
    );
    assert_eq!(
        classify(main.path(), &wt, Path::new("same.env")).unwrap(),
        DiffStatus::Identical
    );
    assert_eq!(
        classify(main.path(), &wt, Path::new("diff.env")).unwrap(),
        DiffStatus::Differs
    );
}

#[test]
fn is_safe_rel_accepts_normal_rejects_escapes() {
    assert!(is_safe_rel(Path::new("apps/api/.dev.vars")));
    assert!(is_safe_rel(Path::new(".superset/magic.local.json")));
    assert!(!is_safe_rel(Path::new("../oops")));
    assert!(!is_safe_rel(Path::new("a/../b")));
    assert!(!is_safe_rel(Path::new("/abs")));
}

// ── apply_decision (merge cockpit apply seam) ────────────────────────────

const TS: &str = "20260716-000000";

fn applied(outcome: ApplyOutcome) -> ApplyResult {
    match outcome {
        ApplyOutcome::Applied(r) => r,
        other => panic!("expected Applied, got {other:?}"),
    }
}

/// Push over an EXISTING main file: main gets the worktree bytes, the OLD main
/// bytes are backed up under `backups_root/TS/rel`, a gitignore rule is
/// appended, and the backup path is reported.
#[test]
fn apply_push_overwrites_main_and_backs_up_old_bytes() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_OLD=1\n");
    write(&wt, "config.env", "WT_NEW=1\n");

    let res = applied(
        apply_decision(
            &wt,
            main.path(),
            backups.path(),
            TS,
            Path::new("config.env"),
            &Decision::Push,
        )
        .unwrap(),
    );

    assert!(matches!(res.direction, WriteDirection::PushToMain));
    assert!(res.gitignore_appended, "push into main must ignore the secret");
    // main now carries the worktree bytes.
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "WT_NEW=1\n"
    );
    // exactly one backup, at the timestamped path, holding the OLD main bytes.
    assert_eq!(res.backups.len(), 1);
    assert_eq!(res.backups[0], backups.path().join(TS).join("config.env"));
    assert_eq!(fs::read_to_string(&res.backups[0]).unwrap(), "MAIN_OLD=1\n");
    assert!(git::is_ignored(main.path(), Path::new("config.env")).unwrap());
}

/// Push to a NEW main path: it is created (parent dirs + file) and gitignored,
/// with no backup since there were no prior bytes.
#[test]
fn apply_push_to_new_main_path_creates_and_gitignores_without_backup() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

    let res = applied(
        apply_decision(
            &wt,
            main.path(),
            backups.path(),
            TS,
            Path::new("apps/api/.dev.vars"),
            &Decision::Push,
        )
        .unwrap(),
    );

    assert!(res.backups.is_empty(), "a new path has no prior bytes to back up");
    assert!(res.gitignore_appended);
    assert_eq!(
        fs::read_to_string(main.path().join("apps/api/.dev.vars")).unwrap(),
        "SECRET=1\n"
    );
    assert!(git::is_ignored(main.path(), Path::new("apps/api/.dev.vars")).unwrap());
}

/// Pull overwrites the WORKTREE with main's bytes and backs up the worktree's
/// old bytes; no gitignore step on the worktree side.
#[test]
fn apply_pull_overwrites_worktree_and_backs_up_its_old_bytes() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_SIDE=1\n");
    write(&wt, "config.env", "WT_OLD=1\n");

    let res = applied(
        apply_decision(
            &wt,
            main.path(),
            backups.path(),
            TS,
            Path::new("config.env"),
            &Decision::Pull,
        )
        .unwrap(),
    );

    assert!(matches!(res.direction, WriteDirection::PullFromMain));
    assert!(!res.gitignore_appended, "pull writes the worktree side only");
    assert_eq!(
        fs::read_to_string(wt.join("config.env")).unwrap(),
        "MAIN_SIDE=1\n"
    );
    assert_eq!(res.backups.len(), 1);
    assert_eq!(fs::read_to_string(&res.backups[0]).unwrap(), "WT_OLD=1\n");
}

/// Merge writes the assembled text to BOTH sides and backs up both originals
/// (to distinct paths, so neither is lost).
#[test]
fn apply_merge_writes_assembled_to_both_and_backs_up_both() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_ORIG=1\n");

    let merged = "ASSEMBLED=1\n".to_string();
    let res = applied(
        apply_decision(
            &wt,
            main.path(),
            backups.path(),
            TS,
            Path::new("config.env"),
            &Decision::Merge(merged.clone()),
        )
        .unwrap(),
    );

    assert!(matches!(res.direction, WriteDirection::MergeBoth));
    assert!(res.gitignore_appended);
    // Both sides converge on the assembled text.
    assert_eq!(fs::read_to_string(wt.join("config.env")).unwrap(), merged);
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        merged
    );
    // Both originals are backed up at distinct paths.
    assert_eq!(res.backups.len(), 2);
    let contents: Vec<String> = res
        .backups
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    assert!(
        contents.contains(&"WT_ORIG=1\n".to_string()),
        "worktree original must be backed up; got {contents:?}"
    );
    assert!(
        contents.contains(&"MAIN_ORIG=1\n".to_string()),
        "main original must be backed up; got {contents:?}"
    );
}

/// A target that changed on disk after its snapshot is skipped, writing
/// NOTHING (no target overwrite, no backup, no gitignore).
#[test]
fn apply_skips_when_target_changed_after_snapshot() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_NEW=1\n");

    let main_path = main.path().join("config.env");
    let mut fired = false;
    let outcome = apply_decision_hooked(
        &wt,
        main.path(),
        backups.path(),
        TS,
        Path::new("config.env"),
        &Decision::Push,
        &mut || {
            if !fired {
                fired = true;
                // Concurrent edit lands in the snapshot→write window (different
                // length ⇒ reliably detected regardless of mtime resolution).
                fs::write(&main_path, "MAIN_CHANGED_CONCURRENTLY=999\n").unwrap();
            }
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    // main retains the concurrent edit, NOT the worktree bytes.
    assert_eq!(
        fs::read_to_string(&main_path).unwrap(),
        "MAIN_CHANGED_CONCURRENTLY=999\n"
    );
    // Nothing else was touched: no backup and no .gitignore created.
    assert!(!backups.path().join(TS).join("config.env").exists());
    assert!(
        !main.path().join(".gitignore").exists(),
        "a skip must not append a gitignore rule"
    );
}

/// An Undecided decision is a no-op skip; neither side is touched.
#[test]
fn apply_undecided_is_skipped() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN=1\n");
    write(&wt, "config.env", "WT=1\n");

    let outcome = apply_decision(
        &wt,
        main.path(),
        backups.path(),
        TS,
        Path::new("config.env"),
        &Decision::Undecided,
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => assert_eq!(reason, "undecided"),
        other => panic!("expected Skipped, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "MAIN=1\n"
    );
    assert_eq!(fs::read_to_string(wt.join("config.env")).unwrap(), "WT=1\n");
}
