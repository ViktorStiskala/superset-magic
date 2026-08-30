//! Pipeline tests: what reaches stdout, what reaches stderr, what lands in the
//! heartbeat, and how the two gates keep a handler from ever running.
//!
//! The pipeline is driven through [`run_with_route`] with in-memory streams and
//! a tempdir store, so every assertion is about the real decode-gate-dispatch-
//! encode path rather than a re-implementation of it. The exit code is not
//! asserted per test: `run` has no expression that can produce a non-zero one,
//! which `the_entry_point_has_no_non_zero_exit` checks at the source level.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::plugin::heartbeat;
use crate::plugin::hook::event::Payload;
use crate::tests::support::{git_run, init_main_repo, make_worktree, write_file};

/// A fixed instant so heartbeat timestamps are stable: 2026-08-30 12:00:00 UTC.
const NOW: u64 = 1_788_091_200;

// ── Driving the pipeline ──────────────────────────────────────────────────────

/// Everything one invocation produced.
struct Run {
    stdout: String,
    stderr: String,
    row: Row,
    store: PathBuf,
}

impl Run {
    /// stdout parsed as the single JSON envelope it must be. Panics if stdout
    /// held anything other than exactly one line of JSON.
    fn envelope(&self) -> serde_json::Value {
        let trimmed = self.stdout.trim_end_matches('\n');
        assert!(
            !trimmed.is_empty() && !trimmed.contains('\n'),
            "stdout is not exactly one line: {:?}",
            self.stdout
        );
        serde_json::from_str(trimmed).expect("stdout does not parse as JSON")
    }

    /// The rows the heartbeat actually holds on disk.
    fn rows(&self) -> Vec<Row> {
        heartbeat::read(&self.store).unwrap()
    }
}

/// Run one invocation against `store`, with the routing decision supplied.
fn drive_at(store: &Path, event: &HookEvent, route: Option<Route>, input: &str) -> Run {
    let mut stdin = std::io::Cursor::new(input.as_bytes().to_vec());
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let row = {
        let mut io = HookIo {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
            store: Some(store.to_path_buf()),
        };
        run_with_route(event, route, &mut io, NOW)
    };
    Run {
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
        row,
        store: store.to_path_buf(),
    }
}

/// Run one invocation with a fresh store, using the production routing table.
fn drive(event: &HookEvent, input: &str) -> (TempDir, Run) {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("plugin");
    let run = drive_at(&store, event, route(event), input);
    (dir, run)
}

// ── Repository fixtures ───────────────────────────────────────────────────────

/// A repository whose state tree is ignored and whose `magic.json` turns the
/// plugin on — the ordinary case, after `ss-magic init` and `plugin enable`.
fn enabled_repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join(".gitignore"), "target/\n.superset/.magic/\n").unwrap();
    write_file(
        &root,
        ".superset/magic.json",
        r#"{"files":[],"plugin":{"enabled":true}}"#,
    );
    (dir, root)
}

/// The envelope a real invocation carries, pointed at `cwd`.
fn envelope(name: &str, cwd: &Path, extra: &str) -> String {
    let extra = extra.trim();
    let tail = if extra.is_empty() {
        String::new()
    } else {
        format!(",{extra}")
    };
    format!(
        r#"{{"session_id":"s-1","transcript_path":"/t/s-1.jsonl","cwd":{},"hook_event_name":"{name}"{tail}}}"#,
        serde_json::to_string(&cwd.to_string_lossy()).unwrap()
    )
}

// ── Test handlers ─────────────────────────────────────────────────────────────
//
// Handlers that fail, panic or talk to stderr. None of the six real handlers
// may do any of this, which is exactly why they cannot be used to test the
// wrapper's behavior when one does.

fn route_to(handler: Handler, writes_state: bool) -> Option<Route> {
    Some(Route {
        module: "test",
        handler,
        writes_state,
    })
}

fn handler_ok(_ctx: &HookContext<'_>) -> Result<Outcome> {
    Ok(Outcome::silent().with_detail("did the work"))
}

