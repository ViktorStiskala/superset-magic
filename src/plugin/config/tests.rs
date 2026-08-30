use std::path::Path;

use super::*;
use crate::tests::support::{exit_code_to_u8, git_run, init_main_repo, make_worktree, write_file};

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

// ── U19: `enable` / `disable` / `config get` / `config set` (R37) ──────────

/// Covers AE21: `config set plugin.enabled false --local` from a worktree
/// edits the MAIN CHECKOUT's `magic.local.json` (R7), and every key that
/// verb does not understand — another tool's block, and the plugin's own
/// `gate` sibling — survives untouched.
#[test]
fn ae21_config_set_local_from_a_worktree_edits_the_main_checkouts_overlay() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[],"plugin":{"enabled":true}}"#);
    let (_wt_dir, wt_root) = make_worktree(main.path());
    write_local(
        main.path(),
        r#"{"files":[],"someOtherTool":{"keep":"me"},"plugin":{"enabled":true,"gate":{"threshold_lines":4000}}}"#,
    );

    run_config_set_core(
        &wt_root,
        "plugin.enabled",
        &["plugin", "enabled"],
        Value::Bool(false),
        true,
    )
    .unwrap();

    // The worktree's OWN overlay must be untouched — nothing was written
    // there at all.
    assert!(superset_files::load_magic_local_json(&wt_root)
        .unwrap()
        .is_none());

    // The main checkout's local overlay is what changed, and only at the one
    // key asked for.
    let local = superset_files::load_magic_local_json(main.path())
        .unwrap()
        .unwrap();
    let plugin = &local.extras["plugin"];
    assert_eq!(plugin["enabled"], Value::Bool(false));
    assert_eq!(plugin["gate"]["threshold_lines"], Value::from(4000));
    assert_eq!(local.extras["someOtherTool"]["keep"], Value::String("me".into()));
}

/// `enable` on a `magic.json` with no `plugin` block at all creates a minimal
/// one, without disturbing `files`.
#[test]
fn enable_on_a_file_with_no_plugin_block_creates_a_minimal_one() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":["a.txt","b.txt"]}"#);

    let code = run_toggle_core(main.path(), false, true).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    let cfg = superset_files::load_magic_json(main.path()).unwrap().unwrap();
    assert_eq!(cfg.files, vec!["a.txt".to_string(), "b.txt".to_string()]);
    assert_eq!(cfg.extras["plugin"]["enabled"], Value::Bool(true));
}

/// `config get` reads the OVERLAID, resolved value — local's override, not
/// just the committed base.
#[test]
fn config_get_reads_the_overlaid_resolved_value_not_just_base() {
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        r#"{"files":[],"plugin":{"enabled":false,"gate":{"threshold_lines":3000}}}"#,
    );
    write_local(main.path(), r#"{"files":[],"plugin":{"enabled":true}}"#);

    let value = run_config_get_core(main.path(), &["plugin", "enabled"]).unwrap();
    assert_eq!(value, Value::Bool(true), "local's override must win over base");

    // R6's override is of the WHOLE `plugin` value, not a per-leaf merge:
    // since local set `plugin` at all, base's `gate` is gone from the
    // overlay too, not just `enabled`.
    let gate = run_config_get_core(main.path(), &["plugin", "gate", "threshold_lines"]).unwrap();
    assert_eq!(
        gate,
        Value::Null,
        "local overriding `plugin` at all replaces the whole value, base's `gate` included"
    );

    // When local does not mention `plugin` at all, the overlay falls all the
    // way through to base, nested keys included.
    let main2 = init_main_repo("main");
    commit_magic_json(
        main2.path(),
        r#"{"files":[],"plugin":{"gate":{"threshold_lines":3000}}}"#,
    );
    let base_only =
        run_config_get_core(main2.path(), &["plugin", "gate", "threshold_lines"]).unwrap();
    assert_eq!(base_only, Value::from(3000));
}

