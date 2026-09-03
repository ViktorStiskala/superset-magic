//! Daily-cached "is a newer release available?" check, resolved per release
//! LINE.
//!
//! One GitHub repository publishes two independent release lines: the CLI on
//! bare `vMAJOR.MINOR.PATCH` tags and the Claude Code plugin on
//! `ss-magic-plugin-vMAJOR.MINOR.PATCH` tags. The repository-wide
//! `releases/latest` mark therefore no longer identifies "the newest CLI
//! release" – it names whichever line published most recently – so the check
//! lists releases and filters them itself (R16): fetch the first page of
//! `/releases`, drop drafts and prereleases, keep only the tags that match
//! this binary's [`Line`] under the anchored [`parse_line_tag`] filter (R17),
//! and take the greatest `(major, minor, patch)` rather than the first entry
//! (the list is in creation order, and a patch release of an older minor can
//! be created after a newer minor).
//!
//! On every invocation the caller asks: is there a newer `ss-magic` release?
//! Answering must be cheap and never break an offline or rate-limited run, so
//! the SELECTED tag is cached in the OS cache dir for 24h and the network is
//! only touched when the cache is stale. Every failure mode – offline,
//! timeout, 403 rate-limit, a body that is not a JSON array, malformed cache,
//! unparseable tag – collapses silently to [`UpdateCheck::UpToDate`] (R19).
//! Nothing here logs, panics, or returns an error to the caller; the public
//! [`check`] is infallible by design.
//!
//! Testability rests on two seams:
//! - The HTTP call lives behind [`ReleaseClient`]; tests inject
//!   200/304/timeout/non-200 outcomes with no network.
//! - The cache path is injected ([`run_check`] takes a `cache_file`), so tests
//!   point it at a tempdir rather than the real OS cache dir.
//!
//! `check` wires the real ureq client + the real OS cache path + [`CLI_LINE`]
//! on top of `run_check`. [`resolve_newest_uncached`] is the cache-free
//! variant the forced `ss-magic update` path uses to pin the tag it hands to
//! the download backend (R18).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// GitHub owner/repo slug. ONE repository hosts BOTH release lines, which is
/// exactly why every consumer must filter by [`Line`]: the slug alone does
/// not identify a line. The CLI's `update::apply` reuses it for the download
/// backend, so it is `pub` rather than module-private.
pub const REPO_SLUG: &str = "ViktorStiskala/superset-magic";

/// How long a cache entry is trusted before we re-check the network.
const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// Per-request network budget for the release check. Applied as ureq's
/// global timeout so connect + send + receive together can't exceed it.
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// Page size of the ONE `/releases` request the check makes. GitHub caps a
/// page at 100, and only the first page is read: a line whose newest release
/// is not among the 100 most recently created releases reads as "no update".
/// That is the conservative direction (a missed update, never a wrong one)
/// and, with two lines sharing the repository, far beyond any realistic gap.
const RELEASES_PER_PAGE: u32 = 100;

/// A release line: the anchored tag shape a consumer accepts and the cache
/// file its selected tag is remembered in. Two lines share [`REPO_SLUG`]; the
/// prefix is what keeps the CLI from ever "updating" to a plugin release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    /// Exact byte prefix a tag must start with; what follows must be exactly
    /// `MAJOR.MINOR.PATCH` (see [`parse_line_tag`]).
    pub tag_prefix: &'static str,
    /// File name of this line's check cache inside [`cache_dir`].
    pub cache_file: &'static str,
}

/// The `ss-magic` CLI: bare `vX.Y.Z` tags, the shape every release to date
/// has used, cached in the pre-existing `version-check.json` so a cache file
/// written by an older binary is still read.
pub const CLI_LINE: Line = Line {
    tag_prefix: "v",
    cache_file: "version-check.json",
};

/// The Claude Code plugin: `ss-magic-plugin-vX.Y.Z` tags (cargo-dist's
/// native `<package>-v<version>` form for the `ss-magic-plugin` package), in
/// its own cache file so the two lines never overwrite each other's answer.
#[allow(dead_code)] // consumed by U6: the plugin's `release-check` verb and its SessionStart suggestion
pub const PLUGIN_LINE: Line = Line {
    tag_prefix: "ss-magic-plugin-v",
    cache_file: "plugin-release-check.json",
};

