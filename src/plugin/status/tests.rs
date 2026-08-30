//! Status tests.
//!
//! Two things these assert that ordinary "does it work" tests would not:
//!
//! - **Nothing is ever blank.** The verb exists so somebody can find out why
//!   nothing is happening, and a row rendered blank reads as "fine". So the
//!   tests below check for the *reason* on every undeterminable row, not just
//!   that the field is `None`.
//! - **Nothing is created.** A diagnostic that scaffolds the tree it is
//!   diagnosing cannot report what was actually wrong, so a collect over a
//!   repository leaves it byte-identical.
//!
//! Every external fact — the harness listing, the bootstrapped binary's
//! version, the plugin root, the data directory, the heartbeat store — is a
//! parameter, so no test depends on the developer's own `~/.claude`, installed
//! plugins, or application-data directory.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

use crate::plugin::heartbeat::{self, Outcome, Row};
use crate::tests::support::{git_run, neutralize_global_excludes};

use super::*;

/// A fixed instant, so timestamps in assertions are stable: 2026-08-30
/// 12:00:00 UTC.
const NOW: u64 = 1_788_091_200;

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// A git repository on `branch` with one commit, and — when `enabled` — a
/// `.superset/magic.json` turning the plugin on.
fn repo(branch: &str, enabled: bool) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", branch], dir.path());
    neutralize_global_excludes(dir.path());
    fs::write(dir.path().join("README.md"), "hi\n").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-qm", "initial"], dir.path());

    fs::create_dir_all(dir.path().join(".superset")).unwrap();
    let body = format!("{{\n  \"files\": [],\n  \"plugin\": {{ \"enabled\": {enabled} }}\n}}\n");
    fs::write(dir.path().join(".superset").join("magic.json"), body).unwrap();
    dir
}

/// Add the `.superset/.magic/` ignore rule the state-writing hooks require.
fn ignore_state_tree(root: &Path) {
    fs::write(root.join(".gitignore"), ".superset/.magic/\n").unwrap();
}

/// Inputs with every external fact undetermined — the shape a bare Bash
/// context, with no `${CLAUDE_PLUGIN_ROOT}` and no `${CLAUDE_PLUGIN_DATA}`,
/// actually sees.
fn bare_inputs(cwd: &Path) -> Inputs {
    Inputs {
        cwd: cwd.to_path_buf(),
        tool_version: "0.10.0".to_string(),
        store: None,
        plugin_root: Located::missing("no ${CLAUDE_PLUGIN_ROOT} in this test"),
        data_dir: Located::missing("no ${CLAUDE_PLUGIN_DATA} in this test"),
        all: false,
    }
}

/// A binary-version probe that reports the binary could not be run.
fn no_version(_bin: &Path) -> std::result::Result<String, String> {
    Err("this test does not run it".to_string())
}

/// A binary-version probe that always answers `0.9.0`.
fn version_090(_bin: &Path) -> std::result::Result<String, String> {
    Ok("0.9.0".to_string())
}

/// A binary-version probe that always answers `0.10.0`.
fn version_0100(_bin: &Path) -> std::result::Result<String, String> {
    Ok("0.10.0".to_string())
}

fn probes(harness: HarnessListing) -> Probes<'static> {
    Probes {
        harness,
        binary_version: &no_version,
    }
}

/// The harness listing for one enabled marketplace registration.
fn listing(enabled: bool) -> HarnessListing {
    HarnessListing::Loaded(vec![Registration {
        id: "ss-magic@ss-magic".to_string(),
        name: "ss-magic".to_string(),
        scope: Some("user".to_string()),
        enabled: Some(enabled),
        version: Some("0.10.0".to_string()),
        install_path: Some("/plugins/ss-magic".to_string()),
        project_path: None,
        errors: Vec::new(),
        notes: Vec::new(),
    }])
}

/// Serialize a report and hand back the JSON an agent would parse.
fn json(status: &Status) -> Value {
    serde_json::to_value(status).unwrap()
}

/// Every working-tree path under `dir`, sorted — for asserting nothing was
/// created.
///
/// `.git/` is skipped: the ignore probe runs `git check-ignore`, which
/// refreshes the index behind a short-lived `index.lock`, and catching that
/// lock mid-write would fail the test for a reason that has nothing to do with
/// what it is asserting. What matters here is the working tree.
fn tree(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != std::ffi::OsStr::new(".git"))
        .filter_map(Result::ok)
        .map(|e| e.path().display().to_string())
        .collect();
    out.sort();
    out
}

