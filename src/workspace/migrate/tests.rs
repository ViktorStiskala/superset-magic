use super::*;
use std::fs;
use tempfile::TempDir;

fn fresh() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn cfg(setup: Vec<&str>, teardown: Vec<&str>, run: Vec<&str>) -> Config {
    Config {
        setup: setup.into_iter().map(String::from).collect(),
        teardown: teardown.into_iter().map(String::from).collect(),
        run: run.into_iter().map(String::from).collect(),
    }
}

// ── detect_branch truth table ───────────────────────────────────────────

/// AE5: setup references neither marker → Init.
#[test]
fn ae5_detect_neither_marker_is_init() {
    let c = cfg(vec!["uv sync", "pnpm install"], vec![], vec![]);
    assert_eq!(detect_branch(Some(&c)), Branch::Init);
}

/// config.json absent → Init.
#[test]
fn detect_absent_config_is_init() {
    assert_eq!(detect_branch(None), Branch::Init);
}

/// Old setup.sh reference → Migrate.
#[test]
fn detect_setup_sh_is_migrate() {
    let c = cfg(vec!["./.superset/setup.sh"], vec![], vec![]);
    assert_eq!(detect_branch(Some(&c)), Branch::Migrate);
}

/// magic.sh marker only → Normal.
#[test]
fn detect_magic_marker_only_is_normal() {
    let c = cfg(vec![MAGIC_WRAPPER_ENTRY], vec![], vec![]);
    assert_eq!(detect_branch(Some(&c)), Branch::Normal);
}

/// `ss-magic sync` style marker (no magic.sh) → Normal.
#[test]
fn detect_ss_magic_sync_marker_is_normal() {
    let c = cfg(vec!["ss-magic sync"], vec![], vec![]);
    assert_eq!(detect_branch(Some(&c)), Branch::Normal);
}

/// Both markers present → Migrate wins.
#[test]
fn detect_both_markers_is_migrate() {
    let c = cfg(
        vec!["./.superset/setup.sh", MAGIC_WRAPPER_ENTRY],
        vec![],
        vec![],
    );
    assert_eq!(detect_branch(Some(&c)), Branch::Migrate);
}

/// Empty setup → Init (neither marker).
#[test]
fn detect_empty_setup_is_init() {
    let c = cfg(vec![], vec![], vec![]);
    assert_eq!(detect_branch(Some(&c)), Branch::Init);
}

