//! Drives [`handle`] directly against a constructed [`HookContext`] rather
//! than the full pipeline (U11's `hook/tests.rs` already covers the gates
//! that sit in front of every handler) — these tests are about what THIS
//! handler does once it is reached: the guidance text, the version-drift
//! notice, and the R15 outside-a-repository case.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use tempfile::TempDir;

use super::*;
use crate::config::PluginConfig;
use crate::git;
use crate::hook::event::{Common, Envelope, Payload, SessionStart};
use crate::HookEvent;
use ss_magic_core::release::Cache;
use ss_magic_core::testutil::{git_run, init_main_repo, neutralize_global_excludes};

const NOW: u64 = 1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.

// ── Fixtures ───────────────────────────────────────────────────────────────

/// A repo whose `.gitignore` already covers the state tree — the ordinary
/// case, after `init`/`migrate` or `plugin enable` has run.
fn ignored_repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    fs::write(dir.path().join(".gitignore"), "target/\n.superset/.magic/\n").unwrap();
    git_run(&["add", ".gitignore"], dir.path());
    git_run(&["commit", "-q", "-m", "gitignore"], dir.path());
    let root = git::cwd_repo_root(dir.path()).unwrap();
    (dir, root)
}

/// A repository nested several directories deep, with a long repo-name
/// component and a branch name at (and past) the 40-character slug
/// truncation bound — the "realistic worst case" the character-budget test
/// has to measure against rather than a nominal template.
fn deep_long_repo() -> (TempDir, PathBuf) {
    let base = tempfile::tempdir().unwrap();
    let long_repo_name = "a".repeat(80);
    let nested = base
        .path()
        .join("Users/example/.superset/worktrees/8f14e45f-ceea-467e-9f0a-3f2e1c9b1a55")
        .join(&long_repo_name);
    fs::create_dir_all(&nested).unwrap();
    git_run(&["init", "-q", "-b", "main"], &nested);
    neutralize_global_excludes(&nested);
    fs::write(nested.join("README.md"), "hi").unwrap();
    git_run(&["add", "."], &nested);
    git_run(&["commit", "-q", "-m", "init"], &nested);

    // A cross-repo PR-style branch name, well past the 40-char truncation
    // bound once slugified.
    let long_branch = format!("someforkowner/{}", "feature-branch-name-".repeat(4));
    git_run(&["checkout", "-q", "-b", &long_branch], &nested);

    fs::write(nested.join(".gitignore"), "target/\n.superset/.magic/\n").unwrap();
    git_run(&["add", ".gitignore"], &nested);
    git_run(&["commit", "-q", "-m", "gitignore"], &nested);

    let root = git::cwd_repo_root(&nested).unwrap();
    (base, root)
}

/// The envelope a real `SessionStart` invocation carries, pointed at `cwd`.
fn envelope_for(cwd: &Path, source: &str, session_id: &str) -> Envelope {
    Envelope {
        common: Common {
            session_id: session_id.to_string(),
            transcript_path: String::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            hook_event_name: "SessionStart".to_string(),
            prompt_id: None,
        permission_mode: None,
        },
        payload: Payload::SessionStart(SessionStart {
            source: source.to_string(),
            context_tokens: None,
            estimated_cache_write_usd: None,
        }),
        raw: serde_json::json!({}),
    }
}

/// A `HookContext` built by hand rather than through the pipeline — this
/// module tests the handler in isolation, not the gates in front of it.
fn ctx_for<'a>(
    event: &'a HookEvent,
    envelope: &'a Envelope,
    repo_root: Option<PathBuf>,
    config: &'a PluginConfig,
) -> HookContext<'a> {
    // Never read by this handler; only `repo_root` and `envelope.common.cwd`
    // are, so an arbitrary placeholder is fine here.
    let config_root = repo_root.clone().unwrap_or_else(|| PathBuf::from("/"));
    HookContext {
        event,
        envelope,
        main_root: repo_root.clone(),
        repo_root,
        config_root,
        config,
        now: NOW,
        diagnostics: RefCell::new(Vec::new()),
    }
}

/// Surroundings that trigger nothing: no plugin root, no override, no
/// entrypoint, no cache directory, no lock root, and a spawner that panics if
/// reached — so a test of the guidance text can never write a once-per-machine
/// marker into the developer's own cache directory, nor start a process.
fn inert() -> Surroundings {
    Surroundings {
        plugin_root: None,
        override_present: false,
        entrypoint: None,
        cache_dir: Box::new(|| None),
        lock_root: Box::new(|| None),
        spawn_refresh: Box::new(|| panic!("inert surroundings must never spawn")),
    }
}

/// A spawner that records how many times it was asked, and succeeds.
fn counting_spawner() -> (Rc<Cell<u32>>, Box<dyn Fn() -> std::result::Result<u32, String>>) {
    let count = Rc::new(Cell::new(0));
    let seen = Rc::clone(&count);
    (
        count,
        Box::new(move || {
            seen.set(seen.get() + 1);
            Ok(4242)
        }),
    )
}

