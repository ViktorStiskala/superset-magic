//! `ss-magic plugin status` — why the plugin is, or is not, doing anything.
//!
//! Every hook this plugin registers is advisory and fails open, which is the
//! right posture and also the reason this verb has to exist: an ss-magic that
//! has silently stopped acting looks, from inside a session, exactly like an
//! ss-magic that had nothing to do. This is the one place that tells the
//! difference apart, so it enumerates *every* way the plugin can quietly do
//! nothing:
//!
//! - `plugin.enabled` is false in the overlaid configuration,
//! - the harness-side registration is disabled, or was never installed,
//! - git does not report the state tree ignored, so state-writing hooks refuse,
//! - the pinned binary is not installed yet, so every hook runs and does
//!   nothing,
//! - the installed plugin manifest and the bootstrapped binary are different
//!   versions.
//!
//! ## The rule that shapes the whole module
//!
//! **A row that could not be determined says so.** Nothing renders blank and
//! nothing is silently omitted, because a blank row reads as "fine" and sends
//! somebody looking in the wrong place. In JSON the invariant is mechanical: a
//! `null` value always has a non-`null` `note` beside it saying why, and an
//! empty list that might have had entries carries a note too. The text
//! rendering prints `unknown — <reason>` for the same cases.
//!
//! ## Two callers
//!
//! A person reads the text form. An agent reads `--json`, and does so with no
//! injected context at all — a dispatched Explore agent gets no `SessionStart`
//! guidance, so `ss-magic plugin status --json` from Bash is how it discovers
//! the session slug, the conclusions directory and the gate threshold. That
//! makes the JSON keys a contract, not an implementation detail, and it is why
//! the verb exits 0 whenever it produced a report: an agent parsing the output
//! must not have to special-case an exit code that means "the report says
//! something is wrong". Only a usage error exits non-zero.
//!
//! ## Nothing here writes
//!
//! `status` never calls `scratchpad::ensure`, never adds a gitignore rule and
//! never creates the machine-level store. It reads what is there and reports
//! it — a diagnostic that repairs what it is diagnosing cannot be trusted to
//! report what was actually wrong. The one subprocess it runs against the
//! outside world, `claude plugin list --json`, is optional, time-bounded and
//! never fatal; `status` works on a machine with no `claude` on `PATH` at all.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::git;
use crate::plugin::heartbeat::{self, Outcome};
use crate::plugin::{
    bypass, cache, checklist, config, expect_artifact, identity, scratchpad, tmproot,
};
use crate::tui::style;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Version of the `--json` shape. Bumped when a key changes meaning or goes
/// away, so a caller pinned to an older reading can tell.
pub const SCHEMA_VERSION: u32 = 1;

/// The events the shipped `hooks/hooks.json` actually registers.
///
/// `file-changed` is deliberately absent. Its handler exists and is routed, but
/// the manifest entry was removed when the probe behind it was not satisfied,
/// so it never fires in a real session — reporting it as a registered hook
/// would send somebody looking for output from something that is not wired up.
pub const DECLARED_EVENTS: [&str; 5] = [
    "session-start",
    "pre-tool-use",
    "pre-compact",
    "subagent-stop",
    "session-end",
];

/// The plugin's manifest name, which is what a harness registration is matched
/// on. Never the registration id: a marketplace install is `ss-magic@ss-magic`
/// while a stray skills-directory copy is `ss-magic@skills-dir`, and matching
/// the id would miss whichever one was not written into the match.
const MANIFEST_NAME: &str = "ss-magic";

/// The plugin's version pin, relative to `${CLAUDE_PLUGIN_ROOT}`.
const PIN_FILE: &str = "ss-magic.version";

/// The bootstrapped binary, relative to `${CLAUDE_PLUGIN_DATA}`.
const BINARY_REL: &str = "bin/ss-magic";

/// Marker the bootstrap writes after an install has completed, holding the
/// version it installed.
const MARKER_INSTALLED: &str = ".ss-magic-installed";
/// Marker recording that the one-time disclosure has been shown.
const MARKER_DISCLOSED: &str = ".ss-magic-disclosed";
/// Marker recording that this platform has no published release binary; it
/// holds the `uname` signature the bootstrap saw.
const MARKER_UNSUPPORTED: &str = ".ss-magic-unsupported";

/// The file the bootstrap writes the resolved `${CLAUDE_PLUGIN_DATA}` into, so
/// a process that never receives that variable can still find the data
/// directory. Lives in the private per-machine temporary root; the shell side
/// spells the same name.
const DATA_ROOT_FILE: &str = "data-root";

/// How long `claude plugin list --json` may take before it is killed. Generous
/// for a local listing, and short enough that a wedged CLI does not make
/// `status` look wedged too.
const HARNESS_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the bootstrapped binary's `--version` may take. It short-circuits
/// before the update gate and before the TUI, so this is one fast spawn.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a bounded subprocess is checked for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

// ── Shared shapes ─────────────────────────────────────────────────────────────

/// A value that may not be determinable, carrying where it came from and — when
/// it is absent — why.
///
/// This is the module's whole "could not determine" discipline in one type: if
/// `value` is `None` then `note` is `Some`, always, so no consumer ever sees a
/// bare null it has to guess about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Field {
    pub value: Option<String>,
    pub source: Option<String>,
    pub note: Option<String>,
}

impl Field {
    /// A determined value and where it came from.
    fn found(value: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            value: Some(value.into()),
            source: Some(source.into()),
            note: None,
        }
    }

    /// An undetermined value and why.
    fn missing(note: impl Into<String>) -> Self {
        Self {
            value: None,
            source: None,
            note: Some(note.into()),
        }
    }

    /// How this renders on one text row.
    fn render(&self) -> String {
        match (&self.value, &self.note) {
            (Some(v), _) => match &self.source {
                Some(s) => format!("{v}   ({s})"),
                None => v.clone(),
            },
            (None, Some(note)) => format!("unknown — {note}"),
            // Unreachable by construction; rendered rather than panicked so a
            // future constructor that forgets a note degrades to a visible
            // oddity instead of killing the diagnostic.
            (None, None) => "unknown — no reason recorded".to_string(),
        }
    }
}

/// A located directory: the path plus the same provenance a [`Field`] carries.
/// Kept apart from `Field` because callers need the `PathBuf` to join onto.
#[derive(Debug, Clone)]
pub struct Located {
    pub path: Option<PathBuf>,
    pub source: Option<String>,
    pub note: Option<String>,
}

impl Located {
    fn found(path: PathBuf, source: impl Into<String>) -> Self {
        Self {
            path: Some(path),
            source: Some(source.into()),
            note: None,
        }
    }

    fn missing(note: impl Into<String>) -> Self {
        Self {
            path: None,
            source: None,
            note: Some(note.into()),
        }
    }

    fn as_field(&self) -> Field {
        Field {
            value: self.path.as_ref().map(|p| p.display().to_string()),
            source: self.source.clone(),
            note: self.note.clone(),
        }
    }
}

// ── The report ────────────────────────────────────────────────────────────────

/// Everything `status` has to say, in the order a person reads it.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// [`SCHEMA_VERSION`].
    pub schema: u32,
    /// The version of the `ss-magic` binary that produced this report — which
    /// is not necessarily the bootstrapped one; see [`Versions`].
    pub tool_version: String,
    /// The directory the report was produced for.
    pub cwd: String,
    pub repo: Repo,
    pub identity: IdentityRow,
    pub enablement: Enablement,
    pub state_tree: StateTree,
    pub gate: Gate,
    pub bootstrap: Bootstrap,
    pub versions: Versions,
    pub hooks: Hooks,
    /// Every reason the plugin might currently be doing nothing, in plain
    /// sentences. Empty when nothing is blocking it — which is the only case
    /// where an empty list here means what it looks like it means.
    pub problems: Vec<String>,
}

