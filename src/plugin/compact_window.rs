//! `ss-magic plugin compact-window` — advice on the auto-compact window, and
//! the one explicit way to write it.
//!
//! Two halves. `--recommend` (R24, R25) reports where a
//! `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` comes from, what `autoCompactWindow` the
//! repository configures, and a window sized from this repository's own
//! recorded sessions — and writes nothing at all. `--set <TOKENS>` (R30, R31)
//! is the opt-in write. The split is the whole safety story of R28: every
//! surface that ADVISES (this verb's `--recommend`, `status`'s compaction
//! section, `enable`'s tip line, the `SessionStart` notice) shares the
//! read-only helpers below, and the only path that touches a settings file is
//! `run_core`, reached from `--set` alone. The tool never edits the user's
//! `~/.claude/settings.json`, the tracked `.claude/settings.json`, or a managed
//! settings file, and never removes the override for anyone: it names the file
//! and asks the person to do it.
//!
//! ## The recommendation (R25, KTD11)
//!
//! Context need is a property of how a repository is worked on, not of its
//! size, so the number comes from the cost ledger: the newest
//! [`RECOMMEND_ROWS`] rows attributable to this repository (any worktree of
//! the same main checkout — see `ledger::rows_for_repository`) that carry a
//! `peak_context_tokens` figure. The window is 1.25x the largest peak — the
//! headroom that keeps auto-compaction from firing at exactly the point the
//! largest session needed — rounded UP to the next 10,000 so it reads as a
//! setting rather than a measurement, then clamped to the harness's own
//! accepted range. Three or more rows are `high` confidence, one or two `low`,
//! and none at all gives no number: the report says so and prints the generic
//! range guidance instead, never a made-up figure.
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
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::fmt::Write as _;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::git;
use crate::git::gitignore::{self, PathKind};
use crate::plugin::{atomic, heartbeat, ledger};
use crate::tui::style;

/// Repo-relative path of the harness's per-machine local settings file.
/// Unlike `.superset/magic.local.json`, nothing gitignores this by default —
/// exactly the gap R30 closes, in the same step as the write.
const SETTINGS_LOCAL_REL: &str = ".claude/settings.local.json";

/// Repo-relative path of the harness's git-tracked project settings file.
/// Read for the report; never written (R28).
const SETTINGS_PROJECT_REL: &str = ".claude/settings.json";

/// The environment variable this verb exists to talk people out of. It is
/// read live from the harness's `process.env`, and a settings file's `env`
/// block is copied into that environment at startup — so it can arrive from
/// the shell, from `~/.claude/settings.json`, from the project's two files,
/// or from a managed settings file, and the report has to say which.
pub const OVERRIDE_ENV: &str = "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE";

/// How many of the newest attributable ledger rows the recommendation reads
/// (R25). Enough that one unusual session does not dominate; few enough that
/// how the repository was worked on last month is what counts.
pub const RECOMMEND_ROWS: usize = 20;

/// Rows from which the recommendation is `high` confidence; fewer is `low`.
const HIGH_CONFIDENCE_ROWS: usize = 3;

/// The recommendation is rounded UP to a multiple of this.
const ROUNDING: u64 = 10_000;

/// Version of the `--recommend --json` shape. Bumped when a key changes
/// meaning or goes away.
pub const RECOMMEND_SCHEMA_VERSION: u32 = 1;

/// The settings key both `/autocompact` and this verb write.
const WINDOW_KEY: &str = "autoCompactWindow";

/// Lower bound `/autocompact`'s own parser enforces (100k tokens). Mirrored
/// here so a value this verb would refuse anyway is caught before it ever
/// reaches a file the harness will try to parse.
const WINDOW_MIN: u64 = 100_000;
/// Upper bound `/autocompact`'s own parser enforces (1M tokens).
const WINDOW_MAX: u64 = 1_000_000;

/// Mode a brand-new `.claude/settings.local.json` is created with, when there
/// is no existing file to preserve the mode of. Ordinary world-readable,
/// matching what a plain file write would have produced anyway — this is a
/// harness settings file, not private state, so there is nothing here that
/// warrants the owner-only mode the state-tree writers use.
const NEW_FILE_MODE: u32 = 0o644;

