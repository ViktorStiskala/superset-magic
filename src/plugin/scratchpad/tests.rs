use super::*;
use crate::tests::support::{exit_code_to_u8, git_run, init_main_repo, make_worktree};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use tempfile::TempDir;

/// A repo whose committed `.gitignore` carries `body` and nothing else, so a
/// test can assert exactly what the bootstrap appended (or, more often, did
/// not). The returned root is canonicalized to match what `git rev-parse
/// --show-toplevel` reports — on macOS a tempdir's `/var/...` path and its
/// physical `/private/var/...` form are different strings.
fn repo_with_gitignore(body: &str) -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    fs::write(dir.path().join(".gitignore"), body).unwrap();
    git_run(&["add", ".gitignore"], dir.path());
    git_run(&["commit", "-q", "-m", "gitignore"], dir.path());
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

/// A repo that already ignores the state tree — the ordinary case, after
/// `init`/`migrate` or `plugin enable` has run.
fn ignored_repo() -> (TempDir, PathBuf) {
    repo_with_gitignore("target/\n.superset/.magic/\n")
}

/// The session directory `ensure` will resolve for `root`.
fn session_rel(root: &Path) -> String {
    let slug = identity::resolve(root).unwrap().slug;
    format!("{STATE_REL}/{SESSIONS_DIR}/{slug}")
}

// ── R63 / R40: nothing is written until git reports the tree ignored ─────────

/// AE24 + AE45. A repo that has never run `init`/`migrate`/`plugin enable`
/// has no rule for the tree, so the bootstrap writes nothing at all and says
/// why. Crucially it does NOT fix the problem itself: the tracked `.gitignore`
/// comes back byte-identical, because no hook path may ever dirty the user's
/// working tree (R40).
#[test]
fn refuses_and_writes_nothing_while_the_tree_is_not_ignored() {
    let (_dir, root) = repo_with_gitignore("target/\n");
    let before = fs::read(root.join(".gitignore")).unwrap();

    let report = ensure(&root).unwrap();

    assert!(!report.wrote_state);
    assert!(report.created.is_empty(), "{:?}", report.created);
    assert!(!root.join(STATE_REL).exists());
    assert_eq!(fs::read(root.join(".gitignore")).unwrap(), before);
    assert_eq!(report.refusals.len(), 1);
    assert_eq!(report.refusals[0].code(), "not-ignored");
    let note = report.heartbeat_note();
    assert!(note.contains(".superset/.magic/"), "{note}");

    // The one writer of the rule — `init`/`migrate` reach it through
    // `migrate::ensure_bootstrap_gitignores`, `plugin enable` calls it
    // directly. Exactly one line joins the file.
    ensure_state_ignored(&root).unwrap();

    let before = String::from_utf8(before).unwrap();
    let after = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(after.lines().count(), before.lines().count() + 1);
    assert!(after.lines().any(|l| l == ".superset/.magic/"), "{after}");

    let report = ensure(&root).unwrap();
    assert!(report.wrote_state);
    assert!(report.refusals.is_empty(), "{:?}", report.refusals);
}

/// AE46. A repo that already carries `.superset/.gitignore` owns that
/// subtree, so the rule lands there — anchored to that file's own directory —
/// rather than leaking up to the repository root. Either way git reports the
/// tree ignored, which is the actual contract.
#[test]
fn the_rule_lands_in_a_nested_superset_gitignore() {
    let (_dir, root) = repo_with_gitignore("target/\n");
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(root.join(".superset/.gitignore"), "backups/\n").unwrap();
    git_run(&["add", "-f", ".superset/.gitignore"], &root);
    git_run(&["commit", "-q", "-m", "nested gitignore"], &root);
    let root_gitignore = fs::read(root.join(".gitignore")).unwrap();

    ensure_state_ignored(&root).unwrap();

    let nested = fs::read_to_string(root.join(".superset/.gitignore")).unwrap();
    assert!(nested.lines().any(|l| l == "/.magic/"), "{nested}");
    assert_eq!(fs::read(root.join(".gitignore")).unwrap(), root_gitignore);
    assert!(ensure(&root).unwrap().wrote_state);
}

/// The human verb reports a refusal as a non-zero exit rather than a success
/// with nothing done — a hook fails open, a typed command does not.
#[test]
fn the_verb_exits_non_zero_on_a_refusal() {
    let (_dir, root) = repo_with_gitignore("target/\n");
    assert_eq!(exit_code_to_u8(run_ensure(&root).unwrap()), 1);
}

// ── AE19: no repository, no identity ────────────────────────────────────────

