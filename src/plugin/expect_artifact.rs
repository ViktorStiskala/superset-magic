//! Pending subagent-output declarations: `.superset/.magic/expect-artifact/<hash>.json`.
//!
//! A dispatched agent's report is tail-truncated by the harness when it runs
//! long, with the explicit marker *"the earlier part of the report is not
//! retrievable"* — the text is not spilled anywhere, it is gone. The way out
//! is for the agent to write its result to a file instead, and the way to make
//! that stick is to notice at stop time that it did not.
//!
//! So, before dispatching, the parent runs:
//!
//! ```text
//! ss-magic plugin expect-artifact docs/REPORT.md
//! ```
//!
//! which drops one record here. When a subagent stops, `hook/subagent_stop.rs`
//! takes the oldest pending record and looks at the file it names: present and
//! non-empty means the contract was kept and the record is simply retired;
//! missing, empty or not a file means the stop is blocked once, naming the
//! file, so the agent gets a chance to write it before it ends.
//!
//! ## Nothing pending means nothing is ever blocked (R51)
//!
//! That is the whole safety story for this feature. An agent nobody made a
//! declaration for stops exactly as it always did, and taking a record is
//! itself the one-shot flag: the record is claimed (and so gone) before the
//! block decision is made, so a second stop finds nothing pending and ends
//! normally. `stop_hook_active` is a second, independent guard on the same
//! property — see `hook/subagent_stop.rs`.
//!
//! ## A declaration names a file, not an agent
//!
//! Nothing in the payload could bind a record to a particular agent even if we
//! wanted it to: the declaration is made *before* `Task` runs, so no agent id
//! exists yet. A record is therefore keyed on the resolved file path —
//! declaring the same file twice is one declaration, not two — and a stop
//! takes the OLDEST pending record rather than trying to guess which agent it
//! belongs to. With several dispatches in flight that pairing can cross, so
//! the guarantee is deliberately stated per record rather than per agent:
//! **each declaration causes at most one block, and each stop is blocked at
//! most once.**
//!
//! ## Records expire (KTD14)
//!
//! A dispatch that crashes between declaring and spawning leaves a record
//! nothing will ever satisfy, and a stale record does real damage — the next
//! unrelated subagent to stop would be blocked for a file it was never asked
//! to write. [`MAX_AGE_SECS`] bounds that at six hours, and every stop sweeps
//! expired records out of the way on its path to the first live one, so the
//! directory does not accumulate them either.

use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::git;
use crate::hashing;
use crate::plugin::claim;
use crate::plugin::scratchpad;
use crate::tui::style;

/// The declaration directory's name under the state root. `scratchpad::ensure`
/// already creates it, so neither the verb nor the hook has to bootstrap a
/// directory mid-flight.
pub const DIR_NAME: &str = "expect-artifact";

/// Record file extension.
const RECORD_EXT: &str = "json";

/// How long a declaration stays in force. Six hours: a dispatch is consumed by
/// the very next subagent stop, ordinarily seconds or minutes later, so
/// anything still here long after that came from a dispatch that never
/// happened. Shorter than the Read gate's day-long bypass claims on purpose —
/// those wait for a person to come back to a read, these wait only for a
/// machine-paced stop, and a stale one costs an unrelated agent a spurious
/// block.
pub const MAX_AGE_SECS: u64 = 6 * 60 * 60;

/// Longest note carried into a block reason. The note is written by the
/// dispatching agent and read back by the dispatched one, and the reason
/// channel has no cap of its own, so it gets one here.
const MAX_NOTE_LEN: usize = 500;

/// Owner-only modes, matching the rest of the state tree (R58).
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// Usage for the `expect-artifact` verb.
const USAGE: &str = "\
Usage: ss-magic plugin expect-artifact <FILE> [--note TEXT]

Declare the file the subagent you are about to dispatch must produce. Run this
BEFORE the Task call, then tell the agent to write its result to that file.

When that agent stops and the file is missing, empty, or not a file, its stop
is blocked once, naming the file, so it can still write the result instead of
losing it to report truncation. It is blocked at most once, and with no
declaration in effect nothing is ever blocked.

  --note TEXT   What the file is for; shown to the agent when it is blocked.

