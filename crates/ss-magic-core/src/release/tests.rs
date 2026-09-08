use super::*;
use std::cell::Cell;
use tempfile::TempDir;

/// Test double for [`ReleaseClient`]. Returns a fixed outcome and records
/// whether it was invoked + the etag it was handed, so tests can assert
/// "no network call" on the fresh-cache path and ETag round-tripping.
struct StubClient {
    outcome: FetchOutcome,
    called: Cell<bool>,
    seen_etag: Cell<Option<String>>,
}

impl StubClient {
    fn new(outcome: FetchOutcome) -> Self {
        Self {
            outcome,
            called: Cell::new(false),
            seen_etag: Cell::new(None),
        }
    }

    /// A `200 OK` whose list holds exactly these published (non-draft,
    /// non-prerelease) tags, in the order given.
    fn ok_with_tags(tags: &[&str], etag: Option<&str>) -> Self {
        Self::new(FetchOutcome::Ok {
            releases: tags.iter().map(|t| published(t)).collect(),
            etag: etag.map(|s| s.to_string()),
        })
    }
}

impl ReleaseClient for StubClient {
    fn fetch_releases(&self, etag: Option<&str>) -> FetchOutcome {
        self.called.set(true);
        self.seen_etag.set(etag.map(|s| s.to_string()));
        self.outcome.clone()
    }
}

fn published(tag: &str) -> ReleaseEntry {
    ReleaseEntry {
        tag_name: tag.to_string(),
        draft: false,
        prerelease: false,
    }
}

fn draft(tag: &str) -> ReleaseEntry {
    ReleaseEntry {
        draft: true,
        ..published(tag)
    }
}

fn prerelease(tag: &str) -> ReleaseEntry {
    ReleaseEntry {
        prerelease: true,
        ..published(tag)
    }
}

/// AE1's release list, in GitHub's creation order: a plugin release, the
/// running CLI version, a draft of the next minor, a wrongly-prefixed CLI
/// tag, the real newest CLI release, and a pre-release.
fn ae1_releases() -> Vec<ReleaseEntry> {
    vec![
        published("ss-magic-plugin-v1.2.0"),
        published("v0.11.3"),
        draft("v0.12.0"),
        published("ss-magic-v0.13.0"),
        published("v0.11.10"),
        published("v0.9.0-rc1"),
    ]
}

fn cache_path(dir: &TempDir) -> PathBuf {
    dir.path().join(CLI_LINE.cache_file)
}

fn write_cache_file(path: &Path, cache: &Cache) {
    let body = serde_json::to_string_pretty(cache).unwrap();
    std::fs::write(path, body).unwrap();
}

