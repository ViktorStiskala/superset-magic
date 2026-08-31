use super::*;
use std::fs;
use tempfile::TempDir;

fn fresh() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// .gitignore absent → created containing exactly the entry + trailing NL.
#[test]
fn creates_file_when_absent() {
    let dir = fresh();
    ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

    let got = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(got, ".superset/magic.local.json\n");
}

/// Entry already present → file is byte-identical (no modification).
#[test]
fn idempotent_when_entry_present() {
    let dir = fresh();
    let gi = dir.path().join(".gitignore");
    let initial = "# auto-generated\n.superset/magic.local.json\nnode_modules/\n";
    fs::write(&gi, initial).unwrap();

    ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

    let after = fs::read_to_string(&gi).unwrap();
    assert_eq!(after, initial, "file must be byte-identical");
}

/// Entry absent among other lines → appended; existing lines untouched.
#[test]
fn appends_when_entry_absent_among_others() {
    let dir = fresh();
    let gi = dir.path().join(".gitignore");
    let initial = "# keep\nnode_modules/\n.env\n";
    fs::write(&gi, initial).unwrap();

    ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

    let after = fs::read_to_string(&gi).unwrap();
    // Existing lines must still be there.
    assert!(after.starts_with(initial), "existing content must be preserved at the start");
    // The new entry must appear at the end.
    assert!(
        after.ends_with(".superset/magic.local.json\n"),
        "new entry must be appended with trailing newline; got: {after:?}"
    );
}

/// File missing trailing newline → newline inserted before the entry.
#[test]
fn inserts_newline_when_file_lacks_trailing_newline() {
    let dir = fresh();
    let gi = dir.path().join(".gitignore");
    // No trailing newline.
    fs::write(&gi, "node_modules/").unwrap();

    ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

    let after = fs::read_to_string(&gi).unwrap();
    assert_eq!(after, "node_modules/\n.superset/magic.local.json\n");
}

/// Empty file → entry appended normally.
#[test]
fn handles_empty_file() {
    let dir = fresh();
    let gi = dir.path().join(".gitignore");
    fs::write(&gi, "").unwrap();

    ensure_entry(dir.path(), "secret.txt").unwrap();

    let after = fs::read_to_string(&gi).unwrap();
    assert_eq!(after, "secret.txt\n");
}

/// Partial match (line is a prefix of an existing entry) is not treated
/// as "already present" — the entry must be exact.
#[test]
fn partial_match_is_not_exact_match() {
    let dir = fresh();
    let gi = dir.path().join(".gitignore");
    fs::write(&gi, ".superset/magic.local.json.bak\n").unwrap();

    ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

    let after = fs::read_to_string(&gi).unwrap();
    assert!(
        after.contains(".superset/magic.local.json\n"),
        "entry must be appended; got: {after:?}"
    );
}

// ── find_covering_rule (U11) ─────────────────────────────────────────────

fn git_init(root: &Path) {
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git init failed in {}", root.display());
    // Don't let the dev's global `.env`/`.dev.vars` ignore leak into these
    // check-ignore assertions — each test repo owns its ignore truth.
    crate::tests::support::neutralize_global_excludes(root);
}

/// A glob rule covering the path is returned as the glob, NOT the literal
/// path — so reverse sync copies the broad rule into main — paired with the
/// (repo-root) directory that owns it.
#[test]
fn covering_rule_returns_glob_not_literal() {
    let dir = fresh();
    git_init(dir.path());
    fs::write(dir.path().join(".gitignore"), "**/.dev.vars\n").unwrap();

    let got = find_covering_rule(dir.path(), Path::new("apps/api/.dev.vars"))
        .unwrap()
        .expect("a covering rule");
    assert_eq!(got.pattern, "**/.dev.vars");
    assert_eq!(got.source_dir.as_deref(), Some(Path::new("")));
}

