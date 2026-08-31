//! Wire-format tests: what decodes, what does not, and exactly what goes on
//! the wire for each response shape.
//!
//! Everything here runs on string literals — no git, no filesystem, no
//! process — which is the whole reason the format lives in its own module.

use super::*;

/// A minimal but realistic envelope for `event`, with `extra` merged in at the
/// top level.
fn envelope_json(name: &str, extra: &str) -> String {
    let extra = extra.trim();
    let tail = if extra.is_empty() {
        String::new()
    } else {
        format!(",{extra}")
    };
    format!(
        r#"{{"session_id":"s-1","transcript_path":"/t/s-1.jsonl",
            "cwd":"/repo","hook_event_name":"{name}"{tail}}}"#
    )
}

// ── Decoding ──────────────────────────────────────────────────────────────────

#[test]
fn session_start_decodes_its_source_and_cost_fields() {
    let input = envelope_json(
        "SessionStart",
        r#""source":"resume","context_tokens":1234,"estimated_cache_write_usd":0.5"#,
    );
    let env = decode(&HookEvent::SessionStart, &input).unwrap();

    assert_eq!(env.common.session_id, "s-1");
    assert_eq!(env.common.cwd, "/repo");
    let Payload::SessionStart(p) = env.payload else {
        panic!("wrong payload variant")
    };
    assert_eq!(p.source, "resume");
    assert_eq!(p.context_tokens, Some(1234));
    assert_eq!(p.estimated_cache_write_usd, Some(0.5));
}

#[test]
fn pre_tool_use_keeps_tool_input_untyped() {
    let input = envelope_json(
        "PreToolUse",
        r#""tool_name":"Read","tool_input":{"file_path":"/repo/big.rs","offset":10},
           "tool_use_id":"tu-9""#,
    );
    let env = decode(&HookEvent::PreToolUse, &input).unwrap();

    let Payload::PreToolUse(p) = env.payload else {
        panic!("wrong payload variant")
    };
    assert_eq!(p.tool_name, "Read");
    assert_eq!(p.tool_input["file_path"], "/repo/big.rs");
    assert_eq!(p.tool_input["offset"], 10);
    assert_eq!(p.tool_use_id.as_deref(), Some("tu-9"));
    // Not a subagent call.
    assert!(p.agent_id.is_none());
}

#[test]
fn subagent_stop_decodes_the_salvage_path_and_reentry_flag() {
    let input = envelope_json(
        "SubagentStop",
        r#""last_assistant_message":"done","agent_id":"a-1","agent_type":"Explore",
           "agent_transcript_path":"/t/s-1/subagents/agent-a-1.jsonl",
           "stop_hook_active":true"#,
    );
    let env = decode(&HookEvent::SubagentStop, &input).unwrap();

    let Payload::SubagentStop(p) = env.payload else {
        panic!("wrong payload variant")
    };
    assert_eq!(
        p.agent_transcript_path.as_deref(),
        Some("/t/s-1/subagents/agent-a-1.jsonl")
    );
    assert!(p.stop_hook_active);
}

#[test]
fn every_routable_event_decodes_the_same_common_fields() {
    for (event, name) in [
        (HookEvent::SessionStart, "SessionStart"),
        (HookEvent::PreToolUse, "PreToolUse"),
        (HookEvent::PreCompact, "PreCompact"),
        (HookEvent::SubagentStop, "SubagentStop"),
        (HookEvent::SessionEnd, "SessionEnd"),
        (HookEvent::FileChanged, "FileChanged"),
    ] {
        let env = decode(&event, &envelope_json(name, "")).unwrap();
        assert_eq!(env.common.cwd, "/repo", "{name}");
        assert_eq!(env.common.transcript_path, "/t/s-1.jsonl", "{name}");
    }
}

