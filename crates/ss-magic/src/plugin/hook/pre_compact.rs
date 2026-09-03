//! `PreCompact` — record that a compaction boundary was crossed, then say
//! nothing at all (R49).
//!
//! This event fires on both the `auto` and `manual` triggers, and even when
//! no compaction actually follows (a `/compact` typed into a session too
//! small to compact still fires it) — the payload gives no way to tell those
//! two cases apart, so the handler below does not try to. It always writes,
//! and it is never allowed to slow the compaction down or veto it: a `manual`
//! `/compact` fires exactly when the user is already at the context wall, and
//! a hook that could refuse it would be a hook that can wedge a session at
//! the worst possible moment (see `hook-contract.md`).
//!
//! ## Why this module never touches the wire
//!
//! `PreCompact` is absent from the harness's `hookSpecificOutput` schema
//! map — emitting one is rejected outright — so [`event::Response`] has no
//! `PreCompact` variant to construct in the first place. The only value this
//! handler can ever return is [`Response::Silent`], which makes "never
//! blocks, never prints" a property of the types rather than a rule to
//! remember. The model-facing half of this feature lives entirely on
//! `SessionStart`'s `compact` source (owned by `session_start.rs`), which is
//! both the designed channel for it and the only reliable signal that a
//! compaction actually happened.
//!
//! ## Where the note goes, and why that respects R17
//!
//! [`scratchpad::STATE_FILES`] (`STATUS.md`, `TASKS.md`, …) are scaffolded
//! once, empty, and never rewritten again — R17's whole point is that their
//! content belongs to the model. Appending ss-magic's own compaction record
//! into one of them would mean putting the tool's words in a file whose
//! contract says the opposite. So this handler writes to a seventh,
//! ss-magic-owned file in the same session directory, [`NOTE_NAME`], that
//! `scratchpad::ensure` never creates and the model is never told to edit.
//! Nothing about it is a rewrite in the R17 sense either: it is opened in
//! append mode and only ever grows, so a byte written on one compaction is
//! never touched by the next.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::plugin::hook::event::Payload;
use crate::plugin::hook::{HookContext, Outcome};
use crate::plugin::scratchpad::{self, Refusal, Report};

/// The compaction log, one per session directory. Deliberately not a name in
/// [`scratchpad::STATE_FILES`] — see the module docs' R17 note — so nothing in
/// `scratchpad::ensure` ever creates, scaffolds or reasons about it.
const NOTE_NAME: &str = "PRE-COMPACT.md";

/// Written once, the first time a session's log is created.
const HEADER: &str = "\
# Pre-compact log

Written by ss-magic's own `PreCompact` hook — one block per compaction
boundary this session crossed, oldest first. This is bookkeeping, not
scratchpad content the model owns: nothing here is ever edited once
appended, and nothing else in `.superset/.magic/` reads it back.
";

/// Owner-only file mode (R58), matching the rest of the state tree. Repeated
/// here rather than imported because this file is created independently of
/// `scratchpad::ensure`, which keeps its own copy private.
const FILE_MODE: u32 = 0o600;