/// Malformed config.json is a HARD ERROR at the load seam — never silently
/// classified as Init. `detect_branch` only ever sees a successfully
/// parsed `Option<&Config>`; the caller (U10) surfaces the parse error
/// from `load_config`, which names the path. This pins that contract so a
/// malformed file can never reach `detect_branch` as `None`.
#[test]
fn malformed_config_is_hard_error_not_init() {
    let repo = fresh();
    fs::create_dir_all(repo.path().join(".superset")).unwrap();
    fs::write(repo.path().join(".superset/config.json"), "{not json").unwrap();

    let err = superset_files::load_config(repo.path()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("config.json"), "error must name the path: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

// ── migrated_setup: marker-replace-in-place + preservation ──────────────

/// setup.sh is replaced in place by the wrapper; other entries keep order.
#[test]
fn migrated_setup_replaces_in_place_preserving_order() {
    let old = vec![
        "echo before".to_string(),
        "./.superset/setup.sh".to_string(),
        "uv sync".to_string(),
    ];
    let new = migrated_setup(&old);
    assert_eq!(
        new,
        vec![
            "echo before".to_string(),
            MAGIC_WRAPPER_ENTRY.to_string(),
            "uv sync".to_string(),
        ]
    );
}

/// Both markers already present → setup.sh stripped, wrapper kept once,
/// no duplicate wrapper.
#[test]
fn migrated_setup_both_markers_strips_setup_sh_keeps_wrapper_once() {
    let old = vec![
        "./.superset/setup.sh".to_string(),
        MAGIC_WRAPPER_ENTRY.to_string(),
        "pnpm i".to_string(),
    ];
    let new = migrated_setup(&old);
    assert_eq!(
        new,
        vec![MAGIC_WRAPPER_ENTRY.to_string(), "pnpm i".to_string()],
        "setup.sh dropped, wrapper not duplicated"
    );
    assert_eq!(
        new.iter().filter(|e| *e == MAGIC_WRAPPER_ENTRY).count(),
        1
    );
}

/// A lone setup.sh entry becomes a lone wrapper entry.
#[test]
fn migrated_setup_lone_setup_sh_becomes_wrapper() {
    let old = vec!["./.superset/setup.sh".to_string()];
    assert_eq!(migrated_setup(&old), vec![MAGIC_WRAPPER_ENTRY.to_string()]);
}

/// Two raw setup.sh entries, no pre-existing wrapper: the FIRST becomes the
/// wrapper in place, the SECOND is dropped (no duplicate wrapper), and the
/// intervening command keeps its position.
#[test]
fn migrated_setup_two_raw_setup_sh_keeps_wrapper_once() {
    let old = vec![
        "./.superset/setup.sh".to_string(),
        "uv sync".to_string(),
        "./.superset/setup.sh".to_string(),
    ];
    let new = migrated_setup(&old);
    assert_eq!(
        new,
        vec![MAGIC_WRAPPER_ENTRY.to_string(), "uv sync".to_string()],
        "first setup.sh → wrapper, second dropped, command preserved"
    );
    assert_eq!(new.iter().filter(|e| *e == MAGIC_WRAPPER_ENTRY).count(), 1);
}

// ── stage_migration: file transforms (no UI) ────────────────────────────

/// Seed the repo with the OLD layout: setup.sh + setup_config.json +
/// config.json referencing setup.sh, plus teardown/run to preserve.
fn seed_old_layout(root: &Path) {
    let dot = root.join(".superset");
    fs::create_dir_all(&dot).unwrap();
    fs::write(dot.join("setup.sh"), "#!/bin/bash\necho old\n").unwrap();
    fs::write(
        dot.join("setup_config.json"),
        r#"{"files":["**/.env","apps/*/.dev.vars"]}"#,
    )
    .unwrap();
    fs::write(
        dot.join("config.json"),
        r#"{"setup":["./.superset/setup.sh","uv sync"],"teardown":["./drop.sh"],"run":["pnpm dev"]}"#,
    )
    .unwrap();
}

/// Old setup.sh reference → staged magic.json carries files, config.json
/// gets the wrapper, teardown/run preserved, magic.sh + magic.local.json
/// staged. Then materialize and assert setup.sh + setup_config.json gone.
#[test]
fn migration_transforms_old_layout_into_new() {
    let repo = fresh();
    seed_old_layout(repo.path());
    let existing = superset_files::load_config(repo.path())
        .unwrap()
        .unwrap();

    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing).unwrap();

    // Staged magic.json carries setup_config.json's files verbatim.
    let staged_magic = superset_files::load_overlaid(stage.path())
        .unwrap()
        .unwrap();
    assert_eq!(staged_magic.files, vec!["**/.env", "apps/*/.dev.vars"]);

    // Staged config.json: setup rewritten in place, teardown/run preserved.
    let staged_cfg = superset_files::load_config(stage.path())
        .unwrap()
        .unwrap();
    assert_eq!(
        staged_cfg.setup,
        vec![MAGIC_WRAPPER_ENTRY.to_string(), "uv sync".to_string()]
    );
    assert_eq!(staged_cfg.teardown, vec!["./drop.sh".to_string()]);
    assert_eq!(staged_cfg.run, vec!["pnpm dev".to_string()]);

    // Staged magic.sh + magic.local.json present.
    assert!(stage.path().join(".superset/magic.sh").is_file());
    assert!(stage.path().join(".superset/magic.local.json").is_file());

    // Now materialize the way run_migrate does, and assert the repo's
    // legacy files are gone and the new ones are present.
    superset_files::copy_into_repo(stage.path(), repo.path(), &[SETUP_SH_REL]).unwrap();
    rename_setup_config(repo.path()).unwrap();
    gitignore::ensure_entry(repo.path(), MAGIC_LOCAL_REL).unwrap();

    let dot = repo.path().join(".superset");
    assert!(!dot.join("setup.sh").exists(), "setup.sh must be deleted");
    assert!(
        !dot.join("setup_config.json").exists(),
        "setup_config.json must be renamed away"
    );
    assert!(dot.join("magic.json").is_file());
    assert!(dot.join("magic.sh").is_file());
    assert!(dot.join("magic.local.json").is_file());

    // magic.sh is executable (0755) on unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dot.join("magic.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "magic.sh must be 0755");
    }

    // .gitignore now ignores magic.local.json.
    let gi = fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(gi.lines().any(|l| l == MAGIC_LOCAL_REL));
}

