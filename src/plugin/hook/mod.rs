//! The one hook entry point: stdin in, at most one JSON object out, always
//! exit 0.
//!
//! Every `ss-magic plugin hook <event>` invocation runs through [`run`], and
//! this module owns every part of it that is not the event's own logic:
//! reading stdin, decoding the envelope, deciding whether the plugin should
//! act at all, dispatching to the per-event handler, encoding the answer onto
//! stdout, and appending the heartbeat row. Per-event modules receive a
//! [`HookContext`] and return an [`Outcome`]. **They never print, never exit,
//! and never touch stdout or stderr.**
//!
//! That split is the point. R9's "nothing on stdout but the envelope" and
//! R47's "diagnostics on stderr, color off" become properties of the pipeline
//! rather than rules six handler modules each have to remember — a handler
//! that wanted to write to stdout would have to reach for `println!` in a file
//! whose tests scan for exactly that.
//!
//! ## Fail-open, structurally
//!
//! `run` returns `ExitCode::SUCCESS` on every path there is. It is not that
//! the error paths happen to return 0; there is no expression anywhere in this
//! module that produces a non-zero code. This matters more than it looks:
//!
//! - Exit 2 from a `PreToolUse` hook is a **block**, and the harness reports it
//!   to the model wrapped in text that includes the hook's own configured
//!   command line. A plugin whose binary crashed would turn every `Read` into a
//!   failure that also leaks its command line into the model's context.
//! - An installed manifest can name an event a running binary has never heard
//!   of — the marketplace ships the manifest and the binary together, but a
//!   user can end up with a newer manifest and an older binary. That has to
//!   look like a hook that decided to do nothing, not like a block.
//!
//! So an unroutable event, a malformed envelope, a handler that returns `Err`
//! and a handler that outright panics all end the same way: empty stdout, exit
//! 0, one heartbeat row naming what happened.
//!
//! ## Two gates, both here, both before dispatch
//!
//! - **Is the plugin enabled for this repository?** A marketplace install is
//!   machine-global; the per-repository `plugin.enabled` key is what keeps an
//!   install made for one repository from acting in every other one. It is
//!   resolved fresh from disk on every invocation, so flipping it off takes
//!   effect on the next hook rather than on the next session.
//! - **Does git report `.superset/.magic/` ignored?** The state tree holds
//!   session notes and cached conclusions, and no hook may ever write it into
//!   somewhere git can see. The rule that makes it ignored is written only by
//!   explicit `ss-magic` invocations — `init`, `migrate`, `plugin enable` —
//!   never by a hook, so a repository that enabled the plugin by hand-editing
//!   `magic.json` genuinely can reach a hook with an unignored tree. The check
//!   is fail-closed in both directions: "git says no" and "git could not be
//!   asked" both refuse.
//!
//! Both gates sit in the pipeline, not in the handlers, and the second applies
//! only to the events that actually write into the state tree — see
//! [`Route::writes_state`].

use std::io::{Read, Write};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::git;
use crate::plugin::config::{self, PluginConfig};
use crate::plugin::heartbeat::{self, Outcome as RowOutcome, Row};
use crate::plugin::HookEvent;

pub mod event;
mod pre_compact;
mod pre_tool_use;
mod session_start;

use event::{DecodeError, Envelope, Response};

/// The path git is asked about for the ignored-tree gate. The trailing slash
/// forces git's directory-only match, the same query `scratchpad::ensure`
/// uses — asking about `.superset/.magic` without it would let a *file* rule
/// answer a question about a directory.
const STATE_QUERY: &str = ".superset/.magic/";

// ── Heartbeat reason classes ──────────────────────────────────────────────────
//
// One `&'static str` per way an invocation can end without a handler running
// to completion. These strings are the log's stable vocabulary: `status` counts
// them and a person greps them, so the constants live together rather than
// being spelled out at each site.

