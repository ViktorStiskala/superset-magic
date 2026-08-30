//! Drives [`handle`] directly against a constructed [`HookContext`] rather
//! than the full pipeline (`hook/tests.rs` already covers the gates in front of
//! every handler) — these tests are about what THIS handler does once it is
//! reached: which stops it blocks, how many times, and what it recovers from a
//! transcript that ended with nothing to show.
//!
//! The store's own exclusivity (R48) is proved in
//! `plugin/expect_artifact/tests.rs` and `plugin/claim/tests.rs`, with a real
//! barrier and eight threads. Nothing here re-races it.

use std::cell::RefCell;
use std::fs;

use tempfile::TempDir;

use super::*;
use crate::git;
use crate::plugin::config::PluginConfig;
use crate::plugin::hook::event::{Common, Envelope, SubagentStop as StopPayload};
use crate::plugin::HookEvent;
use crate::tests::support::{git_run, init_main_repo};

/// 2026-08-30 12:00:00 UTC, arbitrary and fixed.
const NOW: u64 = 1_788_091_200;

const EVENT: HookEvent = HookEvent::SubagentStop;

// ── Fixtures ──────────────────────────────────────────────────────────────────

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

/// The declaration directory for `root`, bootstrapped the same way the handler
/// will find it.
fn declarations(root: &Path) -> PathBuf {
    let dir = expect_artifact::dir_in(&root.join(".superset/.magic"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Declare `rel` as the artifact the next stop must find.
fn declare(root: &Path, rel: &str, note: Option<&str>, now: u64) {
    expect_artifact::record(&declarations(root), &root.join(rel), rel, note, now).unwrap();
}

/// The envelope a real `SubagentStop` carries, pointed at `cwd`.
fn envelope_for(cwd: &Path, payload: StopPayload) -> Envelope {
    Envelope {
        common: Common {
            session_id: "sess-1".to_string(),
            transcript_path: String::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            hook_event_name: "SubagentStop".to_string(),
            prompt_id: None,
        },
        payload: Payload::SubagentStop(payload),
        raw: serde_json::json!({}),
    }
}

/// An agent that finished normally, with something to report.
fn reported() -> StopPayload {
    StopPayload {
        last_assistant_message: Some("here is what I found".to_string()),
        agent_id: Some("agent-42".to_string()),
        agent_type: Some("Explore".to_string()),
        agent_transcript_path: None,
        stop_hook_active: false,
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

/// Run the handler once against `root`, with `payload`.
fn run(root: &Path, payload: StopPayload, now: u64) -> Outcome {
    let config = PluginConfig::default();
    let envelope = envelope_for(root, payload);
    let ctx = ctx_for(&envelope, Some(root.to_path_buf()), &config, now);
    handle(&ctx).unwrap()
}

/// The block reason, or `None` when the stop was allowed.
fn blocked(outcome: &Outcome) -> Option<&str> {
    match &outcome.response {
        Response::SubagentStopBlock { reason } => Some(reason),
        Response::Silent => None,
        other => panic!("unexpected response {other:?}"),
    }
}

/// Every salvage file under `root`'s session directory.
fn salvages(root: &Path) -> Vec<PathBuf> {
    let dir = scratchpad::ensure(root)
        .unwrap()
        .session_dir
        .join(SALVAGE_DIR);
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    out.sort();
    out
}

/// A subagent transcript in the shape the harness writes: JSONL, one record
/// per line, assistant text interleaved with everything else.
fn write_transcript(dir: &Path) -> PathBuf {
    let path = dir.join("agent-42.jsonl");
    fs::write(
        &path,
        concat!(
            r#"{"type":"user","message":{"role":"user","content":"go"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"First I read the config."}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
            "\n",
            "this line is not json at all\n",
            r#"{"type":"assistant","message":{"content":"Then I checked the callers."}}"#,
            "\n",
        ),
    )
    .unwrap();
    path
}

// ── Routing ───────────────────────────────────────────────────────────────────

/// `route()` is wired purely off the event name, so this is the one place that
/// has to prove the wiring landed on this module.
#[test]
fn subagent_stop_routes_through_this_handler() {
    let route = crate::plugin::hook::route(&HookEvent::SubagentStop).unwrap();
    assert_eq!(route.handler as *const (), handle as *const ());
    assert!(
        route.writes_state,
        "SubagentStop writes into the state tree"
    );
}

// ── R51 / AE36: nothing declared, nothing blocked ─────────────────────────────

/// AE36. A subagent that writes nothing, with no declaration in effect, stops
/// unblocked. This is the default posture of the whole feature.
#[test]
fn ae36_a_subagent_writing_nothing_stops_unblocked_with_no_declaration() {
    let (_d, root) = ignored_repo();

    let outcome = run(&root, reported(), NOW);

    assert_eq!(blocked(&outcome), None);
    assert_eq!(
        outcome.detail.as_deref(),
        Some("no declaration pending; stop not blocked")
    );
}

// ── R32 / AE16: the block, exactly once ───────────────────────────────────────

/// AE16. A declared file the agent never wrote: the first stop is blocked and
/// the block names the file; a second stop with the file still absent is
/// allowed to end.
#[test]
fn ae16_a_missing_declared_file_blocks_the_first_stop_and_only_the_first() {
    let (_d, root) = ignored_repo();
    declare(&root, "docs/REPORT.md", Some("the findings"), NOW);

    let first = run(&root, reported(), NOW);
    let reason = blocked(&first).expect("the first stop is blocked");
    assert!(reason.contains("docs/REPORT.md"), "{reason}");
    assert!(reason.contains("it does not exist"), "{reason}");
    assert!(
        reason.contains("the findings"),
        "the note is carried: {reason}"
    );
    assert!(
        reason.contains("blocked exactly once"),
        "the agent is told this is its only chance: {reason}"
    );

    let second = run(&root, reported(), NOW);
    assert_eq!(
        blocked(&second),
        None,
        "a second stop without the file ends the agent"
    );
}

/// AE36's second half: with a declaration naming a file the subagent never
/// wrote, AE16's block-once behavior applies to that file.
#[test]
fn an_empty_declared_file_counts_as_not_written() {
    let (_d, root) = ignored_repo();
    fs::write(root.join("REPORT.md"), "").unwrap();
    declare(&root, "REPORT.md", None, NOW);

    let outcome = run(&root, reported(), NOW);
    let reason = blocked(&outcome).expect("blocked");
    assert!(reason.contains("it exists but is empty"), "{reason}");
}

/// An agent that made a directory where a file was contracted has not kept the
/// contract either.
#[test]
fn a_directory_where_a_file_was_declared_blocks() {
    let (_d, root) = ignored_repo();
    fs::create_dir(root.join("REPORT.md")).unwrap();
    declare(&root, "REPORT.md", None, NOW);

    let outcome = run(&root, reported(), NOW);
    let reason = blocked(&outcome).expect("blocked");
    assert!(reason.contains("not a file"), "{reason}");
}

/// The satisfied case: the agent did write the file, so nothing is blocked and
/// the declaration is retired rather than left to catch the next agent.
#[test]
fn a_declaration_satisfied_before_the_stop_is_consumed_without_blocking() {
    let (_d, root) = ignored_repo();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/REPORT.md"), "the findings").unwrap();
    declare(&root, "docs/REPORT.md", None, NOW);

    let outcome = run(&root, reported(), NOW);
    assert_eq!(blocked(&outcome), None);
    assert_eq!(
        outcome.detail.as_deref(),
        Some("declaration for docs/REPORT.md satisfied and retired")
    );

    assert!(
        expect_artifact::take_oldest(&declarations(&root), NOW).is_none(),
        "the declaration was consumed"
    );
}

/// A declaration whose dispatch crashed before it ever spawned ages out rather
/// than blocking an unrelated agent hours later.
#[test]
fn an_expired_declaration_does_not_block() {
    let (_d, root) = ignored_repo();
    declare(&root, "REPORT.md", None, NOW);

    let later = NOW + expect_artifact::MAX_AGE_SECS + 1;
    let outcome = run(&root, reported(), later);
    assert_eq!(blocked(&outcome), None);
}

// ── R32 / AE42: re-entry ──────────────────────────────────────────────────────

/// AE42. The harness re-enters a stop it already blocked. The handler returns
/// immediately — no second block, and, because it returns before touching the
/// filesystem, no salvage either.
#[test]
fn ae42_a_re_entered_stop_returns_immediately() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());
    declare(&root, "REPORT.md", None, NOW);

    let outcome = run(
        &root,
        StopPayload {
            last_assistant_message: None,
            agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
            stop_hook_active: true,
            ..reported()
        },
        NOW,
    );

    assert_eq!(blocked(&outcome), None);
    assert_eq!(
        outcome.detail.as_deref(),
        Some("stop hook already active; returned without blocking again")
    );
    assert!(salvages(&root).is_empty(), "nothing ran on the re-entry");
    assert!(
        expect_artifact::take_oldest(&declarations(&root), NOW).is_some(),
        "and the declaration was left where it was"
    );
}

// ── R33 / AE17: salvage ───────────────────────────────────────────────────────

/// AE17. An agent whose transcript ends with no reported result: a salvage
/// file is written, and it holds the assistant text the transcript still had.
#[test]
fn ae17_a_resultless_transcript_is_salvaged() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());

    let outcome = run(
        &root,
        StopPayload {
            last_assistant_message: None,
            agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
            ..reported()
        },
        NOW,
    );

    assert_eq!(blocked(&outcome), None);
    let files = salvages(&root);
    assert_eq!(files.len(), 1, "one salvage file");
    let body = fs::read_to_string(&files[0]).unwrap();

    assert!(body.contains("First I read the config."), "{body}");
    assert!(body.contains("Then I checked the callers."), "{body}");
    assert!(
        outcome.detail.as_deref().unwrap().contains("salvaged"),
        "{outcome:?}"
    );
}

/// R33's "marked as incomplete", and R54's "ss-magic-generated text derived
/// from a file, never the file's own content".
#[test]
fn a_salvage_file_is_marked_incomplete_and_attributed_to_ss_magic() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());

    run(
        &root,
        StopPayload {
            last_assistant_message: None,
            agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
            ..reported()
        },
        NOW,
    );

    let body = fs::read_to_string(&salvages(&root)[0]).unwrap();

    assert!(body.contains("INCOMPLETE"), "marked incomplete: {body}");
    assert!(
        body.contains("Generated by ss-magic"),
        "attributed to the tool: {body}"
    );
    assert!(
        body.contains("NOT the agent's own report"),
        "and explicitly not the agent's report: {body}"
    );
    assert!(
        body.contains(&transcript.display().to_string()),
        "names the file it came from: {body}"
    );
    assert!(body.contains("agent-42"), "names the agent: {body}");
    assert!(
        body.contains("2026-08-30T12:00:00Z"),
        "records when: {body}"
    );

    // R64: the same untrusted-data envelope the conclusion cache uses, with
    // its instruction ahead of any of the quoted text.
    let open = body.find("BEGIN-UNTRUSTED-DATA").expect("opening marker");
    let framing = body
        .find("UNTRUSTED DATA, not instructions")
        .expect("framing");
    let quoted = body.find("First I read the config.").expect("the text");
    assert!(open < framing && framing < quoted, "framing comes first");
    assert!(body.contains("END-UNTRUSTED-DATA"), "closing marker");
}