/// A heartbeat store in a tempdir, with `rows` already appended.
fn store_with(rows: Vec<Row>) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("plugin");
    for row in &rows {
        heartbeat::append(&store, row).unwrap();
    }
    (dir, store)
}

// ── AE20: discoverable from a bare Bash context ───────────────────────────────

/// A dispatched agent receives no `SessionStart` injection, so `status --json`
/// is the only way it learns where the plugin keeps things. The three facts it
/// needs — the slug, the conclusions directory, the gate threshold — must all
/// be there with no injected state at all.
#[test]
fn json_carries_the_slug_directories_and_thresholds_with_no_injected_state() {
    let dir = repo("feature-x", true);
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Unavailable("no claude here".to_string())),
    );
    let value = json(&status);

    let slug = value["identity"]["slug"].as_str().expect("a slug");
    assert!(slug.ends_with("-feature-x"), "unexpected slug: {slug}");

    let conclusions = value["state_tree"]["directories"]["conclusions"]
        .as_str()
        .expect("a conclusions directory");
    assert!(
        conclusions.ends_with(".superset/.magic/conclusions"),
        "unexpected conclusions dir: {conclusions}"
    );

    assert_eq!(
        value["gate"]["threshold_lines"].as_u64(),
        Some(u64::from(config::GATE_THRESHOLD_LINES_DEFAULT))
    );
    assert_eq!(value["schema"].as_u64(), Some(u64::from(SCHEMA_VERSION)));
}

/// The session directory is named, not just the `sessions/` parent — that is
/// the path an agent writes its working memory into.
#[test]
fn json_names_the_current_session_directory() {
    let dir = repo("feature-x", true);
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Unavailable("no claude here".to_string())),
    );

    let session = status
        .state_tree
        .directories
        .session
        .expect("a session dir");
    let slug = status.identity.slug.expect("a slug");
    assert!(session.ends_with(&format!("sessions/{slug}")), "{session}");
}

// ── AE52: the two enablement layers ───────────────────────────────────────────

/// The harness-side registration disabled while `plugin.enabled` is true. Both
/// layers are reported and the disabled one is identifiable — otherwise the
/// only visible symptom is that nothing happens.
#[test]
fn a_disabled_harness_registration_is_identifiable_beside_an_enabled_overlay() {
    let dir = repo("main", true);
    let status = collect(&bare_inputs(dir.path()), &probes(listing(false)));
    let value = json(&status);

    assert_eq!(
        value["enablement"]["ss_magic"]["enabled"],
        Value::Bool(true)
    );
    assert_eq!(
        value["enablement"]["harness"]["enabled"],
        Value::Bool(false)
    );
    assert_eq!(value["enablement"]["acting"], Value::Bool(false));

    let reg = &value["enablement"]["harness"]["registrations"][0];
    assert_eq!(reg["id"], Value::String("ss-magic@ss-magic".to_string()));
    assert_eq!(reg["scope"], Value::String("user".to_string()));
    assert_eq!(reg["enabled"], Value::Bool(false));

    assert!(
        status
            .problems
            .iter()
            .any(|p| p.contains("harness-side registration is disabled")),
        "problems did not name the disabled layer: {:?}",
        status.problems
    );
}

/// The mirror: ss-magic's own switch off while the harness has the plugin
/// loaded and enabled.
#[test]
fn a_false_plugin_enabled_is_named_even_when_the_harness_is_happy() {
    let dir = repo("main", false);
    let status = collect(&bare_inputs(dir.path()), &probes(listing(true)));

    assert!(!status.enablement.ss_magic.enabled);
    assert_eq!(status.enablement.harness.enabled, Some(true));
    assert_eq!(status.enablement.acting, Some(false));
    assert!(
        status
            .problems
            .iter()
            .any(|p| p.contains("`plugin.enabled` is not true")),
        "problems did not name the overlay: {:?}",
        status.problems
    );
}

/// Two registrations can carry the same manifest name — a marketplace install
/// and a shadowed skills-directory copy. Both are listed, because which one the
/// harness actually loaded is exactly the question being asked.
#[test]
fn two_registrations_named_ss_magic_are_both_reported() {
    let dir = repo("main", true);
    let text = r#"[
      {"id": "ss-magic@ss-magic", "scope": "user", "enabled": true, "version": "0.10.0"},
      {"id": "ss-magic@skills-dir", "scope": "project", "enabled": false,
       "notes": ["skipped because this workspace was not trusted"]}
    ]"#;
    let regs = parse_listing(text).unwrap();
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Loaded(regs)),
    );

    assert_eq!(status.enablement.harness.registrations.len(), 2);
    // One of the two is enabled, so the harness layer is on — and the note
    // from the other is still surfaced verbatim.
    assert_eq!(status.enablement.harness.enabled, Some(true));
    assert!(
        status
            .problems
            .iter()
            .any(|p| p.contains("was not trusted")),
        "the harness's own note was not surfaced: {:?}",
        status.problems
    );
}

