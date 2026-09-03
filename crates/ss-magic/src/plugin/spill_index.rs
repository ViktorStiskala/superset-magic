//! `ss-magic plugin spill-index` — the harness's own spill files, listed.
//!
//! When a tool's output is too large to hand the model, Claude Code writes the
//! whole thing to a file and returns a pointer to it instead:
//!
//! ```plaintext
//! Output too large (195.3KB). Full output saved to:
//!   ~/.claude/projects/<encoded-cwd>/<session-uuid>/tool-results/<id>.txt
//! ```
//!
//! Those files are full fidelity, survive the session and stay greppable for
//! about a month — but the leaf names are unguessable short ids and there is no
//! index anywhere. On this machine they numbered over a thousand, scattered
//! across ninety-odd per-session directories. That is the gap this verb closes:
//! ss-magic builds no spill mechanism of its own (the harness's works), it just
//! makes the harness's output findable.
//!
//! ## Strictly read-only
//!
//! Nothing here creates, moves or deletes anything, in the harness's tree or in
//! the worktree. It is a `read_dir` plus a `stat` per file, and that is the
//! whole contract — these are the harness's files, and a tool that tidied them
//! would be deleting evidence somebody is about to grep.
//!
//! ## Finding the directory
//!
//! The per-project directory is named after the absolute working directory it
//! belongs to, with the path's punctuation flattened — `/Users/me/.superset/wt`
//! becomes `-Users-me--superset-wt`. The leaf spill names cannot be
//! reconstructed, but that directory name can, so the verb derives it from the
//! git repository root (so running it from a subdirectory finds the same tree)
//! and enumerates every `<session-uuid>/tool-results/` beneath it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::git;
use crate::plugin::scratchpad::format_rfc3339;
use crate::tui::style;

/// The subdirectory of a session directory the harness spills into.
const TOOL_RESULTS_DIR: &str = "tool-results";

// ── The index ─────────────────────────────────────────────────────────────────

/// One spilled tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpillFile {
    /// Absolute path, which is what a caller greps or reads.
    pub path: String,
    /// The leaf name, which is the id the harness printed in the transcript.
    pub name: String,
    /// Size on disk.
    pub bytes: u64,
    /// Last-modified, RFC 3339 UTC — `None` when the filesystem reports no
    /// modification time, with [`SpillFile::note`] saying so.
    pub modified: Option<String>,
    /// The same instant in whole seconds, for sorting and for a caller that
    /// wants to do arithmetic rather than parse a string.
    pub modified_ts: Option<u64>,
    /// Why a field above is absent. `None` when everything was readable.
    pub note: Option<String>,
}

/// One harness session's spill directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpillSession {
    /// The session uuid, taken from the directory name.
    pub session_id: String,
    /// Absolute path of the `tool-results/` directory itself.
    pub path: String,
    /// Its files, newest first.
    pub files: Vec<SpillFile>,
    /// Total bytes across `files`.
    pub bytes: u64,
    /// Why the listing is short or empty, when it is.
    pub note: Option<String>,
}

/// The whole answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Index {
    /// The worktree the index is for.
    pub root: String,
    /// The harness's projects directory, or `None` with a [`Index::note`] when
    /// it could not be located.
    pub projects_root: Option<String>,
    /// The per-project directory for `root`, or `None` when the harness has
    /// never recorded a session there.
    pub project_dir: Option<String>,
    /// Sessions with at least one spill directory, newest first.
    pub sessions: Vec<SpillSession>,
    /// Files across every session.
    pub files: usize,
    /// Bytes across every session.
    pub bytes: u64,
    /// Why the answer is empty or partial. Always set when `project_dir` is
    /// `None`, so an empty result never reads as "there are none".
    pub note: Option<String>,
}

// ── Locating the harness's tree ───────────────────────────────────────────────