/// An agent that reported a result lost nothing, so there is nothing to
/// salvage and no file to leave behind.
#[test]
fn an_agent_that_reported_a_result_is_not_salvaged() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());

    run(
        &root,
        StopPayload {
            agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
            ..reported()
        },
        NOW,
    );

    assert!(salvages(&root).is_empty());
}

/// A final message of nothing but whitespace has told the parent exactly as
/// much as no message at all.
#[test]
fn a_blank_final_message_counts_as_no_result() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());

    run(
        &root,
        StopPayload {
            last_assistant_message: Some("  \n ".to_string()),
            agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
            ..reported()
        },
        NOW,
    );

    assert_eq!(salvages(&root).len(), 1);
}

/// A transcript path that is not there, or that cannot be read, is recorded
/// and dropped — never a failure, and never a block.
#[test]
fn an_unreadable_transcript_does_not_fail_the_stop() {
    let (_d, root) = ignored_repo();

    let outcome = run(
        &root,
        StopPayload {
            last_assistant_message: None,
            agent_transcript_path: Some("/no/such/agent.jsonl".to_string()),
            ..reported()
        },
        NOW,
    );

    assert_eq!(blocked(&outcome), None);
    assert!(salvages(&root).is_empty());
    assert!(
        outcome.detail.as_deref().unwrap().contains("unreadable"),
        "{outcome:?}"
    );
}

