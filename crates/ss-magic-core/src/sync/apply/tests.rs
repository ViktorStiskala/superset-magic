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

// ── Excluded state trees (whole-tree enumeration filter) ────────────────

/// AE86. A bare `.superset` LITERAL in a repository's `files` list never
/// reaches `walk_source` — `expand_patterns` appends the rel directly — so the
/// copy walk is the only place left to guard. It must copy the contract files
/// into the worktree and nothing from the two state trees underneath.
#[test]
fn bare_superset_literal_copies_contract_files_but_not_state_trees() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".superset/config.json", "{}\n");
    write(src.path(), ".superset/magic.sh", "#!/bin/sh\n");
    write(src.path(), ".superset/magic.json", "{\"files\":[]}\n");
    write(src.path(), ".superset/.magic/state.json", "{\"token\":\"s3cret\"}\n");
    write(
        src.path(),
        ".superset/backups/20260101-000000/worktree/.env",
        "RECOVERED=1\n",
    );

    let (summary, _) = collect(src.path(), dest.path(), &[".superset"]);
    assert_eq!(summary.skipped, 0);
    for want in [
        ".superset/config.json",
        ".superset/magic.sh",
        ".superset/magic.json",
    ] {
        assert!(dest.path().join(want).is_file(), "{want} must be copied");
    }
    assert!(
        !dest.path().join(".superset/.magic").exists(),
        "the plugin state tree must never be copied into the worktree"
    );
    assert!(
        !dest.path().join(".superset/backups").exists(),
        "the backups tree must never be copied into the worktree"
    );
}

/// AE18. A `.superset/**` glob would otherwise match paths inside the plugin's
/// state tree. `walk_source` must never yield them, whatever the pattern's
/// breadth, while `.superset/magic.json` still syncs.
#[test]
fn glob_over_superset_skips_the_magic_state_tree() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".superset/magic.json", "{\"files\":[]}\n");
    write(src.path(), ".superset/.magic/state.json", "{\"token\":\"s3cret\"}\n");
    write(src.path(), ".superset/.magic/nested/more.json", "{}\n");

    let (_, events) = collect(src.path(), dest.path(), &[".superset/**"]);
    let copied = copy_events_of(&events);
    assert!(
        copied.contains(&Path::new(".superset/magic.json")),
        "the contract file must still sync: {copied:?}"
    );
    assert!(
        !copied.iter().any(|p| p.starts_with(".superset/.magic")),
        "the state tree must not be enumerated: {copied:?}"
    );
    assert!(!dest.path().join(".superset/.magic").exists());
}

/// A broad `**` sweep must skip every excluded tree — including `.git` and the
/// third-party `.scratchpad/` — while ordinary files still copy.
#[test]
fn broad_glob_skips_every_excluded_tree() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "keep.txt", "keep\n");
    write(src.path(), ".git/config", "[core]\n");
    write(src.path(), ".scratchpad/notes.md", "scratch\n");
    write(src.path(), ".superset/.magic/state.json", "{}\n");
    write(
        src.path(),
        ".superset/backups/20260101-000000/main/.env",
        "R=1\n",
    );

    let (_, events) = collect(src.path(), dest.path(), &["**"]);
    let copied = copy_events_of(&events);
    assert!(copied.contains(&Path::new("keep.txt")), "got {copied:?}");
    for forbidden in [".git", ".scratchpad", ".superset/.magic", ".superset/backups"] {
        assert!(
            !copied.iter().any(|p| p.starts_with(forbidden)),
            "{forbidden} must not be enumerated: {copied:?}"
        );
        assert!(
            !dest.path().join(forbidden).exists(),
            "{forbidden} must not be copied into the destination"
        );
    }
}

/// The whole-tree exclusion is keyed on the exact component path: a sibling
/// `.superset/.magicked/`, a sibling `.superset/backupsfoo/`, and a root-level
/// file named `.magic` are ordinary files that must still sync.
#[test]
fn excluded_trees_match_exact_components_not_prefixes() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".superset/.magicked/keep.txt", "keep\n");
    write(src.path(), ".superset/backupsfoo/keep.txt", "keep\n");
    write(src.path(), ".magic", "root-level file\n");

    let (summary, _) = collect(src.path(), dest.path(), &["**"]);
    assert_eq!(summary.skipped, 0);
    for want in [
        ".superset/.magicked/keep.txt",
        ".superset/backupsfoo/keep.txt",
        ".magic",
    ] {
        assert!(dest.path().join(want).is_file(), "{want} must be copied");
    }
}