/// The git repository the report is about.
#[derive(Debug, Clone, Serialize)]
pub struct Repo {
    pub root: Option<String>,
    /// The main checkout, which is where `plugin.enabled` is read from even
    /// when this is a linked worktree.
    pub main_checkout_root: Option<String>,
    pub is_worktree: Option<bool>,
    pub note: Option<String>,
}

/// The `<repo>-<branch>` session identity that names the scratchpad's session
/// directory.
#[derive(Debug, Clone, Serialize)]
pub struct IdentityRow {
    pub slug: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub note: Option<String>,
}

/// Enablement, which has two independent layers — and either one alone can
/// silence the plugin.
#[derive(Debug, Clone, Serialize)]
pub struct Enablement {
    /// True only when both layers are on. `None` when the harness layer could
    /// not be determined, because "probably" is not an answer this verb gives.
    pub acting: Option<bool>,
    pub ss_magic: SsMagicLayer,
    pub harness: HarnessLayer,
}

/// ss-magic's own layer: the overlaid `plugin.enabled`, which is the single
/// behavioral switch — hooks no-op when it is false no matter how the harness
/// loaded the plugin.
#[derive(Debug, Clone, Serialize)]
pub struct SsMagicLayer {
    pub enabled: bool,
    /// Which checkout's `magic.json` + `magic.local.json` this came from. It is
    /// the MAIN checkout even in a linked worktree, because a worktree's own
    /// copy is one of the files a forward sync overwrites.
    pub source_root: Option<String>,
    pub note: Option<String>,
}

/// The harness's layer: whether Claude Code loaded a registration named
/// `ss-magic`, and whether that registration is enabled.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessLayer {
    /// True when at least one matching registration is enabled. `None` when the
    /// harness could not be asked at all.
    pub enabled: Option<bool>,
    /// Every registration whose manifest name is `ss-magic`. More than one is
    /// possible — a marketplace install and a shadowed skills-directory copy
    /// both appear — and all of them are listed rather than collapsed.
    pub registrations: Vec<Registration>,
    pub note: Option<String>,
}

/// One `claude plugin list --json` entry, as reported.
#[derive(Debug, Clone, Serialize)]
pub struct Registration {
    pub id: String,
    /// The manifest name, taken from the id's prefix.
    pub name: String,
    pub scope: Option<String>,
    pub enabled: Option<bool>,
    pub version: Option<String>,
    pub install_path: Option<String>,
    pub project_path: Option<String>,
    /// Whatever the harness reported, verbatim. Never interpreted here — these
    /// are the harness's words about its own state, and paraphrasing them would
    /// lose the detail somebody needs.
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

/// The `.superset/.magic/` state tree, and whether hooks may write to it.
#[derive(Debug, Clone, Serialize)]
pub struct StateTree {
    pub root: Option<String>,
    pub exists: Option<bool>,
    /// Whether git reports the tree ignored. Every state-writing hook refuses
    /// outright while this is false, so a `false` here is as silencing as
    /// `enabled: false`.
    pub ignored: Option<bool>,
    pub directories: StateDirs,
    pub note: Option<String>,
}

/// The paths inside the state tree an agent may need to name.
#[derive(Debug, Clone, Serialize)]
pub struct StateDirs {
    pub sessions: Option<String>,
    /// `sessions/<slug>` — the current session's own directory.
    pub session: Option<String>,
    pub conclusions: Option<String>,
    pub bypass: Option<String>,
    pub expect_artifact: Option<String>,
    /// `current.json`, the pointer naming the active session.
    pub pointer: Option<String>,
    /// `checklist.json`, the pointer to the active operator checklist.
    pub checklist_pointer: Option<String>,
}

/// The page-fault gate's resolved tunables.
#[derive(Debug, Clone, Serialize)]
pub struct Gate {
    pub threshold_lines: u32,
    pub inline_byte_budget: u32,
    pub exemptions: Vec<String>,
    /// The checkout these came from — this worktree's own, unlike `enabled`.
    pub source_root: Option<String>,
    pub note: Option<String>,
}

/// The `SessionStart` bootstrap's state: what is pinned, what is installed, and
/// how the last attempt ended.
#[derive(Debug, Clone, Serialize)]
pub struct Bootstrap {
    /// `${CLAUDE_PLUGIN_ROOT}` — the installed plugin tree.
    pub plugin_root: Field,
    /// The version literal in `ss-magic.version`, which is what the bootstrap
    /// installs.
    pub pin: Field,
    /// `${CLAUDE_PLUGIN_DATA}` — where the binary is installed to.
    pub data_dir: Field,
    pub binary: Binary,
    pub outcome: BootstrapOutcome,
    pub markers: Markers,
}

/// The bootstrapped binary itself.
#[derive(Debug, Clone, Serialize)]
pub struct Binary {
    pub path: Option<String>,
    pub exists: Option<bool>,
    pub version: Option<String>,
    pub note: Option<String>,
}

/// How the last bootstrap ended.
///
/// The bootstrap is a shell script that leaves no heartbeat row of its own, so
/// the outcome is read from the durable markers it writes into
/// `${CLAUDE_PLUGIN_DATA}` — which is exactly what the script itself consults
/// on the next run to decide whether to reinstall.
#[derive(Debug, Clone, Serialize)]
pub struct BootstrapOutcome {
    /// One of `installed`, `stale`, `unmarked`, `never-run`,
    /// `unsupported-platform`, `unknown`.
    pub state: &'static str,
    /// What that means here, in a sentence.
    pub detail: String,
}

/// The bootstrap's markers, verbatim.
#[derive(Debug, Clone, Serialize)]
pub struct Markers {
    /// The version the last completed install wrote.
    pub installed: Option<String>,
    /// The version at which the one-time disclosure was shown.
    pub disclosed: Option<String>,
    /// The `uname` signature the bootstrap refused to install for.
    pub unsupported: Option<String>,
    pub note: Option<String>,
}

/// The version picture across the three places a version lives.
#[derive(Debug, Clone, Serialize)]
pub struct Versions {
    /// The installed plugin manifest's version, from the harness registration.
    pub manifest: Option<String>,
    /// `ss-magic.version` — what the bootstrap intends to install.
    pub pin: Option<String>,
    /// What is actually installed under `${CLAUDE_PLUGIN_DATA}`.
    pub binary: Option<String>,
    /// The binary running this command, which may be a developer's own build
    /// rather than the bootstrapped one.
    pub cli: String,
    /// `aligned`, `binary-behind`, `binary-ahead`, `differ`, or `unknown`.
    pub drift: &'static str,
    pub detail: String,
}

/// What the hooks have actually been doing, from the heartbeat log.
#[derive(Debug, Clone, Serialize)]
pub struct Hooks {
    pub heartbeat: Field,
    /// `this-worktree` or `machine`.
    pub scope: &'static str,
    /// Rows considered, after the scope filter.
    pub rows: usize,
    pub events: Vec<EventStatus>,
    pub note: Option<String>,
}

/// One event's history.
#[derive(Debug, Clone, Serialize)]
pub struct EventStatus {
    pub event: String,
    /// Whether the shipped manifest registers this event. A `false` here with
    /// rows behind it means the log predates a manifest change.
    pub declared: bool,
    pub last_fired_at: Option<String>,
    /// `ok`, `no-op` or `error`.
    pub last_outcome: Option<String>,
    /// The stable class behind the last row — `disabled`, `not-ignored`,
    /// `handler-error`, … Present on anything that was not a plain success.
    pub last_reason: Option<String>,
    pub last_detail: Option<String>,
    pub counts: Counts,
    pub note: Option<String>,
}

/// Outcome counts, keyed exactly as the heartbeat spells them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub ok: usize,
    #[serde(rename = "no-op")]
    pub no_op: usize,
    pub error: usize,
}