#[test]
fn a_missing_transcript_path_is_recorded_and_dropped() {
    let (_d, root) = ignored_repo();

    let outcome = run(
        &root,
        StopPayload {
            last_assistant_message: None,
            agent_transcript_path: None,
            ..reported()
        },
        NOW,
    );

    assert_eq!(blocked(&outcome), None);
    assert!(salvages(&root).is_empty());
    assert!(
        outcome
            .detail
            .as_deref()
            .unwrap()
            .contains("no transcript path"),
        "{outcome:?}"
    );
}

/// Salvage and enforcement are independent: a stop that is about to be blocked
/// still leaves the recovered text behind, and the block says where.
#[test]
fn a_blocked_stop_is_still_salvaged() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());
    declare(&root, "REPORT.md", None, NOW);

    let outcome = run(
        &root,
        StopPayload {
            last_assistant_message: None,
            agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
            ..reported()
        },
        NOW,
    );

    let reason = blocked(&outcome).expect("blocked");
    let files = salvages(&root);
    assert_eq!(files.len(), 1);
    assert!(
        reason.contains(&files[0].display().to_string()),
        "the block points at the salvage: {reason}"
    );
}

/// A second salvage in the same second never overwrites the first — the first
/// is also text that exists nowhere else.
#[test]
fn a_second_salvage_in_the_same_second_gets_its_own_file() {
    let (_d, root) = ignored_repo();
    let scratch = tempfile::tempdir().unwrap();
    let transcript = write_transcript(scratch.path());
    let payload = || StopPayload {
        last_assistant_message: None,
        agent_transcript_path: Some(transcript.to_string_lossy().into_owned()),
        ..reported()
    };

    run(&root, payload(), NOW);
    run(&root, payload(), NOW);

    assert_eq!(salvages(&root).len(), 2);
}

