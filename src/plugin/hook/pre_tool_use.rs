//! `PreToolUse` — the page-fault gate (R21–R24, R41–R43, R52, R53).
//!
//! The harness never truncates a `Read`. Every other tool spills an oversized
//! result to disk and hands the model a two-kilobyte pointer; `Read` has no
//! such ceiling, which measurement made the single largest un-spilled context
//! sink there is — a 3,000-line read cost 32,060 cached input tokens, an
//! 8,000-line one 60,066, against ~6,600 for any spilled `Bash`. This module is
//! the answer to that: an oversized `Read` is **denied**, and the denial tells
//! the model to send an Explore agent instead. The agent reads the file in its
//! own window and records what it concluded; the next attempt at the same read
//! is denied again, but this time the denial carries that conclusion inline.
//!
//! ## Deny, never rewrite (R20, R81)
//!
//! A working `updatedInput` redirect was built and then rejected. The
//! transcript keeps the ORIGINAL tool call, so a rewritten `Read` leaves the
//! model believing it read the file it asked for — and concurrent hooks' input
//! rewrites are folded last-write-wins, so ours would silently discard the
//! user's own `rtk` wrapper's. The response types offer no rewrite channel at
//! all (see `hook/event.rs`), so this is structural rather than a rule to
//! remember. What makes the denial workable is that `permissionDecisionReason`
//! is uncapped and reaches the model verbatim with no wrapper text: the
//! conclusion rides inline at no fidelity cost, and the model is *told* it was
//! blocked and why. Uncapped also means unprotected, which is why the inline
//! text is bounded by [`crate::plugin::config::GateConfig::inline_byte_budget`]
//! (R23) rather than by the channel.
//!
//! The gate only ever emits `deny`. It never emits `allow`, so it can never
//! remove a capability the user would otherwise have had: everything it lets
//! past goes back into the ordinary permission flow untouched.
//!
//! ## The never-blocked-forever guarantee
//!
//! The same read is never denied *empty* twice. A miss routes to an Explore
//! agent; once that agent has concluded, every later attempt gets the answer.
//! And three separate escape hatches exist below that: a bounded window (R41),
//! a subagent's own read (R52) and the one-shot bypass (R42).
//!
//! ## Fail-open by construction
//!
//! Every uncertainty here allows the read. A file that cannot be stat'd, a
//! cache directory that cannot be reached, a path that resolves nowhere — all
//! of them fall through to "say nothing". This is a context-economy measure,
//! not a security boundary: a hook that exits non-zero, times out or emits a
//! malformed envelope does not block anything either, so a gate that tried to
//! be a boundary would be one with holes in it by design.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use globset::Glob;

use crate::plugin::bypass;
use crate::plugin::cache;
use crate::plugin::checklist;
use crate::plugin::hook::event::{
    Payload, PermissionDecision, PreToolUse, PreToolUseResponse, Response,
};
use crate::plugin::hook::{HookContext, Outcome};
use crate::plugin::scratchpad::STATE_REL;

// ── Binary-owned constants ───────────────────────────────────────────────────

/// Average bytes per line, used to convert the configured line threshold into
/// the byte figure a single `stat` can be compared against.
///
/// The gate is allowed exactly one `stat` before it decides (KTD4), and a
/// `stat` reports bytes while the threshold is stated in lines, so one of the
/// two has to be converted. 40 is measured rather than guessed: this crate's
/// own sources average 39.4 bytes per line across 27,000 lines, and it is the
/// right order of magnitude for source and configuration files generally.
/// Prose runs longer (a 594-line Markdown file measured at 149), so the gate
/// over-estimates a prose file's line count and errs towards gating it — the
/// safe direction, since prose is also what an Explore agent summarizes best.
const BYTES_PER_LINE: u64 = 40;

