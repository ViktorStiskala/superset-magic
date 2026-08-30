//! `ss-magic plugin setup-github-ci` — write the checklist workflow into the
//! repository this is run from.
//!
//! ## Why a verb and not a skill
//!
//! A skill body is Markdown handed to a model; it can describe a workflow but
//! it cannot write one, and a model asked to reproduce 200 lines of YAML from a
//! description will approximate it. The parts that matter here are exactly the
//! parts an approximation loses: the two-job split that keeps a write token
//! away from pull-request code, the `pull_request` trigger rather than
//! `pull_request_target`, the checksum check on the downloaded binary. So the
//! bytes live in `assets/workflow/checklist.yml`, embedded at compile time, and
//! this verb is the only thing that puts them on disk.
//!
//! What the skill still owns is the conversation: which of the four states the
//! repository is in, whether the user wants the write to happen, and what to
//! say when they do not. This verb answers the first question and performs the
//! write; it never asks anything, because it may be running from a hook-free
//! shell with no terminal at all.
//!
//! ## The four states
//!
//! [`classify`] returns exactly one of them, and the first line of output names
//! it in a stable form (`state: absent`, `identical`, `pin-stale`, `differs`)
//! so the skill can branch on a token rather than on prose:
//!
//! - **absent** — no workflow file. Writing it is purely additive.
//! - **identical** — the file already is what this build would write, byte for
//!   byte. Nothing is written, and that is reported as success rather than as a
//!   no-op error.
//! - **pin-stale** — the file is this exact workflow with a different version
//!   pinned. Recognised by re-rendering the template at the version the file
//!   itself names and finding that the result matches: that is what separates
//!   "only the pin moved" from "somebody edited it", without diffing structure.
//! - **differs** — anything else. It might be a deliberate local change (a
//!   different runner, an extra step) so the verb refuses to overwrite it
//!   without `--force`, and prints a diff so the decision can be made on the
//!   actual difference.
//!
//! ## Where confirmation lives
//!
//! There is none in here. `--check` reports and writes nothing; a bare run
//! writes; and the one destructive case, overwriting a workflow somebody
//! changed, needs `--force` on top. So every path that can lose bytes takes a
//! flag that a person or a skill had to type after seeing what `--check` said.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::git;
use crate::plugin::atomic;
use crate::plugin::checklist::{ACTIONS_REL, CHECKLIST_SUFFIX};
use crate::tui::style;

/// The workflow template, embedded at compile time — the same `include_str!`
/// arrangement `assets/magic.sh` uses, and for the same reason: the shipped
/// binary must carry its own copy, because it runs in a repository that has
/// none.
pub const TEMPLATE: &str = include_str!("../../assets/workflow/checklist.yml");

/// The token [`render`] replaces with the pinned version. Chosen so it is not
/// a plausible version string: a template that somehow reached a repository
/// unsubstituted would fail the download loudly rather than resolve to
/// something.
pub const VERSION_PLACEHOLDER: &str = "@SS_MAGIC_VERSION@";

/// Where the workflow is written, relative to the repository root.
///
/// Prefixed with the tool's name rather than being called `checklist.yml`: a
/// repository is free to have its own workflow by any plain name, and this file
/// is replaced wholesale on every write, so it must be unmistakably ours.
pub const WORKFLOW_REL: &str = ".github/workflows/ss-magic-checklist.yml";

/// The key the pinned version is stored under in the workflow's `env:` block.
/// [`pinned_version`] reads it back out of a file on disk, which is what makes
/// a stale pin distinguishable from a hand edit.
const PIN_KEY: &str = "SS_MAGIC_VERSION:";

/// Mode for the written workflow: committed repository content, so
/// world-readable, unlike anything under the plugin's state tree.
const FILE_MODE: u32 = 0o644;

/// How many diff lines are printed for a differing workflow before the rest is
/// summarised. A whole-file diff of a rewritten workflow is 400 lines and tells
/// the reader nothing the first screen did not.
const DIFF_LINE_BUDGET: usize = 120;

/// Usage for the `setup-github-ci` verb.
const USAGE: &str = "\
Usage: ss-magic plugin setup-github-ci [--check] [--force]

Write the GitHub Actions workflow that renders this repository's operator
checklist into a pull-request comment, pinning the ss-magic it installs.

  --check   Report what would happen and write nothing.
  --force   Overwrite a workflow that was changed locally. Without it, that
            one case is refused; every other case writes on its own.

The workflow is written to .github/workflows/ss-magic-checklist.yml.";

// ── Rendering ─────────────────────────────────────────────────────────────────

