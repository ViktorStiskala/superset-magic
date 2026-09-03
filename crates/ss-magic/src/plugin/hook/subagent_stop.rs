//! `SubagentStop` — hold a dispatched agent to its contracted output file, and
//! rescue a transcript that ended with nothing to show for it.
//!
//! This is the one event that addresses genuine **data loss** rather than
//! context economy. A subagent's report is tail-truncated by the harness when
//! it runs long, with the explicit marker *"the earlier part of the report is
//! not retrievable"* — the text is not spilled to a file anywhere, it is gone.
//! Two things follow, and this handler does both.
//!
//! ## Enforcement (R32, R51)
//!
//! If the parent declared an output file before dispatching
//! (`ss-magic plugin expect-artifact <path>`, see
//! [`crate::plugin::expect_artifact`]) and the agent stops without writing it,
//! the stop is blocked once, naming the file, so the agent still has a turn in
//! which to write it. **With nothing declared, nothing is ever blocked** — an
//! agent nobody made a declaration for stops exactly as it always did.
//!
//! "At most once" is guaranteed twice over, deliberately:
//!
//! - `stop_hook_active` tells us the harness is re-entering the stop because a
//!   hook already blocked it. The handler returns immediately on that, before
//!   touching the filesystem at all, so a block can never block its own block.
//! - Taking the declaration IS the one-shot flag. It is claimed — by the
//!   rename in [`crate::plugin::claim`], which is exactly-once even when
//!   duplicate hook invocations race — and so removed from the directory
//!   *before* the block decision is made. A second stop finds nothing pending
//!   and ends normally, which is the same code path as "nothing was ever
//!   declared".
//!
//! ## Salvage (R33, R54)
//!
//! When the payload reports no final message, the agent ended without saying
//! anything the parent can act on, and re-running it costs the whole dispatch
//! again. The payload does carry `agent_transcript_path`, so what the agent
//! *did* say is still on disk — this handler recovers the assistant text from
//! it into a file under the session's `research-salvage/` directory, clearly
//! marked incomplete, and the parent reads that instead.
//!
//! The salvaged text is quoted, never presented as the agent's own report: it
//! goes through [`cache::envelope`], the same untrusted-data framing the
//! conclusion cache uses (R54, R64). Both are ss-magic-generated text derived
//! from a file that some later session reads back with nothing else around to
//! explain where it came from, so they get the same treatment rather than two
//! near-identical markers that could drift apart.

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::plugin::cache::{self, Budget};
use crate::plugin::expect_artifact::{self, Expectation, Unmet};
use crate::plugin::hook::event::{Payload, Response, SubagentStop};
use crate::plugin::hook::{HookContext, Outcome};
use crate::plugin::scratchpad::{self, Refusal, Report};

/// The session subdirectory salvaged transcripts land in. Named to fit the
/// scratchpad contract's `research-<topic>/` slot, so it sits beside the
/// model's own research notes rather than inventing a new kind of directory.
const SALVAGE_DIR: &str = "research-salvage";

/// How much recovered text a salvage file may hold. Generous, because this is
/// the recovery path for output that is otherwise lost outright — but bounded,
/// because the state tree is exempt from the Read gate, so whatever lands here
/// reaches a context window unfiltered when the parent reads it.
const SALVAGE_BYTE_BUDGET: usize = 64 * 1024;

/// How much of a transcript is read at all. A session transcript has been
/// measured at 34 MB; there is no point pulling all of that into memory to
/// keep 64 KB of it.
const MAX_TRANSCRIPT_BYTES: u64 = 16 * 1024 * 1024;

/// How many same-second salvage attempts are given distinct names before the
/// handler gives up. Two salvages for one agent inside one second is already
/// implausible; twenty is a bound, not an expectation.
const MAX_NAME_ATTEMPTS: u32 = 20;

