//! `hooks.jsonl` — one appended line per hook invocation, machine-level.
//!
//! Every `ss-magic plugin hook …` run leaves exactly one row here before it
//! exits, including the runs that did nothing and the runs that failed. That
//! is the whole point: a hook is advisory and fails open, so an ss-magic that
//! silently stopped acting looks, from the outside, exactly like an ss-magic
//! that had nothing to do. The log is what makes the difference visible, and
//! it is what `ss-magic plugin status` reads to report last-fired-at and the
//! outcome counts per event.
//!
//! ## Why machine-level, and why the data path
//!
//! A hook can fire outside any git repository, and the rows must survive the
//! deletion of the worktree they describe — a Superset worktree is a
//! disposable thing, and "what did the plugin do in that worktree" is a
//! question people ask after it is gone. So the log lives beside the cost
//! ledger in ss-magic's `ProjectDirs` app root rather than inside any
//! worktree, and `status` filters by cwd when it is run inside one.
//!
//! It takes `data_dir()`, not the `cache_dir()` `src/update/check.rs` uses for
//! its version-check cache. That module caches something it can always fetch
//! again; this is a record of things that happened, which nothing can
//! reconstruct. A history that vanishes on a disk-cleanup run is worse than no
//! history at all, because nothing reports that it happened.
//!
//! ## Writing is best-effort, and bounded
//!
//! [`append`] takes an exclusive advisory lock, appends one line, and then
//! prunes. The lock is the same `fd_lock::RwLock` pattern the rest of the
//! crate uses — reused through [`crate::plugin::tmproot::with_lock`] rather
//! than reimplemented — because concurrent hook invocations are the normal
//! case: handlers registered on one event all fire at once, and two sessions
//! on one machine share this file.
//!
//! Pruning keeps the newest [`ROWS_KEPT`] rows and drops anything older than
//! [`MAX_AGE_SECS`], mirroring `reverse_sync::prune_old_backups`: bounded,
//! best-effort, and never the reason a hook fails. It only runs once the file
//! has grown past [`PRUNE_TRIGGER_BYTES`], so the ordinary invocation pays one
//! `stat` rather than a full read-filter-rewrite — this sits on the model's
//! clock for every `PreToolUse`.

use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::plugin::scratchpad::format_rfc3339;
use crate::plugin::tmproot;

// ── Layout ────────────────────────────────────────────────────────────────────

/// Subdirectory of ss-magic's app data root that holds every machine-level
/// plugin file. The cost ledger and its offsets store land here too.
pub const STORE_SUBDIR: &str = "plugin";

/// The heartbeat log's file name inside the store.
pub const LOG_FILE_NAME: &str = "hooks.jsonl";

/// The lock guarding append-plus-prune. A sibling file rather than the log
/// itself, matching `update/apply.rs`'s `update.lock`: the lock's lifetime and
/// the log's are unrelated, and locking a file we also truncate-and-replace
/// during a prune would mean holding a lock on an unlinked inode.
const LOCK_FILE_NAME: &str = "hooks.lock";

/// Owner-only mode for everything this module creates. The rows name
/// repository paths and, through them, the projects someone works on.
const DIR_MODE: u32 = 0o700;
/// Owner-only mode for the log file itself.
const FILE_MODE: u32 = 0o600;

// ── Retention ─────────────────────────────────────────────────────────────────

/// How many rows survive a prune. Sized to be useful rather than complete:
/// enough to cover a heavy day's sessions when someone asks `status` why the
/// gate stopped firing, and small enough that the whole file is a cheap read.
pub const ROWS_KEPT: usize = 2_000;

/// How old a row may be before a prune drops it, regardless of the count
/// bound: 30 days.
pub const MAX_AGE_SECS: u64 = 30 * 24 * 60 * 60;

/// File size past which an append also prunes. Below it the append is a pure
/// O(1) operation, which matters because every `PreToolUse` pays for this on
/// the model's clock. 256 KiB is comfortably more than [`ROWS_KEPT`] typical
/// rows, so a prune that has already run leaves the file back under the
/// trigger.
pub const PRUNE_TRIGGER_BYTES: u64 = 256 * 1024;

// ── The row ───────────────────────────────────────────────────────────────────