fn read_cache_file(path: &Path) -> Cache {
    let raw = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

// ── Per-line tag filters (R17, AE2) ────────────────────────────────────

/// AE2's table: every spelling that is NOT exactly `<prefix>MAJOR.MINOR.PATCH`.
const AE2_TAGS: &[&str] = &[
    "v1.0.0",
    "ss-magic-plugin-v1.0.0",
    "ss-magic-v1.0.0",
    "xv1.0.0",
    "V1.0.0",
    "ss-magic-plugin-v1.0.0.zip",
    "v1.0.0-rc1",
    "v1.0",
];

#[test]
fn cli_filter_rejects_plugin_tags() {
    let accepted: Vec<&str> = AE2_TAGS
        .iter()
        .copied()
        .filter(|t| parse_line_tag(&CLI_LINE, t).is_some())
        .collect();
    assert_eq!(accepted, ["v1.0.0"], "the CLI line accepts only the bare v-tag");
    assert_eq!(parse_line_tag(&CLI_LINE, "v1.0.0"), Some((1, 0, 0)));
}

#[test]
fn plugin_filter_rejects_cli_tags() {
    let accepted: Vec<&str> = AE2_TAGS
        .iter()
        .copied()
        .filter(|t| parse_line_tag(&PLUGIN_LINE, t).is_some())
        .collect();
    assert_eq!(
        accepted,
        ["ss-magic-plugin-v1.0.0"],
        "the plugin line accepts only its own prefixed tag"
    );
    assert_eq!(
        parse_line_tag(&PLUGIN_LINE, "ss-magic-plugin-v1.0.0"),
        Some((1, 0, 0))
    );
}

#[test]
fn filters_are_anchored_at_the_start() {
    for tag in ["xv1.2.3", "release-v1.2.3", "ss-magic-v1.2.3", "vv1.2.3"] {
        assert_eq!(
            parse_line_tag(&CLI_LINE, tag),
            None,
            "{tag}: the CLI prefix must sit at byte 0 with nothing before it"
        );
        assert_eq!(
            parse_line_tag(&PLUGIN_LINE, tag),
            None,
            "{tag}: the plugin prefix must sit at byte 0 with nothing before it"
        );
    }
}

#[test]
fn parse_line_tag_requires_three_ascii_digit_components() {
    // `u64::from_str` accepts a leading `+`, so a digit check has to be
    // explicit or `v1.+2.3` would parse.
    for tag in [
        "v1.+2.3", "v1. 2.3", "v1..3", "v.1.2", "v1.2.3.", "v1.2.", "v1", "v",
        "v-1.2.3", "v1.2.3\n",
    ] {
        assert_eq!(parse_line_tag(&CLI_LINE, tag), None, "{tag:?} must be rejected");
    }
    assert_eq!(parse_line_tag(&CLI_LINE, "v0.11.10"), Some((0, 11, 10)));
    assert_eq!(parse_line_tag(&CLI_LINE, "v10.200.3000"), Some((10, 200, 3000)));
}

#[test]
fn the_two_lines_have_distinct_prefixes_and_cache_files() {
    assert_ne!(CLI_LINE.tag_prefix, PLUGIN_LINE.tag_prefix);
    assert_ne!(CLI_LINE.cache_file, PLUGIN_LINE.cache_file);
    assert_eq!(CLI_LINE.tag_prefix, "v");
    assert_eq!(PLUGIN_LINE.tag_prefix, "ss-magic-plugin-v");
}

// ── select_newest (R16, AE1) ───────────────────────────────────────────

#[test]
fn select_newest_takes_the_greatest_triple_not_the_first() {
    assert_eq!(
        select_newest(&CLI_LINE, &ae1_releases()).as_deref(),
        Some("v0.11.10"),
        "v0.11.10 beats v0.11.3 numerically even though it is listed later"
    );
    assert_eq!(
        select_newest(&PLUGIN_LINE, &ae1_releases()).as_deref(),
        Some("ss-magic-plugin-v1.2.0"),
        "the plugin line sees only its own tag in the same list"
    );
}

#[test]
fn select_newest_drops_drafts_and_prereleases() {
    let releases = vec![
        draft("v9.0.0"),
        prerelease("v8.0.0"),
        published("v1.0.0"),
    ];
    assert_eq!(select_newest(&CLI_LINE, &releases).as_deref(), Some("v1.0.0"));
}

#[test]
fn select_newest_compares_numerically_not_lexically() {
    let releases = vec![published("v0.9.0"), published("v0.10.0")];
    assert_eq!(select_newest(&CLI_LINE, &releases).as_deref(), Some("v0.10.0"));
    let releases = vec![published("v0.10.0"), published("v0.9.0")];
    assert_eq!(select_newest(&CLI_LINE, &releases).as_deref(), Some("v0.10.0"));
}

#[test]
fn select_newest_is_none_for_an_empty_or_foreign_list() {
    assert_eq!(select_newest(&CLI_LINE, &[]), None);
    let only_plugin = vec![published("ss-magic-plugin-v1.0.0")];
    assert_eq!(select_newest(&CLI_LINE, &only_plugin), None);
}

// ── resolve_newest_uncached (R18's resolver for the force path) ────────

#[test]
fn resolve_newest_uncached_returns_the_selected_tag_and_sends_no_etag() {
    let client = StubClient::new(FetchOutcome::Ok {
        releases: ae1_releases(),
        etag: Some("\"abc\"".to_string()),
    });
    let tag = resolve_newest_uncached(&client, &CLI_LINE);
    assert_eq!(tag.as_deref(), Some("v0.11.10"));
    assert!(client.called.get());
    assert_eq!(client.seen_etag.take(), None, "uncached: no If-None-Match");
}

#[test]
fn resolve_newest_uncached_is_none_on_failure_or_not_modified() {
    let client = StubClient::new(FetchOutcome::Failed);
    assert_eq!(resolve_newest_uncached(&client, &CLI_LINE), None);
    // A 304 without a sent ETag is not a valid answer to act on.
    let client = StubClient::new(FetchOutcome::NotModified);
    assert_eq!(resolve_newest_uncached(&client, &CLI_LINE), None);
}

// ── Wire format ────────────────────────────────────────────────────────

#[test]
fn release_list_fixture_with_unknown_keys_parses() {
    // The real `/releases` shape carries many more keys than we read; the
    // projection must ignore them and default a missing `draft`/`prerelease`.
    let body = r#"[
      {
        "url": "https://api.github.com/repos/o/r/releases/1",
        "id": 1, "node_id": "RE_1", "name": "ss-magic-plugin 1.2.0",
        "tag_name": "ss-magic-plugin-v1.2.0", "target_commitish": "main",
        "draft": false, "prerelease": false,
        "created_at": "2026-09-01T00:00:00Z", "published_at": "2026-09-01T00:00:00Z",
        "assets": [{"name": "ss-magic-plugin-v1.2.0.zip", "size": 1}],
        "body": "notes"
      },
      { "id": 2, "tag_name": "v0.11.10", "assets": [] },
      { "id": 3, "tag_name": "v0.12.0", "draft": true }
    ]"#;
    let releases: Vec<ReleaseEntry> = serde_json::from_str(body).unwrap();
    assert_eq!(releases.len(), 3);
    assert_eq!(releases[1].tag_name, "v0.11.10");
    assert!(!releases[1].draft && !releases[1].prerelease, "missing keys default to false");
    assert!(releases[2].draft);
    assert_eq!(select_newest(&CLI_LINE, &releases).as_deref(), Some("v0.11.10"));
}

