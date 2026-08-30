//! `SessionStart` — the scratchpad bootstrap and the operating-guidance
//! injection (F2, R19).
//!
//! Fires on all five sources the harness has: `startup`, `resume`, `clear`,
//! `compact` and `fork`. `compact` is why this handler runs on every one of
//! them rather than only `startup` — after a compaction it is the only
//! remaining signal telling the model where its own working memory lives,
//! since `PreCompact` has no model-facing channel of its own at all. The
//! handler is a thin caller: [`scratchpad::ensure`] (U8) owns every rule
//! about what gets created, rewritten or refused in `.superset/.magic/`;
//! this module only turns its [`scratchpad::Report`] into guidance text and
//! decides whether a version-drift notice rides along.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::plugin::hook::event::{Payload, Response};
use crate::plugin::hook::{HookContext, Outcome};
use crate::plugin::scratchpad::{self, Report};

/// The `ss-magic plugin checklist` verb family (R89, R90), spelled out here
/// only so the injected guidance can name them. The schema and the verbs
/// themselves belong to U27; this module never reads, writes, or even checks
/// for `checklist.json`'s existence — it states where the pointer will
/// resolve and how to reach it, nothing more.
const CHECKLIST_VERBS: &str = "\
    ss-magic plugin checklist init <slug>
    ss-magic plugin checklist add-item <section> <id>
    ss-magic plugin checklist add-entry <id>
    ss-magic plugin checklist set <id> <dotted-key> <value>
    ss-magic plugin checklist done <id>
    ss-magic plugin checklist list
    ss-magic plugin checklist verify";

/// The pointer's file name inside the state root (R89) — not yet written by
/// anything in this codebase (U27 owns that write path). Named here only so
/// the guidance can say where it will resolve.
const CHECKLIST_POINTER_NAME: &str = "checklist.json";

/// `(file name, one-line description)`, in the exact order and spelling of
/// [`scratchpad::STATE_FILES`]. This table exists purely to describe files
/// U8 scaffolds — a name here that U8 does not actually create would be
/// guidance worse than none.
const STATE_FILE_NOTES: [(&str, &str); 6] = [
    (
        "CONTEXT.md",
        "context that would be expensive to rediscover, grouped by topic",
    ),
    (
        "DECISIONS.md",
        "settled decisions, with the reasoning behind each",
    ),
    (
        "LEARNINGS.md",
        "append-only — add `## <timestamp> - <label>`, never edit an older one",
    ),
    (
        "OPERATOR-CHECKLIST.md",
        "your own running notes on operational steps (not the repo checklist below)",
    ),
    (
        "STATUS.md",
        "newest block first — demote an old block to history, never delete it",
    ),
    ("TASKS.md", "the task list and where each item stands"),
];

/// The `SessionStart` handler wired into [`crate::plugin::hook::route`].
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    let source = match &ctx.envelope.payload {
        Payload::SessionStart(session_start) => session_start.source.as_str(),
        // Unreachable through `hook::route` — decoding a `SessionStart` event
        // always produces this variant — but falling back to an empty,
        // unrecognized-looking source is safer than a `match` arm that could
        // panic on a future wiring mistake.
        _ => "",
    };

    // R15 — outside a git repository there is no worktree to scaffold and
    // nothing else to do. Most such invocations already stop at the
    // `disabled` gate in `hook/mod.rs` (a non-repository `cwd` resolves no
    // `plugin.enabled` value to be true), but `HookContext::repo_root`'s own
    // contract is that a handler needing a repository checks this itself.
    let Some(repo_root) = ctx.repo_root.as_deref() else {
        return Ok(Outcome::silent().with_detail("not inside a git repository; nothing to do"));
    };

    let report = scratchpad::ensure(ctx.cwd())?;
    let additional_context = build_guidance(repo_root, &report);
    let system_message = version_drift_notice(plugin_root());

    let detail = if source.is_empty() {
        report.heartbeat_note()
    } else {
        format!("{} (source: {source})", report.heartbeat_note())
    };

    Ok(Outcome::new(Response::SessionStart {
        additional_context: Some(additional_context),
        system_message,
    })
    .with_detail(detail))
}