/// A registration's `errors[]` and `notes[]` are the harness's words about its
/// own state. They go through unchanged rather than being interpreted.
#[test]
fn harness_errors_and_notes_are_surfaced_verbatim() {
    let dir = repo("main", true);
    let regs = parse_listing(
        r#"[{"id": "ss-magic@ss-magic", "enabled": true,
             "errors": ["manifest is not valid JSON"],
             "notes": ["1 project-scope directory was skipped"]}]"#,
    )
    .unwrap();
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Loaded(regs)),
    );

    assert!(status
        .problems
        .iter()
        .any(|p| p.contains("manifest is not valid JSON")));
    assert!(status
        .problems
        .iter()
        .any(|p| p.contains("1 project-scope directory was skipped")));
}

/// No `ss-magic` registration at all is a different answer from "disabled",
/// and it says so.
#[test]
fn a_missing_registration_is_reported_as_not_installed() {
    let dir = repo("main", true);
    let regs = parse_listing(r#"[{"id": "other@market", "enabled": true}]"#).unwrap();
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Loaded(regs)),
    );

    assert!(status.enablement.harness.registrations.is_empty());
    let note = status
        .enablement
        .harness
        .note
        .expect("an empty registration list must carry a reason");
    assert!(note.contains("no plugin named"), "{note}");
}

// ── The harness subprocess is optional ────────────────────────────────────────

/// No `claude` on `PATH` is an ordinary result: the report is produced, the
/// harness layer is unknown rather than false, and `acting` refuses to guess.
#[test]
fn an_absent_claude_cli_leaves_the_harness_layer_unknown_not_false() {
    let dir = repo("main", true);
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Unavailable(
            "`claude plugin list --json` could not be run: it is not on PATH".to_string(),
        )),
    );

    assert_eq!(status.enablement.harness.enabled, None);
    assert_eq!(status.enablement.acting, None);
    let note = status.enablement.harness.note.expect("a reason");
    assert!(note.contains("not on PATH"), "{note}");
    assert!(status
        .problems
        .iter()
        .any(|p| p.contains("could not be determined")));
}

/// Malformed output is reported with what was actually seen, so the reader can
/// tell a wrong-CLI problem from a broken-install one.
#[test]
fn malformed_harness_json_is_a_note_not_a_failure() {
    let err = parse_listing("Error: something went wrong\n").unwrap_err();
    assert!(err.contains("did not return readable JSON"), "{err}");
    assert!(err.contains("Error: something went wrong"), "{err}");
}

/// Empty output is its own case — there is nothing to quote back.
#[test]
fn empty_harness_output_is_a_note() {
    let err = parse_listing("   \n").unwrap_err();
    assert!(err.contains("produced no output"), "{err}");
}