// ── Inputs ────────────────────────────────────────────────────────────────────

/// Everything [`collect`] would otherwise read from the process environment.
///
/// Passed in rather than read inside, on the same reasoning as
/// `tmproot::resolve_root_at`: the tests then drive real behavior against
/// tempdirs instead of depending on the developer's own `$HOME`, installed
/// plugins, or app-data directory — and, because these are process-global,
/// without two parallel tests fighting over an environment variable.
pub struct Inputs {
    /// The directory the report is about.
    pub cwd: PathBuf,
    /// This binary's version.
    pub tool_version: String,
    /// The machine-level heartbeat store, or `None` when the platform has no
    /// resolvable application data directory.
    pub store: Option<PathBuf>,
    /// `${CLAUDE_PLUGIN_ROOT}`.
    pub plugin_root: Located,
    /// `${CLAUDE_PLUGIN_DATA}`.
    pub data_dir: Located,
    /// Whether to report every heartbeat row rather than only this worktree's.
    pub all: bool,
}

/// The two things `status` learns by talking to something outside itself.
pub struct Probes<'a> {
    /// The harness listing, already fetched or already explained away.
    pub harness: HarnessListing,
    /// `<binary> --version`, called only when the binary is actually there.
    /// `Err` carries the reason, which becomes the row's note.
    pub binary_version: &'a dyn Fn(&Path) -> std::result::Result<String, String>,
}

/// What `claude plugin list --json` produced.
#[derive(Debug, Clone)]
pub enum HarnessListing {
    /// The registrations it reported — every plugin, not only ours; the match
    /// on manifest name happens in [`collect`].
    Loaded(Vec<Registration>),
    /// It could not be asked, or could not be understood. The string is the
    /// reason, and it becomes the harness row's note verbatim.
    Unavailable(String),
}

// ── Reading the harness ───────────────────────────────────────────────────────

/// One `claude plugin list --json` entry as the harness writes it. Unknown keys
/// are ignored and every field is optional, so a harness that adds or renames
/// something does not turn the whole listing into "unavailable".
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRegistration {
    id: Option<String>,
    name: Option<String>,
    scope: Option<String>,
    enabled: Option<bool>,
    version: Option<String>,
    install_path: Option<String>,
    project_path: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    notes: Vec<String>,
}

/// The listing's outer shape. It is a bare top-level array, but an object with
/// a `plugins` key appears in some documentation, so both are accepted — the
/// cost is one enum and the alternative is reporting a working harness as
/// unreadable.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawListing {
    Array(Vec<RawRegistration>),
    Wrapped { plugins: Vec<RawRegistration> },
}

/// Parse a listing. `Err` carries the reason, ready to be a note.
pub fn parse_listing(text: &str) -> std::result::Result<Vec<Registration>, String> {
    if text.trim().is_empty() {
        return Err("`claude plugin list --json` produced no output".to_string());
    }
    let raw: RawListing = serde_json::from_str(text).map_err(|e| {
        format!(
            "`claude plugin list --json` did not return readable JSON ({e}); \
             first bytes were: {}",
            snippet(text)
        )
    })?;
    let entries = match raw {
        RawListing::Array(entries) => entries,
        RawListing::Wrapped { plugins } => plugins,
    };

    let mut out: Vec<Registration> = Vec::new();
    for entry in entries {
        // An entry with neither an id nor a name cannot be matched against
        // anything, and inventing a placeholder would make it look like a real
        // registration.
        let id = match (&entry.id, &entry.name) {
            (Some(id), _) => id.clone(),
            (None, Some(name)) => name.clone(),
            (None, None) => continue,
        };
        // The manifest name is the id up to the first `@`; `ss-magic@ss-magic`
        // and `ss-magic@skills-dir` are both named `ss-magic`. An explicit
        // `name` field, if the harness ever grows one, wins.
        let name = entry
            .name
            .clone()
            .unwrap_or_else(|| id.split('@').next().unwrap_or(&id).to_string());
        let reg = Registration {
            id,
            name,
            scope: entry.scope,
            enabled: entry.enabled,
            version: entry.version,
            install_path: entry.install_path,
            project_path: entry.project_path,
            errors: entry.errors,
            notes: entry.notes,
        };
        // Duplicate ids appear once per project path, so the pair is the
        // identity; the same pair twice is the harness repeating itself.
        if out
            .iter()
            .any(|r| r.id == reg.id && r.project_path == reg.project_path)
        {
            continue;
        }
        out.push(reg);
    }
    Ok(out)
}

/// The first line of `text`, truncated, for an error message.
fn snippet(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 120 {
        let cut: String = line.chars().take(117).collect();
        format!("{cut}...")
    } else {
        line.to_string()
    }
}

/// Ask the harness what it has loaded.
///
/// Optional by construction: no `claude` on `PATH`, a `claude` that hangs, and a
/// `claude` that prints something unparseable are all ordinary results here, and
/// each becomes a note rather than a failure. The exit code is deliberately
/// ignored — the CLI has been observed exiting 0 on total failure, so the output
/// is the only signal worth reading.
pub fn probe_harness() -> HarnessListing {
    let mut cmd = Command::new("claude");
    cmd.args(["plugin", "list", "--json"]);
    match run_bounded(cmd, HARNESS_TIMEOUT, "claude plugin list --json") {
        Ok(text) => match parse_listing(&text) {
            Ok(regs) => HarnessListing::Loaded(regs),
            Err(reason) => HarnessListing::Unavailable(reason),
        },
        Err(reason) => HarnessListing::Unavailable(reason),
    }
}

/// Ask a binary for its version.
pub fn probe_binary_version(bin: &Path) -> std::result::Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    let text = run_bounded(
        cmd,
        VERSION_TIMEOUT,
        &format!("{} --version", bin.display()),
    )?;
    parse_version_line(&text)
        .ok_or_else(|| format!("`{} --version` printed nothing usable", bin.display()))
}