// --- PathFilter / run_filtered --------------------------------------------

fn collect_filtered(
    src: &Path,
    dest: &Path,
    patterns: &[&str],
    filter: &[&str],
) -> (Summary, Vec<Event>) {
    let pats: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    let filter = PathFilter::new(filter.iter().map(PathBuf::from));
    let mut events = Vec::new();
    let summary = run_filtered(src, dest, &pats, &filter, |e| events.push(e.clone())).unwrap();
    (summary, events)
}

fn filtered_labels(events: &[Event]) -> Vec<&str> {
    skip_events_of(events)
        .into_iter()
        .filter(|(r, _)| matches!(r, SkipReason::Filtered))
        .map(|(_, label)| label)
        .collect()
}

#[test]
fn path_filter_covers_an_entry_and_everything_beneath_it_by_component() {
    let filter = PathFilter::new([PathBuf::from("config"), PathBuf::from("apps/api/.env")]);
    assert!(filter.covers(Path::new("config")));
    assert!(filter.covers(Path::new("config/a.json")));
    assert!(filter.covers(Path::new("config/deep/er/a.json")));
    assert!(filter.covers(Path::new("apps/api/.env")));
    // Whole components only – never a string prefix.
    assert!(!filter.covers(Path::new("config.json")));
    assert!(!filter.covers(Path::new("configs/a.json")));
    assert!(!filter.covers(Path::new("apps/api/.env.local")));
    assert!(!filter.covers(Path::new("apps/api")));
    assert!(!filter.covers(Path::new("apps")));
}

#[test]
fn path_filter_drops_an_empty_path_instead_of_covering_the_whole_tree() {
    let filter = PathFilter::new([PathBuf::new()]);
    assert!(filter.is_empty());
    assert!(!filter.covers(Path::new(".env")));
    assert!(!PathFilter::empty().covers(Path::new(".env")));
}

#[test]
fn filtered_match_is_not_copied_and_not_counted_as_skipped() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), ".env", "MAIN=1\n");
    write(src.path(), "apps/api/.env", "API=1\n");
    // The destination already holds its own copy of the filtered path.
    write(dest.path(), ".env", "ALREADY-THERE=1\n");

    let (summary, events) =
        collect_filtered(src.path(), dest.path(), &[".env", "apps/api/.env"], &[".env"]);

    assert_eq!(summary.copied, 1);
    assert_eq!(summary.skipped, 0, "a filtered path is not a skip");
    assert_eq!(summary.filtered, 1);
    assert_eq!(copy_events_of(&events), vec![Path::new("apps/api/.env")]);
    assert_eq!(filtered_labels(&events), vec![".env"]);
    assert_eq!(
        fs::read_to_string(dest.path().join(".env")).unwrap(),
        "ALREADY-THERE=1\n",
        "the filtered destination file must be left byte-for-byte alone"
    );
    assert!(dest.path().join("apps/api/.env").is_file());
}

#[test]
fn filtered_file_inside_a_matched_directory_is_left_alone() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "config/a.json", "main-a");
    write(src.path(), "config/b.json", "main-b");
    write(dest.path(), "config/a.json", "dest-a");

    // The trailing slash is how a directory literal is often written.
    let (summary, events) =
        collect_filtered(src.path(), dest.path(), &["config/"], &["config/a.json"]);

    assert_eq!(summary.copied, 1, "the directory match itself still copies");
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.filtered, 1);
    assert_eq!(filtered_labels(&events), vec!["config/a.json"]);
    assert_eq!(
        fs::read_to_string(dest.path().join("config/a.json")).unwrap(),
        "dest-a",
        "a filtered file under a matched directory must not be overwritten"
    );
    assert_eq!(
        fs::read_to_string(dest.path().join("config/b.json")).unwrap(),
        "main-b"
    );
}

#[test]
fn filtered_file_absent_from_the_destination_is_not_created() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "config/a.json", "main-a");
    write(src.path(), "config/b.json", "main-b");

    collect_filtered(src.path(), dest.path(), &["config"], &["config/a.json"]);

    assert!(!dest.path().join("config/a.json").exists());
    assert!(dest.path().join("config/b.json").is_file());
}