/// [`handle`] against inert surroundings — what every guidance test drives.
fn handle_inert(ctx: &HookContext<'_>) -> Result<Outcome> {
    handle_with(ctx, &inert())
}

fn session_start_response(outcome: &Outcome) -> (&Option<String>, &Option<String>) {
    match &outcome.response {
        Response::SessionStart {
            additional_context,
            system_message,
        } => (additional_context, system_message),
        other => panic!("expected a SessionStart response, got {other:?}"),
    }
}

// ── Routing ──────────────────────────────────────────────────────────────────

/// U11 wires `route()` purely off the event name, not the payload, so this is
/// the one place that has to prove the wiring landed on this module.
#[test]
fn session_start_routes_through_this_handler() {
    let route = crate::hook::route(&HookEvent::SessionStart).unwrap();
    assert_eq!(route.handler as *const (), handle as *const ());
}

/// All five sources the harness emits reach the same handler and each comes
/// back with guidance — the routing table does not (and must not) branch on
/// `source`.
#[test]
fn every_known_source_produces_guidance() {
    let (_dir, root) = ignored_repo();
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    for source in ["startup", "resume", "clear", "compact", "fork"] {
        let envelope = envelope_for(&root, source, "sess-1");
        let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);
        let outcome = handle_inert(&ctx).unwrap();
        let (additional_context, _) = session_start_response(&outcome);
        assert!(
            additional_context.is_some(),
            "source `{source}` produced no guidance"
        );
    }
}

/// A `source` this build has never heard of (a harness ahead of this binary)
/// is treated exactly like any other — guidance still goes out, and the
/// value is recorded verbatim in the heartbeat detail rather than causing a
/// failure.
#[test]
fn an_unrecognized_source_is_handled_like_any_other() {
    let (_dir, root) = ignored_repo();
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "some-future-source", "sess-1");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let outcome = handle_inert(&ctx).unwrap();
    let (additional_context, _) = session_start_response(&outcome);
    assert!(additional_context.is_some());
    assert!(outcome
        .detail
        .as_deref()
        .unwrap()
        .contains("some-future-source"));
}

/// An envelope with no session id at all (the field defaults to empty on
/// decode) must not stop the handler — nothing here reads it as an
/// identifier, and the guidance is unaffected.
#[test]
fn an_empty_session_id_does_not_prevent_guidance() {
    let (_dir, root) = ignored_repo();
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "startup", "");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let outcome = handle_inert(&ctx).unwrap();
    let (additional_context, _) = session_start_response(&outcome);
    assert!(additional_context.is_some());
}

// ── R15: outside a git repository ─────────────────────────────────────────────

#[test]
fn outside_a_git_repository_emits_nothing() {
    let dir = tempfile::tempdir().unwrap(); // deliberately not a git repository
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(dir.path(), "startup", "sess-1");
    let ctx = ctx_for(&event, &envelope, None, &config);

    let outcome = handle_inert(&ctx).unwrap();
    assert_eq!(outcome.response, Response::Silent);
    assert!(outcome.detail.is_some(), "the heartbeat still needs a reason");
    assert!(
        !dir.path().join(".superset").exists(),
        "nothing may be written outside a git repository"
    );
}

// ── The character budget (R19) ────────────────────────────────────────────────

/// Measured against a realistic worst case — a long repo-name component, a
/// branch past the 40-character truncation bound, and a deeply nested
/// worktree path — not a nominal template. The 10,000-character cliff is the
/// harness's own hard replacement point for `additionalContext`
/// (`hook-contract.md`), so this asserts the actual rendered length.
#[test]
fn additional_context_stays_well_under_the_ten_thousand_character_budget() {
    let (_base, root) = deep_long_repo();
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "startup", "sess-1");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let outcome = handle_inert(&ctx).unwrap();
    let (additional_context, _) = session_start_response(&outcome);
    let text = additional_context.clone().expect("guidance must be injected");

    assert!(
        text.len() < 10_000,
        "additionalContext is {} chars (>= the 10,000 cliff):\n{text}",
        text.len()
    );
    // A margin check, not just the bound itself — this is meant to catch the
    // guidance quietly growing toward the cliff over time, not only crossing
    // it outright.
    assert!(
        text.len() < 5_000,
        "additionalContext used {} of the 10,000-char budget, more than half: \
         reconsider what is inline before this creeps further",
        text.len()
    );
}

/// A `Refusal::TrackedPaths` list is capped, not joined in full, so a
/// repository that `git add -f`s a few hundred files under the gitignored
/// state tree cannot push `additionalContext` past R19's 10,000-character
/// cliff on this one refusal alone. Built directly against [`build_guidance`]
/// rather than through a real git repo — that keeps the test fast and lets
/// it name an exact, worst-case-realistic count of tracked paths — and
/// asserts on the measured length, the property that actually matters, not
/// on how the truncation is spelled.
#[test]
fn a_tracked_paths_refusal_is_capped_so_the_budget_survives_hundreds_of_paths() {
    let root = PathBuf::from("/repo");
    let session_dir = root.join(".superset/.magic/sessions/2026-08-30-abc123");
    let paths: Vec<String> = (0..400)
        .map(|i| format!(".superset/.magic/sessions/2026-08-30-abc123/leaked-secret-{i:04}.env"))
        .collect();
    let report = Report {
        state_root: root.join(".superset/.magic"),
        slug: "2026-08-30-abc123".to_string(),
        session_dir,
        created: Vec::new(),
        refusals: vec![Refusal::TrackedPaths { paths }],
        wrote_state: true,
    };

    let text = build_guidance(&root, &report);

    assert!(
        text.len() < 10_000,
        "additionalContext is {} chars (>= the 10,000 cliff) with 400 tracked paths:\n{text}",
        text.len()
    );
    assert!(
        text.contains("## Operator checklist"),
        "the checklist section must survive an inflated tracked-paths refusal: {text}"
    );
}

