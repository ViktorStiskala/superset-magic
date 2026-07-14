use super::*;
use std::fs;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn collect(src: &Path, dest: &Path, patterns: &[&str]) -> (Summary, Vec<Event>) {
    let pats: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let mut events = Vec::new();
    let summary = run(src, dest, &pats, |e| events.push(e.clone())).unwrap();
    (summary, events)
}

fn copy_events_of(events: &[Event]) -> Vec<&Path> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Copy { rel } => Some(rel.as_path()),
            _ => None,
        })
        .collect()
}

fn skip_events_of(events: &[Event]) -> Vec<(&SkipReason, &str)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Skip { reason, label } => Some((reason, label.as_str())),
            _ => None,
        })
        .collect()
}

#[test]
fn copies_dotenv_at_root() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".env", "FOO=1\n");
    let (summary, events) = collect(src.path(), dest.path(), &[".env"]);
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.skipped, 0);
    assert!(dest.path().join(".env").is_file());
    assert_eq!(copy_events_of(&events), vec![Path::new(".env")]);
}

#[test]
fn glob_matches_multiple_directories_recursively() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "apps/api/config/a.txt", "a");
    write(src.path(), "apps/web/config/b.txt", "b");
    write(src.path(), "apps/api/other.txt", "c");
    let (summary, _) = collect(src.path(), dest.path(), &["apps/*/config"]);
    assert!(dest.path().join("apps/api/config/a.txt").is_file());
    assert!(dest.path().join("apps/web/config/b.txt").is_file());
    assert!(!dest.path().join("apps/api/other.txt").exists());
    assert_eq!(summary.skipped, 0);
    assert!(summary.copied >= 2);
}

#[test]
fn node_modules_matches_are_dropped() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "apps/api/.dev.vars", "ok");
    write(src.path(), "node_modules/foo/.dev.vars", "drop");
    let (summary, events) = collect(src.path(), dest.path(), &["**/.dev.vars"]);
    assert!(dest.path().join("apps/api/.dev.vars").is_file());
    assert!(!dest.path().join("node_modules/foo/.dev.vars").exists());
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.skipped, 0);
    let skips = skip_events_of(&events);
    assert!(
        skips.iter().any(|(r, _)| matches!(r, SkipReason::Excluded)),
        "expected an Excluded skip event, got: {skips:?}"
    );
}

#[test]
fn glob_with_zero_matches_is_non_fatal_and_uncounted() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let (summary, events) = collect(src.path(), dest.path(), &["**/.env"]);
    assert_eq!(summary.copied, 0);
    assert_eq!(summary.skipped, 0, "no-matches must not count");
    let skips = skip_events_of(&events);
    assert!(skips
        .iter()
        .any(|(r, _)| matches!(r, SkipReason::NoMatches)));
}

#[test]
fn existing_destination_files_are_overwritten() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".env", "NEW=1\n");
    write(dest.path(), ".env", "OLD=1\n");
    let (summary, _) = collect(src.path(), dest.path(), &[".env"]);
    assert_eq!(summary.copied, 1);
    let body = fs::read_to_string(dest.path().join(".env")).unwrap();
    assert_eq!(body, "NEW=1\n");
}

#[test]
fn absolute_pattern_is_rejected_and_counted() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let (summary, events) = collect(src.path(), dest.path(), &["/etc/passwd"]);
    assert_eq!(summary.copied, 0);
    assert_eq!(summary.skipped, 1);
    let skips = skip_events_of(&events);
    assert!(skips
        .iter()
        .any(|(r, _)| matches!(r, SkipReason::AbsolutePathRejected)));
}

#[test]
fn parent_segment_pattern_is_rejected_and_counted() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let (summary, events) = collect(src.path(), dest.path(), &["../oops"]);
    assert_eq!(summary.copied, 0);
    assert_eq!(summary.skipped, 1);
    let skips = skip_events_of(&events);
    assert!(skips
        .iter()
        .any(|(r, _)| matches!(r, SkipReason::ParentSegmentRejected)));
}

#[test]
fn missing_literal_is_counted() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let (summary, events) = collect(src.path(), dest.path(), &[".env"]);
    assert_eq!(summary.copied, 0);
    assert_eq!(summary.skipped, 1);
    let skips = skip_events_of(&events);
    assert!(skips
        .iter()
        .any(|(r, _)| matches!(r, SkipReason::MissingLiteral)));
}

#[test]
fn rejected_patterns_dont_abort_processing() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".env", "x");
    let (summary, _) = collect(src.path(), dest.path(), &["/etc/passwd", ".env"]);
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.skipped, 1);
    assert!(dest.path().join(".env").is_file());
}