/// The workflow as it should exist for `version`.
///
/// A single textual substitution, deliberately: the template is a YAML file a
/// person can read and review as the thing that ships, not a structure
/// assembled at runtime that nobody ever sees whole.
pub fn render(version: &str) -> String {
    TEMPLATE.replace(VERSION_PLACEHOLDER, version)
}

/// The version a workflow file on disk pins, if it names one.
///
/// Scans for the `env:` entry rather than parsing YAML: the crate has no YAML
/// reader and does not want one for a single key. A file that has been edited
/// past recognition simply yields `None`, which lands it in
/// [`State::Differs`] — the safe side, since that is the state that refuses to
/// overwrite.
pub fn pinned_version(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(PIN_KEY)?;
        let value = rest.trim();
        // The template quotes the value so YAML cannot read `1.10` as a
        // number. Accept it unquoted too, so a hand-written pin is still
        // recognised rather than being reported as an unrelated edit.
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

// ── The four states ───────────────────────────────────────────────────────────

/// What the repository's copy of the workflow is, relative to what this build
/// would write. See the module docs for what each one means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// No workflow file at all.
    Absent,
    /// Already byte-for-byte what would be written.
    Identical,
    /// This workflow, at another version; the string is the version on disk.
    PinStale { found: String },
    /// Changed locally, or not this workflow at all.
    Differs,
}

impl State {
    /// The stable token printed as the first output line. The skill branches
    /// on these, so they are part of the interface and not free to be reworded.
    pub fn token(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Identical => "identical",
            Self::PinStale { .. } => "pin-stale",
            Self::Differs => "differs",
        }
    }

    /// Whether reaching this state and writing would replace bytes somebody
    /// may have meant to keep.
    fn needs_force(&self) -> bool {
        matches!(self, Self::Differs)
    }
}

/// Decide which state `existing` is in for the given target `version`.
///
/// Pure, so the whole decision is testable without a repository.
pub fn classify(existing: Option<&str>, version: &str) -> State {
    let Some(existing) = existing else {
        return State::Absent;
    };
    if existing == render(version) {
        return State::Identical;
    }
    // Re-render at the version the file itself names. If that reproduces the
    // file exactly, the pin is the only thing that moved — which is the one
    // difference that can be advanced without asking whether a local change
    // was deliberate.
    if let Some(found) = pinned_version(existing) {
        if found != version && existing == render(&found) {
            return State::PinStale { found };
        }
    }
    State::Differs
}

// ── The verb ─────────────────────────────────────────────────────────────────

/// `ss-magic plugin setup-github-ci [--check] [--force]` — a human verb, so
/// problems go to stderr with a non-zero exit.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut check = false;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--check" | "-n" => check = true,
            "--force" | "-f" => force = true,
            other => {
                let what = if other.starts_with('-') {
                    format!("error: unknown `setup-github-ci` flag `{other}`")
                } else {
                    format!("error: `setup-github-ci` takes no arguments, got `{other}`")
                };
                eprintln!("{}", style::err(what));
                eprintln!("{USAGE}");
                return Ok(ExitCode::from(2));
            }
        }
    }

    let cwd = std::env::current_dir().context("reading the current directory")?;
    run_core(&cwd, env!("CARGO_PKG_VERSION"), check, force)
}

/// The verb against an explicit directory and version, so the flow is testable
/// without moving the process's working directory.
///
/// `version` is the running binary's own version at every production call site.
/// The workflow installs the ss-magic that wrote it, which is what keeps the
/// pin from being a value anybody has to maintain: there is no second copy of
/// it in this repository to drift out of step with `Cargo.toml`.
pub fn run_core(cwd: &Path, version: &str, check: bool, force: bool) -> Result<ExitCode> {
    let root = match git::cwd_repo_root(cwd) {
        Ok(root) => root,
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            eprintln!(
                "{}",
                style::info(
                    "`setup-github-ci` writes into a repository's .github/ directory, so it has \
                     to be run inside one."
                )
            );
            return Ok(ExitCode::from(2));
        }
    };

    let path = root.join(WORKFLOW_REL);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        // A file that exists but cannot be read is not the same as no file:
        // treating it as absent would overwrite it. Refuse instead.
        Err(err) => {
            eprintln!(
                "{}",
                style::err(format!("error: cannot read {WORKFLOW_REL}: {err}"))
            );
            return Ok(ExitCode::from(2));
        }
    };

    let state = classify(existing.as_deref(), version);
    println!("state: {}", state.token());
    report_state(&state, version);

    // Advisory, never a refusal: setting CI up before writing the first
    // checklist is a reasonable order to do things in, and the workflow is
    // written to sit quiet until a checklist appears.
    if !has_checklist(&root) {
        println!(
            "{}",
            style::warn(format!(
                "This repository has no {ACTIONS_REL}/*{CHECKLIST_SUFFIX} yet, so the workflow \
                 will find nothing to render."
            ))
        );
        println!(
            "{}",
            style::info("  `ss-magic plugin checklist init <slug>` creates one.")
        );
    }

    if state == State::Differs {
        print_diff(existing.as_deref().unwrap_or(""), &render(version));
    }

    if check {
        println!("{}", style::info(would(&state, force)));
        return Ok(ExitCode::SUCCESS);
    }

    if state == State::Identical {
        println!(
            "{}",
            style::ok(format!(
                "{WORKFLOW_REL} is already current; nothing written."
            ))
        );
        return Ok(ExitCode::SUCCESS);
    }

    if state.needs_force() && !force {
        eprintln!(
            "{}",
            style::err(format!(
                "refused: {WORKFLOW_REL} was changed locally, and writing would discard that."
            ))
        );
        eprintln!(
            "{}",
            style::info(
                "  Re-run with `--force` to replace it, or keep the local version and leave the \
                 pin where it is."
            )
        );
        return Ok(ExitCode::from(1));
    }

    write_workflow(&path, &render(version))?;
    println!(
        "{}",
        style::ok(format!("Wrote {WORKFLOW_REL}, pinning ss-magic {version}."))
    );
    println!(
        "{}",
        style::info("  Commit it; it runs on the next pull request.")
    );
    Ok(ExitCode::SUCCESS)
}

