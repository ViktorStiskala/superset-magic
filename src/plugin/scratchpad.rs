//! The per-worktree scratchpad state tree at `.superset/.magic/` (U8).
//!
//! This is where the plugin keeps everything it learns about one worktree:
//! the session's own working memory (`sessions/<repo>-<branch>/*.md`), the
//! pointer naming which session is active (`current.json`), and the
//! directories later units fill with one-shot claims. The layout is fixed by
//! `docs/plans/2026-08-29-001-ss-magic-plugin/scratchpad-contract.md`.
//!
//! ## Three rules the bootstrap will not bend
//!
//! - **Scaffold, never rewrite (R17).** A state file that already exists is
//!   the model's own content — it is left byte-for-byte alone. Only genuinely
//!   missing files are created, and only `current.json` (a pointer, not
//!   content) is rewritten on every run.
//! - **Never adopt a tracked path (R17).** The slug is `<repo>-<branch>`, so a
//!   public repository can commit `sessions/<repo>-<branch>/STATUS.md` at a
//!   path this bootstrap can predict. Because the Read gate exempts the state
//!   tree, such a file would otherwise be handed to the model on the first
//!   session in a fresh clone as if it were the agent's own prior notes. Any
//!   path under the tree that git positively reports as TRACKED is therefore
//!   left untouched and named in the report; scaffolding continues for its
//!   untracked siblings. Trackedness comes from [`git::tracked_files`], never
//!   from absence in an untracked listing, so an unenumerable name cannot
//!   answer "not tracked" by accident.
//! - **Write nothing until git says the tree is ignored (R63).** The old
//!   design dropped a nested `.gitignore` containing `*` into the tree, which
//!   made it ignored the instant it existed. That file is gone: the single
//!   `.superset/.magic/` rule is now written by [`ensure_state_ignored`] from
//!   `init`/`migrate` (eagerly) and from `plugin enable` (lazily), and NEVER
//!   by a hook (R40). What replaces the old atomicity is this check — if git
//!   does not report the tree ignored, nothing at all is written and the
//!   reason is reported for the heartbeat row.
//!
//! ## Containment (R56)
//!
//! Every directory and file is checked before it is written: an existing entry
//! that is a symlink must resolve back inside the worktree, or the write is
//! refused. A `.superset/.magic` planted as a symlink to somewhere else is the
//! case this exists for — following it would scatter private state outside the
//! repository the user thinks it lives in.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::git;
use crate::git::gitignore::{self, PathKind};
use crate::plugin::atomic;
use crate::plugin::identity::{self, Identity};
use crate::tui::style;

/// The state tree, relative to the worktree root. Two components exactly — the
/// sync/pack exclusion (`crate::sync::EXCLUDED_TREES`) matches this same pair,
/// and neither may ever widen to a bare `.superset`, which holds the committed
/// contract files.
pub const STATE_REL: &str = ".superset/.magic";

/// The same path as git wants it for a directory-only ignore query: a
/// `.superset/.magic/` rule matches the query `.superset/.magic/` even before
/// the directory exists on disk, whereas the slash-less form is treated as a
/// file and misses it (see [`git::is_ignored_str`]).
const STATE_QUERY: &str = ".superset/.magic/";

/// The `.superset` parent, checked for containment on its own so a symlinked
/// `.superset` is caught before anything is created beneath it.
const SUPERSET_REL: &str = ".superset";

/// Session state files, scaffolded empty-but-headed when absent and never
/// rewritten. A floor, not a schema: the model adds whatever else it needs
/// (`research-<topic>/`, `REPORT.md`, …) and ss-magic never prunes.
///
/// `OPERATOR-CHECKLIST.md` here is the model's OWN running notes on
/// operational steps. It is emphatically not the operator checklist of R82,
/// which is committed repository content under `docs/actions/` and is written
/// only through the `ss-magic plugin checklist` verbs.
pub const STATE_FILES: [&str; 6] = [
    "CONTEXT.md",
    "DECISIONS.md",
    "LEARNINGS.md",
    "OPERATOR-CHECKLIST.md",
    "STATUS.md",
    "TASKS.md",
];