/// Extensions a `Read` is never gated on (KTD11, R43).
///
/// Images, PDFs and notebooks are what the harness renders specially rather
/// than as text, and a summary of one is not a substitute for looking at it.
/// The list ships in the binary and configuration can only ADD to it
/// ([`crate::plugin::config::GateConfig::exemptions`]), never shrink it, so no
/// configuration state can make a binary unviewable.
const NON_TEXT_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "ico", "tiff", "tif", "pdf", "ipynb",
];

/// How far up from the envelope's `cwd` the worktree walk looks before giving
/// up. Deep enough for any real checkout, bounded so a pathological path
/// cannot turn the walk into an unbounded loop.
const MAX_WALK_DEPTH: usize = 64;

/// The file whose presence marks a worktree root. This is the `.superset`
/// contract file every ss-magic repository has, which is what lets the walk
/// answer "which worktree is this" without asking git.
const ROOT_MARKER: &str = ".superset/magic.json";

// ── The checklist deny (R88) ─────────────────────────────────────────────────

/// What the checklist classifier decided about a tool's target path.
///
/// Its position in [`gate`] is load-bearing: the classification runs FIRST,
/// ahead of every exemption below it. The reason is specific. A checklist file
/// is meant to be reached through the `ss-magic-plugin checklist` verbs, which
/// render it through the binary's own renderer inside an untrusted-data
/// envelope. If the exemptions ran first, an
/// agent dispatched to "go read the checklist" would be waved through by the
/// subagent exemption and pull the raw document into its context, and from
/// there into its report — precisely the leak the verbs exist to prevent. The
/// state-tree exemption fails the same way for a checklist reached through the
/// pointer inside `.superset/.magic/`.
///
/// The classification is size-independent and applies to `Read`, `Edit`,
/// `Write` and notebook edits alike, which is why it sits above the branch
/// where everything but `Read` is waved through.
pub(crate) enum Classification {
    /// Not a checklist path. The ordinary decision order continues.
    Ordinary,
    /// A checklist path. The gate denies immediately with this reason, which
    /// is the "use the checklist verb" text — never the Explore-routing text,
    /// even for a checklist large enough to trip the size threshold too. The
    /// two denials lead to different places and the model must get the right
    /// one.
    Checklist { reason: String },
}

