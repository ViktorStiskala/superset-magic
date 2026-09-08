//! `ss-magic-plugin`: the Claude Code plugin's hook runtime and verb tree.
//!
//! ## Two callers, two postures
//!
//! Everything in this binary is invoked by one of two very different callers,
//! and they must never share a command:
//!
//! - **The harness**, through `ss-magic-plugin hook <event>`. The envelope
//!   arrives on stdin and the response goes out as JSON on stdout, so nothing
//!   else may be printed there. A hook that cannot do its job exits 0 anyway —
//!   failing loudly would break the user's session over a tool that is only
//!   advisory.
//! - **A skill**, through a named verb (`status`, `checklist`, …). These report
//!   problems on stderr and exit non-zero, the ordinary CLI contract. Nobody is
//!   expected to type one in a terminal: the verbs are reached through Claude
//!   Code, whose Bash tool carries `plugin/bin/ss-magic-plugin` on `PATH`.
//!
//! Keeping them apart is also a safety boundary: only the human verbs reach
//! anything that writes configuration (`enable`, `disable`, `config set`), so a
//! repository cannot arrange its own enablement by getting a hook to fire. The
//! `SessionStart` bootstrap does invoke `seed-config`, but that entry point is
//! structurally incapable of writing the `enabled` key — see `config.rs`.
//! There is no install verb at all — the marketplace is the only delivery path.
//!
//! ## Its own binary, its own release line
//!
//! This used to be a `plugin` subcommand of the `ss-magic` sync CLI, gated out
//! of that binary's self-update path by an inclusion list in its `main.rs`. It
//! is now a separate crate releasing from `ss-magic-plugin-vX.Y.Z` tags, so the
//! separation is structural: this crate depends on neither `self_update` nor
//! any prompt or terminal-UI library, so it cannot update itself or open a TUI
//! even by mistake. That matters because the marketplace ships the binary
//! alongside the skills, hooks and Markdown that describe its behavior; a
//! silent mid-session self-update would leave the two disagreeing.
//!
//! ## Layout
//!
//! The crate root owns only the argv parse and the dispatch table. The work
//! lives in siblings: `config.rs` (the typed `plugin` key, its overlay
//! resolution, the `enable`/`disable`/`config get`/`config set` write path, and
//! the bootstrap's `seed-config` gate-defaults writer), `compact_window.rs`
//! (the opt-in `autoCompactWindow` write behind `compact-window`),
//! `identity.rs` (the `<repo>-<branch>` slug), `scratchpad.rs` (the
//! `.superset/.magic/` state tree), `bypass.rs` (the one-shot Read-gate claims
//! behind `bypass`), `expect_artifact.rs` (the pending subagent-output
//! declarations behind `expect-artifact`, which `SubagentStop` enforces),
//! `claim.rs` (the rename-based exactly-once file claim both of those one-shot
//! stores are built on), `tmproot.rs` (the private per-machine temporary root
//! and its fd-lock helper), `hook/` (the stdin decode, event routing, JSON
//! envelope and fail-open wrapper), `heartbeat.rs` (the machine-level
//! `hooks.jsonl` row every hook leaves behind), `cache.rs` (the hash-keyed
//! conclusion cache behind `conclude` / `conclusions` / `gc`), `ledger.rs` (the
//! machine-level cost ledger `SessionEnd` appends to, and the `cost` verb that
//! reports it), `checklist/` (the typed checklist document, its canonical
//! ordering and its validator, which knows nothing about hooks), `setup_ci.rs`
//! (the pinned GitHub Actions workflow behind `setup-github-ci`, whose bytes
//! are an embedded asset), `status.rs` (the one place that answers "why is the
//! plugin not doing anything", across both enablement layers, the ignored-tree
//! gate and the bootstrap) and `spill_index.rs` (a read-only listing of the
//! harness's own oversized-tool-output files for this worktree). The shared git,
//! hashing, palette and `.superset` plumbing comes from `ss-magic-core`.

use std::process::ExitCode;

use anyhow::Result;

use ss_magic_core::style;

// The shared plumbing lives in `ss-magic-core`; re-exported under the module
// names this tree used while it was a subtree of the `ss-magic` binary, so
// every `crate::git::…` and `crate::hashing::…` path still resolves.
pub(crate) use ss_magic_core::{git, hashing};

// A note on `#[allow(dead_code)]`: several methods below have no production
// caller and are kept for round-trip tests. They carried an `allow` while this
// file was `plugin/mod.rs`, a `pub(crate) mod` inside the `ss-magic` binary.
// It is not needed here and has been removed: `pub` items declared directly in
// a BINARY's crate root count as externally reachable, so rustc's dead-code
// lint does not consider them at all. Re-adding one would suppress nothing and
// would imply a warning that cannot occur.

