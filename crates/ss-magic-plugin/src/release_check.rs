//! `release-check`: the plugin line's release cache, the once-per-release
//! update suggestion behind `SessionStart`, and the verb that refreshes and
//! reports it (R29–R33, KTD12).
//!
//! The plugin never updates itself – that is structural (the crate links no
//! updater) – but a person should still learn that a newer plugin release
//! exists, once, with the remedy. Three rules shape everything here:
//!
//! - **The hook path constructs no HTTP client.** `SessionStart` reads this
//!   line's cache file and, when it is missing or stale, spawns a detached
//!   `release-check --refresh --quiet` and returns without waiting (R30). The
//!   one network call lives in [`refresh_with`], reached only through the
//!   verb; a source-scan test in `hook/tests.rs` pins that no `hook/` module
//!   names the client.
//! - **At most once per newest tag per machine** (R29). The cache record's
//!   `suggested` field remembers the tag already announced; it is written
//!   under the same non-blocking lock the refresh takes, so a refresh cannot
//!   clobber it half-written and two sessions starting together cannot both
//!   announce. Core's `release::refresh_cache` carries the marker across a
//!   daily refresh that re-selects the same tag and clears it only for a
//!   different one.
//! - **Nobody watching, nothing said – and nothing spawned.** The suggestion
//!   and the refresh spawn are both gated on `hook::quiet_mode` (R31): a
//!   session that can never display the notice must not fork a process that
//!   outlives it to make an outbound request on the operator's behalf.
//!
//! Everything reuses core's `release` module – the anchored `PLUGIN_LINE`
//! filter, the draft/prerelease exclusion, the ETag round-trip and the 24 h
//! rule – rather than re-implementing any of it, so the two lines can never
//! disagree about what a release of theirs looks like.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result};
use serde::Serialize;
use ss_magic_core::release::{
    self, Cache, RefreshOutcome, ReleaseClient, UreqReleaseClient, PLUGIN_LINE,
};
use ss_magic_core::style;

use crate::atomic;
use crate::scratchpad::format_rfc3339;
use crate::status;
use crate::tmproot;

/// The lock both writers of the plugin release cache take: the verb's
/// refresh, and the hook's `suggested` write. One name, so they exclude each
/// other; `try_with_lock`, so neither ever waits on the other (a hook must
/// not block on a 5 s fetch, and a refresh skipped this session runs next
/// session).
pub const LOCK_NAME: &str = "release-check.lock";

/// The argv the `SessionStart` hook spawns detached when the cache is stale.
/// Spelled once here so the hook's spawn and the verb's parse agree.
pub const REFRESH_ARGV: [&str; 3] = ["release-check", "--refresh", "--quiet"];

/// Schema of the `--json` report.
pub const SCHEMA_VERSION: u32 = 1;

/// The operator's remedy, named by the notice and the verb alike. It ends in
/// "start a new session" rather than `/reload-plugins` because a reload
/// re-registers the plugin but emits no session-start event: the previous
/// binary keeps serving hooks until a fresh session's bootstrap swaps it
/// (see README, "Install and enable"). Naming the reload alone would send a
/// person to a step that leaves them on the old binary.
pub const REMEDY: &str = "run /plugin, update ss-magic there, then start a new session \
    (/reload-plugins alone keeps the old binary until a fresh session's bootstrap swaps it)";

const USAGE: &str = "\
Usage: ss-magic-plugin release-check [--refresh] [--json] [--quiet]

Report the newest known ss-magic-plugin release against this plugin's pin and
the running binary. Reads the plugin release cache only; nothing here ever
installs anything.

  --refresh   Re-read GitHub's release list once (5 s budget) and rewrite the
              cache, under a non-blocking lock: a concurrent refresh is skipped,
              not waited for. Exits 0 whether or not the fetch succeeded.
  --json      Emit the report as JSON.
  --quiet     Print nothing (what the SessionStart hook spawns, with --refresh).
  -h, --help  Print this text.";

// ── The cache file ────────────────────────────────────────────────────────────

/// Where the plugin line's cache lives: `PLUGIN_LINE.cache_file` in the shared
/// `ss-magic` cache directory, beside the CLI's own. `None` when the platform
/// resolves no cache directory at all.
pub fn cache_file() -> Option<PathBuf> {
    release::cache_dir().map(|dir| dir.join(PLUGIN_LINE.cache_file))
}