fn handler_denies(_ctx: &HookContext<'_>) -> Result<Outcome> {
    Ok(
        Outcome::new(Response::PreToolUse(event::PreToolUseResponse {
            decision: Some(event::PermissionDecision::Deny),
            reason: Some("read the conclusion instead".into()),
            ..Default::default()
        }))
        .with_detail("denied"),
    )
}

fn handler_warns_then_answers(ctx: &HookContext<'_>) -> Result<Outcome> {
    ctx.diagnostic("ss-magic: the conclusion cache is unreadable; gating anyway");
    handler_denies(ctx)
}

fn handler_fails(_ctx: &HookContext<'_>) -> Result<Outcome> {
    anyhow::bail!("the conclusion cache is corrupt")
}

fn handler_warns_then_fails(ctx: &HookContext<'_>) -> Result<Outcome> {
    ctx.diagnostic("ss-magic: about to give up");
    handler_fails(ctx)
}

fn handler_panics(_ctx: &HookContext<'_>) -> Result<Outcome> {
    panic!("index out of bounds in the gate")
}

// ── AE19: no envelope at all ──────────────────────────────────────────────────

/// A person runs the hook verb by hand from a terminal. Nothing is printed on
/// either channel, and the row says why rather than calling it a failure.
#[test]
fn ae19_no_stdin_at_all_is_silent() {
    let (_dir, run) = drive(&HookEvent::PreToolUse, "");

    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, "");
    assert_eq!(run.row.outcome, heartbeat::Outcome::NoOp);
    assert_eq!(run.row.reason.as_deref(), Some("no-input"));
    assert_eq!(run.rows().len(), 1);
}

// ── AE10: malformed stdin ─────────────────────────────────────────────────────

/// The tool call must proceed unchanged, so stdout stays empty — and the row
/// carries the error class, which is the only place the problem is visible.
#[test]
fn ae10_malformed_stdin_is_silent_and_recorded_with_its_class() {
    let (_dir, run) = drive(&HookEvent::PreToolUse, "{not json at all");

    assert_eq!(run.stdout, "");
    assert_eq!(run.row.outcome, heartbeat::Outcome::Error);
    assert_eq!(run.row.reason.as_deref(), Some("malformed-stdin"));
    assert_eq!(run.rows(), vec![run.row.clone()]);
}

/// Valid JSON that is not an envelope gets its own class, so "the harness sent
/// us something odd" is distinguishable from "the harness sent us garbage".
#[test]
fn json_that_is_not_an_envelope_is_recorded_separately() {
    let (_dir, run) = drive(&HookEvent::PreToolUse, r#"{"session_id":"s-1"}"#);

    assert_eq!(run.stdout, "");
    assert_eq!(run.row.reason.as_deref(), Some("not-an-envelope"));
}

/// A malformed envelope that still carries a readable `cwd` records it, so the
/// row says which repository the bad invocation came from.
#[test]
fn a_bad_envelope_still_records_whatever_cwd_it_carried() {
    let (_dir, run) = drive(
        &HookEvent::PreToolUse,
        r#"{"cwd":"/repo","stop_hook_active":"yes","session_id":7}"#,
    );

    assert_eq!(run.row.cwd.as_deref(), Some("/repo"));
}

// ── AE50 / R62: an event this binary cannot route ─────────────────────────────

/// A manifest naming an event an older binary has never heard of must look
/// like a hook that did nothing. Falling through to the unknown-verb exit code
/// would read to the harness as a BLOCK, turning every `Read` into a failure
/// that carries the hook's own command line into the model's context.
#[test]
fn ae50_an_unroutable_event_exits_silently_and_is_recorded() {
    let event = HookEvent::from_token("notification");
    let (_dir, run) = drive(&event, &envelope("Notification", Path::new("/repo"), ""));

    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, "");
    assert_eq!(run.row.event, "notification");
    assert_eq!(run.row.outcome, heartbeat::Outcome::NoOp);
    assert_eq!(run.row.reason.as_deref(), Some("unroutable-event"));
    assert_eq!(run.row.cwd.as_deref(), Some("/repo"));
    assert!(
        run.row.detail.as_deref().unwrap().contains("notification"),
        "{:?}",
        run.row.detail
    );
}