/// `${CLAUDE_PLUGIN_ROOT}`, if the harness set it. Read once here, separately
/// from [`version_drift_notice`], so that function stays a plain
/// path-in/string-out helper a test can drive without touching the process
/// environment.
fn plugin_root() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_PLUGIN_ROOT").map(PathBuf::from)
}

/// Compare this binary's own compiled-in version against the plugin's
/// declared pin at `<plugin_root>/ss-magic.version`, and name a mismatch as a
/// `systemMessage` — an operator notice, never model-facing text (see
/// `hook-contract.md`'s "`systemMessage` is a user/SDK channel").
///
/// This is R77's mixed-version window made visible: the bootstrap script
/// reinstalls the pinned binary only on the `startup` source, so a session
/// that starts right after a plugin update can run this very handler off the
/// *previous* binary while the plugin tree it was loaded from already names
/// the new one. Best-effort throughout — a missing `CLAUDE_PLUGIN_ROOT`
/// (this binary invoked outside a plugin install, e.g. by hand or in a
/// non-plugin test), an unreadable pin file, or one with nothing usable in it
/// all mean "nothing to report", never a reason to fail the hook.
fn version_drift_notice(plugin_root: Option<PathBuf>) -> Option<String> {
    let root = plugin_root?;
    let pin = std::fs::read_to_string(root.join("ss-magic.version")).ok()?;
    let pinned = pin.trim();
    let running = env!("CARGO_PKG_VERSION");
    if pinned.is_empty() || pinned == running {
        return None;
    }
    Some(format!(
        "ss-magic: this hook is running v{running}, but the plugin loaded this session \
         pins v{pinned}. The installed binary updates only on a fresh session start \
         (SessionStart's `startup` source) — resume, clear, compact and fork all keep \
         whichever binary is already on disk. Start a brand-new session once the two agree."
    ))
}

/// `path`, relative to `root`, for display — falling back to the absolute
/// path on the (never-expected-in-practice) chance it does not sit under
/// `root` at all. Both are resolved from the same `cwd` by the same git
/// probe, so the fallback exists only so a display helper degrades rather
/// than panics if that ever stops being true.
fn display_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// Turn one [`scratchpad::Report`] into the `additionalContext` body.
///
/// A hard refusal (`wrote_state == false`) gets a short explanation instead
/// of the usual guidance — nothing was written this run, so nothing here may
/// claim a state file exists.
fn build_guidance(repo_root: &Path, report: &Report) -> String {
    let state_root_rel = display_rel(repo_root, &report.state_root);
    let mut out = String::new();

    if !report.wrote_state {
        let _ = writeln!(
            out,
            "ss-magic: the session scratchpad at {state_root_rel}/ is not set up yet."
        );
        let _ = writeln!(out);
        for refusal in &report.refusals {
            let _ = writeln!(out, "- {refusal}");
        }
        let _ = writeln!(out);
        let _ = write!(
            out,
            "Nothing was written. Once the reason above is fixed, the next session start \
             scaffolds the tree and injects the usual operating guidance."
        );
        return out;
    }

    let session_dir_rel = display_rel(repo_root, &report.session_dir);

    let _ = writeln!(out, "## ss-magic session scratchpad");
    let _ = writeln!(out);
    let _ = writeln!(out, "Session: {}", report.slug);
    let _ = writeln!(
        out,
        "State root: {state_root_rel}/  (gitignored — invisible to `git status`, deleted with the worktree)"
    );
    let _ = writeln!(out, "Session notes: {session_dir_rel}/");
    for (name, desc) in STATE_FILE_NOTES {
        let _ = writeln!(out, "  {name:<22}{desc}");
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "These are yours: read them at the start of a session and keep them current as \
         you work. `ensure` never rewrites a file that already exists, so a `/compact` or \
         a resumed session finds the same files with whatever you last wrote in them."
    );

    if !report.refusals.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Some paths under the tree were left untouched:");
        for refusal in &report.refusals {
            let _ = writeln!(out, "- {refusal}");
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## Operator checklist");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "A separate, project-owned document reviewed on the pull request — not scratchpad \
         state. Its pointer resolves at {state_root_rel}/{CHECKLIST_POINTER_NAME}. Manage it \
         only through these verbs; never Read, Edit or Write the checklist JSON directly:"
    );
    let _ = writeln!(out);
    let _ = write!(out, "{CHECKLIST_VERBS}");

    out
}

#[cfg(test)]
mod tests;