/// How [`gate`] asks whether a path is a checklist file. A function pointer
/// rather than a direct call so the ordering can be tested independently of
/// what counts as a checklist: a test supplies a classifier that says
/// "checklist" for everything and asserts the deny still wins over a subagent
/// read, a state-tree path, a `.png` and an `Edit`.
type Classifier = fn(&HookContext<'_>, &Path) -> Classification;

/// The shipped classifier: is this path the operator checklist?
///
/// Two routes count, because there are two ways to arrive at the document.
/// The pointer at `.superset/.magic/checklist.json` names the active
/// checklist, and the committed naming convention
/// (`docs/actions/<stem>.checklist.json`) identifies one in a repository that
/// has no pointer yet. Either is enough — a repository can have both, and a
/// checklist reached by the pointer and the same checklist reached by its path
/// are the same file and get the same answer.
fn classify_checklist(ctx: &HookContext<'_>, realpath: &Path) -> Classification {
    let root = classification_root(ctx);
    let resolved = absolutize(ctx.cwd(), realpath);
    if !is_checklist_path(&root, &resolved) {
        return Classification::Ordinary;
    }
    Classification::Checklist {
        reason: checklist_reason(tool_label(ctx), &display_path(&root, &resolved)),
    }
}

/// The worktree the checklist is looked for in.
///
/// The same root the rest of the gate uses, resolved the same way. The walk
/// canonicalizes as it goes, so its answer already matches the realpath the
/// gate hands the classifier; only the last-resort fallback has been through
/// no such walk and needs resolving here.
fn classification_root(ctx: &HookContext<'_>) -> PathBuf {
    worktree_root(ctx.cwd()).unwrap_or_else(|| {
        ctx.config_root
            .canonicalize()
            .unwrap_or_else(|_| ctx.config_root.clone())
    })
}

/// The tool's target as an absolute path, without touching the filesystem.
///
/// [`gate`] canonicalizes the target, which resolves it fully — but only when
/// the file is there. A `Write` creating a checklist that does not exist yet
/// leaves the path exactly as the tool spelled it, and the harness's tools take
/// absolute paths, so that is normally already absolute. A relative one is
/// joined onto the envelope's `cwd` rather than left to fail the comparison:
/// otherwise an agent could reach the file simply by spelling the path
/// differently. Stat'ing here would defeat the point, since the file this
/// branch exists for is precisely the one that is not there.
fn absolutize(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    cwd.canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .join(path)
}

/// Whether `path` is the checklist, by either route.
fn is_checklist_path(root: &Path, path: &Path) -> bool {
    // Purely lexical and free, so it goes first — and it is the only one of the
    // two that answers at all before `checklist init` has ever run.
    if checklist::matches_convention(root, path) {
        return true;
    }

    let Some(target) = checklist::pointer_target(root) else {
        return false;
    };

    // The pointer records the intended path and is deliberately never stat'd,
    // so a checklist that has not been written yet still compares equal here.
    // That matters: without it an agent could read (or create) the document by
    // getting to it before `init` does.
    if target == path {
        return true;
    }

    // Only worth resolving once the cheap comparison has failed: the pointer
    // may spell the path through a symlinked directory, in which case the two
    // agree only after both sides are resolved. A target that is not there
    // fails this, which is exactly right — the lexical comparison above is the
    // answer for that case.
    target.canonicalize().is_ok_and(|resolved| resolved == path)
}

/// The tool's own name, for the first line of the denial. Reads it back out of
/// the envelope so the sentence says `Read`, `Edit` or `Write` rather than
/// something generic; the fallback is unreachable through [`gate`], which has
/// already matched the variant.
fn tool_label<'a>(ctx: &'a HookContext<'_>) -> &'a str {
    match &ctx.envelope.payload {
        Payload::PreToolUse(payload) => payload.tool_name.as_str(),
        _ => "tool call",
    }
}

/// The checklist denial (R88): what to run instead of reading or writing the
/// file.
///
/// It deliberately says nothing about Explore agents. The page-fault denials
/// further down route an oversized read to a subagent, and a model handed that
/// instruction for a checklist would dispatch an agent that pulls the raw
/// document into ITS window and repeats it in a report — the leak the verbs
/// exist to prevent by rendering the document through the binary instead. Size
/// never enters into it either: a checklist is machine-written bookkeeping
/// whose useful form is what `list` and `render-md` print, so there is nothing
/// a summary of the raw bytes would add.
///
/// The commands are spelled `ss-magic-plugin`, the wrapper on the Bash tool's
/// PATH, and not a bare `ss-magic`: the model runs these through Bash, where
/// `${CLAUDE_PLUGIN_DATA}` is not exported and the bootstrapped binary cannot
/// be named directly. `session_start.rs` spells the same family the same way,
/// for the same reason.
fn checklist_reason(tool: &str, shown: &str) -> String {
    format!(
        "ss-magic blocked this {tool}: {shown} is the operator checklist, which is \
         reached through its own commands rather than by reading or writing the file \
         directly.\n\
         \n\
         The file is machine-written JSON. Reading it raw spends your context on \
         bookkeeping you do not need, and hand-editing it loses the canonical ordering \
         and the validation every write goes through. Run these instead:\n\
         \n\
         \x20 to see what it says\n\
         \x20      ss-magic-plugin checklist list        the checklist, rendered\n\
         \x20      ss-magic-plugin checklist render-md   the full Markdown rendering\n\
         \x20      ss-magic-plugin checklist verify      what is missing or inconsistent\n\
         \n\
         \x20 to change it\n\
         \x20      ss-magic-plugin checklist init <slug>\n\
         \x20      ss-magic-plugin checklist add-item <section> <id> [TITLE]\n\
         \x20      ss-magic-plugin checklist add-entry <id> [SUMMARY]\n\
         \x20      ss-magic-plugin checklist set <id> <dotted-key> <value>\n\
         \x20      ss-magic-plugin checklist done <id>\n\
         \n\
         Start with `ss-magic-plugin checklist list`. It tells you what this file says \
         without putting the file in your window.\n"
    )
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// The `PreToolUse` handler wired into [`crate::plugin::hook::route`].
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    gate(ctx, classify_checklist)
}

/// Which tool fired, as far as this gate is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateTool {
    /// The one tool the size machinery applies to.
    Read,
    /// `Edit` / `Write` / notebook edits. Matched only so the checklist
    /// classification above can see them; there is no context to save on a
    /// write, so they never reach the size machinery.
    Mutating,
    /// `Grep` / `Glob`. Matched for forward compatibility and deliberately
    /// INERT: neither tool exists in this user's environment (a live session
    /// enumerated its tools and found neither; the model falls back to
    /// `Bash grep`), so their behavior could not be exercised and nothing here
    /// acts on them.
    Search,
    /// Anything else, `Bash` included.
    Other,
}

