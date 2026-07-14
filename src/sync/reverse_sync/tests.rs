use super::*;
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

// ── copy_candidate_into_main ─────────────────────────────────────────────

/// AE9 (copy side): copying `apps/api/.dev.vars` creates `apps/api/` in
/// main and ensures the path is gitignored in main via its COVERING rule
/// (`**/.dev.vars`), appended when absent.
#[test]
fn ae9_copy_creates_dirs_and_appends_covering_rule() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // Worktree ignores the secret via a glob.
    write(&wt, ".gitignore", "**/.dev.vars\n");
    git_run(&["add", ".gitignore"], &wt);
    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

    let outcome = copy_candidate_into_main(
        &wt,
        main.path(),
        Path::new("apps/api/.dev.vars"),
        |_| Ok(true),
    )
    .unwrap();
    assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

    // Directory + file created in main.
    assert!(
        main.path().join("apps/api/.dev.vars").is_file(),
        "secret must be copied into main with parent dirs"
    );
    let copied = fs::read_to_string(main.path().join("apps/api/.dev.vars")).unwrap();
    assert_eq!(copied, "SECRET=1\n");

    // main's .gitignore now carries the COVERING glob, not the literal path.
    let gi = fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert!(
        gi.contains("**/.dev.vars"),
        "covering glob must be appended to main's .gitignore; got: {gi:?}"
    );
    assert!(
        !gi.contains("apps/api/.dev.vars"),
        "literal path must NOT be used when a covering rule exists; got: {gi:?}"
    );
    // And the path is now actually ignored in main.
    assert!(git::is_ignored(main.path(), Path::new("apps/api/.dev.vars")).unwrap());
}

/// No covering rule in the worktree → the literal relative path is appended.
#[test]
fn copy_appends_literal_path_when_no_covering_rule() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());
    // No worktree .gitignore covering the secret.
    write(&wt, "secrets/api.key", "KEY=1\n");

    let outcome =
        copy_candidate_into_main(&wt, main.path(), Path::new("secrets/api.key"), |_| {
            Ok(true)
        })
        .unwrap();
    assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

    let gi = fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert!(
        gi.contains("secrets/api.key"),
        "literal path must be appended when no covering rule; got: {gi:?}"
    );
}

/// Regression (secret-leak boundary): when the worktree ignores the secret
/// via a SUBDIR-anchored rule (`/.dev.vars` in `apps/api/.gitignore`),
/// copying that bare rule into main's ROOT `.gitignore` would NOT match
/// `apps/api/.dev.vars`. The verify-then-fallback must detect that and
/// append the literal repo-relative path so the secret ends up actually
/// ignored in main.
#[test]
fn copy_falls_back_to_literal_when_covering_rule_is_subdir_anchored() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // `/.dev.vars` is anchored to apps/api/, not the repo root.
    write(&wt, "apps/api/.gitignore", "/.dev.vars\n");
    git_run(&["add", "apps/api/.gitignore"], &wt);
    write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

    let outcome = copy_candidate_into_main(
        &wt,
        main.path(),
        Path::new("apps/api/.dev.vars"),
        |_| Ok(true),
    )
    .unwrap();
    assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

    // The secret MUST be ignored in main regardless of which rule landed —
    // this is the boundary the fix protects.
    assert!(
        git::is_ignored(main.path(), Path::new("apps/api/.dev.vars")).unwrap(),
        "subdir-anchored covering rule must fall back to the literal path so \
         the secret is actually ignored in main; .gitignore: {:?}",
        fs::read_to_string(main.path().join(".gitignore")).ok()
    );
}

/// Candidate already gitignored in main via an exact line → no duplicate
/// line appended (already-ignored ⇒ no-op).
#[test]
fn copy_no_duplicate_when_already_gitignored_in_main() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // main already ignores the path via an exact line.
    write(main.path(), ".gitignore", "secrets/api.key\n");
    git_run(&["add", ".gitignore"], main.path());
    write(&wt, "secrets/api.key", "KEY=1\n");

    let outcome =
        copy_candidate_into_main(&wt, main.path(), Path::new("secrets/api.key"), |_| {
            Ok(true)
        })
        .unwrap();
    assert_eq!(
        outcome,
        CopyOutcome::Copied { appended_gitignore: false },
        "already-ignored ⇒ no rule appended"
    );

    let gi = fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert_eq!(
        gi.matches("secrets/api.key").count(),
        1,
        "must not duplicate the existing line; got: {gi:?}"
    );
}

/// Candidate exists in main → overwrite requires the decision; decline
/// leaves main's copy intact.
#[test]
fn copy_overwrite_declined_leaves_main_intact() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write(main.path(), "config.env", "MAIN_ORIGINAL=1\n");
    write(&wt, "config.env", "WORKTREE_NEW=1\n");

    // Decision returns false → skip.
    let outcome = copy_candidate_into_main(&wt, main.path(), Path::new("config.env"), |_| {
        Ok(false)
    })
    .unwrap();
    assert_eq!(outcome, CopyOutcome::SkippedOverwriteDeclined);

    let after = fs::read_to_string(main.path().join("config.env")).unwrap();
    assert_eq!(
        after, "MAIN_ORIGINAL=1\n",
        "declining overwrite must leave main's copy untouched"
    );
}

/// Candidate exists in main → overwrite CONFIRMED replaces main's copy.
#[test]
fn copy_overwrite_confirmed_replaces_main() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    write(main.path(), "config.env", "MAIN_ORIGINAL=1\n");
    write(&wt, "config.env", "WORKTREE_NEW=1\n");

    let outcome = copy_candidate_into_main(&wt, main.path(), Path::new("config.env"), |_| {
        Ok(true)
    })
    .unwrap();
    assert!(matches!(outcome, CopyOutcome::Copied { .. }));

    let after = fs::read_to_string(main.path().join("config.env")).unwrap();
    assert_eq!(after, "WORKTREE_NEW=1\n", "confirmed overwrite must replace");
}

/// magic.local.json flows through the same copy path and lands gitignored.
#[test]
fn magic_local_json_lands_gitignored_in_main() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    // Worktree gitignores magic.local.json (the canonical bootstrap rule).
    write(&wt, ".gitignore", ".superset/magic.local.json\n");
    git_run(&["add", ".gitignore"], &wt);
    write(&wt, ".superset/magic.local.json", "{\"files\":[]}\n");

    let outcome = copy_candidate_into_main(
        &wt,
        main.path(),
        Path::new(".superset/magic.local.json"),
        |_| Ok(true),
    )
    .unwrap();
    assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

    assert!(main.path().join(".superset/magic.local.json").is_file());
    assert!(
        git::is_ignored(main.path(), Path::new(".superset/magic.local.json")).unwrap(),
        "magic.local.json must be gitignored in main after copy"
    );
}

/// Path-safety: an absolute or `..`-bearing rel is rejected before any
/// filesystem mutation.
#[test]
fn copy_rejects_unsafe_paths() {
    let main = init_main_repo();
    let (_wt, wt) = make_worktree(main.path());

    let err =
        copy_candidate_into_main(&wt, main.path(), Path::new("../escape.env"), |_| Ok(true))
            .unwrap_err();
    assert!(
        format!("{err:#}").contains("unsafe path"),
        "must reject `..` paths; got: {err:#}"
    );

    let err =
        copy_candidate_into_main(&wt, main.path(), Path::new("/etc/passwd"), |_| Ok(true))
            .unwrap_err();
    assert!(
        format!("{err:#}").contains("unsafe path"),
        "must reject absolute paths; got: {err:#}"
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