/// How an invocation ended. Three values, deliberately: `status` counts them
/// per event, and a set that grows with every new refusal reason would make
/// those counts meaningless. The specific reason lives in [`Row::reason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// A handler ran to completion.
    Ok,
    /// The wrapper deliberately did not run the handler — the plugin is not
    /// enabled here, the state tree is not ignored, the event is one this
    /// binary cannot route, or there was no envelope to act on.
    NoOp,
    /// Something failed and the invocation fell open.
    Error,
}

/// One line of `hooks.jsonl`.
///
/// Both time fields describe the same instant. `at` is for a person reading
/// the log; `ts` is what pruning compares against, and having it avoids
/// carrying an RFC 3339 *parser* around for the sake of an age check — this
/// crate has no date crate and is not about to grow one for that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// The event as argv named it. Empty for `plugin hook` with no token at
    /// all, which carries no name to report.
    pub event: String,
    /// UTC, RFC 3339, to the second.
    pub at: String,
    /// The same instant, as whole seconds since the Unix epoch.
    pub ts: u64,
    /// The envelope's `cwd`, when there was a decodable envelope to take it
    /// from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// A stable machine-readable class — `disabled`, `not-ignored`,
    /// `malformed-stdin`, … Absent on a plain success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Human-readable detail: which rule is missing, what the handler decided,
    /// what the error said. Free text, never parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Row {
    /// A row stamped `now` (seconds since the Unix epoch).
    pub fn new(event: &str, now: u64, outcome: Outcome) -> Self {
        Self {
            event: event.to_string(),
            at: format_rfc3339(now),
            ts: now,
            cwd: None,
            outcome,
            reason: None,
            detail: None,
        }
    }

    /// Replace the outcome. Lets a caller build the common part of a row once
    /// and then say how it actually ended.
    pub fn with_outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }

    /// Attach the envelope's working directory.
    pub fn with_cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd;
        self
    }

    /// Attach the machine-readable class.
    pub fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_string());
        self
    }

    /// Attach the human-readable detail.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

// ── The store ─────────────────────────────────────────────────────────────────

/// Where the machine-level plugin store lives, WITHOUT creating anything.
///
/// macOS `~/Library/Application Support/ss-magic/plugin`, Linux XDG
/// `~/.local/share/ss-magic/plugin`. `None` when the platform has no home
/// directory to resolve. Split out from [`store_dir`] so the path rule can be
/// asserted without a test suite quietly creating directories in the
/// developer's own app data.
fn store_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "ss-magic")?;
    Some(dirs.data_dir().join(STORE_SUBDIR))
}

/// [`store_path`], created owner-only if it is not there yet.
///
/// `None` when there is no path to resolve, or when the directory cannot be
/// created; the caller then runs without a heartbeat rather than failing, since
/// a hook that refuses to work because it cannot write its own log would defeat
/// the purpose of having one.
pub fn store_dir() -> Option<PathBuf> {
    let dir = store_path()?;
    ensure_store(&dir).ok()?;
    Some(dir)
}

/// Create `dir` and its parents at owner-only mode, or confirm it is already a
/// directory. `DirBuilder`'s mode applies to every component it creates, so a
/// first run stamps the whole `…/ss-magic/plugin` chain 0700.
fn ensure_store(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
        .with_context(|| format!("creating the plugin store at {}", dir.display()))
}

/// The heartbeat log's path inside `store`.
pub fn log_path(store: &Path) -> PathBuf {
    store.join(LOG_FILE_NAME)
}

// ── Append ────────────────────────────────────────────────────────────────────

/// Append `row` to the log in `store`, then prune if the file has grown past
/// the trigger.
///
/// Returns `Err` when the row could not be written. The caller reports that on
/// stderr and carries on: a hook whose heartbeat fails has still done its job,
/// and R50's "always leave a row" is a promise about this function's caller
/// always calling it, not a promise that the disk always cooperates.
///
/// A prune failure is folded into the returned error only if the append itself
/// also failed; a successful append followed by a failed prune returns `Ok`,
/// because the row — the thing that matters — is on disk.
pub fn append(store: &Path, row: &Row) -> Result<()> {
    ensure_store(store)?;
    let path = log_path(store);

    // The whole append-plus-prune runs under one lock: a prune rewrites the
    // file wholesale, so a concurrent appender holding no lock would write into
    // the inode we are about to replace and lose its row.
    let outcome = tmproot::with_lock(store, LOCK_FILE_NAME, || {
        write_row(&path, row)?;
        // Best-effort by construction (KTD14's posture): the prune's own
        // failure is swallowed here so it can never turn a successful append
        // into a reported failure.
        let _ = maybe_prune(&path, now_secs_or(row.ts));
        Ok::<(), anyhow::Error>(())
    })
    .with_context(|| format!("locking the heartbeat log in {}", store.display()))?;

    outcome
}

