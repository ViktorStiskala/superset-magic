//! Heartbeat tests: the row's shape on the wire, the store's permissions, the
//! retention bounds, and what happens when several hooks append at once.
//!
//! Every test points the store at a tempdir. Nothing here touches the real
//! `ProjectDirs` root — a test suite that wrote into the user's own heartbeat
//! log would both pollute it and make `status` lie.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

/// A fixed instant, so timestamps in assertions are stable: 2026-08-30
/// 12:00:00 UTC.
const NOW: u64 = 1_788_091_200;

fn store() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("plugin");
    (dir, store)
}

fn row(event: &str, outcome: Outcome) -> Row {
    Row::new(event, NOW, outcome)
}

// ── The row on the wire ───────────────────────────────────────────────────────

#[test]
fn a_row_round_trips_through_the_log() {
    let (_dir, store) = store();
    let written = row("pre-tool-use", Outcome::Ok)
        .with_cwd(Some("/repo".to_string()))
        .with_reason("disabled")
        .with_detail("nothing else ran");

    append(&store, &written).unwrap();

    assert_eq!(read(&store).unwrap(), vec![written]);
}

/// The fields a person greps for, spelled the way `status` will read them.
/// Both time fields describe the same instant; `at` is the readable one and
/// `ts` is what retention compares against.
#[test]
fn the_row_json_carries_event_time_cwd_outcome_and_class() {
    let (_dir, store) = store();
    append(
        &store,
        &row("session-start", Outcome::NoOp)
            .with_cwd(Some("/repo".to_string()))
            .with_reason("not-ignored")
            .with_detail("no covering rule"),
    )
    .unwrap();

    let text = fs::read_to_string(log_path(&store)).unwrap();
    let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();

    assert_eq!(value["event"], "session-start");
    assert_eq!(value["at"], "2026-08-30T12:00:00Z");
    assert_eq!(value["ts"], NOW);
    assert_eq!(value["cwd"], "/repo");
    assert_eq!(value["outcome"], "no-op");
    assert_eq!(value["reason"], "not-ignored");
    assert_eq!(value["detail"], "no covering rule");
}

/// Absent optional fields are omitted rather than written as nulls, so a
/// plain success is one short line.
#[test]
fn a_plain_success_row_omits_the_optional_fields() {
    let (_dir, store) = store();
    append(&store, &row("session-end", Outcome::Ok)).unwrap();

    let value: serde_json::Value =
        serde_json::from_str(fs::read_to_string(log_path(&store)).unwrap().trim()).unwrap();
    for key in ["cwd", "reason", "detail"] {
        assert!(value.get(key).is_none(), "{key} should be omitted: {value}");
    }
}

#[test]
fn the_three_outcomes_serialize_in_kebab_case() {
    for (outcome, expected) in [
        (Outcome::Ok, "ok"),
        (Outcome::NoOp, "no-op"),
        (Outcome::Error, "error"),
    ] {
        let json = serde_json::to_value(outcome).unwrap();
        assert_eq!(json, expected);
    }
}

#[test]
fn each_row_is_one_line_and_appends_after_the_last() {
    let (_dir, store) = store();
    for event in ["session-start", "pre-tool-use", "session-end"] {
        append(&store, &row(event, Outcome::Ok)).unwrap();
    }

    let text = fs::read_to_string(log_path(&store)).unwrap();
    assert_eq!(text.lines().count(), 3);
    let events: Vec<String> = read(&store).unwrap().into_iter().map(|r| r.event).collect();
    assert_eq!(events, ["session-start", "pre-tool-use", "session-end"]);
}

/// One corrupt line must not hide the rest of the history from `status`.
#[test]
fn read_skips_unparseable_lines_rather_than_failing() {
    let (_dir, store) = store();
    append(&store, &row("session-start", Outcome::Ok)).unwrap();
    let mut text = fs::read_to_string(log_path(&store)).unwrap();
    text.push_str("this is not json\n");
    fs::write(log_path(&store), text).unwrap();
    append(&store, &row("session-end", Outcome::Ok)).unwrap();

    let events: Vec<String> = read(&store).unwrap().into_iter().map(|r| r.event).collect();
    assert_eq!(events, ["session-start", "session-end"]);
}

/// No hook has fired yet is an empty history, not an error.
#[test]
fn reading_a_store_with_no_log_yet_is_empty() {
    let (_dir, store) = store();
    fs::create_dir_all(&store).unwrap();
    assert!(read(&store).unwrap().is_empty());
}

// ── R58: owner-only ───────────────────────────────────────────────────────────