/// Regression: a pre-existing magic.local.json (gitignored, therefore
/// unrecoverable) must NOT be clobbered by the empty bootstrap template —
/// the guard skips staging it so materialization leaves the user's local
/// overlay intact. (Mirrors the same guard in run_init/edit-config.)
#[test]
fn migration_preserves_existing_magic_local_json() {
    let repo = fresh();
    seed_old_layout(repo.path());
    let custom = "{\n  \"files\": [\"custom/**\"]\n}\n";
    fs::write(repo.path().join(".superset/magic.local.json"), custom).unwrap();
    let existing = superset_files::load_config(repo.path()).unwrap().unwrap();

    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing).unwrap();

    // The guard kept magic.local.json OUT of the stage.
    assert!(
        !stage.path().join(".superset/magic.local.json").exists(),
        "existing repo magic.local.json must not be re-staged as the empty template"
    );

    superset_files::copy_into_repo(stage.path(), repo.path(), &[SETUP_SH_REL]).unwrap();

    // The user's custom overlay survived migration.
    let after =
        fs::read_to_string(repo.path().join(".superset/magic.local.json")).unwrap();
    assert_eq!(
        after, custom,
        "migration must not clobber an existing magic.local.json"
    );
}

/// Both-markers-present old config → migration still strips setup.sh and
/// keeps the wrapper exactly once in the staged config.json.
#[test]
fn migration_both_markers_strips_setup_sh_keeps_wrapper() {
    let repo = fresh();
    let dot = repo.path().join(".superset");
    fs::create_dir_all(&dot).unwrap();
    fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();
    fs::write(dot.join("setup_config.json"), r#"{"files":[]}"#).unwrap();
    fs::write(
        dot.join("config.json"),
        format!(
            r#"{{"setup":["./.superset/setup.sh","{MAGIC_WRAPPER_ENTRY}"],"teardown":[],"run":[]}}"#
        ),
    )
    .unwrap();
    let existing = superset_files::load_config(repo.path())
        .unwrap()
        .unwrap();

    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing).unwrap();

    let staged_cfg = superset_files::load_config(stage.path())
        .unwrap()
        .unwrap();
    assert_eq!(staged_cfg.setup, vec![MAGIC_WRAPPER_ENTRY.to_string()]);
    assert_eq!(
        staged_cfg
            .setup
            .iter()
            .filter(|e| *e == MAGIC_WRAPPER_ENTRY)
            .count(),
        1,
        "wrapper must not be duplicated"
    );
}

/// AE6: an already-migrated repo (Normal branch) is the idempotent case.
/// `detect_branch` returns Normal so neither run_migrate nor any
/// rename/delete fires; we assert the branch decision and that
/// `migrated_setup` on an already-migrated array is a no-op (no duplicate
/// wrapper, nothing stripped).
#[test]
fn ae6_already_migrated_is_normal_and_idempotent() {
    let migrated = cfg(vec![MAGIC_WRAPPER_ENTRY, "uv sync"], vec!["./drop.sh"], vec![]);
    assert_eq!(detect_branch(Some(&migrated)), Branch::Normal);
    // migrated_setup is only called on the Migrate branch, but prove it's
    // a structural no-op should it ever run on already-migrated input.
    assert_eq!(
        migrated_setup(&migrated.setup),
        vec![MAGIC_WRAPPER_ENTRY.to_string(), "uv sync".to_string()]
    );
}

/// rename_setup_config is a no-op when the file is already absent.
#[test]
fn rename_setup_config_noop_when_absent() {
    let repo = fresh();
    fs::create_dir_all(repo.path().join(".superset")).unwrap();
    rename_setup_config(repo.path()).unwrap(); // must not error
    assert!(!repo.path().join(".superset/setup_config.json").exists());
}

