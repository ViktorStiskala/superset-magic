//! The hook wire format: what the harness writes to a hook's stdin, and the
//! one JSON object the hook is allowed to write back.
//!
//! Everything here is data — no I/O, no git, no filesystem — so the whole
//! format is unit-testable from string literals. `hook/mod.rs` is the only
//! caller: it decodes stdin into an [`Envelope`], hands that to a per-event
//! handler, and encodes the handler's [`Response`] onto stdout. Per-event
//! modules never touch either end of the pipe.
//!
//! Field names and shapes come from
//! `docs/plans/2026-08-29-001-ss-magic-plugin/hook-contract.md`, which
//! recorded them by capturing real payloads from Claude Code 2.1.251 rather
//! than by reading documentation.
//!
//! ## Decoding is permissive; routing is not
//!
//! Only `cwd` is required, because it is the field every later decision hangs
//! off: which repository this is, whether the plugin is enabled there, and
//! which worktree the state tree belongs to. Everything else defaults, and
//! unknown keys are ignored outright, so a harness that grows a field does not
//! turn every hook invocation into a decode failure. Each envelope also keeps
//! its own [`Envelope::raw`] JSON, which is how a handler reaches a field this
//! module has not typed yet.
//!
//! ## Three things this module deliberately cannot express
//!
//! - **A tool-input rewrite.** There is no `updatedInput` anywhere in the
//!   response types. Handlers registered on one event run concurrently against
//!   the ORIGINAL input and their rewrites are folded last-write-wins, so two
//!   rewriting hooks on one event is a race whose loser vanishes with no error
//!   — and the user's own `rtk` wrapper is a live rewriting hook. Not offering
//!   the channel is what makes "ss-magic never rewrites a tool input" a
//!   property of the types instead of a rule everyone has to remember.
//! - **An `allow` decision.** [`PermissionDecision`] has exactly one variant,
//!   `Deny`. In this user's auto mode a hook `allow` is funnelled back through
//!   the classifier anyway ("Hook approved tool use for X, but auto mode
//!   requires classifier adjudication") and an always-deny rule still overrides
//!   it, so an `allow` would be a capability grant that does not actually
//!   grant anything. `deny` is the only decision with a hard guarantee, and it
//!   composes safely because decisions fold monotonically by rank.
//! - **Anything on `PreCompact`.** That event is absent from the harness's
//!   `hookSpecificOutput` schema map and emitting one is rejected outright, so
//!   [`Response`] offers no `PreCompact` variant: the handler writes its note
//!   to the scratchpad and returns [`Response::Silent`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plugin::HookEvent;

// ── The inbound envelope ──────────────────────────────────────────────────────

/// The fields every hook envelope carries, whatever the event.
///
/// `cwd` is the only one without a default. It follows Claude into a linked
/// worktree, which is why every downstream resolution (the repository root,
/// the overlaid `plugin` config, the session slug) starts from it rather than
/// from this process's own working directory — a hook is spawned by the
/// harness and inherits whatever cwd the harness happened to have.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Common {
    /// The harness's id for this session. Stable across a session's turns; a
    /// `/clear` mints a new one.
    #[serde(default)]
    pub session_id: String,
    /// Absolute path to the session's transcript JSONL.
    #[serde(default)]
    pub transcript_path: String,
    /// The directory the session is working in.
    pub cwd: String,
    /// The event name as the harness spells it (`"PreToolUse"`, …). Recorded
    /// but not routed on: argv is what selects the handler, so a payload whose
    /// `hook_event_name` disagrees with the argv token cannot redirect us.
    #[serde(default)]
    pub hook_event_name: String,
    /// Present on most events; absent on some.
    #[serde(default)]
    pub prompt_id: Option<String>,
}

/// `SessionStart` — fires on all five sources, `compact` included.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SessionStart {
    /// One of `startup`, `resume`, `clear`, `compact`, `fork`.
    #[serde(default)]
    pub source: String,
    /// Present on `resume` and `fork` only — the only place cost-shaped fields
    /// appear in any hook payload.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    /// Present on `resume` and `fork` only.
    #[serde(default)]
    pub estimated_cache_write_usd: Option<f64>,
}

