//! `ss-magic plugin checklist …` — the verbs, and the pointer `init` records.
//!
//! The checklist is committed repository content at
//! `docs/actions/<YYYY-MM-slug>.checklist.json`, and the `PreToolUse` gate
//! denies a direct `Read`, `Edit` or `Write` of it. That makes this module the
//! only write path the model has, which is why the surface is explicit and
//! complete rather than a thin wrapper over "edit some JSON":
//!
//! ```text
//! init <slug>                       add-entry <id> [SUMMARY]
//! add-item <section> <id> [TITLE]   set <id> <dotted-key> [VALUE]
//! done <id>                         list | verify | render-md
//! ```
//!
//! A trailing text argument is optional everywhere it appears: leave it off
//! and the body is read from **stdin** instead, so a multi-line action step or
//! description survives newlines, quoting and shell metacharacters intact.
//!
//! ## Every write is a read-modify-write
//!
//! No verb builds a document out of parts. Each one reads the file as it is,
//! changes the single field it was asked to change, puts the result back into
//! canonical order, re-stamps `updated`, and writes the whole thing back. That
//! is what keeps a key this build has never heard of — written by a newer
//! ss-magic, or typed by a person — alive across a `set` of some unrelated
//! field; the schema's `extras` maps carry it, and only a rebuild-from-parts
//! would drop it.
//!
//! ## Writes are atomic, and serialized where they can be
//!
//! Two mechanisms, covering different failures, the same pairing
//! `scratchpad::write_pointer` uses:
//!
//! - **Temp file then rename.** The bytes land in a sibling temp file inside
//!   the same directory and are renamed over the target. A reader — a person,
//!   git, a CI job — sees either the whole previous document or the whole new
//!   one, and a crash mid-write leaves the previous document untouched. This
//!   alone is what guarantees a concurrent double-write leaves ONE valid
//!   document rather than an interleaved one.
//! - **An advisory lock**, taken on `.superset/.magic/checklist.lock` through
//!   [`tmproot::with_lock`] — the crate's one locking scheme, never a second
//!   one. It additionally stops the second of two concurrent writers from
//!   basing its edit on a document the first has already replaced (a lost
//!   update). It is best-effort in exactly one direction: where the state tree
//!   does not exist, the write still happens, unlocked but still atomic,
//!   because a checklist is repository content and must not become
//!   unmanageable just because the plugin was never bootstrapped here.
//!
//! ## Which checklist is "the" checklist
//!
//! `init` records the answer in `.superset/.magic/checklist.json` — a plain
//! JSON manifest, not a symlink, for the same reason `current.json` is one:
//! ss-magic creates no symlinks anywhere, forward sync skips them and pack
//! only classifies them no-follow, so a symlinked pointer would simply vanish
//! from every copy and every archive. The pointer records the intended path
//! **whether or not that file exists yet**, so `init` on a branch with no
//! checklist is a complete operation rather than a half-state.
//!
//! Nothing is ever written into `.scratchpad/`. That tree belongs to other
//! tooling and ss-magic does not own it.
//!
//! Because the pointer lives in the gitignored state tree, a fresh clone or a
//! newly-created worktree has a committed checklist and no pointer at all.
//! [`resolve_active`] therefore falls back to the `docs/actions/*.checklist.json`
//! naming convention when exactly one document matches, so `verify` and `list`
//! keep working there without anyone having to re-run `init`.

use std::fs;
use std::io::{IsTerminal as _, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::render as render_markdown;
use super::{
    canonicalize, has_errors, is_well_formed_id, read_document, to_json, validate, ChangelogEntry,
    Document, Finding, Item, ItemKind, Priority, Reference, Severity, Timestamp,
};
use crate::git;
use crate::plugin::cache::Budget;
use crate::plugin::scratchpad::{self, format_rfc3339, STATE_REL};
use crate::plugin::tmproot;
use crate::tui::style;

// ── Where things live ─────────────────────────────────────────────────────────

/// The directory checklists are committed under, relative to the repository
/// root. Half of the naming convention [`resolve_active`] falls back to, and
/// the half U28's Read/Edit deny recognises without needing a pointer.
pub const ACTIONS_REL: &str = "docs/actions";

/// The suffix every checklist file carries, after its `<YYYY-MM-slug>` stem.
pub const CHECKLIST_SUFFIX: &str = ".checklist.json";

/// The pointer's name inside the state root. `SessionStart` names this exact
/// path in the guidance it injects, so it is not free to move.
pub const POINTER_NAME: &str = "checklist.json";

/// The lock file guarding both the pointer and the document, beside the file
/// it protects. One lock covers both because `init` touches both.
const LOCK_NAME: &str = "checklist.lock";

/// Mode a freshly-created checklist gets. Committed repository content, so
/// deliberately world-readable — unlike everything under the state tree, which
/// is owner-only. A rewrite of an existing file keeps whatever mode that file
/// already had instead of imposing this one.
const NEW_FILE_MODE: u32 = 0o644;

/// How much of a rendered checklist `list` puts in front of a reader before
/// the envelope truncates it and names the file instead.
///
/// `render-md` is deliberately unbounded: its output is a pull-request comment
/// body, which has no context window to protect. `list` is read by whoever (or
/// whatever) ran the verb, so a checklist that has grown to hundreds of items
/// is summarized rather than pasted in full — at which point the right move is
/// to open the specific section, not to read everything.
const LIST_BYTE_BUDGET: usize = 24_000;

/// The pointer file: which checklist is the active one for this worktree.
///
/// `path` is repository-relative so the pointer stays meaningful after the
/// worktree is moved, exactly as `current.json`'s `dir` is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer {
    /// The checklist, relative to the repository root. Recorded whether or not
    /// the file exists yet.
    pub path: String,
    /// The `<YYYY-MM-slug>` stem the file is named after.
    pub slug: String,
    /// When the pointer was recorded, as UTC RFC 3339.
    pub recorded_at: String,
}