/// `ss-magic 0.10.0` → `0.10.0`. The last whitespace-separated token of the
/// first line, which is the shape `version_line` produces.
pub fn parse_version_line(text: &str) -> Option<String> {
    text.lines()
        .next()?
        .split_whitespace()
        .next_back()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// Run a command with its output captured, killing it if it outstays
/// `timeout`.
///
/// Output goes to a temporary file rather than a pipe: a pipe that fills while
/// nothing is draining it deadlocks the child, and the whole point of the
/// timeout is to guarantee this returns. Polling `try_wait` keeps the child
/// killable, which a blocking `wait_with_output` on a background thread would
/// not be.
fn run_bounded(
    mut cmd: Command,
    timeout: Duration,
    what: &str,
) -> std::result::Result<String, String> {
    let sink = tempfile::NamedTempFile::new()
        .map_err(|e| format!("could not create a temporary file for `{what}`: {e}"))?;
    let handle = sink
        .reopen()
        .map_err(|e| format!("could not open a temporary file for `{what}`: {e}"))?;

    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(handle))
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("`{what}` could not be run: it is not on PATH"));
        }
        Err(e) => return Err(format!("`{what}` could not be run: {e}")),
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => return Err(format!("`{what}` could not be waited for: {e}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "`{what}` did not finish within {} seconds",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    std::fs::read_to_string(sink.path())
        .map_err(|e| format!("could not read the output of `{what}`: {e}"))
}

// ── Locating the plugin's own directories ─────────────────────────────────────

/// Where the bootstrap installs to.
///
/// Three sources, in order of how much they are trusted:
///
/// 1. `${CLAUDE_PLUGIN_DATA}` itself, which hook and MCP processes receive.
/// 2. The `data-root` file the bootstrap writes into the private per-machine
///    temporary root — the bridge for a process that never sees that variable,
///    which is every invocation made through the Bash tool.
/// 3. The documented layout, `<config dir>/plugins/data/ss-magic-ss-magic`.
///
/// The temporary root is only *read*, never created: a diagnostic that
/// scaffolds directories to answer a question about scaffolding is not a
/// diagnostic.
pub fn locate_data_dir() -> Located {
    if let Some(dir) = non_empty_env("CLAUDE_PLUGIN_DATA") {
        return Located::found(PathBuf::from(dir), "${CLAUDE_PLUGIN_DATA}");
    }

    if let Some(path) = read_data_root_pointer() {
        return Located::found(
            PathBuf::from(path.0),
            format!("the bootstrap's data-root pointer at {}", path.1.display()),
        );
    }

    let config = non_empty_env("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| non_empty_env("HOME").map(|h| PathBuf::from(h).join(".claude")));
    match config {
        Some(config) => Located::found(
            config
                .join("plugins")
                .join("data")
                .join(format!("{MANIFEST_NAME}-{MANIFEST_NAME}")),
            "the documented layout (no ${CLAUDE_PLUGIN_DATA}, no data-root pointer)",
        ),
        None => Located::missing(
            "no ${CLAUDE_PLUGIN_DATA}, no data-root pointer from a bootstrap run, \
             and neither $CLAUDE_CONFIG_DIR nor $HOME is set"
                .to_string(),
        ),
    }
}

/// Read the bootstrap's `data-root` pointer without creating anything.
///
/// The temporary root is `<base>/ss-magic-plugin/<identifier>`, where the
/// identifier is derived from `$HOME`; both `/tmp` and `$TMPDIR` are tried,
/// matching the order the bootstrap writes in. Returns the pointer's contents
/// and the file it was read from, so the source can be named.
fn read_data_root_pointer() -> Option<(String, PathBuf)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let id = tmproot::identifier(&home);
    for base in [PathBuf::from("/tmp"), std::env::temp_dir()] {
        let path = base
            .join(tmproot::NAMESPACE_DIR)
            .join(&id)
            .join(DATA_ROOT_FILE);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let line = text.lines().next().unwrap_or("").trim();
            if !line.is_empty() {
                return Some((line.to_string(), path));
            }
        }
    }
    None
}

/// Where the installed plugin tree is.
///
/// `${CLAUDE_PLUGIN_ROOT}` reaches hook processes but not the Bash tool, so the
/// second source is the harness's own registration, whose `installPath` is that
/// same directory.
pub fn locate_plugin_root(harness: &HarnessListing) -> Located {
    if let Some(dir) = non_empty_env("CLAUDE_PLUGIN_ROOT") {
        return Located::found(PathBuf::from(dir), "${CLAUDE_PLUGIN_ROOT}");
    }
    if let HarnessListing::Loaded(regs) = harness {
        if let Some(path) = regs
            .iter()
            .filter(|r| r.name == MANIFEST_NAME)
            .find_map(|r| r.install_path.clone())
        {
            return Located::found(
                PathBuf::from(path),
                "the harness registration's installPath",
            );
        }
    }
    Located::missing(
        "no ${CLAUDE_PLUGIN_ROOT} in this process, and no installPath from the \
         harness registration",
    )
}

/// A non-empty environment variable, or `None`.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

// ── Collecting ────────────────────────────────────────────────────────────────

/// Build the whole report. Never fails: every probe below degrades to a note.
pub fn collect(inputs: &Inputs, probes: &Probes) -> Status {
    let mut problems: Vec<String> = Vec::new();

    let repo_root = git::cwd_repo_root(&inputs.cwd).ok();
    let main_root = repo_root
        .as_ref()
        .and_then(|r| git::main_checkout_root(r).ok());

    let repo = Repo {
        root: repo_root.as_ref().map(|p| p.display().to_string()),
        main_checkout_root: main_root.as_ref().map(|p| p.display().to_string()),
        is_worktree: repo_root.as_ref().and_then(|r| git::is_worktree(r).ok()),
        note: match (&repo_root, &main_root) {
            (None, _) => Some(format!(
                "{} is not inside a git repository, so there is no worktree for \
                 the plugin to act on — every hook no-ops here",
                inputs.cwd.display()
            )),
            (Some(_), None) => Some(
                "git could not report the main checkout, so `plugin.enabled` was \
                 resolved against this checkout instead"
                    .to_string(),
            ),
            _ => None,
        },
    };
    if repo_root.is_none() {
        problems.push(format!(
            "{} is not inside a git repository — the plugin resolves no session \
             identity and does nothing at all here.",
            inputs.cwd.display()
        ));
    }

    let identity = collect_identity(&inputs.cwd, repo_root.is_some());
    let (enablement, harness_regs) = collect_enablement(
        repo_root.as_deref().unwrap_or(&inputs.cwd),
        main_root.as_deref().or(repo_root.as_deref()),
        &probes.harness,
        &mut problems,
    );

    let cfg = config::resolve(repo_root.as_deref().unwrap_or(&inputs.cwd));
    let gate = Gate {
        threshold_lines: cfg.gate.threshold_lines,
        inline_byte_budget: cfg.gate.inline_byte_budget,
        exemptions: cfg.gate.exemptions.clone(),
        source_root: repo_root.as_ref().map(|p| p.display().to_string()),
        note: match repo_root {
            Some(_) => None,
            None => Some(
                "no repository here, so these are the binary's built-in defaults \
                 rather than anything configured"
                    .to_string(),
            ),
        },
    };

    let state_tree = collect_state_tree(repo_root.as_deref(), &identity, &mut problems);
    let bootstrap = collect_bootstrap(inputs, probes, &mut problems);
    let versions = collect_versions(inputs, &bootstrap, &harness_regs, &mut problems);
    let hooks = collect_hooks(inputs, repo_root.as_deref());

    if !enablement.ss_magic.enabled {
        problems.push(format!(
            "`plugin.enabled` is not true in {} — every hook no-ops, whatever the \
             harness has loaded. Turn it on with `ss-magic plugin enable`.",
            enablement
                .ss_magic
                .source_root
                .clone()
                .unwrap_or_else(|| "the overlaid configuration".to_string())
        ));
    }

    Status {
        schema: SCHEMA_VERSION,
        tool_version: inputs.tool_version.clone(),
        cwd: inputs.cwd.display().to_string(),
        repo,
        identity,
        enablement,
        state_tree,
        gate,
        bootstrap,
        versions,
        hooks,
        problems,
    }
}

/// The session slug, or why there is none.
fn collect_identity(cwd: &Path, in_repo: bool) -> IdentityRow {
    match identity::resolve(cwd) {
        Some(id) => IdentityRow {
            slug: Some(id.slug),
            repo: Some(id.repo),
            branch: Some(id.branch),
            note: None,
        },
        None => IdentityRow {
            slug: None,
            repo: None,
            branch: None,
            note: Some(if in_repo {
                "this repository has no commit and no branch to derive a session \
                 identity from (an unborn HEAD)"
                    .to_string()
            } else {
                "not inside a git repository, so there is no <repo>-<branch> \
                 identity to resolve"
                    .to_string()
            }),
        },
    }
}