pub(crate) mod atomic;
pub(crate) mod bypass;
pub(crate) mod cache;
pub(crate) mod checklist;
pub(crate) mod claim;
pub(crate) mod compact_window;
pub(crate) mod config;
pub(crate) mod expect_artifact;
pub(crate) mod heartbeat;
pub(crate) mod hook;
pub(crate) mod identity;
pub(crate) mod ledger;
pub(crate) mod pathnorm;
pub(crate) mod release_check;
pub(crate) mod scratchpad;
pub(crate) mod setup_ci;
pub(crate) mod spill_index;
pub(crate) mod status;
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
    /// `hook` with no event token at all.
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
    /// Fold the gate defaults into an existing `.superset/magic.json` so a
    /// person can find and edit them (writes configuration, but is
    /// structurally unable to write `plugin.enabled` — see `config.rs`).
    SeedConfig,
    /// Inspect or adjust the compaction window.
    CompactWindow,
    /// Report the newest known plugin release against the pin, optionally
    /// refreshing the cache with one bounded fetch. Writes only that cache
    /// file – never configuration, never a binary.
    ReleaseCheck,
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
            "seed-config" => Self::SeedConfig,
            "compact-window" => Self::CompactWindow,
            "release-check" => Self::ReleaseCheck,
            "setup-github-ci" => Self::SetupGithubCi,
            "checklist" => Self::Checklist,
            _ => return None,
        })
    }

    /// The token that selects this verb.
    // No production caller now that every verb dispatches to a real handler —
    // `run_human`'s old "not implemented yet" fallback was the last one, and
    // it is gone now that the fallback arm has nothing left to catch. Kept
    // for round-trip completeness (tests assert `as_str` reverses
    // `from_token` for every verb) and for a future caller that wants to
    // name a verb back to the user.
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
            Self::SeedConfig => "seed-config",
            Self::CompactWindow => "compact-window",
            Self::ReleaseCheck => "release-check",
            Self::SetupGithubCi => "setup-github-ci",
            Self::Checklist => "checklist",
        }
    }

    /// True when running this verb can write configuration.
    ///
    /// This used to double as "nothing reachable from a hook may be one of
    /// these", which is no longer literally true: `SeedConfig` writes
    /// configuration AND is invoked by `hooks/bootstrap.sh`, a `SessionStart`
    /// hook. The property that actually mattered survives intact and is
    /// narrower than the old phrasing — **a repository cannot arrange its own
    /// ENABLEMENT by getting a hook to fire**. `Enable`, `Disable` and `Config`
    /// are the verbs that can reach `plugin.enabled`, and no hook invokes any
    /// of them; `SeedConfig` has no code path to that key at all
    /// (`config::seed_block`), which is a stronger guarantee than being kept
    /// away from it by convention.
    ///
    /// So do NOT re-derive "hook-reachable" from this predicate. It answers
    /// "does this write configuration", nothing more.
    // Asserted by the tests; no production caller reads it today.
    pub fn writes_config(&self) -> bool {
        matches!(
            self,
            Self::Enable | Self::Disable | Self::Config | Self::SeedConfig
        )
    }

    /// True when running this verb can set `plugin.enabled`. The invariant the
    /// plugin's hook/human split exists to protect: this must be false for
    /// every verb any hook invokes.
    pub fn can_set_enabled(&self) -> bool {
        matches!(self, Self::Enable | Self::Disable | Self::Config)
    }
}

// ── Parse result ──────────────────────────────────────────────────────────────

/// A resolved plugin invocation, with the argv the verb still has to interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// `hook <event> [ARGS...]` — driven by stdin, answers on stdout.
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

/// Outcome of the argv parse. The non-`Invocation` variants are terminal
/// signals [`run`] turns into a print plus an exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Dispatch this invocation.
    Invocation(Invocation),
    /// `-V` / `--version`; print [`version_line`] and exit 0.
    Version,
    /// `-h` / `--help`; print usage and exit 0.
    Help,
    /// The binary was invoked with no arguments at all.
    MissingVerb,
    /// A token that is neither `hook` nor a known verb; the string is the
    /// offending token.
    UnknownVerb(String),
}

/// Usage banner for the plugin verb tree.
pub const USAGE: &str = "\
Usage: ss-magic-plugin <VERB> [ARGS...]

Hook entry point (driven by a JSON envelope on stdin, for the harness):
  hook <event>          One of: session-start, pre-tool-use, pre-compact,
                        subagent-stop, session-end. (`file-changed` still routes,
                        but no shipped manifest declares it, so nothing fires it.)