/// The same path as [`cache_file`], but only when the cache directory already
/// exists – resolving it creates nothing. `status` reads through this one so
/// its "nothing is created" promise holds even on a machine that has never
/// run a refresh.
pub fn existing_cache_file() -> Option<PathBuf> {
    release::existing_cache_dir().map(|dir| dir.join(PLUGIN_LINE.cache_file))
}

/// Replace the cache file atomically. Core's own writer is a plain
/// `fs::write`, fine for a file one process owns; this one has two writers
/// (the refresh and the hook's marker) and a lock-free reader (the hook),
/// so a half-written file must never be observable.
pub fn write_cache(path: &Path, cache: &Cache) -> Result<()> {
    let body = serde_json::to_string_pretty(cache).context("encoding the plugin release cache")?;
    atomic::write_atomically(
        path,
        &format!("{body}\n"),
        ".plugin-release-check-",
        ".tmp",
        Some("the plugin release cache"),
        Some(0o600),
        false,
    )
}

// ── The suggestion decision ───────────────────────────────────────────────────

/// What `SessionStart` should do about the plugin release cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Announce `tag`, which is newer than `pinned` and not yet announced.
    Suggest { tag: String, pinned: String },
    /// Say nothing, for the reason given.
    Silent(Silence),
}

/// Why there is nothing to announce. Ordered as [`suggestion`] checks them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Silence {
    /// Only a fresh `startup` carries the notice (R31).
    NotStartup,
    /// No `${CLAUDE_PLUGIN_ROOT}` pin to compare against – this binary is not
    /// running as an installed plugin.
    NoPin,
    /// The cache is missing, unreadable or malformed; the refresh the hook
    /// spawns will fill it in for a later startup.
    NoCache,
    /// The cache holds no tag, or one that fails the plugin line's anchored
    /// filter (a CLI tag written there by mistake reads as "no tag").
    NoTag,
    /// The newest known release is not newer than the pin.
    NotNewer { tag: String, pinned: String },
    /// This exact tag was already announced on this machine.
    AlreadySuggested(String),
    /// There is something to say, but nobody is watching (R31).
    Quiet { tag: String, reason: &'static str },
}

impl Silence {
    /// A note for the heartbeat row, or `None` on the ordinary paths where
    /// nothing was even considered – the same rule the compaction notice
    /// follows, so a plain `resume` records nothing about releases.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::NotStartup | Self::NoPin | Self::NoCache => None,
            Self::NoTag => Some("release suggestion not needed (cache holds no plugin tag)".into()),
            Self::NotNewer { tag, pinned } => Some(format!(
                "release suggestion not needed ({tag} is not newer than the pin {pinned})"
            )),
            Self::AlreadySuggested(tag) => {
                Some(format!("release suggestion already shown ({tag})"))
            }
            Self::Quiet { tag, reason } => Some(format!(
                "release suggestion suppressed (quiet mode: {reason}; {tag} not announced)"
            )),
        }
    }
}

/// The pure decision (KTD12): is there a newer plugin release to announce,
/// given the pin, the cache, the session source and the quiet verdict?
///
/// The checks run from the ones that apply to every session to the ones that
/// apply only once something is worth saying, so the recorded reason is the
/// most specific one: quiet mode is tested LAST, and a quiet session with
/// nothing to announce reads as "not newer", not as "suppressed". A tag that
/// fails the plugin line's anchored filter – the CLI's `v0.11.1`, say – is
/// `NoTag` rather than a comparison, and an unparseable pin makes `is_newer`
/// answer `false`, so a malformed input can never produce a notice.
pub fn suggestion(
    pinned: Option<&str>,
    cache: Option<&Cache>,
    source: &str,
    quiet: Option<&'static str>,
) -> Decision {
    if source != "startup" {
        return Decision::Silent(Silence::NotStartup);
    }
    let Some(pinned) = pinned.map(str::trim).filter(|p| !p.is_empty()) else {
        return Decision::Silent(Silence::NoPin);
    };
    let Some(cache) = cache else {
        return Decision::Silent(Silence::NoCache);
    };
    let tag = cache.tag_name.as_str();
    if release::parse_line_tag(&PLUGIN_LINE, tag).is_none() {
        return Decision::Silent(Silence::NoTag);
    }
    if !release::is_newer(&PLUGIN_LINE, tag, pinned) {
        return Decision::Silent(Silence::NotNewer {
            tag: tag.to_string(),
            pinned: pinned.to_string(),
        });
    }
    if cache.suggested.as_deref() == Some(tag) {
        return Decision::Silent(Silence::AlreadySuggested(tag.to_string()));
    }
    if let Some(reason) = quiet {
        return Decision::Silent(Silence::Quiet {
            tag: tag.to_string(),
            reason,
        });
    }
    Decision::Suggest {
        tag: tag.to_string(),
        pinned: pinned.to_string(),
    }
}