/// Verdict handed back to the caller (the auto-update gate acts on `Newer`).
///
/// Infallible by contract: any failure inside the check collapses to
/// `UpToDate`, so the caller never has to reason about errors here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    /// No newer release than the running binary (or we couldn't tell).
    UpToDate,
    /// A newer release exists; `tag` is its full `tag_name` (e.g. `v1.2.3`),
    /// already known to pass this line's filter.
    Newer { tag: String },
}

/// Minimal projection of one entry of GitHub's `/releases` list JSON. Every
/// field defaults so an entry missing a key (or a fixture that omits the
/// dozens of keys we never read) still parses; a missing `tag_name` yields an
/// empty string, which no line's filter accepts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ReleaseEntry {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
}

/// Outcome of a single release-list fetch, normalized away from the HTTP
/// transport so the core logic and tests share one vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// `200 OK` – `releases` is the decoded first page (drafts and
    /// prereleases still included; [`select_newest`] drops them); `etag` is
    /// the response `ETag` header when present (used to short-circuit future
    /// checks).
    Ok {
        releases: Vec<ReleaseEntry>,
        etag: Option<String>,
    },
    /// `304 Not Modified` – the list is unchanged, so the cached selected tag
    /// is still current.
    NotModified,
    /// Timeout, offline, any non-200/304 status (incl. 403 rate-limit), or a
    /// body that is not a JSON array. The offline-safe fall-through: treated
    /// as "no update".
    Failed,
}

/// The HTTP seam. The real impl talks to GitHub via ureq; tests inject
/// canned outcomes without a network. `etag` is the value of a cached `ETag`,
/// sent as `If-None-Match` to let GitHub answer `304`.
pub trait ReleaseClient {
    fn fetch_releases(&self, etag: Option<&str>) -> FetchOutcome;
}

/// On-disk cache record. Lives at the injected cache path as pretty JSON.
///
/// `checked_at` is unix epoch seconds of the last check (success OR silent
/// failure – every network attempt refreshes it so we don't hammer a flaky
/// endpoint). `tag_name`/`etag` carry the last successfully observed values.
/// The shape is deliberately unchanged from the `releases/latest` era so a
/// cache file written by an older binary still parses; `tag_name` now holds
/// the tag SELECTED by this line's filter rather than whatever GitHub marked
/// latest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    /// Unix epoch seconds of the last check attempt.
    #[serde(default)]
    pub checked_at: u64,
    /// Newest tag of this line last selected from GitHub's list (empty until
    /// the first successful fetch, or when the list held no match).
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

/// The anchored, exact tag filter of a release line (R17).
///
/// Accepts `tag` only when it is `line.tag_prefix` immediately followed by
/// exactly three ASCII-digit components separated by `.` and nothing else,
/// and returns that `(major, minor, patch)`. Everything else is `None`: the
/// other line's tags, `ss-magic-v…`, a pre-release suffix, a fourth
/// component, a case variant (`V1.0.0`), anything before the prefix
/// (`xv1.0.0`) and a doubled prefix (`vv1.0.0`). The prefix match is a byte
/// `strip_prefix`, so it is anchored at the start by construction rather
/// than by a regex anchor someone could drop.
pub fn parse_line_tag(line: &Line, tag: &str) -> Option<(u64, u64, u64)> {
    let rest = tag.strip_prefix(line.tag_prefix)?;
    parse_bare_triple(rest)
}

/// Parse exactly `MAJOR.MINOR.PATCH` (no prefix) into a triple. This is the
/// shape of `CARGO_PKG_VERSION` and of the remainder of a line tag after its
/// prefix. Each component must be non-empty and all ASCII digits – checked
/// explicitly, because `u64::from_str` accepts a leading `+` and would let
/// `v1.+2.3` through.
fn parse_bare_triple(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.split('.');
    let major = ascii_digits(parts.next()?)?;
    let minor = ascii_digits(parts.next()?)?;
    let patch = ascii_digits(parts.next()?)?;
    if parts.next().is_some() {
        // A fourth component (or a trailing `.`): not a plain triple.
        return None;
    }
    Some((major, minor, patch))
}

