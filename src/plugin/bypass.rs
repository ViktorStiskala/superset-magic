//! One-shot Read-gate bypass claims: `.superset/.magic/bypass/<hash>.json`.
//!
//! Every denial the Read gate emits ends by naming an escape hatch, verbatim:
//!
//! ```text
//! ss-magic plugin bypass <path>
//! ```
//!
//! Running it drops a claim file here. The next gated `Read` of that file
//! consumes the claim and goes through untouched; the read after that is gated
//! again. That is the whole feature — a way for a person (or the model, on the
//! model's own judgement) to say "no, I really do want these bytes in this
//! window" without editing configuration, and without leaving the gate off.
//!
//! ## Why a file, and why RENAMING it is the claim
//!
//! The claim has to survive between two separate `ss-magic` processes — the one
//! a person runs in a terminal and the hook the harness spawns for the next
//! `Read` — so it cannot live in memory. And it has to be consumed *exactly*
//! once even when several gated reads race for it, which is what dictates the
//! syscall [`consume`] is built on.
//!
//! The obvious choice, "delete the file and treat a successful delete as the
//! claim", does not work — see [`crate::plugin::claim`], which owns the
//! rename-based mechanism that replaced it and the measurement that ruled the
//! `unlink` version out. Everything this module does with a claim file goes
//! through [`claim::take`]; the subagent-output declarations of
//! [`crate::plugin::expect_artifact`] use the same helper, so the two one-shot
//! stores cannot drift apart on the one property that matters.
//!
//! ## Why claims expire
//!
//! A claim recorded and then forgotten would sit in the tree indefinitely and
//! quietly wave through one oversized read weeks later, at which point nobody
//! remembers asking for it. [`MAX_AGE_SECS`] bounds that: an expired claim is
//! still consumed (so it stops being in the way) but does not open the gate.

use std::fs;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::hashing;
use crate::plugin::atomic;
use crate::plugin::claim;
use crate::plugin::scratchpad::{self, now_secs, STATE_REL};
use crate::tui::style;

/// The claim directory's name under the state root. `scratchpad::ensure`
/// already creates it, so neither the verb nor the gate has to bootstrap a
/// directory mid-flight.
pub const DIR_NAME: &str = "bypass";

/// Claim file extension.
const CLAIM_EXT: &str = "json";

/// How long a claim stays usable. A day: long enough that recording one and
/// getting back to the read across a break still works, short enough that a
/// forgotten claim cannot surprise a later session.
pub const MAX_AGE_SECS: u64 = 24 * 60 * 60;

/// Owner-only modes, matching the rest of the state tree (R58).
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// Usage for the `bypass` verb.
const BYPASS_USAGE: &str = "\
Usage: ss-magic plugin bypass <FILE>

Let the next gated Read of <FILE> through, once. The read after that is gated
again, and nothing else about the gate changes.

Recorded under .superset/.magic/bypass/ and consumed by the next Read of that
file; claims older than 24 hours are discarded unused.";

/// What a claim file holds. The hashed file name is not reversible, so the
/// path is stored inside — otherwise a person looking at the directory could
/// not tell what any of it was for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Claim {
    /// The resolved file this claim is for.
    path: String,
    /// When it was recorded, seconds since the epoch — what [`consume`] ages
    /// against, so expiry never depends on the claim file's own mtime.
    recorded_epoch: u64,
}

/// The claim directory for a worktree root.
pub fn dir_for_root(root: &Path) -> PathBuf {
    root.join(STATE_REL).join(DIR_NAME)
}

/// Where the claim for `realpath` lives.
///
/// The name is a hash rather than the path itself: a repository path can be
/// any length and contain anything a filesystem allows, including separators,
/// so embedding it would produce names that are too long, nested, or simply
/// unrepresentable. [`crate::hashing::fnv1a_64`] is the same non-cryptographic
/// choice the conclusion cache makes, for the same reason — the risk here is an
/// accidental collision, not a constructed one, and a hash that changed between
/// builds would silently orphan every claim.
pub fn claim_path(dir: &Path, realpath: &Path) -> PathBuf {
    let hash = hashing::fnv1a_64(realpath.as_os_str().as_bytes());
    dir.join(format!("{hash:016x}.{CLAIM_EXT}"))
}

