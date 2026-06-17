//! Daily-cached "is a newer release available?" check (KTD2).
//!
//! On every invocation the caller asks: is there a newer `ss-magic` release?
//! Answering must be cheap and never break an offline or rate-limited run, so
//! the answer is cached in the OS cache dir for 24h and the network is only
//! touched when the cache is stale. Every failure mode — offline, timeout,
//! 403 rate-limit, malformed cache, unparseable tag — collapses silently to
//! [`UpdateCheck::UpToDate`]. Nothing here logs, panics, or returns an error
//! to the caller; the public [`check`] is infallible by design.
//!
//! Testability rests on two seams:
//! - The HTTP call lives behind [`ReleaseClient`]; tests inject
//!   200/304/timeout/non-200 outcomes with no network.
//! - The cache path is injected ([`run_check`] takes a `cache_file`), so tests
//!   point it at a tempdir rather than the real OS cache dir.
//!
//! `check` wires the real ureq client + the real OS cache path on top of
//! `run_check`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// GitHub owner/repo slug. Placeholder until the standalone repo exists (see
/// the plan's Open Questions); wired as a single constant to fill at split
/// time. `check`'s default client targets this repo's `releases/latest`;
/// U7's apply path (`super::apply`) reuses the same slug for the download
/// backend, so it is `pub(crate)` rather than module-private.
pub(crate) const REPO_SLUG: &str = "ViktorStiskala/superset-magic";

/// How long a cache entry is trusted before we re-check the network.
const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// Per-request network budget for the release check (R16). Applied as ureq's
/// global timeout so connect + send + receive together can't exceed it.
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// Verdict handed back to the caller (U7/U8 act on `Newer`).
///
/// Infallible by contract: any failure inside the check collapses to
/// `UpToDate`, so the caller never has to reason about errors here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    /// No newer release than the running binary (or we couldn't tell).
    UpToDate,
    /// A newer release exists; `tag` is its `tag_name` (e.g. `v1.2.3`).
    Newer { tag: String },
}

/// Outcome of a single release-endpoint fetch, normalized away from the HTTP
/// transport so the core logic and tests share one vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// `200 OK` — `tag` is the release `tag_name`; `etag` is the response
    /// `ETag` header when present (used to short-circuit future checks).
    Ok {
        tag: String,
        etag: Option<String>,
    },
    /// `304 Not Modified` — the cached tag is still current.
    NotModified,
    /// Timeout, offline, or any non-200/304 status (incl. 403 rate-limit).
    /// The offline-safe fall-through: treated as "no update".
    Failed,
}

/// The HTTP seam. The real impl talks to GitHub via ureq; tests inject
/// canned outcomes without a network. `etag` is the value of a cached `ETag`,
/// sent as `If-None-Match` to let GitHub answer `304`.
pub trait ReleaseClient {
    fn fetch_latest(&self, etag: Option<&str>) -> FetchOutcome;
}

/// On-disk cache record. Lives at the injected cache path as pretty JSON.
///
/// `checked_at` is unix epoch seconds of the last check (success OR silent
/// failure — every network attempt refreshes it so we don't hammer a flaky
/// endpoint). `tag_name`/`etag` carry the last successfully observed values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    /// Unix epoch seconds of the last check attempt.
    #[serde(default)]
    pub checked_at: u64,
    /// Latest `tag_name` last seen from GitHub (empty until first success).
    #[serde(default)]
    pub tag_name: String,
    /// Last `ETag` seen, sent as `If-None-Match` next time.
    #[serde(default)]
    pub etag: Option<String>,
}

/// Current unix time in seconds, saturating to 0 if the clock is before the
/// epoch (impossible in practice; keeps the function total).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read the cache file, treating any error (missing, unreadable, malformed
/// JSON) as "no usable cache" → `None`. A `None` here is what drives the
/// "stale, recreate" path.
fn read_cache(path: &Path) -> Option<Cache> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Cache>(&raw).ok()
}

