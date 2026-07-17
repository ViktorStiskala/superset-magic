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
/// path — so reverse sync copies the broad rule into main.
#[test]
fn covering_rule_returns_glob_not_literal() {
    let dir = fresh();
    git_init(dir.path());
    fs::write(dir.path().join(".gitignore"), "**/.dev.vars\n").unwrap();

    let got =
        find_covering_rule(dir.path(), Path::new("apps/api/.dev.vars")).unwrap();
    assert_eq!(got, Some("**/.dev.vars".to_string()));
}

/// No covering rule → None → caller falls back to the literal path.
#[test]
fn covering_rule_none_when_uncovered() {
    let dir = fresh();
    git_init(dir.path());
    fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

    let got =
        find_covering_rule(dir.path(), Path::new("apps/api/.dev.vars")).unwrap();
    assert_eq!(got, None);
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