/// The rows name repository paths, and through them the projects someone works
/// on. Both the store directory and the log itself are created owner-only.
#[test]
fn the_store_and_the_log_are_created_owner_only() {
    let (dir, _) = store();
    // Two levels deep, so the assertion covers a component `ensure_store`
    // created on the way down rather than only the leaf.
    let store = dir.path().join("ss-magic").join("plugin");

    append(&store, &row("session-start", Outcome::Ok)).unwrap();

    for path in [dir.path().join("ss-magic"), store.clone(), log_path(&store)] {
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let expected = if path.is_dir() { 0o700 } else { 0o600 };
        assert_eq!(mode, expected, "{}", path.display());
    }
}

/// The store sits on ss-magic's app DATA path, not the cache path
/// `update/check.rs` uses. The two must not collide: a hook log that vanishes
/// on a disk-cleanup run is worse than none, because nothing reports that it
/// happened.
///
/// Asserted against `store_path`, which resolves without creating — so running
/// the suite never leaves a directory behind in the developer's own app data.
#[test]
fn the_store_sits_on_the_data_path_not_the_cache_path() {
    let Some(store) = store_path() else {
        // No home directory on this platform; nothing to assert.
        return;
    };
    let dirs = directories::ProjectDirs::from("", "", "ss-magic").unwrap();

    assert_eq!(store, dirs.data_dir().join(STORE_SUBDIR));
    assert_ne!(store.parent(), Some(dirs.cache_dir()));
}

// ── R45: bounded count and age ────────────────────────────────────────────────

/// Seed `path` with `count` rows, the oldest `aged` of them stamped older than
/// the retention window.
fn seed(path: &Path, count: usize, aged: usize) {
    let mut body = String::new();
    for i in 0..count {
        let ts = if i < aged {
            NOW - MAX_AGE_SECS - 1
        } else {
            NOW - (count - i) as u64
        };
        let row = Row::new(&format!("event-{i}"), ts, Outcome::Ok);
        body.push_str(&serde_json::to_string(&row).unwrap());
        body.push('\n');
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn prune_keeps_only_the_newest_rows_within_the_count_bound() {
    let (_dir, store) = store();
    let path = log_path(&store);
    seed(&path, 10, 0);

    let removed = prune(&path, 4, MAX_AGE_SECS, NOW).unwrap();

    assert_eq!(removed, 6);
    let events: Vec<String> = read(&store).unwrap().into_iter().map(|r| r.event).collect();
    assert_eq!(events, ["event-6", "event-7", "event-8", "event-9"]);
}

#[test]
fn prune_drops_rows_past_the_age_bound_even_inside_the_count_bound() {
    let (_dir, store) = store();
    let path = log_path(&store);
    seed(&path, 6, 3);

    let removed = prune(&path, 100, MAX_AGE_SECS, NOW).unwrap();

    assert_eq!(removed, 3);
    let events: Vec<String> = read(&store).unwrap().into_iter().map(|r| r.event).collect();
    assert_eq!(events, ["event-3", "event-4", "event-5"]);
}

/// A line with no timestamp cannot be aged and carries no outcome to count, so
/// keeping it would mean the file grows without bound in exactly the case
/// where something is already writing garbage into it.
#[test]
fn prune_drops_lines_that_do_not_parse() {
    let (_dir, store) = store();
    let path = log_path(&store);
    seed(&path, 3, 0);
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("garbage\n");
    fs::write(&path, text).unwrap();

    prune(&path, 100, MAX_AGE_SECS, NOW).unwrap();

    assert_eq!(read(&store).unwrap().len(), 3);
    assert!(!fs::read_to_string(&path).unwrap().contains("garbage"));
}

/// A row stamped in the future is a clock that jumped backwards since it was
/// written, not a row to discard: it is newer than any cutoff.
#[test]
fn prune_keeps_a_row_stamped_in_the_future() {
    let (_dir, store) = store();
    let path = log_path(&store);
    let future = Row::new("event-future", NOW + 86_400, Outcome::Ok);
    fs::create_dir_all(&store).unwrap();
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string(&future).unwrap()),
    )
    .unwrap();

    assert_eq!(prune(&path, 100, MAX_AGE_SECS, NOW).unwrap(), 0);
    assert_eq!(read(&store).unwrap(), vec![future]);
}

/// Nothing to remove means the file is left exactly as it was, rather than
/// being rewritten for nothing on every single hook invocation.
#[test]
fn prune_is_a_no_op_when_nothing_is_over_the_bounds() {
    let (_dir, store) = store();
    let path = log_path(&store);
    seed(&path, 3, 0);
    let before = fs::read(&path).unwrap();

    assert_eq!(prune(&path, 100, MAX_AGE_SECS, NOW).unwrap(), 0);
    assert_eq!(fs::read(&path).unwrap(), before);
}