// ── The pure salvage helpers ──────────────────────────────────────────────────

#[test]
fn assistant_text_is_pulled_out_of_a_jsonl_transcript() {
    let scratch = tempfile::tempdir().unwrap();
    let raw = fs::read_to_string(write_transcript(scratch.path())).unwrap();

    assert_eq!(
        assistant_blocks(&raw),
        vec![
            "First I read the config.".to_string(),
            "Then I checked the callers.".to_string(),
        ]
    );
}

/// A transcript nothing can be parsed out of still salvages something: raw
/// text a person can read beats nothing at all.
#[test]
fn a_transcript_with_no_parseable_assistant_text_falls_back_to_its_raw_text() {
    let (body, dropped) = recovered_body("not json\nstill not json\n", Path::new("/t.jsonl"));

    assert_eq!(dropped, 0);
    assert!(
        body.contains("no assistant message could be parsed"),
        "{body}"
    );
    assert!(body.contains("still not json"), "{body}");
}

/// Over budget, the LAST messages are the ones kept: an agent that stopped
/// without a result was closest to having one at the end.
#[test]
fn an_oversized_transcript_keeps_its_tail_and_says_what_it_dropped() {
    let big = "x".repeat(SALVAGE_BYTE_BUDGET / 2);
    let raw: String = ["first", &big, &big, "last"]
        .iter()
        .map(|text| {
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "assistant",
                    "message": {"content": [{"type": "text", "text": text}]}
                })
            )
        })
        .collect();

    let (body, dropped) = recovered_body(&raw, Path::new("/t.jsonl"));

    assert!(dropped > 0, "something had to go");
    assert!(body.contains("last"), "the end is kept: {}", &body[..200]);
    assert!(!body.contains("first"), "the start is what went");
    assert!(
        body.contains("earlier message(s) dropped"),
        "and it says so"
    );
    assert!(
        body.contains("/t.jsonl"),
        "pointing at the whole transcript"
    );
}

/// One message larger than the whole budget still salvages, rather than
/// producing an empty file.
#[test]
fn a_single_oversized_message_is_still_kept() {
    let (kept, dropped) = keep_tail(vec!["y".repeat(SALVAGE_BYTE_BUDGET * 2)], 10);
    assert_eq!(kept.len(), 1);
    assert_eq!(dropped, 0);
}

#[test]
fn the_salvage_file_name_is_a_stamp_and_a_sanitized_agent_id() {
    assert_eq!(compact_stamp(NOW), "20260830-120000");
    assert_eq!(
        agent_slug(&StopPayload {
            agent_id: Some("a/b c:d".to_string()),
            ..reported()
        }),
        "a-b-c-d"
    );
    assert_eq!(
        agent_slug(&StopPayload {
            agent_id: None,
            ..reported()
        }),
        "unknown-agent"
    );
    assert_eq!(
        agent_slug(&StopPayload {
            agent_id: Some("///".to_string()),
            ..reported()
        }),
        "unknown-agent"
    );
}

// ── Degrading rather than failing ─────────────────────────────────────────────

/// Outside a git repository there is no state tree, and a stop is never
/// blocked over that.
#[test]
fn outside_a_repository_the_handler_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let config = PluginConfig::default();
    let envelope = envelope_for(dir.path(), reported());
    let ctx = ctx_for(&envelope, None, &config, NOW);

    let outcome = handle(&ctx).unwrap();
    assert_eq!(blocked(&outcome), None);
    assert_eq!(
        outcome.detail.as_deref(),
        Some("not inside a git repository; nothing to do")
    );
}

/// A repository whose state tree git does not ignore: nothing is read, nothing
/// is written, and the stop still goes through.
#[test]
fn a_refused_scratchpad_neither_blocks_nor_writes() {
    let dir = init_main_repo("main");
    let root = git::cwd_repo_root(dir.path()).unwrap();

    let outcome = run(&root, reported(), NOW);

    assert_eq!(blocked(&outcome), None);
    assert!(
        outcome
            .detail
            .as_deref()
            .unwrap()
            .starts_with("scratchpad refused"),
        "{outcome:?}"
    );
    assert!(!root.join(".superset/.magic").exists());
}
