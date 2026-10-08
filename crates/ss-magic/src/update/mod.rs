//! Self-update subsystem.
//!
//! Split into the cheap gate and the heavy apply:
//! - [`ss_magic_core::release`] answers "is a newer release available?" cheaply and
//!   offline-safely — a 24h-cached, per-release-line check that never errors,
//!   never logs, and never blocks an offline or rate-limited run.
//! - [`apply`] is the heavy half: lock → download/verify/swap → re-exec,
//!   always pinned to a tag the check resolved.
//! - `main.rs` wires the auto-update gate into every gated entrypoint via
//!   [`auto_update`], and the explicit `ss-magic update` via
//!   [`update_command`].
//!
//! The two entry points differ only in how they gate the network:
//! - [`auto_update`] consults the 24h cache via [`release::check`]; it acts only
//!   on a `Newer` verdict, then re-execs the swapped binary so the caller's
//!   work runs on the new version.
//! - [`update_command`] (the `ss-magic update` force path) skips the cache
//!   entirely: it resolves the CLI line's newest tag straight from GitHub and
//!   only then hands that tag to `self_update`, reporting the resulting
//!   version, "already latest", or "could not check". It does NOT re-exec:
//!   the update *is* the work, so there is nothing to re-run.
//!
//! Neither path ever lets the backend choose a release (R18): with two release
//! lines in one repository, the backend's own "latest" could name a plugin
//! release, so every `self_update` call is pinned to a tag that passed the
//! CLI line's anchored filter.

pub mod apply;

use std::path::PathBuf;

use apply::{ApplyOutcome, ProcessSpawner};
use ss_magic_core::release::{self, UpdateCheck, UreqReleaseClient, CLI_LINE};

/// Lock-file path inside the OS cache dir, alongside the version cache.
/// `None` when no cache dir resolves (no home dir) — the caller then treats
/// the update as a silent no-op.
fn lock_path() -> Option<PathBuf> {
    release::cache_dir().map(|d| d.join(apply::LOCK_FILE_NAME))
}

/// Outcome of the explicit `ss-magic update` force path, for the caller to
/// render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateReport {
    /// Swapped to a newer release; `version` is the freshly installed one as
    /// a bare `X.Y.Z`.
    Updated { version: String },
    /// The newest release of this line is not newer than the running binary
    /// (or the swap ran and fell through silently).
    AlreadyLatest,
    /// Another updater held the lock; nothing was done (caller can retry).
    Skipped,
    /// The release list could not be resolved – offline, a non-200 status, a
    /// malformed body, or no tag of this line in it – so the backend was never
    /// invoked. Reported distinctly from `AlreadyLatest`: "could not check" is
    /// not "up to date".
    Unavailable,
}

/// `ss-magic update`: force a self-update regardless of the 24h cache.
///
/// Bypasses the daily-cache gate entirely — it does NOT call [`release::check`]
/// and reads no cache file, so a fresh cache does not suppress the re-check
/// (the cache governs only the bare/`sync` auto-gate). Resolution goes
/// through [`release::resolve_newest_uncached`] on the CLI line; the resolved
/// tag is then pinned into the `self_update` swap (lock-guarded when a cache
/// dir resolves, unlocked as a defensive fallback). Does not re-exec.
pub fn update_command() -> UpdateReport {
    update_command_with(
        env!("CARGO_PKG_VERSION"),
        || release::resolve_newest_uncached(
            &UreqReleaseClient::new(env!("CARGO_PKG_VERSION")),
            &CLI_LINE,
        ),
        |tag| match lock_path() {
            Some(lock) => apply::apply_update(&lock, tag),
            None => apply::apply_update_unlocked(tag),
        },
    )
}

/// Testable core of [`update_command`]. `resolve` is the injected per-line
/// resolver (production: the uncached GitHub list fetch) and `run_swap` the
/// injected swap seam (production: the lock-guarded `self_update` apply), so
/// tests assert the decision order without a network or a download:
///
/// 1. `resolve()` → `None` is [`UpdateReport::Unavailable`], and the swap
///    seam is never entered – there is no tag to pin, and the backend must
///    not be asked to pick one.
/// 2. A tag that fails the CLI line's own filter can only come from a broken
///    resolver; it is likewise `Unavailable`, never pinned.
/// 3. A tag not newer than `current_version` is [`UpdateReport::AlreadyLatest`]
///    – decided here, before any download.
/// 4. Otherwise `run_swap(tag)` runs once and its outcome is mapped.
fn update_command_with<R, F>(current_version: &str, resolve: R, run_swap: F) -> UpdateReport
where
    R: FnOnce() -> Option<String>,
    F: FnOnce(&str) -> ApplyOutcome,
{
    let Some(tag) = resolve() else {
        return UpdateReport::Unavailable;
    };
    if release::parse_line_tag(&CLI_LINE, &tag).is_none() {
        return UpdateReport::Unavailable;
    }
    if !release::is_newer(&CLI_LINE, &tag, current_version) {
        return UpdateReport::AlreadyLatest;
    }
    map_report(run_swap(&tag))
}

/// Pure outcome → report mapping for the force path. The backend reports the
/// installed release as a version string derived from the tag; should it ever
/// hand back the raw tag instead, it is normalized through the line filter so
/// the rendered text is `v0.11.10`, never `vv0.11.10`.
fn map_report(outcome: ApplyOutcome) -> UpdateReport {
    match outcome {
        ApplyOutcome::Updated { version } => UpdateReport::Updated {
            version: display_version(version),
        },
        ApplyOutcome::NoUpdate => UpdateReport::AlreadyLatest,
        ApplyOutcome::Skipped => UpdateReport::Skipped,
    }
}

/// A raw CLI-line tag becomes its bare `MAJOR.MINOR.PATCH`; anything else is
/// returned untouched.
fn display_version(raw: String) -> String {
    match release::parse_line_tag(&CLI_LINE, &raw) {
        Some((major, minor, patch)) => format!("{major}.{minor}.{patch}"),
        None => raw,
    }
}

/// The auto-update gate `main.rs` wires into every gated (non-`update`)
/// entrypoint.
///
/// Consults the 24h cache via [`release::check`]; on a [`UpdateCheck::Newer`]
/// verdict it acquires the lock, swaps to the cached newer tag, and — on a
/// successful swap — re-execs the swapped binary with the original args +
/// `SS_MAGIC_UPDATED=1`, blocking on it and exiting with its code (this never
/// returns). On `UpToDate`, lock contention, or any swap failure it returns
/// so the caller proceeds on the current binary.
///
/// The loop guard ([`apply::guard_active`]) is the caller's responsibility to
/// check *before* calling this; we re-check it here as a belt-and-braces
/// early-return so a re-exec'd child can never recurse into another update.
pub fn auto_update() {
    if apply::guard_active() {
        return;
    }
    let UpdateCheck::Newer { tag } = release::check(env!("CARGO_PKG_VERSION")) else {
        return;
    };
    let Some(lock) = lock_path() else {
        return;
    };
    match apply::apply_update(&lock, &tag) {
        ApplyOutcome::Updated { .. } => {
            // Swap done; re-exec the new binary and propagate its exit code.
            // This terminates the process.
            apply::reexec_and_exit(&ProcessSpawner);
        }
        // Contended or no-op → proceed on the current binary.
        ApplyOutcome::Skipped | ApplyOutcome::NoUpdate => {}
    }
}

#[cfg(test)]
mod tests;