/// Both enablement layers, and the registrations behind the harness one.
fn collect_enablement(
    cwd_root: &Path,
    enabled_root: Option<&Path>,
    listing: &HarnessListing,
    problems: &mut Vec<String>,
) -> (Enablement, Vec<Registration>) {
    let cfg = config::resolve(cwd_root);
    let ss_magic = SsMagicLayer {
        enabled: cfg.enabled,
        source_root: enabled_root.map(|p| p.display().to_string()),
        note: match enabled_root {
            Some(_) => None,
            None => Some(
                "no git repository here, so there is no magic.json overlay to read \
                 `plugin.enabled` from; it reads as off"
                    .to_string(),
            ),
        },
    };

    let (harness, mine) = match listing {
        HarnessListing::Unavailable(reason) => (
            HarnessLayer {
                enabled: None,
                registrations: Vec::new(),
                note: Some(reason.clone()),
            },
            Vec::new(),
        ),
        HarnessListing::Loaded(regs) => {
            let mine: Vec<Registration> = regs
                .iter()
                .filter(|r| r.name == MANIFEST_NAME)
                .cloned()
                .collect();
            if mine.is_empty() {
                (
                    HarnessLayer {
                        enabled: Some(false),
                        registrations: Vec::new(),
                        note: Some(format!(
                            "the harness has loaded no plugin named `{MANIFEST_NAME}` \
                             — it is not installed on this machine. Install it with \
                             `claude plugin install {MANIFEST_NAME}@{MANIFEST_NAME}`"
                        )),
                    },
                    Vec::new(),
                )
            } else {
                // Unknown beats false: an entry that reports no `enabled` field
                // is not evidence that it is off.
                let any_enabled = mine.iter().any(|r| r.enabled == Some(true));
                let any_unknown = mine.iter().any(|r| r.enabled.is_none());
                let enabled = if any_enabled {
                    Some(true)
                } else if any_unknown {
                    None
                } else {
                    Some(false)
                };
                let note = if any_unknown && !any_enabled {
                    Some(
                        "the harness reported no `enabled` flag for this \
                         registration, so whether it is loaded cannot be told from \
                         the listing"
                            .to_string(),
                    )
                } else {
                    None
                };
                (
                    HarnessLayer {
                        enabled,
                        registrations: mine.clone(),
                        note,
                    },
                    mine,
                )
            }
        }
    };

    match harness.enabled {
        Some(false) if harness.registrations.is_empty() => problems.push(format!(
            "no `{MANIFEST_NAME}` plugin is registered with the harness — no hook \
             is wired up, so nothing fires regardless of `plugin.enabled`."
        )),
        Some(false) => {
            let which: Vec<String> = harness
                .registrations
                .iter()
                .map(|r| {
                    format!(
                        "{} (scope {})",
                        r.id,
                        r.scope.clone().unwrap_or_else(|| "unknown".to_string())
                    )
                })
                .collect();
            problems.push(format!(
                "the harness-side registration is disabled: {} — no ss-magic hook \
                 fires, even though this is a separate switch from `plugin.enabled`.",
                which.join(", ")
            ));
        }
        None => problems.push(format!(
            "the harness-side registration could not be determined: {}. Whether \
             hooks are wired up at all is therefore unknown.",
            harness
                .note
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string())
        )),
        Some(true) => {}
    }

    // Whatever the harness reports about its own state goes straight through.
    for reg in &harness.registrations {
        for err in &reg.errors {
            problems.push(format!("the harness reports for {}: {err}", reg.id));
        }
        for note in &reg.notes {
            problems.push(format!("the harness notes for {}: {note}", reg.id));
        }
    }

    let acting = match harness.enabled {
        Some(harness_on) => Some(ss_magic.enabled && harness_on),
        None => None,
    };

    (
        Enablement {
            acting,
            ss_magic,
            harness,
        },
        mine,
    )
}

/// The state tree, and whether git will let hooks write into it.
fn collect_state_tree(
    repo_root: Option<&Path>,
    identity: &IdentityRow,
    problems: &mut Vec<String>,
) -> StateTree {
    let Some(root) = repo_root else {
        return StateTree {
            root: None,
            exists: None,
            ignored: None,
            directories: StateDirs {
                sessions: None,
                session: None,
                conclusions: None,
                bypass: None,
                expect_artifact: None,
                pointer: None,
                checklist_pointer: None,
            },
            note: Some(
                "no git repository here, so there is no worktree to hold a state \
                 tree"
                    .to_string(),
            ),
        };
    };

    let state_root = root.join(scratchpad::STATE_REL);
    // The directory-only query: a `.superset/.magic/` rule matches it even
    // before the directory exists, whereas the slash-less form is treated as a
    // file and misses. The rules-only probe matches what the hook wrapper asks,
    // so this row agrees with the decision the hooks actually make.
    let query = format!("{}/", scratchpad::STATE_REL);
    let (ignored, note) = match git::is_ignored_no_index_str(root, &query) {
        Ok(value) => (Some(value), None),
        Err(e) => (
            None,
            Some(format!("could not ask git whether {query} is ignored: {e}")),
        ),
    };

    match ignored {
        Some(false) => problems.push(format!(
            "git does not ignore {query} — every state-writing hook refuses to \
             write anything while that is true. `ss-magic plugin enable` adds the \
             rule."
        )),
        None => problems.push(format!(
            "whether {query} is ignored could not be determined, and hooks treat \
             an unanswered question as a refusal — so they may be writing nothing."
        )),
        Some(true) => {}
    }

    let sessions = state_root.join("sessions");
    StateTree {
        root: Some(state_root.display().to_string()),
        exists: Some(state_root.is_dir()),
        ignored,
        directories: StateDirs {
            sessions: Some(sessions.display().to_string()),
            session: identity
                .slug
                .as_ref()
                .map(|slug| sessions.join(slug).display().to_string()),
            conclusions: Some(state_root.join(cache::DIR_NAME).display().to_string()),
            bypass: Some(state_root.join(bypass::DIR_NAME).display().to_string()),
            expect_artifact: Some(
                state_root
                    .join(expect_artifact::DIR_NAME)
                    .display()
                    .to_string(),
            ),
            pointer: Some(state_root.join("current.json").display().to_string()),
            checklist_pointer: Some(checklist::pointer_path(root).display().to_string()),
        },
        note: match (&note, identity.slug.as_ref()) {
            (Some(n), _) => Some(n.clone()),
            (None, None) => Some(
                "no session identity resolved, so the per-session directory under \
                 sessions/ cannot be named"
                    .to_string(),
            ),
            _ => None,
        },
    }
}

/// The bootstrap's whole state: pin, paths, binary, markers, outcome.
fn collect_bootstrap(inputs: &Inputs, probes: &Probes, problems: &mut Vec<String>) -> Bootstrap {
    let plugin_root = inputs.plugin_root.as_field();
    let data_dir = inputs.data_dir.as_field();

    let pin = match &inputs.plugin_root.path {
        Some(root) => {
            let path = root.join(PIN_FILE);
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let value = text.trim().to_string();
                    if value.is_empty() {
                        Field::missing(format!("{} is empty", path.display()))
                    } else {
                        Field::found(value, path.display().to_string())
                    }
                }
                Err(e) => Field::missing(format!("could not read {}: {e}", path.display())),
            }
        }
        None => Field::missing(format!(
            "the installed plugin tree could not be located, so {PIN_FILE} could not \
             be read — {}",
            inputs
                .plugin_root
                .note
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string())
        )),
    };

    let (binary, markers) = match &inputs.data_dir.path {
        Some(dir) => {
            let bin = dir.join(BINARY_REL);
            let exists = bin.is_file();
            let (version, note) = if exists {
                match (probes.binary_version)(&bin) {
                    Ok(v) => (Some(v), None),
                    Err(reason) => (None, Some(format!("it is there but {reason}"))),
                }
            } else {
                (
                    None,
                    Some(
                        "no binary at this path yet: the SessionStart bootstrap \
                         installs it, and until it has, every ss-magic hook runs \
                         and does nothing"
                            .to_string(),
                    ),
                )
            };
            (
                Binary {
                    path: Some(bin.display().to_string()),
                    exists: Some(exists),
                    version,
                    note,
                },
                Markers {
                    installed: read_marker(dir, MARKER_INSTALLED),
                    disclosed: read_marker(dir, MARKER_DISCLOSED),
                    unsupported: read_marker(dir, MARKER_UNSUPPORTED),
                    note: None,
                },
            )
        }
        None => {
            let why = inputs
                .data_dir
                .note
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string());
            (
                Binary {
                    path: None,
                    exists: None,
                    version: None,
                    note: Some(format!(
                        "the plugin data directory could not be located, so the \
                         binary's path is unknown — {why}"
                    )),
                },
                Markers {
                    installed: None,
                    disclosed: None,
                    unsupported: None,
                    note: Some(format!(
                        "the plugin data directory could not be located, so the \
                         bootstrap's markers could not be read — {why}"
                    )),
                },
            )
        }
    };

    let outcome = derive_outcome(&pin, &binary, &markers, &inputs.data_dir);

    if binary.exists == Some(false) {
        problems.push(format!(
            "the pinned binary is not installed at {} — the hooks fire, resolve no \
             binary and exit silently, so nothing acts. It installs on the next fresh \
             session's SessionStart bootstrap.",
            binary.path.clone().unwrap_or_else(|| "?".to_string())
        ));
    }
    if let Some(signature) = &markers.unsupported {
        problems.push(format!(
            "the bootstrap recorded that no release binary is published for this \
             platform ({signature}) — it installs nothing here and the plugin stays \
             inactive."
        ));
    }

    Bootstrap {
        plugin_root,
        pin,
        data_dir,
        binary,
        outcome,
        markers,
    }
}