/// The listing is a bare array, but an object with a `plugins` key appears in
/// some documentation. Accepting both costs one enum and avoids reporting a
/// working harness as unreadable.
#[test]
fn both_listing_shapes_parse() {
    let array = parse_listing(r#"[{"id": "a@b", "enabled": true}]"#).unwrap();
    let wrapped = parse_listing(r#"{"plugins": [{"id": "a@b", "enabled": true}]}"#).unwrap();
    assert_eq!(array.len(), 1);
    assert_eq!(wrapped.len(), 1);
    assert_eq!(array[0].id, wrapped[0].id);
}

/// The match is on the manifest name, which is the id's prefix — so a
/// marketplace install and a skills-directory copy are both `ss-magic`.
#[test]
fn the_manifest_name_comes_from_the_id_prefix() {
    let regs =
        parse_listing(r#"[{"id": "ss-magic@skills-dir"}, {"id": "compound-engineering@ce"}]"#)
            .unwrap();
    assert_eq!(regs[0].name, "ss-magic");
    assert_eq!(regs[1].name, "compound-engineering");
}

/// Duplicate ids appear once per project path, so the pair is the identity.
#[test]
fn duplicate_registrations_are_deduped_by_id_and_project_path() {
    let regs = parse_listing(
        r#"[{"id": "a@b", "projectPath": "/one"},
            {"id": "a@b", "projectPath": "/one"},
            {"id": "a@b", "projectPath": "/two"}]"#,
    )
    .unwrap();
    assert_eq!(regs.len(), 2);
}

// ── R63: the ignored-tree gate ────────────────────────────────────────────────

/// A state tree git does not ignore silences every state-writing hook, which is
/// the second-most confusing way for the plugin to do nothing. It is a row and
/// a problem, not a footnote.
#[test]
fn an_unignored_state_tree_is_reported_as_a_blocker() {
    let dir = repo("main", true);
    let status = collect(&bare_inputs(dir.path()), &probes(listing(true)));

    assert_eq!(status.state_tree.ignored, Some(false));
    assert!(
        status
            .problems
            .iter()
            .any(|p| p.contains("does not ignore")),
        "problems did not name the missing rule: {:?}",
        status.problems
    );
}

#[test]
fn an_ignored_state_tree_is_not_a_blocker() {
    let dir = repo("main", true);
    ignore_state_tree(dir.path());
    let status = collect(&bare_inputs(dir.path()), &probes(listing(true)));

    assert_eq!(status.state_tree.ignored, Some(true));
    assert!(
        !status
            .problems
            .iter()
            .any(|p| p.contains("does not ignore")),
        "{:?}",
        status.problems
    );
}

/// The whole point of the verb is that it reports rather than repairs: it must
/// not create the state tree, and it must not add the ignore rule it is
/// complaining about.
#[test]
fn collecting_creates_no_state() {
    let dir = repo("main", true);
    let before = tree(dir.path());

    let _ = collect(&bare_inputs(dir.path()), &probes(listing(true)));

    assert_eq!(tree(dir.path()), before);
    assert!(!dir.path().join(".superset/.magic").exists());
    assert!(!dir.path().join(".gitignore").exists());
}

// ── Outside a repository ──────────────────────────────────────────────────────

/// Run somewhere that is not a git repository at all. Nothing panics, and every
/// row that has no answer says why rather than rendering blank.
#[test]
fn outside_a_repository_every_undeterminable_row_states_why() {
    let dir = tempfile::tempdir().unwrap();
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Unavailable("no claude here".to_string())),
    );

    assert!(status.repo.root.is_none());
    assert!(status
        .repo
        .note
        .expect("a reason")
        .contains("not inside a git"));
    assert!(status.identity.slug.is_none());
    assert!(status.identity.note.is_some());
    assert!(status.state_tree.root.is_none());
    assert!(status.state_tree.note.is_some());
    // The gate still reports the binary's own defaults, and says they are
    // defaults rather than anything configured.
    assert_eq!(
        status.gate.threshold_lines,
        config::GATE_THRESHOLD_LINES_DEFAULT
    );
    assert!(status.gate.note.expect("a reason").contains("defaults"));
    assert!(status
        .problems
        .iter()
        .any(|p| p.contains("not inside a git repository")));
}

// ── R77: the bootstrap ────────────────────────────────────────────────────────

/// The first-install window: the plugin is installed but the binary is not
/// there yet. `status` reports the pin, the absent path and the last bootstrap
/// outcome instead of failing.
#[test]
fn a_missing_binary_is_reported_with_the_pin_and_the_path() {
    let repo_dir = repo("main", true);
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join(PIN_FILE), "0.10.0\n").unwrap();
    let data = tempfile::tempdir().unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.plugin_root = Located::found(plugin.path().to_path_buf(), "test");
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");

    let status = collect(&inputs, &probes(listing(true)));

    assert_eq!(status.bootstrap.pin.value.as_deref(), Some("0.10.0"));
    assert_eq!(status.bootstrap.binary.exists, Some(false));
    assert!(status
        .bootstrap
        .binary
        .path
        .expect("the resolved path")
        .ends_with("bin/ss-magic"));
    assert_eq!(status.bootstrap.outcome.state, "never-run");
    assert!(status.bootstrap.outcome.detail.contains("0.10.0"));
    assert!(status
        .problems
        .iter()
        .any(|p| p.contains("pinned binary is not installed")));
}

/// A completed install: the marker matches the pin and the binary is there.
#[test]
fn a_completed_install_reports_installed() {
    let repo_dir = repo("main", true);
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join(PIN_FILE), "0.10.0\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    fs::create_dir_all(data.path().join("bin")).unwrap();
    fs::write(data.path().join(BINARY_REL), "#!/bin/sh\n").unwrap();
    fs::write(data.path().join(MARKER_INSTALLED), "0.10.0\n").unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.plugin_root = Located::found(plugin.path().to_path_buf(), "test");
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");

    let status = collect(
        &inputs,
        &Probes {
            harness: listing(true),
            binary_version: &version_0100,
        },
    );

    assert_eq!(status.bootstrap.outcome.state, "installed");
    assert_eq!(
        status.bootstrap.markers.installed.as_deref(),
        Some("0.10.0")
    );
    assert_eq!(status.bootstrap.binary.version.as_deref(), Some("0.10.0"));
}