// ── Version compare ────────────────────────────────────────────────────

#[test]
fn is_newer_handles_v_prefix_and_components() {
    assert!(is_newer(&CLI_LINE, "v1.2.3", "1.2.2"));
    assert!(is_newer(&CLI_LINE, "v1.3.0", "1.2.9"));
    assert!(is_newer(&CLI_LINE, "v2.0.0", "1.9.9"));
    assert!(!is_newer(&CLI_LINE, "v1.2.3", "1.2.3")); // equal
    assert!(!is_newer(&CLI_LINE, "v1.2.2", "1.2.3")); // lower
    assert!(!is_newer(&CLI_LINE, "v1.0.0", "1.0.0"));
    // The filter is anchored: a bare triple is not a CLI-line tag any more
    // (it used to be accepted by the optional-`v` strip).
    assert!(!is_newer(&CLI_LINE, "1.3.0", "1.2.9"));
}

#[test]
fn is_newer_treats_unparseable_tag_as_not_newer() {
    assert!(!is_newer(&CLI_LINE, "not-a-version", "1.0.0"));
    assert!(!is_newer(&CLI_LINE, "v1.2", "1.0.0")); // too few components
    assert!(!is_newer(&CLI_LINE, "v1.2.3.4", "1.0.0")); // too many components
    assert!(!is_newer(&CLI_LINE, "v1.2.3-beta", "1.0.0")); // suffix
    assert!(!is_newer(&CLI_LINE, "ss-magic-plugin-v9.0.0", "1.0.0")); // other line
}

// ── AE1: the CLI at 0.11.3 selects v0.11.10 from the mixed list ─────────

#[test]
fn ae1_cli_at_0_11_3_selects_v0_11_10_from_the_mixed_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir); // missing → stale → fetch
    let client = StubClient::new(FetchOutcome::Ok {
        releases: ae1_releases(),
        etag: Some("\"list\"".to_string()),
    });
    let verdict = run_check(&path, &client, &CLI_LINE, "0.11.3");
    assert_eq!(
        verdict,
        UpdateCheck::Newer {
            tag: "v0.11.10".to_string()
        }
    );
    let after = read_cache_file(&path);
    assert_eq!(after.tag_name, "v0.11.10", "the SELECTED tag is what the cache stores");
    assert_eq!(after.etag.as_deref(), Some("\"list\""));
}

// ── AE4: a fresh cache holding a plugin tag is harmless ─────────────────

#[test]
fn ae4_fresh_cache_holding_a_plugin_tag_is_up_to_date_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs(),
            tag_name: "ss-magic-plugin-v1.0.0".to_string(),
            etag: None,
            suggested: None,
        },
    );
    let client = StubClient::ok_with_tags(&["v9.9.9"], None);
    let verdict = run_check(&path, &client, &CLI_LINE, "0.11.0");
    assert_eq!(verdict, UpdateCheck::UpToDate, "a plugin tag never moves the CLI");
    assert!(!client.called.get(), "fresh cache: no network");
}

