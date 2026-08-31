//! Drives [`handle`] directly against a constructed [`HookContext`] rather
//! than the full pipeline (`hook/tests.rs` already covers the gates that sit
//! in front of every handler) — these tests are about what THIS handler does
//! once it is reached: the note it writes, where it writes it, and the R49
//! guarantee that nothing here can ever block or print.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::git;
use crate::plugin::config::PluginConfig;
use crate::plugin::hook::event::{
    self, Common, Envelope, PreCompact as PreCompactPayload, Response,
};
use crate::plugin::HookEvent;
use crate::tests::support::{git_run, init_main_repo};

const NOW: u64 = 1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.

// ── Fixtures ───────────────────────────────────────────────────────────────

/// A repo whose `.gitignore` already covers the state tree — the ordinary
/// case, after `init`/`migrate` or `plugin enable` has run.
fn ignored_repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    fs::write(
        dir.path().join(".gitignore"),
        "target/\n.superset/.magic/\n",
    )
    .unwrap();
    git_run(&["add", ".gitignore"], dir.path());
    git_run(&["commit", "-q", "-m", "gitignore"], dir.path());
    let root = git::cwd_repo_root(dir.path()).unwrap();
    (dir, root)
}

/// The session directory `scratchpad::ensure` resolves for `root` — computed
/// by calling the very same function the handler itself calls, rather than
/// re-deriving the layout, so a test never drifts from what `ensure` actually
/// does. Idempotent: re-running it here before or after `handle` scaffolds
/// nothing new and rewrites nothing that already exists.
fn session_dir_for(root: &Path) -> PathBuf {
    scratchpad::ensure(root).unwrap().session_dir
}

/// The envelope a real `PreCompact` invocation carries, pointed at `cwd`.
fn envelope_for(cwd: &Path, trigger: &str, custom_instructions: Option<&str>) -> Envelope {
    Envelope {
        common: Common {
            session_id: "sess-1".to_string(),
            transcript_path: String::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            hook_event_name: "PreCompact".to_string(),
            prompt_id: None,
        },
        payload: Payload::PreCompact(PreCompactPayload {
            trigger: trigger.to_string(),
            custom_instructions: custom_instructions.map(str::to_string),
        }),
        raw: serde_json::json!({}),
    }
}

/// A `HookContext` built by hand rather than through the pipeline — this
/// module tests the handler in isolation, not the gates in front of it.
fn ctx_for<'a>(
    envelope: &'a Envelope,
    repo_root: Option<PathBuf>,
    config: &'a PluginConfig,
    now: u64,
) -> HookContext<'a> {
    let config_root = repo_root.clone().unwrap_or_else(|| PathBuf::from("/"));
    HookContext {
        event: &EVENT,
        envelope,
        repo_root,
        config_root,
        config,
        now,
        diagnostics: RefCell::new(Vec::new()),
    }
}

const EVENT: HookEvent = HookEvent::PreCompact;

// ── Routing ──────────────────────────────────────────────────────────────────

/// U11 wires `route()` purely off the event name, not the payload, so this is
/// the one place that has to prove the wiring landed on this module.
#[test]
fn pre_compact_routes_through_this_handler() {
    let route = crate::plugin::hook::route(&HookEvent::PreCompact).unwrap();
    assert_eq!(route.handler as *const (), handle as *const ());
    assert!(route.writes_state, "PreCompact writes into the state tree");
}

// ── R49: never a block decision, never any stdout ─────────────────────────────

/// The strongest available proof: the handler's own return type has no
/// variant capable of expressing a block or any other wire content for this
/// event (see the module docs), so asserting `Response::Silent` and then
/// running the real encoder is a structural guarantee, not a spot check.
fn assert_silent_and_wire_empty(outcome: &Outcome) {
    assert_eq!(outcome.response, Response::Silent);
    assert_eq!(event::encode(&outcome.response).unwrap(), None);
}