/// Append one JSON line. The file is created owner-only if absent; an existing
/// file's mode is left alone, because it is the user's to change.
fn write_row(path: &Path, row: &Row) -> Result<()> {
    let line = serde_json::to_string(row).context("encoding a heartbeat row")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(FILE_MODE)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(file, "{line}").with_context(|| format!("appending to {}", path.display()))
}

/// The current wall clock in seconds, falling back to `fallback` if the system
/// clock is before the Unix epoch. Only used to age rows, where being a few
/// seconds off is immaterial.
fn now_secs_or(fallback: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(fallback)
}

// ── Prune ─────────────────────────────────────────────────────────────────────

/// Prune only once the file is big enough to be worth reading. The common case
/// costs one `stat`.
fn maybe_prune(path: &Path, now: u64) -> Result<usize> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size < PRUNE_TRIGGER_BYTES {
        return Ok(0);
    }
    prune(path, ROWS_KEPT, MAX_AGE_SECS, now)
}

/// Drop rows older than `max_age` seconds, then everything but the newest
/// `keep`, and rewrite the file atomically. Returns how many rows were
/// removed.
///
/// A line that does not parse as a [`Row`] is dropped: it has no timestamp to
/// age and no outcome to count, so keeping it would mean the file grows
/// without bound in exactly the case where something is already writing
/// garbage into it.
///
/// Must be called with the log's lock held — it replaces the file wholesale.
fn prune(path: &Path, keep: usize, max_age: u64, now: u64) -> Result<usize> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        // Nothing to prune if the log is not there (or is not readable text);
        // the next append recreates it.
        Err(_) => return Ok(0),
    };

    let total = text.lines().filter(|l| !l.trim().is_empty()).count();
    let cutoff = now.saturating_sub(max_age);
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| match serde_json::from_str::<Row>(line) {
            // A row stamped in the future (a clock that jumped backwards since
            // it was written) is kept: it is newer than the cutoff, and
            // discarding real history over a clock anomaly is the worse error.
            Ok(row) => row.ts >= cutoff,
            Err(_) => false,
        })
        .collect();

    let start = kept.len().saturating_sub(keep);
    let kept = &kept[start..];
    if kept.len() == total {
        return Ok(0);
    }

    let mut body = kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    write_atomically(path, &body, ".hooks-")?;
    Ok(total - kept.len())
}

/// Replace `path`'s contents with `body` via a temp file in the same
/// directory, so a crash mid-write leaves the old file intact rather than a
/// truncated one. `prefix` names the temp file, which only ever matters when
/// somebody is looking at the directory mid-write.
///
/// Shared with `plugin::ledger`, whose ledger, offsets store and price
/// snapshots need exactly this and must not grow a second copy of it: the
/// owner-only mode and the same-directory rename are the parts that would
/// drift.
pub fn write_atomically(path: &Path, body: &str, prefix: &str) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(".jsonl")
        .tempfile_in(dir)
        .with_context(|| format!("creating a temp file in {}", dir.display()))?;
    tmp.write_all(body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    tmp.flush()
        .with_context(|| format!("flushing {}", path.display()))?;
    tmp.as_file()
        .set_permissions(std::fs::Permissions::from_mode(FILE_MODE))
        .with_context(|| format!("setting owner-only mode on {}", path.display()))?;
    tmp.persist(path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

// ── Read ──────────────────────────────────────────────────────────────────────

/// Every row in the log, oldest first. Unparseable lines are skipped rather
/// than failing the read, so one bad line does not hide the rest of the
/// history from `status`.
///
/// An absent log is an empty `Vec`, not an error: no hook has fired yet.
// `status` (U28) is the production caller; the tests here read it directly.
#[allow(dead_code)]
pub fn read(store: &Path) -> Result<Vec<Row>> {
    let path = log_path(store);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Row>(line).ok())
        .collect())
}

#[cfg(test)]
mod tests;
