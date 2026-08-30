//! The `ss-magic plugin` entry point: the Claude Code plugin's verb tree.
//!
//! ## Two callers, two postures
//!
//! Everything under `plugin` is invoked by one of two very different callers,
//! and they must never share a command:
//!
//! - **The harness**, through `plugin hook <event>`. The envelope arrives on
//!   stdin and the response goes out as JSON on stdout, so nothing else may be
//!   printed there. A hook that cannot do its job exits 0 anyway — failing
//!   loudly would break the user's session over a tool that is only advisory.
//! - **A human or a skill**, through a named verb (`status`, `checklist`, …).
//!   These report problems on stderr and exit non-zero, the ordinary CLI
//!   contract.
//!
//! Keeping them apart is also a safety boundary: only the human verbs reach
//! anything that writes configuration (`enable`, `disable`, `config set`), so a
//! repository cannot arrange its own enablement by getting a hook to fire.
//! There is no install verb at all — the marketplace is the only delivery path.
//!
//! ## No update gate, no TUI
//!
//! `main.rs` handles [`crate::cli::Parsed::Plugin`] outside the update gate's
//! inclusion list, and nothing here constructs the interactive menu. The
//! binary is pinned alongside the skills, hooks and Markdown the marketplace
//! ships with it; a silent self-update mid-session would leave the binary and
//! those files describing different behavior.
//!
//! ## Layout
//!
//! This module owns only the second-level parse and the dispatch table. The
//! work lives in siblings: `config.rs` (the typed `plugin` key and its
//! overlay resolution), `identity.rs` (the `<repo>-<branch>` slug),
//! `scratchpad.rs` (the `.superset/.magic/` state tree), `tmproot.rs` (the
//! private per-machine temporary root and its fd-lock helper), `hook/` (the
//! stdin decode, event routing, JSON envelope and fail-open wrapper),
//! `heartbeat.rs` (the machine-level `hooks.jsonl` row every hook leaves
//! behind) and, added by later units, `status.rs`, `checklist/`, `ledger.rs`
//! and `cache.rs`.

use std::process::ExitCode;

use anyhow::Result;

use crate::tui::style;

pub(crate) mod config;
pub(crate) mod heartbeat;
pub(crate) mod hook;
pub(crate) mod identity;
pub(crate) mod scratchpad;
pub(crate) mod tmproot;

// ── Hook events ───────────────────────────────────────────────────────────────

/// One harness hook channel, named as it appears in the plugin manifest.
///
/// [`HookEvent::Unknown`] and [`HookEvent::Missing`] are values, not parse
/// errors, on purpose: a manifest from a newer plugin build can name an event
/// this binary has never heard of, and the contract for that case is to exit 0
/// with empty stdout and record the unroutable name — which the hook wrapper
/// can only do if the name reaches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    /// `SessionStart` — inject operating guidance and the checklist location.
    SessionStart,
    /// `PreToolUse` — the read/edit gate.
    PreToolUse,
    /// `PreCompact` — persist what the compaction is about to drop.
    PreCompact,
    /// `SubagentStop` — collect a dispatched agent's result.
    SubagentStop,
    /// `SessionEnd` — close out the ledger row.
    SessionEnd,
    /// `FileChanged` — refresh the environment through direnv on a `.env` /
    /// `.envrc` write.
    FileChanged,
    /// An event name this binary does not route; the string is the name as the
    /// manifest spelled it.
    Unknown(String),
    /// `plugin hook` with no event token at all.
    Missing,
}

impl HookEvent {
    /// Map a manifest event name onto a channel. Total — an unrecognized name
    /// becomes [`HookEvent::Unknown`] rather than failing.
    pub fn from_token(token: &str) -> Self {
        match token {
            "session-start" => Self::SessionStart,
            "pre-tool-use" => Self::PreToolUse,
            "pre-compact" => Self::PreCompact,
            "subagent-stop" => Self::SubagentStop,
            "session-end" => Self::SessionEnd,
            "file-changed" => Self::FileChanged,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// The wire name, for heartbeat rows and diagnostics. Empty for
    /// [`HookEvent::Missing`], which carries no name to report.
    // No production caller yet — the hook wrapper that stamps event names into
    // heartbeat rows arrives with the hook modules.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        match self {
            Self::SessionStart => "session-start",
            Self::PreToolUse => "pre-tool-use",
            Self::PreCompact => "pre-compact",
            Self::SubagentStop => "subagent-stop",
            Self::SessionEnd => "session-end",
            Self::FileChanged => "file-changed",
            Self::Unknown(name) => name,
            Self::Missing => "",
        }
    }
}

// ── Human verbs ───────────────────────────────────────────────────────────────