/// AE19 (human half). Outside a git repository there is no worktree to hold
/// state and no `<repo>-<branch>` to name a session, so the verb fails loudly.
#[test]
fn ensure_outside_a_git_repository_is_an_error() {
    let outside = tempfile::tempdir().unwrap();
    assert!(ensure(outside.path()).is_err());
    assert_ne!(exit_code_to_u8(run_ensure(outside.path()).unwrap()), 0);
}

// ── The happy path ──────────────────────────────────────────────────────────

/// The full layout appears, and `OPERATOR-CHECKLIST.md` is part of it — the
/// pointer `SessionStart` injects has to resolve to a file that exists.
#[test]
fn ensure_scaffolds_the_whole_layout() {
    let (_dir, root) = ignored_repo();

    let report = ensure(&root).unwrap();

    assert!(report.wrote_state);
    assert!(report.refusals.is_empty(), "{:?}", report.refusals);
    let session = root.join(session_rel(&root));
    for name in STATE_FILES {
        assert!(session.join(name).is_file(), "missing {name}");
    }
    assert!(session.join("OPERATOR-CHECKLIST.md").is_file());
    assert!(root.join(STATE_REL).join(README_NAME).is_file());
    assert!(root.join(STATE_REL).join(POINTER_NAME).is_file());
    for dir in CLAIM_DIRS {
        assert!(root.join(STATE_REL).join(dir).is_dir(), "missing {dir}/");
    }
}

/// R16. The active session is recorded as a plain JSON file, readable back as
/// the same identity the bootstrap resolved.
#[test]
fn the_pointer_is_plain_json_naming_the_session() {
    let (_dir, root) = ignored_repo();
    let identity = identity::resolve(&root).unwrap();

    ensure(&root).unwrap();

    let pointer_path = root.join(STATE_REL).join(POINTER_NAME);
    assert!(!fs::symlink_metadata(&pointer_path).unwrap().is_symlink());
    let pointer: Pointer =
        serde_json::from_str(&fs::read_to_string(&pointer_path).unwrap()).unwrap();
    assert_eq!(pointer.slug, identity.slug);
    assert_eq!(pointer.repo, identity.repo);
    assert_eq!(pointer.branch, identity.branch);
    assert_eq!(pointer.dir, session_rel(&root));
    assert!(pointer.resolved_at.ends_with('Z'), "{}", pointer.resolved_at);
}

/// A linked worktree resolves its own branch, so two worktrees of one repo
/// keep separate session directories.
#[test]
fn a_linked_worktree_gets_its_own_session_directory() {
    let (_dir, main_root) = ignored_repo();
    let (_wt, wt_root) = make_worktree(&main_root);

    let report = ensure(&wt_root).unwrap();

    assert!(report.wrote_state);
    assert!(report.slug.ends_with("-feature-sync-flow-test"), "{}", report.slug);
    assert!(report.session_dir.starts_with(&wt_root));
    assert!(report.session_dir.join("STATUS.md").is_file());
}

/// R2 stays honest with a populated tree: every path the bootstrap creates is
/// one the sync and pack enumeration layers refuse to yield.
#[test]
fn everything_created_stays_invisible_to_sync_and_pack() {
    let (_dir, root) = ignored_repo();

    let report = ensure(&root).unwrap();

    assert!(!report.created.is_empty());
    for rel in &report.created {
        assert!(
            crate::sync::under_excluded_tree(Path::new(rel)),
            "{rel} would be synced or packed"
        );
    }
}

/// R58. The tree is owner-only, while `.superset` itself — committed content —
/// keeps whatever permissions it already had.
#[test]
fn the_state_tree_is_created_owner_only() {
    let (_dir, root) = ignored_repo();

    ensure(&root).unwrap();

    let state_root = root.join(STATE_REL);
    for dir in [state_root.clone(), state_root.join(SESSIONS_DIR)] {
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, DIR_MODE, "{} is {mode:o}", dir.display());
    }
    let file_mode = fs::metadata(state_root.join(README_NAME))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, FILE_MODE, "{file_mode:o}");
}

// ── R17: scaffold, never rewrite ────────────────────────────────────────────