/// A run that created directories and THEN refused must not be told "nothing
/// was written", and must not lose the `## Operator checklist` section with
/// that sentence.
///
/// `wrote_state` is not a record of what happened — it answers "is this tree
/// safe to write into". The two late refusal sites (the session pointer, and
/// `scaffold`) fire after the directory pass has already created things, so
/// `wrote_state == false` with a non-empty `created` is a real and expected
/// combination. Branching the "nothing was written" wording on the flag rather
/// than on `created` told the model something false about a run that mostly
/// succeeded, and returned early past the checklist verbs it still needs.
///
/// Built against [`build_guidance`] directly: producing this state through a
/// real repository would mean making one path under the tree escape the
/// worktree mid-scaffold, which says nothing more about the rendering than
/// naming the combination outright does.
#[test]
fn a_refusal_after_directories_were_created_keeps_the_checklist_and_admits_the_writes() {
    let root = PathBuf::from("/repo");
    let session_dir = root.join(".superset/.magic/sessions/2026-08-30-abc123");
    let report = Report {
        state_root: root.join(".superset/.magic"),
        slug: "2026-08-30-abc123".to_string(),
        session_dir: session_dir.clone(),
        created: vec![
            ".superset/.magic".to_string(),
            ".superset/.magic/sessions".to_string(),
            ".superset/.magic/sessions/2026-08-30-abc123".to_string(),
        ],
        refusals: vec![Refusal::Escapes {
            path: ".superset/.magic/sessions/2026-08-30-abc123/STATUS.md".to_string(),
            detail: "resolves to /elsewhere/STATUS.md".to_string(),
        }],
        wrote_state: false,
    };

    let text = build_guidance(&root, &report);

    assert!(
        !text.contains("Nothing was written"),
        "three directories were created; the guidance must not deny it: {text}"
    );
    assert!(
        !text.to_lowercase().contains("not set up yet"),
        "the tree was partly set up, so this wording is false: {text}"
    );
    assert!(
        text.contains("## Operator checklist"),
        "a partial run still needs the checklist verbs: {text}"
    );
    assert!(
        text.contains("ss-magic-plugin checklist list"),
        "and the verbs themselves, not just the heading: {text}"
    );
    // The refusal still has to reach the model in both branches — it is the
    // only signal that the tree is not safe to write into.
    assert!(
        text.contains("/elsewhere/STATUS.md"),
        "the refusal must survive into the full guidance: {text}"
    );
    assert!(
        text.contains("did not finish"),
        "and it must be flagged as a stopped scaffold, not a skipped path: {text}"
    );
}

/// The mirror of the case above: a completed run that merely skipped a tracked
/// path keeps the softer wording, so the warning above stays meaningful.
#[test]
fn a_completed_run_with_a_skipped_path_is_not_reported_as_a_stopped_scaffold() {
    let root = PathBuf::from("/repo");
    let report = Report {
        state_root: root.join(".superset/.magic"),
        slug: "2026-08-30-abc123".to_string(),
        session_dir: root.join(".superset/.magic/sessions/2026-08-30-abc123"),
        created: vec![".superset/.magic".to_string()],
        refusals: vec![Refusal::TrackedPaths {
            paths: vec![".superset/.magic/README.md".to_string()],
        }],
        wrote_state: true,
    };

    let text = build_guidance(&root, &report);

    assert!(text.contains("left untouched"), "{text}");
    assert!(!text.contains("did not finish"), "{text}");
    assert!(text.contains("## Operator checklist"), "{text}");
}

// ── `compact` re-injection (F2) ────────────────────────────────────────────────

/// `compact` is the whole reason this handler runs on all five sources: it
/// has to restore orientation after the window was cleared without touching
/// what the model already wrote in its own files.
#[test]
fn compact_reinjects_guidance_without_touching_existing_files() {
    let (_dir, root) = ignored_repo();
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();

    let first = envelope_for(&root, "startup", "sess-1");
    let ctx = ctx_for(&event, &first, Some(root.clone()), &config);
    handle_inert(&ctx).unwrap();

    let report = scratchpad::ensure(&root).unwrap();
    let status_path = report.session_dir.join("STATUS.md");
    let custom = "# Status\n\nmy own notes from earlier in this session\n";
    fs::write(&status_path, custom).unwrap();

    let second = envelope_for(&root, "compact", "sess-1");
    let ctx2 = ctx_for(&event, &second, Some(root.clone()), &config);
    let outcome = handle_inert(&ctx2).unwrap();

    assert_eq!(
        fs::read_to_string(&status_path).unwrap(),
        custom,
        "a `compact` re-run must never rewrite the model's own notes"
    );
    let (additional_context, _) = session_start_response(&outcome);
    let text = additional_context.as_ref().expect("compact still needs the guidance re-injected");
    assert!(text.contains(&report.slug));
    assert!(text.contains("STATUS.md"));
}

