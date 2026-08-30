//! Drives [`handle`] directly against a constructed [`HookContext`] rather
//! than the full pipeline (U11's `hook/tests.rs` already covers the gates
//! that sit in front of every handler) — these tests are about what THIS
//! handler does once it is reached: the guidance text, the version-drift
//! notice, and the R15 outside-a-repository case.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::git;
use crate::plugin::config::PluginConfig;
use crate::plugin::hook::event::{Common, Envelope, Payload, SessionStart};
use crate::plugin::HookEvent;
use crate::tests::support::{git_run, init_main_repo, neutralize_global_excludes};

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
        repo_root,
        config_root,
        config,
        now: NOW,
        diagnostics: RefCell::new(Vec::new()),
    }
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
    let route = crate::plugin::hook::route(&HookEvent::SessionStart).unwrap();
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
        let outcome = handle(&ctx).unwrap();
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

    let outcome = handle(&ctx).unwrap();
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

    let outcome = handle(&ctx).unwrap();
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

    let outcome = handle(&ctx).unwrap();
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

    let outcome = handle(&ctx).unwrap();
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
    handle(&ctx).unwrap();

    let report = scratchpad::ensure(&root).unwrap();
    let status_path = report.session_dir.join("STATUS.md");
    let custom = "# Status\n\nmy own notes from earlier in this session\n";
    fs::write(&status_path, custom).unwrap();

    let second = envelope_for(&root, "compact", "sess-1");
    let ctx2 = ctx_for(&event, &second, Some(root.clone()), &config);
    let outcome = handle(&ctx2).unwrap();

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

    let outcome = handle(&ctx).unwrap();
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
    fs::write(dir.path().join("ss-magic.version"), "0.0.1\n").unwrap();

    let notice = version_drift_notice(Some(dir.path().to_path_buf()))
        .expect("a differing pin must produce a notice");
    assert!(notice.contains("0.0.1"), "{notice}");
    assert!(notice.contains(env!("CARGO_PKG_VERSION")), "{notice}");
}

#[test]
fn version_drift_notice_is_silent_on_a_match() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("ss-magic.version"),
        format!("{}\n", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();

    assert_eq!(version_drift_notice(Some(dir.path().to_path_buf())), None);
}

#[test]
fn version_drift_notice_is_silent_with_no_plugin_root() {
    assert_eq!(version_drift_notice(None), None);
}

#[test]
fn version_drift_notice_is_silent_when_the_pin_file_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(version_drift_notice(Some(dir.path().to_path_buf())), None);
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
    // Belt and braces: no line may start with the bare binary name, which is
    // what a careless edit would reintroduce.
    assert!(
        !CHECKLIST_VERBS
            .lines()
            .any(|l| l.trim().starts_with("ss-magic plugin")),
        "the guidance must not name a bare `ss-magic`"
    );
}
