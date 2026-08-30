//! Drives [`handle_in_store`] directly against a constructed [`HookContext`].
//! The gates in front of every handler — enablement, the decodable envelope,
//! the `cwd` that has to exist — are `hook/tests.rs`'s subject; these tests are
//! about what this handler does once it is reached, and above all about the
//! ways it must decline to do anything without failing.
//!
//! The ledger's own arithmetic, idempotency and concurrency live in
//! `ledger/tests.rs`. What is proved here is the wiring: the right transcript,
//! the right session id, the right root label, and a silent success on every
//! path there is.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::plugin::config::PluginConfig;
use crate::plugin::hook::event::{
    self, Common, Envelope, Response, SessionEnd as SessionEndPayload,
};
use crate::plugin::ledger;
use crate::plugin::HookEvent;

const NOW: u64 = 1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.
const EVENT: HookEvent = HookEvent::SessionEnd;

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// One assistant record with usage, enough for the ledger to have something to
/// count. The full-fidelity fixture lives in `ledger/tests.rs`.
fn usage_line(cwd: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "cwd": cwd,
        "gitBranch": "feature-x",
        "message": {
            "model": "claude-sonnet-5",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 1_000_000,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 0,
                    "ephemeral_1h_input_tokens": 0,
                },
            },
        },
    })
    .to_string()
}

/// A transcript for `session_id` in its own directory, returning the tempdir
/// (which the caller must hold) and the transcript path.
fn transcript(session_id: &str, cwd: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{session_id}.jsonl"));
    fs::write(&path, format!("{}\n", usage_line(cwd))).unwrap();
    (dir, path)
}

/// The envelope a real `SessionEnd` invocation carries.
fn envelope_for(session_id: &str, transcript_path: &str, cwd: &str, reason: &str) -> Envelope {
    Envelope {
        common: Common {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: "SessionEnd".to_string(),
            prompt_id: None,
        },
        payload: Payload::SessionEnd(SessionEndPayload {
            reason: reason.to_string(),
        }),
        raw: serde_json::json!({}),
    }
}

/// A `HookContext` built by hand rather than through the pipeline.
fn ctx_for<'a>(
    envelope: &'a Envelope,
    repo_root: Option<PathBuf>,
    config: &'a PluginConfig,
) -> HookContext<'a> {
    let config_root = repo_root
        .clone()
        .unwrap_or_else(|| PathBuf::from(&envelope.common.cwd));
    HookContext {
        event: &EVENT,
        envelope,
        repo_root,
        config_root,
        config,
        now: NOW,
        diagnostics: RefCell::new(Vec::new()),
    }
}

/// Every path through this handler must leave stdout empty. `SessionEnd` has
/// no model-facing channel at all — [`Response`] offers it no variant — so
/// running the real encoder over the outcome proves it rather than checking
/// it.
fn assert_silent(outcome: &Outcome) {
    assert_eq!(outcome.response, Response::Silent);
    assert_eq!(event::encode(&outcome.response).unwrap(), None);
}

// ── Routing ───────────────────────────────────────────────────────────────────

/// `route()` dispatches purely on the event name, so this is the one place the
/// wiring itself is asserted — including that `SessionEnd` is NOT a
/// state-writing event: it appends to the machine-level ledger, which lives
/// outside every worktree, so the ignored-tree gate does not apply to it.
#[test]
fn session_end_routes_through_this_handler_and_writes_no_state_tree() {
    let route = crate::plugin::hook::route(&HookEvent::SessionEnd).unwrap();
    assert_eq!(route.handler as *const (), handle as *const ());
    assert!(!route.writes_state);
}

// ── The happy path ────────────────────────────────────────────────────────────

/// The ordinary ending: one row, keyed on the session id, labeled with the
/// worktree root the wrapper already resolved, and nothing on the wire.
#[test]
fn an_ended_session_leaves_one_labeled_row() {
    let store = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let cwd = worktree.path().to_string_lossy().into_owned();
    let (_dir, path) = transcript("s-end", &cwd);

    let envelope = envelope_for("s-end", &path.to_string_lossy(), &cwd, "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, Some(worktree.path().to_path_buf()), &config);

    let outcome = handle_in_store(&ctx, store.path()).unwrap();
    assert_silent(&outcome);
    assert!(
        outcome
            .detail
            .as_deref()
            .unwrap()
            .contains("recorded s-end"),
        "{outcome:?}"
    );

    let rows = ledger::read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "s-end");
    assert_eq!(rows[0].root.as_deref(), Some(cwd.as_str()));
    assert_eq!(rows[0].branch.as_deref(), Some("feature-x"));
    assert_eq!(rows[0].tokens.output, 1_000_000);
}

/// AE11, through the handler rather than the ledger: `/clear` makes two
/// endings inside one CLI process the ordinary case, not an edge one. The
/// second says so in its heartbeat note and writes nothing.
#[test]
fn a_second_ending_for_one_session_id_writes_nothing() {
    let store = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let cwd = worktree.path().to_string_lossy().into_owned();
    let (_dir, path) = transcript("s-clear", &cwd);

    let envelope = envelope_for("s-clear", &path.to_string_lossy(), &cwd, "clear");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, Some(worktree.path().to_path_buf()), &config);

    handle_in_store(&ctx, store.path()).unwrap();
    let second = handle_in_store(&ctx, store.path()).unwrap();

    assert_silent(&second);
    assert!(
        second
            .detail
            .as_deref()
            .unwrap()
            .contains("already recorded"),
        "{second:?}"
    );
    assert_eq!(ledger::read(store.path()).unwrap().len(), 1);
}