/// Owner-only modes (R58), matching the rest of the state tree. Repeated here
/// rather than imported because this file is created independently of
/// `scratchpad::ensure`, which keeps its own copies private.
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// The `SubagentStop` handler wired into [`crate::plugin::hook::route`].
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    let Payload::SubagentStop(payload) = &ctx.envelope.payload else {
        // Unreachable through `hook::route`, which builds the variant from the
        // argv token. Saying nothing beats a `match` arm that could panic on a
        // future wiring mistake.
        return Ok(Outcome::silent().with_detail("payload is not a SubagentStop envelope"));
    };

    // R32 — the harness is re-entering a stop a hook already blocked. Return
    // before anything else runs: this handler's whole job on a re-entry is to
    // get out of the way and let the agent end.
    if payload.stop_hook_active {
        return Ok(Outcome::silent()
            .with_detail("stop hook already active; returned without blocking again"));
    }

    // Outside a git repository there is no state tree to read declarations
    // from or write salvage into. Most such invocations already stop at the
    // `disabled` gate in `hook/mod.rs`, but `HookContext::repo_root`'s own
    // contract is that a handler needing a repository checks this itself.
    if ctx.repo_root.is_none() {
        return Ok(Outcome::silent().with_detail("not inside a git repository; nothing to do"));
    }

    let report = scratchpad::ensure(ctx.cwd())?;
    if !report.wrote_state {
        // A hard refusal — no ignore rule yet, an escaping symlink, an
        // unreadable tracked-paths probe. Nothing can be read or written, and
        // a stop is never blocked on a state problem of ours.
        return Ok(Outcome::silent().with_detail(format!(
            "scratchpad refused, nothing enforced or salvaged: {}",
            report.heartbeat_note()
        )));
    }

    // R33 first, so a stop that is about to be blocked still leaves the
    // recovered text behind — the two are independent, and the salvage is the
    // half that cannot be redone later.
    let salvage = salvage(ctx, payload, &report);

    // R51/R32. Taking the declaration is what makes the block one-shot; see
    // the module docs.
    let dir = expect_artifact::dir_in(&report.state_root);
    let Some(expectation) = expect_artifact::take_oldest(&dir, ctx.now) else {
        return Ok(Outcome::silent().with_detail(match salvage_note(&salvage) {
            Some(note) => format!("no declaration pending; {note}"),
            None => "no declaration pending; stop not blocked".to_string(),
        }));
    };

    let Some(unmet) = expect_artifact::check(Path::new(&expectation.path)) else {
        return Ok(Outcome::silent().with_detail(format!(
            "declaration for {} satisfied and retired{}",
            expectation.relative,
            suffix(&salvage)
        )));
    };

    Ok(Outcome::new(Response::SubagentStopBlock {
        reason: block_reason(&expectation, unmet, salvage_path_of(&salvage)),
    })
    .with_detail(format!(
        "blocked once: {} is {}{}",
        expectation.relative,
        unmet.code(),
        suffix(&salvage)
    )))
}

// ── The block (R32) ───────────────────────────────────────────────────────────

/// What the blocked agent is told. It names the file, says what was actually
/// found there, and states plainly that this is the only block it will get —
/// an agent that thinks it can be nudged repeatedly has no reason to write the
/// file now.
fn block_reason(expectation: &Expectation, unmet: Unmet, salvage: Option<&Path>) -> String {
    let mut reason = format!(
        "ss-magic: you were dispatched to produce a file, and {}:\n\n    {}\n\n",
        unmet.describe(),
        expectation.relative
    );

    if let Some(note) = expectation.note.as_deref().filter(|n| !n.is_empty()) {
        reason.push_str(&format!("What it is for: {note}\n\n"));
    }

    reason.push_str(
        "Write your full result to that file before you finish. It has to exist and hold \
         bytes — an empty file counts as not written.\n\n\
         This matters because your final report is truncated when it runs long, and the \
         part that is cut is not recoverable from anywhere. A file on disk is.\n\n\
         Your stop is blocked exactly once for this file. If you stop again without \
         writing it, you will end and the work will be lost.\n",
    );

    if let Some(path) = salvage {
        reason.push_str(&format!(
            "\nYou also ended without a final message. What you did say has been recovered \
             to {}, marked incomplete.\n",
            path.display()
        ));
    }

    reason
}