/// A non-empty run of ASCII digits, as a number. Anything else is `None`.
fn ascii_digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok()
}

/// Pick this line's newest release out of a fetched list (R16).
///
/// Drafts and prereleases are dropped (preserving the semantics of the
/// `releases/latest` mark this replaced, which never pointed at either), the
/// remaining tags are run through [`parse_line_tag`], and the greatest
/// triple wins – NOT the first match, because the list is ordered by
/// creation time and a `v0.11.10` can be created after a `v0.12.0` draft.
/// The returned string is the entry's full `tag_name`, which is what the
/// download backend needs to pin. `None` when nothing in the list belongs
/// to this line.
pub fn select_newest(line: &Line, releases: &[ReleaseEntry]) -> Option<String> {
    releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| parse_line_tag(line, &r.tag_name).map(|triple| (triple, r)))
        .max_by_key(|(triple, _)| *triple)
        .map(|(_, r)| r.tag_name.clone())
}

/// Resolve this line's newest release tag straight from the network, with no
/// cache read or write and no `If-None-Match` (so a 304 cannot occur and is
/// treated as "no answer" if a client returns one anyway).
///
/// This is the resolver behind the forced `ss-magic update` path (R18): the
/// caller pins whatever tag comes back into the download backend, and a
/// `None` – offline, non-200, malformed body, or no tag of this line in the
/// list – means the caller must report that it could not check, never that
/// the binary is already latest.
pub fn resolve_newest_uncached<C: ReleaseClient>(client: &C, line: &Line) -> Option<String> {
    match client.fetch_releases(None) {
        FetchOutcome::Ok { releases, .. } => select_newest(line, &releases),
        FetchOutcome::NotModified | FetchOutcome::Failed => None,
    }
}

/// Compare a release `tag_name` against the running binary's version.
///
/// The tag must pass the line's anchored filter ([`parse_line_tag`]) and
/// `current` must be a bare `X.Y.Z` (it is `env!("CARGO_PKG_VERSION")`); the
/// two triples are compared numerically. A tag that doesn't parse – including
/// the OTHER line's tag – is treated conservatively as "not newer" → no
/// update. This is what makes a stale cache holding a plugin tag harmless to
/// the CLI (AE4).
pub fn is_newer(line: &Line, tag: &str, current: &str) -> bool {
    match (parse_line_tag(line, tag), parse_bare_triple(current)) {
        (Some(t), Some(c)) => t > c,
        // Unparseable tag (or current) → conservative "no update".
        _ => false,
    }
}