const USAGE: &str = "\
Usage: ss-magic plugin compact-window --recommend [--json]
       ss-magic plugin compact-window --set <TOKENS>

--recommend reports whether CLAUDE_AUTOCOMPACT_PCT_OVERRIDE is set and where
it was found, the autoCompactWindow this repository configures, and a window
recommended from this repository's own recorded sessions, with the exact
--set command. It writes nothing and exits 0.

--set writes an absolute auto-compact window (100000-1000000 tokens) into
this repository's local, gitignored settings file (.claude/settings.local.json),
and gitignores that file in the same step if it is not already. It is the
only thing this verb ever writes.

Never overwrites a window already set there: if `autoCompactWindow` is
already present, --set reports its value and writes nothing. Never touches
the git-tracked .claude/settings.json, the user's own settings, or the
override itself — a context budget is a per-machine preference, and those
files are the person's to edit.";

/// `plugin compact-window` — a human verb; problems report on stderr and
/// exit non-zero.
pub fn run(args: &[String]) -> Result<ExitCode> {
    match parse_args(args) {
        ParsedArgs::Set(window) => {
            let cwd = std::env::current_dir().context("reading the current directory")?;
            let root = git::cwd_repo_root(&cwd).unwrap_or(cwd);
            run_core(&root, window)
        }
        ParsedArgs::Recommend { json } => {
            let cwd = std::env::current_dir().context("reading the current directory")?;
            run_recommend(&cwd, json)
        }
        ParsedArgs::Help => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        ParsedArgs::Error(message) => Ok(usage_error(&message)),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ParsedArgs {
    /// `--set <TOKENS>`, validated.
    Set(u64),
    /// `--recommend`, optionally `--json`.
    Recommend { json: bool },
    Help,
    Error(String),
}

/// Parse argv. `--set` takes a plain absolute integer (no `k`/`M` shorthand
/// — R30 asks for an absolute count, and inventing a second, ambiguous
/// notation on top of it is not this verb's job) inside
/// [`WINDOW_MIN`]..=[`WINDOW_MAX`]; `--recommend` takes an optional `--json`
/// in either order. A bare invocation is still a usage error, not a report:
/// R30's reading of this verb as "an explicit write" is untouched, and a
/// person typing it with nothing after it is told both forms rather than
/// being handed one of them.
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
        [flag] if flag == "--recommend" => ParsedArgs::Recommend { json: false },
        [a, b] if (a == "--recommend" && b == "--json") || (a == "--json" && b == "--recommend") => {
            ParsedArgs::Recommend { json: true }
        }
        [flag] if flag == "--json" => {
            ParsedArgs::Error("`--json` only applies to `--recommend`".to_string())
        }
        [] => ParsedArgs::Error("needs `--recommend` or `--set <TOKENS>`".to_string()),
        _ => ParsedArgs::Error(
            "usage: compact-window --recommend [--json] | --set <TOKENS>".to_string(),
        ),
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
///
/// Goes through the shared atomic-write helper (a same-directory temp file,
/// then a rename over `path`) rather than a bare `fs::write`, so a crash
/// mid-write leaves the previous settings file intact instead of truncated —
/// this file carries a much larger schema than the one key this verb cares
/// about (permissions, env, hook overrides, ...), and a half-written replacement
/// would corrupt all of it, not just the key just added. A rewrite preserves
/// whatever mode the file already had; a brand-new file gets
/// [`NEW_FILE_MODE`].
fn write_settings_object(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = format!("{}\n", serde_json::to_string_pretty(settings)?);
    let mode = fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(NEW_FILE_MODE);
    atomic::write_atomically(
        path,
        &body,
        ".settings-",
        ".tmp",
        Some(SETTINGS_LOCAL_REL),
        Some(mode),
        false,
    )
}

// ── `--recommend`: the read-only half (R24, R25, R28) ───────────────────────

/// Where the advice surfaces look for the override besides the process
/// environment, and what that environment says.
///
/// Passed in rather than read inside so the report is testable against
/// tempdir settings files instead of the developer's own `~/.claude`, and so
/// `status` can hand over the same answers it already resolved. Every path is
/// optional: a machine with no home directory still gets a report, it just
/// says fewer places were searched.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// [`OVERRIDE_ENV`] as this process sees it, `None` when unset or empty
    /// (the harness ignores an empty value the same way).
    pub env_override: Option<String>,
    /// `${CLAUDE_CONFIG_DIR:-~/.claude}/settings.json` — the user's global
    /// settings, the usual home of a stray override.
    pub user_settings: Option<PathBuf>,
    /// The platform's managed settings file, when this platform has one.
    pub managed_settings: Option<PathBuf>,
}

impl Sources {
    /// The real process's view.
    pub fn from_process() -> Self {
        let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let config_dir = non_empty("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| non_empty("HOME").map(|home| PathBuf::from(home).join(".claude")));
        Self {
            env_override: non_empty(OVERRIDE_ENV),
            user_settings: config_dir.map(|dir| dir.join("settings.json")),
            managed_settings: managed_settings_path(),
        }
    }
}

/// The harness's managed-settings file for this platform: the one an
/// administrator writes and a user cannot override. Only ever read.
fn managed_settings_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from(
            "/Library/Application Support/ClaudeCode/managed-settings.json",
        ))
    } else if cfg!(target_os = "linux") {
        Some(PathBuf::from("/etc/claude-code/managed-settings.json"))
    } else {
        None
    }
}