/// Write the cache file (creating the parent dir if needed). Best-effort: a
/// write failure is swallowed so a read-only cache dir never breaks a run.
fn write_cache(path: &Path, cache: &Cache) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(path, body);
    }
}

/// True when `checked_at` is within [`FRESH_FOR`] of now.
fn is_fresh(checked_at: u64, now: u64) -> bool {
    now.saturating_sub(checked_at) < FRESH_FOR.as_secs()
}

/// Compare a release `tag_name` against the running binary's version.
///
/// Tags are plain `vX.Y.Z` (KTD11): strip an optional leading `v`, split on
/// `.`, parse three `u64`s, compare as a tuple. A tag that doesn't parse this
/// way is treated conservatively as "not newer" → no update. `current` is
/// expected to be `env!("CARGO_PKG_VERSION")` (no leading `v`), but the same
/// lenient strip is applied to it for symmetry.
fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_triple(tag), parse_triple(current)) {
        (Some(t), Some(c)) => t > c,
        // Unparseable tag (or current) → conservative "no update".
        _ => false,
    }
}

/// Parse `vX.Y.Z` / `X.Y.Z` into `(major, minor, patch)`. Returns `None`
/// unless there are exactly three dot-separated numeric components after an
/// optional leading `v`.
fn parse_triple(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        // Extra components (e.g. a pre-release suffix) — be conservative.
        return None;
    }
    Some((major, minor, patch))
}

/// Testable core of the daily-cached check.
///
/// `cache_file` is the injected cache path; `client` is the injected HTTP
/// seam; `current_version` is the running binary's version (no leading `v`).
///
/// Flow:
/// 1. Read the cache (missing/malformed → treated as stale).
/// 2. If FRESH (< 24h) → use the cached tag, NO network call.
/// 3. Else fetch via `client` (sending the cached ETag as `If-None-Match`):
///    - `Ok` → store tag + etag, refresh `checked_at`, verdict from the tag.
///    - `NotModified` → keep the stored tag, refresh `checked_at` (+ etag),
///      verdict from the retained tag.
///    - `Failed` → refresh `checked_at` ONLY, verdict `UpToDate` (silent).
/// 4. Compare the resolved tag to `current_version`.
pub fn run_check<C: ReleaseClient>(
    cache_file: &Path,
    client: &C,
    current_version: &str,
) -> UpdateCheck {
    let now = now_secs();
    let cached = read_cache(cache_file);

    // FRESH cache → no network. Verdict purely from the stored tag.
    if let Some(cache) = &cached {
        if cache.checked_at != 0 && is_fresh(cache.checked_at, now) {
            return verdict_from_tag(&cache.tag_name, current_version);
        }
    }

    // STALE (or missing/malformed) → hit the network behind the seam.
    let prior = cached.unwrap_or_default();
    let outcome = client.fetch_latest(prior.etag.as_deref());
    let failed = matches!(outcome, FetchOutcome::Failed);

    let next = match outcome {
        FetchOutcome::Ok { tag, etag } => Cache {
            checked_at: now,
            tag_name: tag,
            etag,
        },
        FetchOutcome::NotModified => Cache {
            checked_at: now,
            // Keep the last-seen tag; retain the etag we sent.
            tag_name: prior.tag_name,
            etag: prior.etag,
        },
        FetchOutcome::Failed => Cache {
            // Offline-safe fall-through: bump the timestamp ONLY, keep
            // whatever tag/etag we already had (don't trust the failed call).
            checked_at: now,
            tag_name: prior.tag_name,
            etag: prior.etag,
        },
    };

    // Persist the refreshed cache, then decide.
    write_cache(cache_file, &next);

    if failed {
        // A failure is always "no update", even if a stale prior tag happened
        // to be newer (we couldn't confirm it now).
        return UpdateCheck::UpToDate;
    }

    verdict_from_tag(&next.tag_name, current_version)
}