/// The notice itself (R29). Operator-facing, on `systemMessage` only; it
/// never asks the model to do anything.
pub fn notice(tag: &str, pinned: &str) -> String {
    format!(
        "ss-magic: plugin release {tag} is available (this plugin pins {pinned}). \
         To update: {REMEDY}. Optional; shown once per release."
    )
}

/// How [`record_suggested`] went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// The marker was written; the caller may announce.
    Written,
    /// Another session recorded this tag between the caller's decision and
    /// the lock; it announced, so the caller must not.
    AlreadyRecorded,
    /// A refresh changed the cache's tag between the decision and the lock;
    /// the caller must not announce a tag that is no longer the newest – the
    /// next startup decides again.
    Superseded,
    /// The lock is held right now (a refresh mid-fetch, or a concurrent
    /// session); the caller withholds the notice rather than risk announcing
    /// twice, and the next startup decides again.
    Busy,
}

/// Write `tag` into the cache's `suggested` field, under the shared lock,
/// re-checking the record first – the caller decided from a lock-free read.
///
/// Refuses to wait: `Busy` on contention. Refuses to record a tag the cache
/// no longer names: `Superseded`. Both mean "say nothing this time", and
/// both cost at most one session's delay, which is cheaper than a repeated
/// notice would be.
pub fn record_suggested(lock_root: &Path, cache_file: &Path, tag: &str) -> Result<Recorded> {
    let outcome = tmproot::try_with_lock(lock_root, LOCK_NAME, || -> Result<Recorded> {
        let current = release::read_cache(cache_file).unwrap_or_default();
        if current.tag_name != tag {
            return Ok(Recorded::Superseded);
        }
        if current.suggested.as_deref() == Some(tag) {
            return Ok(Recorded::AlreadyRecorded);
        }
        write_cache(
            cache_file,
            &Cache {
                suggested: Some(tag.to_string()),
                ..current
            },
        )?;
        Ok(Recorded::Written)
    })
    .with_context(|| format!("locking {}", lock_root.join(LOCK_NAME).display()))?;
    outcome.unwrap_or(Ok(Recorded::Busy))
}

// ── The detached refresh ──────────────────────────────────────────────────────

/// Spawn `exe args…` in its own process group with every standard stream on
/// `/dev/null`, and return its pid without waiting for it.
///
/// The child is dropped rather than joined – the whole point is that the hook
/// returns while the fetch runs – and `process_group(0)` detaches it from the
/// harness's process group, so a signal aimed at the session does not take
/// the refresh with it and the refresh's exit is never the hook's concern.
/// `/dev/null` on every stream matters twice over: a hook's stdout is the
/// JSON envelope the harness parses, and an inherited pipe would keep the
/// harness waiting on a descriptor the child holds open.
pub fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<u32> {
    use std::os::unix::process::CommandExt as _;
    let child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(child.id())
}

/// Spawn this very binary as `release-check --refresh --quiet`, detached.
/// What `SessionStart` calls (R30); `Err` carries a one-line reason for the
/// heartbeat row and is never a failure of the hook.
pub fn spawn_refresh() -> std::result::Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    spawn_detached(&exe, &REFRESH_ARGV).map_err(|e| format!("spawn: {e}"))
}

// ── The refresh ───────────────────────────────────────────────────────────────

/// How one `--refresh` went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshReport {
    /// The fetch ran (successfully or not – `outcome` says) and the cache was
    /// rewritten.
    Ran(RefreshOutcome),
    /// Another refresh (or a hook's marker write) held the lock; skipped.
    Busy,
}