/// Esc/abort safety (logic seam): `stage_migration` writes ONLY into the
/// tempdir. `run_migrate` calls it strictly AFTER `ui::pick_final_action()?`
/// returns Ok, so an Esc/Ctrl-C aborts via `?` before this runs — leaving
/// the on-disk old layout untouched. Here we prove the staging step itself
/// never mutates the repo: after staging, the repo still has the legacy
/// files and none of the new ones.
#[test]
fn staging_does_not_mutate_repo_until_materialized() {
    let repo = fresh();
    seed_old_layout(repo.path());

    // Snapshot the legacy on-disk files.
    let dot = repo.path().join(".superset");
    let setup_sh_before = fs::read_to_string(dot.join("setup.sh")).unwrap();
    let setup_cfg_before = fs::read_to_string(dot.join("setup_config.json")).unwrap();
    let config_before = fs::read_to_string(dot.join("config.json")).unwrap();

    let existing = superset_files::load_config(repo.path())
        .unwrap()
        .unwrap();
    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing).unwrap();

    // Repo is byte-identical: nothing was written, renamed, or deleted.
    assert_eq!(
        fs::read_to_string(dot.join("setup.sh")).unwrap(),
        setup_sh_before
    );
    assert_eq!(
        fs::read_to_string(dot.join("setup_config.json")).unwrap(),
        setup_cfg_before
    );
    assert_eq!(
        fs::read_to_string(dot.join("config.json")).unwrap(),
        config_before
    );
    assert!(
        !dot.join("magic.json").exists(),
        "magic.json must not appear in the repo before materialize"
    );
    assert!(!dot.join("magic.sh").exists());
    assert!(!dot.join("magic.local.json").exists());
    assert!(
        !repo.path().join(".gitignore").exists(),
        ".gitignore must not be created before materialize"
    );
}

/// `copy_into_repo` materializes magic.sh (0755) + magic.json (not the
/// legacy filenames) and deletes setup.sh from the repo via the delete set.
#[test]
fn copy_into_repo_materializes_magic_layout_and_deletes_setup_sh() {
    let repo = fresh();
    // Repo already has a legacy setup.sh that must be deleted.
    let dot = repo.path().join(".superset");
    fs::create_dir_all(&dot).unwrap();
    fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();

    // Stage the new layout.
    let stage = fresh();
    superset_files::write_magic_json(stage.path(), &["**/.env".to_string()]).unwrap();
    superset_files::write_magic_sh(stage.path()).unwrap();
    superset_files::write_config_json(
        stage.path(),
        &cfg(vec![MAGIC_WRAPPER_ENTRY], vec![], vec![]),
    )
    .unwrap();

    superset_files::copy_into_repo(stage.path(), repo.path(), &[SETUP_SH_REL]).unwrap();

    assert!(dot.join("magic.json").is_file(), "magic.json materialized");
    assert!(dot.join("magic.sh").is_file(), "magic.sh materialized");
    assert!(!dot.join("setup.sh").exists(), "setup.sh deleted");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dot.join("magic.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "magic.sh must be 0755");
    }
}

/// Init (AE5) seeds magic.json with default_magic_files() FIRST, then the
/// chosen patterns, deduped. magic.local.json is always present.
#[test]
fn init_magic_files_seeds_defaults_then_chosen_deduped() {
    let chosen = vec!["**/.env".to_string(), "**/.dev.vars".to_string()];
    let files = init_magic_files(&chosen);
    // Defaults first.
    assert_eq!(files[0], ".superset/magic.local.json");
    // Chosen appended, in order.
    assert_eq!(
        files,
        vec![
            ".superset/magic.local.json".to_string(),
            "**/.env".to_string(),
            "**/.dev.vars".to_string(),
        ]
    );
}

/// A chosen pattern that duplicates a default appears only once.
#[test]
fn init_magic_files_dedupes_chosen_against_defaults() {
    let chosen = vec![".superset/magic.local.json".to_string(), ".env".to_string()];
    let files = init_magic_files(&chosen);
    assert_eq!(
        files,
        vec![".superset/magic.local.json".to_string(), ".env".to_string()],
        "magic.local.json must not be duplicated"
    );
}