/// Where the pointer for `root` lives.
pub fn pointer_path(root: &Path) -> PathBuf {
    root.join(STATE_REL).join(POINTER_NAME)
}

/// The checklist the pointer at `root` names, or `None` when there is no
/// pointer, it cannot be read or parsed, or the path it records is not one we
/// are willing to follow.
///
/// **The target is not stat'd.** A pointer to a document that does not exist
/// yet is still a pointer — `init` records the intended path before the file
/// is written, and the Read/Edit deny has to recognise that path anyway.
pub fn pointer_target(root: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(pointer_path(root)).ok()?;
    let pointer: Pointer = serde_json::from_str(&raw).ok()?;
    contained_join(root, &pointer.path)
}

/// Join a repository-relative path onto `root`, refusing anything that could
/// leave the repository.
///
/// The pointer is a file on disk, so its contents are not automatically
/// trustworthy: an absolute path or a `..` segment would turn a `set` into a
/// write anywhere on the filesystem. Checked lexically rather than by
/// resolving, because the target legitimately may not exist yet.
fn contained_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.as_os_str().is_empty() || rel.is_absolute() {
        return None;
    }
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(root.join(rel))
}

/// True when `path` follows the `docs/actions/<stem>.checklist.json` naming
/// convention inside `root`. Purely lexical: nothing is stat'd, so a file that
/// does not exist yet still matches.
pub fn matches_convention(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    if rel.parent() != Some(Path::new(ACTIONS_REL)) {
        return false;
    }
    rel.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.ends_with(CHECKLIST_SUFFIX) && name.len() > CHECKLIST_SUFFIX.len())
}

// ── Parsing ───────────────────────────────────────────────────────────────────

const USAGE: &str = "\
Usage: ss-magic plugin checklist <SUBVERB> [ARGS...]

  init <slug>                     Create docs/actions/<YYYY-MM-slug>.checklist.json
                                  and record it as the active checklist
  add-item <section> <id> [TITLE] Add an item to a section
  add-entry <id> [SUMMARY]        Add a changelog entry
  set <id> <dotted-key> [VALUE]   Set one field on an item, entry, section, or
                                  on `document` for the header itself
  done <id>                       Mark an item done, stamping the completion time
  list                            The checklist, rendered, in canonical order
  verify                          Report every violation; exits non-zero on any
  render-md                       The Markdown CI posts, unbounded

Leave the trailing text argument off and the body is read from stdin instead,
so newlines and quoting survive:

  ss-magic plugin checklist set check-dns why <<'EOF'
  The old record has a 24h TTL, so the cutover has to happen a day early.
  EOF

The literal argument `null` clears a field that may be cleared (`priority`,
`why`, `description`, `expected`, `completed`, `refs`). To store the four
letters as text, pass them on stdin.

Dotted keys address a nested field, the same convention `config set` uses:

  steps            the whole list, one step per line of the value
  steps.-          append one step
  steps.2          replace the third step (`null` removes it)
  refs.-           append a reference with this URL
  refs.0.label     re-label the first reference";

/// One parsed subverb. Text a caller may supply either as a trailing argument
/// or on stdin is `Option<String>` here — `None` means "read stdin", and
/// [`run`] is what actually reads it, so [`run_core`] stays testable without a
/// process.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sub {
    Init {
        slug: String,
    },
    AddItem {
        section: String,
        id: String,
        title: Option<String>,
    },
    AddEntry {
        id: String,
        summary: Option<String>,
    },
    Set {
        id: String,
        key: String,
        /// `None` when the caller wrote the literal `null`, or when the value
        /// is still to come from stdin — [`Sub::wants_stdin`] tells the two
        /// apart, and `run` resolves it before `run_core` sees it.
        value: Option<String>,
        from_stdin: bool,
    },
    Done {
        id: String,
    },
    List,
    Verify,
    RenderMd,
}

impl Sub {
    /// Whether this invocation still needs a body read from stdin.
    fn wants_stdin(&self) -> bool {
        match self {
            Self::AddItem { title, .. } => title.is_none(),
            Self::AddEntry { summary, .. } => summary.is_none(),
            Self::Set { from_stdin, .. } => *from_stdin,
            _ => false,
        }
    }

    /// Whether the body is the whole point of the invocation, so an empty one
    /// is a mistake rather than a field left for later.
    ///
    /// `set` is: there is nothing to set without a value. `add-item` and
    /// `add-entry` are not — a record with an empty title is a legitimate
    /// intermediate state that `set` fills in, and `verify` lists until it is
    /// filled. The distinction decides what happens when nothing is piped in:
    /// see [`run`].
    fn stdin_is_required(&self) -> bool {
        matches!(self, Self::Set { .. })
    }

    /// Fill in the body this invocation was waiting for.
    fn with_body(self, body: String) -> Self {
        match self {
            Self::AddItem { section, id, .. } => Self::AddItem {
                section,
                id,
                title: Some(body),
            },
            Self::AddEntry { id, .. } => Self::AddEntry {
                id,
                summary: Some(body),
            },
            Self::Set { id, key, .. } => Self::Set {
                id,
                key,
                value: Some(body),
                from_stdin: false,
            },
            other => other,
        }
    }
}

/// Outcome of the subverb parse. Mirrors [`crate::plugin::Parsed`]: the
/// non-`Run` variants are terminal signals [`run`] turns into a print plus an
/// exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedSub {
    Run(Sub),
    Help,
    Error(String),
}

