//! U8 gate-decision tests: `should_run_update_gate` truth table (AE3), plus
//! the U6 additions that pin which argv can reach the gate at all and when the
//! interactive menu is allowed to open.
//!
//! These are pure unit tests over the decision helpers only — they do not
//! perform network calls, lock files, or re-exec. The actual block-in-wait
//! and exit-with-child-code behavior is seam-tested in U7 (update/apply.rs).

use crate::cli::{self, Command, Parsed};
use crate::{menu_blocked_reason, should_run_update_gate, version_line};

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
        should_run_update_gate(Command::Sync { no_backup: false }, false),
        "Sync + guard inactive → gate must fire"
    );
}

/// ReverseSync command + no guard → gate fires.
#[test]
fn reverse_sync_no_guard_gate_fires() {
    assert!(
        should_run_update_gate(Command::ReverseSync { no_backup: false }, false),
        "ReverseSync + guard inactive → gate must fire"
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
        !should_run_update_gate(Command::Sync { no_backup: false }, true),
        "Sync + guard active (SS_MAGIC_NO_UPDATE) → gate must not fire"
    );
}

/// ReverseSync + guard active → gate does NOT fire.
#[test]
fn reverse_sync_guard_active_gate_does_not_fire() {
    assert!(
        !should_run_update_gate(Command::ReverseSync { no_backup: false }, true),
        "ReverseSync + guard active → gate must not fire"
    );
}

// ── Which argv can reach the gate at all ────────────────────────────────────
//
// `should_run_update_gate` only ever sees a `Command`, and `Parsed::Version` is
// not one — `main::run` handles it in a sibling arm. So the pin lives one level
// up, at the parse layer: these argvs must never produce a `Parsed::Command`.
//
// There used to be a companion test here asserting that no `ss-magic plugin …`
// argv reached the gate either. It went with the token: the plugin is a
// separate binary that links no updater, so "the plugin never self-updates" is
// now a property of the dependency graph (asserted by
// `build-plugin-zip.py --check`'s workspace-shape guard and by CI's
// `cargo tree -i self_update`) rather than of this parser.

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// AE56: `--version` answers without a network round-trip, from any position
/// in argv.
#[test]
fn version_argv_never_produces_a_gated_command() {
    for tail in [
        vec!["--version"],
        vec!["-V"],
        vec!["sync", "--version"],
        vec!["pack", "-V"],
    ] {
        let parsed = cli::parse(&argv(&tail));
        assert_eq!(
            parsed, Parsed::Version,
            "{tail:?} must short-circuit to Version before any command is selected"
        );
    }
}

#[test]
fn version_line_carries_the_crate_version() {
    let line = version_line();
    assert!(
        line.contains(env!("CARGO_PKG_VERSION")),
        "version line {line:?} should carry the crate version"
    );
}

/// The gate's inclusion list is exhaustive over `Command`. Written as a match
/// so adding a variant fails to compile until someone decides whether it is
/// gated, rather than silently inheriting `false`.
#[test]
fn gate_inclusion_list_is_exactly_the_four_work_commands() {
    let all = [
        Command::Bare,
        Command::Sync { no_backup: false },
        Command::ReverseSync { no_backup: false },
        Command::Pack,
        Command::Update,
    ];
    for cmd in all {
        let expected = match cmd {
            Command::Bare
            | Command::Sync { .. }
            | Command::ReverseSync { .. }
            | Command::Pack => true,
            Command::Update => false,
        };
        assert_eq!(
            should_run_update_gate(cmd, false),
            expected,
            "gate decision for {cmd:?} changed"
        );
    }
}

// ── U6: the interactive menu needs both terminal ends (R69, AE56) ───────────

#[test]
fn menu_opens_only_when_both_ends_are_a_terminal() {
    assert_eq!(menu_blocked_reason(true, true), None);
}

#[test]
fn menu_refuses_and_names_the_missing_end() {
    assert_eq!(
        menu_blocked_reason(false, true),
        Some("stdin is not a terminal")
    );
    assert_eq!(
        menu_blocked_reason(true, false),
        Some("stdout is not a terminal")
    );
    assert_eq!(
        menu_blocked_reason(false, false),
        Some("neither stdin nor stdout is a terminal")
    );
}

/// Every refusal says which end is missing — a bare "cannot open menu" leaves
/// the user with nothing to act on.
#[test]
fn every_menu_refusal_names_a_terminal() {
    for (stdin_tty, stdout_tty) in [(false, true), (true, false), (false, false)] {
        let reason = menu_blocked_reason(stdin_tty, stdout_tty)
            .expect("non-tty combination must be blocked");
        assert!(
            reason.contains("terminal"),
            "reason {reason:?} should name the terminal requirement"
        );
    }
}