// ── Stale cache + injected failure → no update, refresh time ────────────

#[test]
fn stale_cache_plus_failure_returns_up_to_date_and_refreshes_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    // Stale: checked a week ago. A high prior tag must NOT leak through.
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs() - 7 * 24 * 60 * 60,
            tag_name: "v9.9.9".to_string(),
            etag: Some("\"prior\"".to_string()),
            suggested: None,
        },
    );

    let client = StubClient::new(FetchOutcome::Failed);
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");

    assert_eq!(verdict, UpdateCheck::UpToDate, "failure → no update");
    assert!(client.called.get(), "stale cache must hit the network seam");

    // checked_at refreshed to ~now; tag/etag preserved (not trusted-new).
    let after = read_cache_file(&path);
    assert!(
        is_fresh(after.checked_at, now_secs()),
        "checked_at must be refreshed"
    );
    assert_eq!(after.tag_name, "v9.9.9", "prior tag preserved on failure");
}

// ── Fresh cache → NO network call, cached verdict ───────────────────────

#[test]
fn fresh_cache_skips_network_and_returns_cached_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs(), // fresh
            tag_name: "v2.0.0".to_string(),
            etag: None,
            suggested: None,
        },
    );

    // If the client were called it would return a *lower* tag; the fresh
    // cached "v2.0.0" must win, proving no call happened.
    let client = StubClient::ok_with_tags(&["v0.0.1"], None);
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");

    assert!(
        !client.called.get(),
        "fresh cache must NOT invoke the network seam"
    );
    assert_eq!(
        verdict,
        UpdateCheck::Newer {
            tag: "v2.0.0".to_string()
        }
    );
}

#[test]
fn fresh_cache_with_no_newer_tag_returns_up_to_date_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs(),
            tag_name: "v1.0.0".to_string(),
            etag: None,
            suggested: None,
        },
    );
    let client = StubClient::new(FetchOutcome::Failed);
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");
    assert!(!client.called.get());
    assert_eq!(verdict, UpdateCheck::UpToDate);
}

// ── 200 with higher / equal / lower tags ────────────────────────────────

#[test]
fn http_200_higher_tag_reports_newer() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir); // missing file → stale
    let client = StubClient::ok_with_tags(&["v1.5.0"], Some("\"abc\""));
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");
    assert_eq!(
        verdict,
        UpdateCheck::Newer {
            tag: "v1.5.0".to_string()
        }
    );
    // Tag + etag stored for next time.
    let after = read_cache_file(&path);
    assert_eq!(after.tag_name, "v1.5.0");
    assert_eq!(after.etag.as_deref(), Some("\"abc\""));
}

#[test]
fn http_200_equal_tag_reports_up_to_date() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    let client = StubClient::ok_with_tags(&["v1.0.0"], None);
    assert_eq!(
        run_check(&path, &client, &CLI_LINE, "1.0.0"),
        UpdateCheck::UpToDate
    );
}

#[test]
fn http_200_lower_tag_reports_up_to_date() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    let client = StubClient::ok_with_tags(&["v0.9.0"], None);
    assert_eq!(
        run_check(&path, &client, &CLI_LINE, "1.0.0"),
        UpdateCheck::UpToDate
    );
}

#[test]
fn http_200_empty_list_reports_up_to_date_and_stores_no_tag() {
    // A line that has not released among the 100 newest releases reads as
    // "no update" (the documented, conservative pagination outcome).
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs() - 7 * 24 * 60 * 60,
            tag_name: "v9.9.9".to_string(),
            etag: None,
            suggested: None,
        },
    );
    let client = StubClient::ok_with_tags(&[], Some("\"empty\""));
    assert_eq!(
        run_check(&path, &client, &CLI_LINE, "1.0.0"),
        UpdateCheck::UpToDate
    );
    let after = read_cache_file(&path);
    assert_eq!(after.tag_name, "", "a successful fetch with no match replaces the prior tag");
    assert_eq!(after.etag.as_deref(), Some("\"empty\""));
    assert!(is_fresh(after.checked_at, now_secs()));
}

// ── 304 Not Modified → keep tag, retain etag, no update ─────────────────