/// `PreToolUse` — fires before the tool runs, and is the only event whose
/// response can stop it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PreToolUse {
    /// `Read`, `Edit`, `Write`, `NotebookEdit`, `Bash`, …
    #[serde(default)]
    pub tool_name: String,
    /// The tool's arguments, untyped: the shape differs per tool and the gate
    /// only ever reads a few keys out of it (`file_path`, `offset`, `limit`,
    /// `command`).
    #[serde(default)]
    pub tool_input: Value,
    /// The harness's id for this call. Note that a hook matched by an `if`
    /// condition spawns once per matching `&&` subcommand with the SAME id, so
    /// this does not uniquely identify an invocation.
    #[serde(default)]
    pub tool_use_id: Option<String>,
    /// Set when the call comes from a dispatched subagent rather than the main
    /// thread.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The subagent's type, when `agent_id` is set.
    #[serde(default)]
    pub agent_type: Option<String>,
}

/// `PreCompact` — fires before a compaction, on either trigger, and also when
/// no compaction actually follows (a `/compact` on a session too small to
/// compact still fires it).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PreCompact {
    /// `manual` or `auto`.
    #[serde(default)]
    pub trigger: String,
    /// Whatever the user typed after `/compact`, on the manual trigger.
    #[serde(default)]
    pub custom_instructions: Option<String>,
}

/// `SubagentStop` — fires when a dispatched agent finishes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SubagentStop {
    /// The agent's final message, when it produced one.
    #[serde(default)]
    pub last_assistant_message: Option<String>,
    /// The agent's id.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The agent's type.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// `…/<session>/subagents/agent-<id>.jsonl` — the path that makes salvage
    /// possible without guessing where the agent's output went.
    #[serde(default)]
    pub agent_transcript_path: Option<String>,
    /// `true` when this stop is a re-entry caused by a previous block. A
    /// handler that blocks must consult it, or it blocks its own block
    /// forever.
    #[serde(default)]
    pub stop_hook_active: bool,
}

/// `SessionEnd` — six keys and no usage data, which is why the cost ledger has
/// to read the transcript rather than the payload.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SessionEnd {
    /// Why the session ended (`clear`, `exit`, …).
    #[serde(default)]
    pub reason: String,
}

/// `file-changed` — fires on a watched file write.
///
/// The payload shape here is provisional: unlike the other five, it was not
/// captured from a live session, and R92 makes U30 probe it on the pinned
/// harness version before the direnv export ships. Both spellings are typed
/// because either is plausible, and a handler that needs certainty should read
/// [`Envelope::raw`] rather than trusting these two fields to be populated.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FileChanged {
    /// The changed file, if the harness reports a single one.
    #[serde(default)]
    pub file_path: Option<String>,
    /// The changed files, if the harness reports a batch.
    #[serde(default)]
    pub file_paths: Vec<String>,
}

/// The per-event half of a decoded envelope. Which variant is built is decided
/// by the argv token, never by the payload — see [`Common::hook_event_name`].
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    SessionStart(SessionStart),
    PreToolUse(PreToolUse),
    PreCompact(PreCompact),
    SubagentStop(SubagentStop),
    SessionEnd(SessionEnd),
    FileChanged(FileChanged),
}

/// One decoded hook invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    /// The fields shared by every event.
    pub common: Common,
    /// The fields particular to this one.
    pub payload: Payload,
    /// The envelope exactly as it arrived. Kept so a handler can reach a field
    /// this module has not typed — the alternative is guessing at shapes now
    /// and silently dropping whatever the guess missed.
    pub raw: Value,
}

// ── Decode failures ───────────────────────────────────────────────────────────

/// Why an envelope could not be decoded. Every variant ends the same way for
/// the caller — exit 0, empty stdout, one heartbeat row — so the distinctions
/// exist for the row's error class, not for control flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// stdin was empty or whitespace. Ordinarily this means a person ran the
    /// hook verb by hand from a terminal, not that anything went wrong.
    NoInput,
    /// stdin was not JSON at all.
    Malformed(String),
    /// stdin parsed as JSON but is not an envelope: not an object, missing
    /// `cwd`, or carrying a typed field of the wrong type.
    NotAnEnvelope(String),
    /// The argv named an event this binary cannot route. Unreachable through
    /// `hook/mod.rs`, which screens for it before reading stdin; kept so
    /// [`decode`] is total over [`HookEvent`].
    Unroutable(String),
}