/// Non-interactive init (AN1) writes the full layout from CLI patterns
/// (no TUI, no git): magic.json (defaults + patterns), magic.sh,
/// config.json (setup → wrapper), magic.local.json, and the gitignore entry.
#[test]
fn run_init_noninteractive_writes_layout_from_patterns() {
    let repo = fresh();
    run_init_noninteractive(repo.path(), &["**/.env".to_string()]).unwrap();

    let dot = repo.path().join(".superset");
    assert!(dot.join("magic.json").is_file());
    assert!(dot.join("magic.sh").is_file());
    assert!(dot.join("config.json").is_file());
    assert!(dot.join("magic.local.json").is_file());

    let magic = superset_files::load_overlaid(repo.path()).unwrap().unwrap();
    assert!(magic.files.contains(&"**/.env".to_string()));
    assert!(magic
        .files
        .contains(&".superset/magic.local.json".to_string()));

    let cfg = superset_files::load_config(repo.path()).unwrap().unwrap();
    assert_eq!(cfg.setup, vec![MAGIC_WRAPPER_ENTRY.to_string()]);

    let gi = fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(gi.lines().any(|l| l == MAGIC_LOCAL_REL));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dot.join("magic.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "magic.sh must be 0755");
    }
}

/// Non-interactive init preserves a pre-existing magic.local.json (same
/// guard as the interactive path).
#[test]
fn run_init_noninteractive_preserves_existing_magic_local_json() {
    let repo = fresh();
    fs::create_dir_all(repo.path().join(".superset")).unwrap();
    let custom = "{\n  \"files\": [\"x/**\"]\n}\n";
    fs::write(repo.path().join(".superset/magic.local.json"), custom).unwrap();

    run_init_noninteractive(repo.path(), &[]).unwrap();

    let after =
        fs::read_to_string(repo.path().join(".superset/magic.local.json")).unwrap();
    assert_eq!(
        after, custom,
        "init must not clobber an existing magic.local.json"
    );
}

/// stage_migration with no setup_config.json on disk → magic.json has an
/// empty files array (no crash).
#[test]
fn stage_migration_without_setup_config_yields_empty_files() {
    let repo = fresh();
    let dot = repo.path().join(".superset");
    fs::create_dir_all(&dot).unwrap();
    fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();
    fs::write(
        dot.join("config.json"),
        r#"{"setup":["./.superset/setup.sh"],"teardown":[],"run":[]}"#,
    )
    .unwrap();
    let existing = superset_files::load_config(repo.path())
        .unwrap()
        .unwrap();

    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing).unwrap();
    let staged_magic = superset_files::load_overlaid(stage.path())
        .unwrap()
        .unwrap();
    assert!(staged_magic.files.is_empty());
}

// ── build_pattern_options ───────────────────────────────────────────────

/// Helper: all-false fs_match vector (no filesystem hits).
fn no_fs_hits() -> Vec<bool> {
    vec![false; crate::sync::repo_scan::OPTIONS.len()]
}

/// First-time init (no existing magic.json) → only OPTIONS, preselect only
/// fs hits (none here). Regression: no extra entries, no spurious preselects.
#[test]
fn build_pattern_options_first_time_init_empty_existing() {
    let (options, preselected) = build_pattern_options(&[], &no_fs_hits());
    // Options must be exactly the preconfigured OPTIONS.
    let expected_opts: Vec<String> =
        crate::sync::repo_scan::OPTIONS.iter().map(|s| s.to_string()).collect();
    assert_eq!(options, expected_opts, "first-time init must yield only OPTIONS");
    assert!(
        preselected.is_empty(),
        "no fs hits, no existing — nothing preselected; got: {preselected:?}"
    );
}

/// First-time init with a filesystem hit: that OPTIONS index is preselected.
#[test]
fn build_pattern_options_first_time_init_with_fs_hit() {
    // Simulate a hit on OPTIONS[1] = "**/.env".
    let mut fs_match = no_fs_hits();
    fs_match[1] = true;
    let (options, preselected) = build_pattern_options(&[], &fs_match);
    assert_eq!(options.len(), crate::sync::repo_scan::OPTIONS.len());
    assert_eq!(preselected, vec![1], "only the fs-hit index must be preselected");
}

