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
mod tests;
