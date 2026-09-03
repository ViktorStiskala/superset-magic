//! `file-changed` — refresh the session environment through direnv when a
//! `.env` or `.envrc` changes, without ever running what the repository wrote
//! (R92).
//!
//! ## This handler does not ship enabled, and the reason matters
//!
//! R92 reversed an earlier refusal of this event, but gated the reversal on a
//! probe: the event had to be shown, on the pinned harness version, to fire on
//! an `.envrc` write and report a usable `cwd` with two worktrees of one
//! repository open. That probe is **not** satisfied, so no `FileChanged` entry
//! is declared in `plugin/hooks/hooks.json` and nothing in a real session
//! reaches this module. It is written, tested and correct so that landing the
//! feature later is a manifest change rather than a rewrite.
//!
//! Two findings from reading the 2.1.251 bundle explain the gap:
//!
//! - **A `FileChanged` entry's `matcher` is its watch-path list, not a name
//!   pattern.** The harness collects watch paths by splitting each entry's
//!   `matcher` on `|` (resolving relative entries against the session cwd) and
//!   skips any entry that has no matcher at all. An entry written the way the
//!   other events are written — no matcher, filter inside the handler —
//!   therefore registers nothing to watch and can never fire. Whatever ships
//!   eventually has to name the paths up front, or emit `watchPaths` from a
//!   `SessionStart` hook, which is a different design than R92 assumes.
//! - **Cross-worktree behaviour is still unmeasured.** The payload's `cwd`
//!   comes from the session, while `file_path` comes from the watcher, so two
//!   worktrees of one repository is exactly the case that decides whether the
//!   pair can be trusted — and it is the case Q12 refused the event over in
//!   the first place. Reading a bundle cannot answer it; only a live session
//!   can.
//!
//! The one thing the bundle did settle in R92's favour is the export target:
//! `CLAUDE_ENV_FILE` is placed in the hook's environment for `SessionStart`,
//! `Setup`, `CwdChanged` and `FileChanged`, so the channel R92 depends on does
//! exist on this event.
//!
//! ## The whole unit is about not running the repository's shell
//!
//! An `.envrc` is a shell script the repository ships, and loading one is how
//! direnv works. Unguarded, that makes "open a session in a freshly cloned
//! repository" into "run that repository's code", which is the single failure
//! this module exists to prevent.
//!
//! So the handler never asks direnv to load anything on its own authority. It
//! asks a read-only question first — `direnv status --json`, which finds and
//! hashes the rc file but does not source it — and proceeds only when direnv
//! reports that the user has *already* granted this exact file content trust.
//! It never runs `direnv allow`, `direnv permit` or anything else that would
//! grant that trust, so the answer to "may this run?" always comes from a
//! decision made outside ss-magic. An rc file that is merely unknown is
//! refused exactly as firmly as one that was explicitly revoked.
//!
//! ## Where the exported values go, and where they must never go
//!
//! Exported values are secrets. Two rules follow, and both are about the
//! plausible-looking wrong answer rather than the obvious one:
//!
//! - **The target is the harness's, never ours.** The handler writes only to
//!   the path the harness put in `CLAUDE_ENV_FILE`, appends to it, and refuses
//!   if it resolves to somewhere inside the worktree. With no such path it does
//!   nothing at all. Choosing a "sensible" fallback inside the repository is
//!   how secrets get committed, so there is no fallback.
//! - **Nothing exported is ever recorded.** The heartbeat row says that an
//!   export happened; it never says what was in it. No value reaches the state
//!   tree, the heartbeat or the cost ledger.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::plugin::hook::event::{FileChanged, Payload};
use crate::plugin::hook::{HookContext, Outcome};
use crate::plugin::tmproot;

/// The program consulted and, once it has said yes, asked to export.
///
/// Threaded through the helpers as a parameter rather than named at each call
/// site so tests can point them at a stand-in script — which is what keeps
/// this module's tests from ever executing a real `.envrc`. Production passes
/// this constant and nothing else.
const DIRENV: &str = "direnv";

/// The environment variable naming the file the harness wants exports written
/// into. R92 makes this the only acceptable target; see the module docs.
const ENV_FILE_VAR: &str = "CLAUDE_ENV_FILE";

/// Owner-only (R58). The file holds exported secrets, and a group- or
/// world-readable mode on it is the difference between a secret and a
/// published one. This applies at creation only: when the harness created the
/// file first, its mode is the harness's business and appending leaves it be.
const FILE_MODE: u32 = 0o600;

/// The serialization point for concurrent invocations.
///
/// The harness gives each hook entry its own `CLAUDE_ENV_FILE`, but one entry
/// can still be running twice at once — a `.env` and a `.envrc` saved together
/// produce two events against the same file. Two unsynchronized appends of a
/// multi-kilobyte export can interleave into a file that is no longer valid
/// shell, so every invocation takes this lock before writing. One lock for the
/// whole event is deliberate: over-serializing costs a few milliseconds, and
/// per-target lock names would need a hash of a path for no real gain.
const LOCK_NAME: &str = "file-changed.lock";