Recorded under .superset/.magic/expect-artifact/. <FILE> must be inside this
worktree, and declarations expire after six hours.";

// ── The record ────────────────────────────────────────────────────────────────

/// One pending declaration, as it sits on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expectation {
    /// The declared file, resolved to an absolute path. This is what the stop
    /// handler stats — never re-derived from the record's file name, which is
    /// a one-way hash.
    pub path: String,
    /// The same file relative to the worktree root, which is how a person (and
    /// the blocked agent) recognizes it.
    pub relative: String,
    /// What the dispatcher said the file is for, if anything.
    #[serde(default)]
    pub note: Option<String>,
    /// When the declaration was made, seconds since the epoch — what orders
    /// records and ages them out, so neither depends on the record file's own
    /// mtime.
    pub declared_epoch: u64,
}

/// Why a declared file does not satisfy its declaration. `None` from
/// [`check`] means it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmet {
    /// Nothing is there, or it could not be stat'd at all.
    Missing,
    /// It is there and it is a file, but it holds no bytes. An empty file is
    /// the shape an agent leaves behind when it creates the file and then runs
    /// out of room to write it, which is exactly the loss this guards against.
    Empty,
    /// Something is there, but it is a directory or another non-file. The
    /// contract is a file, so this is not kept either.
    NotAFile,
}

impl Unmet {
    /// The clause that goes in the block reason, describing what was found.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Missing => "it does not exist",
            Self::Empty => "it exists but is empty",
            Self::NotAFile => "something is there, but it is not a file",
        }
    }

    /// A stable, machine-readable word for the heartbeat row's detail.
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Empty => "empty",
            Self::NotAFile => "not-a-file",
        }
    }
}

/// Whether the file at `path` keeps the declaration. `None` means it does.
///
/// A stat failure reads as [`Unmet::Missing`] rather than being ignored: a
/// file we cannot see is a file we cannot confirm, and the cost of being wrong
/// is one block on one stop, which the caller is allowed exactly once anyway.
pub fn check(path: &Path) -> Option<Unmet> {
    match fs::metadata(path) {
        Err(_) => Some(Unmet::Missing),
        Ok(meta) if !meta.is_file() => Some(Unmet::NotAFile),
        Ok(meta) if meta.len() == 0 => Some(Unmet::Empty),
        Ok(_) => None,
    }
}

// ── Layout ────────────────────────────────────────────────────────────────────

/// The declaration directory under a resolved state root — what
/// `scratchpad::ensure` hands back, so a caller that has already bootstrapped
/// the tree does not resolve the repository root a second time.
pub fn dir_in(state_root: &Path) -> PathBuf {
    state_root.join(DIR_NAME)
}

/// Where the record for `resolved` lives.
///
/// A hash rather than the path itself, for the reason
/// [`crate::plugin::bypass::claim_path`] gives: a repository path can be any
/// length and hold anything a filesystem allows, separators included, so
/// embedding it would produce names that are too long, nested, or simply
/// unrepresentable. Keying on the path also makes a repeated declaration of
/// the same file one record rather than a pile of them.
pub fn record_path(dir: &Path, resolved: &Path) -> PathBuf {
    let hash = hashing::fnv1a_64(resolved.as_os_str().as_bytes());
    dir.join(format!("{hash:016x}.{RECORD_EXT}"))
}

// ── Writing ───────────────────────────────────────────────────────────────────