/// The envelope named an event this binary cannot route.
const REASON_UNROUTABLE: &str = "unroutable-event";
/// stdin could not be read at all (not the same as it being empty).
const REASON_STDIN: &str = "stdin-read-failed";
/// The response could not be written to stdout — the harness closed the pipe.
const REASON_STDOUT: &str = "stdout-write-failed";
/// The envelope's `cwd` does not exist as a directory.
const REASON_CWD_MISSING: &str = "cwd-missing";
/// `plugin.enabled` is not true for this repository.
const REASON_DISABLED: &str = "disabled";
/// git does not report the state tree ignored, or could not be asked.
const REASON_NOT_IGNORED: &str = "not-ignored";
/// The handler returned an error.
const REASON_HANDLER_ERROR: &str = "handler-error";
/// The handler panicked.
const REASON_HANDLER_PANIC: &str = "handler-panic";
/// The handler's response could not be serialized.
const REASON_ENCODE_FAILED: &str = "encode-failed";

// ── What a handler is given ───────────────────────────────────────────────────

/// Everything a per-event handler may look at, plus the two channels it may
/// use.
///
/// Note what is absent: no stdin, no stdout, no exit code. A handler answers
/// by returning an [`Outcome`]; it reports a problem either by returning `Err`
/// or, when it wants to say something without failing, through
/// [`HookContext::diagnostic`].
// The pipeline builds every field; the readers are the per-event handlers of
// U12 onward, so until those land the compiler sees write-only fields.
#[allow(dead_code)]
pub struct HookContext<'a> {
    /// The event, as argv named it.
    pub event: &'a HookEvent,
    /// The decoded envelope, including its untouched raw JSON.
    pub envelope: &'a Envelope,
    /// The git repository root containing the envelope's `cwd`, or `None` when
    /// that directory is not inside a repository at all. A handler that needs
    /// a repository stops here and returns a silent outcome.
    pub repo_root: Option<PathBuf>,
    /// The root the configuration and the ignored-tree probe were resolved
    /// against: [`Self::repo_root`] when there is one, the envelope's `cwd`
    /// otherwise.
    pub config_root: PathBuf,
    /// The plugin configuration in force for this repository.
    pub config: &'a PluginConfig,
    /// Seconds since the Unix epoch, captured once at the top of the
    /// invocation. Injected rather than read per call so a handler's output is
    /// reproducible in a test and every part of one invocation agrees on the
    /// time.
    pub now: u64,
    /// Diagnostics the wrapper will write to stderr after dispatch. A
    /// `RefCell` because a handler holds `&HookContext`, and giving it `&mut`
    /// would mean threading mutability through every helper it calls for the
    /// sake of an occasional warning.
    diagnostics: std::cell::RefCell<Vec<String>>,
}

impl<'a> HookContext<'a> {
    /// The envelope's working directory.
    // Read by the per-event handlers of U12 onward; the tests here already
    // exercise it.
    #[allow(dead_code)]
    pub fn cwd(&self) -> &Path {
        Path::new(&self.envelope.common.cwd)
    }

    /// Record a line for stderr. The wrapper writes these out verbatim and
    /// uncolored once dispatch is over, including when the handler went on to
    /// fail or panic.
    // First production caller arrives with the per-event handlers; exercised by
    // this module's tests today.
    #[allow(dead_code)]
    pub fn diagnostic(&self, message: impl Into<String>) {
        self.diagnostics.borrow_mut().push(message.into());
    }
}

/// What a handler produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// What goes on stdout. [`Response::Silent`] means nothing at all.
    pub response: Response,
    /// A short human-readable note for the heartbeat row's `detail` — what the
    /// handler decided, in a few words. `None` leaves the field off.
    pub detail: Option<String>,
}

impl Outcome {
    /// A handler that did its work and has nothing to say on the wire.
    pub fn silent() -> Self {
        Self {
            response: Response::Silent,
            detail: None,
        }
    }

    /// A handler that wants `response` written to stdout.
    // Used by the per-event handlers as they land; the stubs below return
    // `silent`.
    #[allow(dead_code)]
    pub fn new(response: Response) -> Self {
        Self {
            response,
            detail: None,
        }
    }