/// Explains the tree to anyone who stumbles on it in a working copy.
const README_NAME: &str = "README.md";

/// The active-session pointer. A plain JSON file, deliberately not a symlink
/// (R16): ss-magic creates no symlinks, forward sync skips them and pack only
/// classifies them no-follow, so a symlinked registry would simply vanish from
/// every copy and every archive.
const POINTER_NAME: &str = "current.json";

/// Lock file guarding the pointer write (R48, KTD5). It sits beside the file
/// it protects rather than in the machine-level temporary root of R80: that
/// root exists for races that must be settled BEFORE the state tree is usable
/// (the bootstrap lock, the machine-global pinned binary), whereas this claim
/// is only ever taken after R63's ignore check has already passed, so the tree
/// is guaranteed to exist and to be invisible to git.
const POINTER_LOCK_NAME: &str = "current.lock";

/// Directory holding one subdirectory per session slug.
const SESSIONS_DIR: &str = "sessions";

/// State-root subdirectories later units fill with one-shot entries: cached
/// Explore conclusions, per-file Read-gate bypass tokens, and pending
/// subagent-output declarations. Created here so no hook verb ever has to
/// bootstrap a directory of its own mid-flight.
const CLAIM_DIRS: [&str; 3] = ["conclusions", "bypass", "expect-artifact"];

/// Owner-only directory mode (R58). Defence in depth rather than the control —
/// what actually keeps the tree out of a sync is the enumeration exclusion in
/// `crate::sync::EXCLUDED_TREES`, since `sync/apply.rs::copy_dir_recursive`
/// creates destinations with default permissions.
const DIR_MODE: u32 = 0o700;

/// Owner-only file mode, for the same reason.
const FILE_MODE: u32 = 0o600;

/// Usage for the `scratchpad` verb family.
const SCRATCHPAD_USAGE: &str = "\
Usage: ss-magic plugin scratchpad <SUBVERB>

  ensure                Create any missing part of .superset/.magic/ for this
                        worktree, leaving existing state files untouched";

// ── The pointer file ──────────────────────────────────────────────────────────

/// `current.json`: which session directory is active, and how its name was
/// derived. `dir` is worktree-relative so the file stays meaningful after the
/// worktree is moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer {
    /// `<repo>-<branch>` — the session directory's name.
    pub slug: String,
    /// The session directory, relative to the worktree root.
    pub dir: String,
    /// The repo half of the slug.
    pub repo: String,
    /// The branch half of the slug, already slugified.
    pub branch: String,
    /// When this pointer was last resolved, as UTC RFC 3339.
    pub resolved_at: String,
}

// ── What the bootstrap refused, and why ───────────────────────────────────────

/// A reason the bootstrap declined to write something. Every variant is
/// destined for a heartbeat row, so each carries enough text to act on
/// without re-running anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// git does not report the state tree ignored, or could not be asked
    /// (R63). Fail-closed in both directions: an unanswerable question counts
    /// as "not ignored", because the whole point is that private state must
    /// never land somewhere git can see it.
    NotIgnored {
        /// Human-readable detail, naming the missing rule.
        detail: String,
    },
    /// A path in the tree is a symlink that resolves outside the worktree, or
    /// cannot be resolved at all (R56).
    Escapes {
        /// The offending path, worktree-relative.
        path: String,
        /// Where it pointed, or why it could not be resolved.
        detail: String,
    },
    /// The state root exists but is not a directory (a stray regular file, or
    /// a symlink to one).
    NotADirectory {
        /// The offending path, worktree-relative.
        path: String,
    },
    /// git could not be asked which paths under the tree are tracked. Treated
    /// as a hard stop rather than assuming "none are": adopting a committed
    /// file as the agent's own memory is exactly what R17 exists to prevent.
    TrackedProbeFailed {
        /// Why the probe failed.
        detail: String,
    },
    /// Paths under the tree that git positively reports as tracked (R17).
    /// Scaffolding continues around them; these specific paths are neither
    /// read as state nor rewritten.
    TrackedPaths {
        /// The offending paths, worktree-relative.
        paths: Vec<String>,
    },
}