/// A glob rule owned by a NESTED `.gitignore` reports that nested directory
/// as its `source_dir` — the piece [`ensure_path_ignored`] needs to detect a
/// scope mismatch (R1).
#[test]
fn covering_rule_reports_owning_nested_directory() {
    let dir = fresh();
    git_init(dir.path());
    fs::create_dir_all(dir.path().join("apps/api")).unwrap();
    fs::write(dir.path().join("apps/api/.gitignore"), "*.log\n").unwrap();

    let got = find_covering_rule(dir.path(), Path::new("apps/api/debug.log"))
        .unwrap()
        .expect("a covering rule");
    assert_eq!(got.pattern, "*.log");
    assert_eq!(got.source_dir.as_deref(), Some(Path::new("apps/api")));
}

/// No covering rule → None → caller falls back to the literal path.
#[test]
fn covering_rule_none_when_uncovered() {
    let dir = fresh();
    git_init(dir.path());
    fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

    let got = find_covering_rule(dir.path(), Path::new("apps/api/.dev.vars")).unwrap();
    assert!(got.is_none());
}

// ── ensure_path_ignored (Task 5) ─────────────────────────────────────────

/// Already-ignored path → `Ignored::Already`, and the `.gitignore` is left
/// byte-identical — no rewrite is attempted once git confirms coverage.
#[test]
fn ensure_path_ignored_noop_when_already_ignored() {
    let dir = fresh();
    git_init(dir.path());
    let gi = dir.path().join(".gitignore");
    let initial = "secret.txt\n";
    fs::write(&gi, initial).unwrap();

    let got =
        ensure_path_ignored(dir.path(), dir.path(), Path::new("secret.txt"), PathKind::File)
            .unwrap();
    assert_eq!(got, Ignored::Already);

    let after = fs::read_to_string(&gi).unwrap();
    assert_eq!(after, initial, ".gitignore must be byte-identical when already covered");
}

/// Uncovered file → an anchored literal is appended to the root `.gitignore`
/// and git now ignores the path.
#[test]
fn ensure_path_ignored_appends_literal_when_uncovered() {
    let dir = fresh();
    git_init(dir.path());

    let got =
        ensure_path_ignored(dir.path(), dir.path(), Path::new("secret.txt"), PathKind::File)
            .unwrap();
    assert_eq!(got, Ignored::Appended);

    let after = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(after, "secret.txt\n");
    assert!(
        git::is_ignored_str(dir.path(), "secret.txt").unwrap(),
        "git must now ignore secret.txt"
    );
}

/// A `Dir` rule for a directory that doesn't exist on disk yet is still
/// queried and written with a trailing slash, and git honors it.
#[test]
fn ensure_path_ignored_dir_kind_ignores_backups_before_dir_exists() {
    let dir = fresh();
    git_init(dir.path());
    assert!(!dir.path().join(".superset/backups").exists());

    let got = ensure_path_ignored(
        dir.path(),
        dir.path(),
        Path::new(".superset/backups"),
        PathKind::Dir,
    )
    .unwrap();
    assert_eq!(got, Ignored::Appended);

    let after = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(after, ".superset/backups/\n");
    assert!(
        git::is_ignored_str(dir.path(), ".superset/backups/").unwrap(),
        "git must ignore the backups dir even before it exists on disk"
    );
}

/// A broader `.superset/` rule already covers the nested backups dir → noop,
/// no new line is written.
#[test]
fn ensure_path_ignored_dir_kind_noop_when_broader_rule_covers() {
    let dir = fresh();
    git_init(dir.path());
    let gi = dir.path().join(".gitignore");
    let initial = ".superset/\n";
    fs::write(&gi, initial).unwrap();

    let got = ensure_path_ignored(
        dir.path(),
        dir.path(),
        Path::new(".superset/backups"),
        PathKind::Dir,
    )
    .unwrap();
    assert_eq!(got, Ignored::Already);

    let after = fs::read_to_string(&gi).unwrap();
    assert_eq!(after, initial, "the broader rule already covers it; no line should be added");
}