impl GateTool {
    fn from_name(name: &str) -> Self {
        match name {
            "Read" => Self::Read,
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => Self::Mutating,
            "Grep" | "Glob" => Self::Search,
            _ => Self::Other,
        }
    }
}

/// The decision order, in one function, in the order KTD4 fixes:
///
/// 1. **config** — the gate's tunables, already resolved for the envelope's
///    `cwd` by the wrapper (R53). The wrapper also ran R55's enablement gate,
///    so a repository with the plugin off never reaches this function at all.
/// 2. **checklist** (R88) — denied outright, ahead of every exemption.
/// 3. **tool** — only `Read` reaches the size machinery.
/// 4. **state tree** (R43) — `.superset/.magic/`, unconditionally.
/// 5. **subagent** (R52) — the agent the gate routes to must be able to read.
/// 6. **non-text** (R43) — the binary-owned extension list.
/// 7. **configured exemptions** (R53) — patterns this repository added.
/// 8. **one stat** — the only filesystem call on the under-threshold path.
/// 9. **threshold** — under it, the read goes through untouched.
/// 10. **window** (R41) — a bounded `offset`/`limit` goes through too.
/// 11. **bypass** (R42) — a one-shot claim, consumed here.
/// 12. **cache** — hit serves the conclusion, miss routes to an Explore agent.
fn gate(ctx: &HookContext<'_>, classify: Classifier) -> Result<Outcome> {
    let Payload::PreToolUse(payload) = &ctx.envelope.payload else {
        // Unreachable through `hook::route`, which builds the variant from the
        // argv token. Saying nothing beats a `match` arm that could panic on a
        // future wiring mistake.
        return Ok(allow("allow: payload is not a PreToolUse envelope"));
    };

    let tool = GateTool::from_name(&payload.tool_name);
    let target = target_path(payload);

    // ── 2. Checklist classification (R88) ────────────────────────────────────
    // Before every exemption, and before the tool branch, because it is the one
    // decision that applies to writes as well as reads. The target is resolved
    // first so reaching the file through a symlink, or through the pointer, is
    // the same case as naming it outright; a target that is not there yet keeps
    // the path as spelled, which the classifier compares lexically.
    if let Some(target) = &target {
        let realpath = target.canonicalize().unwrap_or_else(|_| target.clone());
        if let Classification::Checklist { reason } = classify(ctx, &realpath) {
            return Ok(deny(reason, "deny: checklist path"));
        }
    }

    // ── 3. Only `Read` reaches the size machinery ────────────────────────────
    if tool != GateTool::Read {
        return Ok(allow(match tool {
            GateTool::Mutating => "allow: not a Read (nothing to save on a write)",
            GateTool::Search => "allow: Grep/Glob matcher is inert",
            _ => "allow: tool is not gated",
        }));
    }

    let Some(target) = target else {
        return Ok(allow("allow: the Read carries no file_path"));
    };

    // ── 4. The state tree (R43) ──────────────────────────────────────────────
    // The scratchpad's own STATUS.md was measured at 594 lines / 88.7 KB, and
    // the shipped skill tells the model to read it first on resume. Gating it
    // would deny the read the scratchpad exists to serve — a session that just
    // survived a compaction re-orienting from its own notes. A path test, so it
    // costs nothing and never touches the cache key.
    if in_state_tree(&target) {
        return Ok(allow("allow: inside the .superset/.magic state tree"));
    }

    // ── 5. A subagent's own read (R52) ───────────────────────────────────────
    // Without this the routing eats itself: the Explore agent the miss branch
    // dispatches reads the same file, is denied, and is told to dispatch
    // another Explore agent, forever.
    if let Some(agent) = subagent_label(payload) {
        return Ok(allow(format!("allow: issued inside a subagent ({agent})")));
    }

    // ── 6. Non-text extensions (KTD11, R43) ──────────────────────────────────
    if let Some(ext) = non_text_extension(&target) {
        return Ok(allow(format!("allow: non-text extension .{ext}")));
    }

    // From here on the worktree root matters, for the exemption patterns and
    // for the state directories the last two steps read.
    let root = worktree_root(ctx.cwd()).unwrap_or_else(|| ctx.config_root.clone());

    // ── 7. Configured exemptions (R53) ───────────────────────────────────────
    if let Some(pattern) = configured_exemption(&ctx.config.gate.exemptions, &root, &target) {
        return Ok(allow(format!("allow: exemption pattern `{pattern}`")));
    }

    // ── 8. One stat ──────────────────────────────────────────────────────────
    // A file that is not there, or is a directory, is not ours to refuse: the
    // Read fails (or succeeds) on its own terms and says something useful,
    // which a deny reason about context economy would only obscure.
    let Ok(meta) = fs::metadata(&target) else {
        return Ok(allow("allow: target could not be stat'd"));
    };
    if !meta.is_file() {
        return Ok(allow("allow: target is not a regular file"));
    }
    let size = meta.len();

    // ── 9. The threshold ─────────────────────────────────────────────────────
    let threshold_lines = ctx.config.gate.threshold_lines;
    let limit_bytes = threshold_bytes(threshold_lines);
    if size <= limit_bytes {
        return Ok(allow(format!(
            "allow: under the gate ({size} B <= {limit_bytes} B)"
        )));
    }

    // ── 10. A bounded window (R41) ───────────────────────────────────────────
    // Asking for 200 lines of a 400 KB file costs 200 lines of context, so
    // there is nothing to save by blocking it. The cache key is unaffected:
    // it fingerprints the file, never the window (R24), so a windowed read and
    // a whole-file read of the same bytes share one conclusion.
    let window = window_bytes(size, read_u64(payload, "offset"), read_u64(payload, "limit"));
    if window <= limit_bytes {
        return Ok(allow(format!(
            "allow: requested window is bounded ({window} B <= {limit_bytes} B)"
        )));
    }

    // Everything below needs the file's identity — for the bypass claim and for
    // the cache key alike. Losing it now means the file changed under us
    // between the stat and here, which is a race we resolve by letting the read
    // through rather than denying on a stale fact.
    let Ok(identity) = cache::identify(&target) else {
        return Ok(allow("allow: the target vanished before it could be keyed"));
    };

    // ── 11. The one-shot bypass (R42) ────────────────────────────────────────
    if bypass::consume(&bypass::dir_for_root(&root), &identity.realpath, ctx.now) {
        return Ok(allow("allow: consumed a one-shot bypass claim"));
    }

    // ── 12. The conclusion cache ─────────────────────────────────────────────
    let shown = display_path(&root, &target);
    let key = identity.key();
    let cache_dir = cache::dir_for_root(&root);
    let entry = cache::entry_path(&cache_dir, &key);

    // The budget covers the whole denial, so what is left for the conclusion is
    // whatever ss-magic's own framing did not use. The cache's renderer never
    // cuts the untrusted-data envelope or the stamped header — they are what
    // make the quoted text safe to read at all — so only the agent's body
    // shrinks, and a budget too small to fit the framing yields a slightly
    // longer denial rather than a mutilated one.
    let head = hit_preamble(&shown, size, threshold_lines);
    let tail = hit_epilogue(&shown);
    let body_budget =
        (ctx.config.gate.inline_byte_budget as usize).saturating_sub(head.len() + tail.len());

    match cache::render_cached(&cache_dir, &key, cache::Budget::Bytes(body_budget)) {
        Some(rendered) => Ok(deny(
            format!("{head}{rendered}{tail}"),
            format!("deny: served the cached conclusion for key {key}"),
        )),
        None => Ok(deny(
            miss_reason(
                &shown,
                size,
                threshold_lines,
                &display_path(&root, &entry),
            ),
            format!("deny: no conclusion cached for key {key}"),
        )),
    }
}