/// The bootstrap recorded that this platform has no published binary. That is
/// a permanent, deliberate inactivity — and it must be visible, because the
/// script only says it once.
#[test]
fn an_unsupported_platform_marker_is_reported() {
    let repo_dir = repo("main", true);
    let data = tempfile::tempdir().unwrap();
    fs::write(data.path().join(MARKER_UNSUPPORTED), "MINGW64_NT x86_64\n").unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");

    let status = collect(&inputs, &probes(listing(true)));

    assert_eq!(status.bootstrap.outcome.state, "unsupported-platform");
    assert_eq!(
        status.bootstrap.markers.unsupported.as_deref(),
        Some("MINGW64_NT x86_64")
    );
    assert!(status
        .problems
        .iter()
        .any(|p| p.contains("no release binary is published")));
}

/// A completion marker naming an older version than the current pin: the
/// install is stale and the next fresh session replaces it.
#[test]
fn a_marker_behind_the_pin_reports_stale() {
    let repo_dir = repo("main", true);
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join(PIN_FILE), "0.10.0\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    fs::create_dir_all(data.path().join("bin")).unwrap();
    fs::write(data.path().join(BINARY_REL), "#!/bin/sh\n").unwrap();
    fs::write(data.path().join(MARKER_INSTALLED), "0.9.0\n").unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.plugin_root = Located::found(plugin.path().to_path_buf(), "test");
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");

    let status = collect(
        &inputs,
        &Probes {
            harness: listing(true),
            binary_version: &version_090,
        },
    );

    assert_eq!(status.bootstrap.outcome.state, "stale");
    assert!(status.bootstrap.outcome.detail.contains("0.9.0"));
    assert!(status.bootstrap.outcome.detail.contains("0.10.0"));
}

/// With no plugin root there is no pin to read — and that is a note naming what
/// could not be found, never a blank row.
#[test]
fn an_unlocatable_plugin_root_explains_the_missing_pin() {
    let repo_dir = repo("main", true);
    let status = collect(
        &bare_inputs(repo_dir.path()),
        &probes(HarnessListing::Unavailable("no claude here".to_string())),
    );

    assert!(status.bootstrap.pin.value.is_none());
    let note = status.bootstrap.pin.note.expect("a reason");
    assert!(note.contains("ss-magic.version"), "{note}");
    assert_eq!(status.bootstrap.outcome.state, "unknown");
    assert!(status.bootstrap.markers.note.is_some());
}

// ── Version drift ─────────────────────────────────────────────────────────────

/// The manifest and the binary side by side, with the gap named and attributed
/// to the right side. A plugin update lands the manifest first, so a behind
/// binary is a transient state rather than an error.
#[test]
fn a_binary_behind_the_manifest_is_named_as_behind_and_not_an_error() {
    let repo_dir = repo("main", true);
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join(PIN_FILE), "0.10.0\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    fs::create_dir_all(data.path().join("bin")).unwrap();
    fs::write(data.path().join(BINARY_REL), "#!/bin/sh\n").unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.plugin_root = Located::found(plugin.path().to_path_buf(), "test");
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");

    let status = collect(
        &inputs,
        &Probes {
            harness: listing(true),
            binary_version: &version_090,
        },
    );

    assert_eq!(status.versions.manifest.as_deref(), Some("0.10.0"));
    assert_eq!(status.versions.binary.as_deref(), Some("0.9.0"));
    assert_eq!(status.versions.pin.as_deref(), Some("0.10.0"));
    assert_eq!(status.versions.drift, "binary-behind");
    assert!(status.versions.detail.contains("expected and transient"));
}

#[test]
fn matching_versions_report_aligned() {
    let repo_dir = repo("main", true);
    let data = tempfile::tempdir().unwrap();
    fs::create_dir_all(data.path().join("bin")).unwrap();
    fs::write(data.path().join(BINARY_REL), "#!/bin/sh\n").unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");

    let status = collect(
        &inputs,
        &Probes {
            harness: listing(true),
            binary_version: &version_0100,
        },
    );

    assert_eq!(status.versions.drift, "aligned");
    assert!(!status.problems.iter().any(|p| p.contains("version gap")));
}