/// Everything `--recommend` has to say. Serialized as-is for `--json`; the
/// invariant the text form also keeps is that a `null` value always has a
/// non-`null` `note` beside it, so a reader never has to guess why a number
/// is missing.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// [`RECOMMEND_SCHEMA_VERSION`].
    pub schema: u32,
    /// The repository root the report is about.
    pub root: String,
    /// [`OVERRIDE_ENV`]: whether it is set, and where.
    #[serde(rename = "override")]
    pub override_: OverrideReport,
    /// `autoCompactWindow` in the two project files.
    pub windows: Windows,
    /// The number, or why there is none.
    pub recommendation: Recommendation,
    /// The exact `--set` command, or the syntax to use with a value of the
    /// reader's own choosing when there is no number to recommend.
    pub command: CommandLine,
}

/// Where [`OVERRIDE_ENV`] was found.
#[derive(Debug, Clone, Serialize)]
pub struct OverrideReport {
    /// True when it was found anywhere at all.
    pub set: bool,
    /// Every place it was found, in the order searched.
    pub locations: Vec<OverrideLocation>,
    /// Every place that was looked at, so an empty `locations` says where the
    /// search went rather than leaving it to be assumed.
    pub searched: Vec<String>,
    /// The advice when it is set; the plain fact when it is not.
    pub note: String,
}

/// One place the override was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverrideLocation {
    /// `process environment`, or a settings file's path.
    pub location: String,
    /// The value there, rendered as text.
    pub value: String,
}

/// `autoCompactWindow` across the project's settings files.
#[derive(Debug, Clone, Serialize)]
pub struct Windows {
    /// `.claude/settings.local.json` — the file `--set` writes, and the one
    /// the harness prefers.
    pub local: WindowSetting,
    /// `.claude/settings.json` — the tracked file, read only.
    pub project: WindowSetting,
    /// True when either file sets a window: the fact `status`'s problem line,
    /// `enable`'s tip and the `SessionStart` notice all key on.
    pub configured: bool,
}

/// One settings file's `autoCompactWindow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowSetting {
    /// The file, repo-relative.
    pub file: String,
    /// The value exactly as the file has it (`/autocompact` writes a number,
    /// but a hand edit can put anything there, and the harness's parser is
    /// the authority on what it means). `None` when the key is absent — or
    /// when the file could not be read as an object, which `note` says.
    pub value: Option<Value>,
    /// Why `value` is `None`.
    pub note: Option<String>,
}