// ── A refused scratchpad (R63) ────────────────────────────────────────────────

/// When `ensure` refuses outright (no ignore rule for the state tree yet),
/// the guidance must say so plainly and must not claim any state file
/// exists — none of them were written.
#[test]
fn a_refused_scratchpad_does_not_claim_files_exist() {
    let dir = init_main_repo("main"); // no `.superset/.magic/` ignore rule at all
    let root = git::cwd_repo_root(dir.path()).unwrap();
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "startup", "sess-1");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let outcome = handle_inert(&ctx).unwrap();
    let (additional_context, _) = session_start_response(&outcome);
    let text = additional_context.as_ref().expect("a refusal still gets an explanation");

    for (name, _) in STATE_FILE_NOTES {
        assert!(
            !text.contains(name),
            "a refused scratchpad must not name {name} as if it existed: {text}"
        );
    }
    assert!(text.to_lowercase().contains("not set up"), "{text}");
    assert!(!root.join(scratchpad::STATE_REL).exists());

    let detail = outcome.detail.unwrap();
    assert!(detail.contains("refused"), "{detail}");
}

// ── The state-file table (drift guard) ────────────────────────────────────────

/// The guidance names exactly the files U8 scaffolds, in the same spelling —
/// a name here that U8 does not create would be worse than no guidance.
#[test]
fn state_file_notes_match_what_u8_actually_scaffolds() {
    let names: Vec<&str> = STATE_FILE_NOTES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, scratchpad::STATE_FILES.to_vec());
}

// ── The version-drift self-check ──────────────────────────────────────────────

#[test]
fn version_drift_notice_flags_a_mismatch_and_names_both_versions() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ss-magic-plugin.version"), "0.0.1\n").unwrap();

    let notice = version_drift_notice(Some(dir.path()))
        .expect("a differing pin must produce a notice");
    assert!(notice.contains("0.0.1"), "{notice}");
    assert!(notice.contains(env!("CARGO_PKG_VERSION")), "{notice}");
}

#[test]
fn version_drift_notice_is_silent_on_a_match() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("ss-magic-plugin.version"),
        format!("{}\n", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();

    assert_eq!(version_drift_notice(Some(dir.path())), None);
}

#[test]
fn version_drift_notice_is_silent_with_no_plugin_root() {
    assert_eq!(version_drift_notice(None), None);
}

#[test]
fn version_drift_notice_is_silent_when_the_pin_file_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(version_drift_notice(Some(dir.path())), None);
}

/// The injected guidance must name the wrapper, `ss-magic-plugin`, and never a
/// bare `ss-magic`.
///
/// The model runs these commands through the Bash tool, where
/// `${CLAUDE_PLUGIN_DATA}` is not exported -- so the bootstrapped binary cannot
/// be named directly, and the wrapper is what resolves it. A bare `ss-magic`
/// would also resolve against whatever the user has on PATH, which is the
/// reason R75 gives the wrapper a distinct name in the first place. Guidance
/// naming a command the model cannot reliably run is worse than no guidance,
/// and it ships on every session start, so it is worth a test of its own.
#[test]
fn the_injected_guidance_names_the_wrapper_not_a_bare_ss_magic() {
    for line in CHECKLIST_VERBS.lines() {
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }
        assert!(
            cmd.starts_with("ss-magic-plugin checklist "),
            "every verb line must invoke the wrapper; got: {cmd}"
        );
    }
    // Belt and braces: no line may invoke the sync CLI. `ss-magic` is a
    // different binary on a different release line and has never had these
    // verbs, so a line starting with it would be a command the model cannot
    // run. The check is `"ss-magic "` with the trailing space precisely so it
    // does NOT match the correct `ss-magic-plugin …` spelling.
    assert!(
        !CHECKLIST_VERBS
            .lines()
            .any(|l| l.trim().starts_with("ss-magic ")),
        "the guidance must not invoke the `ss-magic` sync CLI"
    );
}


// ── The compaction notice (R27) ───────────────────────────────────────────────

/// A directory closure over a tempdir — the cache directory or the lock root.
fn marker_in(dir: &Path) -> Box<dyn Fn() -> Option<PathBuf>> {
    let dir = dir.to_path_buf();
    Box::new(move || Some(dir.clone()))
}