impl Refusal {
    /// A stable, machine-readable code for the heartbeat row's `reason` field,
    /// so a later change to the human text does not break anything reading the
    /// heartbeat.
    // Consumed by U11's heartbeat writer; asserted by the tests today.
    #[allow(dead_code)]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotIgnored { .. } => "not-ignored",
            Self::Escapes { .. } => "escapes-worktree",
            Self::NotADirectory { .. } => "not-a-directory",
            Self::TrackedProbeFailed { .. } => "tracked-probe-failed",
            Self::TrackedPaths { .. } => "tracked-paths",
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotIgnored { detail } => write!(f, "{detail}"),
            Self::Escapes { path, detail } => {
                write!(f, "{path} leaves the worktree ({detail})")
            }
            Self::NotADirectory { path } => {
                write!(f, "{path} exists but is not a directory")
            }
            Self::TrackedProbeFailed { detail } => {
                write!(f, "could not determine which paths are tracked: {detail}")
            }
            Self::TrackedPaths { paths } => {
                write!(f, "refused to adopt tracked path(s): {}", paths.join(", "))
            }
        }
    }
}

/// What one [`ensure`] run did.
///
/// This is the seam the heartbeat consumes: a hook verb calls [`ensure`] and
/// writes [`Report::heartbeat_note`] (plus the individual [`Refusal::code`]s)
/// into its own row. Nothing here writes a heartbeat itself, so the bootstrap
/// stays usable from the human verb, where there is no row to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The state root, absolute.
    pub state_root: PathBuf,
    /// The resolved session slug.
    pub slug: String,
    /// The session directory, absolute — reported even when the run refused,
    /// so a diagnostic can name the path that was not created.
    pub session_dir: PathBuf,
    /// Worktree-relative paths this run created, in creation order.
    pub created: Vec<String>,
    /// Everything the run declined to do.
    pub refusals: Vec<Refusal>,
    /// Whether the state tree is safe for a caller to write into.
    ///
    /// Read this before writing, not as a record of what happened. `false`
    /// means some path under the state root escaped the worktree, a regular
    /// file sits where the state root belongs, git does not ignore the tree (or
    /// could not be asked), or the tracked-file probe failed - and a caller
    /// that writes anyway does so through whatever that path actually points
    /// at, which is the escape R56 exists to refuse. Every caller branches on
    /// it for exactly that reason.
    ///
    /// A [`Refusal::TrackedPaths`] is deliberately NOT one of those cases and
    /// leaves this `true`: the tracked files are simply left alone while their
    /// untracked siblings are scaffolded as usual, so the tree stays safe to
    /// write into. (A [`Refusal::TrackedProbeFailed`] is the opposite - an
    /// unanswered question about what is tracked is not permission.)
    ///
    /// It is deliberately not "nothing was written": the two late refusal sites
    /// (the pointer, and `scaffold`) can fire after some directories already
    /// exist, and `created` is the accurate record of that. `false` with a
    /// non-empty `created` is a real and expected combination.
    pub wrote_state: bool,
}

impl Report {
    /// One line summarizing the run, for the heartbeat row and for the human
    /// verb's own output.
    // Consumed by U11's heartbeat writer; asserted by the tests today.
    pub fn heartbeat_note(&self) -> String {
        if self.refusals.is_empty() {
            return format!("scratchpad ready ({} created)", self.created.len());
        }
        let reasons: Vec<String> = self.refusals.iter().map(ToString::to_string).collect();
        // A late refusal - the pointer, or `scaffold` - can fire after some
        // directories already exist, so this wording can read "refused" for a
        // run that created something. `created` is the accurate record of what
        // happened; this line is a summary, and the flag it follows is the one
        // callers act on.
        let prefix = if self.wrote_state {
            "scratchpad partial"
        } else {
            "scratchpad refused"
        };
        format!("{prefix}: {}", reasons.join("; "))
    }
}

// ── The gitignore rule (R40) ──────────────────────────────────────────────────