/// The number and its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recommendation {
    /// The window to set, or `None` with `note` saying why.
    pub tokens: Option<u64>,
    /// `high` (three or more sessions), `low` (one or two), or `None`.
    pub confidence: Option<&'static str>,
    /// How many recorded sessions the number rests on.
    pub sessions: usize,
    /// The largest single-request context any of them needed.
    pub max_peak_context_tokens: Option<u64>,
    /// How the number was arrived at, in a sentence — or, with no number, the
    /// generic guidance that stands in for it.
    pub basis: String,
    /// Why there is no number, when there is none.
    pub note: Option<String>,
}

/// The `--set` command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandLine {
    /// The exact command, when there is a number to put in it.
    pub value: Option<String>,
    /// The syntax to fill in by hand, when there is not.
    pub note: Option<String>,
}

/// R25's arithmetic over the peaks of the rows that qualified.
///
/// `recommended = clamp(ceil_to_10000(1.25 x max_peak), WINDOW_MIN,
/// WINDOW_MAX)`. Integer arithmetic throughout, so an exact multiple stays
/// exact: 1.25 x 80,000 is 100,000, not 100,000.00000001 rounded up to
/// 110,000.
pub(crate) fn recommend(peaks: &[u64]) -> Recommendation {
    let Some(max_peak) = peaks.iter().copied().max() else {
        return no_recommendation("no recorded sessions for this repository yet");
    };
    let sessions = peaks.len();
    // 1.25x as 5/4, ceiling — then up to the next 10,000, then into range.
    let with_headroom = (max_peak * 5).div_ceil(4);
    let rounded = with_headroom.div_ceil(ROUNDING) * ROUNDING;
    let tokens = rounded.clamp(WINDOW_MIN, WINDOW_MAX);
    let confidence = if sessions >= HIGH_CONFIDENCE_ROWS {
        "high"
    } else {
        "low"
    };
    let basis = format!(
        "The largest context any one request needed across the newest {sessions} recorded \
         session{} of this repository was {max_peak} tokens. 1.25x that, rounded up to the \
         next 10,000 and kept within {WINDOW_MIN}-{WINDOW_MAX}, is {tokens}. Confidence is \
         {confidence}: {HIGH_CONFIDENCE_ROWS} or more sessions count as high.",
        if sessions == 1 { "" } else { "s" }
    );
    Recommendation {
        tokens: Some(tokens),
        confidence: Some(confidence),
        sessions,
        max_peak_context_tokens: Some(max_peak),
        basis,
        note: None,
    }
}

/// The shape of a report with no number in it: the note says why, and the
/// basis carries the generic guidance so the reader still knows what the
/// setting means and what range it takes.
fn no_recommendation(note: &str) -> Recommendation {
    Recommendation {
        tokens: None,
        confidence: None,
        sessions: 0,
        max_peak_context_tokens: None,
        basis: format!(
            "The harness accepts {WINDOW_MIN}-{WINDOW_MAX} tokens. Unset, it uses the \
             model's own context window; a value below that compacts earlier, and a \
             value above it only matters on a model whose window is larger. ss-magic \
             sizes a window from the largest context this repository's recorded \
             sessions actually needed (rows are written when a session ends), so run \
             `--recommend` again once a few have ended."
        ),
        note: Some(note.to_string()),
    }
}