Verbs (driven by argv). These are reached through Claude Code, whose Bash tool
carries the wrapper on PATH; none of them is meant to be typed in a terminal:
  status                What the plugin sees, and why it is or is not acting
  cost                  What recorded sessions cost, across every worktree
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
  seed-config           Fold the gate defaults into an existing magic.json
  compact-window        Inspect or adjust the compaction window
  release-check         Newest known plugin release vs. the pin; --refresh re-reads
  setup-github-ci       Write the GitHub Actions workflow into this repository
  checklist             Operator-checklist verbs (the only write path for it)

Options:
  -V, --version         Print `ss-magic-plugin <version>` and exit
  -h, --help            Print this text and exit

This binary never installs an update and never opens an interactive menu — it
links neither an updater nor a terminal-UI library. `release-check` only reports
that a newer plugin release exists; updating it is done through /plugin.";

/// Render the plugin usage text. A function so the help and error paths share
/// one source of truth.
pub fn usage() -> &'static str {
    USAGE
}

/// The single line `ss-magic-plugin --version` prints. Sourced from
/// `Cargo.toml` at compile time, so it cannot drift from the version the
/// marketplace pins.
///
/// Its exact shape is a contract, not a cosmetic choice. `hooks/bootstrap.sh`
/// gates every install on `"$staged_bin" --version | head -1 | awk '{print
/// $NF}'` equalling the pin in `ss-magic-plugin.version`, and `status.rs`
/// probes the same flag to report drift. Both read the LAST whitespace-
/// separated field of the FIRST line, so the version must stay last on line
/// one — and the flag must answer with it rather than with usage text, or the
/// bootstrap would discard every download it made.
pub fn version_line() -> String {
    format!("ss-magic-plugin {}", env!("CARGO_PKG_VERSION"))
}

/// Parse the whole argv, minus the program name.
///
/// Pure and process-free: no stdin is read and no filesystem is touched, so the
/// whole verb tree is unit-testable without spawning anything.
///
/// `-V`/`--version` and `-h`/`--help` are recognized only as the FIRST token,
/// deliberately. Scanning the rest of argv for them — which the `ss-magic` CLI
/// does — would swallow a flag that belongs to a verb, and the verbs here parse
/// their own arguments. First-token-only is also all either caller needs: the
/// bootstrap runs `"$staged_bin" --version` and `status` probes the same shape,
/// both with the flag alone.
pub fn parse(args: &[String]) -> Parsed {
    let Some(first) = args.first() else {
        return Parsed::MissingVerb;
    };

    if first == "-V" || first == "--version" {
        return Parsed::Version;
    }

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

/// Parse the argv and route it. Split from [`main`] so the whole dispatch is
/// exercisable from a test without spawning the process.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let parsed = parse(args);

    if forces_no_color(&parsed) {
        style::init_no_color();
    } else {
        style::init();
    }

    match parsed {
        Parsed::Invocation(inv) => dispatch(inv),
        // Answered before anything else looks at the argv, and with no
        // network call, no state write and no dispatch: the bootstrap runs
        // this on a freshly downloaded binary to confirm it executes on this
        // machine as the pinned version, and it must be cheap and total.
        Parsed::Version => {
            println!("{}", version_line());
            Ok(ExitCode::SUCCESS)
        }
        Parsed::Help => {
            println!("{}", usage());
            Ok(ExitCode::SUCCESS)
        }
        Parsed::MissingVerb => {
            eprintln!("{}", style::err("error: `ss-magic-plugin` needs a verb"));
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

/// Route a human verb to its handler. Every verb here reports problems on
/// stderr and exits non-zero — the posture every human verb keeps.
fn run_human(verb: HumanVerb, args: &[String]) -> Result<ExitCode> {
    match verb {
        HumanVerb::Scratchpad => scratchpad::run(args),
        HumanVerb::Bypass => bypass::run(args),
        HumanVerb::ExpectArtifact => expect_artifact::run(args),
        HumanVerb::Conclude => cache::run_conclude(args),
        HumanVerb::Conclusions => cache::run_conclusions(args),
        HumanVerb::Gc => cache::run_gc(args),
        HumanVerb::Cost => ledger::run(args),
        HumanVerb::SetupGithubCi => setup_ci::run(args),
        HumanVerb::Checklist => checklist::run(args),
        HumanVerb::Status => status::run(args),
        HumanVerb::SpillIndex => spill_index::run(args),
        HumanVerb::Enable => config::run_enable(args),
        HumanVerb::Disable => config::run_disable(args),
        HumanVerb::Config => config::run_config(args),
        HumanVerb::SeedConfig => config::run_seed_config(args),
        HumanVerb::CompactWindow => compact_window::run(args),
        HumanVerb::ReleaseCheck => release_check::run(args),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        // The human verbs' ordinary error posture. A hook never reaches this:
        // `hook::run` has no failing path at all, which is what makes a broken
        // hook indistinguishable from one that decided to do nothing.
        Err(err) => {
            eprintln!("{}", style::err(format!("error: {err:#}")));
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests;
