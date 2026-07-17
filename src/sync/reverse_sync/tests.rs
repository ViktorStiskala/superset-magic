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

/// The real on-disk metadata for `path`, as the review-time baseline would have
/// captured it (panics on a non-`NotFound` stat error — a test bug).
fn meta(path: &Path) -> Option<FileMeta> {
    meta_of(path).unwrap()
}

/// Push over an EXISTING main file: main gets the worktree bytes, the OLD main
/// bytes are backed up under `backups_root/TS/main/rel`, a gitignore rule is
/// appended, and the backup path is reported.
#[test]
fn apply_push_overwrites_main_and_backs_up_old_bytes() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_OLD=1\n");
    write(&wt, "config.env", "WT_NEW=1\n");

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("config.env"),
            &Decision::Push,
            Baseline {
                wt: meta(&wt.join("config.env")),
                main: meta(&main.path().join("config.env")),
            },
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
    // exactly one backup, in the batch's main-side namespace, holding the OLD
    // main bytes.
    assert_eq!(res.backups.len(), 1);
    assert_eq!(
        res.backups[0],
        backups.path().join(TS).join("main").join("config.env")
    );
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

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("apps/api/.dev.vars"),
            &Decision::Push,
            Baseline {
                wt: meta(&wt.join("apps/api/.dev.vars")),
                main: meta(&main.path().join("apps/api/.dev.vars")),
            },
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

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("config.env"),
            &Decision::Pull,
            Baseline {
                wt: meta(&wt.join("config.env")),
                main: meta(&main.path().join("config.env")),
            },
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
    assert_eq!(
        res.backups[0],
        backups.path().join(TS).join("worktree").join("config.env"),
        "pull backs up the worktree side under its namespace"
    );
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
    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("config.env"),
            &Decision::Merge(merged.clone()),
            Baseline {
                wt: meta(&wt.join("config.env")),
                main: meta(&main.path().join("config.env")),
            },
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
    // Both originals are backed up at distinct per-side paths.
    assert_eq!(res.backups.len(), 2);
    assert!(
        res.backups
            .contains(&backups.path().join(TS).join("worktree").join("config.env")),
        "worktree-side backup path missing: {:?}",
        res.backups
    );
    assert!(
        res.backups
            .contains(&backups.path().join(TS).join("main").join("config.env")),
        "main-side backup path missing: {:?}",
        res.backups
    );
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

/// A target whose CURRENT metadata no longer matches its review-time baseline
/// (an edit landed in the review→apply window) is skipped, writing NOTHING (no
/// overwrite, no backup, no gitignore). Simulated by handing `apply_decision` a
/// baseline whose length deliberately mismatches the on-disk file.
#[test]
fn apply_skips_when_target_changed_since_review() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_NEW=1\n");

    // A stale baseline: length differs from the real main file, as if main had
    // been edited since the user reviewed it. (A length mismatch is detected
    // regardless of mtime resolution.)
    let stale_main = Some(FileMeta {
        len: 999_999,
        mtime: None,
        content_hash: None,
    });

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Push,
        Baseline {
            wt: meta(&wt.join("config.env")),
            main: stale_main,
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    // main retains its bytes, NOT the worktree bytes.
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "MAIN_ORIG=1\n"
    );
    // Nothing else was touched: no backup and no .gitignore created.
    assert!(
        !backups.path().join(TS).exists(),
        "a skip must not create the batch's backup dir at all"
    );
    assert!(
        !main.path().join(".gitignore").exists(),
        "a skip must not append a gitignore rule"
    );
}

/// The normal Applied path with a MATCHING baseline still overwrites: passing
/// the real current metadata as the baseline (an untouched review window) lets
/// the push through and backs up the old bytes.
#[test]
fn apply_applies_when_baseline_matches_current() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_NEW=1\n");

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("config.env"),
            &Decision::Push,
            Baseline {
                wt: meta(&wt.join("config.env")),
                main: meta(&main.path().join("config.env")),
            },
        )
        .unwrap(),
    );

    // main took the worktree bytes and the old bytes were backed up.
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "WT_NEW=1\n"
    );
    assert_eq!(res.backups.len(), 1);
    assert_eq!(fs::read_to_string(&res.backups[0]).unwrap(), "MAIN_ORIG=1\n");
}