/// The ordinary case: a fresh `startup`, the override in the environment, no
/// window configured. The notice goes out once and the marker records it;
/// the very next startup on the same machine is silent.
#[test]
fn the_compaction_notice_fires_once_per_machine_on_startup() {
    let (_dir, root) = ignored_repo();
    let cache = tempfile::tempdir().unwrap();

    let first = compaction_advice(&root, "startup", None, true, &marker_in(cache.path()), NOW);
    let message = first.message.expect("the first startup gets the notice");
    assert!(message.contains(OVERRIDE_ENV), "{message}");
    assert!(message.contains("compact-window --recommend"), "{message}");
    assert!(message.contains("never edits"), "{message}");
    assert!(message.contains("once"), "{message}");
    assert!(
        cache.path().join(COMPACT_ADVICE_MARKER).is_file(),
        "the marker must be written when the notice goes out"
    );
    assert!(first.detail.as_deref().unwrap().contains("shown"));

    let second = compaction_advice(&root, "startup", None, true, &marker_in(cache.path()), NOW + 1);
    assert_eq!(second.message, None, "once per machine");
    assert!(
        second.detail.as_deref().unwrap().contains("already shown"),
        "{:?}",
        second.detail
    );
}

/// Only `startup` carries it: a resume, clear, compact or fork is the same
/// person mid-session, and none of them may nag.
#[test]
fn the_compaction_notice_is_absent_on_every_source_but_startup() {
    let (_dir, root) = ignored_repo();
    let cache = tempfile::tempdir().unwrap();
    for source in ["resume", "clear", "compact", "fork", "", "some-future-source"] {
        let advice = compaction_advice(&root, source, None, true, &marker_in(cache.path()), NOW);
        assert_eq!(advice.message, None, "source `{source}` must be silent");
    }
    assert!(
        !cache.path().join(COMPACT_ADVICE_MARKER).exists(),
        "a silent path must not consume the once-per-machine budget"
    );
}

/// A quiet session (nobody watching) gets nothing and keeps its budget.
#[test]
fn the_compaction_notice_is_absent_under_quiet_mode() {
    let (_dir, root) = ignored_repo();
    let cache = tempfile::tempdir().unwrap();
    let advice = compaction_advice(
        &root,
        "startup",
        Some("permission_mode is bypassPermissions"),
        true,
        &marker_in(cache.path()),
        NOW,
    );
    assert_eq!(advice.message, None);
    assert!(advice.detail.as_deref().unwrap().contains("quiet"), "{:?}", advice.detail);
    assert!(!cache.path().join(COMPACT_ADVICE_MARKER).exists());
}

/// A configured window IS the remedy, so there is nothing to advise.
#[test]
fn the_compaction_notice_is_absent_when_a_window_is_configured() {
    let (_dir, root) = ignored_repo();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(
        root.join(".claude/settings.local.json"),
        r#"{"autoCompactWindow":200000}"#,
    )
    .unwrap();
    let cache = tempfile::tempdir().unwrap();
    let advice = compaction_advice(&root, "startup", None, true, &marker_in(cache.path()), NOW);
    assert_eq!(advice.message, None);
    assert!(!cache.path().join(COMPACT_ADVICE_MARKER).exists());
}

/// Without the override there is nothing to advise about, whatever else is
/// configured.
#[test]
fn the_compaction_notice_is_absent_when_the_override_is_not_in_the_environment() {
    let (_dir, root) = ignored_repo();
    let cache = tempfile::tempdir().unwrap();
    let advice = compaction_advice(&root, "startup", None, false, &marker_in(cache.path()), NOW);
    assert_eq!(advice.message, None);
    assert!(!cache.path().join(COMPACT_ADVICE_MARKER).exists());
}

/// With nowhere to record that it was shown, "once per machine" cannot be
/// kept — so the notice is withheld rather than repeated every session.
#[test]
fn the_compaction_notice_is_absent_when_no_marker_directory_can_be_resolved() {
    let (_dir, root) = ignored_repo();
    let advice = compaction_advice(&root, "startup", None, true, &|| None, NOW);
    assert_eq!(advice.message, None);
}

/// The two operator notices share one `systemMessage`, a blank line apart.
#[test]
fn system_messages_join_with_a_blank_line() {
    assert_eq!(
        join_system_messages([Some("a".to_string()), Some("b".to_string())]),
        Some("a\n\nb".to_string())
    );
    assert_eq!(
        join_system_messages([None, Some("b".to_string())]),
        Some("b".to_string())
    );
    assert_eq!(join_system_messages([None, None]), None);
}

/// Through the handler: the notice lands beside the version-drift notice on
/// `systemMessage`, and never in `additionalContext` — the model has nothing
/// to do with it.
#[test]
fn the_handler_puts_the_notice_on_system_message_beside_version_drift() {
    let (_dir, root) = ignored_repo();
    let plugin = tempfile::tempdir().unwrap();
    fs::write(plugin.path().join("ss-magic-plugin.version"), "0.0.1\n").unwrap();
    let cache = tempfile::tempdir().unwrap();
    let surroundings = Surroundings {
        plugin_root: Some(plugin.path().to_path_buf()),
        override_present: true,
        entrypoint: Some("cli".into()),
        cache_dir: marker_in(cache.path()),
        lock_root: Box::new(|| None),
        // The cache directory holds no release cache, so the handler spawns a
        // refresh; a real spawn is not wanted here.
        spawn_refresh: Box::new(|| Ok(1)),
    };
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "startup", "sess-1");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let outcome = handle_with(&ctx, &surroundings).unwrap();
    let (additional_context, system_message) = session_start_response(&outcome);
    let system_message = system_message.as_deref().expect("both notices are due");
    assert!(system_message.contains("pins v0.0.1"), "{system_message}");
    assert!(system_message.contains(OVERRIDE_ENV), "{system_message}");
    assert!(system_message.contains("\n\n"), "{system_message}");
    assert!(
        !additional_context.as_deref().unwrap().contains(OVERRIDE_ENV),
        "the notice must not enter the model's context"
    );
    assert!(outcome.detail.as_deref().unwrap().contains("compaction notice"));
}