/// Parse the argv that followed the `checklist` token. Pure — no stdin, no
/// filesystem.
fn parse(args: &[String]) -> ParsedSub {
    let Some(first) = args.first() else {
        return ParsedSub::Error("`checklist` needs a subverb".to_string());
    };
    // Help wins wherever it appears, so `checklist init --help` prints the
    // usage rather than trying to create a checklist named `--help`.
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return ParsedSub::Help;
    }

    let rest = &args[1..];
    let sub = match first.as_str() {
        "init" => match rest {
            [slug] => Sub::Init { slug: slug.clone() },
            _ => return arity("init <slug>"),
        },
        "add-item" => match rest {
            [section, id] => Sub::AddItem {
                section: section.clone(),
                id: id.clone(),
                title: None,
            },
            [section, id, title] => Sub::AddItem {
                section: section.clone(),
                id: id.clone(),
                title: Some(title.clone()),
            },
            _ => return arity("add-item <section> <id> [TITLE]"),
        },
        "add-entry" => match rest {
            [id] => Sub::AddEntry {
                id: id.clone(),
                summary: None,
            },
            [id, summary] => Sub::AddEntry {
                id: id.clone(),
                summary: Some(summary.clone()),
            },
            _ => return arity("add-entry <id> [SUMMARY]"),
        },
        "set" => match rest {
            [id, key] => Sub::Set {
                id: id.clone(),
                key: key.clone(),
                value: None,
                from_stdin: true,
            },
            [id, key, value] => Sub::Set {
                id: id.clone(),
                key: key.clone(),
                // The literal `null` is how a caller clears a field. A value
                // that has to contain those four letters as text comes from
                // stdin, which is never reinterpreted.
                value: (value != "null").then(|| value.clone()),
                from_stdin: false,
            },
            _ => return arity("set <id> <dotted-key> [VALUE]"),
        },
        "done" => match rest {
            [id] => Sub::Done { id: id.clone() },
            _ => return arity("done <id>"),
        },
        "list" if rest.is_empty() => Sub::List,
        "verify" if rest.is_empty() => Sub::Verify,
        "render-md" if rest.is_empty() => Sub::RenderMd,
        "list" | "verify" | "render-md" => {
            return ParsedSub::Error(format!("`checklist {first}` takes no arguments"))
        }
        other => return ParsedSub::Error(format!("unknown `checklist` subverb `{other}`")),
    };
    ParsedSub::Run(sub)
}

fn arity(shape: &str) -> ParsedSub {
    ParsedSub::Error(format!("usage: ss-magic plugin checklist {shape}"))
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// `ss-magic plugin checklist …` — a human verb, so problems go to stderr with
/// a non-zero exit. Nothing here is reachable from a hook.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let sub = match parse(args) {
        ParsedSub::Run(sub) => sub,
        ParsedSub::Help => {
            println!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        ParsedSub::Error(message) => return Ok(usage_error(&message)),
    };

    // A body is read from stdin only when something is actually piped in.
    // Blocking on a terminal would be worse than useless here: the two-argument
    // `add-item <section> <id>` is the form the session-start guidance names,
    // and a verb that appears to hang when typed exactly as documented is a
    // verb nobody runs twice.
    let sub = if sub.wants_stdin() {
        if std::io::stdin().is_terminal() {
            if sub.stdin_is_required() {
                return Ok(usage_error(
                    "`set` needs a value — pass it as the third argument, or pipe the body \
                     in on stdin",
                ));
            }
            sub.with_body(String::new())
        } else {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading the field body from stdin")?;
            // A heredoc always ends in a newline the author did not type;
            // interior ones are theirs and are kept.
            sub.with_body(buf.trim_end_matches('\n').to_string())
        }
    } else {
        sub
    };

    let cwd = std::env::current_dir().context("reading the current directory")?;
    run_core(&cwd, &sub, now_secs())
}

/// Every verb against an explicit directory and an explicit clock, so the whole
/// flow is testable without a process, a stdin, or a real wall clock.
fn run_core(cwd: &Path, sub: &Sub, now: u64) -> Result<ExitCode> {
    let root = match git::cwd_repo_root(cwd) {
        Ok(root) => root,
        Err(err) => {
            return Ok(fail(format!(
                "`ss-magic plugin checklist` must run inside a git repository: {err:#}"
            )))
        }
    };

    match sub {
        Sub::Init { slug } => run_init(&root, cwd, slug, now),
        Sub::List => run_render(&root, Budget::Bytes(LIST_BYTE_BUDGET), true),
        Sub::RenderMd => run_render(&root, Budget::Unbounded, false),
        Sub::Verify => run_verify(&root),
        _ => run_mutation(&root, sub, now),
    }
}

// ── init ──────────────────────────────────────────────────────────────────────