/// One bounded fetch of the release list under the shared non-blocking lock,
/// rewriting the cache with core's `refresh_cache` derivation (R33).
///
/// The prior record is read INSIDE the lock, so a marker the hook wrote a
/// moment ago is what gets carried forward rather than a stale snapshot.
pub fn refresh_with<C: ReleaseClient>(
    client: &C,
    lock_root: &Path,
    cache_file: &Path,
    now: u64,
) -> Result<RefreshReport> {
    let outcome = tmproot::try_with_lock(lock_root, LOCK_NAME, || -> Result<RefreshOutcome> {
        let prior = release::read_cache(cache_file);
        let refreshed = release::refresh_cache(prior, client, &PLUGIN_LINE, now);
        write_cache(cache_file, &refreshed.cache)?;
        Ok(refreshed.outcome)
    })
    .with_context(|| format!("locking {}", lock_root.join(LOCK_NAME).display()))?;
    Ok(match outcome {
        Some(result) => RefreshReport::Ran(result?),
        None => RefreshReport::Busy,
    })
}

// ── The report ────────────────────────────────────────────────────────────────

/// Everything `release-check` has to say.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// [`SCHEMA_VERSION`].
    pub schema: u32,
    /// The release line this report is about.
    pub line: LineReport,
    /// The cache file and what it holds.
    pub cache: CacheReport,
    /// The plugin's pin, and where it was read from.
    pub pin: PinReport,
    /// The version of the binary producing this report.
    pub running: String,
    /// Whether the newest known release is newer than the pin.
    pub update: UpdateReport,
    /// What `--refresh` did, when it was asked for.
    pub refresh: RefreshField,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineReport {
    pub tag_prefix: &'static str,
    pub cache_file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    /// True when the file exists and parses.
    pub present: bool,
    /// When the list was last checked, RFC 3339 UTC.
    pub checked_at: Option<String>,
    /// Seconds since then.
    pub age_secs: Option<u64>,
    /// Whether that is within the 24 h a hook trusts it for.
    pub fresh: Option<bool>,
    /// The newest plugin tag last selected, when the cache holds one.
    pub newest_tag: Option<String>,
    /// The tag already announced on this machine, when one has been.
    pub suggested: Option<String>,
    /// Why a value above is null.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PinReport {
    pub version: Option<String>,
    pub source: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    /// `None` when either side is unknown; `note` says which.
    pub available: Option<bool>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefreshField {
    pub requested: bool,
    /// `fetched`, `not-modified`, `failed`, `busy`, an error, or
    /// `not-requested`.
    pub outcome: String,
    pub note: String,
}

/// The pin, read the way `status` reads it: `${CLAUDE_PLUGIN_ROOT}` first,
/// then the harness registration's `installPath`. The harness probe is a
/// bounded subprocess and runs only when the variable is absent.
fn locate_pin() -> PinReport {
    let located = match std::env::var("CLAUDE_PLUGIN_ROOT") {
        // `locate_plugin_root` answers from the variable before it looks at
        // the listing, so an unprobed listing is never consulted here.
        Ok(dir) if !dir.is_empty() => status::locate_plugin_root(
            &status::HarnessListing::Unavailable("not probed: ${CLAUDE_PLUGIN_ROOT} is set".into()),
        ),
        _ => status::locate_plugin_root(&status::probe_harness()),
    };
    let Some(root) = located.path else {
        return PinReport {
            version: None,
            source: None,
            note: located.note,
        };
    };
    match status::read_pin_file(&root) {
        Ok((version, path)) => PinReport {
            version: Some(version),
            source: Some(path.display().to_string()),
            note: None,
        },
        Err(note) => PinReport {
            version: None,
            source: None,
            note: Some(note),
        },
    }
}

/// The cache's newest tag when it is a plugin-line tag, or the note saying
/// why it is not: an empty tag means the last fetch found no plugin release,
/// anything else failing the anchored filter is a tag of some other line
/// (the CLI's, say) that has no business being in this cache. Shared with
/// `status`'s "newest release" row so the two reports say the same thing.
pub fn cached_plugin_tag(cache: &Cache) -> std::result::Result<&str, String> {
    if release::parse_line_tag(&PLUGIN_LINE, &cache.tag_name).is_some() {
        Ok(&cache.tag_name)
    } else if cache.tag_name.is_empty() {
        Err("the last fetch found no ss-magic-plugin release".to_string())
    } else {
        Err(format!(
            "the cached tag `{}` is not an ss-magic-plugin release tag",
            cache.tag_name
        ))
    }
}

/// Build the report from a cache file, a pin and the clock. Pure apart from
/// the one file read, so the tests point it at a tempdir.
pub fn report(
    cache_file: Option<&Path>,
    pin: PinReport,
    running: &str,
    now: u64,
    refresh: RefreshField,
) -> Report {
    let cache = match cache_file {
        None => CacheReport {
            present: false,
            checked_at: None,
            age_secs: None,
            fresh: None,
            newest_tag: None,
            suggested: None,
            note: Some("no cache directory resolves on this platform".to_string()),
        },
        Some(path) => match release::read_cache(path) {
            None => CacheReport {
                present: false,
                checked_at: None,
                age_secs: None,
                fresh: None,
                newest_tag: None,
                suggested: None,
                note: Some(format!(
                    "{} is absent or unreadable; a fresh session start refreshes it in the \
                     background, or run `release-check --refresh`",
                    path.display()
                )),
            },
            Some(record) => {
                let (newest_tag, note) = match cached_plugin_tag(&record) {
                    Ok(tag) => (Some(tag.to_string()), None),
                    Err(note) => (None, Some(note)),
                };
                CacheReport {
                    present: true,
                    checked_at: (record.checked_at != 0).then(|| format_rfc3339(record.checked_at)),
                    age_secs: (record.checked_at != 0).then(|| now.saturating_sub(record.checked_at)),
                    fresh: Some(record.is_fresh(now)),
                    newest_tag,
                    suggested: record.suggested.clone(),
                    note,
                }
            }
        },
    };

    let update = match (&cache.newest_tag, &pin.version) {
        (Some(tag), Some(pinned)) => {
            if release::is_newer(&PLUGIN_LINE, tag, pinned) {
                UpdateReport {
                    available: Some(true),
                    note: format!("{tag} is newer than the pin {pinned} — {REMEDY}"),
                }
            } else if release::parse_line_tag(&PLUGIN_LINE, &format!("{}{pinned}", PLUGIN_LINE.tag_prefix)).is_some() {
                UpdateReport {
                    available: Some(false),
                    note: format!("{tag} is not newer than the pin {pinned}"),
                }
            } else {
                UpdateReport {
                    available: None,
                    note: format!("the pin `{pinned}` is not a plain MAJOR.MINOR.PATCH version"),
                }
            }
        }
        (None, _) => UpdateReport {
            available: None,
            note: "no newest release is known yet".to_string(),
        },
        (_, None) => UpdateReport {
            available: None,
            note: "the plugin's pin is unknown".to_string(),
        },
    };

    Report {
        schema: SCHEMA_VERSION,
        line: LineReport {
            tag_prefix: PLUGIN_LINE.tag_prefix,
            cache_file: cache_file
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        },
        cache,
        pin,
        running: running.to_string(),
        update,
        refresh,
    }
}

/// `3h 2m`, `45s`, `2d 1h` – for the age column.
fn format_age(secs: u64) -> String {
    let (d, rem) = (secs / 86_400, secs % 86_400);
    let (h, rem) = (rem / 3_600, rem % 3_600);
    let (m, s) = (rem / 60, rem % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s}s"),
        (0, 0, _) => format!("{m}m {s}s"),
        (0, _, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn render_text(out: &mut String, report: &Report) {
    const WIDTH: usize = 22;
    let row = |out: &mut String, label: &str, value: String| {
        let _ = writeln!(out, "  {label:<WIDTH$}{value}");
    };
    let _ = writeln!(out, "{}", style::header("Plugin release check"));
    match (&report.cache.newest_tag, &report.cache.note) {
        (Some(tag), _) => {
            let when = match (&report.cache.checked_at, report.cache.age_secs) {
                (Some(at), Some(age)) => format!("   (checked {at}, {} ago)", format_age(age)),
                _ => String::new(),
            };
            row(out, "newest known release", format!("{tag}{when}"));
        }
        (None, note) => row(
            out,
            "newest known release",
            style::info(format!(
                "unknown — {}",
                note.as_deref().unwrap_or("no reason recorded")
            )),
        ),
    }
    match (&report.pin.version, &report.pin.source, &report.pin.note) {
        (Some(v), Some(src), _) => row(out, "this plugin pins", format!("{v}   ({src})")),
        (Some(v), None, _) => row(out, "this plugin pins", v.clone()),
        (None, _, note) => row(
            out,
            "this plugin pins",
            style::info(format!(
                "unknown — {}",
                note.as_deref().unwrap_or("no reason recorded")
            )),
        ),
    }
    row(out, "running binary", report.running.clone());
    row(
        out,
        "update",
        match report.update.available {
            Some(true) => style::warn(&report.update.note),
            Some(false) => style::ok(&report.update.note),
            None => style::info(&report.update.note),
        },
    );
    row(
        out,
        "notice",
        match &report.cache.suggested {
            Some(tag) => format!("already shown for {tag}"),
            None => "not shown yet".to_string(),
        },
    );
    if report.refresh.requested {
        row(
            out,
            "refresh",
            format!("{} — {}", report.refresh.outcome, report.refresh.note),
        );
    }
}

// ── The verb ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flags {
    refresh: bool,
    json: bool,
    quiet: bool,
}

enum ParsedArgs {
    Run(Flags),
    Help,
    Error(String),
}

fn parse_args(args: &[String]) -> ParsedArgs {
    let mut flags = Flags {
        refresh: false,
        json: false,
        quiet: false,
    };
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return ParsedArgs::Help,
            "--refresh" => flags.refresh = true,
            "--json" => flags.json = true,
            "--quiet" => flags.quiet = true,
            other => return ParsedArgs::Error(format!("unexpected argument `{other}`")),
        }
    }
    ParsedArgs::Run(flags)
}