impl DecodeError {
    /// A stable, machine-readable class for the heartbeat row, so changing the
    /// human text below never breaks anything reading the log.
    pub fn class(&self) -> &'static str {
        match self {
            Self::NoInput => "no-input",
            Self::Malformed(_) => "malformed-stdin",
            Self::NotAnEnvelope(_) => "not-an-envelope",
            Self::Unroutable(_) => "unroutable-event",
        }
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInput => write!(f, "no envelope on stdin"),
            Self::Malformed(detail) => write!(f, "stdin is not JSON: {detail}"),
            Self::NotAnEnvelope(detail) => write!(f, "stdin is not a hook envelope: {detail}"),
            Self::Unroutable(name) => write!(f, "no handler for hook event `{name}`"),
        }
    }
}

// ── Decode ────────────────────────────────────────────────────────────────────

/// Decode `input` as the envelope for `event`.
pub fn decode(event: &HookEvent, input: &str) -> Result<Envelope, DecodeError> {
    if input.trim().is_empty() {
        return Err(DecodeError::NoInput);
    }

    let raw: Value =
        serde_json::from_str(input).map_err(|e| DecodeError::Malformed(e.to_string()))?;
    if !raw.is_object() {
        return Err(DecodeError::NotAnEnvelope(
            "the top-level JSON value is not an object".to_string(),
        ));
    }

    let common: Common = from_raw(&raw)?;
    let payload = match event {
        HookEvent::SessionStart => Payload::SessionStart(from_raw(&raw)?),
        HookEvent::PreToolUse => Payload::PreToolUse(from_raw(&raw)?),
        HookEvent::PreCompact => Payload::PreCompact(from_raw(&raw)?),
        HookEvent::SubagentStop => Payload::SubagentStop(from_raw(&raw)?),
        HookEvent::SessionEnd => Payload::SessionEnd(from_raw(&raw)?),
        HookEvent::FileChanged => Payload::FileChanged(from_raw(&raw)?),
        HookEvent::Unknown(name) => return Err(DecodeError::Unroutable(name.clone())),
        HookEvent::Missing => return Err(DecodeError::Unroutable(String::new())),
    };

    Ok(Envelope {
        common,
        payload,
        raw,
    })
}

/// Deserialize one typed view out of the raw envelope, turning serde's own
/// message into the shape-level error class.
fn from_raw<T: serde::de::DeserializeOwned>(raw: &Value) -> Result<T, DecodeError> {
    serde_json::from_value(raw.clone()).map_err(|e| DecodeError::NotAnEnvelope(e.to_string()))
}

/// Best-effort read of `cwd` out of an undecoded stdin buffer.
///
/// Used on the paths that never reach [`decode`] — an event this binary cannot
/// route — so the heartbeat row can still say which repository the invocation
/// came from. Anything unparseable is simply `None`; this is a nicety for the
/// log, never a decision input.
pub fn cwd_hint(input: &str) -> Option<String> {
    serde_json::from_str::<Value>(input)
        .ok()?
        .get("cwd")?
        .as_str()
        .map(str::to_string)
}

// ── The outbound response ─────────────────────────────────────────────────────

/// The only permission decision this binary emits. See the module docs for why
/// there is no `Allow` and no `Ask`.
// Constructed by U14's Read gate and U28's checklist deny; the encoder and the
// tests here already exercise it.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Deny,
}

impl PermissionDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
        }
    }
}

/// What a `PreToolUse` handler answers with.
///
/// The two channels have very different capacities and both are used:
/// `reason` rides `permissionDecisionReason`, which is uncapped and delivered
/// to the model verbatim with no wrapper text, while `additional_context`
/// cliffs at 10,000 characters and is replaced by a persisted-output preview
/// past that. Neither is protected from a runaway payload by the harness, so
/// whatever fills `reason` must impose its own byte budget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreToolUseResponse {
    /// `Some(Deny)` blocks the call; `None` leaves the normal permission flow
    /// alone.
    pub decision: Option<PermissionDecision>,
    /// The text the model sees when the call is denied.
    pub reason: Option<String>,
    /// Context injected whether or not the call is denied.
    pub additional_context: Option<String>,
    /// An operator notice. This is a user/SDK channel, not a model channel.
    pub system_message: Option<String>,
}