/// A non-git root (e.g. a unit-test tempdir) never errors: the "already
/// ignored?" probe degrades to `None` and the literal is appended anyway.
#[test]
fn ensure_path_ignored_tolerates_non_git_root() {
    let dir = fresh();

    let got =
        ensure_path_ignored(dir.path(), dir.path(), Path::new("secret.txt"), PathKind::File)
            .unwrap();
    assert_eq!(got, Ignored::Appended);

    let after = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(after, "secret.txt\n");
}

/// A rule for a nested path lands in the closest EXISTING `.gitignore` among
/// its ancestors, not the repo root — and a sibling directory is unaffected.
#[test]
fn ensure_path_ignored_places_rule_in_nested_gitignore() {
    let dir = fresh();
    git_init(dir.path());
    fs::create_dir_all(dir.path().join("apps/api")).unwrap();
    fs::create_dir_all(dir.path().join("apps/api2")).unwrap();
    fs::write(dir.path().join("apps/api/.gitignore"), "node_modules/\n").unwrap();

    let got = ensure_path_ignored(
        dir.path(),
        dir.path(),
        Path::new("apps/api/.env"),
        PathKind::File,
    )
    .unwrap();
    assert_eq!(got, Ignored::Appended);

    let nested = fs::read_to_string(dir.path().join("apps/api/.gitignore")).unwrap();
    assert_eq!(nested, "node_modules/\n/.env\n");
    assert!(
        !dir.path().join(".gitignore").exists(),
        "the rule must not leak into the repo-root .gitignore"
    );
    assert!(git::is_ignored_str(dir.path(), "apps/api/.env").unwrap());
    assert!(
        !git::is_ignored_str(dir.path(), "apps/api2/.env").unwrap(),
        "the nested rule must not leak to a sibling directory"
    );
}

/// The cross-root covering-glob branch (the shape `ensure_gitignored_in_main`
/// uses in production: worktree source, main target): a broad rule resolved from
/// `rule_source_root`'s .gitignore is written into `target_root`'s .gitignore
/// (verified to actually ignore the path), NOT the anchored literal.
#[test]
fn ensure_path_ignored_prefers_covering_glob_from_source_root() {
    let source = fresh();
    let target = fresh();
    git_init(source.path());
    git_init(target.path());
    // Source already ignores the secret via a broad glob; target has no rule.
    fs::write(source.path().join(".gitignore"), "**/.dev.vars\n").unwrap();

    let rel = Path::new("apps/api/.dev.vars");
    let outcome = ensure_path_ignored(target.path(), source.path(), rel, PathKind::File).unwrap();
    assert_eq!(outcome, Ignored::Appended);

    let gi = fs::read_to_string(target.path().join(".gitignore")).unwrap();
    assert!(
        gi.lines().any(|l| l == "**/.dev.vars"),
        "the covering glob must be written into the target, got: {gi:?}"
    );
    assert!(
        !gi.contains("apps/api/.dev.vars"),
        "the anchored literal must NOT be used when a glob covers it: {gi:?}"
    );
    assert!(
        git::is_ignored_str(target.path(), "apps/api/.dev.vars").unwrap(),
        "the target must now ignore the secret"
    );
}

// ── covering-rule re-anchor fix (U1 / R1) ────────────────────────────────