/// `ss-magic plugin hook` with no event token at all takes the same path.
#[test]
fn a_missing_event_token_takes_the_unroutable_path() {
    let (_dir, run) = drive(&HookEvent::Missing, "");

    assert_eq!(run.stdout, "");
    assert_eq!(run.row.event, "");
    assert_eq!(run.row.reason.as_deref(), Some("unroutable-event"));
}

/// R62 is enforced by the routing table, not by the caller: every shipped
/// event routes, and nothing else does.
#[test]
fn exactly_the_six_shipped_events_route() {
    for event in [
        HookEvent::SessionStart,
        HookEvent::PreToolUse,
        HookEvent::PreCompact,
        HookEvent::SubagentStop,
        HookEvent::SessionEnd,
        HookEvent::FileChanged,
    ] {
        assert!(route(&event).is_some(), "{event:?}");
    }
    assert!(route(&HookEvent::Unknown("notification".into())).is_none());
    assert!(route(&HookEvent::Missing).is_none());
}

/// Each event reaches the module that owns it. This is the line U12-U17 and
/// U30 each change, so a mis-wired handler shows up here rather than in a live
/// session.
#[test]
fn every_event_routes_to_the_module_that_owns_it() {
    for (event, module) in [
        (HookEvent::SessionStart, "session_start"),
        (HookEvent::PreToolUse, "pre_tool_use"),
        (HookEvent::PreCompact, "pre_compact"),
        (HookEvent::SubagentStop, "subagent_stop"),
        (HookEvent::SessionEnd, "session_end"),
        (HookEvent::FileChanged, "file_changed"),
    ] {
        assert_eq!(route(&event).unwrap().module, module, "{event:?}");
    }
}

/// R63 applies to the events that write into `.superset/.magic/` and to no
/// others. `session-end` appends to the machine-level ledger; `file-changed`
/// writes only the harness-supplied environment file, which lives outside the
/// worktree by requirement.
#[test]
fn the_state_writing_events_are_exactly_the_four_that_touch_the_tree() {
    for (event, writes) in [
        (HookEvent::SessionStart, true),
        (HookEvent::PreToolUse, true),
        (HookEvent::PreCompact, true),
        (HookEvent::SubagentStop, true),
        (HookEvent::SessionEnd, false),
        (HookEvent::FileChanged, false),
    ] {
        assert_eq!(route(&event).unwrap().writes_state, writes, "{event:?}");
    }
}

// ── AE39 / AE40 / R55: the enablement gate ────────────────────────────────────

/// A marketplace install is machine-global. A repository that never set
/// `plugin.enabled` must be untouched by it — every event no-ops with only a
/// heartbeat row.
#[test]
fn ae39_a_repository_that_never_enabled_the_plugin_is_never_acted_on() {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join(".gitignore"), ".superset/.magic/\n").unwrap();

    for event in [
        HookEvent::SessionStart,
        HookEvent::PreToolUse,
        HookEvent::PreCompact,
        HookEvent::SubagentStop,
        HookEvent::SessionEnd,
        HookEvent::FileChanged,
    ] {
        let (_d, run) = drive(&event, &envelope("X", &root, ""));
        assert_eq!(run.stdout, "", "{event:?}");
        assert_eq!(run.stderr, "", "{event:?}");
        assert_eq!(run.row.outcome, heartbeat::Outcome::NoOp, "{event:?}");
        assert_eq!(run.row.reason.as_deref(), Some("disabled"), "{event:?}");
        assert_eq!(run.rows().len(), 1, "{event:?}");
    }
}

/// The configuration is read from disk on every invocation, so switching the
/// plugin off takes effect on the NEXT hook rather than on the next session.
#[test]
fn ae40_disabling_mid_session_stops_the_very_next_invocation() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");
    let input = envelope("PreToolUse", &root, r#""tool_name":"Read""#);

    let first = drive_at(
        &store,
        &HookEvent::PreToolUse,
        route_to(handler_denies, true),
        &input,
    );
    assert_eq!(first.row.outcome, heartbeat::Outcome::Ok);
    assert_eq!(
        first.envelope()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );

    // No restart, no new process state — just the file on disk changing.
    write_file(
        &root,
        ".superset/magic.json",
        r#"{"files":[],"plugin":{"enabled":false}}"#,
    );

    let second = drive_at(
        &store,
        &HookEvent::PreToolUse,
        route_to(handler_denies, true),
        &input,
    );
    assert_eq!(second.stdout, "");
    assert_eq!(second.row.reason.as_deref(), Some("disabled"));
    assert_eq!(second.rows().len(), 2);
}