/// Either side unknown is `unknown`, never a guess — and the detail says which
/// side is missing.
#[test]
fn an_unknown_side_reports_unknown_drift_with_the_reason() {
    let repo_dir = repo("main", true);
    let status = collect(&bare_inputs(repo_dir.path()), &probes(listing(true)));

    assert_eq!(status.versions.manifest.as_deref(), Some("0.10.0"));
    assert_eq!(status.versions.binary, None);
    assert_eq!(status.versions.drift, "unknown");
    assert!(
        status.versions.detail.contains("binary"),
        "{}",
        status.versions.detail
    );
}

#[test]
fn semver_parsing_rejects_anything_that_is_not_three_numbers() {
    assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
    assert_eq!(parse_semver("0.10.0"), Some((0, 10, 0)));
    assert_eq!(parse_semver("1.2"), None);
    assert_eq!(parse_semver("1.2.3.4"), None);
    assert_eq!(parse_semver("1.2.3-rc1"), None);
}

#[test]
fn a_version_line_yields_its_last_token() {
    assert_eq!(
        parse_version_line("ss-magic 0.10.0\n"),
        Some("0.10.0".into())
    );
    assert_eq!(parse_version_line(""), None);
}

// ── AE35 (status half): the heartbeat ─────────────────────────────────────────

/// After a fail-open row, that event's last-fired-at and its error class are
/// both on the row. The class is the machine-readable half — `handler-error`,
/// `disabled`, `not-ignored` — and is what tells apart "it broke" from "it
/// decided not to".
#[test]
fn a_fail_open_row_shows_its_time_and_error_class() {
    let repo_dir = repo("main", true);
    let (_store_dir, store) = store_with(vec![Row::new("pre-tool-use", NOW, Outcome::Error)
        .with_cwd(Some(repo_dir.path().display().to_string()))
        .with_reason("handler-error")
        .with_detail("the gate blew up")]);

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store);

    let status = collect(&inputs, &probes(listing(true)));
    let event = status
        .hooks
        .events
        .iter()
        .find(|e| e.event == "pre-tool-use")
        .expect("the event row");

    assert_eq!(event.last_outcome.as_deref(), Some("error"));
    assert_eq!(event.last_reason.as_deref(), Some("handler-error"));
    assert_eq!(event.last_detail.as_deref(), Some("the gate blew up"));
    assert_eq!(event.last_fired_at.as_deref(), Some("2026-08-30T12:00:00Z"));
    assert_eq!(event.counts.error, 1);
}

/// Counts are per outcome and per event, spelled exactly as the log spells
/// them, so a script can key off them without a translation table.
#[test]
fn outcome_counts_are_per_event() {
    let repo_dir = repo("main", true);
    let cwd = repo_dir.path().display().to_string();
    let (_store_dir, store) = store_with(vec![
        Row::new("session-start", NOW, Outcome::Ok).with_cwd(Some(cwd.clone())),
        Row::new("session-start", NOW + 1, Outcome::NoOp)
            .with_cwd(Some(cwd.clone()))
            .with_reason("disabled"),
        Row::new("pre-tool-use", NOW + 2, Outcome::Ok).with_cwd(Some(cwd)),
    ]);

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store);

    let status = collect(&inputs, &probes(listing(true)));
    let value = json(&status);
    let session_start = value["hooks"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"] == "session-start")
        .unwrap()
        .clone();

    assert_eq!(session_start["counts"]["ok"].as_u64(), Some(1));
    assert_eq!(session_start["counts"]["no-op"].as_u64(), Some(1));
    assert_eq!(session_start["counts"]["error"].as_u64(), Some(0));
    assert_eq!(
        session_start["last_reason"],
        Value::String("disabled".into())
    );
}

/// Rows from another worktree do not count against this one.
#[test]
fn rows_are_scoped_to_this_worktree_by_default() {
    let repo_dir = repo("main", true);
    let (_store_dir, store) = store_with(vec![
        Row::new("session-start", NOW, Outcome::Ok)
            .with_cwd(Some(repo_dir.path().display().to_string())),
        Row::new("session-start", NOW + 1, Outcome::Ok)
            .with_cwd(Some("/somewhere/else".to_string())),
    ]);

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store.clone());
    let scoped = collect(&inputs, &probes(listing(true)));
    assert_eq!(scoped.hooks.scope, "this-worktree");
    assert_eq!(scoped.hooks.rows, 1);

    inputs.all = true;
    let all = collect(&inputs, &probes(listing(true)));
    assert_eq!(all.hooks.scope, "machine");
    assert_eq!(all.hooks.rows, 2);
}