/// Record a declaration for `resolved`, replacing any declaration already
/// standing for that file.
///
/// Written through a temp file and a rename so a concurrent stop never reads a
/// half-written record, and so a second declaration for one file leaves one
/// whole record rather than an interleaved one.
pub fn record(
    dir: &Path,
    resolved: &Path,
    relative: &str,
    note: Option<&str>,
    now: u64,
) -> Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let expectation = Expectation {
        path: resolved.to_string_lossy().into_owned(),
        relative: relative.to_string(),
        note: note.map(trim_note),
        declared_epoch: now,
    };
    let body = format!("{}\n", serde_json::to_string_pretty(&expectation)?);

    let path = record_path(dir, resolved);
    let mut tmp = tempfile::Builder::new()
        .prefix(".expect-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .with_context(|| format!("creating a temp file in {}", dir.display()))?;
    tmp.write_all(body.as_bytes())
        .context("writing the artifact declaration")?;
    tmp.flush().context("flushing the artifact declaration")?;
    tmp.as_file()
        .set_permissions(fs::Permissions::from_mode(FILE_MODE))
        .with_context(|| format!("setting owner-only mode on {}", path.display()))?;
    tmp.as_file().sync_all().ok();
    tmp.persist(&path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(path)
}

/// Bound a note and tidy its edges, so a runaway one cannot pad out a block
/// reason. Cut at a character boundary, since the note is arbitrary text.
fn trim_note(note: &str) -> String {
    let note = note.trim();
    if note.len() <= MAX_NOTE_LEN {
        return note.to_string();
    }
    let mut end = MAX_NOTE_LEN;
    while end > 0 && !note.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &note[..end])
}

// ── Taking one ────────────────────────────────────────────────────────────────

/// Claim the oldest declaration still in force, sweeping expired and unusable
/// records out of the way on the path to it.
///
/// Winning the claim IS taking the declaration (see [`crate::plugin::claim`]),
/// so what comes back has already been removed from the directory: the caller
/// is the only one that will ever see it, and a second stop finds nothing
/// pending. `None` means there is nothing in force, which is R51's "never
/// block" case.
///
/// Candidates are ordered by declaration time and each is claimed in turn, so
/// a caller that loses a race for one record simply moves on to the next
/// rather than dropping the whole sweep — two duplicate stops firing at the
/// same instant still check two records between them, not one twice.
pub fn take_oldest(dir: &Path, now: u64) -> Option<Expectation> {
    for path in candidates(dir) {
        let Some(claimed) = claim::take(dir, &path) else {
            // Gone, or another stop took it. Either way it is not ours to act
            // on; try the next-oldest.
            continue;
        };

        // Parsed only now that the record is ours, so nothing can rewrite it
        // between the decision and the read. Unlike a bypass claim, a record
        // we cannot parse cannot be honored — there is no file name in it to
        // enforce — so it is discarded, which at least clears it away.
        let Some(expectation) = claimed
            .text()
            .and_then(|text| serde_json::from_str::<Expectation>(&text).ok())
        else {
            continue;
        };

        if now.saturating_sub(expectation.declared_epoch) > MAX_AGE_SECS {
            // A dispatch that never happened. Dropped rather than enforced:
            // blocking an unrelated agent hours later for a file nobody asked
            // it for is worse than not enforcing at all.
            continue;
        }

        return Some(expectation);
    }
    None
}

/// Record paths in the directory, oldest declaration first.
///
/// Read before claiming, so the ordering is advisory — a record may be taken
/// by someone else, or rewritten, between being listed and being claimed. That
/// is fine: [`take_oldest`] re-reads whatever it actually wins. Records that
/// cannot be read or parsed sort last and are swept rather than trusted.
fn candidates(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut out: Vec<(u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(RECORD_EXT) {
            continue;
        }
        // A landing file from `claim::take` is `.taken-*.tmp` and a
        // half-written record is `.expect-*.tmp`, so neither reaches here; the
        // leading-dot check covers anything else that shows up hidden.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        let declared = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Expectation>(&text).ok())
            .map_or(u64::MAX, |e| e.declared_epoch);
        out.push((declared, path));
    }

    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, path)| path).collect()
}

// ── Resolving what was declared ───────────────────────────────────────────────