/// The `PreCompact` handler wired into [`crate::plugin::hook::route`].
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    let (trigger, custom_instructions) = match &ctx.envelope.payload {
        Payload::PreCompact(pre_compact) => (
            pre_compact.trigger.as_str(),
            pre_compact.custom_instructions.as_deref(),
        ),
        // Unreachable through `hook::route` — decoding a `PreCompact` event
        // always produces this variant — but falling back to an empty trigger
        // is safer than a `match` arm that could panic on a future wiring
        // mistake.
        _ => ("", None),
    };
    let trigger_display = if trigger.is_empty() {
        "(unset)"
    } else {
        trigger
    };

    // Outside a git repository there is no scratchpad to write to, and
    // `scratchpad::ensure` would only fail on the same question a moment
    // later. Most such invocations already stop at the `disabled` gate in
    // `hook/mod.rs` (a non-repository `cwd` resolves no `plugin.enabled`
    // value to be true), but `HookContext::repo_root`'s own contract is that
    // a handler needing a repository checks this itself.
    let Some(repo_root) = ctx.repo_root.as_deref() else {
        return Ok(Outcome::silent().with_detail("not inside a git repository; nothing to do"));
    };

    let report = scratchpad::ensure(ctx.cwd())?;
    if !report.wrote_state {
        // A hard refusal — no ignore rule yet, an escaping symlink, an
        // unreadable tracked-paths probe, … — means the tree on disk is
        // exactly as it was. R49 does not bend for this: the compaction goes
        // ahead regardless, and the refusal is only ever a heartbeat note.
        return Ok(Outcome::silent().with_detail(format!(
            "scratchpad refused, note not written: {}",
            report.heartbeat_note()
        )));
    }

    // `ensure` itself only guards the paths it writes (the six state files
    // and the pointer); this file is not one of them, so the same R17
    // tracked-path refusal has to be re-checked here before writing to it. A
    // public repository could in principle have this exact path committed at
    // the session directory's predictable location.
    let rel_note = rel_to_repo(repo_root, &report.session_dir.join(NOTE_NAME));
    if is_tracked(&report, &rel_note) {
        return Ok(Outcome::silent().with_detail(format!(
            "{rel_note} is a tracked path; refused to write the compaction note there (R17)"
        )));
    }

    append_note(&report.session_dir, ctx.now, trigger, custom_instructions)
        .with_context(|| format!("recording the pre-compact note for session {}", report.slug))?;

    Ok(Outcome::silent().with_detail(format!(
        "compaction note recorded (trigger: {trigger_display})"
    )))
}

/// `path`, relative to `repo_root`, in the same form `git::tracked_files`
/// (and so [`Refusal::TrackedPaths`]) reports paths in.
fn rel_to_repo(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// Whether `scratchpad::ensure` already reported `rel` as a tracked path
/// somewhere under the state tree.
fn is_tracked(report: &Report, rel: &str) -> bool {
    report.refusals.iter().any(|refusal| {
        matches!(refusal, Refusal::TrackedPaths { paths } if paths.iter().any(|p| p == rel))
    })
}

/// Append one entry to the compaction log, writing [`HEADER`] first if this
/// is the session's first compaction. Opened in append mode throughout, so an
/// earlier entry — and, on a repeated compaction, every entry before it — is
/// never touched, only added to.
fn append_note(
    session_dir: &Path,
    now: u64,
    trigger: &str,
    custom_instructions: Option<&str>,
) -> Result<()> {
    let path = session_dir.join(NOTE_NAME);

    // Newness is decided by the open itself, not by a `path.exists()` check
    // made beforehand: the harness can spawn duplicate hooks for the same
    // event (see `claim.rs`'s module doc), and two such invocations racing
    // here could both see "not there yet" before either had opened the file,
    // then both go on to write HEADER — a duplicated header mid-file.
    // `create_new` opens with `O_EXCL`, which the kernel guarantees at most
    // one of two racing opens can win; the loser's `AlreadyExists` is exactly
    // the fact that the file is no longer new, so it falls back to an
    // ordinary append-mode open and skips the header.
    let (mut file, is_new) = match fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&path)
    {
        Ok(file) => (file, true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let file = fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .with_context(|| format!("opening {}", path.display()))?;
            (file, false)
        }
        Err(e) => return Err(e).with_context(|| format!("opening {}", path.display())),
    };

    if is_new {
        file.write_all(HEADER.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
    }

    let trigger_display = if trigger.is_empty() {
        "(unset)"
    } else {
        trigger
    };
    write!(
        file,
        "\n## {} — trigger: {trigger_display}\n",
        scratchpad::format_rfc3339(now)
    )
    .with_context(|| format!("writing {}", path.display()))?;

    if let Some(instructions) = custom_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        writeln!(file, "\n{instructions}")
            .with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