/// One bootstrap marker's contents, trimmed. `None` when it is not there — the
/// ordinary state for a machine that has not hit that path.
fn read_marker(dir: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    let value = text.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Classify how the last bootstrap ended, from the markers plus what is
/// actually on disk.
fn derive_outcome(
    pin: &Field,
    binary: &Binary,
    markers: &Markers,
    data_dir: &Located,
) -> BootstrapOutcome {
    if data_dir.path.is_none() {
        return BootstrapOutcome {
            state: "unknown",
            detail: format!(
                "the plugin data directory could not be located, so nothing about \
                 the last bootstrap can be read — {}",
                data_dir
                    .note
                    .clone()
                    .unwrap_or_else(|| "no reason recorded".to_string())
            ),
        };
    }

    if let Some(signature) = &markers.unsupported {
        return BootstrapOutcome {
            state: "unsupported-platform",
            detail: format!(
                "the bootstrap found no published release binary for {signature} and \
                 installed nothing; it says so once and stays quiet afterwards"
            ),
        };
    }

    let bin_path = binary.path.clone().unwrap_or_else(|| "?".to_string());
    match (&markers.installed, binary.exists) {
        (Some(marked), Some(false)) => BootstrapOutcome {
            state: "stale",
            detail: format!(
                "the last bootstrap completed and installed {marked}, but nothing is \
                 at {bin_path} now; the next fresh session reinstalls it"
            ),
        },
        (Some(marked), _) => match &pin.value {
            Some(pin) if pin != marked => BootstrapOutcome {
                state: "stale",
                detail: format!(
                    "the last completed install was {marked}, and the plugin now pins \
                     {pin}; the next fresh session's bootstrap installs the pinned \
                     version"
                ),
            },
            Some(pin) => BootstrapOutcome {
                state: "installed",
                detail: format!("the last bootstrap completed and installed {pin}"),
            },
            None => BootstrapOutcome {
                state: "installed",
                detail: format!(
                    "the last bootstrap completed and installed {marked}; the pin it \
                     was compared against could not be read, so whether that is still \
                     the wanted version is unknown"
                ),
            },
        },
        (None, Some(true)) => BootstrapOutcome {
            state: "unmarked",
            detail: format!(
                "a binary is at {bin_path} but no completion marker is beside it, so \
                 the last install did not finish (or the marker was removed); the \
                 next fresh session reinstalls"
            ),
        },
        (None, _) => BootstrapOutcome {
            state: "never-run",
            detail: match &pin.value {
                Some(pin) => format!(
                    "no bootstrap has completed on this machine: nothing at {bin_path} \
                     and no marker beside it. The next fresh session installs {pin}"
                ),
                None => format!(
                    "no bootstrap has completed on this machine: nothing at {bin_path} \
                     and no marker beside it"
                ),
            },
        },
    }
}

/// Line the three versions up and say which, if either, is behind.
fn collect_versions(
    inputs: &Inputs,
    bootstrap: &Bootstrap,
    registrations: &[Registration],
    problems: &mut Vec<String>,
) -> Versions {
    let manifest = registrations.iter().find_map(|r| r.version.clone());
    let pin = bootstrap.pin.value.clone();
    let binary = bootstrap.binary.version.clone();

    let (drift, detail) = match (&manifest, &binary) {
        (Some(m), Some(b)) if m == b => (
            "aligned",
            format!("the installed plugin manifest and the bootstrapped binary are both {m}"),
        ),
        (Some(m), Some(b)) => match (parse_semver(m), parse_semver(b)) {
            (Some(mv), Some(bv)) if bv < mv => (
                "binary-behind",
                format!(
                    "the installed plugin manifest is {m} but the bootstrapped binary \
                     is {b} — the binary is behind. A plugin update lands the new \
                     manifest first and the binary at the next fresh session's \
                     bootstrap, so this gap is expected and transient rather than an \
                     error; the session that closes it runs with ss-magic's hooks \
                     inert"
                ),
            ),
            (Some(_), Some(_)) => (
                "binary-ahead",
                format!(
                    "the bootstrapped binary is {b} while the installed plugin manifest \
                     is {m} — the binary is ahead, which happens when the plugin was \
                     rolled back but the data directory kept the newer binary"
                ),
            ),
            _ => (
                "differ",
                format!(
                    "the installed plugin manifest says {m} and the bootstrapped binary \
                     says {b}; neither is a plain MAJOR.MINOR.PATCH version, so which \
                     is newer cannot be told"
                ),
            ),
        },
        (None, Some(b)) => (
            "unknown",
            format!(
                "the bootstrapped binary is {b}, but the installed plugin manifest's \
                 version is unknown — {}",
                bootstrap
                    .plugin_root
                    .note
                    .clone()
                    .or_else(|| Some("the harness registration reported none".to_string()))
                    .unwrap_or_default()
            ),
        ),
        (Some(m), None) => (
            "unknown",
            format!(
                "the installed plugin manifest is {m}, but the bootstrapped binary's \
                 version is unknown — {}",
                bootstrap
                    .binary
                    .note
                    .clone()
                    .unwrap_or_else(|| "no reason recorded".to_string())
            ),
        ),
        (None, None) => (
            "unknown",
            format!(
                "neither the installed plugin manifest nor the bootstrapped binary \
                 reported a version — {}",
                bootstrap
                    .binary
                    .note
                    .clone()
                    .unwrap_or_else(|| "no reason recorded".to_string())
            ),
        ),
    };

    if drift == "binary-behind" || drift == "binary-ahead" || drift == "differ" {
        problems.push(format!("version gap: {detail}."));
    }

    Versions {
        manifest,
        pin,
        binary,
        cli: inputs.tool_version.clone(),
        drift,
        detail,
    }
}

/// `1.2.3` → `(1, 2, 3)`. `None` for anything that is not exactly three
/// numbers.
fn parse_semver(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Per-event history from the heartbeat log.
fn collect_hooks(inputs: &Inputs, repo_root: Option<&Path>) -> Hooks {
    let scope = if inputs.all || repo_root.is_none() {
        "machine"
    } else {
        "this-worktree"
    };

    let (heartbeat, rows, note) = match &inputs.store {
        None => (
            Field::missing(
                "this platform has no application data directory, so no heartbeat \
                 log is kept and nothing can be said about what the hooks have done"
                    .to_string(),
            ),
            Vec::new(),
            Some(
                "no heartbeat log to read; every event row below is therefore \
                 unknown rather than empty"
                    .to_string(),
            ),
        ),
        Some(store) => {
            let path = heartbeat::log_path(store);
            match heartbeat::read(store) {
                Ok(rows) if rows.is_empty() && !path.exists() => (
                    Field::found(path.display().to_string(), "machine-level plugin store"),
                    rows,
                    Some(format!(
                        "no heartbeat log at {} yet — no hook has run on this machine",
                        path.display()
                    )),
                ),
                Ok(rows) => (
                    Field::found(path.display().to_string(), "machine-level plugin store"),
                    rows,
                    None,
                ),
                Err(e) => (
                    Field::missing(format!("could not read {}: {e}", path.display())),
                    Vec::new(),
                    Some(format!(
                        "the heartbeat log could not be read, so every event row \
                         below is unknown rather than empty: {e}"
                    )),
                ),
            }
        }
    };

    // Filter to this worktree unless asked for everything. A row's `cwd` is the
    // envelope's, which can be a subdirectory of the root, so the comparison is
    // by prefix rather than by equality — and it is made against the RESOLVED
    // path on both sides. `git rev-parse` hands back a physical path, while a
    // hook envelope's `cwd` can carry a symlinked one (`/var/...` for
    // `/private/var/...` on macOS is the everyday case), so comparing the two
    // as written would drop a worktree's own rows on the floor. Resolution is
    // memoized per distinct `cwd` string: a session contributes many rows and
    // they nearly all name the same directory.
    let mut resolved: HashMap<&str, Option<PathBuf>> = HashMap::new();
    let rows: Vec<&heartbeat::Row> = rows
        .iter()
        .filter(|row| {
            if scope == "machine" {
                return true;
            }
            let Some(root) = repo_root else { return true };
            // A row with no recorded cwd cannot be attributed to a worktree, so
            // it is left out of a worktree-scoped view rather than counted
            // against one it may not belong to.
            let Some(cwd) = row.cwd.as_deref() else {
                return false;
            };
            if Path::new(cwd).starts_with(root) {
                return true;
            }
            resolved
                .entry(cwd)
                .or_insert_with(|| std::fs::canonicalize(cwd).ok())
                .as_ref()
                .is_some_and(|real| real.starts_with(root))
        })
        .collect();

    // Every declared event gets a row whether or not it has ever fired, then any
    // event the log knows about that the manifest no longer declares.
    let mut names: Vec<String> = DECLARED_EVENTS.iter().map(|e| (*e).to_string()).collect();
    for row in &rows {
        if !row.event.is_empty() && !names.iter().any(|n| n == &row.event) {
            names.push(row.event.clone());
        }
    }

    let events = names
        .iter()
        .map(|name| {
            let declared = DECLARED_EVENTS.contains(&name.as_str());
            let mine: Vec<&&heartbeat::Row> =
                rows.iter().filter(|row| &row.event == name).collect();
            let mut counts = Counts::default();
            for row in &mine {
                match row.outcome {
                    Outcome::Ok => counts.ok += 1,
                    Outcome::NoOp => counts.no_op += 1,
                    Outcome::Error => counts.error += 1,
                }
            }
            // The log is appended in order, so the last matching row is the
            // most recent one.
            let last = mine.last();
            EventStatus {
                event: name.clone(),
                declared,
                last_fired_at: last.map(|r| r.at.clone()),
                last_outcome: last.map(|r| outcome_name(r.outcome).to_string()),
                last_reason: last.and_then(|r| r.reason.clone()),
                last_detail: last.and_then(|r| r.detail.clone()),
                counts,
                note: match (last, declared, note.is_some()) {
                    // An unreadable or absent log is reported once, above; the
                    // per-event note repeats it so a row read on its own is not
                    // mistaken for "this never fired".
                    (None, _, true) => {
                        Some("not known: the heartbeat log could not be read".to_string())
                    }
                    (None, true, false) => Some(
                        "no rows recorded — this event has not fired in the scope \
                         shown"
                            .to_string(),
                    ),
                    (_, false, _) => Some(
                        "the shipped manifest does not register this event, so these \
                         rows are from an older plugin version"
                            .to_string(),
                    ),
                    _ => None,
                },
            }
        })
        .collect();

    Hooks {
        heartbeat,
        scope,
        rows: rows.len(),
        events,
        note,
    }
}

/// The wire spelling of an outcome, matching what the log itself stores.
fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Ok => "ok",
        Outcome::NoOp => "no-op",
        Outcome::Error => "error",
    }
}