/// A verb a person or a skill types. Deliberately a closed set: anything not
/// listed here is a loud error, so a typo never silently does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanVerb {
    /// Report what the plugin sees and why it is or is not acting.
    Status,
    /// Report the token/cost ledger, optionally backfilling a lost session.
    Cost,
    /// List the harness's own spill files for this worktree, read-only.
    SpillIndex,
    /// Bootstrap and inspect the scratchpad state tree.
    Scratchpad,
    /// Record a conclusion for a file so a repeat read is answered from cache.
    Conclude,
    /// List recorded conclusions.
    Conclusions,
    /// Prune expired state.
    Gc,
    /// Record a one-shot claim that lets the next matching read through.
    Bypass,
    /// Declare an artifact a later step is expected to produce.
    ExpectArtifact,
    /// Turn the hooks on for this repository (writes configuration).
    Enable,
    /// Stop the hooks acting, leaving the installed tree alone (writes
    /// configuration).
    Disable,
    /// `config get` / `config set` over the plugin block (`set` writes
    /// configuration).
    Config,
    /// Inspect or adjust the compaction window.
    CompactWindow,
    /// Write the GitHub Actions workflow into the consuming repository.
    SetupGithubCi,
    /// The operator-checklist verb family (`init`, `add-item`, `set`, `done`,
    /// `list`, `verify`, `render-md`, …).
    Checklist,
}

impl HumanVerb {
    /// Map a typed token onto a verb, or `None` when it is not one of ours.
    /// Note what is absent: there is no `install` token, because the
    /// marketplace is the only way the plugin is ever installed.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "status" => Self::Status,
            "cost" => Self::Cost,
            "spill-index" => Self::SpillIndex,
            "scratchpad" => Self::Scratchpad,
            "conclude" => Self::Conclude,
            "conclusions" => Self::Conclusions,
            "gc" => Self::Gc,
            "bypass" => Self::Bypass,
            "expect-artifact" => Self::ExpectArtifact,
            "enable" => Self::Enable,
            "disable" => Self::Disable,
            "config" => Self::Config,
            "compact-window" => Self::CompactWindow,
            "setup-github-ci" => Self::SetupGithubCi,
            "checklist" => Self::Checklist,
            _ => return None,
        })
    }

    /// The token that selects this verb.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Cost => "cost",
            Self::SpillIndex => "spill-index",
            Self::Scratchpad => "scratchpad",
            Self::Conclude => "conclude",
            Self::Conclusions => "conclusions",
            Self::Gc => "gc",
            Self::Bypass => "bypass",
            Self::ExpectArtifact => "expect-artifact",
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Config => "config",
            Self::CompactWindow => "compact-window",
            Self::SetupGithubCi => "setup-github-ci",
            Self::Checklist => "checklist",
        }
    }

    /// True when running this verb can write configuration. Used to keep the
    /// hook/human split honest: nothing reachable from a hook may be one of
    /// these.
    // Asserted by the tests today; the config-writing verbs consume it when
    // they land.
    #[allow(dead_code)]
    pub fn writes_config(&self) -> bool {
        matches!(self, Self::Enable | Self::Disable | Self::Config)
    }
}

// ── Parse result ──────────────────────────────────────────────────────────────

/// A resolved plugin invocation, with the argv the verb still has to interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// `plugin hook <event> [ARGS...]` — driven by stdin, answers on stdout.
    Hook {
        /// Which channel fired.
        event: HookEvent,
        /// Whatever followed the event token.
        args: Vec<String>,
    },
    /// `plugin <verb> [ARGS...]` — driven by argv, answers on stdout/stderr.
    Human {
        /// Which verb was named.
        verb: HumanVerb,
        /// Whatever followed the verb token.
        args: Vec<String>,
    },
}

/// Outcome of the second-level parse. Mirrors [`crate::cli::Parsed`]: the
/// non-`Invocation` variants are terminal signals [`run`] turns into a print
/// plus an exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Dispatch this invocation.
    Invocation(Invocation),
    /// `plugin --help`; print usage and exit 0.
    Help,
    /// `plugin` with nothing after it.
    MissingVerb,
    /// A token that is neither `hook` nor a known verb; the string is the
    /// offending token.
    UnknownVerb(String),
}

/// Usage banner for the plugin verb tree.
pub const USAGE: &str = "\
Usage: ss-magic plugin <VERB> [ARGS...]

Hook entry point (driven by a JSON envelope on stdin, for the harness):
  hook <event>          One of: session-start, pre-tool-use, pre-compact,
                        subagent-stop, session-end, file-changed