/// A file absent at review time (baseline `None`) that APPEARS before apply
/// (current `Some`) is treated as Changed and skipped — reverse sync never
/// clobbers a file that materialized during the review window (Finding 5:
/// baseline `None` + current `Some` ⇒ Changed, not a fresh no-backup write).
#[test]
fn apply_skips_when_target_appeared_after_review() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(&wt, "config.env", "WT_NEW=1\n");
    // main did NOT hold the file at review time, but it appears before apply.
    write(main.path(), "config.env", "APPEARED=1\n");

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Push,
        Baseline {
            wt: meta(&wt.join("config.env")),
            main: None, // review-time baseline: main absent
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    // main keeps the appeared content, untouched.
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "APPEARED=1\n"
    );
    assert!(
        !backups.path().join(TS).exists(),
        "a skip must not create the batch's backup dir at all"
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

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Undecided,
        Baseline {
            wt: meta(&wt.join("config.env")),
            main: meta(&main.path().join("config.env")),
        },
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

/// Finding 3: a Push whose SOURCE (the worktree file) changed since review is
/// skipped and main is left untouched — reverse sync never pushes source bytes
/// the user never reviewed. Simulated with a STALE worktree (source) baseline
/// whose length no longer matches the on-disk worktree file, while the main
/// (target) baseline matches current — so only the source-side guard can trip.
#[test]
fn apply_push_skips_when_source_changed_since_review() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_NEW=1\n");

    let stale_wt = Some(FileMeta {
        len: 999_999,
        mtime: None,
        content_hash: None,
    });

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Push,
        Baseline {
            wt: stale_wt,
            main: meta(&main.path().join("config.env")),
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    // main keeps its original bytes; nothing was written, backed up, or ignored.
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "MAIN_ORIG=1\n"
    );
    assert!(
        !backups.path().join(TS).exists(),
        "a skip must not create the batch's backup dir at all"
    );
    assert!(
        !main.path().join(".gitignore").exists(),
        "a source-changed skip must not append a gitignore rule"
    );
}

/// Symmetric to the Push case: a Pull whose SOURCE (main) changed since review
/// is skipped and the worktree is left untouched.
#[test]
fn apply_pull_skips_when_source_changed_since_review() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_SIDE=1\n");
    write(&wt, "config.env", "WT_ORIG=1\n");

    let stale_main = Some(FileMeta {
        len: 999_999,
        mtime: None,
        content_hash: None,
    });

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Pull,
        Baseline {
            wt: meta(&wt.join("config.env")),
            main: stale_main,
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    // The worktree keeps its original bytes.
    assert_eq!(
        fs::read_to_string(wt.join("config.env")).unwrap(),
        "WT_ORIG=1\n"
    );
    assert!(
        !backups.path().join(TS).exists(),
        "a skip must not create the batch's backup dir at all"
    );
}

/// Finding 6: one file's apply error does NOT abort the batch — later files are
/// still applied and the failure is tallied. An unsafe (`..`-escaping) rel makes
/// `apply_decision` bail; the good push ordered AFTER it must still land in main.
#[test]
fn apply_batch_continues_past_a_failing_file() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "good.env", "OLD=1\n");
    write(&wt, "good.env", "NEW=1\n");

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };

    // The failing (unsafe) decision is FIRST, proving the batch keeps going and
    // still applies the good one after it.
    let bad = PathBuf::from("../escape.env");
    let good = PathBuf::from("good.env");
    let decisions = vec![(bad.clone(), Decision::Push), (good.clone(), Decision::Push)];
    let mut baseline: HashMap<PathBuf, (Option<FileMeta>, Option<FileMeta>)> = HashMap::new();
    baseline.insert(bad, (None, None));
    baseline.insert(
        good,
        (
            meta(&wt.join("good.env")),
            meta(&main.path().join("good.env")),
        ),
    );

    let summary = apply_batch(&ctx, &decisions, &baseline);

    assert_eq!(summary.failed, 1, "the unsafe path must be counted failed");
    assert_eq!(summary.applied, 1, "the good push must still apply");
    assert_eq!(summary.skipped, 0);
    // The good file really landed in main despite the earlier failure.
    assert_eq!(
        fs::read_to_string(main.path().join("good.env")).unwrap(),
        "NEW=1\n"
    );
    assert_eq!(
        summary.backups.len(),
        1,
        "the good push backed up main's old bytes"
    );
}