// ── Characterization tests: pin engine semantics before config source changes ──

/// `**` depth: a `**/<name>` pattern must match at any nesting depth,
/// including deep (3+ levels) and shallow (1 level).
#[test]
fn double_glob_matches_at_any_depth() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".dev.vars", "root");
    write(src.path(), "apps/api/.dev.vars", "l1");
    write(src.path(), "apps/api/nested/deep/.dev.vars", "deep");
    let (summary, events) = collect(src.path(), dest.path(), &["**/.dev.vars"]);
    assert!(dest.path().join(".dev.vars").is_file(), "root-level match");
    assert!(
        dest.path().join("apps/api/.dev.vars").is_file(),
        "one-level match"
    );
    assert!(
        dest.path().join("apps/api/nested/deep/.dev.vars").is_file(),
        "deep match"
    );
    assert_eq!(summary.copied, 3, "all three depths must be copied");
    assert_eq!(
        copy_events_of(&events).len(),
        3,
        "three Copy events expected"
    );
}

/// `.venv` exclusion: matches inside a `.venv` dir at any depth are
/// silently dropped (non-fatal, not counted).
#[test]
fn venv_matches_are_dropped() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "apps/api/.env", "ok");
    write(src.path(), ".venv/lib/python3.11/.env", "drop");
    write(src.path(), "packages/foo/.venv/pyvenv.cfg", "drop");
    let (summary, events) = collect(src.path(), dest.path(), &["**/.env", "**/*.cfg"]);
    assert!(
        dest.path().join("apps/api/.env").is_file(),
        "real .env must be copied"
    );
    assert!(
        !dest.path().join(".venv/lib/python3.11/.env").exists(),
        ".venv/.env must be excluded"
    );
    assert!(
        !dest.path()
            .join("packages/foo/.venv/pyvenv.cfg")
            .exists(),
        ".venv/*.cfg must be excluded"
    );
    assert_eq!(summary.copied, 1, "only the real .env is copied");
    assert_eq!(summary.skipped, 0, "excluded items must not count");
    let skips = skip_events_of(&events);
    assert!(
        skips
            .iter()
            .filter(|(r, _)| matches!(r, SkipReason::Excluded))
            .count()
            >= 2,
        "expected at least two Excluded skip events, got: {skips:?}"
    );
}

/// Characterization: `*` in globset matches path separators (unlike POSIX shell
/// glob). `apps/*/.env` therefore matches both `apps/api/.env` AND
/// `apps/api/nested/.env`. This pins the engine semantics so callers don't
/// silently rely on `*` = "one component only".
#[test]
fn single_star_matches_across_path_separators_in_globset() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "apps/api/.env", "ok");
    write(src.path(), "apps/api/nested/.env", "nested");
    // globset's `*` is NOT literal-separator-aware by default, so
    // `apps/*/.env` matches paths at any depth below `apps/` ending in `/.env`.
    let (summary, _) = collect(src.path(), dest.path(), &["apps/*/.env"]);
    assert!(
        dest.path().join("apps/api/.env").is_file(),
        "direct child must match"
    );
    assert!(
        dest.path().join("apps/api/nested/.env").is_file(),
        "globset `*` crosses path separators — nested path also matches"
    );
    assert_eq!(summary.copied, 2);
}

/// Directory copies via `apps/*/config` pattern: the directory itself is
/// matched (not its entries), so all files inside are copied recursively.
#[test]
fn matched_directory_is_copied_recursively() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "apps/api/config/a.toml", "a");
    write(src.path(), "apps/api/config/sub/b.toml", "b");
    let (summary, _) = collect(src.path(), dest.path(), &["apps/api/config"]);
    assert!(
        dest.path().join("apps/api/config/a.toml").is_file(),
        "top-level file in dir"
    );
    assert!(
        dest.path().join("apps/api/config/sub/b.toml").is_file(),
        "nested file in dir"
    );
    // The directory itself is one matched entry; its contents are copied
    // recursively — summary.copied == 1 (the dir match), not 2 (the files).
    assert_eq!(summary.copied, 1, "directory counts as one copy event");
    assert_eq!(summary.skipped, 0);
}

/// De-duplication: a path matched by two different patterns appears only once.
#[test]
fn duplicate_match_across_patterns_is_deduplicated() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".env", "x");
    // Both patterns match `.env`.
    let (summary, events) = collect(src.path(), dest.path(), &[".env", "**/.env"]);
    assert_eq!(summary.copied, 1, ".env must be copied exactly once");
    assert_eq!(
        copy_events_of(&events).len(),
        1,
        "only one Copy event for a deduped match"
    );
}