/// Where the harness keeps per-project session state: `$CLAUDE_CONFIG_DIR/
/// projects`, or `~/.claude/projects`. `None` when neither is resolvable, which
/// is the case a caller must report rather than treat as "no spills".
///
/// Deliberately the same resolution `plugin::ledger` uses for transcripts —
/// they are siblings in the same tree, and two different answers to "where does
/// the harness keep this" would be a bug waiting to happen.
pub fn projects_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("projects"));
        }
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// The harness's directory name for `root`: every character outside
/// `[A-Za-z0-9-]` replaced by `-`, with no collapsing of the runs that produces
/// and no trimming of the leading `-` an absolute path always yields. So
/// `/Users/me/.superset/wt` is `-Users-me--superset-wt` — the doubled dash is
/// the `/` and the `.` each mapping to one.
///
/// `keep_underscore` exists because underscores are the one character class
/// this encoding was not directly observed on: no directory name among the two
/// hundred-odd sampled contained one, which is suggestive but not proof that
/// `_` is replaced rather than kept. [`project_dir_for`] simply tries both
/// spellings, which costs one `is_dir` and removes the guess.
fn encode_root(root: &Path, keep_underscore: bool) -> String {
    root.to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || (keep_underscore && ch == '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// The per-project directory for `root` under `projects_root`, or `None` when
/// the harness has no directory for it — either because no session has ever run
/// there, or because the harness's layout is not the one this encoding
/// describes.
fn project_dir_for(projects_root: &Path, root: &Path) -> Option<PathBuf> {
    for keep_underscore in [false, true] {
        let candidate = projects_root.join(encode_root(root, keep_underscore));
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

// ── Collecting ────────────────────────────────────────────────────────────────

/// Build the index for `root` under `projects_root`. Both are parameters rather
/// than read from the environment so the whole thing is testable against a
/// fixture tree.
///
/// Never fails: an unreadable directory becomes a note on the enclosing level
/// rather than an error, because a permission problem on one session must not
/// hide the other ninety.
pub fn collect(projects_root: &Path, root: &Path) -> Index {
    let mut index = Index {
        root: root.display().to_string(),
        projects_root: Some(projects_root.display().to_string()),
        project_dir: None,
        sessions: Vec::new(),
        files: 0,
        bytes: 0,
        note: None,
    };

    if !projects_root.is_dir() {
        index.note = Some(format!(
            "the harness keeps no project state at {} — nothing has run here, \
             or the harness stores it somewhere else",
            projects_root.display()
        ));
        return index;
    }

    let Some(project_dir) = project_dir_for(projects_root, root) else {
        index.note = Some(format!(
            "no harness project directory for {} under {} — no session has \
             recorded anything for this worktree",
            root.display(),
            projects_root.display()
        ));
        return index;
    };
    index.project_dir = Some(project_dir.display().to_string());

    let entries = match fs::read_dir(&project_dir) {
        Ok(entries) => entries,
        Err(e) => {
            index.note = Some(format!("could not list {}: {e}", project_dir.display()));
            return index;
        }
    };

    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path().join(TOOL_RESULTS_DIR);
        if !dir.is_dir() {
            // A session that never spilled has no `tool-results/` at all, which
            // is the common case and not worth a note.
            continue;
        }
        let session_id = entry.file_name().to_string_lossy().into_owned();
        index.sessions.push(collect_session(&session_id, &dir));
    }

    // Newest first at both levels, so the file somebody is looking for — almost
    // always the one from the run they just watched — is at the top.
    index
        .sessions
        .sort_by(|a, b| newest(b).cmp(&newest(a)).then_with(|| a.path.cmp(&b.path)));

    index.files = index.sessions.iter().map(|s| s.files.len()).sum();
    index.bytes = index.sessions.iter().map(|s| s.bytes).sum();

    if index.sessions.is_empty() {
        index.note = Some(format!(
            "no session under {} has spilled a tool result",
            project_dir.display()
        ));
    }

    index
}

/// The newest modification time in a session, or 0 when nothing in it reports
/// one. Only used for ordering.
fn newest(session: &SpillSession) -> u64 {
    session
        .files
        .iter()
        .filter_map(|f| f.modified_ts)
        .max()
        .unwrap_or(0)
}

/// List one `tool-results/` directory.
fn collect_session(session_id: &str, dir: &Path) -> SpillSession {
    let mut session = SpillSession {
        session_id: session_id.to_string(),
        path: dir.display().to_string(),
        files: Vec::new(),
        bytes: 0,
        note: None,
    };

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            session.note = Some(format!("could not list {}: {e}", dir.display()));
            return session;
        }
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // `symlink_metadata`, not `metadata`: these are the harness's files and
        // a symlink among them is reported as what it is rather than followed.
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) => {
                session.files.push(SpillFile {
                    path: path.display().to_string(),
                    name,
                    bytes: 0,
                    modified: None,
                    modified_ts: None,
                    note: Some(format!("could not stat it: {e}")),
                });
                continue;
            }
        };
        if meta.is_dir() {
            continue;
        }
        let ts = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        session.files.push(SpillFile {
            path: path.display().to_string(),
            name,
            bytes: meta.len(),
            modified: ts.map(format_rfc3339),
            modified_ts: ts,
            note: match ts {
                Some(_) => None,
                None => Some("this filesystem reports no modification time for it".to_string()),
            },
        });
    }

    session.files.sort_by(|a, b| {
        b.modified_ts
            .cmp(&a.modified_ts)
            .then_with(|| a.name.cmp(&b.name))
    });
    session.bytes = session.files.iter().map(|f| f.bytes).sum();
    session
}

