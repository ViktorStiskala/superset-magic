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

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::plugin::atomic;
use crate::plugin::compact_window::{self, OVERRIDE_ENV};
use crate::plugin::hook::event::{Payload, Response};
use crate::plugin::hook::{self, HookContext, Outcome};
use crate::plugin::scratchpad::{self, Refusal, Report};

/// The checklist verb family (R89, R90), spelled out here only so the injected
/// guidance can name them. The schema and the verbs themselves belong to
/// `checklist`; this module never reads, writes, or even checks for
/// `checklist.json`'s existence — it states where the pointer will resolve and
/// how to reach it, nothing more.
///
/// These are spelled `ss-magic-plugin`, the wrapper on the Bash tool's PATH —
/// **not** a bare `ss-magic`. The model runs these through the Bash tool, where
/// `${CLAUDE_PLUGIN_DATA}` is not exported and so the bootstrapped binary
/// cannot be named directly; the wrapper resolves it and injects the `plugin`
/// verb. A bare `ss-magic` would also resolve against whatever the user happens
/// to have on PATH, which is exactly why R75 gives the wrapper a distinct name.
const CHECKLIST_VERBS: &str = "\
    ss-magic-plugin checklist init <slug>
    ss-magic-plugin checklist add-item <section> <id>
    ss-magic-plugin checklist add-entry <id>
    ss-magic-plugin checklist set <id> <dotted-key> <value>
    ss-magic-plugin checklist done <id>
    ss-magic-plugin checklist list
    ss-magic-plugin checklist verify
    ss-magic-plugin checklist render-md";

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

/// The name of the once-per-machine marker for the compaction notice (R27),
/// in the `ss-magic` cache directory beside the version caches.
const COMPACT_ADVICE_MARKER: &str = "compact-advice-shown";

/// What this handler reads from outside the envelope: the process
/// environment and the machine-level cache directory.
///
/// Gathered into one value and passed in, rather than read where needed, so
/// the handler is testable without a test writing a once-per-machine marker
/// into the developer's own cache directory — which is exactly what running
/// the handler against the real environment would do on a machine where the
/// override is set, and this repository's own author's is one.
pub(crate) struct Surroundings {
    /// `${CLAUDE_PLUGIN_ROOT}`, for the version-drift notice.
    pub plugin_root: Option<PathBuf>,
    /// Whether `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` is set (and non-empty) in the
    /// hook's environment — the environment the harness itself runs with,
    /// every settings file's `env` block included.
    pub override_present: bool,
    /// `CLAUDE_CODE_ENTRYPOINT`, for the quiet-mode decision.
    pub entrypoint: Option<OsString>,
    /// Where the once-per-machine marker lives. A closure so the directory is
    /// resolved (and, on first use, created) only once the cheap checks have
    /// decided a notice is actually due — never on a plain `resume`.
    pub marker_dir: Box<dyn Fn() -> Option<PathBuf>>,
}

impl Surroundings {
    /// The real process's view.
    fn from_process() -> Self {
        Self {
            plugin_root: std::env::var_os("CLAUDE_PLUGIN_ROOT").map(PathBuf::from),
            override_present: std::env::var_os(OVERRIDE_ENV).is_some_and(|v| !v.is_empty()),
            entrypoint: std::env::var_os(hook::ENTRYPOINT_ENV),
            marker_dir: Box::new(crate::update::check::cache_dir),
        }
    }
}

/// The `SessionStart` handler wired into [`crate::plugin::hook::route`].
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    handle_with(ctx, &Surroundings::from_process())
}