/// direnv's `allowed` code for "the user has allowed this exact content".
///
/// The other two it reports are `1` (never allowed) and `2` (explicitly
/// revoked). Both must refuse — and so must any value a future direnv invents,
/// which is why the gate below tests `== ALLOWED` rather than excluding the
/// two known-bad codes. The unknown answer has to be the safe one.
const ALLOWED: i64 = 0;

/// The file names direnv treats as an rc file, and so the only changes this
/// handler reacts to. Note that `.env` is one of them: direnv loads a bare
/// `.env` as an rc in its own right, and gates it behind the same allow list,
/// which is why both names take the identical path through this module.
const RC_NAMES: [&str; 2] = [".envrc", ".env"];

/// The `file-changed` handler wired into [`crate::plugin::hook::route`].
///
/// The environment read is the whole of this function because everything below
/// it is driven by parameters instead: that keeps the real logic testable
/// without a test mutating process-global state that its neighbours are
/// reading at the same moment.
pub(crate) fn handle(ctx: &HookContext<'_>) -> Result<Outcome> {
    let target = std::env::var(ENV_FILE_VAR).unwrap_or_default();
    handle_with(ctx, DIRENV, &target)
}

/// [`handle`], with the two pieces of outside world named explicitly.
fn handle_with(ctx: &HookContext<'_>, program: &str, raw_target: &str) -> Result<Outcome> {
    let payload = match &ctx.envelope.payload {
        Payload::FileChanged(payload) => payload,
        // Unreachable through `hook::route`, which builds the variant from the
        // argv token. Doing nothing beats a `match` arm that could panic on a
        // future wiring mistake.
        _ => return Ok(Outcome::silent().with_detail("not a file-changed payload")),
    };

    let Some(changed) = first_rc_path(payload) else {
        return Ok(Outcome::silent().with_detail("no .env or .envrc among the changed paths"));
    };

    // `unlink` means the rc file is gone. direnv would either find nothing or
    // find some ancestor's rc, and re-exporting an unrelated directory's
    // environment because a file was deleted is not what R92 asks for.
    if payload.event.as_deref() == Some("unlink") {
        return Ok(Outcome::silent().with_detail("the rc file was removed; nothing to export"));
    }

    // Every path the watcher reports is absolute. A relative one would have to
    // be resolved against a guess, and every guess here is a guess about where
    // to look for secrets.
    let changed = Path::new(&changed);
    let Some(dir) = changed
        .parent()
        .filter(|d| changed.is_absolute() && d.is_dir())
    else {
        return Ok(Outcome::silent()
            .with_detail("the changed path is not an absolute path in an existing directory"));
    };

    let target = match resolve_target(raw_target) {
        Ok(target) => target,
        Err(why) => return Ok(Outcome::silent().with_detail(why)),
    };

    // R92's boundary. Both roots are checked because they can differ — a hook
    // can run with a `cwd` below the repository root — and a target inside
    // either one is a target that a `git add .` can pick up.
    for boundary in [ctx.repo_root.as_deref(), Some(ctx.cwd())]
        .into_iter()
        .flatten()
    {
        if within(&target, boundary) {
            return Ok(Outcome::silent().with_detail(format!(
                "{ENV_FILE_VAR} resolves inside {}; refused to write exported values there",
                boundary.display()
            )));
        }
    }

    // The read-only question. Nothing below this point runs unless direnv
    // itself says the user already trusts this file.
    let Some(status) = direnv_status(program, dir)? else {
        return Ok(Outcome::silent().with_detail("direnv is not installed; nothing exported"));
    };
    if status.allowed != Some(ALLOWED) {
        return Ok(Outcome::silent().with_detail(format!(
            "direnv does not report {} as allowed (allowed: {}); refused to export, \
             and never granted the trust itself",
            status.rc_display(changed),
            status
                .allowed
                .map_or_else(|| "unknown".to_string(), |code| code.to_string()),
        )));
    }

    let Some(script) = direnv_export(program, dir)? else {
        return Ok(Outcome::silent().with_detail("direnv is not installed; nothing exported"));
    };
    let Some(script) = script else {
        return Ok(Outcome::silent().with_detail("direnv export failed; nothing appended"));
    };
    if script.trim().is_empty() {
        return Ok(Outcome::silent().with_detail("direnv exported an empty environment"));
    }

    // A missing temporary root means no lock is available. Appending anyway is
    // right: the lock guards against a rare interleave, while skipping the
    // write would drop the export entirely on every machine without a usable
    // root.
    match tmproot::resolve_root() {
        Ok(root) => tmproot::with_lock(&root, LOCK_NAME, || append_export(&target, &script))
            .context("taking the file-changed lock")?,
        Err(_) => append_export(&target, &script),
    }?;

    // Deliberately says nothing about what was exported — see the module docs.
    Ok(Outcome::silent().with_detail("appended a direnv export to the harness-supplied env file"))
}

/// The first changed path that names a direnv rc file, taking the singular
/// field the harness actually sends before the batch spelling U11 also typed.
fn first_rc_path(payload: &FileChanged) -> Option<String> {
    payload
        .file_path
        .iter()
        .chain(payload.file_paths.iter())
        .find(|path| is_rc_path(path))
        .cloned()
}

