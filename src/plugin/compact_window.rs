//! `ss-magic plugin compact-window --set <TOKENS>` — opt in to an absolute
//! auto-compact window (R30, R31).
//!
//! Claude Code auto-compacts a session once its transcript nears the model's
//! context window. `autoCompactWindow` is the harness's own first-class
//! settings key for overriding that trigger point with an absolute token
//! count (100,000–1,000,000) — the same key `/autocompact` writes. This verb
//! exists because the alternative knob, the `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`
//! environment variable, is a percentage that can only ever LOWER the window,
//! is bound to a field literally named `testPctOverride`, and drifts in
//! meaning across models with different context sizes — none of which is
//! what a repository wants to express.
//!
//! ## Which file, and why
//!
//! The harness reads `autoCompactWindow` from `.claude/settings.json`
//! (git-tracked, repository-wide) and `.claude/settings.local.json`
//! (gitignored, per-machine), with the local file winning. A context budget
//! is a per-machine preference, not a repository fact, so this verb writes
//! ONLY the local file (R31) — never the tracked one, which would commit one
//! developer's setting to everyone who clones the repository.
//!
//! ## Opt-in only, and never a clobber
//!
//! R30 requires the write to be explicit (a bare `ss-magic plugin
//! compact-window` with no `--set` does nothing but print usage) and R31
//! requires it to never overwrite a value already there. Both matter for the
//! same reason: this is the user's own context-budget preference, and a tool
//! that silently changed or replaced it would be actively hostile to a
//! setting they already made deliberately, whether by hand or via
//! `/autocompact`. When `autoCompactWindow` is already present in the local
//! file, this verb reports its value and writes nothing.
//!
//! ## Round-trips unrelated content
//!
//! `.claude/settings.local.json` is a harness-owned file with a much larger
//! schema than the one key this verb cares about (permissions, env, hook
//! overrides, ...). The write is a load-modify-write over a generic JSON
//! object — insert `autoCompactWindow` alongside whatever else is already
//! there — never a fresh file built from just this one key, so an existing
//! settings file never loses content it already carried.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use crate::git;
use crate::git::gitignore::{self, PathKind};
use crate::tui::style;

/// Repo-relative path of the harness's per-machine local settings file.
/// Unlike `.superset/magic.local.json`, nothing gitignores this by default —
/// exactly the gap R30 closes, in the same step as the write.
const SETTINGS_LOCAL_REL: &str = ".claude/settings.local.json";

/// The settings key both `/autocompact` and this verb write.
const WINDOW_KEY: &str = "autoCompactWindow";

/// Lower bound `/autocompact`'s own parser enforces (100k tokens). Mirrored
/// here so a value this verb would refuse anyway is caught before it ever
/// reaches a file the harness will try to parse.
const WINDOW_MIN: u64 = 100_000;
/// Upper bound `/autocompact`'s own parser enforces (1M tokens).
const WINDOW_MAX: u64 = 1_000_000;

const USAGE: &str = "\
Usage: ss-magic plugin compact-window --set <TOKENS>

Write an absolute auto-compact window (100000-1000000 tokens) into this
repository's local, gitignored settings file (.claude/settings.local.json),
and gitignore that file in the same step if it is not already.

Never overwrites a window already set there: if `autoCompactWindow` is
already present, this reports its value and writes nothing. Never touches
the git-tracked .claude/settings.json — a context budget is a per-machine
preference, not a repository fact.";

/// `plugin compact-window` — a human verb; problems report on stderr and
/// exit non-zero.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let window = match parse_args(args) {
        ParsedArgs::Set(window) => window,
        ParsedArgs::Help => {
            println!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        ParsedArgs::Error(message) => return Ok(usage_error(&message)),
    };

    let cwd = std::env::current_dir().context("reading the current directory")?;
    let root = git::cwd_repo_root(&cwd).unwrap_or(cwd);
    run_core(&root, window)
}

#[derive(Debug)]
enum ParsedArgs {
    Set(u64),
    Help,
    Error(String),
}

/// Parse argv into a validated window value. `--set` is the only supported
/// flag; the value must be a plain absolute integer (no `k`/`M` shorthand —
/// R30 asks for an absolute count, and inventing a second, ambiguous
/// notation on top of it is not this verb's job) inside
/// [`WINDOW_MIN`]..=[`WINDOW_MAX`].
fn parse_args(args: &[String]) -> ParsedArgs {
    match args {
        [flag] if flag == "-h" || flag == "--help" => ParsedArgs::Help,
        [flag, value] if flag == "--set" => match value.parse::<u64>() {
            Ok(window) if (WINDOW_MIN..=WINDOW_MAX).contains(&window) => ParsedArgs::Set(window),
            Ok(window) => ParsedArgs::Error(format!(
                "`{window}` is outside the allowed range \
                 {WINDOW_MIN}-{WINDOW_MAX} tokens"
            )),
            Err(_) => ParsedArgs::Error(format!(
                "`{value}` is not a whole number of tokens"
            )),
        },
        [] => ParsedArgs::Error("needs `--set <TOKENS>`".to_string()),
        _ => ParsedArgs::Error("usage: compact-window --set <TOKENS>".to_string()),
    }
}