/// [`handle`] with the environment injected.
pub(crate) fn handle_with(ctx: &HookContext<'_>, surroundings: &Surroundings) -> Result<Outcome> {
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

    // Both operator notices ride `systemMessage` and never the model-facing
    // channel: neither is something the model can act on.
    let drift = version_drift_notice(surroundings.plugin_root.clone());
    let advice = compaction_advice(
        repo_root,
        source,
        hook::quiet_mode(ctx.envelope, surroundings.entrypoint.as_deref()),
        surroundings.override_present,
        &*surroundings.marker_dir,
        ctx.now,
    );
    let system_message = join_system_messages([drift, advice.message]);

    let mut detail = if source.is_empty() {
        report.heartbeat_note()
    } else {
        format!("{} (source: {source})", report.heartbeat_note())
    };
    if let Some(note) = advice.detail {
        detail.push_str("; ");
        detail.push_str(&note);
    }

    Ok(Outcome::new(Response::SessionStart {
        additional_context: Some(additional_context),
        system_message,
    })
    .with_detail(detail))
}

/// What [`compaction_advice`] decided.
struct CompactionAdvice {
    /// The notice, when one is due.
    message: Option<String>,
    /// A short note for the heartbeat row saying what happened — `None` on
    /// the ordinary paths where nothing was even considered.
    detail: Option<String>,
}

/// The notice itself. Operator-facing, so it names the verb a person types in
/// a terminal; it never asks the model to do anything.
const COMPACT_ADVICE_TEXT: &str = "\
ss-magic: CLAUDE_AUTOCOMPACT_PCT_OVERRIDE is set in this session's environment and this \
repository configures no autoCompactWindow. The percentage override can only lower the \
auto-compact window and means a different cap on every model; an absolute window fits \
better. Run `ss-magic plugin compact-window --recommend` for a value sized from this \
repository's own recorded sessions — nothing is written until you run `--set`, and \
ss-magic never edits the file the override lives in. Shown once per machine.";

/// R27's one-time nudge: on a fresh `startup`, when the override is in the
/// hook's environment and the repository configures no `autoCompactWindow`,
/// say so once per machine on the operator channel.
///
/// The checks run cheapest first and the marker directory is resolved last,
/// so a plain `resume` — or any startup on a machine without the override —
/// never touches the cache directory at all. A silent path never writes the
/// marker: the once-per-machine budget is spent only by a notice that
/// actually went out. Without a directory to record it in the notice is
/// withheld rather than repeated every session, since the bound is the
/// promise. Advice only, every branch: nothing here writes a setting (R28).
fn compaction_advice(
    repo_root: &Path,
    source: &str,
    quiet: Option<&str>,
    override_present: bool,
    marker_dir: &dyn Fn() -> Option<PathBuf>,
    now: u64,
) -> CompactionAdvice {
    let silent = |detail: Option<String>| CompactionAdvice {
        message: None,
        detail,
    };
    // The ordinary paths: nothing was even considered, so nothing to record.
    if source != "startup" || !override_present {
        return silent(None);
    }
    if let Some(reason) = quiet {
        return silent(Some(format!("compaction notice withheld (quiet mode: {reason})")));
    }
    if compact_window::window_configured(repo_root) {
        return silent(Some(
            "compaction notice not needed (a window is configured)".to_string(),
        ));
    }
    let Some(dir) = marker_dir() else {
        return silent(Some(
            "compaction notice withheld (no cache directory to record it in)".to_string(),
        ));
    };
    let marker = dir.join(COMPACT_ADVICE_MARKER);
    if marker.exists() {
        return silent(Some(
            "compaction notice already shown on this machine".to_string(),
        ));
    }
    // The marker's contents are for a person looking at the file: when it
    // was shown. Its existence is what the check above reads.
    let detail = match atomic::write_atomically(
        &marker,
        &format!("{}\n", scratchpad::format_rfc3339(now)),
        ".compact-advice-",
        ".tmp",
        Some(COMPACT_ADVICE_MARKER),
        Some(0o600),
        false,
    ) {
        Ok(()) => "compaction notice shown".to_string(),
        // Best-effort: the notice still goes out; the worst case is a repeat
        // on a machine whose cache directory refuses writes.
        Err(e) => format!("compaction notice shown (marker could not be written: {e:#})"),
    };
    CompactionAdvice {
        message: Some(COMPACT_ADVICE_TEXT.to_string()),
        detail: Some(detail),
    }
}