/// R7 puts the switch in the MAIN checkout's overlay, and the hook inherits
/// that: a worktree acts because main says so, not because of anything in the
/// worktree's own copy.
#[test]
fn a_worktree_takes_its_enablement_from_the_main_checkout() {
    let (main_dir, main_root) = enabled_repo();
    git_run(&["add", "-A"], &main_root);
    git_run(&["commit", "-q", "-m", "enable"], &main_root);
    let (_wt, wt_root) = make_worktree(main_dir.path());
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");

    let run = drive_at(
        &store,
        &HookEvent::PreToolUse,
        route_to(handler_ok, true),
        &envelope("PreToolUse", &wt_root, ""),
    );

    assert_eq!(run.row.outcome, heartbeat::Outcome::Ok, "{:?}", run.row);
    assert_eq!(run.row.cwd.as_deref(), Some(wt_root.to_str().unwrap()));
}

// ── AE45 / R63: the ignored-tree gate ─────────────────────────────────────────

/// A repository enabled by hand-editing `magic.json`, without ever running
/// `init`, `migrate` or `plugin enable`, has no rule making the state tree
/// invisible to git. Every state-writing event must refuse — nothing written,
/// nothing in `git status`, and a row naming the missing rule.
#[test]
fn ae45_an_enabled_repo_with_an_unignored_tree_writes_nothing() {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    write_file(
        &root,
        ".superset/magic.json",
        r#"{"files":[],"plugin":{"enabled":true}}"#,
    );
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");

    for event in [
        HookEvent::SessionStart,
        HookEvent::PreToolUse,
        HookEvent::PreCompact,
        HookEvent::SubagentStop,
    ] {
        let run = drive_at(
            &store,
            &event,
            route_to(handler_denies, true),
            &envelope("X", &root, ""),
        );

        assert_eq!(run.stdout, "", "{event:?}");
        assert_eq!(run.row.outcome, heartbeat::Outcome::NoOp, "{event:?}");
        assert_eq!(run.row.reason.as_deref(), Some("not-ignored"), "{event:?}");
        let detail = run.row.detail.clone().unwrap();
        assert!(detail.contains(".superset/.magic/"), "{detail}");
        assert!(detail.contains("ss-magic plugin enable"), "{detail}");
    }

    // Nothing the hook could have written is on disk, and the working tree is
    // exactly as clean as it was.
    assert!(!root.join(".superset/.magic").exists());
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&root)
        .output()
        .unwrap();
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(!status.contains(".magic"), "{status}");
}

