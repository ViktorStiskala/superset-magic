//! The pure decision table, the lock-guarded marker write, the refresh under
//! contention, the detached spawn, and the report's shape. Nothing here
//! touches the network: every fetch goes through a stub client, and the one
//! process spawned is a shell script that records its argv.

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ss_magic_core::release::{FetchOutcome, ReleaseEntry};
use tempfile::TempDir;

use super::*;

const NOW: u64 = 1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.
const DAY: u64 = 24 * 60 * 60;

// ── Fixtures ───────────────────────────────────────────────────────────────

fn cached(tag: &str, suggested: Option<&str>) -> Cache {
    Cache {
        checked_at: NOW - 60,
        tag_name: tag.to_string(),
        etag: Some("\"e1\"".to_string()),
        suggested: suggested.map(str::to_string),
    }
}

struct StubClient {
    outcome: FetchOutcome,
    calls: Cell<u32>,
}

impl StubClient {
    fn new(outcome: FetchOutcome) -> Self {
        Self {
            outcome,
            calls: Cell::new(0),
        }
    }

    fn ok_with(tags: &[&str]) -> Self {
        Self::new(FetchOutcome::Ok {
            releases: tags
                .iter()
                .map(|t| ReleaseEntry {
                    tag_name: t.to_string(),
                    draft: false,
                    prerelease: false,
                })
                .collect(),
            etag: Some("\"e2\"".to_string()),
        })
    }
}

impl ReleaseClient for StubClient {
    fn fetch_releases(&self, _etag: Option<&str>) -> FetchOutcome {
        self.calls.set(self.calls.get() + 1);
        self.outcome.clone()
    }
}

/// A cache file inside a tempdir, holding `cache`.
fn cache_in(dir: &TempDir, cache: &Cache) -> PathBuf {
    let path = dir.path().join(PLUGIN_LINE.cache_file);
    write_cache(&path, cache).unwrap();
    path
}

fn read(path: &Path) -> Cache {
    release::read_cache(path).expect("the cache file parses")
}

// ── The decision table (R29, R31) ──────────────────────────────────────────

/// Pinned vs newest, over every shape the cache can hold.
#[test]
fn suggestion_table_over_pinned_versus_newest() {
    let newer = cached("ss-magic-plugin-v1.1.0", None);
    let equal = cached("ss-magic-plugin-v1.0.0", None);
    let older = cached("ss-magic-plugin-v0.9.0", None);
    let cli_tag = cached("v1.1.0", None);
    let empty = cached("", None);
    let announced = cached("ss-magic-plugin-v1.1.0", Some("ss-magic-plugin-v1.1.0"));
    let announced_older = cached("ss-magic-plugin-v1.1.0", Some("ss-magic-plugin-v1.0.5"));

    let cases: &[(Option<&str>, Option<&Cache>, Decision)] = &[
        (
            Some("1.0.0"),
            Some(&newer),
            Decision::Suggest {
                tag: "ss-magic-plugin-v1.1.0".into(),
                pinned: "1.0.0".into(),
            },
        ),
        (
            Some("1.0.0"),
            Some(&equal),
            Decision::Silent(Silence::NotNewer {
                tag: "ss-magic-plugin-v1.0.0".into(),
                pinned: "1.0.0".into(),
            }),
        ),
        (
            Some("1.0.0"),
            Some(&older),
            Decision::Silent(Silence::NotNewer {
                tag: "ss-magic-plugin-v0.9.0".into(),
                pinned: "1.0.0".into(),
            }),
        ),
        // A pin that is not a plain triple can never be "older than" anything.
        (
            Some("1.0.0-dev"),
            Some(&newer),
            Decision::Silent(Silence::NotNewer {
                tag: "ss-magic-plugin-v1.1.0".into(),
                pinned: "1.0.0-dev".into(),
            }),
        ),
        (Some("1.0.0"), Some(&cli_tag), Decision::Silent(Silence::NoTag)),
        (Some("1.0.0"), Some(&empty), Decision::Silent(Silence::NoTag)),
        (Some("1.0.0"), None, Decision::Silent(Silence::NoCache)),
        (None, Some(&newer), Decision::Silent(Silence::NoPin)),
        (Some("  "), Some(&newer), Decision::Silent(Silence::NoPin)),
        (
            Some("1.0.0"),
            Some(&announced),
            Decision::Silent(Silence::AlreadySuggested(
                "ss-magic-plugin-v1.1.0".into(),
            )),
        ),
        // A marker for a DIFFERENT tag does not silence this one.
        (
            Some("1.0.0"),
            Some(&announced_older),
            Decision::Suggest {
                tag: "ss-magic-plugin-v1.1.0".into(),
                pinned: "1.0.0".into(),
            },
        ),
    ];
    for (pinned, cache, expected) in cases {
        assert_eq!(
            &suggestion(*pinned, *cache, "startup", None),
            expected,
            "pinned {pinned:?}, cache {cache:?}"
        );
    }
}