// ── Outcomes ─────────────────────────────────────────────────────────────────

/// Say nothing. The read goes back into the ordinary permission flow exactly as
/// it arrived — no decision, no context, no rewrite.
fn allow(detail: impl Into<String>) -> Outcome {
    Outcome::silent().with_detail(detail)
}

/// Block the call, with `reason` as the text the model sees.
fn deny(reason: impl Into<String>, detail: impl Into<String>) -> Outcome {
    Outcome::new(Response::PreToolUse(PreToolUseResponse {
        decision: Some(PermissionDecision::Deny),
        reason: Some(reason.into()),
        ..PreToolUseResponse::default()
    }))
    .with_detail(detail)
}

// ── Reading the tool input ───────────────────────────────────────────────────

/// The path this tool call targets, if it names one. `NotebookEdit` spells the
/// key `notebook_path`; everything else uses `file_path`.
fn target_path(payload: &PreToolUse) -> Option<PathBuf> {
    for key in ["file_path", "notebook_path"] {
        if let Some(value) = payload.tool_input.get(key).and_then(|v| v.as_str()) {
            if !value.is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

/// A short label for the dispatching agent when this call came from inside one,
/// or `None` when it came from the main thread.
///
/// Either field alone is enough. The hook contract confirmed both on
/// `SubagentStop` but could not confirm them on `PreToolUse`, so this reads
/// whichever arrives rather than requiring the pair — and if neither ever
/// arrives, the exemption simply never fires and a subagent's read is gated,
/// which the cache hit still answers on the second attempt.
fn subagent_label(payload: &PreToolUse) -> Option<&str> {
    payload
        .agent_type
        .as_deref()
        .or(payload.agent_id.as_deref())
        .filter(|s| !s.is_empty())
}

/// One numeric key out of the untyped `tool_input`.
///
/// A model can spell a number as a JSON number or, occasionally, as a string,
/// and either is worth honoring: reading `limit` as absent when the model
/// actually sent one would deny a read the window rule was meant to allow. A
/// negative or unparseable value reads as absent, which is the conservative
/// answer — the gate then prices the window as unbounded.
fn read_u64(payload: &PreToolUse, key: &str) -> Option<u64> {
    let value = payload.tool_input.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

// ── The exemptions ───────────────────────────────────────────────────────────

/// Whether `path` sits inside `.superset/.magic/`.
///
/// Matched on the two components adjacent to each other, anywhere in the path,
/// which is the same thing as the exact two-component prefix relative to a
/// worktree root — and it works without knowing the root, so a state file
/// reached through an unusual path still matches. It must never widen to a bare
/// `.superset`: that directory holds the committed contract files, and an
/// over-threshold `.superset/magic.json` is still gated.
fn in_state_tree(path: &Path) -> bool {
    let (parent, leaf) = STATE_REL.split_once('/').expect("STATE_REL has two components");
    let components: Vec<_> = path.components().collect();
    components.windows(2).any(|pair| {
        pair[0].as_os_str() == parent && pair[1].as_os_str() == leaf
    })
}

/// The binary-owned non-text extension, lowercased, if `path` has one.
fn non_text_extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    NON_TEXT_EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

/// The first configured exemption pattern matching `path`, if any.
///
/// Patterns are matched against the worktree-relative path — what a person
/// writing `docs/**` in `magic.json` means — and, for a path outside the
/// worktree entirely, against the absolute path so an absolute pattern still
/// works. A pattern that does not compile is dropped rather than failing the
/// gate: that can only shrink the exemption list and make the gate fire more
/// often, which is the safe direction.
fn configured_exemption<'a>(
    patterns: &'a [String],
    root: &Path,
    path: &Path,
) -> Option<&'a str> {
    if patterns.is_empty() {
        return None;
    }
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let relative = absolute.strip_prefix(root).unwrap_or(&absolute);

    patterns.iter().find_map(|pattern| {
        let matcher = Glob::new(pattern).ok()?.compile_matcher();
        (matcher.is_match(relative) || matcher.is_match(&absolute)).then_some(pattern.as_str())
    })
}

// ── Size arithmetic ──────────────────────────────────────────────────────────

/// The configured line threshold as a byte count, which is what a single
/// `stat` can be compared against.
fn threshold_bytes(threshold_lines: u32) -> u64 {
    u64::from(threshold_lines) * BYTES_PER_LINE
}

/// What the requested window is estimated to cost, in bytes (R41).
///
/// `offset` is a 1-based line number and `limit` a line count, so both have to
/// be priced in bytes before they can be compared against a threshold a `stat`
/// produced. The conversion uses the file's own average line length — its size
/// divided by the number of lines it is estimated to hold — so a window of the
/// same line count costs more in a file of long lines than in one of short
/// ones. An absent `limit` means "to the end", which is the whole remainder.
fn window_bytes(size: u64, offset: Option<u64>, limit: Option<u64>) -> u64 {
    // Both divisions round UP, which is what keeps the unbounded case honest:
    // rounding down would make a whole-file window price out at slightly LESS
    // than the file (3,000 lines of 40 bytes for a 120,001-byte file), and a
    // file one byte past the gate would let itself through. Rounding up
    // guarantees `estimated_lines * average_line >= size`.
    let estimated_lines = size.div_ceil(BYTES_PER_LINE).max(1);
    let average_line = size.div_ceil(estimated_lines).max(1);

    // `offset` names the first line to return, so the lines skipped are one
    // fewer. An offset past the end leaves nothing to read, and a window of
    // nothing costs nothing.
    let skipped = offset.unwrap_or(1).saturating_sub(1);
    let remaining = estimated_lines.saturating_sub(skipped);
    let lines = match limit {
        Some(limit) => limit.min(remaining),
        None => remaining,
    };
    lines.saturating_mul(average_line)
}

// ── Locating the worktree (KTD4) ─────────────────────────────────────────────

/// Memoized [`walk_for_root`], per `cwd`, for the life of the process.
///
/// One hook invocation asks more than once — for the exemption patterns, the
/// bypass directory, the cache directory and the paths the deny reason
/// prints — and each answer is the same walk over the same unchanging
/// directories.
fn worktree_root(cwd: &Path) -> Option<PathBuf> {
    static MEMO: OnceLock<Mutex<HashMap<PathBuf, Option<PathBuf>>>> = OnceLock::new();
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));

    // A poisoned lock means another thread panicked mid-insert. The memo is a
    // cache of a pure function, so recomputing is always correct; taking the
    // inner value keeps a panic in one test from breaking every later caller.
    let mut memo = memo.lock().unwrap_or_else(|e| e.into_inner());
    memo.entry(cwd.to_path_buf())
        .or_insert_with(|| walk_for_root(cwd))
        .clone()
}