/// The model owns the content of every state file. A re-run leaves an edited
/// file byte-for-byte alone and only fills in what is genuinely missing.
#[test]
fn an_existing_state_file_survives_and_a_missing_sibling_is_scaffolded() {
    let (_dir, root) = ignored_repo();
    ensure(&root).unwrap();
    let session = root.join(session_rel(&root));
    fs::write(session.join("STATUS.md"), "## CURRENT STATE - mine\n").unwrap();
    fs::remove_file(session.join("TASKS.md")).unwrap();

    let report = ensure(&root).unwrap();

    assert_eq!(
        fs::read_to_string(session.join("STATUS.md")).unwrap(),
        "## CURRENT STATE - mine\n"
    );
    assert!(session.join("TASKS.md").is_file());
    let created: Vec<&str> = report.created.iter().map(String::as_str).collect();
    assert_eq!(created, vec![format!("{}/TASKS.md", session_rel(&root))]);
}

/// R17's fresh-clone case. A public repository can commit a file at the very
/// path the slug predicts; adopting it would hand the model someone else's
/// text as its own prior working memory. The planted file is left exactly as
/// committed, named in the report, and its untracked siblings are still
/// scaffolded around it.
#[test]
fn a_tracked_state_file_is_never_adopted() {
    let (_dir, root) = ignored_repo();
    let rel = format!("{}/STATUS.md", session_rel(&root));
    let planted = root.join(&rel);
    fs::create_dir_all(planted.parent().unwrap()).unwrap();
    fs::write(&planted, "planted by the repository\n").unwrap();
    git_run(&["add", "-f", &rel], &root);
    git_run(&["commit", "-q", "-m", "planted state"], &root);

    let report = ensure(&root).unwrap();

    assert!(report.wrote_state);
    assert_eq!(
        fs::read_to_string(&planted).unwrap(),
        "planted by the repository\n"
    );
    assert!(!report.created.iter().any(|c| c == &rel), "{:?}", report.created);
    assert_eq!(report.refusals.len(), 1);
    assert_eq!(report.refusals[0].code(), "tracked-paths");
    let note = report.heartbeat_note();
    assert!(note.contains(&rel), "{note}");
    assert!(root.join(format!("{}/TASKS.md", session_rel(&root))).is_file());
}

// ── R56: containment ────────────────────────────────────────────────────────

/// AE43. The state root is a symlink out of the worktree. Following it would
/// scatter private state into a directory the user never pointed at, so the
/// whole run is refused before anything is created — the target stays empty.
#[test]
fn a_state_root_symlinked_out_of_the_worktree_is_refused() {
    let (_dir, root) = ignored_repo();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.join(SUPERSET_REL)).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join(STATE_REL)).unwrap();

    let report = ensure(&root).unwrap();

    assert!(!report.wrote_state);
    assert!(report.created.is_empty());
    assert_eq!(report.refusals.len(), 1);
    assert_eq!(report.refusals[0].code(), "escapes-worktree");
    assert!(report.heartbeat_note().contains(STATE_REL));
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}

/// A symlinked state root is refused even when its target stays inside the
/// worktree. It passes the containment check, but git flatly refuses to
/// resolve a pathspec that crosses a symlink (`fatal: pathspec … is beyond a
/// symbolic link`), so the ignore question cannot be answered — and an
/// unanswered ignore question is not permission to write (R63).
#[test]
fn a_state_root_symlinked_inside_the_worktree_is_still_unignorable() {
    let (_dir, root) = ignored_repo();
    let real = root.join(".magic-store");
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(root.join(SUPERSET_REL)).unwrap();
    std::os::unix::fs::symlink(&real, root.join(STATE_REL)).unwrap();

    let report = ensure(&root).unwrap();

    assert!(!report.wrote_state);
    assert_eq!(report.refusals[0].code(), "not-ignored");
    assert!(fs::read_dir(&real).unwrap().next().is_none());
}

/// A regular file squatting on the state root is reported, not deleted — it
/// is not ours, and whatever wrote it may want it back.
#[test]
fn a_regular_file_where_the_state_root_belongs_is_refused() {
    let (_dir, root) = ignored_repo();
    fs::create_dir_all(root.join(SUPERSET_REL)).unwrap();
    fs::write(root.join(STATE_REL), "not a directory\n").unwrap();

    let report = ensure(&root).unwrap();

    assert!(!report.wrote_state);
    assert_eq!(report.refusals[0].code(), "not-a-directory");
    assert_eq!(
        fs::read_to_string(root.join(STATE_REL)).unwrap(),
        "not a directory\n"
    );
}