/// Join whichever operator notices are present into one `systemMessage`,
/// separated by a blank line; `None` when there is nothing to say.
fn join_system_messages<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let present: Vec<String> = parts.into_iter().flatten().collect();
    if present.is_empty() {
        None
    } else {
        Some(present.join("\n\n"))
    }
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

/// How many entries [`render_refusal`] lists inline for a
/// [`Refusal::TrackedPaths`] before collapsing the rest into a "… and N more"
/// marker.
///
/// `Refusal`'s own `Display` joins the whole path list with no cap, which is
/// fine for a heartbeat row on disk but not for `additionalContext`: a
/// repository that gitignores the state tree (so the R63 ignore check
/// passes) and then `git add -f`s a few hundred files under it would push
/// this one line past R19's 10,000-character budget on every session start,
/// crowding out the `## Operator checklist` section that follows it in the
/// guidance text.
const MAX_LISTED_TRACKED_PATHS: usize = 20;

/// Render one refusal for the guidance text.
///
/// Every variant except [`Refusal::TrackedPaths`] already renders a small,
/// fixed amount of text via its `Display` impl, so those pass straight
/// through unchanged. `TrackedPaths` carries a list whose length is decided
/// by the repository, not by ss-magic, so it is capped here the same way the
/// cache module bounds an oversized cached body (see `cache::bound`): show
/// the first [`MAX_LISTED_TRACKED_PATHS`] and say how many more there were,
/// rather than either dropping them silently or letting the line grow
/// without limit.
fn render_refusal(refusal: &Refusal) -> String {
    match refusal {
        Refusal::TrackedPaths { paths } if paths.len() > MAX_LISTED_TRACKED_PATHS => {
            let shown = paths[..MAX_LISTED_TRACKED_PATHS].join(", ");
            let more = paths.len() - MAX_LISTED_TRACKED_PATHS;
            format!("refused to adopt tracked path(s): {shown}, … and {more} more")
        }
        other => other.to_string(),
    }
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
/// A run that refused before creating anything gets a short explanation
/// instead of the usual guidance: there is no tree to describe, so nothing
/// here may claim a state file exists.
///
/// That case is decided by `created` being empty as well as `wrote_state`
/// being false, not by `wrote_state` alone. `wrote_state` answers "is this
/// tree safe to write into", and the two late refusal sites (the session
/// pointer, and `scaffold`) can fire *after* directories have already been
/// created — so "unsafe" and "wrote something" is a real combination. Such a
/// run gets the ordinary guidance, the `## Operator checklist` section
/// included, with a refusal block that says the scaffold did not finish;
/// telling it "nothing was written" would be false, and dropping the checklist
/// verbs with that sentence would withhold them from a run that mostly
/// succeeded.
fn build_guidance(repo_root: &Path, report: &Report) -> String {
    let state_root_rel = display_rel(repo_root, &report.state_root);
    let mut out = String::new();

    if !report.wrote_state && report.created.is_empty() {
        let _ = writeln!(
            out,
            "ss-magic: the session scratchpad at {state_root_rel}/ is not set up yet."
        );
        let _ = writeln!(out);
        for refusal in &report.refusals {
            let _ = writeln!(out, "- {}", render_refusal(refusal));
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
        // Two different situations share this block. `wrote_state` still true
        // means the run completed and merely skipped a path or two (a tracked
        // file it must not overwrite). False here means the scaffold stopped
        // part-way: some of the files named above were created and some were
        // not, and the tree is not safe to write into until the reason is
        // fixed. Both need the reasons; only the second needs the warning.
        let _ = writeln!(
            out,
            "{}",
            if report.wrote_state {
                "Some paths under the tree were left untouched:"
            } else {
                "The scaffold did not finish, so some of the files named above may not \
                 exist — do not write into the tree until this is fixed:"
            }
        );
        for refusal in &report.refusals {
            let _ = writeln!(out, "- {}", render_refusal(refusal));
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