/// Record a claim for `realpath`, replacing any claim already there.
///
/// Written through a temp file and a rename so the gate never reads a
/// half-written claim, and so recording a second claim for the same file
/// leaves one whole claim rather than an interleaved one.
pub fn record(dir: &Path, realpath: &Path, now: u64) -> Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let claim = Claim {
        path: realpath.to_string_lossy().into_owned(),
        recorded_epoch: now,
    };
    let body = format!("{}\n", serde_json::to_string_pretty(&claim)?);

    let path = claim_path(dir, realpath);
    atomic::write_atomically(
        &path,
        &body,
        ".bypass-",
        ".tmp",
        Some("the bypass claim"),
        Some(FILE_MODE),
        true,
    )?;
    Ok(path)
}

/// Claim the bypass for `realpath`, if there is one. `true` means this caller
/// won it and the read goes through.
///
/// Winning the rename *is* the claim (see the module docs), so the expiry check
/// happens after it: an expired claim is still taken out of circulation, which
/// clears it away instead of leaving it to be found again on the next read.
pub fn consume(dir: &Path, realpath: &Path, now: u64) -> bool {
    let path = claim_path(dir, realpath);

    // Either there was no claim, or a concurrent read took it first. Both mean
    // the same thing here: this read is not the one that gets through.
    let Some(claimed) = claim::take(dir, &path) else {
        return false;
    };

    // Read the claim only now that it is ours, so nothing can change under us
    // between deciding and reading. A body we cannot parse is still a claim
    // somebody deliberately recorded, so it is honored rather than discarded on
    // a technicality.
    let recorded = claimed
        .text()
        .and_then(|text| serde_json::from_str::<Claim>(&text).ok())
        .map(|claim| claim.recorded_epoch);

    match recorded {
        Some(recorded) => now.saturating_sub(recorded) <= MAX_AGE_SECS,
        None => true,
    }
}

// ── The verb ─────────────────────────────────────────────────────────────────

/// `ss-magic plugin bypass <FILE>` — a human verb, so problems go to stderr
/// with a non-zero exit. Nothing here is reachable from a hook.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut target: Option<&str> = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{BYPASS_USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            flag if flag.starts_with('-') => {
                eprintln!(
                    "{}",
                    style::err(format!("error: unknown `bypass` flag `{flag}`"))
                );
                eprintln!("{BYPASS_USAGE}");
                return Ok(ExitCode::from(2));
            }
            path if target.is_none() => target = Some(path),
            extra => {
                eprintln!(
                    "{}",
                    style::err(format!("error: unexpected argument `{extra}`"))
                );
                eprintln!("{BYPASS_USAGE}");
                return Ok(ExitCode::from(2));
            }
        }
    }

    let Some(target) = target else {
        eprintln!(
            "{}",
            style::err("error: `bypass` needs the file to let through")
        );
        eprintln!("{BYPASS_USAGE}");
        return Ok(ExitCode::from(2));
    };

    let cwd = std::env::current_dir().context("reading the current directory")?;
    run_core(&cwd, target, now_secs())
}

/// `bypass` against an explicit directory and clock, so the whole flow is
/// testable without moving the process's working directory.
fn run_core(cwd: &Path, target: &str, now: u64) -> Result<ExitCode> {
    // The gate keys on the resolved physical path, so the claim has to as well:
    // reaching a file through a symlink and reaching it directly must be one
    // claim, not two.
    let realpath = match Path::new(target).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{}",
                style::err(format!("error: cannot resolve {target}: {e}"))
            );
            return Ok(ExitCode::from(2));
        }
    };

    // Bootstrapped through the same path every other state writer uses, so
    // `bypass` inherits its refusals — above all the one that declines to write
    // while git does not yet ignore the state tree, which is what keeps a claim
    // from turning up as an untracked file in the user's working copy.
    let report = scratchpad::ensure(cwd)?;
    if !report.wrote_state {
        for refusal in &report.refusals {
            eprintln!("{}", style::err(format!("refused: {refusal}")));
        }
        return Ok(ExitCode::from(1));
    }
    for refusal in &report.refusals {
        eprintln!("{}", style::warn(format!("refused: {refusal}")));
    }

    let dir = report.state_root.join(DIR_NAME);
    let path = record(&dir, &realpath, now)?;

    println!(
        "{}",
        style::ok(format!("The next Read of {target} will go through."))
    );
    println!("{}", style::info(format!("  claim {}", path.display())));
    println!(
        "{}",
        style::info("  The read after that is gated again. Unused claims expire after 24 hours.")
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests;