/// Every verb against an explicit root, so the whole flow is testable
/// without a process or a real current directory.
fn run_core(root: &Path, window: u64) -> Result<ExitCode> {
    let path = root.join(SETTINGS_LOCAL_REL);

    let mut settings = match read_settings_object(&path)? {
        ExistingSettings::Absent => Map::new(),
        ExistingSettings::Object(map) => map,
        ExistingSettings::NotAnObject => {
            return Ok(fail(format!(
                "{SETTINGS_LOCAL_REL} exists but its top-level value is not a JSON object; \
                 fix it by hand before opting in"
            )))
        }
        ExistingSettings::Malformed(reason) => {
            return Ok(fail(format!(
                "{SETTINGS_LOCAL_REL} exists but is not valid JSON ({reason}); fix it by hand \
                 before opting in"
            )))
        }
    };

    if let Some(existing) = settings.get(WINDOW_KEY) {
        println!(
            "{}",
            style::ok(format!(
                "`{WINDOW_KEY}` is already {existing} in {SETTINGS_LOCAL_REL}; leaving it \
                 unchanged"
            ))
        );
        return Ok(ExitCode::SUCCESS);
    }

    settings.insert(WINDOW_KEY.to_string(), Value::from(window));
    write_settings_object(&path, &settings)?;
    gitignore::ensure_path_ignored(root, root, Path::new(SETTINGS_LOCAL_REL), PathKind::File)
        .with_context(|| format!("gitignoring {SETTINGS_LOCAL_REL}"))?;

    println!(
        "{}",
        style::ok(format!("Wrote `{WINDOW_KEY}: {window}` to {SETTINGS_LOCAL_REL}"))
    );
    println!("{}", style::ok(format!("Gitignored {SETTINGS_LOCAL_REL}")));
    Ok(ExitCode::SUCCESS)
}

/// The shapes the local settings file can already be in, from this verb's
/// point of view.
enum ExistingSettings {
    /// No file at all — the ordinary "first opt-in" case.
    Absent,
    /// A JSON object, whatever keys it already carries.
    Object(Map<String, Value>),
    /// The file parses as JSON but its top-level value isn't an object (an
    /// array, a bare number, `null`, ...) — there is nowhere to insert a key
    /// without discarding whatever is actually there.
    NotAnObject,
    /// The file's bytes are not valid JSON at all. Carries `serde_json`'s own
    /// message (line/column included) so the refusal below can point at
    /// exactly what is wrong.
    Malformed(String),
}

/// Read `path` as a JSON object, or report why it cannot be treated as one.
/// A missing file is [`ExistingSettings::Absent`], not a failure. Malformed
/// JSON and a well-formed-but-non-object value are both reported as VALUES
/// here, not `Err` — this file is not ss-magic's own, and rebuilding it from
/// nothing on a parse failure would silently discard whatever a person or the
/// harness already put there, so the caller has to see the problem and
/// refuse cleanly rather than have it surface as a generic I/O-flavored
/// error. Only a genuine I/O failure reading the file (permissions, ...)
/// propagates as `Err`.
fn read_settings_object(path: &Path) -> Result<ExistingSettings> {
    if !path.exists() {
        return Ok(ExistingSettings::Absent);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(map)) => ExistingSettings::Object(map),
        Ok(_) => ExistingSettings::NotAnObject,
        Err(err) => ExistingSettings::Malformed(err.to_string()),
    })
}

/// Write `settings` to `path`, pretty-printed with a trailing newline,
/// creating `.claude/` if it does not exist yet.
fn write_settings_object(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = format!("{}\n", serde_json::to_string_pretty(settings)?);
    fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

/// Report a usage mistake and hand back its exit code.
fn usage_error(message: &str) -> ExitCode {
    eprintln!("{}", style::err(format!("error: {message}")));
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// Report a refusal that is not a usage mistake (a settings file this verb
/// cannot safely touch) with the same exit code — both mean "the command as
/// typed cannot be carried out".
fn fail(message: String) -> ExitCode {
    eprintln!("{}", style::err(format!("error: {message}")));
    ExitCode::from(2)
}

#[cfg(test)]
mod tests;
