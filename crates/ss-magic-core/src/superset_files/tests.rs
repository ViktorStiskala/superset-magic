use super::*;
use std::fs;
use tempfile::TempDir;

/// The writers stage a sibling file and rename it over the target, so a
/// finished write leaves exactly the target behind – no `.magic.json.<pid>.tmp`
/// stranger for `git status` to show.
#[test]
fn magic_json_writers_leave_no_staging_file_behind() {
    let dir = TempDir::new().unwrap();
    let cfg = MagicConfig {
        files: vec![".env".to_string()],
        ..Default::default()
    };
    write_magic_json(dir.path(), &cfg).unwrap();
    write_magic_local_json(dir.path(), &cfg).unwrap();
    let names: Vec<String> = fs::read_dir(dir.path().join(".superset"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| n == MAGIC_JSON || n == MAGIC_LOCAL_JSON),
        "unexpected entries in .superset: {names:?}"
    );
    assert_eq!(load_magic_json(dir.path()).unwrap().unwrap().files, cfg.files);
}

/// Eight writers racing on one `magic.json` never leave a torn file: every
/// observed state is one whole document some writer produced. This is the
/// property the rename buys over a truncating `fs::write`, and it is asserted
/// concurrently on purpose – a sequential test cannot tell the two apart.
#[test]
fn concurrent_magic_json_writes_never_tear_the_file() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let root = root.clone();
            std::thread::spawn(move || {
                for round in 0..25 {
                    let cfg = MagicConfig {
                        files: (0..40).map(|k| format!("writer-{i}-round-{round}-file-{k}")).collect(),
                        ..Default::default()
                    };
                    write_magic_json(&root, &cfg).unwrap();
                    let seen = load_magic_json(&root).unwrap().unwrap();
                    assert_eq!(seen.files.len(), 40, "a reader saw a partial document");
                    let head = seen.files[0].clone();
                    assert!(
                        seen.files.iter().all(|f| f.starts_with(&head[..head.rfind("-file-").unwrap()])),
                        "a reader saw two writers' bytes mixed: {:?}",
                        seen.files
                    );
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

const OPTIONS: [&str; 4] = [".env", "**/.env", ".env.local", "**/.dev.vars"];

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

/// A `MagicConfig` with no extras — the shape a fixture wants when it isn't
/// exercising unknown-key preservation itself.
fn magic_cfg(files: &[&str]) -> MagicConfig {
    MagicConfig {
        files: files.iter().map(|s| s.to_string()).collect(),
        extras: serde_json::Map::new(),
    }
}

#[test]
fn write_config_json_emits_expected_shape() {
    let dir = fresh();
    let root = dir.path();
    write_config_json(
        root,
        &cfg(vec!["./.superset/magic.sh sync"], vec![], vec![]),
    )
    .unwrap();

    let dot = root.join(".superset");
    assert!(dot.join("config.json").is_file());

    // config.json matches the shape we wrote
    let parsed: Config =
        serde_json::from_str(&fs::read_to_string(dot.join("config.json")).unwrap()).unwrap();
    assert_eq!(parsed.setup, vec!["./.superset/magic.sh sync".to_string()]);
    assert!(parsed.teardown.is_empty());
    assert!(parsed.run.is_empty());
}

#[test]
fn load_config_returns_none_when_absent() {
    let dir = fresh();
    assert!(load_config(dir.path()).unwrap().is_none());
}

#[test]
fn load_config_round_trips() {
    let dir = fresh();
    let root = dir.path();
    fs::create_dir_all(root.join(".superset")).unwrap();
    let body = r#"{
      "setup": ["./.superset/setup.sh", "uv sync"],
      "teardown": ["./drop.sh"],
      "run": ["pnpm dev"]
    }"#;
    fs::write(root.join(".superset/config.json"), body).unwrap();

    let parsed = load_config(root).unwrap().unwrap();
    assert_eq!(parsed.setup, vec!["./.superset/setup.sh", "uv sync"]);
    assert_eq!(parsed.teardown, vec!["./drop.sh"]);
    assert_eq!(parsed.run, vec!["pnpm dev"]);
}

#[test]
fn malformed_config_returns_clean_error() {
    let dir = fresh();
    let root = dir.path();
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(root.join(".superset/config.json"), "{not json").unwrap();
    let err = load_config(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("config.json"), "msg: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

#[test]
fn merge_setup_into_config_with_none_yields_empty_teardown_run() {
    let merged = merge_setup_into_config(None, vec!["./.superset/setup.sh".to_string()]);
    assert_eq!(merged.setup, vec!["./.superset/setup.sh".to_string()]);
    assert!(merged.teardown.is_empty());
    assert!(merged.run.is_empty());
}

#[test]
fn merge_setup_into_config_preserves_teardown_and_run_verbatim() {
    let existing = cfg(
        vec!["./.superset/setup.sh"],
        vec!["./drop.sh", "psql -f cleanup.sql"],
        vec!["pnpm dev", "uv run task"],
    );
    let merged = merge_setup_into_config(
        Some(&existing),
        vec!["./.superset/setup.sh".into(), "uv sync".into()],
    );
    assert_eq!(merged.setup, vec!["./.superset/setup.sh", "uv sync"]);
    assert_eq!(merged.teardown, existing.teardown);
    assert_eq!(merged.run, existing.run);
}

#[test]
fn write_config_json_is_pretty_with_trailing_newline_and_round_trips() {
    let dir = fresh();
    let root = dir.path();
    let original = cfg(
        vec!["./.superset/setup.sh", "uv sync"],
        vec!["./drop.sh"],
        vec!["pnpm dev"],
    );
    write_config_json(root, &original).unwrap();

    let raw = fs::read_to_string(root.join(".superset/config.json")).unwrap();
    assert!(raw.contains('\n'), "expected pretty-printed JSON");
    assert!(raw.ends_with('\n'), "expected trailing newline");

    let parsed = load_config(root).unwrap().unwrap();
    assert_eq!(parsed.setup, original.setup);
    assert_eq!(parsed.teardown, original.teardown);
    assert_eq!(parsed.run, original.run);
}

#[test]
fn existing_unknown_entries_keeps_non_preconfigured() {
    let existing = vec![
        "apps/*/config".to_string(),
        ".env".to_string(),
        "packages/**/fixtures".to_string(),
    ];
    let unknown = existing_unknown_entries(&existing, &OPTIONS);
    assert_eq!(
        unknown,
        vec![
            "apps/*/config".to_string(),
            "packages/**/fixtures".to_string()
        ]
    );
}

/// `load_setup_config` (the legacy reader migration relies on) round-trips
/// a `files` array from a raw `setup_config.json` on disk.
#[test]
fn load_setup_config_reads_files_array() {
    let dir = fresh();
    let root = dir.path();
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(
        root.join(".superset/setup_config.json"),
        r#"{"files":[".env","**/.dev.vars","apps/*/config"]}"#,
    )
    .unwrap();

    let parsed = load_setup_config(root).unwrap().unwrap();
    assert_eq!(
        parsed.files,
        vec![
            ".env".to_string(),
            "**/.dev.vars".to_string(),
            "apps/*/config".to_string(),
        ]
    );
}

#[test]
fn malformed_setup_config_returns_clean_error() {
    let dir = fresh();
    let root = dir.path();
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(root.join(".superset/setup_config.json"), "{not json").unwrap();
    let err = load_setup_config(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("setup_config.json"), "msg: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

#[test]
fn copy_into_repo_materializes_all_staged_files() {
    let stage = fresh();
    let dest = fresh();
    write_magic_sh(stage.path()).unwrap();
    write_config_json(
        stage.path(),
        &cfg(vec!["./.superset/magic.sh sync"], vec![], vec![]),
    )
    .unwrap();
    write_magic_json(stage.path(), &magic_cfg(&[".env"])).unwrap();

    copy_into_repo(stage.path(), dest.path(), &[]).unwrap();

    let real = dest.path().join(".superset");
    assert!(real.join("magic.sh").is_file());
    assert!(real.join("config.json").is_file());
    assert!(real.join("magic.json").is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(real.join("magic.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}

#[test]
fn copy_into_repo_overwrites_existing_config_json() {
    let stage = fresh();
    let dest = fresh();
    write_magic_sh(stage.path()).unwrap();
    write_config_json(
        stage.path(),
        &cfg(vec!["./.superset/magic.sh sync", "uv sync"], vec![], vec![]),
    )
    .unwrap();
    write_magic_json(stage.path(), &magic_cfg(&[".env"])).unwrap();

    let dest_dir = dest.path().join(".superset");
    fs::create_dir_all(&dest_dir).unwrap();
    let pre_existing =
        r#"{"setup":["./.superset/magic.sh sync","./extra.sh"],"teardown":[],"run":[]}"#;
    fs::write(dest_dir.join("config.json"), pre_existing).unwrap();

    copy_into_repo(stage.path(), dest.path(), &[]).unwrap();
    let staged = fs::read_to_string(stage.path().join(".superset/config.json")).unwrap();
    let after = fs::read_to_string(dest_dir.join("config.json")).unwrap();
    assert_eq!(
        after, staged,
        "destination must mirror the staged config.json"
    );
}

/// Seed a `.superset/.magic/` subtree under `root` shaped like the plugin's
/// real state: a session directory, a cached conclusion, a pending one-shot
/// claim, and the checklist pointer file. Returns each file's repo-relative
/// path paired with the bytes written, so a caller can assert byte-for-byte
/// survival after `copy_into_repo` runs.
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

/// KTD2's invariant: `copy_into_repo` never removes a destination entry that
/// isn't named in its `delete` list. `.superset/.magic/` — the plugin's
/// session state, conclusion cache, and pending one-shot claims — lives
/// inside `repo_root/.superset/`, the very directory this function owns, yet
/// is never staged and never named in `delete`. It must survive byte-for-byte.
#[test]
fn copy_into_repo_preserves_untracked_plugin_state_ktd2() {
    let stage = fresh();
    let dest = fresh();
    write_magic_sh(stage.path()).unwrap();
    write_config_json(
        stage.path(),
        &cfg(vec!["./.superset/magic.sh sync"], vec![], vec![]),
    )
    .unwrap();
    write_magic_json(stage.path(), &magic_cfg(&[".env"])).unwrap();

    // Pre-existing plugin state in the destination, absent from the stage
    // and absent from `delete` — the exact shape the invariant protects.
    let seeded = seed_plugin_state(dest.path());

    // Also exercise a non-empty `delete` list that names something else
    // entirely, so the invariant is checked against a real deletion
    // happening elsewhere in the same call, not just an empty no-op.
    fs::write(dest.path().join(".superset/setup.sh"), "#!/bin/bash\n").unwrap();
    copy_into_repo(stage.path(), dest.path(), &[".superset/setup.sh"]).unwrap();

    assert!(
        !dest.path().join(".superset/setup.sh").exists(),
        "the named delete target must actually be removed"
    );
    for (rel, body) in &seeded {
        let path = dest.path().join(rel);
        assert!(path.is_file(), "{} must survive copy_into_repo", rel.display());
        assert_eq!(
            &fs::read(&path).unwrap(),
            body,
            "{} must survive byte-for-byte",
            rel.display()
        );
    }
}

#[test]
fn bootstrap_simulation_preserves_teardown_across_rerun() {
    // Pre-existing config.json on disk carries a non-empty teardown.
    let dest = fresh();
    let dest_dir = dest.path().join(".superset");
    fs::create_dir_all(&dest_dir).unwrap();
    let pre_existing = r#"{"setup":["./old.sh"],"teardown":["./drop.sh"],"run":[]}"#;
    fs::write(dest_dir.join("config.json"), pre_existing).unwrap();

    // Migration simulation: read existing, merge with new setup, stage, copy.
    let existing = load_config(dest.path()).unwrap();
    let new_setup: Vec<String> = vec!["./.superset/magic.sh sync".into(), "uv sync".into()];
    let merged = merge_setup_into_config(existing.as_ref(), new_setup);

    let stage = fresh();
    write_magic_sh(stage.path()).unwrap();
    write_config_json(stage.path(), &merged).unwrap();
    write_magic_json(stage.path(), &magic_cfg(&[])).unwrap();

    copy_into_repo(stage.path(), dest.path(), &[]).unwrap();

    let final_cfg = load_config(dest.path()).unwrap().unwrap();
    assert_eq!(final_cfg.setup, vec!["./.superset/magic.sh sync", "uv sync"]);
    assert_eq!(final_cfg.teardown, vec!["./drop.sh".to_string()]);
    assert!(final_cfg.run.is_empty());
}

#[test]
fn superset_as_file_returns_clear_error() {
    let dir = fresh();
    let root = dir.path();
    fs::write(root.join(".superset"), "not a dir").unwrap();
    let err = ensure_superset_dir(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("not a directory"), "msg: {msg}");
}

// ── MagicConfig / load_overlaid tests ────────────────────────────────────

fn magic_dir(root: &std::path::Path) {
    fs::create_dir_all(root.join(".superset")).unwrap();
}

fn write_magic_json_raw(root: &std::path::Path, body: &str) {
    magic_dir(root);
    fs::write(root.join(".superset/magic.json"), body).unwrap();
}

fn write_magic_local_raw(root: &std::path::Path, body: &str) {
    magic_dir(root);
    fs::write(root.join(".superset/magic.local.json"), body).unwrap();
}

/// AE7 — union of distinct patterns; magic.json order first.
#[test]
fn ae7_overlay_unions_and_dedupes_files() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":["**/.env"]}"#);
    write_magic_local_raw(root, r#"{"files":["**/.dev.vars"]}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(result.files, vec!["**/.env", "**/.dev.vars"]);
}

/// Local entry that repeats a base pattern appears only once (base position kept).
#[test]
fn overlay_dedupes_repeated_local_entry() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":["**/.env","**/.dev.vars"]}"#);
    write_magic_local_raw(root, r#"{"files":["**/.dev.vars","extra.txt"]}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    // **/.dev.vars must appear exactly once, in base position (index 1).
    assert_eq!(result.files, vec!["**/.env", "**/.dev.vars", "extra.txt"]);
}

/// magic.json present, magic.local.json absent → base only.
#[test]
fn overlay_base_only_when_local_absent() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":["**/.env",".dev.vars"]}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(result.files, vec!["**/.env", ".dev.vars"]);
}

/// magic.json absent → Ok(None).
#[test]
fn overlay_returns_none_when_base_absent() {
    let dir = fresh();
    let root = dir.path();
    // No magic.json, not even a .superset dir.
    let result = load_overlaid(root).unwrap();
    assert!(result.is_none());
}

/// Malformed magic.json → error naming the path.
#[test]
fn overlay_malformed_base_returns_error_with_path() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, "{not json");

    let err = load_overlaid(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("magic.json"), "msg: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

/// Malformed magic.local.json → error naming the path (no silent fallback).
#[test]
fn overlay_malformed_local_returns_error_with_path() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":["**/.env"]}"#);
    write_magic_local_raw(root, "{bad json");

    let err = load_overlaid(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("magic.local.json"), "msg: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

/// R6 — a non-`files` key absent from magic.local.json inherits the base
/// value unchanged (the merge loop only ever touches keys local actually
/// mentions).
#[test]
fn overlay_non_files_key_absent_in_local_inherits_base() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":[],"plugin":{"enabled":true}}"#);
    write_magic_local_raw(root, r#"{"files":[]}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(
        result.extras.get("plugin"),
        Some(&serde_json::json!({"enabled": true}))
    );
}

/// R6 — an explicit `null` in magic.local.json for a non-`files` key means
/// "off": it overrides the base value with `null` rather than being treated
/// as absent.
#[test]
fn overlay_non_files_key_explicit_null_in_local_means_off() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":[],"plugin":{"enabled":true}}"#);
    write_magic_local_raw(root, r#"{"files":[],"plugin":null}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(result.extras.get("plugin"), Some(&serde_json::Value::Null));
}

/// R6 — local's value replaces base's WHOLE; this is not a deep merge. A
/// local `plugin` object that omits a sub-key the base had does not carry
/// that sub-key forward — the whole base `plugin` value is discarded, not
/// merged key-by-key underneath it.
#[test]
fn overlay_non_files_key_local_value_replaces_base_whole_not_deep_merged() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(
        root,
        r#"{"files":[],"plugin":{"enabled":true,"gate":{"threshold_lines":5000}}}"#,
    );
    write_magic_local_raw(root, r#"{"files":[],"plugin":{"enabled":false}}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(
        result.extras.get("plugin"),
        Some(&serde_json::json!({"enabled": false})),
        "local's plugin value must replace base's whole, dropping base's `gate` \
         rather than inheriting it underneath local's `enabled`"
    );
}

/// write_magic_json produces pretty-printed JSON with a trailing newline
/// that round-trips through load_overlaid.
#[test]
fn write_magic_json_is_pretty_with_trailing_newline_and_round_trips() {
    let dir = fresh();
    let root = dir.path();
    let patterns = vec!["**/.env".to_string(), ".dev.vars".to_string()];
    write_magic_json(root, &magic_cfg(&["**/.env", ".dev.vars"])).unwrap();

    let raw = fs::read_to_string(root.join(".superset/magic.json")).unwrap();
    assert!(raw.contains('\n'), "expected pretty-printed JSON");
    assert!(raw.ends_with('\n'), "expected trailing newline");

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(result.files, patterns);
}

/// AE2 — a magic.json written by a newer ss-magic can carry top-level keys
/// this version doesn't know about (a `plugin` block, plus an arbitrary
/// future key). The load-modify-write pattern every write path now follows
/// (read the current file, change just `files` via
/// `merge_files_into_magic_config`, write back) must not drop them.
#[test]
fn ae2_write_magic_json_preserves_unknown_top_level_keys() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(
        root,
        r#"{"files":["**/.env"],"plugin":{"enabled":true,"name":"foo"},"future_key":"stays"}"#,
    );

    let existing = load_magic_json(root).unwrap();
    let mut new_files = existing.as_ref().map(|c| c.files.clone()).unwrap_or_default();
    new_files.push("**/.dev.vars".to_string());
    let updated = merge_files_into_magic_config(existing.as_ref(), new_files);
    write_magic_json(root, &updated).unwrap();

    let raw = fs::read_to_string(root.join(".superset/magic.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value["files"],
        serde_json::json!(["**/.env", "**/.dev.vars"]),
        "files must be updated"
    );
    assert_eq!(
        value.get("plugin"),
        Some(&serde_json::json!({"enabled": true, "name": "foo"})),
        "plugin block must survive the rewrite"
    );
    assert_eq!(
        value.get("future_key"),
        Some(&serde_json::json!("stays")),
        "unrecognized future key must survive the rewrite"
    );
}

/// An empty `extras` map (the common case — no unknown keys at all) produces
/// exactly today's shape: only `files`, nothing else.
#[test]
fn write_magic_json_with_empty_extras_matches_files_only_shape() {
    let dir = fresh();
    let root = dir.path();
    let cfg = MagicConfig {
        files: vec!["**/.env".to_string()],
        extras: serde_json::Map::new(),
    };
    write_magic_json(root, &cfg).unwrap();

    let raw = fs::read_to_string(root.join(".superset/magic.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"files": ["**/.env"]}),
        "no extras must mean no extra keys in the output"
    );
}

/// Two successive load-modify-write round trips through the same unknown
/// keys keep those keys' values unchanged (order may be normalized by the
/// underlying map, but content and repeatability must hold).
#[test]
fn extras_survive_two_successive_round_trips() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(
        root,
        r#"{"files":["**/.env"],"zeta":"z","alpha":"a"}"#,
    );

    for next_pattern in ["**/.dev.vars", ".env.local"] {
        let existing = load_magic_json(root).unwrap();
        let mut files = existing.as_ref().map(|c| c.files.clone()).unwrap_or_default();
        files.push(next_pattern.to_string());
        let updated = merge_files_into_magic_config(existing.as_ref(), files);
        write_magic_json(root, &updated).unwrap();
    }

    let raw = fs::read_to_string(root.join(".superset/magic.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value["files"],
        serde_json::json!(["**/.env", "**/.dev.vars", ".env.local"])
    );
    assert_eq!(value["zeta"], serde_json::json!("z"));
    assert_eq!(value["alpha"], serde_json::json!("a"));
}

/// `merge_files_into_magic_config` with `existing: None` (no prior file, the
/// first-ever init) starts from an empty extras map — nothing to preserve,
/// nothing fabricated.
#[test]
fn merge_files_into_magic_config_with_none_yields_empty_extras() {
    let merged = merge_files_into_magic_config(None, vec!["**/.env".to_string()]);
    assert_eq!(merged.files, vec!["**/.env".to_string()]);
    assert!(merged.extras.is_empty());
}

/// A malformed magic.json is still a hard error when loaded for the
/// load-modify-write path — extras preservation must not paper over a
/// genuinely broken file.
#[test]
fn load_magic_json_malformed_returns_clean_error_before_merge() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, "{not json");

    let err = load_magic_json(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("magic.json"), "msg: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

/// `load_magic_local_json` mirrors `load_magic_json` exactly, but reads
/// `magic.local.json` and does NOT merge it with the base file — that is
/// `load_overlaid`'s job, not this one's.
#[test]
fn load_magic_local_json_reads_only_the_local_file_unmerged() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json(root, &magic_cfg(&["**/.env"])).unwrap();
    write_magic_local_json(root, &magic_cfg(&["**/.dev.vars"])).unwrap();

    let local = load_magic_local_json(root).unwrap().unwrap();
    assert_eq!(
        local.files,
        vec!["**/.dev.vars".to_string()],
        "must read the local file's own files, not the base's, and not the union"
    );
}

/// A missing `magic.local.json` is `Ok(None)`, the same as a missing
/// `magic.json` — not an error, even when the base file is present.
#[test]
fn load_magic_local_json_absent_is_none() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json(root, &magic_cfg(&["**/.env"])).unwrap();

    assert!(load_magic_local_json(root).unwrap().is_none());
}

/// `write_magic_local_json` produces pretty-printed JSON with a trailing
/// newline, round-tripping through `load_magic_local_json` — the same shape
/// `write_magic_json` produces for the base file.
#[test]
fn write_magic_local_json_is_pretty_with_trailing_newline_and_round_trips() {
    let dir = fresh();
    let root = dir.path();
    write_magic_local_json(root, &magic_cfg(&["**/.dev.vars"])).unwrap();

    let raw = fs::read_to_string(root.join(".superset/magic.local.json")).unwrap();
    assert!(raw.contains('\n'), "expected pretty-printed JSON");
    assert!(raw.ends_with('\n'), "expected trailing newline");

    let result = load_magic_local_json(root).unwrap().unwrap();
    assert_eq!(result.files, vec!["**/.dev.vars".to_string()]);
}

/// `write_magic_local_json` round-trips `extras` the same way the base
/// writer does — it is a plain writer with no preservation of its own, so a
/// caller supplying `extras` gets them back unchanged.
#[test]
fn write_magic_local_json_round_trips_extras() {
    let dir = fresh();
    let root = dir.path();
    let mut extras = serde_json::Map::new();
    extras.insert("plugin".to_string(), serde_json::json!({"enabled": true}));
    let cfg = MagicConfig {
        files: vec![],
        extras,
    };
    write_magic_local_json(root, &cfg).unwrap();

    let local = load_magic_local_json(root).unwrap().unwrap();
    assert_eq!(local.extras["plugin"]["enabled"], serde_json::json!(true));
}

/// A malformed `magic.local.json` is a hard error, same as the base file —
/// no silent fallback to "as if absent".
#[test]
fn load_magic_local_json_malformed_returns_clean_error() {
    let dir = fresh();
    let root = dir.path();
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(root.join(".superset/magic.local.json"), "{not json").unwrap();

    let err = load_magic_local_json(root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("magic.local.json"), "msg: {msg}");
    assert!(msg.contains("malformed JSON"), "msg: {msg}");
}

/// empty magic.json files array + non-empty local → local entries appended.
#[test]
fn overlay_empty_base_files_plus_local() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{"files":[]}"#);
    write_magic_local_raw(root, r#"{"files":["secrets/**"]}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(result.files, vec!["secrets/**"]);
}

/// Both magic.json and magic.local.json have no files key (serde default).
#[test]
fn overlay_missing_files_key_defaults_to_empty() {
    let dir = fresh();
    let root = dir.path();
    write_magic_json_raw(root, r#"{}"#);
    write_magic_local_raw(root, r#"{}"#);

    let result = load_overlaid(root).unwrap().unwrap();
    assert!(result.files.is_empty());
}

// ── bootstrap_magic_local_json / default_magic_files tests ───────────────

/// Bootstrapped magic.local.json parses as {} (+ comment key) and overlays
/// as empty files (the _comment key is ignored by serde).
#[test]
fn bootstrap_magic_local_json_creates_valid_overlay_noop() {
    let dir = fresh();
    let root = dir.path();

    // Need a magic.json so load_overlaid can return Some(_).
    write_magic_json_raw(root, r#"{"files":["**/.env"]}"#);

    bootstrap_magic_local_json(root).unwrap();

    let path = root.join(".superset/magic.local.json");
    assert!(path.is_file(), "magic.local.json must be created");

    // Must be valid JSON.
    let raw = fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .expect("bootstrapped magic.local.json must be valid JSON");
    assert!(parsed.is_object(), "must be a JSON object");
    assert!(parsed.get("_comment").is_some(), "must contain _comment key");

    // load_overlaid must round-trip: local contributes zero extra files.
    let result = load_overlaid(root).unwrap().unwrap();
    assert_eq!(
        result.files,
        vec!["**/.env"],
        "local overlay must add no files beyond the base"
    );
}

/// bootstrap_magic_local_json is idempotent: existing file is not overwritten.
#[test]
fn bootstrap_magic_local_json_idempotent_when_file_exists() {
    let dir = fresh();
    let root = dir.path();
    let path = root.join(".superset/magic.local.json");

    // Write a custom file first.
    fs::create_dir_all(root.join(".superset")).unwrap();
    let custom = r#"{"files":["custom/**"]}"#;
    fs::write(&path, custom).unwrap();

    bootstrap_magic_local_json(root).unwrap();

    // Must be unchanged.
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, custom, "existing file must not be overwritten");
}

/// Bootstrapped file has a trailing newline (consistent with the write convention).
#[test]
fn bootstrap_magic_local_json_has_trailing_newline() {
    let dir = fresh();
    let root = dir.path();

    bootstrap_magic_local_json(root).unwrap();

    let raw = fs::read_to_string(root.join(".superset/magic.local.json")).unwrap();
    assert!(raw.ends_with('\n'), "must end with a trailing newline");
}

/// default_magic_files includes .superset/magic.local.json.
#[test]
fn default_magic_files_includes_magic_local_json() {
    let defaults = default_magic_files();
    assert!(
        defaults.iter().any(|s| s == ".superset/magic.local.json"),
        "default_magic_files() must include .superset/magic.local.json; got: {defaults:?}"
    );
}

// ── write_magic_sh / magic.sh asset tests ────────────────────────────────

/// write_magic_sh emits a file byte-equal to the embedded MAGIC_SH asset
/// and marks it executable (mode 0755) on Unix.
#[test]
fn write_magic_sh_emits_executable_file_matching_embedded_asset() {
    let dir = fresh();
    let root = dir.path();
    write_magic_sh(root).unwrap();

    let path = root.join(".superset/magic.sh");
    assert!(path.is_file(), "magic.sh must be created");

    let on_disk = fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, MAGIC_SH, "on-disk content must match embedded asset");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "magic.sh must be mode 0755");
    }
}

/// Find bash via the host environment, bypassing any controlled PATH we set
/// on child processes.  Returns the absolute path to bash, panicking if it
/// cannot be located — the tests require bash.
fn find_bash() -> std::path::PathBuf {
    // Try common locations so the test works regardless of the PATH value
    // we override on child processes.
    for candidate in &[
        "/opt/homebrew/bin/bash",
        "/usr/local/bin/bash",
        "/usr/bin/bash",
        "/bin/bash",
    ] {
        let p = std::path::Path::new(candidate);
        if p.exists() {
            return p.to_path_buf();
        }
    }
    // Fall back to whatever the host PATH exposes at test-compilation time.
    panic!("bash not found; tests require bash");
}

/// Covers AE8: running magic.sh with ss-magic absent from PATH prints an
/// install error to stderr and exits 0 (pipeline must not be interrupted).
#[test]
fn ae8_magic_sh_absent_binary_prints_error_and_exits_zero() {
    let dir = fresh();
    let root = dir.path();
    write_magic_sh(root).unwrap();
    let script = root.join(".superset/magic.sh");

    // Use an empty temp dir as PATH so ss-magic is guaranteed absent.
    let empty_path_dir = tempfile::tempdir().unwrap();

    let output = std::process::Command::new(find_bash())
        .arg(&script)
        .env("PATH", empty_path_dir.path())
        // Ensure NO_COLOR is unset so the color branch is exercised (stderr
        // may or may not be a TTY in CI — we only verify the text content).
        .env_remove("NO_COLOR")
        .output()
        .expect("failed to run magic.sh via bash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "magic.sh must exit 0 when ss-magic is absent; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ss-magic is not installed"),
        "stderr must mention 'ss-magic is not installed'; got: {stderr}"
    );
    assert!(
        stderr.contains("ViktorStiskala/superset-magic"),
        "stderr must reference the install repo; got: {stderr}"
    );
}

/// Exit-code propagation via exec: a stub ss-magic that exits 3 must cause
/// magic.sh to exit 3 as well.
#[test]
fn magic_sh_propagates_exit_code_from_ss_magic_via_exec() {
    let dir = fresh();
    let root = dir.path();
    write_magic_sh(root).unwrap();
    let script = root.join(".superset/magic.sh");

    // Create a stub ss-magic in a temp dir that always exits 3.
    let stub_dir = tempfile::tempdir().unwrap();
    let stub_path = stub_dir.path().join("ss-magic");
    fs::write(&stub_path, "#!/bin/sh\nexit 3\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub_path, perms).unwrap();
    }

    // Prepend the stub dir to a minimal PATH so ss-magic resolves to our stub.
    let path_val = format!("{}:/usr/bin:/bin", stub_dir.path().display());

    let status = std::process::Command::new(find_bash())
        .arg(&script)
        .env("PATH", &path_val)
        .status()
        .expect("failed to run magic.sh via bash");

    assert_eq!(
        status.code(),
        Some(3),
        "magic.sh must propagate ss-magic's exit code (3) via exec"
    );
}

// ── local install: config.local.json, load_sync_config, install_mode ─────

use serde_json::{json, Value};

/// Parse a JSON literal into the top-level map `merge_sync_entry_into_local_config`
/// takes, so a fixture reads as the file's literal text.
fn obj(v: Value) -> serde_json::Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("fixture is not an object: {other}"),
    }
}

fn merged(existing: Option<Value>) -> LocalConfigMerge {
    let existing = existing.map(obj);
    merge_sync_entry_into_local_config(existing.as_ref()).unwrap()
}

#[test]
fn local_merge_absent_file_gets_setup_before_entry() {
    let m = merged(None);
    assert!(m.changed);
    assert_eq!(Value::Object(m.config), json!({"setup": {"before": ["ss-magic sync"]}}));
}

#[test]
fn local_merge_file_without_setup_key_keeps_other_keys() {
    let m = merged(Some(json!({"note": 1})));
    assert!(m.changed);
    assert_eq!(
        Value::Object(m.config),
        json!({"note": 1, "setup": {"before": ["ss-magic sync"]}})
    );
}

#[test]
fn local_merge_object_setup_with_only_after_gains_before() {
    let m = merged(Some(json!({"setup": {"after": ["./post.sh"]}})));
    assert!(m.changed);
    assert_eq!(
        Value::Object(m.config),
        json!({"setup": {"before": ["ss-magic sync"], "after": ["./post.sh"]}})
    );
}

#[test]
fn local_merge_object_setup_with_before_prepends_entry() {
    let m = merged(Some(json!({"setup": {"before": ["./pre.sh"], "after": ["./post.sh"]}})));
    assert!(m.changed);
    assert_eq!(
        Value::Object(m.config),
        json!({"setup": {"before": ["ss-magic sync", "./pre.sh"], "after": ["./post.sh"]}})
    );
}

/// AE3: the replace form (`setup` is a plain array) keeps that form and
/// prepends, and every other key survives with its value.
#[test]
fn local_merge_array_setup_is_prepended_and_other_keys_survive() {
    let m = merged(Some(
        json!({"setup": ["./mine.sh"], "teardown": {"after": ["x"]}, "note": 1}),
    ));
    assert!(m.changed);
    assert_eq!(
        Value::Object(m.config),
        json!({"setup": ["ss-magic sync", "./mine.sh"], "teardown": {"after": ["x"]}, "note": 1})
    );
}

#[test]
fn local_merge_reports_no_change_when_marker_already_present() {
    for existing in [
        json!({"setup": {"before": ["ss-magic sync", "./a.sh"]}}),
        json!({"setup": {"after": ["ss-magic sync"]}}),
        json!({"setup": ["./a.sh", "ss-magic sync"]}),
        // A flagged invocation is still the marker, matching the committed
        // install's `entry_is_magic_marker` test (a substring match).
        json!({"setup": ["ss-magic sync --no-backup"]}),
    ] {
        let m = merged(Some(existing.clone()));
        assert!(!m.changed, "{existing} must read as already registered");
        assert_eq!(Value::Object(m.config), existing);
    }
}

#[test]
fn local_merge_refuses_setup_of_another_json_type() {
    for bad in [json!("./one.sh"), json!(3), json!(null), json!(true)] {
        let existing = obj(json!({"setup": bad}));
        let err = merge_sync_entry_into_local_config(Some(&existing)).unwrap_err();
        assert!(
            format!("{err:#}").contains("setup"),
            "error must name the setup key: {err:#}"
        );
    }
}

#[test]
fn local_merge_refuses_a_non_array_before() {
    let existing = obj(json!({"setup": {"before": "./one.sh"}}));
    assert!(merge_sync_entry_into_local_config(Some(&existing)).is_err());
}

#[test]
fn local_merge_refuses_a_non_array_after() {
    let existing = obj(json!({"setup": {"after": "./one.sh"}}));
    let err = merge_sync_entry_into_local_config(Some(&existing)).unwrap_err();
    assert!(format!("{err:#}").contains("setup.after"), "{err:#}");
}

/// The shape is validated BEFORE the marker shortcut: a value that contains the
/// command in a place Superset would not run it as a setup step must be
/// refused, not reported as already registered.
#[test]
fn local_merge_refuses_an_unsupported_shape_even_when_it_carries_the_marker() {
    for bad in [
        json!("ss-magic sync"),
        json!({"before": "./one.sh", "after": ["ss-magic sync"]}),
        json!({"before": ["ss-magic sync"], "after": 3}),
    ] {
        let existing = obj(json!({"setup": bad}));
        assert!(
            merge_sync_entry_into_local_config(Some(&existing)).is_err(),
            "{bad} must be refused"
        );
    }
}

#[test]
fn classify_local_setup_covers_every_shape() {
    use LocalSetup::*;
    let c = |v: Value| classify_local_setup(&v);
    assert_eq!(c(json!(["a", "ss-magic sync"])), Replace { has_marker: true });
    assert_eq!(c(json!(["./magic.sh sync"])), Replace { has_marker: false });
    assert_eq!(c(json!([])), Replace { has_marker: false });
    assert_eq!(c(json!({"before": ["ss-magic sync"]})), Wrap { has_marker: true });
    assert_eq!(c(json!({"after": ["ss-magic sync"]})), Wrap { has_marker: true });
    assert_eq!(c(json!({})), Wrap { has_marker: false });
    assert_eq!(
        c(json!({"before": ["ss-magic-plugin status"]})),
        Wrap { has_marker: false }
    );
    // Only `before` / `after` are commands: a marker under another key or
    // nested deeper than a string item does not count.
    assert_eq!(c(json!({"note": "ss-magic sync"})), Wrap { has_marker: false });
    assert_eq!(c(json!([["ss-magic sync"]])), Replace { has_marker: false });
    for bad in [
        json!("ss-magic sync"),
        json!(null),
        json!(7),
        json!(true),
        json!({"before": "x"}),
        json!({"after": {"a": 1}}),
    ] {
        assert!(matches!(c(bad.clone()), Unsupported(_)), "{bad}");
    }
}

#[test]
fn rel_eq_ignore_ascii_case_compares_components() {
    let eq = |a: &str, b: &str| rel_eq_ignore_ascii_case(Path::new(a), Path::new(b));
    assert!(eq(".superset/config.local.json", ".superset/config.local.json"));
    assert!(eq(".Superset/CONFIG.LOCAL.JSON", ".superset/config.local.json"));
    assert!(eq("./.superset/config.local.json", ".superset/config.local.json"));
    assert!(eq(".superset/./config.local.json", ".superset/config.local.json"));
    assert!(!eq("apps/.superset/config.local.json", ".superset/config.local.json"));
    assert!(!eq(".superset/config.local.json.bak", ".superset/config.local.json"));
    assert!(!eq(".superset", ".superset/config.local.json"));
}

#[test]
fn load_config_local_json_absent_is_none() {
    let dir = fresh();
    assert!(load_config_local_json(dir.path()).unwrap().is_none());
}

#[test]
fn load_config_local_json_malformed_names_the_path() {
    let dir = fresh();
    fs::create_dir_all(dir.path().join(".superset")).unwrap();
    fs::write(dir.path().join(".superset/config.local.json"), "{ not json").unwrap();
    let err = load_config_local_json(dir.path()).unwrap_err();
    assert!(
        format!("{err:#}").contains("config.local.json"),
        "error must name the file: {err:#}"
    );
}

#[test]
fn load_config_local_json_rejects_a_non_object_document() {
    let dir = fresh();
    fs::create_dir_all(dir.path().join(".superset")).unwrap();
    fs::write(dir.path().join(".superset/config.local.json"), "[1]").unwrap();
    assert!(load_config_local_json(dir.path()).is_err());
}

#[test]
fn write_config_local_json_round_trips_values_and_leaves_no_staging_file() {
    let dir = fresh();
    let root = dir.path();
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(
        root.join(".superset/config.local.json"),
        r#"{"setup": ["./mine.sh"], "teardown": {"after": ["x"]}, "note": 1}"#,
    )
    .unwrap();
    let before = load_config_local_json(root).unwrap();
    let m = merge_sync_entry_into_local_config(before.as_ref()).unwrap();
    write_config_local_json(root, &m.config).unwrap();

    let after = load_config_local_json(root).unwrap().unwrap();
    assert_eq!(
        Value::Object(after),
        json!({"setup": ["ss-magic sync", "./mine.sh"], "teardown": {"after": ["x"]}, "note": 1})
    );
    let raw = fs::read_to_string(root.join(".superset/config.local.json")).unwrap();
    assert!(raw.ends_with('\n'), "pretty-printed with a trailing newline");
    let names: Vec<String> = fs::read_dir(root.join(".superset"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["config.local.json".to_string()]);
}

#[test]
fn write_config_local_json_creates_the_superset_dir() {
    let dir = fresh();
    write_config_local_json(dir.path(), &obj(json!({"setup": []}))).unwrap();
    assert!(dir.path().join(".superset/config.local.json").is_file());
}

#[test]
fn local_install_default_files_are_the_two_local_files() {
    assert_eq!(
        local_install_default_files(),
        vec![
            ".superset/magic.local.json".to_string(),
            ".superset/config.local.json".to_string()
        ]
    );
}

fn write_superset_file(root: &Path, name: &str, body: &str) {
    fs::create_dir_all(root.join(".superset")).unwrap();
    fs::write(root.join(".superset").join(name), body).unwrap();
}

#[test]
fn install_mode_is_derived_from_files() {
    let dir = fresh();
    let root = dir.path();
    assert_eq!(install_mode(root), InstallMode::None);

    // config.json alone (a team's own setup) is not an ss-magic install.
    write_superset_file(root, "config.json", r#"{"setup": []}"#);
    assert_eq!(install_mode(root), InstallMode::None);

    // config.local.json alone does not decide the mode either, marker or not.
    write_superset_file(root, "config.local.json", r#"{"setup": ["ss-magic sync"]}"#);
    assert_eq!(install_mode(root), InstallMode::None);

    write_superset_file(root, "magic.local.json", r#"{"files": [".env"]}"#);
    assert_eq!(install_mode(root), InstallMode::Local);

    // A committed magic.json wins over a coexisting magic.local.json.
    write_superset_file(root, "magic.json", r#"{"files": []}"#);
    assert_eq!(install_mode(root), InstallMode::Committed);
}

#[test]
fn install_mode_committed_without_a_local_overlay() {
    let dir = fresh();
    write_superset_file(dir.path(), "magic.json", r#"{"files": []}"#);
    assert_eq!(install_mode(dir.path()), InstallMode::Committed);
}

#[test]
fn load_sync_config_returns_the_overlay_when_magic_json_exists() {
    let dir = fresh();
    let root = dir.path();
    write_superset_file(root, "magic.json", r#"{"files": [".env"]}"#);
    write_superset_file(root, "magic.local.json", r#"{"files": [".env", "x.key"]}"#);
    let cfg = load_sync_config(root).unwrap().unwrap();
    assert_eq!(cfg.files, vec![".env", "x.key"]);
    // Identical to what load_overlaid answers: the plugin's loader is untouched.
    assert_eq!(cfg.files, load_overlaid(root).unwrap().unwrap().files);
}

#[test]
fn load_sync_config_uses_the_local_list_alone_without_magic_json() {
    let dir = fresh();
    let root = dir.path();
    write_superset_file(
        root,
        "magic.local.json",
        r#"{"_comment": "kept", "files": [".superset/magic.local.json", ".env"]}"#,
    );
    let cfg = load_sync_config(root).unwrap().unwrap();
    assert_eq!(cfg.files, vec![".superset/magic.local.json", ".env"]);
    assert_eq!(cfg.extras.get("_comment"), Some(&json!("kept")));
    // load_overlaid still answers None here: its behavior must not change.
    assert!(load_overlaid(root).unwrap().is_none());
}

#[test]
fn load_sync_config_is_none_when_neither_file_exists() {
    let dir = fresh();
    write_superset_file(dir.path(), "config.json", r#"{"setup": []}"#);
    assert!(load_sync_config(dir.path()).unwrap().is_none());
}

#[test]
fn load_sync_config_malformed_local_list_is_an_error_naming_the_path() {
    let dir = fresh();
    write_superset_file(dir.path(), "magic.local.json", "{ nope");
    let err = load_sync_config(dir.path()).unwrap_err();
    assert!(format!("{err:#}").contains("magic.local.json"), "{err:#}");
}