/// The same handler under quiet surroundings: the drift notice (ungated, as
/// before) still goes out, the compaction notice does not.
#[test]
fn the_handler_withholds_the_notice_under_quiet_mode() {
    let (_dir, root) = ignored_repo();
    let cache = tempfile::tempdir().unwrap();
    let surroundings = Surroundings {
        plugin_root: None,
        override_present: true,
        entrypoint: Some("sdk-ts".into()),
        cache_dir: marker_in(cache.path()),
        lock_root: Box::new(|| None),
        spawn_refresh: Box::new(|| panic!("a quiet session must not spawn a refresh")),
    };
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "startup", "sess-1");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let outcome = handle_with(&ctx, &surroundings).unwrap();
    let (_, system_message) = session_start_response(&outcome);
    assert_eq!(*system_message, None);
    assert!(!cache.path().join(COMPACT_ADVICE_MARKER).exists());
}

// ── The plugin release suggestion (R29–R31, AE10–AE12) ────────────────────────

/// The version the plugin roots below pin: the running binary's own, so
/// `version_drift_notice` stays silent and every `systemMessage` assertion
/// sees the release suggestion alone. It used to be the literal `"1.0.0"`,
/// which quietly assumed the crate would stay at that version; the first bump
/// (to 1.0.1) turned on the drift notice and failed six tests here.
const PIN: &str = env!("CARGO_PKG_VERSION");

/// A plugin release newer than any version this crate will carry, so the
/// suggestion fires whatever `PIN` is.
const NEWER: &str = "ss-magic-plugin-v999.0.0";

/// A plugin root pinning `pin`.
fn plugin_root_pinning(pin: &str) -> TempDir {
    let plugin = tempfile::tempdir().unwrap();
    fs::write(
        plugin.path().join(crate::status::PIN_FILE),
        format!("{pin}\n"),
    )
    .unwrap();
    plugin
}

/// A cache directory whose plugin release cache holds `tag`, checked `age`
/// seconds before `NOW`.
fn cache_dir_with(tag: &str, age: u64) -> TempDir {
    let cache = tempfile::tempdir().unwrap();
    crate::release_check::write_cache(
        &cache.path().join(PLUGIN_LINE.cache_file),
        &Cache {
            checked_at: NOW - age,
            tag_name: tag.to_string(),
            etag: None,
            suggested: None,
        },
    )
    .unwrap();
    cache
}

fn read_release_cache(cache: &TempDir) -> Cache {
    release::read_cache(&cache.path().join(PLUGIN_LINE.cache_file)).unwrap()
}

/// Surroundings for a watched terminal session on an installed plugin
/// pinning `PIN`, with the given cache directory and a recording spawner.
fn watched(
    plugin: &TempDir,
    cache: &TempDir,
    lock: &TempDir,
) -> (Surroundings, Rc<Cell<u32>>) {
    let (count, spawner) = counting_spawner();
    (
        Surroundings {
            plugin_root: Some(plugin.path().to_path_buf()),
            override_present: false,
            entrypoint: Some("cli".into()),
            cache_dir: marker_in(cache.path()),
            lock_root: marker_in(lock.path()),
            spawn_refresh: spawner,
        },
        count,
    )
}

fn run_startup(root: &Path, surroundings: &Surroundings, session: &str) -> Outcome {
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(root, "startup", session);
    let ctx = ctx_for(&event, &envelope, Some(root.to_path_buf()), &config);
    handle_with(&ctx, surroundings).unwrap()
}

/// AE10: pin `PIN`, cache newest `NEWER`, nothing suggested yet, `startup`,
/// `permission_mode` default. The first run announces on `systemMessage`
/// with the `/plugin` flow; the second is silent because `suggested` now
/// equals that tag; `additionalContext` never mentions it.
#[test]
fn ae10_plugin_update_is_suggested_once() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    let cache = cache_dir_with(NEWER, 60);
    let lock = tempfile::tempdir().unwrap();
    let (surroundings, _spawns) = watched(&plugin, &cache, &lock);

    let first = run_startup(&root, &surroundings, "sess-1");
    let (additional_context, system_message) = session_start_response(&first);
    let message = system_message.as_deref().expect("the first startup announces");
    assert!(message.contains(NEWER), "{message}");
    assert!(message.contains(&format!("pins {PIN}")), "{message}");
    assert!(message.contains("/plugin"), "{message}");
    assert!(message.contains("new session"), "{message}");
    assert!(
        !additional_context.as_deref().unwrap().contains(NEWER),
        "the suggestion must never enter the model's context"
    );
    assert!(first.detail.as_deref().unwrap().contains("release suggestion shown"));
    assert_eq!(read_release_cache(&cache).suggested.as_deref(), Some(NEWER));

    let second = run_startup(&root, &surroundings, "sess-2");
    let (additional_context, system_message) = session_start_response(&second);
    assert_eq!(*system_message, None, "once per release");
    assert!(!additional_context.as_deref().unwrap().contains(NEWER));
    assert!(
        second.detail.as_deref().unwrap().contains("already shown"),
        "{:?}",
        second.detail
    );
}