#[test]
fn http_304_keeps_tag_and_retains_etag() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    // Stale prior with a tag equal to current and a stored etag.
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs() - 7 * 24 * 60 * 60,
            tag_name: "v1.0.0".to_string(),
            etag: Some("\"etag-1\"".to_string()),
            suggested: None,
        },
    );
    let client = StubClient::new(FetchOutcome::NotModified);
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");

    assert_eq!(verdict, UpdateCheck::UpToDate);
    // The stored etag must have been sent as If-None-Match.
    assert_eq!(client.seen_etag.take().as_deref(), Some("\"etag-1\""));
    // Cache retains tag + etag, refreshes time.
    let after = read_cache_file(&path);
    assert_eq!(after.tag_name, "v1.0.0");
    assert_eq!(after.etag.as_deref(), Some("\"etag-1\""));
    assert!(is_fresh(after.checked_at, now_secs()));
}

#[test]
fn http_304_with_newer_prior_tag_reports_newer() {
    // 304 means "the list you cached is unchanged"; if the tag selected
    // from it is newer than the running binary, the verdict is Newer.
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs() - 7 * 24 * 60 * 60,
            tag_name: "v2.0.0".to_string(),
            etag: Some("\"etag-2\"".to_string()),
            suggested: None,
        },
    );
    let client = StubClient::new(FetchOutcome::NotModified);
    assert_eq!(
        run_check(&path, &client, &CLI_LINE, "1.0.0"),
        UpdateCheck::Newer {
            tag: "v2.0.0".to_string()
        }
    );
}

// ── Non-200 (rate-limit body) → no update, no panic ─────────────────────

#[test]
fn non_200_is_treated_as_no_update() {
    // A 403 rate-limit normalizes to Failed at the seam; the core never
    // panics and reports UpToDate.
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    let client = StubClient::new(FetchOutcome::Failed);
    assert_eq!(
        run_check(&path, &client, &CLI_LINE, "1.0.0"),
        UpdateCheck::UpToDate
    );
    // Cache still written (time refreshed) so we don't re-hit immediately.
    let after = read_cache_file(&path);
    assert!(is_fresh(after.checked_at, now_secs()));
}

// ── Malformed / missing cache → treated as stale, recreated ─────────────

#[test]
fn malformed_cache_is_treated_as_stale_and_recreated() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    std::fs::write(&path, "{ this is not valid json").unwrap();

    let client = StubClient::ok_with_tags(&["v3.0.0"], Some("\"fresh\""));
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");

    assert!(client.called.get(), "malformed cache → stale → network call");
    assert_eq!(
        verdict,
        UpdateCheck::Newer {
            tag: "v3.0.0".to_string()
        }
    );
    // Recreated as valid JSON.
    let after = read_cache_file(&path);
    assert_eq!(after.tag_name, "v3.0.0");
    assert_eq!(after.etag.as_deref(), Some("\"fresh\""));
}

#[test]
fn missing_cache_is_treated_as_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir); // does not exist yet
    let client = StubClient::ok_with_tags(&["v1.1.0"], None);
    let verdict = run_check(&path, &client, &CLI_LINE, "1.0.0");
    assert!(client.called.get());
    assert_eq!(
        verdict,
        UpdateCheck::Newer {
            tag: "v1.1.0".to_string()
        }
    );
    assert!(path.is_file(), "cache file must be created");
}

#[test]
fn first_run_with_no_etag_sends_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    let client = StubClient::ok_with_tags(&["v1.0.0"], Some("\"new\""));
    let _ = run_check(&path, &client, &CLI_LINE, "1.0.0");
    assert_eq!(client.seen_etag.take(), None, "no prior etag → None sent");
}

#[test]
fn pre_split_cache_file_shape_still_parses() {
    // The on-disk `Cache` shape is unchanged so a file written by an older
    // binary keeps working; it is read, not rejected as malformed.
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    std::fs::write(
        &path,
        format!(
            r#"{{ "checked_at": {}, "tag_name": "v1.5.0", "etag": "\"old\"" }}"#,
            now_secs()
        ),
    )
    .unwrap();
    let client = StubClient::new(FetchOutcome::Failed);
    assert_eq!(
        run_check(&path, &client, &CLI_LINE, "1.0.0"),
        UpdateCheck::Newer {
            tag: "v1.5.0".to_string()
        }
    );
    assert!(!client.called.get(), "a fresh pre-split cache is honored as-is");
}