/// Ensure git ignores `.superset/.magic/` under `root`, adding a `Dir` rule
/// only when it does not already. Idempotent, and lands in the closest
/// EXISTING `.gitignore` among the path's ancestors — the repository root
/// ordinarily, but `.superset/.gitignore` in a repo that carries one.
///
/// The ONE place this rule is written. Two callers pair up exactly as
/// `reverse_sync::ensure_backups_ignored` and its callers do: `init`/`migrate`
/// call it eagerly through `migrate::ensure_bootstrap_gitignores`, and
/// `plugin enable` / `config set plugin.enabled true` call it lazily for a
/// repository that was initialized before the plugin existed. No hook verb
/// ever calls it (R40) — a hook that could edit `.gitignore` would be a hook
/// that can dirty the user's working tree behind their back.
pub fn ensure_state_ignored(root: &Path) -> Result<()> {
    gitignore::ensure_path_ignored(root, root, Path::new(STATE_REL), PathKind::Dir)?;
    Ok(())
}

// ── The bootstrap ─────────────────────────────────────────────────────────────

/// Shared state for one [`ensure`] run: where the worktree is, its resolved
/// physical path (for containment comparisons), and which paths under the
/// tree git reports as tracked.
struct Ctx {
    root: PathBuf,
    root_canon: PathBuf,
    tracked: HashSet<String>,
}

impl Ctx {
    /// `path` expressed relative to the worktree root, for reporting and for
    /// comparison against the tracked set (which git reports in exactly this
    /// form).
    fn rel_of(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    /// Confirm writing to `path` stays inside the worktree (R56).
    ///
    /// An absent path is fine — it will be created under a parent that has
    /// already been checked. An existing non-symlink is fine for the same
    /// reason. An existing symlink is followed exactly once, to compare the
    /// physical target against the worktree's own physical path; a target
    /// outside it, or one that cannot be resolved at all (a dangling link),
    /// is refused rather than written through.
    fn verify_contained(&self, path: &Path) -> Result<(), Refusal> {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return Ok(());
        };
        if !meta.is_symlink() {
            return Ok(());
        }
        match path.canonicalize() {
            Ok(target) if target.starts_with(&self.root_canon) => Ok(()),
            Ok(target) => Err(Refusal::Escapes {
                path: self.rel_of(path),
                detail: format!("symlink to {}", target.display()),
            }),
            Err(e) => Err(Refusal::Escapes {
                path: self.rel_of(path),
                detail: format!("unresolvable symlink: {e}"),
            }),
        }
    }

    /// True when git positively reports `path` as tracked, in which case the
    /// bootstrap leaves it entirely alone (R17).
    fn is_tracked(&self, path: &Path) -> bool {
        self.tracked.contains(&self.rel_of(path))
    }
}

