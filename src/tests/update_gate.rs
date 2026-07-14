//! U8 gate-decision tests: `should_run_update_gate` truth table (AE3).
//!
//! These are pure unit tests over the decision helper only — they do not
//! perform network calls, lock files, or re-exec. The actual block-in-wait
//! and exit-with-child-code behavior is seam-tested in U7 (update/apply.rs).

use crate::cli::Command;
use crate::should_run_update_gate;

// ── Gate fires for Bare / Sync when guard is inactive ───────────────────

/// AE3 (wiring): Bare command + no guard → gate fires.
#[test]
fn ae3_bare_no_guard_gate_fires() {
    assert!(
        should_run_update_gate(Command::Bare, false),
        "Bare + guard inactive → gate must fire"
    );
}

/// Sync command + no guard → gate fires.
#[test]
fn sync_no_guard_gate_fires() {
    assert!(
        should_run_update_gate(Command::Sync, false),
        "Sync + guard inactive → gate must fire"
    );
}

/// Pack command + no guard → gate fires (gated like Sync).
#[test]
fn pack_no_guard_gate_fires() {
    assert!(
        should_run_update_gate(Command::Pack, false),
        "Pack + guard inactive → gate must fire"
    );
}

/// Pack + guard active → gate does NOT fire.
#[test]
fn pack_guard_active_gate_does_not_fire() {
    assert!(
        !should_run_update_gate(Command::Pack, true),
        "Pack + guard active → gate must not fire"
    );
}

// ── Update bypasses the gate regardless of guard state ──────────────────

/// Update + no guard → gate does NOT fire (uses its own force path).
#[test]
fn update_no_guard_gate_does_not_fire() {
    assert!(
        !should_run_update_gate(Command::Update, false),
        "Update must bypass the daily-cache gate (uses force path)"
    );
}

/// Update + guard active → gate does NOT fire.
#[test]
fn update_guard_active_gate_does_not_fire() {
    assert!(
        !should_run_update_gate(Command::Update, true),
        "Update + guard active → gate must not fire"
    );
}

// ── Guard active short-circuits the gate for all commands ───────────────

/// AE4 (no loop): re-exec'd child has SS_MAGIC_UPDATED=1 → guard active →
/// gate does not fire, preventing infinite re-exec loops.
#[test]
fn ae4_bare_guard_active_gate_does_not_fire() {
    assert!(
        !should_run_update_gate(Command::Bare, true),
        "Bare + guard active → gate must not fire (loop prevention)"
    );
}

/// Sync + guard active → gate does NOT fire.
#[test]
fn sync_guard_active_gate_does_not_fire() {
    assert!(
        !should_run_update_gate(Command::Sync, true),
        "Sync + guard active (SS_MAGIC_NO_UPDATE) → gate must not fire"
    );
}