/// The gate is not a blanket veto: an event that writes nothing into the tree
/// still runs with the tree unignored, because R63 is about state, not about
/// the event firing.
#[test]
fn a_non_state_writing_event_runs_even_with_the_tree_unignored() {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    write_file(
        &root,
        ".superset/magic.json",
        r#"{"files":[],"plugin":{"enabled":true}}"#,
    );
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");

    let run = drive_at(
        &store,
        &HookEvent::SessionEnd,
        route_to(handler_ok, false),
        &envelope("SessionEnd", &root, r#""reason":"exit""#),
    );

    assert_eq!(run.row.outcome, heartbeat::Outcome::Ok, "{:?}", run.row);
}

/// Fail closed in both directions: git being unable to answer is not
/// permission. Here `cwd` is a real directory outside any repository, so the
/// probe errors rather than answering.
#[test]
fn a_repository_git_cannot_answer_for_is_refused_not_assumed_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    // Enabled without a repository: `config::resolve` falls back to `cwd`
    // itself when there is no main checkout to redirect to.
    write_file(
        &root,
        ".superset/magic.json",
        r#"{"files":[],"plugin":{"enabled":true}}"#,
    );
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");

    let run = drive_at(
        &store,
        &HookEvent::SessionStart,
        route_to(handler_ok, true),
        &envelope("SessionStart", &root, r#""source":"startup""#),
    );

    assert_eq!(run.row.reason.as_deref(), Some("not-ignored"));
    assert!(
        run.row
            .detail
            .as_deref()
            .unwrap()
            .contains("could not ask git"),
        "{:?}",
        run.row.detail
    );
}

/// The query git is asked names the same tree the scratchpad bootstraps, in
/// the directory-only spelling. Asking about `.superset/.magic` without the
/// trailing slash would let a file rule answer a question about a directory.
#[test]
fn the_ignore_query_is_the_state_tree_with_a_trailing_slash() {
    assert_eq!(
        STATE_QUERY,
        format!("{}/", crate::plugin::scratchpad::STATE_REL)
    );
}

// ── The cwd itself ────────────────────────────────────────────────────────────

/// A worktree deleted between the harness spawning us and us running gets its
/// own class, rather than being reported as "disabled" — which would be true
/// but would send someone looking at their configuration.
#[test]
fn a_cwd_that_is_not_a_directory_is_reported_as_such() {
    let (_dir, run) = drive(
        &HookEvent::SessionEnd,
        &envelope("SessionEnd", Path::new("/no/such/worktree"), ""),
    );

    assert_eq!(run.stdout, "");
    assert_eq!(run.row.outcome, heartbeat::Outcome::NoOp);
    assert_eq!(run.row.reason.as_deref(), Some("cwd-missing"));
    assert_eq!(run.row.cwd.as_deref(), Some("/no/such/worktree"));
}

// ── AE32 / R47: diagnostics on stderr, one envelope on stdout ─────────────────

/// A handler that has something to say says it on stderr, uncolored, while
/// stdout carries exactly one envelope and nothing else.
#[test]
fn ae32_a_diagnostic_goes_to_stderr_while_stdout_stays_one_envelope() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();

    let run = drive_at(
        &store_dir.path().join("plugin"),
        &HookEvent::PreToolUse,
        route_to(handler_warns_then_answers, true),
        &envelope("PreToolUse", &root, r#""tool_name":"Read""#),
    );

    assert!(
        run.stderr.contains("the conclusion cache is unreadable"),
        "{:?}",
        run.stderr
    );
    // Uncolored: no ANSI escape introducer anywhere on either channel.
    assert!(!run.stderr.contains('\x1b'), "{:?}", run.stderr);
    assert!(!run.stdout.contains('\x1b'), "{:?}", run.stdout);

    let envelope = run.envelope();
    assert_eq!(envelope["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(run.row.outcome, heartbeat::Outcome::Ok);
    assert_eq!(run.row.detail.as_deref(), Some("denied"));
}

/// A diagnostic the handler managed to record before failing is still printed.
/// The explanation of a failure is worth more than one dropped because the
/// failure happened.
#[test]
fn a_diagnostic_survives_the_handler_failing_afterwards() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();

    let run = drive_at(
        &store_dir.path().join("plugin"),
        &HookEvent::PreToolUse,
        route_to(handler_warns_then_fails, true),
        &envelope("PreToolUse", &root, ""),
    );

    assert!(run.stderr.contains("about to give up"), "{:?}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(run.row.reason.as_deref(), Some("handler-error"));
}

// ── AE35: a handler that fails internally ─────────────────────────────────────

/// The session proceeds unchanged, and the row names both the event and the
/// error class — which is the only trace a fail-open path leaves.
#[test]
fn ae35_a_failing_handler_leaves_a_row_naming_the_event_and_class() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");

    let run = drive_at(
        &store,
        &HookEvent::PreToolUse,
        route_to(handler_fails, true),
        &envelope("PreToolUse", &root, ""),
    );

    assert_eq!(run.stdout, "");
    assert_eq!(run.row.event, "pre-tool-use");
    assert_eq!(run.row.outcome, heartbeat::Outcome::Error);
    assert_eq!(run.row.reason.as_deref(), Some("handler-error"));
    assert!(
        run.row
            .detail
            .as_deref()
            .unwrap()
            .contains("cache is corrupt"),
        "{:?}",
        run.row.detail
    );
    assert_eq!(run.rows(), vec![run.row.clone()]);
}

/// A handler that panics rather than returning `Err` takes the same path. A
/// panic that escaped would abort the process with a non-zero code, which a
/// `PreToolUse` hook's caller reads as a block.
#[test]
fn a_panicking_handler_is_caught_and_recorded_like_any_other_failure() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();

    // The default panic hook would print the unwind to the real stderr and
    // clutter the test output; the message itself is asserted from the row.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let run = drive_at(
        &store_dir.path().join("plugin"),
        &HookEvent::PreToolUse,
        route_to(handler_panics, true),
        &envelope("PreToolUse", &root, ""),
    );
    std::panic::set_hook(previous);

    assert_eq!(run.stdout, "");
    assert_eq!(run.row.outcome, heartbeat::Outcome::Error);
    assert_eq!(run.row.reason.as_deref(), Some("handler-panic"));
    assert!(
        run.row
            .detail
            .as_deref()
            .unwrap()
            .contains("index out of bounds"),
        "{:?}",
        run.row.detail
    );
}