/// A session that ended outside any git repository still gets a row — the
/// ledger groups on whatever the transcript's own `cwd` normalizes to, and a
/// missing repository root is not a reason to lose the session.
#[test]
fn a_session_outside_a_repository_is_still_recorded() {
    let store = tempfile::tempdir().unwrap();
    let (_dir, path) = transcript("s-norepo", "/tmp/not-a-repo");

    let envelope = envelope_for("s-norepo", &path.to_string_lossy(), "/tmp", "other");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, None, &config);

    assert_silent(&handle_in_store(&ctx, store.path()).unwrap());
    assert_eq!(ledger::read(store.path()).unwrap().len(), 1);
}

// ── Failing open ──────────────────────────────────────────────────────────────

/// The transcript the payload names is not there — the file was cleaned up, or
/// the session never wrote one. Silent success, a note for the heartbeat, no
/// row.
#[test]
fn a_missing_transcript_is_a_silent_success() {
    let store = tempfile::tempdir().unwrap();
    let envelope = envelope_for("s-missing", "/tmp/nowhere/session.jsonl", "/tmp", "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, None, &config);

    let outcome = handle_in_store(&ctx, store.path()).unwrap();
    assert_silent(&outcome);
    assert!(
        outcome.detail.as_deref().unwrap().contains("no transcript"),
        "{outcome:?}"
    );
    assert!(ledger::read(store.path()).unwrap().is_empty());
}

/// An envelope with no `transcript_path` at all — the field defaults to empty
/// rather than failing the decode, so the handler is what has to notice.
#[test]
fn an_envelope_without_a_transcript_path_is_a_silent_success() {
    let store = tempfile::tempdir().unwrap();
    let envelope = envelope_for("s-nopath", "", "/tmp", "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, None, &config);

    let outcome = handle_in_store(&ctx, store.path()).unwrap();
    assert_silent(&outcome);
    assert!(
        outcome
            .detail
            .as_deref()
            .unwrap()
            .contains("no transcript path"),
        "{outcome:?}"
    );
    assert!(ledger::read(store.path()).unwrap().is_empty());
}

/// With no session id there is no key to record under, and a row keyed on the
/// empty string would collide with every other such session. Nothing is
/// written.
#[test]
fn an_envelope_without_a_session_id_records_nothing() {
    let store = tempfile::tempdir().unwrap();
    let (_dir, path) = transcript("s-anon", "/tmp/p");
    let envelope = envelope_for("   ", &path.to_string_lossy(), "/tmp", "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, None, &config);

    let outcome = handle_in_store(&ctx, store.path()).unwrap();
    assert_silent(&outcome);
    assert!(
        outcome.detail.as_deref().unwrap().contains("no session id"),
        "{outcome:?}"
    );
    assert!(ledger::read(store.path()).unwrap().is_empty());
}

/// A transcript full of lines this build cannot parse still ends the session
/// quietly, with a row recording what little could be read.
#[test]
fn a_malformed_transcript_still_ends_the_session_quietly() {
    let store = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s-garbage.jsonl");
    fs::write(&path, "not json\n{\"type\":\"assistant\"\n").unwrap();

    let envelope = envelope_for("s-garbage", &path.to_string_lossy(), "/tmp", "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, None, &config);

    assert_silent(&handle_in_store(&ctx, store.path()).unwrap());
    let rows = ledger::read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tokens.output, 0);
}

/// The handler must never reach for the state tree: `SessionEnd` is not a
/// state-writing event, so nothing here may create `.superset/.magic/` in a
/// repository whose gitignore has not been prepared for it.
#[test]
fn nothing_is_written_into_the_worktree() {
    let store = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let cwd = worktree.path().to_string_lossy().into_owned();
    let (_dir, path) = transcript("s-clean", &cwd);

    let envelope = envelope_for("s-clean", &path.to_string_lossy(), &cwd, "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, Some(worktree.path().to_path_buf()), &config);
    handle_in_store(&ctx, store.path()).unwrap();

    assert!(!worktree.path().join(".superset").exists());
    assert_eq!(fs::read_dir(worktree.path()).unwrap().count(), 0);
}

/// The handler writes only into the store it is given — never into the
/// developer's own application data directory, and never anywhere else.
#[test]
fn only_the_given_store_is_written() {
    let store = tempfile::tempdir().unwrap();
    let (_dir, path) = transcript("s-scope", "/tmp/p");
    let envelope = envelope_for("s-scope", &path.to_string_lossy(), "/tmp", "exit");
    let config = PluginConfig::default();
    let ctx = ctx_for(&envelope, None, &config);
    handle_in_store(&ctx, store.path()).unwrap();

    let mut names: Vec<String> = fs::read_dir(store.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "cost.jsonl".to_string(),
            "cost.lock".to_string(),
            "prices".to_string(),
            "transcript-offsets.json".to_string(),
        ]
    );
    assert!(
        Path::new(&path).exists(),
        "the transcript is read, never moved"
    );
}