#[test]
fn filter_entry_naming_a_directory_covers_the_whole_match() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "config/a.json", "a");
    write(src.path(), "config/nested/b.json", "b");
    write(src.path(), "other.txt", "o");

    let (summary, events) =
        collect_filtered(src.path(), dest.path(), &["config", "other.txt"], &["config"]);

    assert!(!dest.path().join("config").exists(), "nothing under config/ is copied");
    assert!(dest.path().join("other.txt").is_file());
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.filtered, 1, "a filtered directory is reported once");
    assert_eq!(filtered_labels(&events), vec!["config"]);
}

#[test]
fn filter_entry_naming_a_directory_covers_a_match_beneath_it() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "config/local/a.json", "a");
    // `.txt`, not `.json`: globset's `*` crosses path separators here, so a
    // `.json` file in `sub/` would be a second glob match of its own.
    write(src.path(), "config/local/sub/b.txt", "b");
    write(src.path(), "config/shared.json", "s");

    // A glob match beneath the filtered directory, and a directory match that
    // is an ancestor of it: both enforcement points in one run.
    let (summary, events) = collect_filtered(
        src.path(),
        dest.path(),
        &["config/local/*.json", "config"],
        &["config/local"],
    );

    assert!(!dest.path().join("config/local").exists());
    assert!(dest.path().join("config/shared.json").is_file());
    assert_eq!(summary.skipped, 0);
    assert_eq!(
        filtered_labels(&events),
        vec!["config/local/a.json", "config/local"],
        "one event for the filtered glob match, one for the pruned directory"
    );
    assert_eq!(summary.filtered, 2);
}

/// A directory match whose every file is filtered copies nothing, and says so:
/// one "left alone" event per file, no "copied" event for the directory.
#[test]
fn a_directory_match_with_every_file_filtered_is_not_reported_as_copied() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    write(src.path(), "config/local/a.json", "main-a");
    write(src.path(), "config/local/b.json", "main-b");
    write(src.path(), "other.txt", "o");
    write(dest.path(), "config/local/a.json", "dest-a");
    write(dest.path(), "config/local/b.json", "dest-b");

    let (summary, events) = collect_filtered(
        src.path(),
        dest.path(),
        &["config/local", "other.txt"],
        &["config/local/a.json", "config/local/b.json"],
    );

    assert_eq!(copy_events_of(&events), vec![Path::new("other.txt")]);
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.filtered, 2);
    assert_eq!(summary.skipped, 0);
    assert_eq!(
        fs::read_to_string(dest.path().join("config/local/a.json")).unwrap(),
        "dest-a"
    );
}

/// An EMPTY matched directory is still reported as copied, as it always was:
/// nothing was filtered, so the empty directory is the whole result.
#[test]
fn an_empty_directory_match_is_still_reported_as_copied() {
    let src = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    fs::create_dir_all(src.path().join("empty")).unwrap();

    let (summary, events) =
        collect_filtered(src.path(), dest.path(), &["empty"], &["unrelated"]);

    assert_eq!(copy_events_of(&events), vec![Path::new("empty")]);
    assert_eq!(summary.copied, 1);
    assert!(dest.path().join("empty").is_dir());
}

#[test]
fn empty_filter_behaves_exactly_like_run() {
    let build = || {
        let src = tempfile::tempdir().unwrap();
        write(src.path(), ".env", "E");
        write(src.path(), "config/a.json", "a");
        write(src.path(), "node_modules/x/.env", "drop");
        src
    };
    let patterns = [".env", "config", "**/.env", "missing.txt", "/abs"];

    let src = build();
    let dest_plain = tempfile::tempdir().unwrap();
    let (plain, plain_events) = collect(src.path(), dest_plain.path(), &patterns);
    let dest_filtered = tempfile::tempdir().unwrap();
    let (filtered, filtered_events) =
        collect_filtered(src.path(), dest_filtered.path(), &patterns, &[]);

    assert_eq!(plain.copied, filtered.copied);
    assert_eq!(plain.skipped, filtered.skipped);
    assert_eq!(plain.filtered, 0);
    assert_eq!(filtered.filtered, 0);
    assert_eq!(format!("{plain_events:?}"), format!("{filtered_events:?}"));
}

#[test]
fn filtered_skip_reason_never_counts() {
    assert!(!SkipReason::Filtered.counts());
}
