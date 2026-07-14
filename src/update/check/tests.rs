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
}

impl ReleaseClient for StubClient {
    fn fetch_latest(&self, etag: Option<&str>) -> FetchOutcome {
        self.called.set(true);
        self.seen_etag.set(etag.map(|s| s.to_string()));
        self.outcome.clone()
    }
}

fn cache_path(dir: &TempDir) -> PathBuf {
    dir.path().join("version-check.json")
}

fn write_cache_file(path: &Path, cache: &Cache) {
    let body = serde_json::to_string_pretty(cache).unwrap();
    std::fs::write(path, body).unwrap();
}

fn read_cache_file(path: &Path) -> Cache {
    let raw = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

// ── Version compare ────────────────────────────────────────────────────

#[test]
fn is_newer_handles_v_prefix_and_components() {
    assert!(is_newer("v1.2.3", "1.2.2"));
    assert!(is_newer("1.3.0", "1.2.9"));
    assert!(is_newer("v2.0.0", "1.9.9"));
    assert!(!is_newer("v1.2.3", "1.2.3")); // equal
    assert!(!is_newer("v1.2.2", "1.2.3")); // lower
    assert!(!is_newer("v1.0.0", "1.0.0"));
}

#[test]
fn is_newer_treats_unparseable_tag_as_not_newer() {
    assert!(!is_newer("not-a-version", "1.0.0"));
    assert!(!is_newer("v1.2", "1.0.0")); // too few components
    assert!(!is_newer("v1.2.3.4", "1.0.0")); // too many components
    assert!(!is_newer("v1.2.3-beta", "1.0.0")); // suffix
}

// ── AE1: stale cache + injected failure → no update, refresh time ───────

#[test]
fn ae1_stale_cache_plus_failure_returns_up_to_date_and_refreshes_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    // Stale: checked a week ago. A high prior tag must NOT leak through.
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs() - 7 * 24 * 60 * 60,
            tag_name: "v9.9.9".to_string(),
            etag: Some("\"prior\"".to_string()),
        },
    );

    let client = StubClient::new(FetchOutcome::Failed);
    let verdict = run_check(&path, &client, "1.0.0");

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
        },
    );

    // If the client were called it would return a *lower* tag; the fresh
    // cached "v2.0.0" must win, proving no call happened.
    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v0.0.1".to_string(),
        etag: None,
    });
    let verdict = run_check(&path, &client, "1.0.0");

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
        },
    );
    let client = StubClient::new(FetchOutcome::Failed);
    let verdict = run_check(&path, &client, "1.0.0");
    assert!(!client.called.get());
    assert_eq!(verdict, UpdateCheck::UpToDate);
}

// ── 200 with higher / equal / lower tags ────────────────────────────────

#[test]
fn http_200_higher_tag_reports_newer() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir); // missing file → stale
    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v1.5.0".to_string(),
        etag: Some("\"abc\"".to_string()),
    });
    let verdict = run_check(&path, &client, "1.0.0");
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
    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v1.0.0".to_string(),
        etag: None,
    });
    assert_eq!(run_check(&path, &client, "1.0.0"), UpdateCheck::UpToDate);
}

#[test]
fn http_200_lower_tag_reports_up_to_date() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v0.9.0".to_string(),
        etag: None,
    });
    assert_eq!(run_check(&path, &client, "1.0.0"), UpdateCheck::UpToDate);
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
        },
    );
    let client = StubClient::new(FetchOutcome::NotModified);
    let verdict = run_check(&path, &client, "1.0.0");

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
    // 304 means "the tag you cached is still the latest"; if that cached
    // tag is newer than the running binary, the verdict is Newer.
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(&dir);
    write_cache_file(
        &path,
        &Cache {
            checked_at: now_secs() - 7 * 24 * 60 * 60,
            tag_name: "v2.0.0".to_string(),
            etag: Some("\"etag-2\"".to_string()),
        },
    );
    let client = StubClient::new(FetchOutcome::NotModified);
    assert_eq!(
        run_check(&path, &client, "1.0.0"),
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
    assert_eq!(run_check(&path, &client, "1.0.0"), UpdateCheck::UpToDate);
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

    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v3.0.0".to_string(),
        etag: Some("\"fresh\"".to_string()),
    });
    let verdict = run_check(&path, &client, "1.0.0");

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
    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v1.1.0".to_string(),
        etag: None,
    });
    let verdict = run_check(&path, &client, "1.0.0");
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
    let client = StubClient::new(FetchOutcome::Ok {
        tag: "v1.0.0".to_string(),
        etag: Some("\"new\"".to_string()),
    });
    let _ = run_check(&path, &client, "1.0.0");
    assert_eq!(client.seen_etag.take(), None, "no prior etag → None sent");
}