/// Delete removes the file from BOTH sides and backs up both originals under
/// their per-side namespaces first; no gitignore rule is appended (nothing is
/// written into main).
#[test]
fn apply_delete_removes_both_sides_and_backs_up_both() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_ORIG=1\n");

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("config.env"),
            &Decision::Delete,
            Baseline {
                wt: meta(&wt.join("config.env")),
                main: meta(&main.path().join("config.env")),
            },
        )
        .unwrap(),
    );

    assert!(matches!(res.direction, WriteDirection::DeleteBoth));
    assert!(!res.gitignore_appended, "delete writes nothing into main");
    assert!(!wt.join("config.env").exists(), "worktree copy removed");
    assert!(!main.path().join("config.env").exists(), "main copy removed");
    assert!(
        !main.path().join(".gitignore").exists(),
        "delete must not append a gitignore rule"
    );
    // Both originals were backed up before the unlinks.
    assert_eq!(res.backups.len(), 2);
    let wt_backup = backups.path().join(TS).join("worktree").join("config.env");
    let main_backup = backups.path().join(TS).join("main").join("config.env");
    assert_eq!(fs::read_to_string(&wt_backup).unwrap(), "WT_ORIG=1\n");
    assert_eq!(fs::read_to_string(&main_backup).unwrap(), "MAIN_ORIG=1\n");
}

/// Delete of a worktree-only file removes just the worktree copy (main has
/// nothing), with a single worktree-side backup.
#[test]
fn apply_delete_worktree_only_removes_and_backs_up_worktree() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let res = applied(
        apply_decision(
            &ctx,
            Path::new("apps/api/.dev.vars"),
            &Decision::Delete,
            Baseline {
                wt: meta(&wt.join("apps/api/.dev.vars")),
                main: None,
            },
        )
        .unwrap(),
    );

    assert!(!wt.join("apps/api/.dev.vars").exists(), "worktree copy removed");
    assert_eq!(res.backups.len(), 1, "only the worktree side existed");
    assert_eq!(
        res.backups[0],
        backups
            .path()
            .join(TS)
            .join("worktree")
            .join("apps/api/.dev.vars")
    );
    assert_eq!(fs::read_to_string(&res.backups[0]).unwrap(), "SECRET=1\n");
}

/// A delete whose worktree side changed since review is skipped: BOTH files
/// stay on disk and nothing is backed up — a concurrent edit is never deleted.
#[test]
fn apply_delete_skips_when_side_changed_since_review() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_ORIG=1\n");
    write(&wt, "config.env", "WT_EDITED=1\n");

    let stale_wt = Some(FileMeta {
        len: 999_999,
        mtime: None,
        content_hash: None,
    });

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Delete,
        Baseline {
            wt: stale_wt,
            main: meta(&main.path().join("config.env")),
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    assert!(wt.join("config.env").exists(), "worktree copy must survive");
    assert!(
        main.path().join("config.env").exists(),
        "main copy must survive"
    );
    assert!(
        !backups.path().join(TS).exists(),
        "a skip must not create the batch's backup dir at all"
    );
}

/// Twin of the worktree-side guard test: a delete whose MAIN side changed
/// since review is skipped independently — both files survive, nothing is
/// backed up.
#[test]
fn apply_delete_skips_when_main_side_changed_since_review() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(main.path(), "config.env", "MAIN_EDITED=1\n");
    write(&wt, "config.env", "WT_ORIG=1\n");

    let stale_main = Some(FileMeta {
        len: 999_999,
        mtime: None,
        content_hash: None,
    });

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Delete,
        Baseline {
            wt: meta(&wt.join("config.env")),
            main: stale_main,
        },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => {
            assert!(reason.contains("changed since review"), "got: {reason}")
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
    assert!(wt.join("config.env").exists(), "worktree copy must survive");
    assert!(
        main.path().join("config.env").exists(),
        "main copy must survive"
    );
    assert!(
        !backups.path().join(TS).exists(),
        "a skip must not create the batch's backup dir at all"
    );
}

/// The defensive both-sides-missing branch: a delete where neither side
/// existed at review nor exists now is a "nothing to delete" skip, not an
/// Applied that pretends to have removed something.
#[test]
fn apply_delete_with_nothing_on_disk_is_skipped() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("ghost.env"),
        &Decision::Delete,
        Baseline { wt: None, main: None },
    )
    .unwrap();

    match outcome {
        ApplyOutcome::Skipped(reason) => assert_eq!(reason, "nothing to delete"),
        other => panic!("expected Skipped, got {other:?}"),
    }
    assert!(!backups.path().join(TS).exists());
}