/// A field the harness grows tomorrow must not turn every hook invocation into
/// a decode failure. Unknown keys are ignored, and the raw JSON keeps them
/// reachable for a handler that learns about them before this module does.
#[test]
fn unknown_keys_are_ignored_but_survive_in_raw() {
    let input = envelope_json("SessionEnd", r#""reason":"exit","some_new_field":[1,2]"#);
    let env = decode(&HookEvent::SessionEnd, &input).unwrap();

    let Payload::SessionEnd(p) = &env.payload else {
        panic!("wrong payload variant")
    };
    assert_eq!(p.reason, "exit");
    assert_eq!(env.raw["some_new_field"][1], 2);
}

/// Absent optional fields default rather than failing: a `SessionStart` on
/// `startup` carries no cost fields at all.
#[test]
fn absent_optional_fields_default() {
    let env = decode(&HookEvent::SessionStart, &envelope_json("SessionStart", "")).unwrap();
    let Payload::SessionStart(p) = env.payload else {
        panic!("wrong payload variant")
    };
    assert_eq!(p.source, "");
    assert!(p.context_tokens.is_none());
}

#[test]
fn empty_and_whitespace_stdin_is_no_input() {
    for input in ["", "   ", "\n\n"] {
        assert_eq!(
            decode(&HookEvent::PreToolUse, input),
            Err(DecodeError::NoInput),
            "{input:?}"
        );
    }
}

#[test]
fn non_json_stdin_is_malformed() {
    let err = decode(&HookEvent::PreToolUse, "{not json").unwrap_err();
    assert_eq!(err.class(), "malformed-stdin");
}

/// Valid JSON that is not an envelope: the two shapes worth distinguishing are
/// "not an object at all" and "an object missing the one required field".
#[test]
fn valid_json_that_is_not_an_envelope_is_rejected() {
    for input in ["[1,2,3]", "\"a string\"", "42", "null"] {
        let err = decode(&HookEvent::PreToolUse, input).unwrap_err();
        assert_eq!(err.class(), "not-an-envelope", "{input}");
        assert!(err.to_string().contains("not an object"), "{input}");
    }

    let err = decode(&HookEvent::PreToolUse, r#"{"session_id":"s-1"}"#).unwrap_err();
    assert_eq!(err.class(), "not-an-envelope");
    assert!(err.to_string().contains("cwd"), "{err}");
}

/// A typed field of the wrong JSON type is a shape error, not a silent
/// default: a handler reading `stop_hook_active` must never see `false`
/// because the harness sent `"true"`.
#[test]
fn a_typed_field_of_the_wrong_type_is_a_shape_error() {
    let input = envelope_json("SubagentStop", r#""stop_hook_active":"yes""#);
    let err = decode(&HookEvent::SubagentStop, &input).unwrap_err();
    assert_eq!(err.class(), "not-an-envelope");
}

#[test]
fn an_unroutable_event_never_produces_an_envelope() {
    for event in [
        HookEvent::Unknown("notification".to_string()),
        HookEvent::Missing,
    ] {
        let err = decode(&event, &envelope_json("Notification", "")).unwrap_err();
        assert_eq!(err.class(), "unroutable-event");
    }
}

#[test]
fn cwd_hint_reads_cwd_out_of_anything_parseable() {
    assert_eq!(
        cwd_hint(&envelope_json("PreToolUse", "")).as_deref(),
        Some("/repo")
    );
    // Not JSON, no `cwd`, and a `cwd` of the wrong type all give up quietly —
    // this only ever decorates a log line.
    assert_eq!(cwd_hint("{nope"), None);
    assert_eq!(cwd_hint(r#"{"session_id":"s"}"#), None);
    assert_eq!(cwd_hint(r#"{"cwd":7}"#), None);
}

// ── Encoding ──────────────────────────────────────────────────────────────────

/// Parse an encoded response back, so assertions are about JSON structure
/// rather than key order.
fn encoded(response: &Response) -> Option<Value> {
    encode(response)
        .unwrap()
        .map(|line| serde_json::from_str(&line).unwrap())
}

#[test]
fn silent_encodes_to_nothing_at_all() {
    assert_eq!(encode(&Response::Silent).unwrap(), None);
}

#[test]
fn session_start_encodes_additional_context_and_system_message() {
    let out = encoded(&Response::SessionStart {
        additional_context: Some("guidance".into()),
        system_message: Some("notice".into()),
    })
    .unwrap();

    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert_eq!(out["hookSpecificOutput"]["additionalContext"], "guidance");
    assert_eq!(out["systemMessage"], "notice");
}

/// A response carrying nothing prints nothing. Emitting a bare
/// `{"hookSpecificOutput":{"hookEventName":"SessionStart"}}` would be a valid
/// envelope that says nothing, and on `SessionStart` even that is paid for in
/// tokens on every single session.
#[test]
fn a_response_with_no_content_encodes_to_nothing() {
    assert_eq!(
        encode(&Response::SessionStart {
            additional_context: None,
            system_message: None,
        })
        .unwrap(),
        None
    );
    assert_eq!(
        encode(&Response::PreToolUse(PreToolUseResponse::default())).unwrap(),
        None
    );
}

#[test]
fn pre_tool_use_deny_carries_its_reason_on_the_uncapped_channel() {
    let out = encoded(&Response::PreToolUse(PreToolUseResponse {
        decision: Some(PermissionDecision::Deny),
        reason: Some("read the conclusion instead".into()),
        ..Default::default()
    }))
    .unwrap();

    let specific = &out["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], "PreToolUse");
    assert_eq!(specific["permissionDecision"], "deny");
    assert_eq!(
        specific["permissionDecisionReason"],
        "read the conclusion instead"
    );
    // Nothing that was not set appears at all.
    assert!(specific.get("additionalContext").is_none());
    assert!(out.get("systemMessage").is_none());
}

/// The advisory shape R91's commit nudge uses: context and no decision, so it
/// never enters the permission fold.
#[test]
fn pre_tool_use_can_carry_context_with_no_decision() {
    let out = encoded(&Response::PreToolUse(PreToolUseResponse {
        additional_context: Some("the checklist is stale".into()),
        ..Default::default()
    }))
    .unwrap();

    let specific = &out["hookSpecificOutput"];
    assert_eq!(specific["additionalContext"], "the checklist is stale");
    assert!(specific.get("permissionDecision").is_none());
}

/// `SubagentStop` blocks with a TOP-LEVEL decision. The nested
/// `hookSpecificOutput` form fails validation on this event — there is no
/// `Stop` member in that union.
#[test]
fn subagent_stop_blocks_at_the_top_level() {
    let out = encoded(&Response::SubagentStopBlock {
        reason: "the declared artifact is missing".into(),
    })
    .unwrap();

    assert_eq!(out["decision"], "block");
    assert_eq!(out["reason"], "the declared artifact is missing");
    assert!(out.get("hookSpecificOutput").is_none());
}

/// R20/R81. Two rewriting hooks on one event is a race whose loser vanishes
/// with no error anywhere, so ss-magic emits no rewrite on any event — and the
/// way that is guaranteed is that no response shape can express one. This
/// asserts the guarantee over every variant rather than trusting review.
#[test]
fn no_response_shape_can_express_a_tool_input_rewrite() {
    let responses = [
        Response::Silent,
        Response::SessionStart {
            additional_context: Some("x".into()),
            system_message: Some("y".into()),
        },
        Response::PreToolUse(PreToolUseResponse {
            decision: Some(PermissionDecision::Deny),
            reason: Some("r".into()),
            additional_context: Some("c".into()),
            system_message: Some("m".into()),
        }),
        Response::SubagentStopBlock { reason: "r".into() },
    ];

    for response in responses {
        let Some(line) = encode(&response).unwrap() else {
            continue;
        };
        assert!(!line.contains("updatedInput"), "{line}");
        assert!(!line.contains("permissionDecision\":\"allow"), "{line}");
    }
}

/// Every encoded response is exactly one line, so the wrapper's single
/// `writeln!` can never emit two envelopes.
#[test]
fn every_encoded_response_is_a_single_line() {
    for response in [
        Response::SessionStart {
            additional_context: Some("two\nlines\nof guidance".into()),
            system_message: None,
        },
        Response::PreToolUse(PreToolUseResponse {
            decision: Some(PermissionDecision::Deny),
            reason: Some("a reason\nwith a newline".into()),
            ..Default::default()
        }),
        Response::SubagentStopBlock {
            reason: "multi\nline".into(),
        },
    ] {
        let line = encode(&response).unwrap().unwrap();
        assert!(!line.contains('\n'), "{line}");
    }
}