/// Turn a stored tag into a verdict against `current`. Empty tag (never
/// successfully fetched) → `UpToDate`.
fn verdict_from_tag(tag: &str, current: &str) -> UpdateCheck {
    if !tag.is_empty() && is_newer(tag, current) {
        UpdateCheck::Newer {
            tag: tag.to_string(),
        }
    } else {
        UpdateCheck::UpToDate
    }
}

/// Resolve the app-scoped OS cache dir for the version cache (R17).
///
/// macOS `~/Library/Caches/ss-magic`, Linux XDG `~/.cache/ss-magic`, Windows
/// equivalent — via `directories::ProjectDirs`. Creates the dir if absent.
/// Returns `None` when the platform has no home dir (the caller then treats
/// the whole check as a silent no-op).
pub fn cache_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "ss-magic")?;
    let dir = dirs.cache_dir().to_path_buf();
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// File name of the version-check cache inside [`cache_dir`].
const CACHE_FILE_NAME: &str = "version-check.json";

/// The real HTTP client: GitHub `releases/latest` over ureq + rustls (R16).
///
/// `http_status_as_error(false)` keeps 304/403/etc. as normal responses we
/// inspect via `.status()` rather than ureq errors. A 5s global timeout
/// bounds connect+send+recv. A `User-Agent` is sent because the GitHub API
/// rejects requests without one.
pub struct UreqReleaseClient {
    url: String,
}

impl Default for UreqReleaseClient {
    fn default() -> Self {
        Self {
            url: format!("https://api.github.com/repos/{REPO_SLUG}/releases/latest"),
        }
    }
}

/// Minimal projection of the GitHub release JSON we care about.
#[derive(Deserialize)]
struct LatestRelease {
    #[serde(default)]
    tag_name: String,
}

impl ReleaseClient for UreqReleaseClient {
    fn fetch_latest(&self, etag: Option<&str>) -> FetchOutcome {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("ss-magic/{}", env!("CARGO_PKG_VERSION")))
            .build();
        let agent: ureq::Agent = config.into();

        let mut req = agent
            .get(&self.url)
            .header("Accept", "application/vnd.github+json");
        if let Some(tag) = etag {
            req = req.header("If-None-Match", tag);
        }

        let mut resp = match req.call() {
            Ok(r) => r,
            // Timeout / offline / connection error → silent fall-through.
            Err(_) => return FetchOutcome::Failed,
        };

        let status = resp.status().as_u16();
        if status == 304 {
            return FetchOutcome::NotModified;
        }
        if status != 200 {
            // 403 rate-limit, 404, 5xx, etc. — all "no update".
            return FetchOutcome::Failed;
        }

        // Capture the ETag before consuming the body.
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Read the body as a string and parse with serde_json directly, so we
        // don't need ureq's optional `json` feature (keeps the dep minimal).
        let body = match resp.body_mut().read_to_string() {
            Ok(s) => s,
            Err(_) => return FetchOutcome::Failed,
        };
        match serde_json::from_str::<LatestRelease>(&body) {
            Ok(rel) if !rel.tag_name.is_empty() => FetchOutcome::Ok {
                tag: rel.tag_name,
                etag,
            },
            // Body unparseable or missing tag_name → can't act, treat as fail.
            _ => FetchOutcome::Failed,
        }
    }
}

/// Public entry point: wire the real OS cache path + real ureq client, then
/// run the cached check against the compiled-in version. Infallible — any
/// inability to resolve a cache dir collapses to `UpToDate`.
#[allow(dead_code)] // consumed by U8 (startup gate); the U7 force path bypasses it
pub fn check() -> UpdateCheck {
    let Some(dir) = cache_dir() else {
        return UpdateCheck::UpToDate;
    };
    let cache_file = dir.join(CACHE_FILE_NAME);
    run_check(
        &cache_file,
        &UreqReleaseClient::default(),
        env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
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
}