/// Bootstrap the scratchpad state tree for the worktree containing `cwd`.
///
/// `cwd` must be the directory the caller actually cares about. For a hook
/// that is the `cwd` field of the stdin envelope, which follows Claude into a
/// worktree — never `${CLAUDE_PROJECT_DIR}`, which stays put, and never the
/// process's own working directory.
///
/// Returns `Err` only when there is no worktree or no session identity to work
/// with at all; everything else — including a flat refusal to write — comes
/// back as an `Ok(Report)` the caller turns into a heartbeat row.
pub fn ensure(cwd: &Path) -> Result<Report> {
    let root = git::cwd_repo_root(cwd)
        .context("`ss-magic plugin scratchpad` must run inside a git repository")?;
    let identity = identity::resolve(cwd).context(
        "could not resolve a <repo>-<branch> session identity for this checkout",
    )?;

    let state_root = root.join(STATE_REL);
    let session_dir = state_root.join(SESSIONS_DIR).join(&identity.slug);
    let mut report = Report {
        state_root: state_root.clone(),
        slug: identity.slug.clone(),
        session_dir: session_dir.clone(),
        created: Vec::new(),
        refusals: Vec::new(),
        wrote_state: false,
    };

    let root_canon = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    let mut ctx = Ctx {
        root: root.clone(),
        root_canon,
        tracked: HashSet::new(),
    };

    // R56 — the two ancestors, checked before anything is created beneath
    // them. `.superset` is checked separately because a symlinked `.superset`
    // would relocate the whole tree just as effectively as a symlinked
    // `.magic`. This runs before the ignore probe below purely so the reported
    // reason is the accurate one: git refuses to resolve a pathspec that
    // crosses a symlink, so an escaping state root would otherwise surface as
    // a confusing "could not ask git whether it is ignored". Neither check
    // writes anything, so the order is free.
    for rel in [SUPERSET_REL, STATE_REL] {
        if let Err(refusal) = ctx.verify_contained(&root.join(rel)) {
            report.refusals.push(refusal);
            return Ok(report);
        }
    }

    // A stray regular file where the tree belongs. Reported rather than
    // deleted: it is not ours, and whatever put it there may want it back.
    if state_root.exists()
        && !fs::metadata(&state_root)
            .with_context(|| format!("reading {}", state_root.display()))?
            .is_dir()
    {
        report.refusals.push(Refusal::NotADirectory {
            path: ctx.rel_of(&state_root),
        });
        return Ok(report);
    }

    // R63 — the fail-closed gate. Both "git says no" and "git could not be
    // asked" land here, because an unanswered question is not permission. The
    // rules-only probe is deliberate: the default index-aware `check-ignore`
    // calls a directory unignored the moment anything under it is tracked,
    // which would turn R17's tracked-file case (handled below, by leaving that
    // file alone) into a blanket refusal to bootstrap at all.
    match git::is_ignored_no_index_str(&root, STATE_QUERY) {
        Ok(true) => {}
        Ok(false) => {
            report.refusals.push(Refusal::NotIgnored {
                detail: format!(
                    "git does not ignore {STATE_QUERY} — no covering rule in any .gitignore; \
                     run `ss-magic plugin enable` (or `ss-magic init`) to add it"
                ),
            });
            return Ok(report);
        }
        Err(e) => {
            report.refusals.push(Refusal::NotIgnored {
                detail: format!("could not ask git whether {STATE_QUERY} is ignored: {e}"),
            });
            return Ok(report);
        }
    }

    // R17 — POSITIVE tracked determination. `git ls-files --cached` over the
    // tree's pathspec; a failure stops the run rather than being read as "no
    // tracked paths".
    match git::tracked_files(&root, &[STATE_REL]) {
        Ok(paths) => {
            let tracked: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if !tracked.is_empty() {
                report.refusals.push(Refusal::TrackedPaths {
                    paths: tracked.clone(),
                });
            }
            ctx.tracked = tracked.into_iter().collect();
        }
        Err(e) => {
            report.refusals.push(Refusal::TrackedProbeFailed {
                detail: format!("{e:#}"),
            });
            return Ok(report);
        }
    }

    report.wrote_state = true;

    // `.superset` itself is committed content, so it keeps default
    // permissions; only the tree below it is owner-only (R58).
    let superset_dir = root.join(SUPERSET_REL);
    if !superset_dir.exists() {
        fs::create_dir_all(&superset_dir)
            .with_context(|| format!("creating {}", superset_dir.display()))?;
    }

    let mut dirs = vec![
        state_root.clone(),
        state_root.join(SESSIONS_DIR),
        session_dir.clone(),
    ];
    dirs.extend(CLAIM_DIRS.iter().map(|d| state_root.join(d)));
    for dir in &dirs {
        if let Err(refusal) = ctx.verify_contained(dir) {
            // Clear the flag rather than leaving it set from above: callers
            // read `wrote_state` as "the tree is safe to write into", and a
            // containment refusal means precisely the opposite.
            report.wrote_state = false;
            report.refusals.push(refusal);
            return Ok(report);
        }
        if dir.exists() {
            continue;
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        report.created.push(ctx.rel_of(dir));
    }

    scaffold(&ctx, &state_root.join(README_NAME), README_BODY, &mut report)?;
    for name in STATE_FILES {
        scaffold(&ctx, &session_dir.join(name), state_file_body(name), &mut report)?;
    }

    // The pointer is the one file rewritten every run — it records which
    // session is current, not content the model owns.
    let pointer = state_root.join(POINTER_NAME);
    if !ctx.is_tracked(&pointer) {
        if let Err(refusal) = ctx.verify_contained(&pointer) {
            report.wrote_state = false;
            report.refusals.push(refusal);
            return Ok(report);
        }
        let existed = pointer.exists();
        write_pointer(&state_root, &pointer, &identity, &ctx.rel_of(&session_dir))?;
        if !existed {
            report.created.push(ctx.rel_of(&pointer));
        }
    }

    Ok(report)
}

/// Create `path` with `body` when it is absent, leaving an existing file
/// completely alone (R17) and skipping anything git reports as tracked.
///
/// `create_new` does the "only if absent" test and the create in one syscall,
/// so a second `ensure` racing this one cannot slip between the check and the
/// write and lose the winner's content; an `AlreadyExists` from that race is
/// the expected outcome, not an error.
fn scaffold(ctx: &Ctx, path: &Path, body: &str, report: &mut Report) -> Result<()> {
    if ctx.is_tracked(path) {
        return Ok(());
    }
    if let Err(refusal) = ctx.verify_contained(path) {
        // One escaping path is enough to distrust the whole tree: `ensure`
        // carries on scaffolding the siblings, but the caller must not treat
        // what it produced as safe to write into.
        report.wrote_state = false;
        report.refusals.push(refusal);
        return Ok(());
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(body.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            report.created.push(ctx.rel_of(path));
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e).with_context(|| format!("creating {}", path.display())),
    }
}

/// Write `current.json` under an exclusive claim (R48, KTD5).
///
/// Two mechanisms, each covering what the other cannot. The `fd-lock`
/// advisory write lock — the same primitive the self-updater uses, never a
/// second locking scheme — serializes concurrent ss-magic processes; it is
/// taken blocking, because unlike the update lock there is no work to skip,
/// only a short write to wait for. The temp-file-then-rename makes the
/// replacement atomic for READERS, which hold no lock at all: a reader either
/// sees the whole previous pointer or the whole new one, never a half-written
/// file.
fn write_pointer(
    state_root: &Path,
    path: &Path,
    identity: &Identity,
    session_rel: &str,
) -> Result<()> {
    let lock_path = state_root.join(POINTER_LOCK_NAME);
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    let mut lock = fd_lock::RwLock::new(lock_file);
    let _claim = lock
        .write()
        .with_context(|| format!("locking {}", lock_path.display()))?;

    let pointer = Pointer {
        slug: identity.slug.clone(),
        dir: session_rel.to_string(),
        repo: identity.repo.clone(),
        branch: identity.branch.clone(),
        resolved_at: now_rfc3339(),
    };
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(&pointer).context("serializing the session pointer")?
    );

    atomic::write_atomically(
        path,
        &body,
        ".current-",
        "",
        Some("the session pointer"),
        None,
        false,
    )?;
    Ok(())
}