/// metas_match: mtimes present on both sides decide; without an mtime the
/// content hashes decide; a bare length (no mtime, no hash) must NEVER pass
/// as unchanged — a same-length edit would slip through the TOCTOU guard.
#[test]
fn metas_match_requires_a_real_change_signal() {
    let m = |len: u64, mtime: Option<SystemTime>, content_hash: Option<u64>| FileMeta {
        len,
        mtime,
        content_hash,
    };
    let t = SystemTime::UNIX_EPOCH;
    let t2 = t + std::time::Duration::from_secs(1);
    assert!(metas_match(&m(5, Some(t), None), &m(5, Some(t), None)));
    assert!(!metas_match(&m(5, Some(t), None), &m(5, Some(t2), None)));
    assert!(!metas_match(&m(5, Some(t), None), &m(6, Some(t), None)));
    // No mtime: the content hashes decide.
    assert!(metas_match(&m(5, None, Some(1)), &m(5, None, Some(1))));
    assert!(!metas_match(&m(5, None, Some(1)), &m(5, None, Some(2))));
    // No mtime and no hash: fail safe — never trusted as unchanged.
    assert!(!metas_match(&m(5, None, None), &m(5, None, None)));
    // Mixed signals (one side lost its mtime, no hash on the other): fail safe.
    assert!(!metas_match(&m(5, Some(t), None), &m(5, None, Some(1))));
}

/// Bugbot (stale status): a candidate classified WorktreeOnly whose main copy
/// APPEARS before the baseline capture must still get a `None` main-side
/// baseline — the user reviews it as a plain create (the confirm lists no
/// overwrite), so apply must SKIP rather than overwrite the copy the review
/// never covered.
#[test]
fn review_baseline_pins_main_absent_for_worktree_only_status() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    let backups = tempfile::tempdir().unwrap();

    write(&wt, "config.env", "WT=1\n");
    // Main gains a copy AFTER classify said WorktreeOnly, BEFORE the capture.
    write(main.path(), "config.env", "APPEARED=1\n");

    let (wt_meta, main_meta) = review_baseline(
        &wt,
        main.path(),
        Path::new("config.env"),
        DiffStatus::WorktreeOnly,
    )
    .unwrap();
    assert!(wt_meta.is_some());
    assert!(main_meta.is_none(), "worktree-only status pins main absent");

    // The apply-time guard then refuses the push: main keeps its bytes.
    let ctx = ApplyContext {
        worktree_root: &wt,
        main_root: main.path(),
        backups_root: backups.path(),
        ts: TS,
    };
    let outcome = apply_decision(
        &ctx,
        Path::new("config.env"),
        &Decision::Push,
        Baseline {
            wt: wt_meta,
            main: main_meta,
        },
    )
    .unwrap();
    assert!(
        matches!(outcome, ApplyOutcome::Skipped(_)),
        "expected Skipped, got {outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(main.path().join("config.env")).unwrap(),
        "APPEARED=1\n"
    );

    // A Differs-status candidate captures main's REAL metadata as before.
    let (_w, m) = review_baseline(
        &wt,
        main.path(),
        Path::new("config.env"),
        DiffStatus::Differs,
    )
    .unwrap();
    assert!(m.is_some(), "differs status captures main's metadata");
}

// ── Backup timestamps + retention ────────────────────────────────────────

/// format_timestamp renders epoch seconds as UTC `YYYYmmdd-HHMMSS`, including
/// the leap-day and end-of-day edges.
#[test]
fn format_timestamp_renders_utc_dates() {
    assert_eq!(format_timestamp(0), "19700101-000000");
    assert_eq!(format_timestamp(86_399), "19700101-235959");
    assert_eq!(format_timestamp(86_400), "19700102-000000");
    // Well-known epoch: 2001-09-09 01:46:40 UTC.
    assert_eq!(format_timestamp(1_000_000_000), "20010909-014640");
    // Leap day: 2000-02-29 00:00:00 UTC.
    assert_eq!(format_timestamp(951_782_400), "20000229-000000");
}

/// Retention recognizes only the two batch-name shapes this tool has ever
/// written — current `YYYYmmdd-HHMMSS` and legacy all-digits epoch — and
/// nothing else.
#[test]
fn is_backup_batch_name_matches_only_our_shapes() {
    assert!(is_backup_batch_name("20260716-153000"));
    assert!(is_backup_batch_name("1752624000")); // legacy epoch
    assert!(!is_backup_batch_name("worktree"));
    assert!(!is_backup_batch_name("main"));
    assert!(!is_backup_batch_name("2026-07-16"));
    assert!(!is_backup_batch_name("20260716_153000"));
    assert!(!is_backup_batch_name("20260716-15300")); // 14 chars
    assert!(!is_backup_batch_name(""));
}