/// The pin is trimmed: a trailing newline from the version file is not a
/// different version.
#[test]
fn a_pin_with_surrounding_whitespace_still_compares() {
    let newer = cached("ss-magic-plugin-v1.1.0", None);
    assert_eq!(
        suggestion(Some("1.0.0\n"), Some(&newer), "startup", None),
        Decision::Suggest {
            tag: "ss-magic-plugin-v1.1.0".into(),
            pinned: "1.0.0".into(),
        }
    );
}

/// Only `startup` carries the notice, whatever the cache says.
#[test]
fn every_source_but_startup_is_silent() {
    let newer = cached("ss-magic-plugin-v1.1.0", None);
    for source in ["resume", "clear", "compact", "fork", "", "future"] {
        assert_eq!(
            suggestion(Some("1.0.0"), Some(&newer), source, None),
            Decision::Silent(Silence::NotStartup),
            "source `{source}`"
        );
    }
}

/// Quiet mode is decided LAST, so its reason names the tag that was
/// withheld — and a quiet session with nothing to say reads as "not newer",
/// never as "suppressed".
#[test]
fn quiet_mode_suppresses_only_a_suggestion_that_was_due() {
    let newer = cached("ss-magic-plugin-v1.1.0", None);
    let equal = cached("ss-magic-plugin-v1.0.0", None);
    let reason = "permission_mode is bypassPermissions";
    assert_eq!(
        suggestion(Some("1.0.0"), Some(&newer), "startup", Some(reason)),
        Decision::Silent(Silence::Quiet {
            tag: "ss-magic-plugin-v1.1.0".into(),
            reason,
        })
    );
    assert!(matches!(
        suggestion(Some("1.0.0"), Some(&equal), "startup", Some(reason)),
        Decision::Silent(Silence::NotNewer { .. })
    ));
}

/// The heartbeat note: recorded on the paths where something was
/// considered, absent on the ordinary ones (AE11's wording included).
#[test]
fn silence_details_name_what_happened() {
    assert_eq!(Silence::NotStartup.detail(), None);
    assert_eq!(Silence::NoPin.detail(), None);
    assert_eq!(Silence::NoCache.detail(), None);
    assert!(Silence::NoTag.detail().unwrap().contains("no plugin tag"));
    let quiet = Silence::Quiet {
        tag: "ss-magic-plugin-v1.1.0".into(),
        reason: "permission_mode is bypassPermissions",
    }
    .detail()
    .unwrap();
    assert!(quiet.starts_with("release suggestion suppressed (quiet mode"), "{quiet}");
    assert!(quiet.contains("ss-magic-plugin-v1.1.0"), "{quiet}");
    let shown = Silence::AlreadySuggested("ss-magic-plugin-v1.1.0".into())
        .detail()
        .unwrap();
    assert!(shown.contains("already shown"), "{shown}");
}