/// `ss-magic-plugin release-check [--refresh] [--json] [--quiet]`.
///
/// Exit 0 on every path that produced a report, including a failed fetch
/// (R33): the verb is asked "what do you know", and "the network was down"
/// is an answer. Only an argument the verb cannot interpret exits 2.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let flags = match parse_args(args) {
        ParsedArgs::Run(flags) => flags,
        ParsedArgs::Help => {
            println!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        ParsedArgs::Error(message) => {
            eprintln!("{}", style::err(format!("error: {message}")));
            eprintln!("{USAGE}");
            return Ok(ExitCode::from(2));
        }
    };

    let now = release::now_secs();
    let cache_file = cache_file();

    let refresh = if flags.refresh {
        run_refresh(cache_file.as_deref(), now)
    } else {
        RefreshField {
            requested: false,
            outcome: "not-requested".to_string(),
            note: "pass --refresh to re-read the release list".to_string(),
        }
    };

    if flags.quiet {
        return Ok(ExitCode::SUCCESS);
    }

    let report = report(
        cache_file.as_deref(),
        locate_pin(),
        env!("CARGO_PKG_VERSION"),
        now,
        refresh,
    );
    if flags.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let mut out = String::new();
        render_text(&mut out, &report);
        print!("{out}");
    }
    Ok(ExitCode::SUCCESS)
}