// ── Salvage (R33, R54) ────────────────────────────────────────────────────────

/// What a salvage attempt did, phrased for a heartbeat row.
struct Salvaged {
    /// The file written, when one was.
    path: Option<PathBuf>,
    /// The note for the heartbeat row.
    note: String,
}

/// The salvage file, if one was actually written.
fn salvage_path_of(salvage: &Option<Salvaged>) -> Option<&Path> {
    salvage.as_ref().and_then(|s| s.path.as_deref())
}

/// What the salvage attempt has to say for the heartbeat row, if it ran.
fn salvage_note(salvage: &Option<Salvaged>) -> Option<&str> {
    salvage.as_ref().map(|s| s.note.as_str())
}

/// The same note, ready to append to a detail that already says something.
fn suffix(salvage: &Option<Salvaged>) -> String {
    salvage_note(salvage).map_or(String::new(), |note| format!("; {note}"))
}

/// Recover a resultless agent's transcript into the session's
/// `research-salvage/` directory (R33).
///
/// `None` means there was nothing to do — the agent reported a result, so
/// nothing was lost. Every other outcome, including a failure, comes back as a
/// [`Salvaged`] carrying the note: a stop is never failed or blocked because
/// salvage did not work out.
fn salvage(ctx: &HookContext<'_>, payload: &SubagentStop, report: &Report) -> Option<Salvaged> {
    // "No reported result" covers both an absent message and a blank one — an
    // agent that ends with whitespace has told the parent exactly as much as
    // one that ends with nothing.
    if payload
        .last_assistant_message
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty())
    {
        return None;
    }

    let Some(transcript) = payload
        .agent_transcript_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    else {
        return Some(Salvaged {
            path: None,
            note: "no result reported and no transcript path to salvage from".to_string(),
        });
    };
    let transcript = Path::new(transcript);

    let raw = match read_bounded(transcript) {
        Ok(raw) => raw,
        Err(e) => {
            // A transcript that is not there or cannot be read is the one case
            // where the loss is simply unrecoverable. Recorded, never fatal.
            return Some(Salvaged {
                path: None,
                note: format!(
                    "no result reported; {} unreadable: {e}",
                    transcript.display()
                ),
            });
        }
    };

    let (body, dropped) = recovered_body(&raw, transcript);
    let head = salvage_header(payload, transcript, ctx.now, dropped);
    let rendered = cache::envelope(&head, &body, transcript, Budget::Unbounded);

    match write_salvage(report, payload, ctx.now, &rendered) {
        Ok(path) => {
            let note = format!("salvaged the transcript to {}", path.display());
            Some(Salvaged {
                path: Some(path),
                note,
            })
        }
        Err(e) => Some(Salvaged {
            path: None,
            note: format!("no result reported; could not write the salvage file: {e:#}"),
        }),
    }
}