/// Find [`OVERRIDE_ENV`] in the process environment and the `env` block of
/// every settings file the harness reads (KTD11).
pub(crate) fn detect_override(root: &Path, sources: &Sources) -> OverrideReport {
    let mut searched: Vec<String> = Vec::new();
    let mut locations: Vec<OverrideLocation> = Vec::new();

    // The process environment first: it is what a hook sees, and what every
    // settings file below is copied into at startup.
    searched.push("process environment".to_string());
    if let Some(value) = &sources.env_override {
        locations.push(OverrideLocation {
            location: "process environment".to_string(),
            value: value.clone(),
        });
    }

    // Then every settings file the harness merges an `env` block from, in
    // the order it reads them. Each is read as a whole object through the
    // same reader `--set` trusts, so a malformed file is a non-finding here
    // exactly as it is a refusal there.
    let files: Vec<PathBuf> = sources
        .user_settings
        .iter()
        .cloned()
        .chain([root.join(SETTINGS_PROJECT_REL), root.join(SETTINGS_LOCAL_REL)])
        .chain(sources.managed_settings.iter().cloned())
        .collect();
    for file in files {
        let label = file.display().to_string();
        searched.push(format!("{label} (env block)"));
        if let Some(value) = env_value_in(&file) {
            locations.push(OverrideLocation {
                location: label,
                value,
            });
        }
    }

    let set = !locations.is_empty();
    let note = if set {
        format!(
            "{OVERRIDE_ENV} is a percentage that can only LOWER the auto-compact window, and \
             the same percentage means a different absolute cap on every model. Remove the \
             key by hand from the location(s) above — ss-magic never edits a settings file \
             and never removes the override for you — then set an absolute window with \
             the command below."
        )
    } else {
        format!("{OVERRIDE_ENV} is not set in any of the places searched.")
    };

    OverrideReport {
        set,
        locations,
        searched,
        note,
    }
}

/// [`OVERRIDE_ENV`] inside `path`'s `env` block, rendered as text. `None`
/// for an absent file, a file that is not a JSON object, an absent block or
/// an absent key — all of which mean "not set here".
fn env_value_in(path: &Path) -> Option<String> {
    let ExistingSettings::Object(map) = read_settings_object(path).ok()? else {
        return None;
    };
    let value = map.get("env")?.as_object()?.get(OVERRIDE_ENV)?;
    Some(match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })
}

/// `autoCompactWindow` in `root/<rel>`, or why it could not be read.
pub(crate) fn read_window(root: &Path, rel: &str) -> WindowSetting {
    let (value, note) = match read_settings_object(&root.join(rel)) {
        Ok(ExistingSettings::Object(map)) => match map.get(WINDOW_KEY) {
            Some(Value::Null) | None => (None, Some("not set".to_string())),
            Some(value) => (Some(value.clone()), None),
        },
        Ok(ExistingSettings::Absent) => (None, Some("not set (no such file)".to_string())),
        Ok(ExistingSettings::NotAnObject) => (
            None,
            Some("the file's top-level value is not a JSON object".to_string()),
        ),
        Ok(ExistingSettings::Malformed(reason)) => {
            (None, Some(format!("the file is not valid JSON ({reason})")))
        }
        Err(e) => (None, Some(format!("could not be read: {e:#}"))),
    };
    WindowSetting {
        file: rel.to_string(),
        value,
        note,
    }
}

/// Whether either project settings file configures a window — the fact every
/// advisory surface keys on.
pub(crate) fn window_configured(root: &Path) -> bool {
    read_window(root, SETTINGS_LOCAL_REL).value.is_some()
        || read_window(root, SETTINGS_PROJECT_REL).value.is_some()
}

/// The one line `enable` prints after its success line when the repository
/// configures no window (R27) — a pointer to `--recommend`, never a value and
/// never a write. `None` when a window is already set, so the tip is not
/// repeated at a person who has already made the choice.
pub(crate) fn enable_tip(root: &Path) -> Option<String> {
    if window_configured(root) {
        return None;
    }
    Some(format!(
        "Tip: no autoCompactWindow is configured for this repository. \
         `ss-magic plugin compact-window --recommend` sizes one from its recorded \
         sessions; nothing is written until you run `--set`."
    ))
}