/// Seconds since the Unix epoch, or 0 if the clock is before it.
///
/// Shared crate-wide: bypass claims, the conclusion cache, the cost ledger,
/// artifact expectations, checklist documents, and the hook dispatcher each
/// used to define this same function locally — one copy here instead of six
/// byte-identical ones.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The current UTC time as RFC 3339, to the second.
fn now_rfc3339() -> String {
    format_rfc3339(now_secs())
}

/// Format `secs` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Pure and dependency-free, using Howard Hinnant's civil-from-days algorithm
/// — the same technique as `sync::reverse_sync`'s backup-directory timestamp,
/// which is NOT shared with it because the two produce different formats for
/// different audiences (a directory name there, a machine-readable instant
/// here). `plugin::heartbeat` wants exactly this format for the same audience,
/// so it calls this rather than growing a third copy of the date arithmetic.
pub(crate) fn format_rfc3339(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Shift the epoch to 0000-03-01 so leap days land at the end of the cycle.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);

    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

// ── Scaffolded bodies ─────────────────────────────────────────────────────────

/// The state root's README. Explains what the tree is to whoever finds it, and
/// says plainly that ss-magic will not touch what the model writes here.
const README_BODY: &str = "\
# `.superset/.magic/` — ss-magic plugin state

Per-worktree working memory for the Claude Code session running in this
worktree. Gitignored, machine-local, and deleted along with the worktree.