/// The notice names the release, the pin, `/plugin`, and the honest remedy —
/// and never a bare `ss-magic ` command line, since the operator does not
/// have the sync CLI's verbs for this.
#[test]
fn the_notice_names_the_release_the_pin_and_the_plugin_flow() {
    let text = notice("ss-magic-plugin-v1.1.0", "1.0.0");
    assert!(text.contains("ss-magic-plugin-v1.1.0"), "{text}");
    assert!(text.contains("pins 1.0.0"), "{text}");
    assert!(text.contains("/plugin"), "{text}");
    assert!(text.contains("new session"), "{text}");
    assert!(text.contains("once per release"), "{text}");
    assert!(!text.contains("ss-magic plugin "), "{text}");
    for line in text.lines() {
        assert!(!line.trim_start().starts_with("ss-magic "), "{line}");
    }
}

// ── record_suggested ───────────────────────────────────────────────────────

/// The ordinary case: the marker is written into the record, and every other
/// field survives untouched.
#[test]
fn record_suggested_writes_the_marker_and_keeps_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let before = cached("ss-magic-plugin-v1.1.0", None);
    let path = cache_in(&dir, &before);

    let outcome = record_suggested(lock.path(), &path, "ss-magic-plugin-v1.1.0").unwrap();
    assert_eq!(outcome, Recorded::Written);
    assert_eq!(
        read(&path),
        Cache {
            suggested: Some("ss-magic-plugin-v1.1.0".into()),
            ..before
        }
    );
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the cache is owner-only");
}

/// A second writer for the same tag finds it recorded and must not announce.
#[test]
fn record_suggested_reports_a_marker_another_session_already_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let path = cache_in(
        &dir,
        &cached("ss-magic-plugin-v1.1.0", Some("ss-magic-plugin-v1.1.0")),
    );
    assert_eq!(
        record_suggested(lock.path(), &path, "ss-magic-plugin-v1.1.0").unwrap(),
        Recorded::AlreadyRecorded
    );
}

/// The decision was made from a lock-free read; if a refresh moved the tag
/// in between, the stale tag is not recorded and nothing is announced.
#[test]
fn record_suggested_refuses_a_tag_the_cache_no_longer_names() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let before = cached("ss-magic-plugin-v1.2.0", None);
    let path = cache_in(&dir, &before);
    assert_eq!(
        record_suggested(lock.path(), &path, "ss-magic-plugin-v1.1.0").unwrap(),
        Recorded::Superseded
    );
    assert_eq!(read(&path), before, "nothing written");
}

/// Contention on the shared lock is a skip, never a wait: the caller
/// withholds the notice this session rather than risk announcing twice.
#[test]
fn record_suggested_skips_when_the_lock_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let before = cached("ss-magic-plugin-v1.1.0", None);
    let path = cache_in(&dir, &before);

    let held = tmproot::with_lock(lock.path(), LOCK_NAME, || {
        let started = Instant::now();
        let outcome = record_suggested(lock.path(), &path, "ss-magic-plugin-v1.1.0").unwrap();
        (outcome, started.elapsed())
    })
    .unwrap();
    assert_eq!(held.0, Recorded::Busy);
    assert!(held.1 < Duration::from_secs(2), "must not block: {:?}", held.1);
    assert_eq!(read(&path), before, "nothing written under contention");
}

// ── refresh_with (R33) ─────────────────────────────────────────────────────

/// A `Failed` client bumps `checked_at` and keeps the prior tag and marker,
/// through core's derivation and this module's atomic write.
#[test]
fn refresh_with_a_failed_client_bumps_checked_at_and_keeps_the_prior_tag() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let prior = Cache {
        checked_at: NOW - 2 * DAY,
        ..cached("ss-magic-plugin-v1.1.0", Some("ss-magic-plugin-v1.1.0"))
    };
    let path = cache_in(&dir, &prior);
    let client = StubClient::new(FetchOutcome::Failed);

    let report = refresh_with(&client, lock.path(), &path, NOW).unwrap();
    assert_eq!(report, RefreshReport::Ran(RefreshOutcome::Failed));
    assert_eq!(client.calls.get(), 1);
    assert_eq!(
        read(&path),
        Cache {
            checked_at: NOW,
            ..prior
        }
    );
}

