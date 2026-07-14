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