    /// Attach the heartbeat note.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// The signature every per-event handler implements.
///
/// A plain `fn` pointer rather than a trait: there is exactly one
/// implementation per event, they hold no state, and a table of function
/// pointers is something a reader can check against the event list at a
/// glance.
pub type Handler = fn(&HookContext<'_>) -> Result<Outcome>;

// ── The routing table ─────────────────────────────────────────────────────────

/// How one event is handled.
#[derive(Clone, Copy)]
pub struct Route {
    /// The module that owns this event's handler. Used by the routing test and
    /// by nothing else — it exists so "did I wire the new module to the right
    /// event" is a question a test can answer.
    #[allow(dead_code)]
    pub module: &'static str,
    /// The handler itself.
    pub handler: Handler,
    /// Whether running this handler can write into `.superset/.magic/`, and so
    /// whether the ignored-tree gate applies to it.
    ///
    /// `session-start` scaffolds the tree and rewrites the session pointer;
    /// `pre-tool-use` reads and writes the conclusion cache and consumes
    /// one-shot bypass claims; `pre-compact` writes the compaction-survival
    /// note; `subagent-stop` writes its block-once claim and any salvaged
    /// transcript. The other two write nothing there at all: `session-end`
    /// appends to the machine-level cost ledger, and `file-changed` writes only
    /// to the environment file the harness hands it, which R92 requires to lie
    /// outside the worktree.
    pub writes_state: bool,
}

/// The routing table. `None` is R62's case — an event this binary cannot
/// route, which must look to the harness like a hook that did nothing.
pub fn route(event: &HookEvent) -> Option<Route> {
    let (module, handler, writes_state): (_, Handler, _) = match event {
        HookEvent::SessionStart => ("session_start", session_start::handle, true),
        // U14's Read gate is wired; U28 (the checklist deny) and U29 (the
        // commit nudge) plug into the same handler.
        HookEvent::PreToolUse => ("pre_tool_use", pre_tool_use::handle, true),
        HookEvent::PreCompact => ("pre_compact", pre_compact::handle, true),
        // U16.
        HookEvent::SubagentStop => ("subagent_stop", not_implemented, true),
        // U17.
        HookEvent::SessionEnd => ("session_end", not_implemented, false),
        // U30.
        HookEvent::FileChanged => ("file_changed", not_implemented, false),
        HookEvent::Unknown(_) | HookEvent::Missing => return None,
    };
    Some(Route {
        module,
        handler,
        writes_state,
    })
}

/// Stand-in for every per-event handler until its own unit lands.
///
/// Silent and successful on purpose: an unimplemented event must be
/// indistinguishable, from the harness's side, from an event whose handler
/// decided there was nothing to do. The heartbeat note is what tells the
/// difference, and it is the only place the difference shows.
fn not_implemented(_ctx: &HookContext<'_>) -> Result<Outcome> {
    Ok(Outcome::silent().with_detail("handler not implemented yet"))
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// The wiring the pipeline needs to the outside world, gathered in one place
/// so the whole thing can be driven from a test with in-memory buffers and a
/// temporary store.
struct HookIo<'a> {
    stdin: &'a mut dyn Read,
    stdout: &'a mut dyn Write,
    stderr: &'a mut dyn Write,
    /// The machine-level store the heartbeat row goes into. `None` when the
    /// platform has no resolvable app directory, in which case the invocation
    /// runs normally and simply leaves no row.
    store: Option<PathBuf>,
}

/// Run one hook invocation. Always `Ok(ExitCode::SUCCESS)`.
pub fn run(event: &HookEvent) -> Result<ExitCode> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut io = HookIo {
        stdin: &mut stdin.lock(),
        stdout: &mut stdout.lock(),
        stderr: &mut stderr.lock(),
        store: heartbeat::store_dir(),
    };
    run_with(event, &mut io, now_secs());
    // Every path above already handled its own failure. There is deliberately
    // no branch here that could produce another code.
    Ok(ExitCode::SUCCESS)
}

/// The testable core: run the pipeline against injected I/O and clock, and
/// return the heartbeat row it recorded.
fn run_with(event: &HookEvent, io: &mut HookIo<'_>, now: u64) -> Row {
    run_with_route(event, route(event), io, now)
}

/// [`run_with`] with the routing decision supplied rather than looked up.
///
/// Production always passes [`route`]'s own answer. The parameter exists so a
/// test can point an event at a handler that fails, panics or writes a
/// diagnostic — behaviors the six real handlers must never exhibit, and which
/// therefore cannot be provoked through the real table.
fn run_with_route(event: &HookEvent, route: Option<Route>, io: &mut HookIo<'_>, now: u64) -> Row {
    let mut input = String::new();
    let read = io.stdin.read_to_string(&mut input);

    let row = match read {
        Ok(_) => pipeline(event, route, &input, io, now),
        Err(e) => Row::new(event.as_str(), now, RowOutcome::Error)
            .with_reason(REASON_STDIN)
            .with_detail(e.to_string()),
    };

    // R50: the row goes down last, so it records what actually happened
    // including the stdout write. A failure to record it is reported and
    // dropped — the hook has already done its job, and refusing to exit
    // because a log line did not land would be exactly the loud failure this
    // whole module exists to avoid.
    if let Some(store) = io.store.as_deref() {
        if let Err(e) = heartbeat::append(store, &row) {
            let _ = writeln!(
                io.stderr,
                "ss-magic: could not record the hook heartbeat: {e:#}"
            );
        }
    }

    row
}

/// Decode, gate, dispatch, encode. Returns the row describing what happened;
/// writing that row is the caller's job.
fn pipeline(
    event: &HookEvent,
    route: Option<Route>,
    input: &str,
    io: &mut HookIo<'_>,
    now: u64,
) -> Row {
    let base = || Row::new(event.as_str(), now, RowOutcome::NoOp);

    // R62 — screened before anything else looks at the payload, because for an
    // event we cannot route there is no payload shape to decode against. The
    // envelope's `cwd` is still worth having in the row, so it is fished out of
    // the raw JSON.
    let Some(route) = route else {
        return base()
            .with_cwd(event::cwd_hint(input))
            .with_reason(REASON_UNROUTABLE)
            .with_detail(format!(
                "`{}` is not an event this build routes; ignored",
                event.as_str()
            ));
    };

    let envelope = match event::decode(event, input) {
        Ok(envelope) => envelope,
        // No envelope at all is what a person running the verb by hand from a
        // terminal produces. Not a failure of ours, so it is a no-op rather
        // than an error, and nothing is printed on either channel.
        Err(DecodeError::NoInput) => {
            return base()
                .with_reason(DecodeError::NoInput.class())
                .with_detail(DecodeError::NoInput.to_string());
        }
        Err(e) => {
            return Row::new(event.as_str(), now, RowOutcome::Error)
                .with_cwd(event::cwd_hint(input))
                .with_reason(e.class())
                .with_detail(e.to_string());
        }
    };

    let cwd = PathBuf::from(&envelope.common.cwd);
    let base = || base().with_cwd(Some(envelope.common.cwd.clone()));

    // A `cwd` that is not a directory means the worktree moved or was deleted
    // between the harness spawning us and us running. Reported as its own class
    // rather than falling through to "disabled", which would be true but
    // misleading.
    if !cwd.is_dir() {
        return base()
            .with_reason(REASON_CWD_MISSING)
            .with_detail(format!("{} is not a directory", cwd.display()));
    }

    // Outside a git repository there is no root to resolve against, so the
    // config resolution falls back to `cwd` itself and degrades to its safe
    // defaults — which means `enabled` is false and the invocation stops at the
    // next gate.
    let repo_root = git::cwd_repo_root(&cwd).ok();
    let config_root = repo_root.clone().unwrap_or_else(|| cwd.clone());
    let config = config::resolve(&config_root);

    // R55 — resolved from disk on this invocation, so disabling the plugin
    // takes effect on the next hook and not on the next session.
    if !config.enabled {
        return base()
            .with_reason(REASON_DISABLED)
            .with_detail("`plugin.enabled` is not true for this repository; nothing else ran");
    }

    // R63 — the ignored-tree precondition, enforced once, here. Per-event
    // modules do not repeat it.
    if route.writes_state {
        if let Some(detail) = state_tree_refusal(&config_root) {
            return base().with_reason(REASON_NOT_IGNORED).with_detail(detail);
        }
    }

    let ctx = HookContext {
        event,
        envelope: &envelope,
        repo_root,
        config_root,
        config: &config,
        now,
        diagnostics: std::cell::RefCell::new(Vec::new()),
    };

    // A handler that panics must not take the session with it, so the call is
    // caught the same way an `Err` is. `AssertUnwindSafe` is honest here: the
    // only state shared across the boundary is the diagnostics buffer, and a
    // half-filled diagnostics buffer is exactly what we want to print.
    let dispatched = std::panic::catch_unwind(AssertUnwindSafe(|| (route.handler)(&ctx)));

    // Whatever the handler managed to say goes out before we deal with how it
    // ended — a diagnostic explaining a failure is worth more than one that is
    // dropped because the failure happened.
    flush_diagnostics(&ctx, io);

    let outcome = match dispatched {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => {
            return base()
                .with_outcome(RowOutcome::Error)
                .with_reason(REASON_HANDLER_ERROR)
                .with_detail(format!("{e:#}"));
        }
        Err(payload) => {
            return base()
                .with_outcome(RowOutcome::Error)
                .with_reason(REASON_HANDLER_PANIC)
                .with_detail(panic_message(payload.as_ref()));
        }
    };

    match event::encode(&outcome.response) {
        Ok(Some(line)) => {
            if let Err(e) = writeln!(io.stdout, "{line}") {
                // stdout is gone (the harness closed the pipe, most likely).
                // Nothing to be done about it, and certainly nothing to fail
                // over.
                return base()
                    .with_outcome(RowOutcome::Error)
                    .with_reason(REASON_STDOUT)
                    .with_detail(format!("writing the response: {e}"));
            }
        }
        Ok(None) => {}
        Err(e) => {
            return base()
                .with_outcome(RowOutcome::Error)
                .with_reason(REASON_ENCODE_FAILED)
                .with_detail(e.to_string());
        }
    }

    let row = base().with_outcome(RowOutcome::Ok);
    match outcome.detail {
        Some(detail) => row.with_detail(detail),
        None => row,
    }
}

/// The ignored-tree gate: `None` when git reports `.superset/.magic/` ignored,
/// otherwise the sentence that goes in the heartbeat row.
///
/// The probe is rules-only. git's default `check-ignore` calls a directory
/// unignored the moment anything under it is tracked, which would turn a single
/// committed file inside the tree into a blanket refusal; the question this
/// gate asks is "would a file created here be invisible to git", which is what
/// `--no-index` answers.
fn state_tree_refusal(root: &Path) -> Option<String> {
    match git::is_ignored_no_index_str(root, STATE_QUERY) {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "git does not ignore {STATE_QUERY} — no covering rule in any .gitignore; \
             run `ss-magic plugin enable` (or `ss-magic init`) to add it. No state was written."
        )),
        // Fail closed: an unanswered question is not permission.
        Err(e) => Some(format!(
            "could not ask git whether {STATE_QUERY} is ignored: {e}. No state was written."
        )),
    }
}

/// Write the handler's collected diagnostics to stderr, one per line, with no
/// styling of any kind. Color is forced off for hook verbs at the process
/// level too, but these lines never reach a styling function in the first
/// place, so there is nothing here for a mis-set global to color.
fn flush_diagnostics(ctx: &HookContext<'_>, io: &mut HookIo<'_>) {
    for line in ctx.diagnostics.borrow().iter() {
        let _ = writeln!(io.stderr, "{line}");
    }
}

/// Recover whatever text a panic carried. `panic!` produces a `&str` or a
/// `String`; anything else is a caller doing something exotic and gets a
/// placeholder rather than an empty detail field.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "handler panicked with a non-string payload".to_string()
}

/// Seconds since the Unix epoch, or 0 if the system clock is set before it.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