/// AE11: the same cache under `bypassPermissions` (or an SDK entrypoint):
/// no `systemMessage`, the heartbeat detail says why, nothing is written,
/// and — R30 — no refresh is spawned either.
#[test]
fn ae11_a_headless_session_stays_silent_and_spawns_nothing() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    // Stale, so a watched session WOULD spawn a refresh here.
    let cache = cache_dir_with(NEWER, 2 * 24 * 60 * 60);
    let lock = tempfile::tempdir().unwrap();

    // Via the envelope's permission mode.
    let (surroundings, spawns) = watched(&plugin, &cache, &lock);
    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let mut envelope = envelope_for(&root, "startup", "sess-1");
    envelope.common.permission_mode = Some("bypassPermissions".to_string());
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);
    let outcome = handle_with(&ctx, &surroundings).unwrap();
    let (_, system_message) = session_start_response(&outcome);
    assert_eq!(*system_message, None);
    let detail = outcome.detail.as_deref().unwrap();
    assert!(
        detail.contains("release suggestion suppressed (quiet mode"),
        "{detail}"
    );
    assert_eq!(spawns.get(), 0, "no refresh for a session nobody watches");
    assert_eq!(read_release_cache(&cache).suggested, None, "budget not spent");

    // Via the entrypoint.
    let (mut surroundings, spawns) = watched(&plugin, &cache, &lock);
    surroundings.entrypoint = Some("sdk-ts".into());
    let outcome = run_startup(&root, &surroundings, "sess-2");
    let (_, system_message) = session_start_response(&outcome);
    assert_eq!(*system_message, None);
    assert_eq!(spawns.get(), 0);

    // And the very next watched startup gets the notice: quiet mode did not
    // consume the once-per-tag budget.
    let (surroundings, _) = watched(&plugin, &cache, &lock);
    let outcome = run_startup(&root, &surroundings, "sess-3");
    let (_, system_message) = session_start_response(&outcome);
    assert!(system_message.as_deref().unwrap().contains(NEWER));
}

/// AE12: no cache at all. The handler returns (nothing here waits on a
/// network — the source scan in `hook/tests.rs` proves no client exists on
/// this path), the detached refresh was asked for exactly once, and the
/// startup after the refresh has cached a newer tag announces it.
#[test]
fn ae12_session_start_spawns_a_detached_refresh_and_does_not_wait() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    let cache = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let (surroundings, spawns) = watched(&plugin, &cache, &lock);

    let outcome = run_startup(&root, &surroundings, "sess-1");
    let (_, system_message) = session_start_response(&outcome);
    assert_eq!(*system_message, None, "nothing to suggest before the first refresh");
    assert_eq!(spawns.get(), 1, "one detached refresh");
    assert!(
        outcome.detail.as_deref().unwrap().contains("release refresh spawned"),
        "{:?}",
        outcome.detail
    );

    // "The next startup reads whatever that process cached": simulate the
    // refresh having landed.
    crate::release_check::write_cache(
        &cache.path().join(PLUGIN_LINE.cache_file),
        &Cache {
            checked_at: NOW,
            tag_name: NEWER.to_string(),
            etag: None,
            suggested: None,
        },
    )
    .unwrap();
    let outcome = run_startup(&root, &surroundings, "sess-2");
    let (_, system_message) = session_start_response(&outcome);
    assert!(system_message.as_deref().unwrap().contains(NEWER));
    assert_eq!(spawns.get(), 1, "a fresh cache spawns nothing more");
}