/// `config get` on a key that is not set anywhere reads as JSON `null`,
/// consistent with R6's own null-means-absent convention, rather than
/// erroring.
#[test]
fn config_get_on_an_unset_key_is_null() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);

    let value = run_config_get_core(main.path(), &["plugin", "gate", "exemptions"]).unwrap();
    assert_eq!(value, Value::Null);
}

/// `config get`/`config set` refuse a key not rooted at `plugin` — `files`
/// and any other top-level key have their own editors; this verb answers
/// only for the plugin configuration (R37).
#[test]
fn config_rejects_a_key_not_rooted_at_plugin() {
    assert!(validate_plugin_key("files").is_err());
    assert!(validate_plugin_key("plugin").is_ok());
    assert!(validate_plugin_key("plugin.enabled").is_ok());
    // A stray empty segment (leading/trailing/doubled dot) is rejected too.
    assert!(validate_plugin_key(".plugin").is_err());
    assert!(validate_plugin_key("plugin.").is_err());
    assert!(validate_plugin_key("plugin..enabled").is_err());
}

/// Covers AE24's remediation half: `enable` in a repository whose
/// `.gitignore` carries no rule for the state tree appends EXACTLY the
/// `.superset/.magic/` line and changes nothing else in the file. A second
/// `enable` is then a no-op on that file.
#[test]
fn ae24_enable_appends_exactly_the_state_tree_rule_and_is_idempotent() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);
    write_file(main.path(), ".gitignore", "/target\n");

    run_toggle_core(main.path(), false, true).unwrap();

    let gi_after_first = std::fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert_eq!(
        gi_after_first,
        "/target\n.superset/.magic/\n",
        "must append exactly the state-tree rule, changing nothing else"
    );

    // A second `enable` is a no-op on the file — `ensure_path_ignored` is
    // idempotent, and this proves the caller doesn't defeat that by some
    // other path.
    run_toggle_core(main.path(), false, true).unwrap();
    let gi_after_second = std::fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert_eq!(gi_after_second, gi_after_first);
}

/// `disable` never removes the `.superset/.magic/` gitignore rule `enable`
/// added — R40 is explicit that nothing here ever edits `.gitignore` except
/// to add that one line.
#[test]
fn disable_never_removes_the_ignore_rule() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);
    write_file(main.path(), ".gitignore", "/target\n");

    run_toggle_core(main.path(), false, true).unwrap(); // enable: adds the rule
    let gi_after_enable = std::fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert!(gi_after_enable.contains(".superset/.magic/"));

    run_toggle_core(main.path(), false, false).unwrap(); // disable
    let gi_after_disable = std::fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert_eq!(
        gi_after_disable, gi_after_enable,
        "disable must not touch .gitignore at all"
    );

    let cfg = superset_files::load_magic_json(main.path()).unwrap().unwrap();
    assert_eq!(cfg.extras["plugin"]["enabled"], Value::Bool(false));
}

/// `disable` (which never turns the plugin on) must not add the ignore rule
/// either, on a repository that starts with none — only a transition TO
/// enabled triggers R40's lazy write.
#[test]
fn disable_alone_never_adds_the_ignore_rule() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);

    run_toggle_core(main.path(), false, false).unwrap();

    let gi_path = main.path().join(".gitignore");
    let gi = std::fs::read_to_string(&gi_path).unwrap_or_default();
    assert!(
        !gi.contains(".superset/.magic/"),
        "disable alone must never write the state-tree ignore rule"
    );
}