// ── The happy path ────────────────────────────────────────────────────────────

/// A well-formed envelope in an enabled, ignored repository reaches the
/// handler with the envelope decoded and both roots resolved.
#[test]
fn a_well_formed_envelope_reaches_the_handler_with_its_context_resolved() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();

    fn assert_context(ctx: &HookContext<'_>) -> Result<Outcome> {
        assert_eq!(ctx.event, &HookEvent::SessionStart);
        assert_eq!(ctx.envelope.common.session_id, "s-1");
        assert_eq!(ctx.now, NOW);
        assert!(ctx.config.enabled);
        assert_eq!(ctx.cwd(), ctx.config_root);
        assert_eq!(ctx.repo_root.as_deref(), Some(ctx.config_root.as_path()));
        let Payload::SessionStart(p) = &ctx.envelope.payload else {
            panic!("wrong payload variant")
        };
        assert_eq!(p.source, "compact");
        Ok(Outcome::new(Response::SessionStart {
            additional_context: Some("guidance".into()),
            system_message: None,
        }))
    }

    let run = drive_at(
        &store_dir.path().join("plugin"),
        &HookEvent::SessionStart,
        route_to(assert_context, true),
        &envelope("SessionStart", &root, r#""source":"compact""#),
    );

    assert_eq!(
        run.envelope()["hookSpecificOutput"]["additionalContext"],
        "guidance"
    );
    assert_eq!(run.row.outcome, heartbeat::Outcome::Ok);
}

/// The six shipped events all reach their (still unimplemented) handlers and
/// leave a successful row saying so. Until U12 onward land, "no handler yet"
/// must be indistinguishable from "the handler had nothing to do" everywhere
/// except the heartbeat note.
#[test]
fn the_unimplemented_handlers_are_silent_successes() {
    let (_dir, root) = enabled_repo();

    for event in [
        HookEvent::SessionStart,
        HookEvent::PreToolUse,
        HookEvent::PreCompact,
        HookEvent::SubagentStop,
        HookEvent::SessionEnd,
        HookEvent::FileChanged,
    ] {
        let (_d, run) = drive(&event, &envelope("X", &root, ""));
        assert_eq!(run.stdout, "", "{event:?}");
        assert_eq!(run.stderr, "", "{event:?}");
        assert_eq!(run.row.outcome, heartbeat::Outcome::Ok, "{event:?}");
        assert_eq!(run.row.event, event.as_str(), "{event:?}");
        assert_eq!(
            run.row.detail.as_deref(),
            Some("handler not implemented yet"),
            "{event:?}"
        );
    }
}

/// R50: exactly one row per invocation, on every path there is.
#[test]
fn every_path_leaves_exactly_one_heartbeat_row() {
    let (_dir, root) = enabled_repo();
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("plugin");
    let good = envelope("PreToolUse", &root, "");

    let cases: Vec<(HookEvent, Option<Route>, &str)> = vec![
        (HookEvent::PreToolUse, route_to(handler_ok, true), &good),
        (HookEvent::PreToolUse, route_to(handler_fails, true), &good),
        (HookEvent::PreToolUse, route_to(handler_ok, true), ""),
        (HookEvent::PreToolUse, route_to(handler_ok, true), "{bad"),
        (HookEvent::Unknown("notification".into()), None, &good),
    ];
    let expected = cases.len();

    for (event, route, input) in cases {
        drive_at(&store, &event, route, input);
    }

    assert_eq!(heartbeat::read(&store).unwrap().len(), expected);
}