/// prune_old_backups keeps the newest `keep` batch dirs (legacy epoch names
/// count as OLDER than every `YYYYmmdd` name), deletes the rest, and never
/// touches non-batch entries.
#[test]
fn prune_old_backups_keeps_newest_and_ignores_foreign_entries() {
    let root = tempfile::tempdir().unwrap();
    let mk = |name: &str| {
        let d = root.path().join(name).join("main");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("x.env"), "X=1\n").unwrap();
    };
    // Two legacy epoch batches (oldest), three current-format batches.
    mk("1752624000");
    mk("1752624100");
    mk("20260716-100000");
    mk("20260716-110000");
    mk("20260716-120000");
    // Foreign entries that must survive: a non-batch dir and a plain file.
    fs::create_dir_all(root.path().join("notes")).unwrap();
    fs::write(root.path().join("README.txt"), "hands off\n").unwrap();

    let pruned = prune_old_backups(root.path(), 3).unwrap();

    let pruned_names: Vec<String> = pruned
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        {
            let mut v = pruned_names.clone();
            v.sort();
            v
        },
        vec!["1752624000".to_string(), "1752624100".to_string()],
        "the two legacy (oldest) batches are pruned; got {pruned_names:?}"
    );
    // The three newest batches remain, foreign entries untouched.
    assert!(root.path().join("20260716-100000").is_dir());
    assert!(root.path().join("20260716-110000").is_dir());
    assert!(root.path().join("20260716-120000").is_dir());
    assert!(!root.path().join("1752624000").exists());
    assert!(!root.path().join("1752624100").exists());
    assert!(root.path().join("notes").is_dir(), "non-batch dir must survive");
    assert!(root.path().join("README.txt").is_file(), "plain file must survive");
}

/// The legacy (unreleased-0.4.0) merge layout — top-level `local/<epoch>/` and
/// `main/<epoch>/` — is folded into its epoch's batch: pruned under the same
/// keep budget (together with the top-level `<epoch>/` dir of the same batch),
/// with the emptied side dirs removed, while a foreign dir named `local`
/// holding non-batch children survives untouched.
#[test]
fn prune_old_backups_folds_legacy_merge_layout_into_batches() {
    let root = tempfile::tempdir().unwrap();
    let seed = |rel: &str| {
        let p = root.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "X=1\n").unwrap();
    };
    // One legacy 0.4.0 batch (epoch 1752624000): push/pull backups at the top
    // level PLUS merge backups under local/ + main/.
    seed("1752624000/config.env");
    seed("local/1752624000/config.env");
    seed("main/1752624000/config.env");
    // Two newer modern batches.
    seed("20260716-100000/main/config.env");
    seed("20260716-110000/main/config.env");
    // A foreign non-batch child under local/ must survive.
    seed("local/notes/keep.txt");

    let pruned = prune_old_backups(root.path(), 2).unwrap();

    // The whole legacy batch — all three of its directories — is pruned.
    assert_eq!(pruned.len(), 3, "got {pruned:?}");
    assert!(!root.path().join("1752624000").exists());
    assert!(!root.path().join("local/1752624000").exists());
    assert!(!root.path().join("main/1752624000").exists());
    // The modern batches survive.
    assert!(root.path().join("20260716-100000").is_dir());
    assert!(root.path().join("20260716-110000").is_dir());
    // local/ still holds the foreign child, so it survives; main/ was emptied
    // by the prune and is removed.
    assert!(root.path().join("local/notes/keep.txt").is_file());
    assert!(!root.path().join("main").exists(), "emptied legacy side dir is removed");
}

/// Fewer batches than `keep` → nothing pruned; a missing backups root is a
/// clean no-op (first-ever sync has no backups dir).
#[test]
fn prune_old_backups_noop_under_threshold_and_missing_root() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("20260716-100000")).unwrap();
    assert!(prune_old_backups(root.path(), 10).unwrap().is_empty());
    assert!(root.path().join("20260716-100000").is_dir());

    let missing = root.path().join("no-such-dir");
    assert!(prune_old_backups(&missing, 10).unwrap().is_empty());
}