/// Resolve `target` against `cwd` and confirm it lands inside `root_canon`.
///
/// The declared file ordinarily does NOT exist yet — that is the whole point —
/// so `canonicalize` cannot be used on it directly. Instead the path is
/// normalized lexically (which is what removes any `..`), its deepest existing
/// ancestor is canonicalized so a symlinked parent cannot smuggle the
/// declaration out of the worktree, and the components that do not exist yet
/// are re-appended to that.
///
/// Containment is enforced here rather than at stop time so a bad declaration
/// is rejected while a person is looking at the error (R56). The stop handler
/// stats exactly the path this produced.
fn resolve_declared(cwd: &Path, root_canon: &Path, target: &str) -> Result<PathBuf, String> {
    if target.trim().is_empty() {
        return Err("the file to expect cannot be empty".to_string());
    }

    let joined = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        cwd.join(target)
    };
    let lexical = normalize(&joined);

    // Walk up to the deepest ancestor that exists, remembering what we passed.
    let mut existing = lexical.as_path();
    let mut missing: Vec<OsString> = Vec::new();
    while !existing.exists() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return Err(format!("cannot resolve {target}"));
        };
        missing.push(name.to_os_string());
        existing = parent;
    }

    let mut resolved = existing
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", existing.display()))?;
    for name in missing.iter().rev() {
        resolved.push(name);
    }

    if !resolved.starts_with(root_canon) {
        return Err(format!(
            "{} is outside this worktree ({}); a declared artifact has to live where the \
             subagent and the hook can both see it",
            resolved.display(),
            root_canon.display()
        ));
    }

    if resolved.is_dir() {
        return Err(format!(
            "{} is a directory; a declared artifact has to be a single file",
            resolved.display()
        ));
    }

    Ok(resolved)
}

/// Remove `.` components and resolve `..` textually. Purely lexical: the
/// caller canonicalizes the existing part afterwards, which is what handles
/// symlinks.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Seconds since the Unix epoch, or 0 if the clock is before it.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── The verb ─────────────────────────────────────────────────────────────────

/// `ss-magic plugin expect-artifact <FILE> [--note TEXT]` — a human verb, so
/// problems go to stderr with a non-zero exit. Nothing here is reachable from
/// a hook.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut target: Option<&str> = None;
    let mut note: Option<String> = None;

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--note" => match rest.next() {
                Some(text) => note = Some(text.clone()),
                None => return Ok(refuse("`--note` needs the text that follows it")),
            },
            flag if flag.starts_with("--note=") => {
                note = Some(flag.trim_start_matches("--note=").to_string());
            }
            flag if flag.starts_with('-') => {
                return Ok(refuse(format!("unknown `expect-artifact` flag `{flag}`")));
            }
            path if target.is_none() => target = Some(path),
            extra => return Ok(refuse(format!("unexpected argument `{extra}`"))),
        }
    }

    let Some(target) = target else {
        return Ok(refuse(
            "`expect-artifact` needs the file the subagent must produce",
        ));
    };

    let cwd = std::env::current_dir().context("reading the current directory")?;
    run_core(&cwd, target, note.as_deref(), now_secs())
}

/// Report a usage problem the same way every time, and exit 2.
fn refuse(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("{}", style::err(format!("error: {message}")));
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// `expect-artifact` against an explicit directory and clock, so the whole
/// flow is testable without moving the process's working directory.
fn run_core(cwd: &Path, target: &str, note: Option<&str>, now: u64) -> Result<ExitCode> {
    let root = match git::cwd_repo_root(cwd) {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: `expect-artifact` must run inside a git worktree: {e}"
                ))
            );
            return Ok(ExitCode::from(2));
        }
    };
    let root_canon = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;

    let resolved = match resolve_declared(cwd, &root_canon, target) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("{}", style::err(format!("error: {e}")));
            return Ok(ExitCode::from(2));
        }
    };
    let relative = resolved
        .strip_prefix(&root_canon)
        .unwrap_or(&resolved)
        .to_string_lossy()
        .into_owned();

    // Bootstrapped through the same path every other state writer uses, so
    // this verb inherits its refusals — above all the one that declines to
    // write while git does not yet ignore the state tree, which is what keeps
    // a declaration from turning up as an untracked file in the user's
    // working copy.
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

    let dir = dir_in(&report.state_root);
    let path = record(&dir, &resolved, &relative, note, now)?;

    println!(
        "{}",
        style::ok(format!(
            "Expecting {relative} from the next subagent to stop."
        ))
    );
    println!("{}", style::info(format!("  record {}", path.display())));
    println!(
        "{}",
        style::info(
            "  Tell the agent to write its result there. If the file is missing or empty \
             when it stops, that stop is blocked once."
        )
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests;