/// An append below the trigger size does not read or rewrite the file — the
/// common case must stay O(1), because every `PreToolUse` pays for it on the
/// model's clock.
#[test]
fn a_small_log_is_not_pruned_on_append() {
    let (_dir, store) = store();
    let path = log_path(&store);
    seed(&path, ROWS_KEPT + 50, 0);
    assert!(fs::metadata(&path).unwrap().len() < PRUNE_TRIGGER_BYTES);

    append(&store, &row("session-end", Outcome::Ok)).unwrap();

    assert_eq!(read(&store).unwrap().len(), ROWS_KEPT + 51);
}

/// Once the file is big enough to be worth reading, the append trims it back
/// to the bound.
#[test]
fn the_log_is_trimmed_to_its_bound_once_it_passes_the_trigger() {
    let (_dir, store) = store();
    let path = log_path(&store);

    // Rows padded so the file crosses the trigger without needing hundreds of
    // thousands of them.
    let padding = "x".repeat(400);
    let mut body = String::new();
    for i in 0..(ROWS_KEPT + 200) {
        let r = Row::new("pre-tool-use", NOW - 1, Outcome::Ok).with_detail(format!("{i}{padding}"));
        body.push_str(&serde_json::to_string(&r).unwrap());
        body.push('\n');
    }
    fs::create_dir_all(&store).unwrap();
    fs::write(&path, body).unwrap();
    assert!(fs::metadata(&path).unwrap().len() > PRUNE_TRIGGER_BYTES);

    append(&store, &row("session-end", Outcome::Ok)).unwrap();

    // The newly appended row survives — it is the newest — and the file is
    // back at the bound.
    let rows = read(&store).unwrap();
    assert_eq!(rows.len(), ROWS_KEPT);
    assert_eq!(rows.last().unwrap().event, "session-end");
}

/// R45's posture: the prune is best-effort and can never turn a successful
/// append into a reported failure.
///
/// The setup is a read-only store directory holding a log that is already past
/// the prune trigger. Directory write permission is needed to CREATE an entry,
/// not to write an existing one, so the lock file and the log itself both still
/// open — but `write_atomically`'s temp file cannot be created, so the prune
/// fails on every attempt. The row still lands, and `append` still reports
/// success, because the row is the thing that matters.
#[test]
fn a_prune_failure_never_fails_the_append() {
    let (_dir, store) = store();
    let path = log_path(&store);
    seed(&path, 40, 0);
    // Pad the log past the trigger so the append genuinely tries to prune.
    let padding = format!("{}\n", "x".repeat(PRUNE_TRIGGER_BYTES as usize));
    fs::write(
        &path,
        format!("{}{padding}", fs::read_to_string(&path).unwrap()),
    )
    .unwrap();
    // Pre-create the lock file: a read-only directory refuses new entries, and
    // this test is about the rewrite failing, not the lock.
    fs::write(store.join("hooks.lock"), "").unwrap();

    let mut perms = fs::metadata(&store).unwrap().permissions();
    let original = perms.mode();
    perms.set_mode(0o500);
    fs::set_permissions(&store, perms).unwrap();

    let pruned_directly = prune(&path, 1, MAX_AGE_SECS, NOW);
    let appended = append(&store, &row("session-end", Outcome::Ok));

    let mut perms = fs::metadata(&store).unwrap().permissions();
    perms.set_mode(original);
    fs::set_permissions(&store, perms).unwrap();

    assert!(
        pruned_directly.is_err(),
        "the prune itself should report the failure it hit"
    );
    assert!(appended.is_ok(), "the append swallows it: {appended:?}");
    // And the row it was asked to write is on disk, unpruned.
    let rows = read(&store).unwrap();
    assert_eq!(rows.last().unwrap().event, "session-end");
    assert!(rows.len() > 1, "nothing was pruned, as expected");
}

// ── Concurrency ───────────────────────────────────────────────────────────────

/// Hook handlers registered on one event all fire at once, and two sessions on
/// one machine share this file. Every row must survive, whole — a half-written
/// line would be indistinguishable from corruption.
#[test]
fn concurrent_appends_all_land_intact() {
    let (_dir, store) = store();
    const WRITERS: usize = 8;
    const EACH: usize = 12;

    std::thread::scope(|scope| {
        for w in 0..WRITERS {
            let store = store.clone();
            scope.spawn(move || {
                for i in 0..EACH {
                    let r = Row::new("pre-tool-use", NOW, Outcome::Ok)
                        .with_detail(format!("writer-{w}-row-{i}"));
                    append(&store, &r).unwrap();
                }
            });
        }
    });

    let rows = read(&store).unwrap();
    assert_eq!(rows.len(), WRITERS * EACH);
    // No line was interleaved into another: every row parses AND every
    // expected detail is present exactly once.
    let details: std::collections::HashSet<String> =
        rows.into_iter().filter_map(|r| r.detail).collect();
    assert_eq!(details.len(), WRITERS * EACH);
}