/// Reproduces the live three-command defect (R1 / AE13): a NESTED
/// `.gitignore` covers the reverse-synced file in the source tree with a bare
/// `*` — the shape `.scratchpad/.gitignore` already takes in Superset
/// worktrees (a fixture stand-in; the plugin itself never creates such a
/// file). The target tree has no `.gitignore` at that same nested directory,
/// so the only place the OLD code could reuse the pattern was the target's
/// closest EXISTING `.gitignore` — here, none exists at all, so it would land
/// at the target repo root, turning "ignore everything under `.scratchpad/`"
/// into "ignore the entire repo". The fix must recognize the scope mismatch
/// and fall through to an anchored literal instead.
#[test]
fn ensure_path_ignored_does_not_reanchor_nested_wildcard_at_broader_scope() {
    let source = fresh();
    let target = fresh();
    git_init(source.path());
    git_init(target.path());

    // Source's `.scratchpad/` subtree is entirely self-ignored via a nested
    // `.gitignore` containing a bare `*`.
    fs::create_dir_all(source.path().join(".scratchpad")).unwrap();
    fs::write(source.path().join(".scratchpad/.gitignore"), "*\n").unwrap();

    let rel = Path::new(".scratchpad/notes.txt");
    let outcome = ensure_path_ignored(target.path(), source.path(), rel, PathKind::File).unwrap();
    assert_eq!(outcome, Ignored::Appended);

    // The target root .gitignore (created or not) must never carry the bare
    // `*` verbatim — that would ignore the whole repo.
    let root_gi_path = target.path().join(".gitignore");
    if root_gi_path.exists() {
        let root_gi = fs::read_to_string(&root_gi_path).unwrap();
        assert!(
            !root_gi.lines().any(|l| l == "*"),
            "the target root .gitignore must never gain a bare `*`; got: {root_gi:?}"
        );
    }
    assert!(
        !target.path().join(".scratchpad/.gitignore").exists(),
        "the fix must not fabricate a nested .gitignore just to host the reused rule"
    );
    // The file must still end up ignored — via the anchored-literal fallback.
    assert!(
        git::is_ignored_str(target.path(), ".scratchpad/notes.txt").unwrap(),
        "the target must still ignore the file, via the anchored literal fallback"
    );
}

/// AE46 shape (R40): the repo already carries its own nested `.superset/
/// .gitignore`. A same-root `Dir` rule for a path under it must land in that
/// closer file, not the repository root — and this placement must survive
/// the R1 fix (i.e. the fix must not become so conservative that it starts
/// mis-routing plain anchored-literal placement too).
#[test]
fn ensure_path_ignored_ae46_lands_in_existing_nested_gitignore_not_root() {
    let dir = fresh();
    git_init(dir.path());
    fs::create_dir_all(dir.path().join(".superset")).unwrap();
    fs::write(dir.path().join(".superset/.gitignore"), "magic.local.json\n").unwrap();

    let got = ensure_path_ignored(
        dir.path(),
        dir.path(),
        Path::new(".superset/backups"),
        PathKind::Dir,
    )
    .unwrap();
    assert_eq!(got, Ignored::Appended);

    let nested = fs::read_to_string(dir.path().join(".superset/.gitignore")).unwrap();
    assert_eq!(nested, "magic.local.json\n/backups/\n");
    assert!(
        !dir.path().join(".gitignore").exists(),
        "the rule must land in the closer nested file, not a newly created repo-root .gitignore"
    );
    assert!(git::is_ignored_str(dir.path(), ".superset/backups/").unwrap());
}

/// When the target tree already has its own `.gitignore` at the SAME
/// relative directory that owns the covering rule in the source tree, the
/// pattern is reused there verbatim (not converted to an anchored literal,
/// and not lifted to the root) — the scopes line up, so this is unaffected
/// by the R1 fix.
#[test]
fn ensure_path_ignored_reuses_covering_rule_when_target_has_matching_nested_gitignore() {
    let source = fresh();
    let target = fresh();
    git_init(source.path());
    git_init(target.path());

    fs::create_dir_all(source.path().join("apps/api")).unwrap();
    fs::write(source.path().join("apps/api/.gitignore"), "*.log\n").unwrap();
    fs::create_dir_all(target.path().join("apps/api")).unwrap();
    fs::write(target.path().join("apps/api/.gitignore"), "node_modules/\n").unwrap();

    let rel = Path::new("apps/api/debug.log");
    let outcome = ensure_path_ignored(target.path(), source.path(), rel, PathKind::File).unwrap();
    assert_eq!(outcome, Ignored::Appended);

    let nested = fs::read_to_string(target.path().join("apps/api/.gitignore")).unwrap();
    assert_eq!(
        nested, "node_modules/\n*.log\n",
        "the covering pattern must be reused in the matching nested file, got: {nested:?}"
    );
    assert!(
        !target.path().join(".gitignore").exists(),
        "the rule must not be lifted to the repo root"
    );
    assert!(git::is_ignored_str(target.path(), "apps/api/debug.log").unwrap());
}