/// `enable`/`disable`/`config set` all round-trip a key none of them
/// understand, at both the base and the local file — the packet's own
/// verification requirement: no verb rebuilds the file from parts.
#[test]
fn every_config_writing_verb_preserves_unknown_keys() {
    let unknown = r#""futureFeature":{"on":true}"#;

    // enable / disable, base file.
    let main = init_main_repo("main");
    commit_magic_json(
        main.path(),
        &format!(r#"{{"files":["keep.txt"],{unknown}}}"#),
    );
    run_toggle_core(main.path(), false, true).unwrap();
    let cfg = superset_files::load_magic_json(main.path()).unwrap().unwrap();
    assert_eq!(cfg.files, vec!["keep.txt".to_string()]);
    assert_eq!(cfg.extras["futureFeature"]["on"], Value::Bool(true));

    // config set, local file (`--local` at the main checkout itself).
    let main2 = init_main_repo("main");
    commit_magic_json(main2.path(), r#"{"files":["keep.txt"]}"#);
    write_local(
        main2.path(),
        &format!(r#"{{"files":[],{unknown},"plugin":{{"enabled":false}}}}"#),
    );
    run_config_set_core(
        main2.path(),
        "plugin.enabled",
        &["plugin", "enabled"],
        Value::Bool(true),
        true,
    )
    .unwrap();
    let local = superset_files::load_magic_local_json(main2.path())
        .unwrap()
        .unwrap();
    assert_eq!(local.extras["futureFeature"]["on"], Value::Bool(true));
    assert_eq!(local.extras["plugin"]["enabled"], Value::Bool(true));
}

/// `config set` on a key the typed [`GateConfig`]/[`PluginConfig`] schema
/// does not define still writes through — KTD8's promise is forward
/// compatibility (a key a NEWER ss-magic understands survives being set by
/// an older one), not a closed schema enforced at write time. The read side
/// already treats an unrecognized key safely (it is simply not surfaced by
/// [`resolve`]), so there is nothing to validate here.
#[test]
fn config_set_on_an_undefined_key_still_writes_through() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);

    run_config_set_core(
        main.path(),
        "plugin.notARealKey",
        &["plugin", "notARealKey"],
        Value::from(42),
        false,
    )
    .unwrap();

    let cfg = superset_files::load_magic_json(main.path()).unwrap().unwrap();
    assert_eq!(cfg.extras["plugin"]["notARealKey"], Value::from(42));
    // And it does not fool the typed reader into thinking it's `enabled`.
    assert!(!resolve(main.path()).enabled);
}

/// `--local` from the main checkout itself (not a worktree) is simply that
/// checkout's own `magic.local.json` — `main_checkout_or_self` returns the
/// same root unchanged, so there is no special-casing needed to get here.
#[test]
fn local_flag_from_the_main_checkout_itself_edits_its_own_local_file() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);

    run_toggle_core(main.path(), true, true).unwrap();

    let local = superset_files::load_magic_local_json(main.path())
        .unwrap()
        .unwrap();
    assert_eq!(local.extras["plugin"]["enabled"], Value::Bool(true));
}

/// `enable` in a directory that is not a git repository at all still works:
/// `git::cwd_repo_root` fails, so it falls back to treating the directory
/// itself as the root — the same tolerance [`resolve`] already has on the
/// read side, and `gitignore::ensure_path_ignored` is independently
/// git-tolerant (it degrades to a literal `.gitignore` append).
#[test]
fn enable_works_outside_any_git_repository() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), ".superset/magic.json", r#"{"files":[]}"#);

    let code = run_toggle_core(dir.path(), false, true).unwrap();
    assert_eq!(exit_code_to_u8(code), 0);

    let cfg = superset_files::load_magic_json(dir.path()).unwrap().unwrap();
    assert_eq!(cfg.extras["plugin"]["enabled"], Value::Bool(true));
    let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gi.contains(".superset/.magic/"));
}

