use super::*;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn fresh() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Seed a `.superset/.magic/` subtree under `root` shaped like the plugin's
/// real state (a session directory, a cached conclusion, a pending one-shot
/// claim, and the checklist pointer). Mirrors the identically-named helper
/// in `superset_files::tests` — this module exercises the invariant (KTD2)
/// at the `init`/`migrate` call sites rather than at `copy_into_repo`
/// directly. Returns each file's repo-relative path paired with its bytes,
/// for a byte-for-byte survival assertion.
fn seed_plugin_state(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let entries: &[(&str, &[u8])] = &[
        (
            ".superset/.magic/sessions/2026-08-30-abc123/session.json",
            b"{\"status\":\"active\"}",
        ),
        (
            ".superset/.magic/cache/conclusions/deadbeef.json",
            b"{\"conclusion\":\"cached result\"}",
        ),
        (
            ".superset/.magic/claims/pending-one-shot.json",
            b"{\"claim\":\"one-shot-42\"}",
        ),
        (".superset/.magic/checklist-pointer.json", b"{\"seq\":7}"),
    ];
    let mut written = Vec::new();
    for (rel, body) in entries {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        written.push((PathBuf::from(rel), body.to_vec()));
    }
    written
}

/// Assert every `(rel, body)` pair seeded by [`seed_plugin_state`] is still
/// present under `root`, byte-for-byte.
fn assert_plugin_state_intact(root: &Path, seeded: &[(PathBuf, Vec<u8>)]) {
    for (rel, body) in seeded {
        let path = root.join(rel);
        assert!(path.is_file(), "{} must survive", rel.display());
        assert_eq!(
            &fs::read(&path).unwrap(),
            body,
            "{} must survive byte-for-byte",
            rel.display()
        );
    }
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
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();

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

/// KTD2: the same pre-existing `.superset/.magic/` plugin state survives
/// `migrate`, not just `init` — `stage_migration` + the materialize step
/// (`copy_into_repo` + `rename_setup_config`) never stage or name it, so it
/// must come through the old→new layout rewrite untouched.
#[test]
fn migration_preserves_plugin_state_ktd2() {
    let repo = fresh();
    seed_old_layout(repo.path());
    let seeded = seed_plugin_state(repo.path());
    let existing = superset_files::load_config(repo.path())
        .unwrap()
        .unwrap();

    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();
    superset_files::copy_into_repo(stage.path(), repo.path(), &[SETUP_SH_REL]).unwrap();
    rename_setup_config(repo.path()).unwrap();

    // Migration actually ran (sanity check we're not passing vacuously).
    assert!(!repo.path().join(".superset/setup.sh").exists());
    assert!(repo.path().join(".superset/magic.json").is_file());
    assert_plugin_state_intact(repo.path(), &seeded);
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
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();

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
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();

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
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();

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
    superset_files::write_magic_json(
        stage.path(),
        &superset_files::MagicConfig {
            files: vec!["**/.env".to_string()],
            ..Default::default()
        },
    )
    .unwrap();
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
    // The backups tree is gitignored up front by the same bootstrap step, so a
    // recovered secret is never committed even before the first sync runs.
    assert!(
        gi.lines().any(|l| l == ".superset/backups/"),
        "ss-magic init must gitignore the backups tree, got: {gi:?}"
    );
    // ...and so is the plugin's machine-local state tree (R4's eager half).
    // A fresh init protects it before any plugin state exists, rather than
    // waiting for `ss-magic-plugin enable` to add the rule lazily.
    assert!(
        gi.lines().any(|l| l == ".superset/.magic/"),
        "ss-magic init must gitignore the plugin state tree, got: {gi:?}"
    );

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

/// KTD2: a pre-existing `.superset/.magic/` subtree (the Claude plugin's
/// session state, conclusion cache, and pending one-shot claims) survives
/// `ss-magic init` byte-for-byte, even though it sits inside the very
/// `.superset/` directory `copy_into_repo` writes into and is never staged.
#[test]
fn run_init_noninteractive_preserves_plugin_state_ktd2() {
    let repo = fresh();
    let seeded = seed_plugin_state(repo.path());

    run_init_noninteractive(repo.path(), &["**/.env".to_string()]).unwrap();

    // Init actually ran (sanity check we're not passing vacuously).
    assert!(repo.path().join(".superset/magic.json").is_file());
    assert_plugin_state_intact(repo.path(), &seeded);
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
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();
    let staged_magic = superset_files::load_overlaid(stage.path())
        .unwrap()
        .unwrap();
    assert!(staged_magic.files.is_empty());
}

/// `stage_migration` preserves unknown top-level keys (KTD8) already present
/// in the repo's `magic.json` — the half-migrated case (both the old
/// setup.sh marker and the new magic.sh marker present) can already have one.
#[test]
fn stage_migration_preserves_existing_magic_json_unknown_keys() {
    let repo = fresh();
    let dot = repo.path().join(".superset");
    fs::create_dir_all(&dot).unwrap();
    fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();
    fs::write(
        dot.join("config.json"),
        r#"{"setup":["./.superset/setup.sh"],"teardown":[],"run":[]}"#,
    )
    .unwrap();
    fs::write(
        dot.join("magic.json"),
        r#"{"files":["**/.env"],"plugin":{"enabled":true}}"#,
    )
    .unwrap();
    let existing = superset_files::load_config(repo.path())
        .unwrap()
        .unwrap();

    let stage = fresh();
    stage_migration(repo.path(), stage.path(), &existing, None).unwrap();

    let raw = fs::read_to_string(stage.path().join(".superset/magic.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value.get("plugin"),
        Some(&serde_json::json!({"enabled": true})),
        "plugin block must survive migration staging"
    );
}

/// The edit-config path is `run_init` reused verbatim (see `tui/menu.rs`),
/// and `run_init_noninteractive` shares its exact staging logic
/// (`merge_files_into_magic_config`). Covering the non-interactive path
/// covers the shared mechanism `run_init`/edit-config relies on without
/// driving the interactive picker prompts.
#[test]
fn run_init_noninteractive_preserves_unknown_top_level_keys() {
    let repo = fresh();
    fs::create_dir_all(repo.path().join(".superset")).unwrap();
    fs::write(
        repo.path().join(".superset/magic.json"),
        r#"{"files":["**/.env"],"plugin":{"enabled":true,"name":"foo"},"future_key":"stays"}"#,
    )
    .unwrap();

    run_init_noninteractive(repo.path(), &["**/.dev.vars".to_string()]).unwrap();

    let raw = fs::read_to_string(repo.path().join(".superset/magic.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value.get("plugin"),
        Some(&serde_json::json!({"enabled": true, "name": "foo"})),
        "plugin block must survive `ss-magic init` re-run"
    );
    assert_eq!(
        value.get("future_key"),
        Some(&serde_json::json!("stays")),
        "unrecognized future key must survive `ss-magic init` re-run"
    );
    let files = value["files"].as_array().unwrap();
    let files: Vec<&str> = files.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(files.contains(&"**/.dev.vars"), "files: {files:?}");
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

// ── R66: clearing the pre-marketplace skills-directory copy ─────────────────

/// An earlier revision of this tool copied the plugin into
/// `~/.claude/skills/ss-magic/`. A marketplace install outranks that copy, so
/// the leftover does no work and only shows up as a conflict in the harness's
/// plugin-errors view — `init` and `migrate` remove it.
#[test]
fn removes_a_legacy_skills_directory() {
    let home = fresh();
    let legacy = home.path().join(LEGACY_SKILLS_REL);
    fs::create_dir_all(legacy.join("nested")).unwrap();
    fs::write(legacy.join("SKILL.md"), "old copy").unwrap();
    // A sibling skill from some other tool, which is none of our business.
    let sibling = home.path().join(".claude/skills/other");
    fs::create_dir_all(&sibling).unwrap();
    fs::write(sibling.join("SKILL.md"), "not ours").unwrap();

    assert!(remove_legacy_skills_dir(home.path()).unwrap());

    assert!(!legacy.exists());
    assert_eq!(fs::read_to_string(sibling.join("SKILL.md")).unwrap(), "not ours");
    assert!(home.path().join(".claude/skills").is_dir());
}

/// The ordinary case on any machine that never ran the old revision: nothing
/// to remove, and nothing reported.
#[test]
fn removing_an_absent_legacy_skills_directory_is_a_no_op() {
    let home = fresh();
    assert!(!remove_legacy_skills_dir(home.path()).unwrap());

    // Not even the parent directories are created on the way to looking.
    assert!(!home.path().join(".claude").exists());
}

/// A symlink at that path has the LINK removed, never the tree it points at —
/// deleting someone's checked-out plugin source because they symlinked it into
/// place would be an unrecoverable surprise.
#[test]
fn a_symlinked_legacy_skills_path_loses_only_the_link() {
    let home = fresh();
    let target = fresh();
    fs::write(target.path().join("SKILL.md"), "the real thing").unwrap();
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(target.path(), home.path().join(LEGACY_SKILLS_REL)).unwrap();

    assert!(remove_legacy_skills_dir(home.path()).unwrap());

    assert!(!home.path().join(LEGACY_SKILLS_REL).exists());
    assert_eq!(
        fs::read_to_string(target.path().join("SKILL.md")).unwrap(),
        "the real thing"
    );
}


/// The one bootstrap step all three write paths share must write all three
/// rules.
///
/// `run_migrate`, `run_init` and `run_init_noninteractive` each call
/// [`ensure_bootstrap_gitignores`] and nothing else writes these rules, so
/// asserting here covers the two interactive paths that have no unit test of
/// their own. It is a real gap-filler rather than a restatement: before this
/// test, deleting the `state_tree::ensure_state_ignored` line left the entire
/// workspace suite green, and R4's eager half would have vanished silently.
/// The lazy half (`ss-magic-plugin enable` / `config set`) is covered
/// separately in `plugin/config/tests.rs`; both call the same function through
/// the `scratchpad` re-export, so they cannot drift apart.
#[test]
fn bootstrap_gitignores_cover_local_config_backups_and_state_tree() {
    let repo = tempfile::TempDir::new().unwrap();
    fs::write(repo.path().join(".gitignore"), "/target\n").unwrap();

    ensure_bootstrap_gitignores(repo.path()).unwrap();

    let gi = fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    for rule in [MAGIC_LOCAL_REL, ".superset/backups/", ".superset/.magic/"] {
        assert!(
            gi.lines().any(|l| l == rule),
            "ensure_bootstrap_gitignores must write {rule:?}, got: {gi:?}"
        );
    }
}

// ── Orca files written by init / migrate / edit-config ──────────────────────

use crate::orca::worktreeinclude;
use ss_magic_core::testutil::{git_run, init_main_repo, write_file};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn read_text(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn stored_shared(root: &Path) -> Option<Vec<String>> {
    superset_files::load_magic_json(root)
        .unwrap()
        .unwrap()
        .shared_directories
}

fn orca_dirs(root: &Path) -> Vec<String> {
    crate::orca::orca_yaml::shared_directories(&read_text(root, "orca.yaml"))
}

fn orca_setup_has_the_line(root: &Path) -> bool {
    crate::orca::orca_yaml::has_setup_line(&read_text(root, "orca.yaml"))
}

/// The wrapper entry Superset runs and the line Orca runs are one and the same
/// command; the two constants must never drift apart.
#[test]
fn the_orca_setup_line_is_the_superset_wrapper_entry() {
    assert_eq!(MAGIC_WRAPPER_ENTRY, crate::orca::orca_yaml::SETUP_LINE);
}

/// F1 in a git repository: the shared directories are detected and stored as
/// patterns, `orca.yaml` lists them beside the setup line, `.worktreeinclude`
/// holds the snapshot, and the snapshot file is gitignored.
#[test]
fn noninteractive_init_in_a_git_repo_writes_both_orca_files() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\nnode_modules/\n");
    write_file(root, ".env", "S=1\n");
    write_file(root, "apps/web/.env", "S=1\n");
    write_file(root, "node_modules/a/index.js", "x");
    write_file(root, "apps/web/node_modules/b/index.js", "x");

    run_init_noninteractive(root, &strings(&["**/.env", ".env"])).unwrap();

    assert_eq!(stored_shared(root), Some(strings(&["**/node_modules"])));
    assert_eq!(
        orca_dirs(root),
        strings(&["apps/web/node_modules", "node_modules"])
    );
    assert!(orca_setup_has_the_line(root));

    let include = read_text(root, worktreeinclude::FILE_NAME);
    assert_eq!(
        worktreeinclude::entries(&include),
        strings(&[".env", ".superset/magic.local.json", "apps/web/.env"]),
        "the snapshot is taken after the tree materializes, so the bootstrapped \
         overlay is in it"
    );
    let gitignore = read_text(root, ".gitignore");
    assert!(
        gitignore.lines().any(|l| l == worktreeinclude::FILE_NAME),
        "the snapshot file must be gitignored: {gitignore:?}"
    );
    // orca.yaml is an ordinary untracked file for the team to commit.
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain", "--", "orca.yaml", worktreeinclude::FILE_NAME])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "?? orca.yaml\n");
}

/// Outside git nothing is positively ignored: init still succeeds, with an
/// empty directory list and an empty section.
#[test]
fn noninteractive_init_outside_git_writes_empty_orca_lists() {
    let repo = fresh();
    write_file(repo.path(), ".env", "S=1\n");
    write_file(repo.path(), "node_modules/a/index.js", "x");

    run_init_noninteractive(repo.path(), &strings(&[".env"])).unwrap();

    assert_eq!(
        read_text(repo.path(), "orca.yaml"),
        "scripts:\n  setup: |\n    ./.superset/magic.sh sync\nworktree:\n  sharedDirectories: []\n"
    );
    assert_eq!(
        stored_shared(repo.path()),
        None,
        "nothing detected and nobody asked: the key is left out, not frozen at `[]`"
    );
    let include = read_text(repo.path(), worktreeinclude::FILE_NAME);
    assert!(worktreeinclude::entries(&include).is_empty(), "{include}");
    assert!(include.contains("ss-magic"), "the section markers are there: {include}");
}

/// AE4 at the flow level: an existing `orca.yaml` keeps its script and its
/// other keys; the script gains the sync line.
#[test]
fn ae4_init_keeps_an_existing_orca_yaml_and_adds_the_line() {
    let repo = fresh();
    let original = "# ours\nscripts:\n  setup: pnpm install\ndefaultTabs:\n  - title: dev\n    command: pnpm dev\n";
    write_file(repo.path(), "orca.yaml", original);

    run_init_noninteractive(repo.path(), &[]).unwrap();

    let yaml = read_text(repo.path(), "orca.yaml");
    assert!(yaml.contains("./.superset/magic.sh sync\n    pnpm install\n"), "{yaml}");
    assert!(
        yaml.contains("defaultTabs:\n  - title: dev\n    command: pnpm dev\n"),
        "defaultTabs must be byte-identical: {yaml}"
    );
    assert!(yaml.starts_with("# ours\n"));
}

/// AE9 at the flow level: an `orca.yaml` ss-magic cannot edit is left
/// byte-identical, the summary and the notices say so and show the block to add
/// by hand, and every other file is still written.
#[test]
fn ae9_an_uneditable_orca_yaml_is_left_alone_and_everything_else_is_written() {
    let repo = fresh();
    let original = "scripts: {setup: make}\n";
    write_file(repo.path(), "orca.yaml", original);

    let plan = orca_write::plan(repo.path(), &[], SetupPolicy::Ensure);
    let summary = plan.summary_lines();
    assert!(
        summary[0].starts_with("Cannot edit orca.yaml (`scripts` is a flow mapping"),
        "{summary:?}"
    );

    let code = run_init_noninteractive(repo.path(), &strings(&[".env"])).unwrap();

    assert_eq!(code, ExitCode::SUCCESS, "an uneditable orca.yaml never fails init");
    assert_eq!(read_text(repo.path(), "orca.yaml"), original);
    assert!(repo.path().join(".superset/magic.json").is_file());
    assert!(repo.path().join(".superset/config.json").is_file());
    assert!(repo.path().join(worktreeinclude::FILE_NAME).is_file());

    let notices = plan.apply(repo.path());
    let text: String = notices.iter().map(|n| format!("{}\n", n.text)).collect();
    assert!(text.contains("orca.yaml was not changed"), "{text}");
    assert!(text.contains("flow mapping"), "{text}");
    assert!(
        text.contains("    scripts:\n      setup: |\n        ./.superset/magic.sh sync\n"),
        "the block to add by hand is shown: {text}"
    );
}

/// An `orca.yaml` committed as a link to a file outside the repository is
/// never written through; a link that stays inside the repository is.
#[cfg(unix)]
#[test]
fn an_orca_yaml_symlink_out_of_the_repository_is_never_written_through() {
    let outside = fresh();
    let victim = outside.path().join("someone-elses.yaml");
    fs::write(&victim, "private: true\n").unwrap();
    let repo = fresh();
    std::os::unix::fs::symlink(&victim, repo.path().join("orca.yaml")).unwrap();

    let plan = orca_write::plan(repo.path(), &[], SetupPolicy::Ensure);
    assert!(
        plan.summary_lines()[0].contains("outside the repository"),
        "{:?}",
        plan.summary_lines()
    );
    run_init_noninteractive(repo.path(), &[]).unwrap();

    assert_eq!(fs::read_to_string(&victim).unwrap(), "private: true\n");
    assert!(repo.path().join(".superset/magic.json").is_file());
    assert!(repo.path().join(worktreeinclude::FILE_NAME).is_file());

    // In-repo link: written through onto its target, and still a link.
    let repo = fresh();
    write_file(repo.path(), "config/orca.yaml", "defaultTabs: []\n");
    std::os::unix::fs::symlink("config/orca.yaml", repo.path().join("orca.yaml")).unwrap();

    run_init_noninteractive(repo.path(), &[]).unwrap();

    let target = read_text(repo.path(), "config/orca.yaml");
    assert!(target.starts_with("defaultTabs: []\n") && target.contains("magic.sh sync"), "{target}");
    assert!(fs::symlink_metadata(repo.path().join("orca.yaml"))
        .unwrap()
        .file_type()
        .is_symlink());
}

/// A `.worktreeinclude` that is not text is never rebuilt from an empty parse:
/// its bytes stay, with a notice, and the rest is written.
#[test]
fn a_non_utf8_worktreeinclude_is_left_untouched_with_a_notice() {
    let repo = fresh();
    let bytes = b"caf\xe9/secret.pem\n";
    fs::write(repo.path().join(worktreeinclude::FILE_NAME), bytes).unwrap();

    let plan = orca_write::plan(repo.path(), &[], SetupPolicy::Ensure);
    assert!(
        plan.summary_lines()[1].starts_with("Leave .worktreeinclude untouched"),
        "{:?}",
        plan.summary_lines()
    );
    run_init_noninteractive(repo.path(), &[]).unwrap();

    assert_eq!(fs::read(repo.path().join(worktreeinclude::FILE_NAME)).unwrap(), bytes);
    assert!(repo.path().join("orca.yaml").is_file());
    let notices = plan.apply(repo.path());
    assert!(
        notices
            .iter()
            .any(|n| n.level == orca_write::Level::Warn && n.text.contains("UTF-8")),
        "{notices:?}"
    );
}

/// "Chose none" is a stored choice. A later non-interactive init keeps the
/// list empty instead of detecting the directories again.
#[test]
fn a_stored_empty_shared_list_stays_empty_on_a_later_init() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n");
    write_file(root, "node_modules/a/index.js", "x");
    // What the interactive picker stores when every row is unticked.
    superset_files::write_magic_json(
        root,
        &superset_files::merge_files_into_magic_config(None, Vec::new(), Some(Vec::new())),
    )
    .unwrap();

    run_init_noninteractive(root, &[]).unwrap();

    assert_eq!(stored_shared(root), Some(Vec::new()));
    assert!(orca_dirs(root).is_empty());
}

/// A stored list is used as it is – including a custom pattern – and a kind
/// that is not stored is not added back just because it matches.
#[test]
fn a_stored_shared_list_is_used_as_it_is() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n.venv/\ndist/\n");
    write_file(root, "node_modules/a/index.js", "x");
    write_file(root, ".venv/pyvenv.cfg", "x");
    write_file(root, "packages/ui/dist/index.js", "x");
    superset_files::write_magic_json(
        root,
        &superset_files::merge_files_into_magic_config(
            None,
            Vec::new(),
            Some(strings(&["**/.venv", "**/dist"])),
        ),
    )
    .unwrap();

    run_init_noninteractive(root, &[]).unwrap();

    assert_eq!(stored_shared(root), Some(strings(&["**/.venv", "**/dist"])));
    assert_eq!(orca_dirs(root), strings(&[".venv", "packages/ui/dist"]));
}

/// A tracked `.worktreeinclude` still gets its section, but no ignore rule –
/// a rule cannot ignore a tracked file – and the notice says so.
#[test]
fn a_tracked_worktreeinclude_gets_its_section_but_no_ignore_rule() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\n");
    write_file(root, ".env", "S=1\n");
    write_file(root, worktreeinclude::FILE_NAME, "by/hand.pem\n");
    git_run(&["add", ".gitignore", worktreeinclude::FILE_NAME], root);
    git_run(&["commit", "-q", "-m", "track the list"], root);

    let plan = orca_write::plan(root, &[], SetupPolicy::Ensure);
    run_init_noninteractive(root, &strings(&[".env"])).unwrap();

    let include = read_text(root, worktreeinclude::FILE_NAME);
    assert!(include.starts_with("by/hand.pem\n"), "{include}");
    assert!(worktreeinclude::entries(&include).contains(&".env".to_string()));
    assert!(
        !read_text(root, ".gitignore")
            .lines()
            .any(|l| l == worktreeinclude::FILE_NAME),
        "no ignore rule may be added for a tracked file"
    );
    let notices = plan.apply(root);
    assert!(
        notices.iter().any(|n| n.text.contains("tracked by git")),
        "{notices:?}"
    );
}

/// Migration from the legacy `setup.sh` layout writes the Orca files alongside
/// the migrated tree.
#[test]
fn migration_writes_the_orca_files_alongside_the_new_layout() {
    let main = init_main_repo("main");
    let root = main.path();
    seed_old_layout(root);
    write_file(root, ".gitignore", ".env\nnode_modules/\n");
    write_file(root, "apps/web/.env", "S=1\n");
    write_file(root, "node_modules/a/index.js", "x");
    let existing = superset_files::load_config(root).unwrap().unwrap();
    let shared = shared_dirs::stored_or_detected(root, None);
    assert_eq!(shared, Some(strings(&["**/node_modules"])));
    let plan = orca_write::plan(
        root,
        shared.as_deref().unwrap_or_default(),
        SetupPolicy::Ensure,
    );
    assert_eq!(
        plan.summary_lines(),
        vec![
            "Create orca.yaml (Orca setup script, 1 shared directory)".to_string(),
            "Write .worktreeinclude (files Orca copies into a new worktree) + add it to .gitignore"
                .to_string(),
        ]
    );

    materialize_migration(root, &existing, shared.as_deref(), &plan).unwrap();

    // The migrated Superset tree ...
    assert!(!root.join(".superset/setup.sh").exists());
    assert!(!root.join(".superset/setup_config.json").exists());
    let magic = superset_files::load_magic_json(root).unwrap().unwrap();
    assert_eq!(magic.files, strings(&["**/.env", "apps/*/.dev.vars"]));
    assert_eq!(magic.shared_directories, Some(strings(&["**/node_modules"])));
    // ... and the Orca files beside it.
    assert_eq!(orca_dirs(root), strings(&["node_modules"]));
    assert!(orca_setup_has_the_line(root));
    assert_eq!(
        worktreeinclude::entries(&read_text(root, worktreeinclude::FILE_NAME)),
        strings(&["apps/web/.env"])
    );
}

/// The summary names what will happen to `orca.yaml`: created, updated, kept
/// or impossible to edit.
#[test]
fn the_summary_says_create_update_keep_or_cannot_edit() {
    let repo = fresh();
    let line = |root: &Path| orca_write::plan(root, &[], SetupPolicy::Ensure).summary_lines()[0].clone();

    assert_eq!(
        line(repo.path()),
        "Create orca.yaml (Orca setup script, 0 shared directories)"
    );
    write_file(repo.path(), "orca.yaml", "defaultTabs: []\n");
    assert_eq!(
        line(repo.path()),
        "Update orca.yaml (Orca setup script, 0 shared directories)"
    );
    run_init_noninteractive(repo.path(), &[]).unwrap();
    assert_eq!(line(repo.path()), "Keep orca.yaml (already up to date)");
    write_file(repo.path(), "orca.yaml", "scripts: [oops\n");
    assert!(
        line(repo.path()).starts_with("Cannot edit orca.yaml (it is not valid YAML"),
        "{}",
        line(repo.path())
    );
}

/// Running init twice is quiet the second time: nothing about the Orca files
/// changes, and the ignore rule is not appended again.
#[test]
fn a_second_init_changes_nothing_about_the_orca_files() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", ".env\nnode_modules/\n");
    write_file(root, ".env", "S=1\n");
    write_file(root, "node_modules/a/index.js", "x");
    run_init_noninteractive(root, &strings(&[".env"])).unwrap();
    let snapshot = |rel: &str| read_text(root, rel);
    let before = (
        snapshot("orca.yaml"),
        snapshot(worktreeinclude::FILE_NAME),
        snapshot(".gitignore"),
        snapshot(".superset/magic.json"),
    );

    run_init_noninteractive(root, &strings(&[".env"])).unwrap();

    assert_eq!(
        (
            snapshot("orca.yaml"),
            snapshot(worktreeinclude::FILE_NAME),
            snapshot(".gitignore"),
            snapshot(".superset/magic.json"),
        ),
        before
    );
}

/// A committing finishing action stages `orca.yaml` with the `.superset` tree
/// and never the per-machine snapshot.
#[test]
fn the_finishing_action_stages_orca_yaml_and_never_the_snapshot() {
    assert!(FINAL_ACTION_PATHS.contains(&"orca.yaml"));
    assert!(!FINAL_ACTION_PATHS.contains(&worktreeinclude::FILE_NAME));

    let main = init_main_repo("main");
    let root = main.path();
    run_init_noninteractive(root, &[]).unwrap();
    git::stage_paths(root, &FINAL_ACTION_PATHS).unwrap();

    let staged = std::process::Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(root)
        .output()
        .unwrap();
    let staged = String::from_utf8_lossy(&staged.stdout);
    assert!(staged.lines().any(|l| l == "orca.yaml"), "{staged}");
    assert!(
        !staged.lines().any(|l| l == worktreeinclude::FILE_NAME),
        "the snapshot is gitignored and must not be staged: {staged}"
    );
}

/// A scripted init that detects nothing must not record a choice nobody made.
/// Run before dependencies are installed, it leaves `sharedDirectories` out of
/// `magic.json`; run again once they exist, it detects and stores them.
#[test]
fn an_init_that_detects_nothing_leaves_the_choice_open_for_a_later_run() {
    let main = init_main_repo("main");
    let root = main.path();
    write_file(root, ".gitignore", "node_modules/\n");

    run_init_noninteractive(root, &[]).unwrap();

    assert_eq!(stored_shared(root), None);
    assert!(
        !read_text(root, ".superset/magic.json").contains("sharedDirectories"),
        "the key must be absent from the file"
    );
    assert!(orca_dirs(root).is_empty());

    // `npm install` happens, then init runs again.
    write_file(root, "node_modules/a/index.js", "x");
    run_init_noninteractive(root, &[]).unwrap();

    assert_eq!(stored_shared(root), Some(strings(&["**/node_modules"])));
    assert_eq!(orca_dirs(root), strings(&["node_modules"]));
}