/// A store that cannot be written is not a reason to fail: the hook has still
/// done its job. The failure is reported on stderr and nothing else changes.
#[test]
fn a_heartbeat_that_cannot_be_written_is_reported_not_fatal() {
    let (_dir, root) = enabled_repo();
    let blocker = tempfile::tempdir().unwrap();
    // A regular file where the store directory belongs: every write under it
    // fails.
    let store = blocker.path().join("plugin");
    fs::write(&store, "not a directory").unwrap();

    let run = drive_at(
        &store,
        &HookEvent::PreToolUse,
        route_to(handler_denies, true),
        &envelope("PreToolUse", &root, ""),
    );

    assert_eq!(
        run.envelope()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    assert!(run.stderr.contains("heartbeat"), "{:?}", run.stderr);
}

// ── Structural guarantees (KTD1) ──────────────────────────────────────────────
//
// R9's stdout posture and R47's channel ownership are meant to be properties of
// the pipeline, not rules six handler modules each remember. These scans are
// what turns "meant to be" into "is": they read the per-event modules' source
// and fail if one reaches for a channel or a writer it must not have. They
// keep working as U12 onward add files, which review alone would not.

/// The per-event handler modules — everything in `src/plugin/hook/` except this
/// module and the wire format, both of which legitimately handle the channels.
fn per_event_sources() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugin/hook");
    let mut out = Vec::new();
    let mut stack = vec![dir];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let rel = path
                .strip_prefix(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
                .unwrap()
                .to_string_lossy()
                .into_owned();
            // `mod.rs` IS the wrapper and `event.rs` IS the wire format; the
            // `event/` and `tests.rs` bodies are tests.
            if name == "mod.rs" || name == "event.rs" || name == "tests.rs" {
                continue;
            }
            out.push((rel, fs::read_to_string(&path).unwrap()));
        }
    }
    out
}

/// No per-event module may write to stdout or stderr. The encode seam in
/// `mod.rs` is the only writer of either, which is what makes "a hook prints
/// nothing but its JSON envelope" structural.
#[test]
fn no_per_event_module_writes_to_a_standard_stream() {
    for (path, body) in per_event_sources() {
        for forbidden in [
            "println!",
            "print!",
            "eprintln!",
            "eprint!",
            "io::stdout",
            "io::stderr",
        ] {
            assert!(
                !body.contains(forbidden),
                "{path} uses `{forbidden}`; handlers answer by returning an \
                 Outcome and report through HookContext::diagnostic"
            );
        }
    }
}

/// R40: no hook verb ever writes a `.gitignore`. A hook that could edit one
/// would be a hook that dirties the user's working tree behind their back, and
/// the wrapper's ignored-tree gate exists precisely because the rule is
/// written elsewhere.
#[test]
fn no_hook_module_writes_a_gitignore_rule() {
    let mut sources = per_event_sources();
    let mod_rs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugin/hook/mod.rs");
    sources.push((
        "src/plugin/hook/mod.rs".to_string(),
        fs::read_to_string(mod_rs).unwrap(),
    ));

    for (path, body) in sources {
        for forbidden in [
            "ensure_state_ignored",
            "ensure_path_ignored",
            "ensure_entry",
        ] {
            assert!(!body.contains(forbidden), "{path} calls `{forbidden}`");
        }
    }
}

/// R9's exit posture: the wrapper has no expression anywhere that produces a
/// non-zero exit code, so no future edit can make a hook block by accident.
#[test]
fn the_entry_point_has_no_non_zero_exit() {
    let body = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugin/hook/mod.rs"),
    )
    .unwrap();
    let code = body.split("#[cfg(test)]").next().unwrap();

    assert!(code.contains("Ok(ExitCode::SUCCESS)"));
    assert!(!code.contains("ExitCode::from"), "a non-zero exit code");
    assert!(!code.contains("ExitCode::FAILURE"), "a non-zero exit code");
    assert!(!code.contains("std::process::exit"), "a bare process exit");
}