/// Read at most [`MAX_TRANSCRIPT_BYTES`] of `path`, lossily as UTF-8.
///
/// Lossy on purpose: a transcript with one bad byte in it is still worth
/// recovering, and failing the whole salvage over an encoding problem would
/// throw away the very thing this exists to keep.
fn read_bounded(path: &Path) -> Result<String> {
    let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut bytes = Vec::new();
    std::io::Read::take(file, MAX_TRANSCRIPT_BYTES)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The text to salvage out of a transcript, plus how many recovered blocks had
/// to be dropped to fit [`SALVAGE_BYTE_BUDGET`].
///
/// A subagent transcript is JSONL, one record per line, and what matters in it
/// is what the assistant actually said. Those blocks are pulled out and joined;
/// a transcript this cannot make sense of falls back to its raw text, because
/// unparsed JSON that a person can still read beats nothing at all.
///
/// When the recovered text does not fit, the blocks kept are the LAST ones. An
/// agent that stopped without a result was closest to having one at the end, so
/// truncating from the front keeps the useful half — the opposite of what a
/// plain byte-prefix cut would do.
fn recovered_body(raw: &str, transcript: &Path) -> (String, usize) {
    let blocks = assistant_blocks(raw);
    if blocks.is_empty() {
        let (kept, dropped) = keep_tail(
            raw.lines().map(str::to_string).collect(),
            SALVAGE_BYTE_BUDGET,
        );
        let mut body = String::from(
            "(no assistant message could be parsed out of the transcript; its raw text \
             follows)\n\n",
        );
        body.push_str(&kept.join("\n"));
        return (body, dropped);
    }

    let (kept, dropped) = keep_tail(blocks, SALVAGE_BYTE_BUDGET);
    let mut body = String::new();
    if dropped > 0 {
        body.push_str(&format!(
            "[ss-magic: {dropped} earlier message(s) dropped to fit the salvage budget; the \
             whole transcript is at {}]\n\n",
            transcript.display()
        ));
    }
    body.push_str(&kept.join("\n\n"));
    (body, dropped)
}

/// Every assistant text block in a JSONL transcript, in order.
///
/// Deliberately forgiving about shape: a line that is not JSON, a record with
/// no `message`, or a content entry of a type this does not recognize is
/// skipped rather than failing the salvage. The transcript format belongs to
/// the harness and may grow fields at any time.
fn assistant_blocks(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = record.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        match content {
            Value::String(text) => push_text(&mut out, text),
            Value::Array(parts) => {
                for part in parts {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            push_text(&mut out, text);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Add `text` unless it is blank — a transcript is full of empty text parts
/// that sit alongside tool calls, and they only dilute the salvage.
fn push_text(out: &mut Vec<String>, text: &str) {
    if !text.trim().is_empty() {
        out.push(text.trim_end().to_string());
    }
}

/// The last `blocks` that fit in `budget` bytes, and how many were dropped off
/// the front to make them fit. At least one block is always kept, even an
/// oversized one, so a single enormous message does not salvage to nothing.
fn keep_tail(blocks: Vec<String>, budget: usize) -> (Vec<String>, usize) {
    let mut total = 0usize;
    let mut keep = 0usize;
    for block in blocks.iter().rev() {
        // +2 for the separator this block will be joined with.
        let cost = block.len() + 2;
        if keep > 0 && total + cost > budget {
            break;
        }
        total += cost;
        keep += 1;
    }
    let dropped = blocks.len() - keep;
    (blocks.into_iter().skip(dropped).collect(), dropped)
}

/// The provenance header (R54). Says who the text came from, that ss-magic
/// recovered it rather than the agent reporting it, and that it is incomplete
/// — because this file is read back in a later turn, or a later session, with
/// nothing else around to explain any of that.
fn salvage_header(payload: &SubagentStop, transcript: &Path, now: u64, dropped: usize) -> String {
    let unknown = "(unreported)";
    let mut head = format!(
        "# ss-magic salvaged subagent transcript — INCOMPLETE\n\
         \n\
         - status: INCOMPLETE — the agent stopped without reporting a result\n\
         - agent-id: {agent}\n\
         - agent-type: {kind}\n\
         - transcript: {transcript}\n\
         - salvaged: {when}\n\
         - salvaged-epoch: {now}\n",
        agent = payload.agent_id.as_deref().unwrap_or(unknown),
        kind = payload.agent_type.as_deref().unwrap_or(unknown),
        transcript = transcript.display(),
        when = scratchpad::format_rfc3339(now),
    );
    if dropped > 0 {
        head.push_str(&format!("- dropped-messages: {dropped}\n"));
    }
    head.push_str(
        "\n\
         Generated by ss-magic: text recovered from the transcript named above after the\n\
         agent ended without reporting a result. This is NOT the agent's own report and\n\
         NOT the transcript file's content — it is a partial reconstruction that may stop\n\
         mid-thought, and the agent may never have reached a conclusion at all. Treat any\n\
         finding in it as unconfirmed.\n",
    );
    head
}

/// Write `rendered` into the session's salvage directory, under a name that
/// does not collide with an existing salvage.
fn write_salvage(
    report: &Report,
    payload: &SubagentStop,
    now: u64,
    rendered: &str,
) -> Result<PathBuf> {
    let dir = report.session_dir.join(SALVAGE_DIR);

    // `scratchpad::ensure` only guards the paths it writes itself, so R17's
    // refusal to touch a tracked path has to be re-checked for this one: a
    // public repository could have committed something at the session
    // directory's predictable location.
    let rel_dir = rel_to_state(report, &dir);
    if tracked_under(report, &rel_dir) {
        anyhow::bail!("{rel_dir} is tracked by git; refused to write a salvage there (R17)");
    }

    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let stem = format!("{}-{}", compact_stamp(now), agent_slug(payload));
    for attempt in 0..MAX_NAME_ATTEMPTS {
        let name = if attempt == 0 {
            format!("{stem}.md")
        } else {
            format!("{stem}-{}.md", attempt + 1)
        };
        let path = dir.join(&name);

        // `create_new` rather than a plain create: a salvage never overwrites
        // an earlier one, because the earlier one is also text that exists
        // nowhere else.
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(rendered.as_bytes())
                    .with_context(|| format!("writing {}", path.display()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(anyhow::Error::new(e).context(format!("creating {}", path.display())));
            }
        }
    }

    anyhow::bail!(
        "could not find an unused salvage name in {} after {MAX_NAME_ATTEMPTS} attempts",
        dir.display()
    )
}

/// `YYYYmmdd-HHMMSS`, from the same RFC 3339 formatter everything else uses so
/// there is no second copy of the date arithmetic.
fn compact_stamp(now: u64) -> String {
    scratchpad::format_rfc3339(now)
        .chars()
        .filter_map(|c| match c {
            '-' | ':' | 'Z' => None,
            'T' => Some('-'),
            other => Some(other),
        })
        .collect()
}

/// The agent id, reduced to something safe in a file name. Anything that is
/// not alphanumeric, `-` or `_` becomes `-`, and the result is bounded so an
/// unexpectedly long id cannot produce a name the filesystem rejects.
fn agent_slug(payload: &SubagentStop) -> String {
    let raw = payload
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or("unknown-agent");

    let slug: String = raw
        .chars()
        .take(40)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();

    if slug.trim_matches('-').is_empty() {
        "unknown-agent".to_string()
    } else {
        slug
    }
}

/// `path` relative to the worktree root, in the form `git::tracked_files` (and
/// so [`Refusal::TrackedPaths`]) reports paths in. Derived from the state root
/// the report already carries, so the handler does not need the repository
/// root separately.
fn rel_to_state(report: &Report, path: &Path) -> String {
    // The state root is `<root>/.superset/.magic`, so its grandparent is the
    // worktree root.
    let root = report
        .state_root
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&report.state_root);
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// Whether `scratchpad::ensure` reported anything tracked at `rel` or beneath
/// it. A directory never appears in git's own tracked listing, so a committed
/// file inside one is what has to be looked for.
fn tracked_under(report: &Report, rel: &str) -> bool {
    let prefix = format!("{rel}/");
    report.refusals.iter().any(|refusal| {
        matches!(
            refusal,
            Refusal::TrackedPaths { paths }
                if paths.iter().any(|p| p == rel || p.starts_with(&prefix))
        )
    })
}

#[cfg(test)]
mod tests;