// ── The verb ──────────────────────────────────────────────────────────────────

/// Usage for `status`.
const STATUS_USAGE: &str = "\
Usage: ss-magic plugin status [OPTIONS]

Report what the plugin sees, and why it is or is not acting: both enablement
layers, the state tree, the gate's settings, the bootstrapped binary, and what
each hook has actually done. Reads only — nothing is created or changed.

Options:
  --all               Report every heartbeat row, not just this worktree's
  --json              Machine-readable output
  -h, --help          This text

Exits 0 whenever a report was produced, including one full of problems, so a
script can parse the output without special-casing the exit code.";

/// `ss-magic plugin status` — a human verb, so usage problems go to stderr with
/// a non-zero exit.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut json = false;
    let mut all = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{STATUS_USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--json" => json = true,
            "--all" => all = true,
            other => {
                eprintln!(
                    "{}",
                    style::err(format!("error: unexpected argument `{other}`"))
                );
                eprintln!("{STATUS_USAGE}");
                return Ok(ExitCode::from(2));
            }
        }
    }

    let cwd = std::env::current_dir().context("reading the current directory")?;
    let harness = probe_harness();
    let inputs = Inputs {
        cwd,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        // `store_path`'s creating sibling is deliberately avoided: `store_dir`
        // would create the machine-level store, and a status verb must not
        // scaffold the thing it is reporting on. An absent store is reported as
        // "no hook has run", which is what it means.
        store: heartbeat_store_if_present(),
        plugin_root: locate_plugin_root(&harness),
        data_dir: locate_data_dir(),
        all,
    };
    let probes = Probes {
        harness,
        binary_version: &probe_binary_version,
    };

    let status = collect(&inputs, &probes);
    emit(&status, json)?;
    Ok(ExitCode::SUCCESS)
}

/// The machine-level store, but only if it is already there.
///
/// `heartbeat::store_dir` creates it, which is right for a hook about to append
/// and wrong for a diagnostic: creating the store would make "no hook has ever
/// run here" indistinguishable from "the store exists and is empty" on the very
/// next run.
fn heartbeat_store_if_present() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "ss-magic")?;
    let dir = dirs.data_dir().join(heartbeat::STORE_SUBDIR);
    dir.is_dir().then_some(dir)
}

/// Render a report, as JSON or for a person.
fn emit(status: &Status, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(status)?);
        return Ok(());
    }
    let mut out = String::new();
    render_text(&mut out, status);
    print!("{out}");
    Ok(())
}

/// Width the label column is padded to, so the values line up.
const LABEL_WIDTH: usize = 22;

/// The text a row shows when there is no value: never blank, always the
/// reason. One function so no row can accidentally grow its own spelling of
/// "we don't know".
fn unknown(note: Option<&str>) -> String {
    style::warn(format!(
        "unknown — {}",
        note.unwrap_or("no reason recorded")
    ))
}

/// One `label: value` row.
fn row(out: &mut String, label: &str, value: impl AsRef<str>) {
    let _ = writeln!(out, "  {label:<LABEL_WIDTH$}{}", value.as_ref());
}

/// A row whose value may be unknown.
fn row_opt(out: &mut String, label: &str, value: Option<&str>, note: Option<&str>) {
    match value {
        Some(v) => row(out, label, v),
        None => row(out, label, unknown(note)),
    }
}