/// `checklist init <slug>`.
///
/// Two halves that both have to happen for the operation to be complete: the
/// document under `docs/actions/`, and the pointer naming it. The pointer is
/// written second but its preconditions are checked FIRST — R89 wants the
/// containment and ignored-tree checks to pass before anything is recorded, so
/// a repository where the state tree is not yet ignored is refused outright
/// rather than left with a document nothing points at.
///
/// Running it twice is not an error and never overwrites: an existing document
/// is adopted, and only the pointer is (re)written. That is also how a
/// repository with several checklists picks which one is live.
fn run_init(root: &Path, cwd: &Path, slug: &str, now: u64) -> Result<ExitCode> {
    let stem = match checklist_stem(slug, now) {
        Ok(stem) => stem,
        Err(message) => return Ok(usage_error(&message)),
    };
    let rel = format!("{ACTIONS_REL}/{stem}{CHECKLIST_SUFFIX}");
    let path = root.join(&rel);

    // The state tree has to be usable before the pointer can be recorded, and
    // `ensure` is what enforces R56's containment and R63's ignored-tree gate.
    // A hard refusal stops the whole verb: half of `init` is worse than none.
    let report = scratchpad::ensure(cwd)?;
    if !report.wrote_state {
        for refusal in &report.refusals {
            eprintln!("{}", style::err(format!("refused: {refusal}")));
        }
        eprintln!(
            "{}",
            style::info(
                "The pointer records which checklist is live and lives in that tree, so \
                 `init` writes nothing until the tree is usable."
            )
        );
        return Ok(ExitCode::from(1));
    }
    for refusal in &report.refusals {
        eprintln!("{}", style::warn(format!("refused: {refusal}")));
    }

    let pointer = Pointer {
        path: rel.clone(),
        slug: stem.clone(),
        recorded_at: format_rfc3339(now),
    };

    // One lock across the whole operation: the "does it already exist?" test
    // and the write that depends on its answer have to be one step, or two
    // concurrent `init`s of the same slug could each decide the file is absent
    // and the second would overwrite the first's document.
    let existed = with_lock(&report.state_root, || -> Result<bool> {
        let existed = path.exists();
        if !existed {
            let mut doc = Document::new(
                title_from_slug(&stem),
                &stem,
                Timestamp::from_epoch_secs(now),
            );
            write_document(&path, &mut doc)?;
        }
        write_pointer(&report.state_root, &pointer)?;
        Ok(existed)
    })??;

    if existed {
        println!(
            "{}",
            style::ok(format!("{rel} is now the active checklist"))
        );
        println!(
            "{}",
            style::info("  the document already existed and was left exactly as it was")
        );
    } else {
        println!("{}", style::ok(format!("Created {rel}")));
    }
    println!(
        "{}",
        style::info(format!(
            "  pointer {}",
            display_rel(root, &pointer_path(root))
        ))
    );
    if !existed {
        println!(
            "{}",
            style::info(
                "  next: `checklist add-item <section> <id>` — `checklist list` shows the sections"
            )
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Turn a caller-supplied slug into the `<YYYY-MM-slug>` stem the file is
/// named after.
///
/// A slug that already carries a `YYYY-MM-` prefix is taken as the whole stem,
/// so re-running `init` with the name printed by a previous run addresses the
/// same document instead of nesting a second date onto it.
fn checklist_stem(slug: &str, now: u64) -> std::result::Result<String, String> {
    let (prefix, bare) = match split_date_prefix(slug) {
        Some((prefix, bare)) => (prefix.to_string(), bare),
        None => (year_month(now), slug),
    };
    if !is_well_formed_id(bare) {
        return Err(format!(
            "`{bare}` is not a well-formed slug; slugs are kebab-case, begin with a letter, \
             and hold only lowercase letters, digits and single hyphens"
        ));
    }
    Ok(format!("{prefix}-{bare}"))
}

/// Split a leading `YYYY-MM-` off a stem, returning `(YYYY-MM, rest)`.
fn split_date_prefix(slug: &str) -> Option<(&str, &str)> {
    let bytes = slug.as_bytes();
    if bytes.len() < 8 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits_ok =
        bytes[..4].iter().all(u8::is_ascii_digit) && bytes[5..7].iter().all(u8::is_ascii_digit);
    digits_ok.then(|| (&slug[..7], &slug[8..]))
}

/// `YYYY-MM` for an epoch instant, taken off the crate's one UTC formatter so
/// the date in a file name and the dates inside the document agree.
fn year_month(now: u64) -> String {
    format_rfc3339(now)[..7].to_string()
}

/// A readable title from a stem: the date prefix dropped, hyphens turned back
/// into spaces, first letter capitalized. Only ever a starting point —
/// `checklist set document title …` is how it gets a real one.
fn title_from_slug(stem: &str) -> String {
    let bare = split_date_prefix(stem).map_or(stem, |(_, rest)| rest);
    let spaced = bare.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

// ── Reading verbs ─────────────────────────────────────────────────────────────

/// `checklist list` and `checklist render-md`, which differ only in their
/// destination: `list` is read by whoever ran it and is bounded, `render-md`
/// is the exact body a CI job posts on a pull request and is not.
fn run_render(root: &Path, budget: Budget, note_findings: bool) -> Result<ExitCode> {
    let Some((path, doc)) = load_active(root)? else {
        return Ok(refused());
    };

    let repo_url = browsable_origin(root);
    print!(
        "{}",
        render_markdown(&doc, &path, repo_url.as_deref(), budget)
    );

    // The render is honest about a broken document rather than hiding it, but
    // it is not the gate — `verify` is, and it is what CI runs.
    if note_findings {
        let findings = validate(&doc);
        if has_errors(&findings) {
            eprintln!(
                "{}",
                style::warn(format!(
                    "{} error(s) in this checklist — run `ss-magic plugin checklist verify`",
                    findings
                        .iter()
                        .filter(|f| f.severity == Severity::Error)
                        .count()
                ))
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `checklist verify`. Reports every finding and exits non-zero on any error.
///
/// It deliberately does NOT render: the point of `verify` is to say what is
/// wrong with a document, and handing a malformed one to the renderer would
/// produce output that looks authoritative while describing a file nothing
/// should be acting on yet. Warnings are printed but do not fail the run —
/// they describe shape defects the next CLI write repairs on its own, and no
/// repository's CI should go red over something no reader would notice.
fn run_verify(root: &Path) -> Result<ExitCode> {
    let Some((path, doc)) = load_active(root)? else {
        return Ok(refused());
    };

    let findings = validate(&doc);
    print_findings(&findings);

    let rel = display_rel(root, &path);
    if has_errors(&findings) {
        eprintln!("{}", style::err(format!("{rel} is not valid")));
        return Ok(ExitCode::from(1));
    }
    if findings.is_empty() {
        println!("{}", style::ok(format!("{rel} is valid")));
    } else {
        println!(
            "{}",
            style::ok(format!(
                "{rel} is valid ({} warning(s) the next write will tidy)",
                findings.len()
            ))
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn print_findings(findings: &[Finding]) {
    for finding in findings {
        let line = format!("{} — {}", finding.location, finding.message);
        match finding.severity {
            Severity::Error => eprintln!("{}", style::err(format!("error: {line}"))),
            Severity::Warning => eprintln!("{}", style::warn(format!("warning: {line}"))),
        }
    }
}

// ── Writing verbs ─────────────────────────────────────────────────────────────

/// `add-item`, `add-entry`, `set` and `done`, which share the whole
/// read-modify-write shape and differ only in the mutation in the middle.
fn run_mutation(root: &Path, sub: &Sub, now: u64) -> Result<ExitCode> {
    // The lock spans the read, the edit and the write together. Holding it for
    // the write alone would still produce a whole file, but two concurrent
    // `set`s would each have read the document before the other wrote it, and
    // the loser's field would silently vanish.
    let state_root = root.join(STATE_REL);
    with_lock(&state_root, || mutate_locked(root, sub, now))?
}

/// The body of [`run_mutation`], run while the write lock is held.
fn mutate_locked(root: &Path, sub: &Sub, now: u64) -> Result<ExitCode> {
    let Some((path, mut doc)) = load_active(root)? else {
        return Ok(refused());
    };

    let outcome = match sub {
        Sub::AddItem { section, id, title } => {
            add_item(&mut doc, section, id, title.as_deref().unwrap_or(""), now)
        }
        Sub::AddEntry { id, summary } => {
            add_entry(&mut doc, id, summary.as_deref().unwrap_or(""), now)
        }
        Sub::Set { id, key, value, .. } => apply_set(&mut doc, id, key, value.as_deref(), now),
        Sub::Done { id } => mark_done(&mut doc, id, now),
        // The reading verbs never reach here — `run_core` routes them first.
        _ => unreachable!("run_mutation only handles the writing verbs"),
    };
    let note = match outcome {
        Ok(note) => note,
        Err(err) => return Ok(fail(format!("{err:#}"))),
    };

    doc.updated = Timestamp::from_epoch_secs(now);
    write_document(&path, &mut doc)?;

    println!("{}", style::ok(note));
    println!("{}", style::info(format!("  {}", display_rel(root, &path))));

    // Surfaced, not enforced. A freshly added item legitimately has no steps
    // and no expectation yet, so refusing to persist it would make the verb
    // useless; naming what is still missing is what turns `verify` into a
    // to-do list rather than a surprise at commit time.
    let findings = validate(&doc);
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    if errors > 0 {
        println!(
            "{}",
            style::warn(format!(
                "  {errors} thing(s) still to fill in — `ss-magic plugin checklist verify` lists them"
            ))
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Add an empty item to a named section.
fn add_item(
    doc: &mut Document,
    section_id: &str,
    id: &str,
    title: &str,
    now: u64,
) -> Result<String> {
    check_new_id(doc, id)?;
    let index = doc
        .sections
        .iter()
        .position(|s| s.id == section_id)
        .with_context(|| {
            format!(
                "no section `{section_id}` in this checklist; it has {}",
                list_ids(doc.sections.iter().map(|s| s.id.as_str()))
            )
        })?;

    doc.sections[index].items.push(Item {
        id: id.to_string(),
        title: title.to_string(),
        kind: ItemKind::Check,
        created: Timestamp::from_epoch_secs(now),
        // Written as an explicit null rather than left absent: `expected` is an
        // always-present key, and a check-kind item with a null expectation is
        // exactly the state `verify` should be reporting until it is filled in.
        expected: Some(None),
        ..Item::default()
    });
    Ok(format!("Added `{id}` to section `{section_id}`"))
}

/// Add a changelog entry.
fn add_entry(doc: &mut Document, id: &str, summary: &str, now: u64) -> Result<String> {
    check_new_id(doc, id)?;
    doc.changelog.push(ChangelogEntry {
        id: id.to_string(),
        created: Timestamp::from_epoch_secs(now),
        summary: summary.to_string(),
        ..ChangelogEntry::default()
    });
    Ok(format!("Recorded changelog entry `{id}`"))
}

/// Mark an item done, stamping the completion time.
///
/// Idempotent: an item that is already done keeps the timestamp it was
/// completed at, because that is a historical fact and re-running the verb is
/// not a reason to rewrite it.
fn mark_done(doc: &mut Document, id: &str, now: u64) -> Result<String> {
    let Located::Item(section, index) = locate(doc, id).with_context(|| unknown_id(doc, id))?
    else {
        bail!("`{id}` is not an item, so it cannot be marked done");
    };
    let item = &mut doc.sections[section].items[index];

    if item.done {
        let when = item
            .completed
            .as_ref()
            .filter(|ts| !ts.is_empty())
            .map_or_else(|| "an unrecorded time".to_string(), ToString::to_string);
        return Ok(format!("`{id}` was already done at {when}; left as it was"));
    }
    item.done = true;
    if item.completed.as_ref().is_none_or(Timestamp::is_empty) {
        item.completed = Some(Timestamp::from_epoch_secs(now));
    }
    Ok(format!("Marked `{id}` done"))
}

// ── `set` and its dotted keys ─────────────────────────────────────────────────

/// Which record an id names. Resolved to indices rather than to a reference so
/// the lookup can finish before the mutable borrow starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Located {
    Entry(usize),
    Section(usize),
    Item(usize, usize),
}

/// The id that addresses the document header itself. Spelled the same as the
/// `location` the validator reports header findings under, so a finding names
/// the id that fixes it.
const DOCUMENT_ID: &str = "document";

fn locate(doc: &Document, id: &str) -> Option<Located> {
    if let Some(index) = doc.changelog.iter().position(|e| e.id == id) {
        return Some(Located::Entry(index));
    }
    for (s, section) in doc.sections.iter().enumerate() {
        if section.id == id {
            return Some(Located::Section(s));
        }
        if let Some(i) = section.items.iter().position(|item| item.id == id) {
            return Some(Located::Item(s, i));
        }
    }
    None
}

/// `set <id> <dotted-key> <value>`. `value` is `None` for the literal `null`,
/// which clears whichever fields may be cleared and is an error on the rest.
fn apply_set(
    doc: &mut Document,
    id: &str,
    key: &str,
    value: Option<&str>,
    now: u64,
) -> Result<String> {
    if id == DOCUMENT_ID {
        set_document_field(doc, key, value)?;
        return Ok(format!("Set `{key}` on the document"));
    }

    let located = locate(doc, id).with_context(|| unknown_id(doc, id))?;
    match located {
        Located::Entry(index) => set_entry_field(&mut doc.changelog[index], key, value)?,
        Located::Section(index) => {
            let section = &mut doc.sections[index];
            match key {
                "title" => section.title = required_text(key, value)?,
                other => bail!(unknown_key(other, "a section", &["title"])),
            }
        }
        Located::Item(section, index) => {
            set_item_field(&mut doc.sections[section].items[index], key, value, now)?
        }
    }
    Ok(format!("Set `{key}` on `{id}`"))
}

fn set_document_field(doc: &mut Document, key: &str, value: Option<&str>) -> Result<()> {
    match key {
        "title" => doc.title = required_text(key, value)?,
        // The slug is metadata inside the document. Changing it deliberately
        // does NOT rename the file: the path is what the pointer, the pull
        // request and every reference already point at, and a rename behind a
        // `set` would break all three silently.
        "slug" => doc.slug = required_text(key, value)?,
        "created" => doc.created = timestamp(key, value)?,
        other => bail!(unknown_key(
            other,
            "the document",
            &["title", "slug", "created"]
        )),
    }
    Ok(())
}

fn set_entry_field(entry: &mut ChangelogEntry, key: &str, value: Option<&str>) -> Result<()> {
    match key {
        "summary" => entry.summary = required_text(key, value)?,
        "details" => entry.details = optional_text(value),
        "created" => entry.created = timestamp(key, value)?,
        other if other == "refs" || other.starts_with("refs.") => {
            set_refs(&mut entry.refs, other, value)?
        }
        other => bail!(unknown_key(
            other,
            "a changelog entry",
            &["summary", "details", "created", "refs…"]
        )),
    }
    Ok(())
}

fn set_item_field(item: &mut Item, key: &str, value: Option<&str>, now: u64) -> Result<()> {
    match key {
        "title" => item.title = required_text(key, value)?,
        "description" => item.description = optional_text(value),
        "why" => item.why = optional_text(value),
        "created" => item.created = timestamp(key, value)?,
        // Always-present key: a null here is the deliberate "there is nothing
        // to check", which the validator allows only on a record- or
        // decision-kind item.
        "expected" => item.expected = Some(optional_text(value)),
        // Always-present key too, but with no null/absent distinction to keep:
        // both spellings mean "no completion was recorded".
        "completed" => {
            item.completed = match value {
                None => None,
                Some(_) => Some(timestamp(key, value)?),
            }
        }
        "kind" => {
            item.kind = match required_text(key, value)?.as_str() {
                "check" => ItemKind::Check,
                "record" => ItemKind::Record,
                "decision" => ItemKind::Decision,
                other => bail!("`{other}` is not a kind; write check, record or decision"),
            }
        }
        // Omitted entirely when unset, never written as a null — "unranked" is
        // an absence the ordering rule sorts last, not a value.
        "priority" => {
            item.priority = match value {
                None => None,
                Some("blocking") => Some(Priority::Blocking),
                Some("decision-blocking") => Some(Priority::DecisionBlocking),
                Some("follow-up") => Some(Priority::FollowUp),
                Some(other) => bail!(
                    "`{other}` is not a priority; write blocking, decision-blocking, \
                     follow-up, or null to leave it unranked"
                ),
            }
        }
        "done" => {
            let done = match required_text(key, value)?.as_str() {
                "true" => true,
                "false" => false,
                other => bail!("`{other}` is not a boolean; write true or false"),
            };
            item.done = done;
            // Keep `done` and the completion timestamp agreeing, which is what
            // the validator checks in both directions. Setting it true stamps
            // the time if none was recorded; setting it false drops a stamp
            // that would otherwise describe work that is not finished.
            if done {
                if item.completed.as_ref().is_none_or(Timestamp::is_empty) {
                    item.completed = Some(Timestamp::from_epoch_secs(now));
                }
            } else {
                item.completed = None;
            }
        }
        other if other == "steps" || other.starts_with("steps.") => {
            set_steps(&mut item.steps, other, value)?
        }
        other if other == "refs" || other.starts_with("refs.") => {
            set_refs(&mut item.refs, other, value)?
        }
        other => bail!(unknown_key(
            other,
            "an item",
            &[
                "title",
                "kind",
                "priority",
                "done",
                "completed",
                "description",
                "why",
                "expected",
                "created",
                "steps…",
                "refs…",
            ]
        )),
    }
    Ok(())
}

/// `steps`, `steps.-` and `steps.<n>`.
///
/// Bare `steps` replaces the whole list, one step per non-empty line, which is
/// what makes a heredoc of several steps a single command. `steps.-` appends
/// the value as ONE step, newlines and all, for a step whose text genuinely
/// spans lines.
fn set_steps(steps: &mut Vec<String>, key: &str, value: Option<&str>) -> Result<()> {
    match key.strip_prefix("steps.") {
        None => {
            *steps = value
                .map(|text| {
                    text.lines()
                        .map(str::trim_end)
                        .filter(|line| !line.trim().is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
        }
        Some("-") => steps.push(
            value
                .context("`steps.-` appends a step, so it needs text rather than null")?
                .to_string(),
        ),
        Some(index) => {
            let index = parse_index(index, "steps")?;
            if index >= steps.len() {
                bail!(
                    "there is no step {index}; this item has {} step(s), \
                     and `steps.-` is how a new one is appended",
                    steps.len()
                );
            }
            match value {
                Some(text) => steps[index] = text.to_string(),
                None => {
                    steps.remove(index);
                }
            }
        }
    }
    Ok(())
}

/// `refs`, `refs.-`, `refs.<n>.url` and `refs.<n>.label`.
///
/// `refs.-` appends a reference whose label starts out as the URL itself, so a
/// one-command append never leaves the bare-URL rendering the validator warns
/// about; `refs.<n>.label` is how it gets a real one.
fn set_refs(refs: &mut Vec<Reference>, key: &str, value: Option<&str>) -> Result<()> {
    let Some(tail) = key.strip_prefix("refs.") else {
        if value.is_some() {
            bail!(
                "`refs` holds structured entries; append one with `refs.-  <url>`, \
                 re-label it with `refs.0.label`, or write null to clear them all"
            );
        }
        refs.clear();
        return Ok(());
    };

    if tail == "-" {
        let url = value
            .context("`refs.-` appends a reference, so it needs a URL rather than null")?
            .trim()
            .to_string();
        refs.push(Reference {
            label: url.clone(),
            url,
            ..Reference::default()
        });
        return Ok(());
    }

    let (index, field) = tail
        .split_once('.')
        .context("a reference field is addressed as `refs.<n>.url` or `refs.<n>.label`")?;
    let index = parse_index(index, "refs")?;
    let reference = refs.get_mut(index).with_context(|| {
        format!("there is no reference {index}; append one with `refs.-  <url>` first")
    })?;
    match field {
        "url" => reference.url = required_text("refs.url", value)?.trim().to_string(),
        "label" => reference.label = required_text("refs.label", value)?,
        other => bail!("`{other}` is not a reference field; write url or label"),
    }
    Ok(())
}

fn parse_index(raw: &str, what: &str) -> Result<usize> {
    raw.parse::<usize>()
        .with_context(|| format!("`{raw}` is not an index; `{what}.<n>` addresses one by position"))
}

/// A field that may be absent: the literal `null` and text that is empty or
/// only whitespace both clear it.
///
/// Collapsing the two is what makes an empty stdin body do the obvious thing.
/// Writing the empty string instead would leave `"why": ""` in the file — a
/// key that renders as an empty section and says nothing, where an absent key
/// says the author had nothing to add. `expected` is the sharpest case: an
/// empty string there is what the validator specifically asks to be written as
/// null instead.
fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

/// A field that has to hold text: the literal `null` is refused rather than
/// quietly writing an empty string the validator would then report.
fn required_text(key: &str, value: Option<&str>) -> Result<String> {
    let text =
        value.with_context(|| format!("`{key}` is required, so it cannot be set to null"))?;
    if text.trim().is_empty() {
        bail!("`{key}` is required, so it cannot be set to empty text");
    }
    Ok(text.to_string())
}

/// A timestamp, refused at the point of entry when it cannot be read. Catching
/// it here rather than at `verify` time means the file never holds a value
/// nothing can order by.
fn timestamp(key: &str, value: Option<&str>) -> Result<Timestamp> {
    let raw = required_text(key, value)?;
    let ts = Timestamp::new(raw.trim());
    if let Err(err) = ts.instant() {
        bail!("`{key}` (`{ts}`) cannot be read: {err}");
    }
    Ok(ts)
}

// ── Resolving which document to work on ───────────────────────────────────────

/// Find the active checklist and read it.
///
/// `Ok(None)` means there is nothing to work on and the reason — no checklist,
/// a pointer to a file that is gone, a document hand-edited into invalid JSON
/// — has already been reported on stderr. Every one of those is the same thing
/// to a caller: the command as typed cannot be carried out, so it returns
/// [`REFUSED`].
///
/// The pointer is asked first. Where there is none — a fresh clone, or a
/// worktree created after the checklist was committed, both of which have the
/// document but not the gitignored state tree — the `docs/actions/` naming
/// convention answers instead, as long as it answers unambiguously.
fn load_active(root: &Path) -> Result<Option<(PathBuf, Document)>> {
    let path = match resolve_active(root)? {
        Some(path) => path,
        None => {
            eprintln!(
                "{}",
                style::err("error: this repository has no checklist yet")
            );
            eprintln!(
                "{}",
                style::info("Create one with `ss-magic plugin checklist init <slug>`.")
            );
            return Ok(None);
        }
    };

    match read_document(&path) {
        Ok(Some(doc)) => Ok(Some((path, doc))),
        Ok(None) => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: {} is the active checklist but does not exist",
                    display_rel(root, &path)
                ))
            );
            eprintln!(
                "{}",
                style::info(
                    "`ss-magic plugin checklist init <slug>` creates it, or points at another one."
                )
            );
            Ok(None)
        }
        // A hand-edited file that no longer parses. Reported verbatim, with
        // the parse error serde produced, because that names the offending key
        // and is the only thing that helps here.
        Err(err) => {
            fail(format!("{err:#}"));
            Ok(None)
        }
    }
}

/// The active checklist's path, whether or not the file exists.
fn resolve_active(root: &Path) -> Result<Option<PathBuf>> {
    if let Some(path) = pointer_target(root) {
        return Ok(Some(path));
    }

    let dir = root.join(ACTIONS_REL);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(None);
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches_convention(root, p))
        .collect();
    found.sort();

    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => bail!(
            "no pointer records which checklist is active, and {ACTIONS_REL}/ holds several: \
             {}. Run `ss-magic plugin checklist init <slug>` with the one you mean.",
            list_ids(found.iter().filter_map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.trim_end_matches(CHECKLIST_SUFFIX))
            }))
        ),
    }
}

// ── Writing files ─────────────────────────────────────────────────────────────

/// Put `doc` into canonical order, repair the shape defects the validator
/// promises the next write repairs, and replace `path` atomically.
///
/// The temp file is a sibling of the target so the rename never crosses a
/// filesystem, and it is named with a leading dot so a run interrupted between
/// the write and the rename leaves something obviously not a checklist rather
/// than a second document in `docs/actions/`.
fn write_document(path: &Path, doc: &mut Document) -> Result<()> {
    canonicalize(doc);
    for section in doc.sections.iter_mut() {
        for item in &mut section.items {
            // `expected` is an always-present key. An absent one is what the
            // validator warns about with "the next write will add it as null";
            // this is that write keeping the promise.
            if item.expected.is_none() {
                item.expected = Some(None);
            }
        }
    }
    let body = to_json(doc)?;

    let dir = path
        .parent()
        .context("the checklist path has no parent directory")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    // A rewrite keeps whatever mode the file already had; a new file gets the
    // ordinary world-readable mode, because this is committed content rather
    // than private state.
    let mode = fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(NEW_FILE_MODE);

    let mut tmp = tempfile::Builder::new()
        .prefix(".checklist-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .with_context(|| format!("creating a temp file in {}", dir.display()))?;
    tmp.write_all(body.as_bytes())
        .context("writing the checklist")?;
    tmp.flush().context("flushing the checklist")?;
    tmp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting the mode on {}", path.display()))?;
    tmp.as_file().sync_all().ok();
    tmp.persist(path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Replace the pointer atomically, for exactly the reasons the document write
/// is atomic: a reader sees one whole pointer or the other.
fn write_pointer(state_root: &Path, pointer: &Pointer) -> Result<()> {
    let path = state_root.join(POINTER_NAME);
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(pointer).context("serializing the checklist pointer")?
    );

    let mut tmp = tempfile::Builder::new()
        .prefix(".checklist-")
        .suffix(".tmp")
        .tempfile_in(state_root)
        .with_context(|| format!("creating a temp file in {}", state_root.display()))?;
    tmp.write_all(body.as_bytes())
        .context("writing the checklist pointer")?;
    tmp.flush().context("flushing the checklist pointer")?;
    tmp.as_file().sync_all().ok();
    tmp.persist(&path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Run `f` holding the advisory lock that serializes checklist writes.
///
/// Best-effort by design: where the state tree does not exist there is nowhere
/// to put a lock file, and a checklist is repository content that must stay
/// writable regardless. The write is atomic either way — the lock only adds
/// protection against a lost update, never against a torn file.
fn with_lock<T>(state_root: &Path, f: impl FnOnce() -> T) -> Result<T> {
    if !state_root.is_dir() {
        return Ok(f());
    }
    tmproot::with_lock(state_root, LOCK_NAME, f)
        .with_context(|| format!("locking {}", state_root.join(LOCK_NAME).display()))
}

// ── Odds and ends ─────────────────────────────────────────────────────────────

/// A browsable repository URL from the `origin` remote, for the render's
/// metadata block.
///
/// The render puts this in a Markdown link, so an scp-style remote
/// (`git@github.com:owner/repo.git`) is rewritten into the `https://` form a
/// reader can actually click, and anything that is neither is dropped rather
/// than rendered as a dead link. `None` — no origin, or an unrecognizable one
/// — simply omits the line.
fn browsable_origin(root: &Path) -> Option<String> {
    let raw = git::origin_url(root).ok().flatten()?;
    let raw = raw.trim().trim_end_matches('/');
    let raw = raw.strip_suffix(".git").unwrap_or(raw);

    if let Some(rest) = raw.strip_prefix("ssh://") {
        // `ssh://[user@]host[:port]/path` — the transport is not browsable,
        // but the host and path are.
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        return Some(format!("https://{host}/{path}"));
    }
    if raw.starts_with("https://") || raw.starts_with("http://") {
        return Some(raw.to_string());
    }
    if raw.contains("://") {
        // `git://`, `file://` and anything else: not a URL a reader can open.
        return None;
    }
    // The scp-like form, `[user@]host:owner/repo`.
    let (authority, path) = raw.split_once(':')?;
    let host = authority.rsplit('@').next()?;
    (!host.is_empty() && !path.is_empty()).then(|| format!("https://{host}/{path}"))
}

/// `path` as the repository-relative text a person recognises, falling back to
/// the absolute path when it is somehow outside the repository.
fn display_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Refuse an id that is malformed or already taken. Both are permanent
/// mistakes — references and history hang off an id — so they are caught
/// before anything points at it rather than at `verify` time.
fn check_new_id(doc: &Document, id: &str) -> Result<()> {
    if !is_well_formed_id(id) {
        bail!(
            "`{id}` is not a well-formed id; ids are kebab-case, begin with a letter, \
             and hold only lowercase letters, digits and single hyphens"
        );
    }
    if id == DOCUMENT_ID {
        bail!("`{DOCUMENT_ID}` addresses the checklist header itself and cannot name a record");
    }
    if locate(doc, id).is_some() {
        bail!("`{id}` is already used in this checklist; ids are unique across the whole document");
    }
    Ok(())
}

/// The message for an id that addresses nothing, naming what the document
/// actually holds so the next attempt can be right.
fn unknown_id(doc: &Document, id: &str) -> String {
    format!(
        "no record `{id}` in this checklist; it holds {}",
        list_ids(
            doc.sections
                .iter()
                .map(|s| s.id.as_str())
                .chain(
                    doc.sections
                        .iter()
                        .flat_map(|s| s.items.iter().map(|i| i.id.as_str()))
                )
                .chain(doc.changelog.iter().map(|e| e.id.as_str()))
        )
    )
}

fn unknown_key(key: &str, what: &str, accepted: &[&str]) -> String {
    format!(
        "`{key}` is not a field of {what}; it accepts {}",
        list_ids(accepted.iter().copied())
    )
}

/// A comma-separated list of names, or an explicit "nothing" so a message
/// never trails off after "it holds ".
fn list_ids<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<String> = ids.map(|id| format!("`{id}`")).collect();
    if names.is_empty() {
        "nothing".to_string()
    } else {
        names.join(", ")
    }
}

/// Report a usage mistake and hand back the exit code for one.
fn usage_error(message: &str) -> ExitCode {
    eprintln!("{}", style::err(format!("error: {message}")));
    eprintln!("{USAGE}");
    refused()
}

/// The one non-zero code every refusal uses: the command as typed cannot be
/// carried out. `verify` is the exception — a document that is merely invalid
/// exits 1, so a CI job can tell "your checklist has errors" from "your
/// command was wrong". A function rather than a constant because
/// `ExitCode::from` is not callable in a constant.
fn refused() -> ExitCode {
    ExitCode::from(2)
}

/// Report a refusal that is not a usage mistake — an unknown id, an unreadable
/// document — with the same exit code, since both mean "the command as typed
/// cannot be carried out".
fn fail(message: String) -> ExitCode {
    eprintln!("{}", style::err(format!("error: {message}")));
    refused()
}

/// Seconds since the Unix epoch, or 0 if the clock is before it.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