/// The `--refresh` half: the one place in the crate an HTTP client is
/// constructed. Every failure becomes a field of the report, never an exit
/// code.
fn run_refresh(cache_file: Option<&Path>, now: u64) -> RefreshField {
    let field = |outcome: &str, note: String| RefreshField {
        requested: true,
        outcome: outcome.to_string(),
        note,
    };
    let Some(cache_file) = cache_file else {
        return field("failed", "no cache directory resolves on this platform".to_string());
    };
    let lock_root = match tmproot::resolve_root() {
        Ok(root) => root,
        Err(e) => return field("failed", format!("no lock root: {e}")),
    };
    let client = UreqReleaseClient::for_product("ss-magic-plugin", env!("CARGO_PKG_VERSION"));
    match refresh_with(&client, &lock_root, cache_file, now) {
        Ok(RefreshReport::Ran(RefreshOutcome::Fetched)) => {
            field("fetched", "the release list was re-read".to_string())
        }
        Ok(RefreshReport::Ran(RefreshOutcome::NotModified)) => field(
            "not-modified",
            "GitHub answered 304; the cached tag is still current".to_string(),
        ),
        Ok(RefreshReport::Ran(RefreshOutcome::Failed)) => field(
            "failed",
            "the fetch failed (offline, rate-limited, or a malformed answer); the \
             prior record was kept"
                .to_string(),
        ),
        Ok(RefreshReport::Busy) => field(
            "busy",
            "another refresh holds the lock; skipped rather than waited".to_string(),
        ),
        Err(e) => field("failed", format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests;