impl PreToolUseResponse {
    /// True when nothing here would reach anyone, so the wrapper can print
    /// nothing at all rather than an envelope with no content in it.
    fn is_empty(&self) -> bool {
        self.decision.is_none()
            && self.reason.is_none()
            && self.additional_context.is_none()
            && self.system_message.is_none()
    }
}

/// What a per-event handler wants written to stdout.
///
/// There is one variant per event that HAS a model-facing channel, plus
/// [`Response::Silent`] for the events that do not (`PreCompact`,
/// `SessionEnd`, `file-changed`) and for any handler that decides to say
/// nothing. Nothing here can express a tool-input rewrite.
// The non-`Silent` variants are constructed by the per-event handlers of U12
// onward; the encoder and the tests here already exercise every one.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Write nothing. stdout stays completely empty.
    Silent,
    /// `SessionStart` — one of only three events whose output reaches the
    /// model at all. Budget `additional_context` under 10,000 characters or
    /// the harness swaps it for a persisted-output pointer that costs the
    /// model an extra tool call.
    SessionStart {
        additional_context: Option<String>,
        system_message: Option<String>,
    },
    /// `PreToolUse` — the gate.
    PreToolUse(PreToolUseResponse),
    /// `SubagentStop` — a top-level block, which is the shape this event
    /// requires: there is no `Stop` member in the `hookSpecificOutput` union,
    /// so the nested form fails validation here.
    SubagentStopBlock { reason: String },
}

// ── Serialized shapes ─────────────────────────────────────────────────────────
//
// Separate private structs rather than hand-built `json!` objects, so the wire
// keys live next to the types that own them and a rename cannot silently
// change what goes out.

#[derive(Serialize)]
struct SessionStartOut<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: SessionStartSpecific<'a>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<&'a str>,
}

#[derive(Serialize)]
struct SessionStartSpecific<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    additional_context: Option<&'a str>,
}

#[derive(Serialize)]
struct PreToolUseOut<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: PreToolUseSpecific<'a>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<&'a str>,
}

#[derive(Serialize)]
struct PreToolUseSpecific<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision", skip_serializing_if = "Option::is_none")]
    permission_decision: Option<&'static str>,
    #[serde(
        rename = "permissionDecisionReason",
        skip_serializing_if = "Option::is_none"
    )]
    permission_decision_reason: Option<&'a str>,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    additional_context: Option<&'a str>,
}

#[derive(Serialize)]
struct SubagentStopOut<'a> {
    decision: &'static str,
    reason: &'a str,
}

/// Render `response` as the single line of JSON that goes on stdout, or `None`
/// when it carries nothing and stdout should stay empty.
///
/// `Err` only when serialization itself fails, which for these shapes means a
/// non-finite float or a similarly impossible value; the caller treats it like
/// any other internal error and stays silent.
pub fn encode(response: &Response) -> Result<Option<String>, serde_json::Error> {
    let text = match response {
        Response::Silent => return Ok(None),

        Response::SessionStart {
            additional_context,
            system_message,
        } => {
            if additional_context.is_none() && system_message.is_none() {
                return Ok(None);
            }
            serde_json::to_string(&SessionStartOut {
                hook_specific_output: SessionStartSpecific {
                    hook_event_name: "SessionStart",
                    additional_context: additional_context.as_deref(),
                },
                system_message: system_message.as_deref(),
            })?
        }

        Response::PreToolUse(inner) => {
            if inner.is_empty() {
                return Ok(None);
            }
            serde_json::to_string(&PreToolUseOut {
                hook_specific_output: PreToolUseSpecific {
                    hook_event_name: "PreToolUse",
                    permission_decision: inner.decision.map(PermissionDecision::as_str),
                    permission_decision_reason: inner.reason.as_deref(),
                    additional_context: inner.additional_context.as_deref(),
                },
                system_message: inner.system_message.as_deref(),
            })?
        }

        Response::SubagentStopBlock { reason } => serde_json::to_string(&SubagentStopOut {
            decision: "block",
            reason,
        })?,
    };
    Ok(Some(text))
}

#[cfg(test)]
mod tests;