/// `yes`/`no`, or an explained unknown.
fn row_bool(out: &mut String, label: &str, value: Option<bool>, note: Option<&str>) {
    match value {
        Some(true) => row(out, label, style::ok("yes")),
        Some(false) => row(out, label, style::warn("no")),
        None => row(out, label, unknown(note)),
    }
}

/// The whole report, for a person. Writes into a buffer rather than printing,
/// so the layout is testable without capturing the process's stdout — the same
/// separation `main.rs` keeps between its event stream and `print_event`.
fn render_text(out: &mut String, status: &Status) {
    let _ = writeln!(
        out,
        "{}",
        style::header(format!(
            "ss-magic plugin status (ss-magic {})",
            status.tool_version
        ))
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("Enablement"));
    row_bool(
        out,
        "plugin.enabled",
        Some(status.enablement.ss_magic.enabled),
        status.enablement.ss_magic.note.as_deref(),
    );
    row_opt(
        out,
        "  read from",
        status.enablement.ss_magic.source_root.as_deref(),
        status.enablement.ss_magic.note.as_deref(),
    );
    if status.enablement.harness.registrations.is_empty() {
        // "none installed" and "could not be asked" are different answers and
        // must not render the same: the first is a fact, the second is a gap.
        let note = status.enablement.harness.note.as_deref();
        match status.enablement.harness.enabled {
            Some(_) => row(
                out,
                "harness registration",
                style::warn(note.unwrap_or("none installed")),
            ),
            None => row(out, "harness registration", unknown(note)),
        }
    } else {
        for reg in &status.enablement.harness.registrations {
            let state = match reg.enabled {
                Some(true) => style::ok("enabled"),
                Some(false) => style::warn("DISABLED"),
                None => style::warn("enabled flag not reported"),
            };
            row(
                out,
                "harness registration",
                format!(
                    "{} — scope {}, {}{}",
                    reg.id,
                    reg.scope.clone().unwrap_or_else(|| "unknown".to_string()),
                    state,
                    reg.version
                        .as_ref()
                        .map(|v| format!(", version {v}"))
                        .unwrap_or_default()
                ),
            );
            for err in &reg.errors {
                row(out, "  harness error", style::err(err));
            }
            for note in &reg.notes {
                row(out, "  harness note", style::warn(note));
            }
        }
    }
    row_bool(
        out,
        "acting",
        status.enablement.acting,
        status
            .enablement
            .harness
            .note
            .as_deref()
            .or(Some("the harness layer is unknown")),
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("Session"));
    row_opt(
        out,
        "repository",
        status.repo.root.as_deref(),
        status.repo.note.as_deref(),
    );
    row_opt(
        out,
        "main checkout",
        status.repo.main_checkout_root.as_deref(),
        status.repo.note.as_deref(),
    );
    row_opt(
        out,
        "slug",
        status.identity.slug.as_deref(),
        status.identity.note.as_deref(),
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("State tree"));
    row_opt(
        out,
        "path",
        status.state_tree.root.as_deref(),
        status.state_tree.note.as_deref(),
    );
    row_bool(
        out,
        "gitignored",
        status.state_tree.ignored,
        status.state_tree.note.as_deref(),
    );
    row_bool(
        out,
        "exists",
        status.state_tree.exists,
        status.state_tree.note.as_deref(),
    );
    let dirs = &status.state_tree.directories;
    for (label, value) in [
        ("session dir", &dirs.session),
        ("conclusions", &dirs.conclusions),
        ("bypass", &dirs.bypass),
        ("expect-artifact", &dirs.expect_artifact),
        ("checklist pointer", &dirs.checklist_pointer),
    ] {
        row_opt(
            out,
            label,
            value.as_deref(),
            status.state_tree.note.as_deref(),
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("Read gate"));
    row(
        out,
        "threshold_lines",
        status.gate.threshold_lines.to_string(),
    );
    row(
        out,
        "inline_byte_budget",
        status.gate.inline_byte_budget.to_string(),
    );
    row(
        out,
        "exemptions",
        if status.gate.exemptions.is_empty() {
            "(none)".to_string()
        } else {
            status.gate.exemptions.join(", ")
        },
    );
    if let Some(note) = &status.gate.note {
        row(out, "", style::warn(note));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("Bootstrap"));
    row(out, "plugin root", status.bootstrap.plugin_root.render());
    row(out, "pin", status.bootstrap.pin.render());
    row(out, "data directory", status.bootstrap.data_dir.render());
    row_opt(
        out,
        "binary",
        status.bootstrap.binary.path.as_deref(),
        status.bootstrap.binary.note.as_deref(),
    );
    row_bool(
        out,
        "  installed",
        status.bootstrap.binary.exists,
        status.bootstrap.binary.note.as_deref(),
    );
    row_opt(
        out,
        "  version",
        status.bootstrap.binary.version.as_deref(),
        status.bootstrap.binary.note.as_deref(),
    );
    row(
        out,
        "last outcome",
        format!(
            "{} — {}",
            status.bootstrap.outcome.state, status.bootstrap.outcome.detail
        ),
    );
    if let Some(note) = &status.bootstrap.markers.note {
        row(out, "markers", unknown(Some(note)));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "{}", style::header("Versions"));
    row_opt(
        out,
        "plugin manifest",
        status.versions.manifest.as_deref(),
        // The manifest version comes from the harness registration, so when it
        // is missing the harness's own reason is the useful one — the drift
        // sentence only restates that both sides are unknown.
        status
            .enablement
            .harness
            .note
            .as_deref()
            .or(Some("the harness registration reported no version")),
    );
    row_opt(
        out,
        "pin",
        status.versions.pin.as_deref(),
        status.bootstrap.pin.note.as_deref(),
    );
    row_opt(
        out,
        "bootstrapped binary",
        status.versions.binary.as_deref(),
        status.bootstrap.binary.note.as_deref(),
    );
    row(out, "this binary", &status.versions.cli);
    row(
        out,
        "drift",
        format!("{} — {}", status.versions.drift, status.versions.detail),
    );
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "{}",
        style::header(format!(
            "Hooks ({}, {} row{})",
            status.hooks.scope,
            status.hooks.rows,
            if status.hooks.rows == 1 { "" } else { "s" }
        ))
    );
    row(out, "heartbeat log", status.hooks.heartbeat.render());
    if let Some(note) = &status.hooks.note {
        row(out, "", style::warn(note));
    }
    for event in &status.hooks.events {
        let summary = match (&event.last_fired_at, &event.note) {
            (Some(at), _) => format!(
                "last {at} {}{}   (ok {} / no-op {} / error {})",
                event.last_outcome.clone().unwrap_or_default(),
                event
                    .last_reason
                    .as_ref()
                    .map(|r| format!(" [{r}]"))
                    .unwrap_or_default(),
                event.counts.ok,
                event.counts.no_op,
                event.counts.error
            ),
            // Same distinction as the harness row: "it never fired" is an
            // answer, "the log could not be read" is a gap, and only the second
            // is an unknown.
            (None, Some(note)) if status.hooks.note.is_some() => unknown(Some(note)),
            (None, Some(note)) => style::info(note),
            // Unreachable: an event with no rows always gets a note above.
            (None, None) => unknown(Some("never fired")),
        };
        let label = if event.declared {
            event.event.clone()
        } else {
            format!("{} (not shipped)", event.event)
        };
        row(out, &label, summary);
    }
    let _ = writeln!(out);

    if status.problems.is_empty() {
        let _ = writeln!(out, "{}", style::ok("Nothing is blocking the plugin."));
    } else {
        let _ = writeln!(out, "{}", style::header("Why nothing may be happening"));
        for problem in &status.problems {
            let _ = writeln!(out, "{}", style::warn(format!("  - {problem}")));
        }
    }
}

#[cfg(test)]
mod tests;
