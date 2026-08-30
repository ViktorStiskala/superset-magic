use std::path::Path;

use super::*;
use crate::tests::support::{git_run, init_main_repo, make_worktree, write_file};

/// Write and COMMIT `.superset/magic.json` at `root`, so a linked worktree
/// created afterwards checks the file out too (the committed base is shared
/// across a repo's worktrees; the gitignored local overlay is not).
fn commit_magic_json(root: &Path, body: &str) {
    write_file(root, ".superset/magic.json", body);
    git_run(&["add", ".superset/magic.json"], root);
    git_run(&["commit", "-q", "-m", "magic.json"], root);
}

/// Write (never commit) `.superset/magic.local.json` at `root` — the
/// gitignored per-machine overlay `load_overlaid` reads on top of the base.
fn write_local(root: &Path, body: &str) {
    write_file(root, ".superset/magic.local.json", body);
}

// ── R5 / R7: enabled, and the main-checkout redirect ────────────────────────

/// Covers AE3. The main checkout's local override wins over a worktree's own
/// (here: absent) overlay — R7 exists precisely because a worktree's
/// `magic.local.json` is itself a forward-sync target and cannot be trusted
/// for the per-machine toggle.
#[test]
fn ae3_worktree_reads_enabled_false_from_main_checkouts_local_override() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[],"plugin":{"enabled":true}}"#);
    let (_wt_dir, wt_root) = make_worktree(main.path());

    write_local(main.path(), r#"{"files":[],"plugin":{"enabled":false}}"#);

    let cfg = resolve(&wt_root);
    assert!(
        !cfg.enabled,
        "R7: the main checkout's local override must win over the worktree's own overlay"
    );
}

/// The mirror of AE3: resolving from the main checkout itself is just that
/// checkout's own overlay, since `main_checkout_root` returns the same root
/// unchanged.
#[test]
fn resolve_from_main_checkout_itself_uses_its_own_overlay() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[],"plugin":{"enabled":true}}"#);
    write_local(main.path(), r#"{"files":[],"plugin":{"enabled":false}}"#);

    assert!(!resolve(main.path()).enabled);
}

/// An absent `plugin` key in magic.local.json inherits the base value (R6).
#[test]
fn absent_plugin_key_in_local_inherits_base_enabled() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[],"plugin":{"enabled":true}}"#);
    write_local(main.path(), r#"{"files":[]}"#);

    assert!(resolve(main.path()).enabled);
}

/// An explicit `null` `plugin` value in magic.local.json means off (R6).
#[test]
fn explicit_null_plugin_in_local_disables() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[],"plugin":{"enabled":true}}"#);
    write_local(main.path(), r#"{"files":[],"plugin":null}"#);

    assert!(!resolve(main.path()).enabled);
}

/// No `magic.json` at all resolves to disabled with no error (R5).
#[test]
fn missing_magic_json_resolves_to_disabled_with_no_error() {
    let main = init_main_repo("main");
    assert_eq!(resolve(main.path()), PluginConfig::default());
}

/// `magic.json` present but with no `plugin` key at all resolves to disabled
/// with no error (R5) — the same outcome as a missing file entirely.
#[test]
fn missing_plugin_block_resolves_to_disabled_with_no_error() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);
    assert_eq!(resolve(main.path()), PluginConfig::default());
}

/// A `plugin` value that isn't a JSON object (a string, a number) degrades
/// to the same defaults as an absent block, never an error.
#[test]
fn plugin_value_not_an_object_degrades_to_defaults() {
    let strings = init_main_repo("main");
    commit_magic_json(strings.path(), r#"{"files":[],"plugin":"oops"}"#);
    assert_eq!(resolve(strings.path()), PluginConfig::default());

    let numbers = init_main_repo("main");
    commit_magic_json(numbers.path(), r#"{"files":[],"plugin":42}"#);
    assert_eq!(resolve(numbers.path()), PluginConfig::default());
}

/// Outside any git repository at all (so `main_checkout_root` fails),
/// `resolve` falls back to reading `cwd_root`'s own overlay directly rather
/// than defaulting blindly — the fallback path is exercised, not just the
/// total-absence path.
#[test]
fn resolve_falls_back_to_cwd_when_not_a_git_repository() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        ".superset/magic.json",
        r#"{"files":[],"plugin":{"enabled":true}}"#,
    );

    assert!(
        resolve(dir.path()).enabled,
        "outside any git repository, resolve must still read cwd's own overlay"
    );
}

// ── R53: the gate's own tunables ────────────────────────────────────────────

#[test]
fn gate_threshold_below_bound_clamps_to_min() {
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":10}}}"#,
    );
    assert_eq!(
        resolve(main.path()).gate.threshold_lines,
        GATE_THRESHOLD_LINES_MIN
    );
}

#[test]
fn gate_threshold_above_bound_clamps_to_max() {
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":999999}}}"#,
    );
    assert_eq!(
        resolve(main.path()).gate.threshold_lines,
        GATE_THRESHOLD_LINES_MAX
    );
}

#[test]
fn gate_inline_byte_budget_clamps_to_bounds() {
    let below = init_main_repo("main");
    commit_magic_json(
        below.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"inline_byte_budget":1}}}"#,
    );
    assert_eq!(
        resolve(below.path()).gate.inline_byte_budget,
        GATE_INLINE_BYTE_BUDGET_MIN
    );

    let above = init_main_repo("main");
    commit_magic_json(
        above.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"inline_byte_budget":1000000}}}"#,
    );
    assert_eq!(
        resolve(above.path()).gate.inline_byte_budget,
        GATE_INLINE_BYTE_BUDGET_MAX
    );
}

/// A non-numeric threshold falls back to the default rather than erroring
/// or being interpreted as zero.
#[test]
fn gate_threshold_wrong_type_falls_back_to_default() {
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":"lots"}}}"#,
    );
    assert_eq!(
        resolve(main.path()).gate.threshold_lines,
        GATE_THRESHOLD_LINES_DEFAULT
    );
}

/// A non-string entry in `exemptions` is dropped; the well-formed entries
/// around it survive. A malformed entry can only shrink the exemption list
/// (make the gate fire MORE often), never widen it.
#[test]
fn gate_exemptions_drops_non_string_entries_keeps_the_rest() {
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"exemptions":["docs/**",42,null,"assets/*.png"]}}}"#,
    );
    assert_eq!(
        resolve(main.path()).gate.exemptions,
        vec!["docs/**".to_string(), "assets/*.png".to_string()]
    );
}

/// R53 resolves the gate's tunables against `cwd_root`'s own overlay, in
/// contrast to R7's `enabled`, which is redirected through the main
/// checkout. A worktree's own `gate` settings must win for `gate`, even
/// though its `enabled` value is ignored in favor of main's.
#[test]
fn gate_settings_resolve_from_cwds_own_overlay_not_main() {
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":4000}}}"#,
    );
    let (_wt_dir, wt_root) = make_worktree(main.path());

    // Main's own local sets a different threshold — must NOT be picked up
    // for `gate` (only for `enabled`).
    write_local(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":9000}}}"#,
    );
    // The worktree's own local sets yet another threshold — this is what
    // must win for `gate`.
    write_local(
        &wt_root,
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":6000}}}"#,
    );

    let cfg = resolve(&wt_root);
    assert_eq!(
        cfg.gate.threshold_lines, 6000,
        "gate tunables resolve from cwd's own overlay, not main's"
    );
    assert!(cfg.enabled);
}