/// A preconfigured OPTION already in magic.json is preselected even
/// without a filesystem hit.
#[test]
fn build_pattern_options_preconfigured_in_existing_preselected_without_fs_hit() {
    // ".env" is OPTIONS[0]; mark it as present in the existing magic.json.
    let existing = vec![".env".to_string()];
    let (options, preselected) = build_pattern_options(&existing, &no_fs_hits());
    assert_eq!(options.len(), crate::sync::repo_scan::OPTIONS.len());
    assert!(
        preselected.contains(&0),
        "OPTIONS[0] (.env) is in existing magic.json → must be preselected; got {preselected:?}"
    );
    // Custom count is zero — no extra options appended.
    assert_eq!(
        options.len(),
        crate::sync::repo_scan::OPTIONS.len(),
        "no custom patterns → options length must equal OPTIONS.len()"
    );
}

/// Existing custom patterns appear in options after OPTIONS and are always
/// preselected.
#[test]
fn build_pattern_options_custom_patterns_appended_and_preselected() {
    let existing = vec![
        "apps/*/.env".to_string(),
        "packages/**/.dev.vars".to_string(),
    ];
    let (options, preselected) = build_pattern_options(&existing, &no_fs_hits());

    let opts_len = crate::sync::repo_scan::OPTIONS.len();
    // Custom entries appended after the four OPTIONS.
    assert_eq!(
        options.len(),
        opts_len + 2,
        "two custom patterns must be appended"
    );
    assert_eq!(options[opts_len], "apps/*/.env");
    assert_eq!(options[opts_len + 1], "packages/**/.dev.vars");

    // Both custom indices are preselected.
    assert!(
        preselected.contains(&opts_len),
        "first custom pattern must be preselected"
    );
    assert!(
        preselected.contains(&(opts_len + 1)),
        "second custom pattern must be preselected"
    );
}

/// A pattern in existing magic.json that IS a preconfigured OPTION is NOT
/// double-counted as a custom entry (it goes into the OPTIONS preselection
/// path, not into the custom tail).
#[test]
fn build_pattern_options_preconfigured_not_duplicated_as_custom() {
    // "**/.env" is OPTIONS[1]. It must NOT appear again as a custom entry.
    let existing = vec!["**/.env".to_string(), "apps/*/.secrets".to_string()];
    let (options, preselected) = build_pattern_options(&existing, &no_fs_hits());

    let opts_len = crate::sync::repo_scan::OPTIONS.len();
    // Only one custom pattern (apps/*/.secrets); "**/.env" is an OPTION.
    assert_eq!(
        options.len(),
        opts_len + 1,
        "only one custom entry expected; got options: {options:?}"
    );
    assert_eq!(options[opts_len], "apps/*/.secrets");

    // OPTIONS[1] ("**/.env") is preselected (from existing), not at opts_len.
    assert!(preselected.contains(&1), "OPTIONS[1] must be preselected");
    assert!(
        preselected.contains(&opts_len),
        "custom 'apps/*/.secrets' must be preselected"
    );
}

/// Combined: fs hit on one OPTION, existing config has that same OPTION plus
/// a custom pattern. Both preselected, custom appended, no duplication.
#[test]
fn build_pattern_options_combined_fs_hit_and_existing() {
    // OPTIONS[0] = ".env" has a filesystem hit and is also in existing.
    let mut fs_match = no_fs_hits();
    fs_match[0] = true;
    let existing = vec![".env".to_string(), "custom/secret".to_string()];
    let (options, preselected) = build_pattern_options(&existing, &fs_match);

    let opts_len = crate::sync::repo_scan::OPTIONS.len();
    assert_eq!(options.len(), opts_len + 1, "one custom entry expected");
    assert_eq!(options[opts_len], "custom/secret");

    // OPTIONS[0] preselected (fs hit + in existing — still just one entry).
    assert_eq!(
        preselected.iter().filter(|&&i| i == 0).count(),
        1,
        "OPTIONS[0] must appear exactly once in preselected"
    );
    assert!(preselected.contains(&opts_len), "custom must be preselected");
}