Verbs (driven by argv, for humans and skills):
  status                What the plugin sees, and why it is or is not acting
  cost                  Token/cost ledger for this repository's sessions
  spill-index           List the harness's spill files for this worktree
  scratchpad            Bootstrap and inspect the scratchpad state tree
  conclude              Record a conclusion about a file
  conclusions           List recorded conclusions
  gc                    Prune expired plugin state
  bypass                Let the next matching read through, once
  expect-artifact       Declare an artifact a later step must produce
  enable                Turn the hooks on for this repository
  disable               Stop the hooks acting (leaves the install in place)
  config                Read or write plugin configuration keys
  compact-window        Inspect or adjust the compaction window
  setup-github-ci       Write the GitHub Actions workflow into this repository
  checklist             Operator-checklist verbs (the only write path for it)

This entry point never checks for or installs an update, and never opens the
interactive menu.";

/// Render the plugin usage text. A function so the help and error paths share
/// one source of truth, matching `cli::usage`.
pub fn usage() -> &'static str {
    USAGE
}

/// Parse the argv that followed the `plugin` token.
///
/// Pure and process-free: no stdin is read and no filesystem is touched, so the
/// whole verb tree is unit-testable without spawning anything.
pub fn parse(args: &[String]) -> Parsed {
    let Some(first) = args.first() else {
        return Parsed::MissingVerb;
    };

    if first == "-h" || first == "--help" {
        return Parsed::Help;
    }

    if first == "hook" {
        // An absent or unrecognized event is a value the hook wrapper reports,
        // not a parse failure — see `HookEvent`.
        let event = match args.get(1) {
            Some(token) => HookEvent::from_token(token),
            None => HookEvent::Missing,
        };
        let args = args.get(2..).map(<[String]>::to_vec).unwrap_or_default();
        return Parsed::Invocation(Invocation::Hook { event, args });
    }

    match HumanVerb::from_token(first) {
        Some(verb) => Parsed::Invocation(Invocation::Human {
            verb,
            args: args[1..].to_vec(),
        }),
        None => Parsed::UnknownVerb(first.clone()),
    }
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Entry point for `ss-magic plugin …`, called from `main.rs` outside the
/// auto-update gate. Parses the argv and routes it.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let parsed = parse(args);

    if forces_no_color(&parsed) {
        style::init_no_color();
    } else {
        style::init();
    }

    match parsed {
        Parsed::Invocation(inv) => dispatch(inv),
        Parsed::Help => {
            println!("{}", usage());
            Ok(ExitCode::SUCCESS)
        }
        Parsed::MissingVerb => {
            eprintln!(
                "{}",
                style::err("error: `ss-magic plugin` needs a verb")
            );
            eprintln!("{}", usage());
            Ok(ExitCode::from(2))
        }
        Parsed::UnknownVerb(token) => {
            eprintln!(
                "{}",
                style::err(format!("error: unknown plugin verb `{token}`"))
            );
            eprintln!("{}", usage());
            Ok(ExitCode::from(2))
        }
    }
}

/// Whether this invocation must force color off.
///
/// A hook verb owns stdout for its JSON envelope and stderr for plain-text
/// diagnostics; an ANSI escape would make the first unparseable and the second
/// harder to read in a transcript. Every other invocation — including a hook
/// event this binary cannot route, which prints nothing anyway — makes the
/// ordinary terminal-detection decision.
///
/// `main.rs` deliberately leaves the choice to us rather than initializing
/// style before the parse: the decision lives in a `OnceLock`, so whichever
/// call makes it first wins for the whole process.
fn forces_no_color(parsed: &Parsed) -> bool {
    matches!(parsed, Parsed::Invocation(Invocation::Hook { .. }))
}

/// Route a resolved invocation to its handler.
fn dispatch(inv: Invocation) -> Result<ExitCode> {
    match inv {
        Invocation::Hook { event, .. } => run_hook(&event),
        Invocation::Human { verb, args } => run_human(verb, &args),
    }
}

/// Hand the invocation to the hook pipeline, which owns the stdin decode, the
/// enablement and ignored-tree gates, the per-event dispatch, the JSON
/// envelope on stdout and the heartbeat row.
///
/// It always exits 0 — including for an event this binary cannot route, and
/// including when a handler fails outright. An unimplemented or broken hook
/// has to look to the harness exactly like a hook that decided to do nothing,
/// or an in-flight session would break on a tool that is only ever advisory.
fn run_hook(event: &HookEvent) -> Result<ExitCode> {
    hook::run(event)
}

/// Route a human verb to its handler. Verbs later units own still land on the
/// not-implemented arm, which reports on stderr and exits non-zero — the
/// posture every human verb keeps.
fn run_human(verb: HumanVerb, args: &[String]) -> Result<ExitCode> {
    match verb {
        HumanVerb::Scratchpad => scratchpad::run(args),
        _ => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: `ss-magic plugin {}` is not implemented yet",
                    verb.as_str()
                ))
            );
            Ok(ExitCode::from(2))
        }
    }
}

#[cfg(test)]
mod tests;