/// Whether `path`'s final component is one of [`RC_NAMES`].
fn is_rc_path(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| RC_NAMES.contains(&name))
}

/// Turn the harness's raw `CLAUDE_ENV_FILE` into a path the boundary check can
/// be trusted on, or say why it cannot be.
///
/// The parent is canonicalized while the file name is left alone: the file
/// itself need not exist yet (this may be the write that creates it), but its
/// directory must, and resolving that directory's symlinks is what stops a
/// link that points back into the worktree from walking straight past the
/// boundary check below.
fn resolve_target(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(format!(
            "the harness supplied no {ENV_FILE_VAR}; nothing exported"
        ));
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(format!(
            "{ENV_FILE_VAR} is not absolute; refused to guess where it points"
        ));
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(format!("{ENV_FILE_VAR} names no file; nothing exported"));
    };
    match parent.canonicalize() {
        Ok(parent) => Ok(parent.join(name)),
        Err(err) => Err(format!(
            "{ENV_FILE_VAR}'s directory could not be resolved ({err}); nothing exported"
        )),
    }
}

/// Whether `path` lies inside `boundary`.
///
/// A boundary that will not canonicalize counts as containing the target. That
/// is the fail-closed direction: refusing costs one skipped export, whereas
/// proceeding on a boundary nobody could resolve is how exported secrets end
/// up written inside the worktree.
fn within(path: &Path, boundary: &Path) -> bool {
    boundary
        .canonicalize()
        .map_or(true, |boundary| path.starts_with(boundary))
}

/// What `direnv status --json` said about a directory.
struct Status {
    /// `state.foundRC.allowed`, absent when direnv found no rc file or
    /// answered in a shape this does not recognize.
    allowed: Option<i64>,
    /// `state.foundRC.path`, for the refusal message.
    rc_path: Option<String>,
}

impl Status {
    /// The rc file to name in a refusal: direnv's own answer when it gave one,
    /// otherwise the file whose change triggered this.
    fn rc_display(&self, changed: &Path) -> String {
        self.rc_path
            .clone()
            .unwrap_or_else(|| changed.display().to_string())
    }
}

/// Ask direnv what it knows about `dir`, without loading anything.
///
/// `direnv status --json` locates the rc file and compares its hash against
/// the allow list. It does not source it, which is what makes it safe to run
/// against a repository nobody has vetted. `Ok(None)` means direnv is not
/// installed; a direnv that runs but fails answers `allowed: None`, which the
/// caller treats as "not allowed" like any other non-zero code.
fn direnv_status(program: &str, dir: &Path) -> Result<Option<Status>> {
    let Some(output) = run_direnv(program, dir, &["status", "--json"])? else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(Some(Status {
            allowed: None,
            rc_path: None,
        }));
    }
    Ok(Some(parse_status(&String::from_utf8_lossy(&output.stdout))))
}

/// Pull `state.foundRC` out of `direnv status --json`.
///
/// Indexing a `serde_json::Value` yields `Null` for anything missing rather
/// than panicking, so a reshaped or truncated answer degrades to
/// `allowed: None` — refused — instead of failing the invocation.
fn parse_status(stdout: &str) -> Status {
    let root: serde_json::Value = serde_json::from_str(stdout).unwrap_or(serde_json::Value::Null);
    let found = &root["state"]["foundRC"];
    Status {
        allowed: found["allowed"].as_i64(),
        rc_path: found["path"].as_str().map(str::to_string),
    }
}

/// Run `direnv export bash`, which *does* evaluate the rc file — reached only
/// after [`direnv_status`] confirmed the user already allowed it.
///
/// The shell code to append comes back on stdout; direnv's own commentary
/// ("direnv: loading …") goes to stderr and is dropped. The outer `None` is
/// direnv missing, the inner `None` is direnv failing.
fn direnv_export(program: &str, dir: &Path) -> Result<Option<Option<String>>> {
    let Some(output) = run_direnv(program, dir, &["export", "bash"])? else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(Some(None));
    }
    Ok(Some(Some(
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )))
}

/// Spawn `program` in `dir`. `Ok(None)` distinguishes "not installed" — a
/// silent no-op by R92 — from a real spawn failure worth reporting.
fn run_direnv(program: &str, dir: &Path, args: &[&str]) -> Result<Option<std::process::Output>> {
    match Command::new(program).args(args).current_dir(dir).output() {
        Ok(output) => Ok(Some(output)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("running {program} {}", args.join(" "))),
    }
}

/// Append `script` to `target`, creating it owner-only if it is not there yet.
///
/// Append mode throughout: this file is a script the harness runs before each
/// Bash command, it may already hold another hook's exports, and truncating it
/// would silently drop them. The block is written with a leading newline so it
/// can never glue itself onto an unterminated last line left by someone else.
fn append_export(target: &Path, script: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(FILE_MODE)
        .open(target)
        .with_context(|| format!("opening {}", target.display()))?;
    let block = format!("\n{}\n", script.trim_end());
    file.write_all(block.as_bytes())
        .with_context(|| format!("appending to {}", target.display()))
}

#[cfg(test)]
mod tests;