/// Build the whole report. Reads only: the two project settings files, the
/// settings files in `sources`, and the cost ledger in `store` — and creates
/// none of them. `main_root` is the repository's main checkout (the
/// attribution key for ledger rows); `None` when git could not name one, in
/// which case the recommendation says so rather than guessing at a population.
pub fn recommend_report(
    root: &Path,
    main_root: Option<&Path>,
    store: Option<&Path>,
    sources: &Sources,
) -> Report {
    let override_ = detect_override(root, sources);
    let local = read_window(root, SETTINGS_LOCAL_REL);
    let project = read_window(root, SETTINGS_PROJECT_REL);
    let configured = local.value.is_some() || project.value.is_some();

    let recommendation = match (main_root, store) {
        (None, _) => no_recommendation(
            "git could not name this repository's main checkout, so its recorded \
             sessions cannot be told apart from another repository's",
        ),
        (_, None) => no_recommendation(
            "this platform has no application data directory, so no cost ledger is kept \
             to size a window from",
        ),
        (Some(main_root), Some(store)) => {
            match ledger::rows_for_repository(store, main_root, RECOMMEND_ROWS) {
                Ok(rows) => {
                    let peaks: Vec<u64> = rows.iter().filter_map(|r| r.peak_context_tokens).collect();
                    recommend(&peaks)
                }
                Err(e) => no_recommendation(&format!("the cost ledger could not be read: {e:#}")),
            }
        }
    };

    let command = match recommendation.tokens {
        Some(tokens) => CommandLine {
            value: Some(format!("ss-magic plugin compact-window --set {tokens}")),
            note: None,
        },
        None => CommandLine {
            value: None,
            note: Some(format!(
                "no number to recommend yet; choose a value in {WINDOW_MIN}-{WINDOW_MAX} and \
                 run `ss-magic plugin compact-window --set <TOKENS>`"
            )),
        },
    };

    Report {
        schema: RECOMMEND_SCHEMA_VERSION,
        root: root.display().to_string(),
        override_,
        windows: Windows {
            local,
            project,
            configured,
        },
        recommendation,
        command,
    }
}

/// `--recommend` against the real current directory, environment and store.
fn run_recommend(cwd: &Path, json: bool) -> Result<ExitCode> {
    let resolved = git::discover::roots(cwd);
    let root = resolved
        .repo_root
        .clone()
        .unwrap_or_else(|| cwd.to_path_buf());
    // The non-creating lookup: a report must not scaffold the store it reads.
    let store = heartbeat::existing_store_dir();
    let report = recommend_report(
        &root,
        resolved.main_root.as_deref(),
        store.as_deref(),
        &Sources::from_process(),
    );
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// The report for a person. Into a buffer, not stdout, so the layout is
/// testable without capturing the process's output.
pub(crate) fn render_text(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}",
        style::header(format!("Auto-compact window for {}", report.root))
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header(OVERRIDE_ENV));
    if report.override_.set {
        for found in &report.override_.locations {
            let _ = writeln!(
                out,
                "  {}",
                style::warn(format!("set to {} in {}", found.value, found.location))
            );
        }
    } else {
        let _ = writeln!(out, "  {}", style::ok("not set"));
    }
    let _ = writeln!(out, "  {}", style::info(&report.override_.note));
    let _ = writeln!(
        out,
        "  {}",
        style::info(format!("searched: {}", report.override_.searched.join(", ")))
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header(WINDOW_KEY));
    for setting in [&report.windows.local, &report.windows.project] {
        let value = match (&setting.value, &setting.note) {
            (Some(v), _) => style::ok(v.to_string()),
            (None, Some(note)) => style::info(note),
            (None, None) => style::info("not set"),
        };
        let _ = writeln!(out, "  {:<30}{value}", setting.file);
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("Recommendation"));
    let rec = &report.recommendation;
    match (rec.tokens, rec.confidence) {
        (Some(tokens), confidence) => {
            let _ = writeln!(
                out,
                "  {}",
                style::ok(format!(
                    "{tokens} tokens (confidence {})",
                    confidence.unwrap_or("unknown")
                ))
            );
        }
        (None, _) => {
            let _ = writeln!(
                out,
                "  {}",
                style::warn(rec.note.as_deref().unwrap_or("no recommendation"))
            );
        }
    }
    let _ = writeln!(out, "  {}", style::info(&rec.basis));
    match (&report.command.value, &report.command.note) {
        (Some(command), _) => {
            let _ = writeln!(out, "  {}", style::ok(command));
        }
        (None, Some(note)) => {
            let _ = writeln!(out, "  {}", style::info(note));
        }
        (None, None) => {}
    }
    let _ = writeln!(
        out,
        "  {}",
        style::info(
            "Nothing was written. `--set` is the only write this verb makes, and only to \
             .claude/settings.local.json."
        )
    );
    out
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