/// This file must never read its own crate's version.
///
/// `env!("CARGO_PKG_VERSION")` expands to the version of the crate being
/// COMPILED, which for everything in this file is `ss-magic-core` – a library
/// version bumped only on an incompatible API change, deliberately decoupled
/// from the `ss-magic` release line the updater compares against. While this
/// code lived in the binary crate the two were the same string, so the macro
/// was correct there; the workspace split moved the file byte-for-byte and
/// changed what it meant, which no behavioural test could catch because every
/// other test injects `current_version` explicitly.
///
/// The live consequence was that `check()` reported `Newer` for every
/// published tag on every run (`0.11.0 > 0.1.0`), defeating the 24 h cache and
/// the ETag round-trip that exist to avoid exactly that request. A second
/// guard in the CLI's `update::apply` kept it from installing anything, which
/// is precisely why it was invisible.
///
/// Both the comparison version and the `User-Agent` are caller-supplied now.
/// This test is the guard that keeps them that way.
#[test]
fn release_never_reads_its_own_crate_version() {
    // Comments are stripped first: documenting the trap is the point, and a
    // scan that forbade naming it would push the explanation out of the file
    // it protects.
    let source = include_str!("../release.rs");
    let code: String = source
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("CARGO_PKG_VERSION"),
        "release.rs must take the caller's version as an argument, never read \
         env!(\"CARGO_PKG_VERSION\"), which here resolves to ss-magic-core's \
         own version rather than the CLI release line's"
    );
}

// ── refresh_cache (U6, KTD12) ────────────────────────────────────────────────

/// A prior plugin-line cache that has already announced its tag.
fn announced(tag: &str) -> Cache {
    Cache {
        checked_at: 1_000,
        tag_name: tag.to_string(),
        etag: Some("\"prior\"".to_string()),
        suggested: Some(tag.to_string()),
    }
}

/// A `Failed` fetch learns nothing: the prior tag, ETag and `suggested` marker
/// all survive, and only `checked_at` moves – so a flaky endpoint is not
/// hammered and the once-per-release notice is not re-armed by an outage.
#[test]
fn refresh_with_a_failed_client_bumps_checked_at_and_keeps_the_prior_record() {
    let client = StubClient::new(FetchOutcome::Failed);
    let prior = announced("ss-magic-plugin-v1.1.0");
    let refreshed = refresh_cache(Some(prior.clone()), &client, &PLUGIN_LINE, 5_000);

    assert_eq!(refreshed.outcome, RefreshOutcome::Failed);
    assert_eq!(refreshed.cache.checked_at, 5_000);
    assert_eq!(refreshed.cache.tag_name, prior.tag_name);
    assert_eq!(refreshed.cache.etag, prior.etag);
    assert_eq!(refreshed.cache.suggested, prior.suggested);
    assert_eq!(
        client.seen_etag.take().as_deref(),
        Some("\"prior\""),
        "the prior ETag is always sent"
    );
}

/// `304 Not Modified` is the same record with a newer `checked_at`.
#[test]
fn refresh_not_modified_keeps_tag_etag_and_suggested() {
    let client = StubClient::new(FetchOutcome::NotModified);
    let prior = announced("ss-magic-plugin-v1.1.0");
    let refreshed = refresh_cache(Some(prior.clone()), &client, &PLUGIN_LINE, 5_000);

    assert_eq!(refreshed.outcome, RefreshOutcome::NotModified);
    assert_eq!(
        refreshed.cache,
        Cache {
            checked_at: 5_000,
            ..prior
        }
    );
}

/// A successful fetch that re-selects the SAME tag carries the marker
/// forward: the daily refresh must not re-announce a release already shown.
#[test]
fn refresh_that_reselects_the_same_tag_carries_suggested_forward() {
    let client = StubClient::ok_with_tags(
        &["v0.11.1", "ss-magic-plugin-v1.1.0", "ss-magic-plugin-v1.0.0"],
        Some("\"fresh\""),
    );
    let refreshed = refresh_cache(
        Some(announced("ss-magic-plugin-v1.1.0")),
        &client,
        &PLUGIN_LINE,
        5_000,
    );

    assert_eq!(refreshed.outcome, RefreshOutcome::Fetched);
    assert_eq!(refreshed.cache.tag_name, "ss-magic-plugin-v1.1.0");
    assert_eq!(refreshed.cache.etag.as_deref(), Some("\"fresh\""));
    assert_eq!(
        refreshed.cache.suggested.as_deref(),
        Some("ss-magic-plugin-v1.1.0")
    );
}

