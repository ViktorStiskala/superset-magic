use super::*;
use std::cell::{Cell, RefCell};

/// A swap seam that records the tag it was pinned to and returns a canned
/// outcome, so the force path is exercised without a live download.
struct RecordingSwap {
    outcome: ApplyOutcome,
    seen_tag: RefCell<Option<String>>,
    calls: Cell<usize>,
}

impl RecordingSwap {
    fn new(outcome: ApplyOutcome) -> Self {
        Self {
            outcome,
            seen_tag: RefCell::new(None),
            calls: Cell::new(0),
        }
    }

    fn run(&self, tag: &str) -> ApplyOutcome {
        self.calls.set(self.calls.get() + 1);
        *self.seen_tag.borrow_mut() = Some(tag.to_string());
        self.outcome.clone()
    }
}

// ── Force-path report mapping ───────────────────────────────────────────

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

/// The backend reports the installed release as a version string; when it
/// comes back as the raw tag instead, the report still carries the bare
/// triple so the caller prints `v0.11.10`, never `vv0.11.10`.
#[test]
fn map_report_updated_normalizes_a_raw_tag_to_its_triple() {
    assert_eq!(
        map_report(ApplyOutcome::Updated {
            version: "v0.11.10".to_string()
        }),
        UpdateReport::Updated {
            version: "0.11.10".to_string()
        }
    );
    // A bare triple is already in display form.
    assert_eq!(
        map_report(ApplyOutcome::Updated {
            version: "0.11.10".to_string()
        }),
        UpdateReport::Updated {
            version: "0.11.10".to_string()
        }
    );
}

/// A swap that ran but installed nothing reports "already latest".
#[test]
fn map_report_no_update_reports_already_latest() {
    assert_eq!(map_report(ApplyOutcome::NoUpdate), UpdateReport::AlreadyLatest);
}

#[test]
fn map_report_skipped_reports_skipped() {
    assert_eq!(map_report(ApplyOutcome::Skipped), UpdateReport::Skipped);
}

// ── AE3: a failed resolution is Unavailable, and the backend is never built ─

#[test]
fn ae3_failed_resolution_reports_unavailable_without_invoking_swap() {
    let swap = RecordingSwap::new(ApplyOutcome::Updated {
        version: "9.9.9".to_string(),
    });
    let resolved = Cell::new(0);
    let report = update_command_with(
        "1.0.0",
        || {
            resolved.set(resolved.get() + 1);
            None
        },
        |tag| swap.run(tag),
    );
    assert_eq!(resolved.get(), 1, "the resolver runs exactly once");
    assert_eq!(
        report,
        UpdateReport::Unavailable,
        "offline must read as 'could not check', never 'already latest'"
    );
    assert_eq!(swap.calls.get(), 0, "no tag → the swap seam is never entered");
}

/// A resolved tag that fails the CLI line's own filter can only come from a
/// broken resolver; it is reported as Unavailable rather than pinned.
#[test]
fn resolved_tag_of_another_line_reports_unavailable_without_swap() {
    let swap = RecordingSwap::new(ApplyOutcome::NoUpdate);
    let report = update_command_with(
        "1.0.0",
        || Some("ss-magic-plugin-v9.0.0".to_string()),
        |tag| swap.run(tag),
    );
    assert_eq!(report, UpdateReport::Unavailable);
    assert_eq!(swap.calls.get(), 0);
}

// ── Not newer → AlreadyLatest, decided BEFORE the backend ───────────────

#[test]
fn resolved_tag_not_newer_reports_already_latest_without_swap() {
    for tag in ["v1.0.0", "v0.9.9"] {
        let swap = RecordingSwap::new(ApplyOutcome::Updated {
            version: "9.9.9".to_string(),
        });
        let report = update_command_with("1.0.0", || Some(tag.to_string()), |t| swap.run(t));
        assert_eq!(report, UpdateReport::AlreadyLatest, "{tag} is not newer than 1.0.0");
        assert_eq!(swap.calls.get(), 0, "{tag}: no download for a non-newer tag");
    }
}

// ── Newer → the swap runs exactly once, pinned to the resolved tag ──────

#[test]
fn resolved_newer_tag_is_pinned_into_the_swap() {
    let swap = RecordingSwap::new(ApplyOutcome::Updated {
        version: "2.0.0".to_string(),
    });
    let report = update_command_with("1.0.0", || Some("v2.0.0".to_string()), |t| swap.run(t));
    assert_eq!(swap.calls.get(), 1, "force path must invoke the swap exactly once");
    assert_eq!(
        swap.seen_tag.borrow().as_deref(),
        Some("v2.0.0"),
        "the swap must be pinned to the resolved tag, verbatim"
    );
    assert_eq!(
        report,
        UpdateReport::Updated {
            version: "2.0.0".to_string()
        }
    );
}

#[test]
fn newer_tag_whose_swap_installs_nothing_reports_already_latest() {
    // The backend fell through (asset missing, swap failed) → NoUpdate →
    // "already latest", the pre-existing mapping for a swap that ran.
    let swap = RecordingSwap::new(ApplyOutcome::NoUpdate);
    let report = update_command_with("1.0.0", || Some("v2.0.0".to_string()), |t| swap.run(t));
    assert_eq!(swap.calls.get(), 1);
    assert_eq!(report, UpdateReport::AlreadyLatest);
}

#[test]
fn newer_tag_with_contended_lock_reports_skipped() {
    let swap = RecordingSwap::new(ApplyOutcome::Skipped);
    let report = update_command_with("1.0.0", || Some("v2.0.0".to_string()), |t| swap.run(t));
    assert_eq!(report, UpdateReport::Skipped);
}