// ── R56 × `wrote_state`: a late refusal must clear the flag (regression) ─────
//
// The three tests below guard one property that is easy to get wrong and
// expensive when it is: `Report::wrote_state` must be FALSE after any
// containment refusal, including the ones raised late.
//
// `ensure` sets `wrote_state = true` once the R63 ignore probe and the R17
// tracked probe have passed, because from that point on it does start
// creating things. Three containment checks (R56) come AFTER that line: the
// directory loop, `scaffold`, and the pointer. Each of those used to push a
// `Refusal` and leave the flag as it found it — true.
//
// That flag is the whole contract with callers. `bypass`, `cache`,
// `expect-artifact`, `subagent_stop` and `pre_compact` all branch on
// `!report.wrote_state` to decide whether to stop; with the flag still true
// they printed the refusal as a mere warning and then wrote their own file
// into `report.state_root` anyway — straight through the symlink R56 had just
// refused to follow. Planting `.superset/.magic/bypass` as a symlink out of
// the worktree was enough to make `ss-magic plugin bypass <file>` deposit its
// claim outside the repository.
//
// The trap for a future reader: the two long-standing containment tests above
// (`a_state_root_symlinked_*`) both refuse EARLY, before the flag is ever set,
// so they pass whether or not the late sites clear it. A test that plants a
// symlink and only checks that a refusal was reported passes with the bug
// present too — the refusal was always reported. `wrote_state` is the
// assertion that bites, so each test below asserts it FIRST and by name.

/// A directory somewhere outside any worktree, for a symlink to escape into.
fn outside_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Site 1 of 3 — the directory loop. `.superset/.magic/bypass` is a symlink
/// out of the worktree, which is the exact shape that turned into a real
/// escape: the claim directories are what `bypass` and the conclusion cache
/// write into.
#[test]
fn a_claim_directory_symlinked_out_of_the_worktree_clears_wrote_state() {
    let (_dir, root) = ignored_repo();
    let outside = outside_dir();
    // A real state root, so the R63 ignore probe and the R17 tracked probe
    // both answer and `wrote_state` is genuinely set to true before the loop
    // reaches the symlink.
    fs::create_dir_all(root.join(STATE_REL)).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join(STATE_REL).join("bypass")).unwrap();

    let report = ensure(&root).unwrap();

    assert!(
        !report.wrote_state,
        "wrote_state stayed true after a containment refusal in the directory \
         loop; every caller reads it as `the state tree is safe to write into` \
         and would now write through the escaping symlink"
    );
    assert_eq!(report.refusals.len(), 1, "{:?}", report.refusals);
    assert_eq!(report.refusals[0].code(), "escapes-worktree");
    assert!(
        fs::read_dir(outside.path()).unwrap().next().is_none(),
        "the bootstrap wrote through the symlink into {}",
        outside.path().display()
    );
}

/// Site 2 of 3 — the pointer. `current.json` is the one file rewritten on
/// every run, so a symlink there is a standing write primitive rather than a
/// one-off.
#[test]
fn a_pointer_symlinked_out_of_the_worktree_clears_wrote_state() {
    let (_dir, root) = ignored_repo();
    let outside = outside_dir();
    let planted = outside.path().join("stolen.json");
    fs::write(&planted, "not ours\n").unwrap();
    fs::create_dir_all(root.join(STATE_REL)).unwrap();
    std::os::unix::fs::symlink(&planted, root.join(STATE_REL).join(POINTER_NAME)).unwrap();

    let report = ensure(&root).unwrap();

    assert!(
        !report.wrote_state,
        "wrote_state stayed true after the pointer's containment refusal"
    );
    assert_eq!(report.refusals.len(), 1, "{:?}", report.refusals);
    assert_eq!(report.refusals[0].code(), "escapes-worktree");
    assert_eq!(fs::read_to_string(&planted).unwrap(), "not ours\n");
}

/// Site 3 of 3 — `scaffold`, which is the odd one out: it does NOT return.
/// One escaping state file is refused and the run carries on creating that
/// file's siblings, so the flag is the only thing left telling the caller the
/// tree is not trustworthy. Nothing about the run's shape says so.
#[test]
fn a_state_file_symlinked_out_of_the_worktree_clears_wrote_state_and_scaffolding_continues() {
    let (_dir, root) = ignored_repo();
    let outside = outside_dir();
    // Dangling on purpose: `create_new` follows a symlink, so without the
    // containment check this scaffold would CREATE the file outside the
    // worktree rather than merely failing on an existing one.
    let escape = outside.path().join("STATUS.md");
    let session = root.join(session_rel(&root));
    fs::create_dir_all(&session).unwrap();
    std::os::unix::fs::symlink(&escape, session.join("STATUS.md")).unwrap();

    let report = ensure(&root).unwrap();

    assert!(
        !report.wrote_state,
        "wrote_state stayed true after `scaffold` refused an escaping state \
         file; the run does not return here, so the flag is the caller's only \
         signal that the tree is unsafe"
    );
    assert_eq!(report.refusals.len(), 1, "{:?}", report.refusals);
    assert_eq!(report.refusals[0].code(), "escapes-worktree");
    assert!(!escape.exists(), "the scaffold wrote outside the worktree");
    // The sibling that follows STATUS.md in the same loop is still created —
    // this is what makes the flag load-bearing rather than redundant with an
    // early return.
    assert!(session.join("TASKS.md").is_file());
}