/// A successful fetch that selects a DIFFERENT tag clears the marker, so the
/// new release gets its own once-per-release notice.
#[test]
fn refresh_that_selects_a_different_tag_clears_suggested() {
    let client = StubClient::ok_with_tags(
        &["ss-magic-plugin-v1.2.0", "ss-magic-plugin-v1.1.0"],
        None,
    );
    let refreshed = refresh_cache(
        Some(announced("ss-magic-plugin-v1.1.0")),
        &client,
        &PLUGIN_LINE,
        5_000,
    );

    assert_eq!(refreshed.cache.tag_name, "ss-magic-plugin-v1.2.0");
    assert_eq!(refreshed.cache.suggested, None);
}

/// A list with no tag of this line at all selects the empty tag – which is a
/// different tag from a non-empty prior one, so the marker clears with it.
#[test]
fn refresh_that_finds_no_tag_of_this_line_stores_an_empty_tag_and_clears_suggested() {
    let client = StubClient::ok_with_tags(&["v0.11.1", "v0.11.0"], None);
    let refreshed = refresh_cache(
        Some(announced("ss-magic-plugin-v1.1.0")),
        &client,
        &PLUGIN_LINE,
        5_000,
    );
    assert_eq!(refreshed.cache.tag_name, "");
    assert_eq!(refreshed.cache.suggested, None);
}

/// No prior cache at all: the refresh starts from the default record and
/// sends no ETag.
#[test]
fn refresh_from_no_prior_cache_sends_no_etag() {
    let client = StubClient::ok_with_tags(&["ss-magic-plugin-v1.0.0"], Some("\"e\""));
    let refreshed = refresh_cache(None, &client, &PLUGIN_LINE, 5_000);
    assert_eq!(client.seen_etag.take(), None);
    assert_eq!(
        refreshed.cache,
        Cache {
            checked_at: 5_000,
            tag_name: "ss-magic-plugin-v1.0.0".to_string(),
            etag: Some("\"e\"".to_string()),
            suggested: None,
        }
    );
}

/// The CLI's cache file must not change shape: `suggested` is skipped when
/// `None`, so `version-check.json` serializes exactly as it did before the
/// field existed – and a file carrying the key still reads back.
#[test]
fn suggested_is_absent_from_the_serialized_cache_unless_set() {
    let plain = Cache {
        checked_at: 1,
        tag_name: "v1.0.0".to_string(),
        etag: None,
        suggested: None,
    };
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("suggested"), "{json}");

    let marked = Cache {
        suggested: Some("v1.0.0".to_string()),
        ..plain.clone()
    };
    let json = serde_json::to_string(&marked).unwrap();
    assert!(json.contains("\"suggested\":\"v1.0.0\""), "{json}");
    assert_eq!(serde_json::from_str::<Cache>(&json).unwrap(), marked);

    // A pre-U6 file (no key) reads as `None`.
    let legacy = r#"{"checked_at":1,"tag_name":"v1.0.0","etag":null}"#;
    assert_eq!(serde_json::from_str::<Cache>(legacy).unwrap(), plain);
}

/// `Cache::is_fresh` treats an unset `checked_at` as never fresh.
#[test]
fn a_zero_checked_at_is_never_fresh() {
    let cache = Cache::default();
    assert!(!cache.is_fresh(0));
    assert!(!cache.is_fresh(10));
    let checked = Cache {
        checked_at: 1_000,
        ..Cache::default()
    };
    assert!(checked.is_fresh(1_000 + FRESH_FOR.as_secs() - 1));
    assert!(!checked.is_fresh(1_000 + FRESH_FOR.as_secs()));
}

/// The plugin identifies itself as its own product in the user agent.
#[test]
fn for_product_names_the_calling_binary() {
    let client = UreqReleaseClient::for_product("ss-magic-plugin", "1.0.0");
    assert_eq!(client.user_agent, "ss-magic-plugin/1.0.0");
    let cli = UreqReleaseClient::new("0.11.1");
    assert_eq!(cli.user_agent, "ss-magic/0.11.1");
}