/// A successful fetch rewrites the record from the list, and a refresh from
/// no cache at all creates the file.
#[test]
fn refresh_with_a_fetch_writes_the_selected_tag() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let path = dir.path().join(PLUGIN_LINE.cache_file);
    assert!(!path.exists());
    let client = StubClient::ok_with(&["v0.11.1", "ss-magic-plugin-v1.1.0", "ss-magic-plugin-v1.0.0"]);

    let report = refresh_with(&client, lock.path(), &path, NOW).unwrap();
    assert_eq!(report, RefreshReport::Ran(RefreshOutcome::Fetched));
    assert_eq!(
        read(&path),
        Cache {
            checked_at: NOW,
            tag_name: "ss-magic-plugin-v1.1.0".into(),
            etag: Some("\"e2\"".into()),
            suggested: None,
        }
    );
}

/// A refresh always fetches — there is no freshness short-circuit in this
/// verb, because a person asking for `--refresh` means it.
#[test]
fn refresh_fetches_even_when_the_cache_is_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let path = cache_in(&dir, &cached("ss-magic-plugin-v1.0.0", None));
    let client = StubClient::new(FetchOutcome::NotModified);
    assert_eq!(
        refresh_with(&client, lock.path(), &path, NOW).unwrap(),
        RefreshReport::Ran(RefreshOutcome::NotModified)
    );
    assert_eq!(client.calls.get(), 1);
}

/// Two refreshes cannot run at once; the second is skipped, not queued, and
/// the network is not touched by it.
#[test]
fn refresh_skips_when_the_lock_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let path = cache_in(&dir, &cached("ss-magic-plugin-v1.0.0", None));
    let client = StubClient::ok_with(&["ss-magic-plugin-v9.9.9"]);

    let report = tmproot::with_lock(lock.path(), LOCK_NAME, || {
        refresh_with(&client, lock.path(), &path, NOW).unwrap()
    })
    .unwrap();
    assert_eq!(report, RefreshReport::Busy);
    assert_eq!(client.calls.get(), 0, "no fetch under contention");
    assert_eq!(read(&path).tag_name, "ss-magic-plugin-v1.0.0");
}

// ── spawn_detached (R30, AE12) ─────────────────────────────────────────────

/// The spawn returns at once, the child runs on its own with every stream on
/// /dev/null, and it receives exactly the argv it was given.
#[test]
fn spawn_detached_returns_without_waiting_and_passes_argv_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("argv");
    let script = dir.path().join("fake-plugin.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 1\nprintf '%s\\n' \"$@\" > '{}'\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let started = Instant::now();
    let pid = spawn_detached(&script, &REFRESH_ARGV).unwrap();
    assert!(pid > 0);
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "the spawn must not wait for the child's sleep: {:?}",
        started.elapsed()
    );

    // The child sleeps a second before writing, so the marker's absence now
    // is what shows the spawn did not block on it.
    assert!(!marker.exists(), "the child is still running");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "release-check\n--refresh\n--quiet\n"
    );
}

/// A missing executable is an `Err` for the heartbeat row, not a panic.
#[test]
fn spawn_detached_reports_a_missing_executable() {
    let dir = tempfile::tempdir().unwrap();
    let err = spawn_detached(&dir.path().join("does-not-exist"), &REFRESH_ARGV).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

// ── The report ─────────────────────────────────────────────────────────────

fn pin(version: &str) -> PinReport {
    PinReport {
        version: Some(version.to_string()),
        source: Some("test".to_string()),
        note: None,
    }
}

fn no_refresh() -> RefreshField {
    RefreshField {
        requested: false,
        outcome: "not-requested".into(),
        note: "test".into(),
    }
}

/// Newer than the pin: `update.available` is true and the note carries the
/// remedy; `notice` reflects the marker.
#[test]
fn report_flags_an_available_update() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_in(&dir, &cached("ss-magic-plugin-v1.1.0", None));
    let r = report(Some(&path), pin("1.0.0"), "1.0.0", NOW, no_refresh());
    assert!(r.cache.present);
    assert_eq!(r.cache.newest_tag.as_deref(), Some("ss-magic-plugin-v1.1.0"));
    assert_eq!(r.cache.age_secs, Some(60));
    assert_eq!(r.cache.fresh, Some(true));
    assert_eq!(r.cache.suggested, None);
    assert_eq!(r.update.available, Some(true));
    assert!(r.update.note.contains("/plugin"), "{}", r.update.note);

    let mut out = String::new();
    render_text(&mut out, &r);
    assert!(out.contains("ss-magic-plugin-v1.1.0"), "{out}");
    assert!(out.contains("not shown yet"), "{out}");
    assert!(!out.contains("refresh"), "no refresh row unless requested: {out}");
}

