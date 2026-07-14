//! Self-update subsystem.
//!
//! Split into the cheap gate and the heavy apply (per the plan's Phase C):
//! - [`check`] (U6) answers "is a newer release available?" cheaply and
//!   offline-safely — a 24h-cached check that never errors, never logs, and
//!   never blocks an offline or rate-limited run.
//! - [`apply`] (U7) is the heavy half: lock → download/verify/swap → re-exec,
//!   gated on a `Newer` verdict from the check (or the forced check behind
//!   `ss-magic update`).
//! - Wiring the auto-update gate into every entrypoint is U8 (`main.rs`); U7
//!   provides the functions U8 calls ([`auto_update`]) plus the explicit force
//!   path ([`update_command`]).
//!
//! The two entry points differ only in how they gate the network:
//! - [`auto_update`] consults the 24h cache via [`check::check`]; it acts only
//!   on a `Newer` verdict, then re-execs the swapped binary so the caller's
//!   work runs on the new version.
//! - [`update_command`] (the `ss-magic update` force path, R4) skips the cache
//!   entirely and asks `self_update` to swap to the latest release directly,
//!   reporting the resulting version or "already latest". It does NOT re-exec:
//!   the update *is* the work, so there is nothing to re-run.

pub mod apply;
pub mod check;

use std::path::PathBuf;

use apply::{ApplyOutcome, ProcessSpawner};
use check::UpdateCheck;

// Re-exports for U8 (startup wiring). `check` is consumed only by the U8 gate,
// so it keeps an allow; the other items below are consumed by U7 here.
#[allow(unused_imports)] // available for direct use by external callers if needed
pub use check::check;

/// Lock-file path inside the OS cache dir, alongside the version cache.
/// `None` when no cache dir resolves (no home dir) — the caller then treats
/// the update as a silent no-op.
fn lock_path() -> Option<PathBuf> {
    check::cache_dir().map(|d| d.join(apply::LOCK_FILE_NAME))
}

/// Outcome of the explicit `ss-magic update` force path (R4), for the caller
/// to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateReport {
    /// Swapped to a newer release; `version` is the freshly installed one.
    Updated { version: String },
    /// Already on the latest release (or the swap fell through silently).
    AlreadyLatest,
    /// Another updater held the lock; nothing was done (caller can retry).
    Skipped,
}

/// `ss-magic update` (R4): force a self-update regardless of the 24h cache.
///
/// Bypasses the daily-cache gate entirely — it does NOT call [`check::check`],
/// so a fresh cache does not suppress the re-check (the cache governs only the
/// bare/`sync` auto-gate). Runs the `self_update` swap directly (which fetches
/// the latest release and compares versions itself), holding the update lock
/// for the duration. Reports the resulting version or "already latest". Does
/// not re-exec.
pub fn update_command() -> UpdateReport {
    // The swap seam: production runs the real `self_update` apply (lock-guarded
    // when a cache dir resolves, unlocked as a defensive fallback). Crucially,
    // nothing here reads the 24h version cache — the force path is NOT gated by
    // [`check::check`], so a fresh cache cannot suppress the re-check.
    update_command_with(|| match lock_path() {
        Some(lock) => apply::apply_update(&lock, None),
        None => apply::apply_update_unlocked(None),
    })
}

/// Testable core of [`update_command`]: maps an [`ApplyOutcome`] (from the
/// injected `run_swap` seam) to an [`UpdateReport`]. The seam lets tests assert
/// the report mapping and the cache-bypass property without a live download.
fn update_command_with<F: FnOnce() -> ApplyOutcome>(run_swap: F) -> UpdateReport {
    map_report(run_swap())
}

/// Pure outcome → report mapping for the force path.
fn map_report(outcome: ApplyOutcome) -> UpdateReport {
    match outcome {
        ApplyOutcome::Updated { version } => UpdateReport::Updated { version },
        ApplyOutcome::NoUpdate => UpdateReport::AlreadyLatest,
        ApplyOutcome::Skipped => UpdateReport::Skipped,
    }
}

/// The auto-update gate U8 wires into every (non-`update`) entrypoint.
///
/// Consults the 24h cache via [`check::check`]; on a [`UpdateCheck::Newer`]
/// verdict it acquires the lock, swaps to the cached newer tag, and — on a
/// successful swap — re-execs the swapped binary with the original args +
/// `SS_MAGIC_UPDATED=1`, blocking on it and exiting with its code (this never
/// returns). On `UpToDate`, lock contention, or any swap failure it returns
/// so the caller proceeds on the current binary.
///
/// The loop guard ([`apply::guard_active`]) is the caller's responsibility to
/// check *before* calling this (U8); we re-check it here as a belt-and-braces
/// early-return so a re-exec'd child can never recurse into another update.
pub fn auto_update() {
    if apply::guard_active() {
        return;
    }
    let UpdateCheck::Newer { tag } = check::check() else {
        return;
    };
    let Some(lock) = lock_path() else {
        return;
    };
    match apply::apply_update(&lock, Some(&tag)) {
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