/// The refresh is spawned only when the cache is missing or older than 24 h,
/// only on `startup`, and only when a pin exists to compare against.
#[test]
fn the_refresh_is_spawned_only_when_stale_on_startup_with_a_pin() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    let lock = tempfile::tempdir().unwrap();

    // Fresh cache: no spawn.
    let fresh = cache_dir_with(&format!("ss-magic-plugin-v{PIN}"), 60);
    let (surroundings, spawns) = watched(&plugin, &fresh, &lock);
    run_startup(&root, &surroundings, "sess-1");
    assert_eq!(spawns.get(), 0, "fresh cache");

    // Stale cache: spawn.
    let stale = cache_dir_with(&format!("ss-magic-plugin-v{PIN}"), 25 * 60 * 60);
    let (surroundings, spawns) = watched(&plugin, &stale, &lock);
    run_startup(&root, &surroundings, "sess-2");
    assert_eq!(spawns.get(), 1, "stale cache");

    // Stale cache, but not a startup: no spawn, and no notice either.
    let (surroundings, spawns) = watched(&plugin, &stale, &lock);
    for source in ["resume", "clear", "compact", "fork"] {
        let event = HookEvent::SessionStart;
        let config = PluginConfig::default();
        let envelope = envelope_for(&root, source, "sess-3");
        let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);
        let outcome = handle_with(&ctx, &surroundings).unwrap();
        let (_, system_message) = session_start_response(&outcome);
        assert_eq!(*system_message, None, "source `{source}`");
        assert!(
            !outcome.detail.as_deref().unwrap().contains("release"),
            "source `{source}` records nothing about releases: {:?}",
            outcome.detail
        );
    }
    assert_eq!(spawns.get(), 0, "non-startup sources");

    // No pin (not an installed plugin): no spawn.
    let (mut surroundings, spawns) = watched(&plugin, &stale, &lock);
    surroundings.plugin_root = None;
    run_startup(&root, &surroundings, "sess-4");
    assert_eq!(spawns.get(), 0, "no pin");
}

/// A spawner that fails is a heartbeat note, never a failure of the hook.
#[test]
fn a_failed_spawn_is_recorded_not_raised() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    let cache = tempfile::tempdir().unwrap();
    let lock = tempfile::tempdir().unwrap();
    let (mut surroundings, _) = watched(&plugin, &cache, &lock);
    surroundings.spawn_refresh = Box::new(|| Err("spawn: boom".to_string()));
    let outcome = run_startup(&root, &surroundings, "sess-1");
    assert!(
        outcome
            .detail
            .as_deref()
            .unwrap()
            .contains("release refresh not spawned (spawn: boom)"),
        "{:?}",
        outcome.detail
    );
}

/// The three notices share one `systemMessage`, and the suggestion changes
/// nothing about `additionalContext` — byte-for-byte the U3 output.
#[test]
fn the_suggestion_rides_system_message_and_leaves_additional_context_unchanged() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning("0.0.1"); // also triggers the drift notice
    let cache = cache_dir_with(NEWER, 60);
    let lock = tempfile::tempdir().unwrap();
    let (surroundings, _) = watched(&plugin, &cache, &lock);

    let event = HookEvent::SessionStart;
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "startup", "sess-1");
    let ctx = ctx_for(&event, &envelope, Some(root.clone()), &config);

    let baseline = handle_inert(&ctx).unwrap();
    let with_notices = handle_with(&ctx, &surroundings).unwrap();
    let (baseline_context, baseline_message) = session_start_response(&baseline);
    let (context, message) = session_start_response(&with_notices);

    assert_eq!(*baseline_message, None);
    assert_eq!(context, baseline_context, "additionalContext is unchanged");
    let message = message.as_deref().unwrap();
    assert!(message.contains("pins v0.0.1"), "drift notice: {message}");
    assert!(message.contains(NEWER), "suggestion: {message}");
    assert!(message.contains("\n\n"), "a blank line apart: {message}");
}

/// Contention on the release cache's lock withholds the notice for this
/// session rather than announcing without recording — the record is what
/// keeps the promise, so no record means no notice.
#[test]
fn the_suggestion_is_withheld_while_the_release_cache_is_locked() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    let cache = cache_dir_with(NEWER, 60);
    let lock = tempfile::tempdir().unwrap();
    let (surroundings, _) = watched(&plugin, &cache, &lock);

    let outcome = crate::tmproot::with_lock(lock.path(), crate::release_check::LOCK_NAME, || {
        run_startup(&root, &surroundings, "sess-1")
    })
    .unwrap();
    let (_, system_message) = session_start_response(&outcome);
    assert_eq!(*system_message, None);
    assert!(
        outcome.detail.as_deref().unwrap().contains("release suggestion deferred"),
        "{:?}",
        outcome.detail
    );
    assert_eq!(read_release_cache(&cache).suggested, None);

    // Once the lock is free, the next startup announces.
    let outcome = run_startup(&root, &surroundings, "sess-2");
    let (_, system_message) = session_start_response(&outcome);
    assert!(system_message.as_deref().unwrap().contains(NEWER));
}

/// Without a private lock root there is nowhere safe to record the notice,
/// so it is withheld rather than written unlocked.
#[test]
fn the_suggestion_is_withheld_without_a_lock_root() {
    let (_dir, root) = ignored_repo();
    let plugin = plugin_root_pinning(PIN);
    let cache = cache_dir_with(NEWER, 60);
    let lock = tempfile::tempdir().unwrap();
    let (mut surroundings, _) = watched(&plugin, &cache, &lock);
    surroundings.lock_root = Box::new(|| None);
    let outcome = run_startup(&root, &surroundings, "sess-1");
    let (_, system_message) = session_start_response(&outcome);
    assert_eq!(*system_message, None);
    assert!(
        outcome.detail.as_deref().unwrap().contains("no private lock root"),
        "{:?}",
        outcome.detail
    );
}