// ── The verb ──────────────────────────────────────────────────────────────────

/// Usage for `spill-index`.
const SPILL_INDEX_USAGE: &str = "\
Usage: ss-magic plugin spill-index [OPTIONS]

List the spill files Claude Code wrote for this worktree — the full-fidelity
copies of tool output that was too large to return inline. Read-only: nothing
is created, moved or removed.

Options:
  --json              Machine-readable output
  -h, --help          This text";

/// `ss-magic plugin spill-index` — a human verb, so problems go to stderr with
/// a non-zero exit.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{SPILL_INDEX_USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--json" => json = true,
            other => {
                eprintln!(
                    "{}",
                    style::err(format!("error: unexpected argument `{other}`"))
                );
                eprintln!("{SPILL_INDEX_USAGE}");
                return Ok(ExitCode::from(2));
            }
        }
    }

    let cwd = std::env::current_dir().context("reading the current directory")?;
    // The repository root, so running this from a subdirectory finds the same
    // harness directory a session started at the root would have. Outside a
    // repository the cwd is the best available answer, and the harness names
    // its directory after whatever directory the session actually ran in.
    let root = git::cwd_repo_root(&cwd).unwrap_or(cwd);

    let Some(projects) = projects_root() else {
        let index = Index {
            root: root.display().to_string(),
            projects_root: None,
            project_dir: None,
            sessions: Vec::new(),
            files: 0,
            bytes: 0,
            note: Some(
                "cannot locate the harness's project directory: neither \
                 $CLAUDE_CONFIG_DIR nor $HOME is set"
                    .to_string(),
            ),
        };
        return emit(&index, json).map(|()| ExitCode::from(1));
    };

    let index = collect(&projects, &root);
    emit(&index, json)?;
    Ok(ExitCode::SUCCESS)
}

/// Render an index, as JSON or for a person.
fn emit(index: &Index, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(index)?);
        return Ok(());
    }

    println!(
        "{}",
        style::header(format!("Spill files for {}", index.root))
    );
    match &index.projects_root {
        Some(path) => println!("{}", style::info(format!("  harness state: {path}"))),
        None => println!("{}", style::warn("  harness state: unknown")),
    }
    if let Some(dir) = &index.project_dir {
        println!("{}", style::info(format!("  project dir:   {dir}")));
    }
    // A note is printed whether or not anything was found: an empty listing
    // with no explanation reads as "there are none", which is exactly the wrong
    // conclusion when the real answer is "this could not be looked up".
    if let Some(note) = &index.note {
        println!("{}", style::warn(format!("  {note}")));
    }
    if index.sessions.is_empty() {
        return Ok(());
    }

    println!();
    for session in &index.sessions {
        println!(
            "{}",
            style::ok(format!(
                "{}  ({} file{}, {})",
                session.session_id,
                session.files.len(),
                plural(session.files.len()),
                human_bytes(session.bytes)
            ))
        );
        if let Some(note) = &session.note {
            println!("{}", style::warn(format!("  {note}")));
        }
        for file in &session.files {
            let when = file.modified.as_deref().unwrap_or("unknown time");
            println!(
                "{}",
                style::info(format!(
                    "  {when}  {:>9}  {}",
                    human_bytes(file.bytes),
                    file.path
                ))
            );
            if let Some(note) = &file.note {
                println!("{}", style::warn(format!("      {note}")));
            }
        }
        println!();
    }
    println!(
        "{}",
        style::info(format!(
            "{} file{} across {} session{}, {}",
            index.files,
            plural(index.files),
            index.sessions.len(),
            plural(index.sessions.len()),
            human_bytes(index.bytes)
        ))
    );
    Ok(())
}

/// `""` or `"s"`.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Bytes rendered for a person. Deliberately coarse — the point of the column
/// is "is this the big one", not an exact figure.
fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests;