/// Find the worktree root containing `cwd` by walking up for
/// `.superset/magic.json`.
///
/// Deliberately a filesystem walk and not `git rev-parse`: this runs on every
/// `PreToolUse`, which is the most frequent hook there is, and spawning a
/// subprocess per tool call to answer a question a handful of `stat`s answer is
/// a cost the user pays on every keystroke's worth of work. The walk is bounded
/// by [`MAX_WALK_DEPTH`] and stops at the filesystem root.
fn walk_for_root(cwd: &Path) -> Option<PathBuf> {
    let start = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut current = start.as_path();
    for _ in 0..MAX_WALK_DEPTH {
        if current.join(ROOT_MARKER).is_file() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

/// `path` relative to `root` for display, falling back to the path as given
/// when it does not sit under `root` — the model asked for a path and should
/// see one it recognizes, not a `/private/var/...` form it has never seen.
fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

// ── Deny reasons ─────────────────────────────────────────────────────────────

/// A size in whole kilobytes, for a sentence rather than a table.
fn kb(size: u64) -> u64 {
    size.div_ceil(1024)
}

/// `103000` as `103,000`. The deny reason is prose the model reads once, and a
/// bare run of digits is exactly the kind of thing that gets misread at a
/// glance.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The text preceding a cached conclusion. Everything here is ss-magic's own
/// prose; the conclusion itself follows inside U13's untrusted-data envelope,
/// whose framing has to be the first thing read about the quoted text (R64) and
/// so must not be preceded by anything that describes the text as guidance.
fn hit_preamble(shown: &str, size: u64, threshold_lines: u32) -> String {
    format!(
        "ss-magic blocked this Read: {shown} is {} KB, past this repository's \
         {threshold_lines}-line gate. You do not need the file — a conclusion about it \
         was recorded earlier and is quoted below.\n\n",
        kb(size)
    )
}

/// The text following a cached conclusion: what to do if it did not answer the
/// question, including the bypass invocation verbatim (R42).
fn hit_epilogue(shown: &str) -> String {
    format!(
        "\nIf that does not answer your question, dispatch an Explore agent to read \
         {shown} — the gate does not apply inside a subagent — and have it record what \
         it finds with `ss-magic plugin conclude {shown}`, replacing the entry above.\n\
         \n\
         To read the raw bytes in THIS window instead, run exactly:\n\
         \n    ss-magic plugin bypass {shown}\n\n\
         and read the file again. That lets exactly the next Read of this file through, \
         once; the one after it is gated again.\n"
    )
}

/// The miss branch: no conclusion exists, so the denial has to explain how one
/// gets made. It names the cache path and routes the work to an Explore agent
/// (R21), and ends with the bypass invocation verbatim (R42).
fn miss_reason(shown: &str, size: u64, threshold_lines: u32, entry: &str) -> String {
    format!(
        "ss-magic blocked this Read to keep it out of your context window.\n\
         \n\
         \x20 file       {shown}\n\
         \x20 size       {size_kb} KB, past this repository's {threshold_lines}-line gate\n\
         \x20 estimate   reading it here would cost roughly {tokens} tokens\n\
         \n\
         Nothing is cached for this file yet, so there is no summary to hand you \
         instead. Get one made:\n\
         \n\
         1. Dispatch an Explore agent and ask it to read {shown} and answer the question \
         you actually have about it. The gate does not apply inside a subagent, so it can \
         read the whole file.\n\
         2. Have that agent record what it found, so the file is never read twice:\n\
         \n\
         \x20      ss-magic plugin conclude {shown} <<'EOF'\n\
         \x20      <what it found>\n\
         \x20      EOF\n\
         \n\
         \x20  The entry lands at {entry}.\n\
         3. Read the file again. The Read is still blocked, but the block then carries \
         that conclusion inline — the answer, without the file.\n\
         \n\
         A narrower read is not blocked: a Read with `limit` at or under \
         {threshold_lines} lines goes through untouched.\n\
         \n\
         If you truly need the raw bytes in THIS window, run exactly:\n\
         \n    ss-magic plugin bypass {shown}\n\n\
         and read the file again. That lets exactly the next Read of this file through, \
         once; the one after it is gated again.\n",
        size_kb = kb(size),
        // page-fault.md's measured rule of thumb: roughly one token per four
        // bytes of text.
        tokens = grouped(size / 4),
    )
}

#[cfg(test)]
mod tests;
