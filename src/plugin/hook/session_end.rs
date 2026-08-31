//! `SessionEnd` — walk the ending session's transcript tree and leave one
//! ledger row behind.
//!
//! This is the only moment the row can be written. An on-demand subcommand
//! cannot observe a session that has already ended, and the payload the
//! harness hands us carries no usage data at all — six keys, none of them a
//! number of tokens — so the cost has to be read out of the transcript. What
//! makes that safe to do here is that the transcript is already **complete**
//! when this event fires: a snapshot taken inside the hook was byte-identical
//! to the file after the CLI exited, measured twice.
//!
//! ## Why the scan runs inline, and what it is budgeted against
//!
//! The whole thing is sized to fit the harness's DEFAULT per-hook timeout and
//! nothing longer. That default is 1500 ms nominal; a hook body actually gets
//! about 1.15 s of it, because process spawn and start-up have already spent
//! the rest by the time this function is called. Measured end to end against
//! the worst session tree on the author's machine — 1,257 files, 382 MiB —
//! the whole invocation takes ~0.85 s cold and ~35 ms once the offsets store
//! lets it read only the tail. An ordinary session is orders of magnitude
//! smaller, so there is nothing to gain by detaching this into a background
//! process nobody can observe or debug.
//!
//! Raising the configured timeout is NOT the remedy if a scan ever does run
//! long. The CLI genuinely blocks on exit waiting for this event's hooks — a
//! `"timeout": 30` turned a ~3 s run into 8.39 s in testing — so every extra
//! second is paid in user-visible exit latency, by the user, on every session.
//! The remedy is to make the scan cheaper, which is what
//! [`crate::plugin::ledger`]'s offsets store is for.
//!
//! ## Failing open
//!
//! Nothing here can stop a session ending. A missing transcript, an
//! unreadable one, no application data directory to write to — each returns a
//! silent success carrying a note for the heartbeat, and a genuine error
//! propagates to the wrapper, which turns it into exit 0 with empty stdout and
//! an `Error` heartbeat row. The row is the audit channel; the session never
//! hears about any of it.

use std::path::Path;

use anyhow::Result;

use crate::plugin::heartbeat;
use crate::plugin::hook::event::Payload;
use crate::plugin::hook::{HookContext, Outcome};
use crate::plugin::ledger::{self, Ingest, Recorded};

/// The `SessionEnd` handler wired into [`crate::plugin::hook::route`].
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    // No application data directory means nowhere machine-level to write, and
    // the ledger is deliberately not per-worktree — a row has to outlive the
    // worktree it describes. Nothing else to do.
    let Some(store) = heartbeat::store_dir() else {
        return Ok(
            Outcome::silent().with_detail("no application data directory; no ledger row written")
        );
    };
    handle_in_store(ctx, &store)
}

/// [`handle`] against an explicit store, so the whole flow is testable without
/// writing into the developer's own application data directory.
pub(crate) fn handle_in_store(ctx: &HookContext<'_>, store: &Path) -> Result<Outcome> {
    let reason = match &ctx.envelope.payload {
        Payload::SessionEnd(payload) => payload.reason.as_str(),
        // Unreachable through `hook::route`, which builds the payload from the
        // argv token. Falling back to an empty reason beats a match arm that
        // could panic on a future wiring mistake.
        _ => "",
    };

    let session_id = ctx.envelope.common.session_id.trim();
    if session_id.is_empty() {
        return Ok(Outcome::silent().with_detail("envelope carried no session id; nothing keyed"));
    }

    let transcript = Path::new(&ctx.envelope.common.transcript_path);
    if ctx.envelope.common.transcript_path.is_empty() {
        return Ok(Outcome::silent().with_detail(format!(
            "session {session_id} ended ({reason}) with no transcript path"
        )));
    }

    let ingest = Ingest {
        session_id,
        transcript,
        cwd: Some(ctx.cwd()),
        repo_root: ctx.repo_root.as_deref(),
        now: ctx.now,
    };

    let detail = match ledger::record(store, &ingest)? {
        Recorded::Written(row) => format!(
            "recorded {session_id} ({reason}): {} file{}, ${:.2}",
            row.files,
            if row.files == 1 { "" } else { "s" },
            row.cost_usd
        ),
        // The ordinary duplicate: `/clear` fires this event more than once per
        // CLI process, and the harness is free to spawn a hook twice.
        Recorded::Unchanged => {
            format!("{session_id} ({reason}) is already recorded and unchanged")
        }
        Recorded::NoTranscript => format!(
            "no transcript at {} for {session_id} ({reason})",
            transcript.display()
        ),
    };

    Ok(Outcome::silent().with_detail(detail))
}

#[cfg(test)]
mod tests;