/// The human half of the state line: what was found, in the terms the skill
/// needs in order to say something useful about it.
fn report_state(state: &State, version: &str) {
    match state {
        State::Absent => println!(
            "{}",
            style::info(format!("No {WORKFLOW_REL} in this repository."))
        ),
        State::Identical => println!(
            "{}",
            style::info(format!(
                "{WORKFLOW_REL} matches ss-magic {version} exactly."
            ))
        ),
        State::PinStale { found } => println!(
            "{}",
            style::info(format!(
                "{WORKFLOW_REL} is this workflow, pinning ss-magic {found}; \
                 the current pin is {version}."
            ))
        ),
        State::Differs => println!(
            "{}",
            style::info(format!(
                "{WORKFLOW_REL} exists and is not what ss-magic {version} would write."
            ))
        ),
    }
}

/// What a `--check` run says it would do. Spelled out per state rather than
/// left implicit, because this line is what the skill quotes when it asks.
fn would(state: &State, force: bool) -> String {
    match state {
        State::Absent => "A run without --check would create it.".to_string(),
        State::Identical => "A run without --check would leave it alone.".to_string(),
        State::PinStale { .. } => "A run without --check would advance the pin.".to_string(),
        State::Differs if force => {
            "A run without --check would replace it, discarding the local changes.".to_string()
        }
        State::Differs => {
            "A run without --check would refuse; `--force` replaces it, discarding the local \
             changes."
                .to_string()
        }
    }
}

/// Whether any document matching the `docs/actions/` naming convention exists.
///
/// The same convention the workflow's own glob uses, and the same one
/// `checklist verify` falls back to when the gitignored pointer is absent — as
/// it always is in CI.
fn has_checklist(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root.join(ACTIONS_REL)) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(CHECKLIST_SUFFIX))
    })
}

/// Print a bounded unified diff of the workflow on disk against the one that
/// would be written.
///
/// The skill is told to show the difference before proposing an overwrite, and
/// it has no way to compute one: it cannot see the bytes this build would
/// write, since they only exist inside the binary. So the verb produces it.
fn print_diff(current: &str, proposed: &str) {
    let diff = similar::TextDiff::from_lines(current, proposed);
    let text = diff
        .unified_diff()
        .context_radius(3)
        .header("on disk", "would write")
        .to_string();

    let lines: Vec<&str> = text.lines().collect();
    for line in lines.iter().take(DIFF_LINE_BUDGET) {
        println!("{}", style::info(*line));
    }
    if lines.len() > DIFF_LINE_BUDGET {
        println!(
            "{}",
            style::info(format!(
                "  … and {} more diff line(s); run `git diff` against a written copy to see \
                 all of it.",
                lines.len() - DIFF_LINE_BUDGET
            ))
        );
    }
}

/// Replace the workflow atomically.
///
/// Temp file then rename, the same shape every other writer in the crate uses:
/// a reader — git, a reviewer, a CI checkout — sees either the whole previous
/// workflow or the whole new one, and an interrupted run leaves the previous
/// file untouched rather than a half-written YAML file that Actions would
/// reject.
fn write_workflow(path: &Path, body: &str) -> Result<()> {
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    atomic::write_atomically(
        path,
        body,
        ".ss-magic-checklist-",
        ".tmp",
        Some("the workflow"),
        Some(FILE_MODE),
        true,
    )
}

#[cfg(test)]
mod tests;