/// AE34. An `auto` trigger, with a scratchpad that already carries stale
/// model content from an earlier turn, is never blocked: the note is written
/// and the wire stays empty regardless of what the scratchpad already held.
#[test]
fn ae34_an_auto_trigger_with_stale_scratchpad_state_is_never_blocked() {
    let (_dir, root) = ignored_repo();
    let config = PluginConfig::default();

    // Seed a stale scratchpad by running once already.
    let first = envelope_for(&root, "auto", None);
    let ctx = ctx_for(&first, Some(root.clone()), &config, NOW);
    handle(&ctx).unwrap();
    let session_dir = session_dir_for(&root);
    let status_path = session_dir.join("STATUS.md");
    fs::write(
        &status_path,
        "# Status\n\nstale, from a much earlier turn\n",
    )
    .unwrap();

    let second = envelope_for(&root, "auto", None);
    let ctx2 = ctx_for(&second, Some(root.clone()), &config, NOW + 60);
    let outcome = handle(&ctx2).unwrap();

    assert_silent_and_wire_empty(&outcome);
    assert_eq!(
        fs::read_to_string(&status_path).unwrap(),
        "# Status\n\nstale, from a much earlier turn\n",
        "PreCompact must never touch the model's own state files"
    );
    let note = fs::read_to_string(session_dir.join("PRE-COMPACT.md")).unwrap();
    assert!(note.contains("auto"), "{note}");
}

/// AE41. A `manual` `/compact`, with the user's own custom instructions,
/// records them and lets the compaction proceed untouched.
#[test]
fn ae41_a_manual_trigger_records_custom_instructions_and_never_blocks() {
    let (_dir, root) = ignored_repo();
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "manual", Some("focus on the auth module next"));
    let ctx = ctx_for(&envelope, Some(root.clone()), &config, NOW);

    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    let note = fs::read_to_string(session_dir_for(&root).join("PRE-COMPACT.md")).unwrap();
    assert!(note.contains("manual"), "{note}");
    assert!(note.contains("focus on the auth module next"), "{note}");
}

/// The hook cannot tell whether a compaction actually followed — a `/compact`
/// on a session too small to compact still fires it — so the note is written
/// unconditionally rather than only on a "real" compaction the payload cannot
/// distinguish anyway.
#[test]
fn a_manual_trigger_with_no_compaction_following_still_writes_the_note() {
    let (_dir, root) = ignored_repo();
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "manual", None);
    let ctx = ctx_for(&envelope, Some(root.clone()), &config, NOW);

    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    assert!(session_dir_for(&root).join("PRE-COMPACT.md").exists());
}

/// A trigger value this build has never seen (a harness ahead of this
/// binary) is recorded verbatim rather than causing a failure or a block.
#[test]
fn an_unrecognized_trigger_is_recorded_verbatim_and_never_blocks() {
    let (_dir, root) = ignored_repo();
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "some-future-trigger", None);
    let ctx = ctx_for(&envelope, Some(root.clone()), &config, NOW);

    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    let note = fs::read_to_string(session_dir_for(&root).join("PRE-COMPACT.md")).unwrap();
    assert!(note.contains("some-future-trigger"), "{note}");
    assert!(outcome
        .detail
        .as_deref()
        .unwrap()
        .contains("some-future-trigger"));
}

// ── R15: outside a git repository ─────────────────────────────────────────────

#[test]
fn outside_a_git_repository_writes_nothing_and_stays_silent() {
    let dir = tempfile::tempdir().unwrap(); // deliberately not a git repository
    let config = PluginConfig::default();
    let envelope = envelope_for(dir.path(), "auto", None);
    let ctx = ctx_for(&envelope, None, &config, NOW);

    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    assert!(!dir.path().join(".superset").exists());
}

// ── A refused scratchpad (R63/R49 interaction) ────────────────────────────────

/// When `ensure` refuses outright (no ignore rule for the state tree yet),
/// the note must not be written — there is no session directory to write it
/// into — and the compaction must still proceed with an empty wire.
#[test]
fn a_refused_scratchpad_writes_no_note_and_never_blocks() {
    let dir = init_main_repo("main"); // no `.superset/.magic/` ignore rule at all
    let root = git::cwd_repo_root(dir.path()).unwrap();
    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "auto", None);
    let ctx = ctx_for(&envelope, Some(root.clone()), &config, NOW);

    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    assert!(!root.join(scratchpad::STATE_REL).exists());
    let detail = outcome.detail.unwrap();
    assert!(detail.contains("refused"), "{detail}");
}

// ── Repeated compactions accumulate ───────────────────────────────────────────