/// A truncated final line — a hook killed mid-append — must not hide the rows
/// before it.
#[test]
fn a_truncated_final_heartbeat_line_does_not_hide_the_rest() {
    let repo_dir = repo("main", true);
    let cwd = repo_dir.path().display().to_string();
    let (_store_dir, store) = store_with(vec![
        Row::new("session-start", NOW, Outcome::Ok).with_cwd(Some(cwd.clone()))
    ]);
    let path = heartbeat::log_path(&store);
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "{{\"event\":\"pre-tool-use\",\"cwd\":\"{cwd}\",\"outc"
    ));
    fs::write(&path, text).unwrap();

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store);

    let status = collect(&inputs, &probes(listing(true)));

    assert_eq!(status.hooks.rows, 1);
    let session_start = status
        .hooks
        .events
        .iter()
        .find(|e| e.event == "session-start")
        .unwrap();
    assert_eq!(session_start.counts.ok, 1);
}

/// Every declared event gets a row whether or not it has ever fired, and one
/// that has not says so rather than rendering blank.
#[test]
fn an_event_that_never_fired_says_so() {
    let repo_dir = repo("main", true);
    let (_store_dir, store) = store_with(Vec::new());

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store);

    let status = collect(&inputs, &probes(listing(true)));

    assert_eq!(status.hooks.events.len(), DECLARED_EVENTS.len());
    for event in &status.hooks.events {
        assert!(event.declared, "{} should be declared", event.event);
        assert!(
            event.last_fired_at.is_none() && event.note.is_some(),
            "{} rendered blank",
            event.event
        );
    }
}

/// `file-changed` is routed in this binary but the shipped manifest does not
/// register it, so it must not appear as a wired-up hook.
#[test]
fn file_changed_is_not_reported_as_a_declared_event() {
    assert!(!DECLARED_EVENTS.contains(&"file-changed"));

    let repo_dir = repo("main", true);
    let (_store_dir, store) = store_with(Vec::new());
    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store);

    let status = collect(&inputs, &probes(listing(true)));

    assert!(!status
        .hooks
        .events
        .iter()
        .any(|e| e.event == "file-changed"));
}

/// A row for an event the manifest no longer declares is still reported — with
/// the fact that it is no longer shipped, so an old row is not mistaken for a
/// live hook.
#[test]
fn an_undeclared_event_with_rows_is_reported_as_not_shipped() {
    let repo_dir = repo("main", true);
    let (_store_dir, store) = store_with(vec![Row::new("file-changed", NOW, Outcome::Ok)
        .with_cwd(Some(repo_dir.path().display().to_string()))]);

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.store = Some(store);

    let status = collect(&inputs, &probes(listing(true)));
    let event = status
        .hooks
        .events
        .iter()
        .find(|e| e.event == "file-changed")
        .expect("the row should still be reported");

    assert!(!event.declared);
    assert!(event
        .note
        .as_deref()
        .unwrap()
        .contains("does not register this event"));
}

/// No heartbeat store at all: every event row says the log could not be read,
/// rather than looking like five events that never fired.
#[test]
fn an_absent_heartbeat_store_makes_every_event_row_say_unknown() {
    let repo_dir = repo("main", true);
    let status = collect(&bare_inputs(repo_dir.path()), &probes(listing(true)));

    assert!(status.hooks.heartbeat.value.is_none());
    assert!(status.hooks.heartbeat.note.is_some());
    assert!(status.hooks.note.is_some());
    for event in &status.hooks.events {
        let note = event.note.as_deref().expect("a reason");
        assert!(
            note.contains("could not be read"),
            "{}: {note}",
            event.event
        );
    }
}

// ── The "nothing renders blank" invariant ─────────────────────────────────────

/// A `Field` with no value always carries a reason, and renders it.
#[test]
fn a_field_with_no_value_renders_its_reason() {
    let field = Field::missing("nothing to read");
    assert!(field.note.is_some());
    assert_eq!(field.render(), "unknown — nothing to read");
}

/// The maximally-undetermined case: no repository, no plugin root, no data
/// directory, no heartbeat store, no harness. Every `Field` in the report has
/// either a value or a reason — never neither.
#[test]
fn every_undetermined_field_carries_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Unavailable("no claude here".to_string())),
    );

    for (name, field) in [
        ("plugin_root", &status.bootstrap.plugin_root),
        ("pin", &status.bootstrap.pin),
        ("data_dir", &status.bootstrap.data_dir),
        ("heartbeat", &status.hooks.heartbeat),
    ] {
        assert!(
            field.value.is_some() || field.note.is_some(),
            "{name} rendered blank"
        );
    }
    // And the same for the plain optional rows an agent reads.
    assert!(status.repo.note.is_some());
    assert!(status.identity.note.is_some());
    assert!(status.state_tree.note.is_some());
    assert!(status.bootstrap.binary.note.is_some());
    assert!(status.bootstrap.markers.note.is_some());
    assert!(status.enablement.harness.note.is_some());
}