/// Testable core of the daily-cached check.
///
/// `cache_file` is the injected cache path; `client` is the injected HTTP
/// seam; `line` is the release line whose tags count; `current_version` is
/// the running binary's version (no leading `v`).
///
/// Flow:
/// 1. Read the cache (missing/malformed → treated as stale).
/// 2. If FRESH (< 24h) → use the cached tag, NO network call.
/// 3. Else fetch via `client` (sending the cached ETag as `If-None-Match`):
///    - `Ok` → select this line's newest tag from the list, store it (empty
///      when the list holds none) + etag, refresh `checked_at`, verdict from
///      the selected tag.
///    - `NotModified` → keep the stored tag, refresh `checked_at` (+ etag),
///      verdict from the retained tag.
///    - `Failed` → refresh `checked_at` ONLY, verdict `UpToDate` (silent).
/// 4. Compare the resolved tag to `current_version` through the line filter.
pub fn run_check<C: ReleaseClient>(
    cache_file: &Path,
    client: &C,
    line: &Line,
    current_version: &str,
) -> UpdateCheck {
    let now = now_secs();
    let cached = read_cache(cache_file);

    // FRESH cache → no network. Verdict purely from the stored tag.
    if let Some(cache) = &cached {
        if cache.checked_at != 0 && is_fresh(cache.checked_at, now) {
            return verdict_from_tag(line, &cache.tag_name, current_version);
        }
    }

    // STALE (or missing/malformed) → hit the network behind the seam.
    let prior = cached.unwrap_or_default();
    let outcome = client.fetch_releases(prior.etag.as_deref());
    let failed = matches!(outcome, FetchOutcome::Failed);

    let next = match outcome {
        FetchOutcome::Ok { releases, etag } => Cache {
            checked_at: now,
            // A successful fetch is authoritative: a list with no tag of this
            // line stores an empty tag (no update), replacing any prior one.
            tag_name: select_newest(line, &releases).unwrap_or_default(),
            etag,
        },
        FetchOutcome::NotModified => Cache {
            checked_at: now,
            // Keep the last-selected tag; retain the etag we sent.
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

    verdict_from_tag(line, &next.tag_name, current_version)
}

/// Turn a stored tag into a verdict against `current`. Empty tag (never
/// successfully fetched, or nothing of this line in the list) → `UpToDate`.
fn verdict_from_tag(line: &Line, tag: &str, current: &str) -> UpdateCheck {
    if !tag.is_empty() && is_newer(line, tag, current) {
        UpdateCheck::Newer {
            tag: tag.to_string(),
        }
    } else {
        UpdateCheck::UpToDate
    }
}

/// Resolve the app-scoped OS cache dir for the version caches.
///
/// macOS `~/Library/Caches/ss-magic`, Linux XDG `~/.cache/ss-magic`, Windows
/// equivalent – via `directories::ProjectDirs`. Creates the dir if absent.
/// Returns `None` when the platform has no home dir (the caller then treats
/// the whole check as a silent no-op). Both lines' cache files live here,
/// under their own [`Line::cache_file`] names.
pub fn cache_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "ss-magic")?;
    let dir = dirs.cache_dir().to_path_buf();
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// The real HTTP client: GitHub's `/releases` list over ureq + rustls.
///
/// `http_status_as_error(false)` keeps 304/403/etc. as normal responses we
/// inspect via `.status()` rather than ureq errors. A 5s global timeout
/// bounds connect+send+recv. A `User-Agent` is sent because the GitHub API
/// rejects requests without one. The URL is per-repository, not per-line:
/// the same list serves both lines and each caller filters it with its own
/// [`Line`].
pub struct UreqReleaseClient {
    url: String,
    user_agent_version: String,
}

impl UreqReleaseClient {
    /// `user_agent_version` is the CALLING BINARY's version, and there is
    /// deliberately no `Default` that would fill it in from here.
    ///
    /// This file lives in `ss-magic-core`, so an `env!("CARGO_PKG_VERSION")`
    /// read at this level resolves to the LIBRARY's version (`0.1.0`, bumped
    /// only on an incompatible API change) rather than to the release line the
    /// caller actually ships on. The two were the same string while this code
    /// lived in the binary crate, and the workspace split silently decoupled
    /// them – the file moved byte-for-byte and changed meaning anyway. Making
    /// the version a required argument is what stops that from recurring:
    /// there is no construction path that can pick the wrong one.
    pub fn new(user_agent_version: &str) -> Self {
        Self {
            url: format!(
                "https://api.github.com/repos/{REPO_SLUG}/releases?per_page={RELEASES_PER_PAGE}"
            ),
            user_agent_version: user_agent_version.to_string(),
        }
    }
}

impl ReleaseClient for UreqReleaseClient {
    fn fetch_releases(&self, etag: Option<&str>) -> FetchOutcome {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("ss-magic/{}", self.user_agent_version))
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
            // 403 rate-limit, 404, 5xx, etc. – all "no update".
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
        match serde_json::from_str::<Vec<ReleaseEntry>>(&body) {
            Ok(releases) => FetchOutcome::Ok { releases, etag },
            // Not a JSON array (an error object, HTML, truncation) → can't
            // act, treat as fail. An EMPTY array is a valid `Ok`: the
            // repository simply has no releases yet.
            Err(_) => FetchOutcome::Failed,
        }
    }
}

/// Public entry point: wire the real OS cache path + real ureq client + the
/// CLI's own [`Line`], then run the cached check against the compiled-in
/// version. Infallible – any inability to resolve a cache dir collapses to
/// `UpToDate`.
pub fn check(current_version: &str) -> UpdateCheck {
    let Some(dir) = cache_dir() else {
        return UpdateCheck::UpToDate;
    };
    let cache_file = dir.join(CLI_LINE.cache_file);
    run_check(
        &cache_file,
        &UreqReleaseClient::new(current_version),
        &CLI_LINE,
        current_version,
    )
}

#[cfg(test)]
mod tests;