- `current.json` — which session directory is active, and how its name was
  derived. Rewritten by `ss-magic plugin scratchpad ensure`.
- `sessions/<repo>-<branch>/` — the session's own notes. Created empty; the
  content belongs to the model and ss-magic never rewrites it.
- `conclusions/`, `bypass/`, `expect-artifact/` — short-lived entries the
  plugin's hooks use to avoid re-reading what has already been read.

Nothing here is copied by `ss-magic sync` or captured by `ss-magic pack`.
";

/// The starter body for one session state file: a heading plus a sentence on
/// how the file is meant to be kept. Deliberately short — these are the
/// model's files, and a long template would just be something to delete.
fn state_file_body(name: &str) -> &'static str {
    match name {
        "CONTEXT.md" => "\
# Context

Context that would be expensive to rediscover. Grouped by topic, not by time.
",
        "DECISIONS.md" => "\
# Decisions

Settled decisions, each with the reasoning and the evidence behind it.
",
        "LEARNINGS.md" => "\
# Learnings

Append-only. Add a `## <timestamp> - <label>` block; never edit an older one.
",
        "OPERATOR-CHECKLIST.md" => "\
# Operator checklist (working notes)

The model's own running notes on operational steps. Not the committed
operator checklist under `docs/actions/`, which is edited only through the
`ss-magic plugin checklist` verbs.
",
        "STATUS.md" => "\
# Status

Newest block first. Demote an old block to history rather than deleting it:

## CURRENT STATE - <timestamp> (read this block first; everything below is history)

## HISTORY - <timestamp> block (superseded, kept for the audit trail)
",
        "TASKS.md" => "\
# Tasks

The task list and where each item currently stands.
",
        // Unreachable for `STATE_FILES`; a future addition falls back to a
        // bare title rather than silently getting an unrelated body.
        _ => "",
    }
}

// ── The `scratchpad` verb (R9, R35) ───────────────────────────────────────────

/// `ss-magic plugin scratchpad <SUBVERB>` — the loud, argv-driven entry point.
/// A human verb, so problems go to stderr with a non-zero exit; nothing here
/// is reachable from a hook.
pub fn run(args: &[String]) -> Result<ExitCode> {
    match args.first().map(String::as_str) {
        Some("ensure") => {
            let cwd = std::env::current_dir().context("reading the current directory")?;
            run_ensure(&cwd)
        }
        Some(other) => {
            eprintln!(
                "{}",
                style::err(format!("error: unknown `scratchpad` subverb `{other}`"))
            );
            eprintln!("{SCRATCHPAD_USAGE}");
            Ok(ExitCode::from(2))
        }
        None => {
            eprintln!(
                "{}",
                style::err("error: `ss-magic plugin scratchpad` needs a subverb")
            );
            eprintln!("{SCRATCHPAD_USAGE}");
            Ok(ExitCode::from(2))
        }
    }
}

/// `scratchpad ensure` against an explicit directory, so the exit-code mapping
/// is testable without moving the process's own working directory.
///
/// Three outcomes, three codes: 0 when the tree is in shape, 1 when a refusal
/// stopped it (a real condition the caller may want to act on, not a crash),
/// and 2 when there was nothing to work with at all — no repository, or no
/// resolvable session identity.
fn run_ensure(cwd: &Path) -> Result<ExitCode> {
    match ensure(cwd) {
        Ok(report) => {
            for refusal in &report.refusals {
                eprintln!("{}", style::warn(format!("refused: {refusal}")));
            }
            if report.wrote_state {
                println!(
                    "{}",
                    style::ok(format!("Scratchpad ready: {}", report.session_dir.display()))
                );
                for rel in &report.created {
                    println!("{}", style::info(format!("  created {rel}")));
                }
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
        Err(e) => {
            eprintln!("{}", style::err(format!("error: {e:#}")));
            Ok(ExitCode::from(2))
        }
    }
}

#[cfg(test)]
mod tests;