/// Three compactions in one session must leave three entries, oldest first,
/// with none of the earlier ones altered — this is the log's whole point.
#[test]
fn repeated_compactions_accumulate_rather_than_replace() {
    let (_dir, root) = ignored_repo();
    let config = PluginConfig::default();

    for (trigger, at) in [("auto", NOW), ("manual", NOW + 60), ("auto", NOW + 120)] {
        let envelope = envelope_for(&root, trigger, None);
        let ctx = ctx_for(&envelope, Some(root.clone()), &config, at);
        let outcome = handle(&ctx).unwrap();
        assert_silent_and_wire_empty(&outcome);
    }

    let note = fs::read_to_string(session_dir_for(&root).join("PRE-COMPACT.md")).unwrap();
    assert_eq!(
        note.matches("## ").count(),
        3,
        "expected three accumulated entries:\n{note}"
    );
    // The header is written exactly once, on the first compaction.
    assert_eq!(note.matches("Pre-compact log").count(), 1, "{note}");
}

/// A note file that has already grown large (an unrelated long-lived
/// session) is appended to, not replaced or truncated — the earlier bytes
/// must survive byte-for-byte.
#[test]
fn a_large_existing_note_file_is_appended_to_not_replaced() {
    let (_dir, root) = ignored_repo();
    let config = PluginConfig::default();

    let seed = envelope_for(&root, "auto", None);
    let ctx = ctx_for(&seed, Some(root.clone()), &config, NOW);
    handle(&ctx).unwrap();

    let note_path = session_dir_for(&root).join("PRE-COMPACT.md");
    let mut existing = fs::read_to_string(&note_path).unwrap();
    // Pad well past a single filesystem block, standing in for years of
    // accumulated entries.
    existing.push_str(&"filler content from earlier compactions\n".repeat(50_000));
    fs::write(&note_path, &existing).unwrap();

    let envelope = envelope_for(&root, "manual", None);
    let ctx = ctx_for(&envelope, Some(root.clone()), &config, NOW + 60);
    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    let after = fs::read_to_string(&note_path).unwrap();
    assert!(
        after.starts_with(&existing),
        "appending must never disturb what was already on disk"
    );
    assert!(after.len() > existing.len());
    assert_eq!(after.matches("manual").count(), 1, "{after}");
}

// ── The header TOCTOU (duplicate hooks racing the first write) ───────────────

/// The harness can spawn duplicate hooks for the same event (see `claim.rs`'s
/// module doc), so the first compaction of a session can be raced by more
/// than one invocation of this handler. Ten threads all calling
/// [`append_note`] against a log that does not exist yet, released at the
/// same instant, must still leave exactly one header — the old
/// `!path.exists()` check, made separately from the `open()` call, left a
/// window where every racing thread could see "not there yet" and each write
/// its own header.
#[test]
fn a_racing_first_write_never_duplicates_the_header() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().to_path_buf();
    const RACERS: usize = 10;

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(RACERS));
    let handles: Vec<_> = (0..RACERS)
        .map(|i| {
            let session_dir = session_dir.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                append_note(&session_dir, NOW + i as u64, "auto", None).unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let note = fs::read_to_string(session_dir.join(NOTE_NAME)).unwrap();
    assert_eq!(
        note.matches("Pre-compact log").count(),
        1,
        "a racing first write must produce exactly one header:\n{note}"
    );
    assert_eq!(
        note.matches("## ").count(),
        RACERS,
        "none of the racing entries may be lost:\n{note}"
    );
}

// ── R17: a tracked path under the state tree is never adopted ────────────────

/// A public repository could commit a file at this exact predictable path.
/// Just like the six model-owned state files, it must be left alone rather
/// than adopted as if it were ss-magic's own log.
#[test]
fn a_tracked_note_path_is_never_written_to() {
    let (_dir, root) = ignored_repo();
    let session_dir = session_dir_for(&root);
    fs::create_dir_all(&session_dir).unwrap();
    let planted = session_dir.join("PRE-COMPACT.md");
    fs::write(&planted, "planted by the repository\n").unwrap();
    let rel = planted
        .strip_prefix(&root)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    git_run(&["add", "-f", &rel], &root);
    git_run(&["commit", "-q", "-m", "planted note"], &root);

    let config = PluginConfig::default();
    let envelope = envelope_for(&root, "auto", None);
    let ctx = ctx_for(&envelope, Some(root.clone()), &config, NOW);
    let outcome = handle(&ctx).unwrap();

    assert_silent_and_wire_empty(&outcome);
    assert_eq!(
        fs::read_to_string(&planted).unwrap(),
        "planted by the repository\n",
        "a tracked path must never be adopted, even for ss-magic's own log"
    );
    let detail = outcome.detail.unwrap();
    assert!(detail.contains("tracked"), "{detail}");
}