/// A whole-block replacement (`config set plugin '{"enabled":true,...}'`)
/// also counts as "turns the plugin on" for R40's lazy ignore-rule write —
/// not just the ordinary `plugin.enabled` spelling.
#[test]
fn config_set_whole_plugin_block_to_enabled_also_adds_the_ignore_rule() {
    let main = init_main_repo("main");
    commit_magic_json(main.path(), r#"{"files":[]}"#);
    write_file(main.path(), ".gitignore", "/target\n");

    let value: Value = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    run_config_set_core(main.path(), "plugin", &["plugin"], value, false).unwrap();

    let gi = std::fs::read_to_string(main.path().join(".gitignore")).unwrap();
    assert!(gi.contains(".superset/.magic/"));
}

/// A malformed on-disk file is a hard error, not silently rebuilt from
/// nothing — the same "no silent fallback" contract [`load_overlaid`]
/// documents for reads applies to this write path too.
#[test]
fn write_over_a_malformed_existing_file_is_a_hard_error() {
    let main = init_main_repo("main");
    write_file(main.path(), ".superset/magic.json", "{ not json");
    git_run(&["add", "."], main.path());
    git_run(&["commit", "-q", "-m", "malformed"], main.path());

    let result = run_toggle_core(main.path(), false, true);
    assert!(result.is_err(), "a malformed magic.json must not be silently rebuilt");
}

/// `parse_value` parses JSON when it parses, and falls back to a plain
/// string otherwise — so a caller never has to hand-quote ordinary text.
#[test]
fn parse_value_parses_json_and_falls_back_to_a_string() {
    assert_eq!(parse_value("true"), Value::Bool(true));
    assert_eq!(parse_value("3000"), Value::from(3000));
    assert_eq!(
        parse_value(r#"["docs/**","assets/*.png"]"#),
        Value::Array(vec![
            Value::String("docs/**".into()),
            Value::String("assets/*.png".into())
        ])
    );
    assert_eq!(parse_value("null"), Value::Null);
    // Not valid JSON on its own — falls back to a plain string.
    assert_eq!(parse_value("docs/**"), Value::String("docs/**".into()));
}

// ── argv parsing — pure, no filesystem and no process cwd ───────────────────
//
// Every case below hits a usage-error (or help) return before either
// `run_toggle`/`run_config_get`/`run_config_set` would read the real current
// directory, so these exercise the public, argv-parsing entry points
// directly rather than the `_core` functions.

#[test]
fn enable_and_disable_help_flags_print_usage_and_succeed() {
    for flag in ["-h", "--help"] {
        assert_eq!(exit_code_to_u8(run_enable(&[flag.to_string()]).unwrap()), 0);
        assert_eq!(exit_code_to_u8(run_disable(&[flag.to_string()]).unwrap()), 0);
    }
}

#[test]
fn enable_rejects_an_unknown_flag_and_extra_arguments() {
    assert_ne!(exit_code_to_u8(run_enable(&["--bogus".to_string()]).unwrap()), 0);
    assert_ne!(
        exit_code_to_u8(run_enable(&["--local".to_string(), "extra".to_string()]).unwrap()),
        0
    );
}

#[test]
fn config_requires_a_get_or_set_subcommand() {
    assert_ne!(exit_code_to_u8(run_config(&[]).unwrap()), 0);
    assert_ne!(
        exit_code_to_u8(run_config(&["bogus".to_string()]).unwrap()),
        0
    );
    assert_eq!(
        exit_code_to_u8(run_config(&["--help".to_string()]).unwrap()),
        0
    );
}

#[test]
fn config_get_requires_exactly_one_key() {
    assert_ne!(exit_code_to_u8(run_config_get(&[]).unwrap()), 0);
    assert_ne!(
        exit_code_to_u8(
            run_config_get(&["plugin.enabled".to_string(), "extra".to_string()]).unwrap()
        ),
        0
    );
}

/// `config set` refuses a key not rooted at `plugin` before it ever touches
/// the filesystem — the same rejection [`config_rejects_a_key_not_rooted_at_plugin`]
/// exercises directly on [`validate_plugin_key`], reached this time through
/// the full argv path.
#[test]
fn config_set_rejects_a_non_plugin_key_before_touching_the_filesystem() {
    assert_ne!(
        exit_code_to_u8(
            run_config_set(&["files".to_string(), "true".to_string()]).unwrap()
        ),
        0
    );
}

#[test]
fn config_set_requires_a_key_and_a_value() {
    assert_ne!(
        exit_code_to_u8(run_config_set(&["plugin.enabled".to_string()]).unwrap()),
        0
    );
}
