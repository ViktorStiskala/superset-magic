use super::*;
use std::cell::Cell;

// ── Force-path report mapping (R4) ──────────────────────────────────────

#[test]
fn map_report_updated_reports_version() {
    assert_eq!(
        map_report(ApplyOutcome::Updated {
            version: "1.4.0".to_string()
        }),
        UpdateReport::Updated {
            version: "1.4.0".to_string()
        }
    );
}

/// `ss-magic update` with no newer release reports "already latest".
#[test]
fn map_report_no_update_reports_already_latest() {
    assert_eq!(map_report(ApplyOutcome::NoUpdate), UpdateReport::AlreadyLatest);
}

#[test]
fn map_report_skipped_reports_skipped() {
    assert_eq!(map_report(ApplyOutcome::Skipped), UpdateReport::Skipped);
}

/// The force path runs the swap seam exactly once and maps "no update" to
/// "already latest" — the end-to-end `ss-magic update`-with-no-newer-release
/// contract, exercised without a live download.
#[test]
fn update_command_with_no_newer_release_reports_already_latest() {
    let ran = Cell::new(0);
    let report = update_command_with(|| {
        ran.set(ran.get() + 1);
        ApplyOutcome::NoUpdate
    });
    assert_eq!(ran.get(), 1, "force path must invoke the swap exactly once");
    assert_eq!(report, UpdateReport::AlreadyLatest);
}

/// The force path is NOT gated by the 24h version cache: its core
/// ([`update_command_with`]) takes only a swap seam and unconditionally
/// runs it, with no `check::check`/cache read in the path at all. This is
/// the "fresh cache still re-checks" guarantee structurally — the
/// daily-cache gate governs only bare/`sync`, never the explicit `update`
/// command. (The real `update_command` builds this exact seam over
/// `apply::apply_update`, which fetches the latest release itself.)
#[test]
fn force_path_always_runs_swap_no_cache_gate() {
    // Even when a verdict of "already up to date" would be the cached
    // answer, the force path must still drive the swap (which re-checks
    // against GitHub). We model that as the seam unconditionally running.
    let ran = Cell::new(false);
    let report = update_command_with(|| {
        ran.set(true);
        ApplyOutcome::Updated {
            version: "2.0.0".to_string(),
        }
    });
    assert!(
        ran.get(),
        "force path must run the swap unconditionally (no daily-cache gate)"
    );
    assert_eq!(
        report,
        UpdateReport::Updated {
            version: "2.0.0".to_string()
        }
    );
}