// ── R48: the pointer claim ──────────────────────────────────────────────────

/// Two bootstraps racing each other must not interleave into a half-written
/// pointer. Both write the same content here, so what the test actually pins
/// is that the file always parses and always names the resolved session.
#[test]
fn concurrent_ensures_produce_one_coherent_pointer() {
    let (_dir, root) = ignored_repo();
    let identity = identity::resolve(&root).unwrap();

    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| ensure(&root).unwrap());
        }
    });

    let body = fs::read_to_string(root.join(STATE_REL).join(POINTER_NAME)).unwrap();
    let pointer: Pointer = serde_json::from_str(&body).unwrap();
    assert_eq!(pointer.slug, identity.slug);
}

/// `flock` is released by the kernel when a process dies, so a lock FILE left
/// behind by a crash is inert — the next run takes the claim without waiting.
#[test]
fn a_lock_file_left_by_a_crashed_process_does_not_block() {
    let (_dir, root) = ignored_repo();
    let state_root = root.join(STATE_REL);
    fs::create_dir_all(&state_root).unwrap();
    fs::write(state_root.join(POINTER_LOCK_NAME), "").unwrap();

    let report = ensure(&root).unwrap();

    assert!(report.wrote_state, "{:?}", report.refusals);
    assert!(state_root.join(POINTER_NAME).is_file());
}

/// The pointer is refreshed on every run, since it records which session is
/// current rather than content the model owns.
#[test]
fn the_pointer_is_rewritten_on_a_re_run() {
    let (_dir, root) = ignored_repo();
    ensure(&root).unwrap();
    let pointer_path = root.join(STATE_REL).join(POINTER_NAME);
    fs::write(&pointer_path, "{}\n").unwrap();

    let report = ensure(&root).unwrap();

    let pointer: Pointer =
        serde_json::from_str(&fs::read_to_string(&pointer_path).unwrap()).unwrap();
    assert_eq!(pointer.slug, report.slug);
    // Rewriting an existing pointer is not a creation.
    assert!(report.created.is_empty(), "{:?}", report.created);
}

// ── I/O failure ─────────────────────────────────────────────────────────────

/// A session directory that cannot be created is a real error, surfaced with
/// the path rather than silently swallowed.
#[test]
fn an_unwritable_sessions_directory_surfaces_the_error() {
    let (_dir, root) = ignored_repo();
    ensure(&root).unwrap();
    let session = root.join(session_rel(&root));
    let sessions = session.parent().unwrap().to_path_buf();
    fs::remove_dir_all(&session).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o500)).unwrap();

    let err = ensure(&root).unwrap_err();

    assert!(format!("{err:#}").contains("sessions"), "{err:#}");
    // Restore write permission so the tempdir can be cleaned up.
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
}

// ── Pure helpers ────────────────────────────────────────────────────────────

/// The epoch itself, and a date past several leap years, formatted the way a
/// machine reader expects.
#[test]
fn rfc3339_formatting() {
    assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_rfc3339(1_772_236_800), "2026-02-28T00:00:00Z");
    assert_eq!(format_rfc3339(1_772_323_199), "2026-02-28T23:59:59Z");
}

/// A clean run says so; a refusal says what it refused, in one line.
#[test]
fn heartbeat_note_shape() {
    let mut report = Report {
        state_root: PathBuf::from("/x/.superset/.magic"),
        slug: "repo-branch".to_string(),
        session_dir: PathBuf::from("/x/.superset/.magic/sessions/repo-branch"),
        created: vec!["a".to_string()],
        refusals: Vec::new(),
        wrote_state: true,
    };
    assert_eq!(report.heartbeat_note(), "scratchpad ready (1 created)");

    report.refusals.push(Refusal::TrackedPaths {
        paths: vec!["p".to_string()],
    });
    assert_eq!(
        report.heartbeat_note(),
        "scratchpad partial: refused to adopt tracked path(s): p"
    );

    report.wrote_state = false;
    assert!(report.heartbeat_note().starts_with("scratchpad refused: "));
}