/// Equal to the pin: available is false; an unparseable pin: null with a
/// note; no cache: every null carries a note.
#[test]
fn report_nulls_always_carry_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_in(&dir, &cached("ss-magic-plugin-v1.0.0", None));
    let same = report(Some(&path), pin("1.0.0"), "1.0.0", NOW, no_refresh());
    assert_eq!(same.update.available, Some(false));

    let odd = report(Some(&path), pin("1.0.0-dev"), "1.0.0", NOW, no_refresh());
    assert_eq!(odd.update.available, None);
    assert!(odd.update.note.contains("MAJOR.MINOR.PATCH"), "{}", odd.update.note);

    let absent = report(
        Some(&dir.path().join("missing.json")),
        pin("1.0.0"),
        "1.0.0",
        NOW,
        no_refresh(),
    );
    assert!(!absent.cache.present);
    assert!(absent.cache.note.as_deref().unwrap().contains("--refresh"));
    assert_eq!(absent.update.available, None);
    assert!(!absent.update.note.is_empty());

    let nowhere = report(None, pin("1.0.0"), "1.0.0", NOW, no_refresh());
    assert!(nowhere.cache.note.as_deref().unwrap().contains("no cache directory"));

    // And through JSON: any null at the top of `cache`/`pin`/`update` has a
    // sibling `note` that is not null.
    let json: serde_json::Value = serde_json::to_value(&absent).unwrap();
    for section in ["cache", "pin", "update"] {
        let obj = json[section].as_object().unwrap();
        let has_null = obj.iter().any(|(k, v)| k != "note" && v.is_null());
        if has_null {
            assert!(!obj["note"].is_null(), "{section}: {obj:?}");
        }
    }
}

/// A cache holding a CLI tag reads as "no plugin tag", with the tag named.
#[test]
fn report_explains_a_cache_holding_the_other_lines_tag() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_in(&dir, &cached("v0.11.1", None));
    let r = report(Some(&path), pin("1.0.0"), "1.0.0", NOW, no_refresh());
    assert_eq!(r.cache.newest_tag, None);
    assert!(r.cache.note.as_deref().unwrap().contains("v0.11.1"));
}

/// Argument parsing: the three flags in any order, help, and a loud error.
#[test]
fn parse_args_accepts_the_three_flags_in_any_order() {
    let argv = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match parse_args(&argv(&["--quiet", "--json", "--refresh"])) {
        ParsedArgs::Run(flags) => assert_eq!(
            flags,
            Flags {
                refresh: true,
                json: true,
                quiet: true
            }
        ),
        _ => panic!("expected a run"),
    }
    match parse_args(&argv(&[])) {
        ParsedArgs::Run(flags) => assert_eq!(
            flags,
            Flags {
                refresh: false,
                json: false,
                quiet: false
            }
        ),
        _ => panic!("expected a run"),
    }
    assert!(matches!(parse_args(&argv(&["--help"])), ParsedArgs::Help));
    match parse_args(&argv(&["--now"])) {
        ParsedArgs::Error(message) => assert!(message.contains("--now")),
        _ => panic!("expected an error"),
    }
}

#[test]
fn format_age_picks_the_two_largest_units() {
    assert_eq!(format_age(5), "5s");
    assert_eq!(format_age(65), "1m 5s");
    assert_eq!(format_age(3_600 * 3 + 120), "3h 2m");
    assert_eq!(format_age(DAY * 2 + 3_600), "2d 1h");
}

/// Nothing the verb prints tells a person to type the retired CLI form.
#[test]
fn usage_names_the_plugin_binary_not_a_cli_subcommand() {
    assert!(USAGE.contains("ss-magic-plugin release-check"));
    assert!(!USAGE.contains("ss-magic plugin"));
}