/// A clean, fully-working setup produces no problems — which is the only case
/// where an empty `problems` list means what it looks like it means.
#[test]
fn a_healthy_setup_reports_no_problems() {
    let repo_dir = repo("main", true);
    ignore_state_tree(repo_dir.path());
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join(PIN_FILE), "0.10.0\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    fs::create_dir_all(data.path().join("bin")).unwrap();
    fs::write(data.path().join(BINARY_REL), "#!/bin/sh\n").unwrap();
    fs::write(data.path().join(MARKER_INSTALLED), "0.10.0\n").unwrap();
    let (_store_dir, store) = store_with(vec![Row::new("session-start", NOW, Outcome::Ok)
        .with_cwd(Some(repo_dir.path().display().to_string()))]);

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.plugin_root = Located::found(plugin.path().to_path_buf(), "test");
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");
    inputs.store = Some(store);

    let status = collect(
        &inputs,
        &Probes {
            harness: listing(true),
            binary_version: &version_0100,
        },
    );

    assert_eq!(status.enablement.acting, Some(true));
    assert!(
        status.problems.is_empty(),
        "unexpected problems: {:?}",
        status.problems
    );
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// The text form must not hide an unknown behind an empty column: every row
/// with no value carries its reason on the same line.
#[test]
fn the_text_rendering_never_leaves_an_unknown_blank() {
    let dir = tempfile::tempdir().unwrap();
    let status = collect(
        &bare_inputs(dir.path()),
        &probes(HarnessListing::Unavailable(
            "`claude` is not on PATH".to_string(),
        )),
    );

    let mut out = String::new();
    render_text(&mut out, &status);

    // Nothing after a label may be empty — an unknown row says "unknown — why".
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if rest.trim().is_empty() {
            continue;
        }
        assert!(
            rest.len() <= LABEL_WIDTH || !rest[LABEL_WIDTH..].trim().is_empty(),
            "a row rendered with an empty value: {line:?}"
        );
    }

    // And the specific unknowns this fixture produces are all explained.
    assert!(out.contains("unknown — "), "{out}");
    assert!(out.contains("is not on PATH"), "{out}");
    assert!(out.contains("not inside a git repository"), "{out}");
    assert!(out.contains("Why nothing may be happening"), "{out}");
}

/// A healthy setup renders the positive answer rather than a wall of unknowns.
#[test]
fn the_text_rendering_reports_a_healthy_setup_plainly() {
    let repo_dir = repo("main", true);
    ignore_state_tree(repo_dir.path());
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join(PIN_FILE), "0.10.0\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    fs::create_dir_all(data.path().join("bin")).unwrap();
    fs::write(data.path().join(BINARY_REL), "#!/bin/sh\n").unwrap();
    fs::write(data.path().join(MARKER_INSTALLED), "0.10.0\n").unwrap();
    let (_store_dir, store) = store_with(vec![Row::new("session-start", NOW, Outcome::Ok)
        .with_cwd(Some(repo_dir.path().display().to_string()))]);

    let mut inputs = bare_inputs(repo_dir.path());
    inputs.plugin_root = Located::found(plugin.path().to_path_buf(), "test");
    inputs.data_dir = Located::found(data.path().to_path_buf(), "test");
    inputs.store = Some(store);

    let status = collect(
        &inputs,
        &Probes {
            harness: listing(true),
            binary_version: &version_0100,
        },
    );

    let mut out = String::new();
    render_text(&mut out, &status);

    assert!(out.contains("Nothing is blocking the plugin."), "{out}");
    assert!(out.contains("aligned"), "{out}");
    assert!(out.contains("ss-magic@ss-magic"), "{out}");
}

/// The unknown text is one function, so a row cannot grow its own spelling of
/// "we don't know" — and a caller that forgets the reason still renders
/// something visible rather than an empty column.
#[test]
fn the_unknown_text_always_says_something() {
    assert_eq!(
        unknown(Some("the file was not there")),
        "unknown — the file was not there"
    );
    assert_eq!(unknown(None), "unknown — no reason recorded");
}
