//! Typed interpretation of the `plugin` key inside the overlaid `magic.json`.
//!
//! `workspace::superset_files::load_overlaid` already merges `magic.json` with
//! `magic.local.json` at the raw JSON level (KTD8's `extras` map): a
//! non-`files` key found in `magic.local.json` replaces the base value WHOLE
//! rather than being deep-merged with it, an absent key inherits the base
//! value, and an explicit `null` overrides to "off" (R6). This module reads
//! the merged `plugin` value out of that map and turns it into two closed
//! structs — [`PluginConfig`] and its nested [`GateConfig`] — with the
//! binary-owned defaults and bounds R53 requires, so every later reader (the
//! page-fault gate, `status`, `config get`/`set`) shares one interpretation
//! of the schema instead of re-deriving it from `serde_json::Value` itself.
//!
//! ## Fail-safe, not fail-loud
//!
//! Every parse failure here — a missing `plugin` block, a `plugin` value
//! that isn't a JSON object, a `gate` sub-key of the wrong type, a numeric
//! value outside R53's stated bounds, a non-string entry in `exemptions` —
//! degrades to a safe value rather than returning an error. [`resolve`]
//! cannot fail: it is called (via later units) from the `PreToolUse` hook
//! wrapper, whose contract is to never break a session over a configuration
//! problem (see [`crate::run_hook`]'s doc comment). "Safe" means the
//! MORE conservative reading in each direction: a malformed `enabled`
//! defaults to `false` (the plugin does nothing rather than acting on
//! ill-defined settings, the same outcome as an absent block); an
//! out-of-bounds numeric value CLAMPS to the nearer edge of its range rather
//! than being ignored outright, so a configured value is never honored
//! wider than R53's stated bounds; and a malformed exemption entry is
//! dropped rather than kept, so a bad entry can only shrink the exemption
//! list (make the gate fire MORE often), never widen it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::git;
use crate::{compact_window, scratchpad, tmproot};
use ss_magic_core::style;
use ss_magic_core::superset_files::{self, MagicConfig};

/// The one lock every writer of `magic.json` / `magic.local.json` in this
/// crate takes, under the private per-machine temp root (R80) so it exists
/// before any repository state does. It serializes the load-modify-write:
/// `seed-config` runs unattended from every session start, and a seed that
/// loaded the file before `enable` wrote `plugin.enabled` would otherwise
/// write the loaded copy back without the key – enablement silently lost.
/// One machine-wide name rather than one per repository, because contention
/// is rare (a few milliseconds per write) and the root is shared anyway.
const CONFIG_LOCK_NAME: &str = "magic-json.lock";

/// Run `write` under [`CONFIG_LOCK_NAME`], WAITING for a holder to finish.
/// For the human verbs (`enable`, `disable`, `config set`): a person asked
/// for the write, so it must happen, and a wait of milliseconds is fine.
/// Without a usable temp root the write still happens, unlocked and with a
/// warning – a human verb must not be refused over a temporary directory.
fn write_locked<T>(write: impl FnOnce() -> Result<T>) -> Result<T> {
    match tmproot::resolve_root() {
        Ok(lock_root) => tmproot::with_lock(&lock_root, CONFIG_LOCK_NAME, write)
            .context("taking the config lock")?,
        Err(reason) => {
            eprintln!(
                "{}",
                style::warn(format!(
                    "no private temp root to lock the config write ({reason}); \
                     writing without the lock"
                ))
            );
            write()
        }
    }
}

// ── Bounds and defaults (R53) ───────────────────────────────────────────────

/// Default size threshold, in lines, above which the page-fault gate acts.
/// Derived from page-fault.md's measured read costs: a 3,000-line read costs
/// 32,060 tokens — comfortably inside the harness's own 25,000-token `Read`
/// budget — while an 8,000-line read (60,066 tokens) is not, so the default
/// sits at the lower measured point rather than the higher one.
pub const GATE_THRESHOLD_LINES_DEFAULT: u32 = 3_000;
/// Lower bound a configured threshold clamps to.
pub const GATE_THRESHOLD_LINES_MIN: u32 = 500;
/// Upper bound a configured threshold clamps to.
pub const GATE_THRESHOLD_LINES_MAX: u32 = 20_000;

/// Default byte budget for an inline conclusion. Sized to the measured
/// 10,000-character cliff the hook contract records for the
/// `additionalContext` channel; the deny channel (`permissionDecisionReason`)
/// is uncapped and not governed by this value.
pub const GATE_INLINE_BYTE_BUDGET_DEFAULT: u32 = 10_000;
/// Lower bound a configured byte budget clamps to.
pub const GATE_INLINE_BYTE_BUDGET_MIN: u32 = 1_000;
/// Upper bound a configured byte budget clamps to.
pub const GATE_INLINE_BYTE_BUDGET_MAX: u32 = 100_000;

// ── Typed shape ──────────────────────────────────────────────────────────────

/// The plugin's own resolved view of the overlaid `plugin` key.
///
/// `enabled` is the per-repository switch (R5): the plugin acts in a
/// repository only when this is `true`, and it is `false` whenever the
/// `enabled` key, or the whole `plugin` block, is absent. `gate` holds the
/// page-fault gate's own tunables (R53). The two fields are resolved
/// against different roots — see [`resolve`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PluginConfig {
    pub enabled: bool,
    pub gate: GateConfig,
}

/// The page-fault gate's resolved tunables (R53), each defaulted and bounded
/// independently of the others and of `enabled`. JSON shape (nested under
/// `plugin`):
///
/// ```json
/// { "plugin": { "gate": {
///   "threshold_lines": 3000,
///   "inline_byte_budget": 10000,
///   "exemptions": ["docs/**"]
/// } } }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GateConfig {
    /// Line count above which a `Read` is gated. Clamped to
    /// [`GATE_THRESHOLD_LINES_MIN`]..=[`GATE_THRESHOLD_LINES_MAX`].
    pub threshold_lines: u32,
    /// Byte budget for an inline conclusion riding `additionalContext`.
    /// Clamped to
    /// [`GATE_INLINE_BYTE_BUDGET_MIN`]..=[`GATE_INLINE_BYTE_BUDGET_MAX`].
    pub inline_byte_budget: u32,
    /// Patterns the gate never applies its threshold to. Empty by default —
    /// nothing is exempt until the configuration names it.
    pub exemptions: Vec<String>,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            threshold_lines: GATE_THRESHOLD_LINES_DEFAULT,
            inline_byte_budget: GATE_INLINE_BYTE_BUDGET_DEFAULT,
            exemptions: Vec::new(),
        }
    }
}

// ── Resolution ───────────────────────────────────────────────────────────────

/// Resolve the effective plugin configuration for `cwd_root`.
///
/// The two halves of [`PluginConfig`] are deliberately resolved against
/// different roots:
///
/// - `enabled` (R7) always comes from the MAIN CHECKOUT's overlay —
///   `magic.json` plus THAT checkout's own `magic.local.json` — found via
///   [`git::main_checkout_root`], regardless of whether `cwd_root` is a
///   linked worktree or the main checkout itself. A worktree's own
///   `magic.local.json` is one of the files `ss-magic sync` copies down by
///   default, so trusting a worktree's own copy of `enabled` would mean the
///   next forward sync silently overrides whatever a person just set on
///   this machine; reading it from main sidesteps that entirely. When
///   `cwd_root` already IS the main checkout, `main_checkout_root` returns
///   it unchanged, so this is exactly `cwd_root`'s own overlay.
/// - `gate` (R53) resolves directly against `cwd_root`'s own overlay. The
///   gate's thresholds and exemptions are tuning knobs, not a per-machine
///   safety toggle, so there is no reason to redirect them away from
///   whatever this worktree actually has checked out.
///
/// Infallible: every failure mode (no git repository reachable from
/// `cwd_root`, no `magic.json` on disk, a malformed `magic.json` or
/// `magic.local.json`, a `plugin` value that is not a JSON object) degrades
/// to the corresponding half of [`PluginConfig::default`] rather than
/// propagating an error to the caller.
pub fn resolve(cwd_root: &Path) -> PluginConfig {
    resolve_with_roots(cwd_root, git::main_checkout_root(cwd_root).ok().as_deref())
}

/// [`resolve`] with the main checkout root supplied by the caller instead of
/// probed here.
///
/// The hook pipeline (U2) discovers both roots from the filesystem before
/// resolving the configuration and hands the main root in, so that the
/// enablement gate — which every `PreToolUse` invocation reaches — costs no
/// `git rev-parse --git-common-dir` subprocess on the ordinary layouts. The
/// human verbs keep calling [`resolve`], which probes.
///
/// `main_root` is trusted as given: `enabled` (R7) is read from ITS overlay.
/// `None` means no main checkout could be named at all (outside any git
/// repository, or git could not answer), and `enabled` then falls back to
/// `cwd_root`'s own overlay — still the safest available answer, and strictly
/// better than refusing to resolve. `gate` (R53) always resolves against
/// `cwd_root`.
pub fn resolve_with_roots(cwd_root: &Path, main_root: Option<&Path>) -> PluginConfig {
    PluginConfig {
        enabled: enabled_from_value(plugin_value(main_root.unwrap_or(cwd_root)).as_ref()),
        gate: gate_from_value(plugin_value(cwd_root).as_ref()),
    }
}

/// The main checkout's root, or `cwd_root` itself when none can be found
/// (outside any git repository, or the `git` invocation otherwise fails) —
/// the same fallback [`resolve`] uses for `enabled`, shared here because the
/// `--local` write path (R7) needs exactly the same root.
fn main_checkout_or_self(cwd_root: &Path) -> PathBuf {
    git::main_checkout_root(cwd_root).unwrap_or_else(|_| cwd_root.to_path_buf())
}

/// The merged `plugin` value at `root` (base `magic.json` overlaid with that
/// root's own `magic.local.json`, via [`superset_files::load_overlaid`]), or
/// `None` when there is no `magic.json` at all, it fails to parse, or the
/// merged config carries no `plugin` key. Those cases are deliberately
/// folded into the same `None` here: a hook must never fail loudly over a
/// configuration problem, so the difference between "not configured" and
/// "misconfigured" is not this function's to preserve — both degrade to the
/// same safe defaults downstream.
fn plugin_value(root: &Path) -> Option<Value> {
    superset_files::load_overlaid(root)
        .ok()
        .flatten()
        .and_then(|cfg| cfg.extras.get("plugin").cloned())
}

/// Interpret a merged `plugin` value as the `enabled` switch. Anything other
/// than a literal JSON `true` (an absent key, `null`, a non-bool value, or
/// the `plugin` value itself not being an object) reads as `false`.
fn enabled_from_value(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .and_then(|map| map.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Interpret a merged `plugin` value's `gate` sub-key. A `plugin` value that
/// isn't an object, an absent or non-object `gate`, or an individual
/// tunable of the wrong JSON type each fall back to that one tunable's
/// default — one bad field never invalidates the rest of the block.
fn gate_from_value(value: Option<&Value>) -> GateConfig {
    let gate = value
        .and_then(Value::as_object)
        .and_then(|map| map.get("gate"))
        .and_then(Value::as_object);

    GateConfig {
        threshold_lines: gate
            .and_then(|g| g.get("threshold_lines"))
            .and_then(Value::as_u64)
            .map(|n| {
                n.clamp(
                    u64::from(GATE_THRESHOLD_LINES_MIN),
                    u64::from(GATE_THRESHOLD_LINES_MAX),
                ) as u32
            })
            .unwrap_or(GATE_THRESHOLD_LINES_DEFAULT),
        inline_byte_budget: gate
            .and_then(|g| g.get("inline_byte_budget"))
            .and_then(Value::as_u64)
            .map(|n| {
                n.clamp(
                    u64::from(GATE_INLINE_BYTE_BUDGET_MIN),
                    u64::from(GATE_INLINE_BYTE_BUDGET_MAX),
                ) as u32
            })
            .unwrap_or(GATE_INLINE_BYTE_BUDGET_DEFAULT),
        // A non-string entry (a number, an object, ...) is dropped rather
        // than kept or defaulted whole — see the module doc's "fail-safe"
        // note: this can only shrink the exemption list, never widen it.
        exemptions: gate
            .and_then(|g| g.get("exemptions"))
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

// ── Write path: `enable` / `disable` / `config get` / `config set` (U19, R37) ──
//
// Everything above this line only READS the overlaid `plugin` value. What
// follows WRITES it, from four human verbs sharing one discipline:
//
// - Every write is a load-modify-write over the ONE file being targeted
//   (never the merged/overlaid view — that would bake local's already-applied
//   values back into base, or vice versa). It changes exactly the key it was
//   asked to change and carries every other key on the file forward
//   untouched, including keys this build has never heard of (KTD8) — the
//   same discipline [`superset_files::merge_files_into_magic_config`]
//   documents for `files`, applied here to `plugin`.
// - `--local` (R7) redirects the target from the caller's own repository root
//   to the MAIN CHECKOUT's `magic.local.json`, resolved with the same
//   `git::main_checkout_root` fallback [`resolve`] uses, because a
//   worktree's own local overlay is itself a forward-sync target and cannot
//   be trusted to hold the per-machine enable toggle.
// - `get` always answers from the OVERLAID value (base plus the caller's own
//   local overlay) — never base alone — since that is what a person actually
//   wants to know: what the plugin will do, not what one file happens to say.
// - Whenever a write turns `plugin.enabled` on, it also gitignores
//   `.superset/.magic/` at the CALLER's OWN root (via
//   [`scratchpad::ensure_state_ignored`]) — R40's lazy half of the ignore
//   rule. This runs at the repository the invocation is ACTUALLY standing in,
//   not at `--local`'s redirected target: hooks fire, and the ignored-tree
//   gate is checked, wherever the session's cwd is, so that is the tree that
//   has to be protected right now regardless of which file recorded the
//   toggle. Turning the plugin off never removes the rule — R40 is explicit
//   that nothing here ever edits `.gitignore` except to add this one line.
// - `config` is scoped to keys rooted at `"plugin"` only. `files` and any
//   other top-level key already have their own editors (the bootstrap
//   picker, the edit-config menu); this verb is "the plugin configuration
//   from the command line" (R37), not a general JSON editor for the file.

/// The one top-level key `config get`/`config set` may address. Also the
/// first segment `write_plugin_key` expects in every path it is handed.
const PLUGIN_KEY: &str = "plugin";

pub(crate) const ENABLE_USAGE: &str = "\
Usage: ss-magic-plugin enable [--local]

Turn the plugin's hooks on for this repository by setting `plugin.enabled`
to `true`.

Without --local, writes .superset/magic.json (committed — affects every
worktree once the change is committed and synced). With --local, writes the
main checkout's .superset/magic.local.json instead (R7: a worktree's own
local overlay is itself a forward-sync target, so the per-machine toggle
always lands in the main checkout's).

Also gitignores .superset/.magic/ in THIS repository if it is not already,
so a repository initialized before this shipped is not silenced the moment
it is turned on.";

pub(crate) const DISABLE_USAGE: &str = "\
Usage: ss-magic-plugin disable [--local]

Stop the plugin's hooks from acting on this repository by setting
`plugin.enabled` to `false`. The installed tree (binary, hooks, skills) is
left in place — this only flips the switch.

Without --local, writes .superset/magic.json (committed). With --local,
writes the main checkout's .superset/magic.local.json instead (R7).

Never removes the .superset/.magic/ gitignore rule `enable` may have added.";

pub(crate) const CONFIG_USAGE: &str = "\
Usage: ss-magic-plugin config get <plugin.DOTTED.KEY>
       ss-magic-plugin config set <plugin.DOTTED.KEY> <VALUE> [--local]

Read or write one key under the plugin configuration block (magic.json's
`plugin` object) — not `files`, and not any other top-level key.

`get` always reads the OVERLAID, resolved value (magic.json plus this
repository's own magic.local.json), never just the committed base.

`set` parses VALUE as JSON when it parses (true, 3000, [\"docs/**\"], null,
...), otherwise takes it as a plain string. Without --local it edits
.superset/magic.json; with --local it edits the main checkout's
.superset/magic.local.json instead (R7), resolved from any worktree.

Setting plugin.enabled to true also gitignores .superset/.magic/ in this
repository if it is not already (R40); nothing here ever removes that rule.";

pub(crate) const SEED_CONFIG_USAGE: &str = "\
Usage: ss-magic-plugin seed-config

Fold a `plugin` block carrying the gate defaults into an EXISTING
.superset/magic.json, so the knobs are visible and editable in the file a
person already has open.

Invoked once by hooks/bootstrap.sh after it installs the binary. Writes
NOTHING unless there is a .superset/magic.json with no `plugin` key at all,
never writes `plugin.enabled`, and never stages the change.";

/// What one [`run_seed_config`] pass did. Every variant is a normal outcome,
/// not an error: the verb runs unattended from a `SessionStart` hook in
/// whatever repository the session happens to be in, and most of those are not
/// ss-magic workspaces at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    /// There was a `.superset/magic.json` with no `plugin` key; the block was
    /// written.
    Seeded,
    /// A `plugin` key already existed. The file is left byte-identical.
    AlreadyPresent,
    /// No `.superset/magic.json` to fold into — absent, or present but not
    /// parseable as one. Nothing was written and nothing was created.
    NotAWorkspace,
    /// `.superset/magic.json` resolves OUTSIDE the repository, through a
    /// symlink on the file or on an ancestor. Nothing was written.
    OutsideRepository,
}

/// The block the seed writes, and the ONE place its shape is decided.
///
/// It is built field by field out of [`GateConfig::default`] rather than
/// serialized from a struct, and that is deliberate: the requirement is that
/// this writer be structurally incapable of emitting an `enabled` key, and a
/// literal map with three named inserts is something a reader can verify at a
/// glance and a test can assert exhaustively. Deriving `Serialize` on a config
/// struct would move that guarantee into whatever fields the struct grows next.
///
/// Why `enabled` must be ABSENT rather than `false`: an absent key already
/// reads as off (`enabled_from_value` returns `false` for anything that is not
/// a literal JSON `true`), so writing `false` would buy nothing and would make
/// the seed look like a decision about enablement. It is not one. The plugin's
/// standing rule is that a repository cannot arrange its own enablement by
/// getting a hook to fire, and this verb runs from a hook — so the safe design
/// is a writer with no code path to that key at all, which is what this is.
fn seed_block() -> Value {
    let defaults = GateConfig::default();
    let mut gate = Map::new();
    gate.insert(
        "threshold_lines".to_string(),
        Value::from(defaults.threshold_lines),
    );
    gate.insert(
        "inline_byte_budget".to_string(),
        Value::from(defaults.inline_byte_budget),
    );
    gate.insert(
        "exemptions".to_string(),
        Value::Array(defaults.exemptions.into_iter().map(Value::String).collect()),
    );
    let mut block = Map::new();
    block.insert("gate".to_string(), Value::Object(gate));
    Value::Object(block)
}

/// The pure half of `seed-config`: decide and write, against `root`.
///
/// Split from [`run_seed_config`] so every branch is testable against a
/// tempdir without a git repository or a process.
///
/// The four things it will not do, each of which is a test rather than a
/// convention:
///
/// - **It never creates the file.** An absent `.superset/`, an absent
///   `magic.json`, or a `magic.json` that does not parse all mean
///   [`SeedOutcome::NotAWorkspace`]. Installing the plugin must not introduce a
///   tracked file into a checkout that is not an ss-magic workspace at all, and
///   a file that does not parse is far more likely a merge conflict or a
///   half-written edit than an invitation to rebuild it from nothing.
/// - **It is strictly once.** Any existing `plugin` key — seeded by an earlier
///   session, hand-edited, or committed by a teammate — means
///   [`SeedOutcome::AlreadyPresent`] and no write, so the seed can neither
///   repeat on later sessions nor fight a deliberate edit.
/// - **It never writes `enabled`** — see [`seed_block`].
/// - **It never stages.** No `git add` here or downstream. The change shows up
///   in `git status` as an ordinary worktree modification for a person to read
///   and commit on purpose, which is the point: the block is being SURFACED,
///   not slipped in. Note what "the change" is: `write_magic_json` re-serializes
///   the whole document rather than patching it, and `serde_json` is built
///   without `preserve_order`, so unknown keys come back alphabetized. Values
///   all survive, but a hand-ORDERED file is reordered. In practice the file was
///   written by `ss-magic init` through this same serializer and is already in
///   that form, so the diff is just the added block — measured, not assumed.
///
/// `root` is the current checkout's own root, not the main checkout's. That is
/// the right root for what is written: `gate` (unlike `enabled`) resolves
/// against the cwd root's own overlay, so seeding anywhere else would put the
/// defaults where this worktree's gate never reads them.
pub fn seed_config_at(root: &Path) -> Result<SeedOutcome> {
    // Containment first, and as an OUTCOME rather than an error, because the
    // seed runs unattended from `hooks/bootstrap.sh` in whatever checkout the
    // person opened and must report, never fail. `write_plugin_key` refuses
    // an outside target on its own as well; this earlier decision exists so
    // the seed can answer `OutsideRepository` instead of surfacing that
    // refusal as a failed write. The shared decision also folds "the file is
    // not there" into `NotAWorkspace`: a file that does not exist is what the
    // load below would report anyway, and the seed never creates one.
    match landing(root, false) {
        Ok(Landing::Existing) => {}
        Ok(Landing::Outside) => return Ok(SeedOutcome::OutsideRepository),
        Ok(Landing::Fresh) | Err(_) => return Ok(SeedOutcome::NotAWorkspace),
    }

    // One guard for all three "nothing to fold into" cases, because
    // `load_magic_json` already distinguishes them: `Ok(None)` is an absent
    // file, `Err` is one that exists but does not parse. A malformed file reads
    // as NotAWorkspace rather than as an error — this runs unattended from an
    // install and must never turn a bad `magic.json` into a failed session
    // start or a silently rebuilt file.
    //
    // An `is_file()` precondition above this used to spell the absent case out
    // a second time. It was removed because it was unobservable: with it gone
    // every input still produces the same outcome, so it was a branch no test
    // could pin and a future edit could quietly make load-bearing.
    let Ok(Some(existing)) = superset_files::load_magic_json(root) else {
        return Ok(SeedOutcome::NotAWorkspace);
    };
    if existing.extras.contains_key(PLUGIN_KEY) {
        return Ok(SeedOutcome::AlreadyPresent);
    }
    write_plugin_key(root, false, &[PLUGIN_KEY], seed_block())?;
    Ok(SeedOutcome::Seeded)
}

/// `ss-magic-plugin seed-config` — invoked by `hooks/bootstrap.sh` once, right
/// after it installs the binary.
///
/// This verb exists because there is no longer any terminal path to the plugin
/// configuration at all: the `ss-magic` CLI dropped its `plugin` subcommand
/// when the plugin became its own binary, and the new binary lives under
/// `${CLAUDE_PLUGIN_DATA}`, which is not on a person's `PATH`. Rather than
/// document a command nobody can type, the bootstrap makes the configuration
/// visible where a person is already looking — in the workspace contract file
/// their repository already tracks.
///
/// Outside a git repository it does nothing, like every other verb here.
pub fn run_seed_config(args: &[String]) -> Result<ExitCode> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{SEED_CONFIG_USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(unexpected) = args.first() {
        return Ok(usage_error(
            SEED_CONFIG_USAGE,
            &format!("`seed-config` takes no arguments, got `{unexpected}`"),
        ));
    }

    let cwd = std::env::current_dir().context("getting current directory")?;
    let Ok(root) = git::cwd_repo_root(&cwd) else {
        println!(
            "{}",
            style::info("not inside a git repository; nothing to configure.")
        );
        return Ok(ExitCode::SUCCESS);
    };

    // Non-blocking, unlike the human verbs: this runs from a session-start
    // hook, so it must never wait, and a seed skipped this session costs
    // nothing – the next session's bootstrap calls it again, and the block
    // is only ever written when there is no `plugin` key at all.
    let Ok(lock_root) = tmproot::resolve_root() else {
        println!(
            "{}",
            style::info("no private temp root to lock the config write; seeding deferred.")
        );
        return Ok(ExitCode::SUCCESS);
    };
    let outcome = tmproot::try_with_lock(&lock_root, CONFIG_LOCK_NAME, || seed_config_at(&root))
        .context("taking the config lock")?;
    let Some(outcome) = outcome else {
        println!(
            "{}",
            style::info(
                "another ss-magic-plugin process is writing this repository's config; \
                 seeding deferred to the next session."
            )
        );
        return Ok(ExitCode::SUCCESS);
    };

    match outcome? {
        SeedOutcome::Seeded => println!(
            "{}",
            style::ok(
                "wrote the default `plugin` block into .superset/magic.json. It is \
                 unstaged: review it and commit it if you want it shared. The plugin \
                 stays OFF until `plugin.enabled` is set to true."
            )
        ),
        SeedOutcome::AlreadyPresent => println!(
            "{}",
            style::info(".superset/magic.json already has a `plugin` block; left as it is.")
        ),
        SeedOutcome::NotAWorkspace => println!(
            "{}",
            style::info("no readable .superset/magic.json here; nothing was written.")
        ),
        // Loud, and on stderr: unlike the other three this is not a normal
        // state. Still exit 0 — the bootstrap discards this output and must
        // never fail a session start over it — so the message is the whole
        // signal.
        SeedOutcome::OutsideRepository => eprintln!(
            "{}",
            style::warn(
                "refusing to seed: .superset/magic.json resolves outside this \
                 repository (a symlink on the file or on .superset). Nothing was \
                 written."
            )
        ),
    }
    Ok(ExitCode::SUCCESS)
}

/// `plugin enable` — a human verb; problems report on stderr and exit
/// non-zero.
pub fn run_enable(args: &[String]) -> Result<ExitCode> {
    run_toggle(args, ENABLE_USAGE, true)
}

/// `plugin disable` — the mirror of [`run_enable`].
pub fn run_disable(args: &[String]) -> Result<ExitCode> {
    run_toggle(args, DISABLE_USAGE, false)
}

/// Shared body for `enable`/`disable`: both are "set `plugin.enabled` to a
/// fixed literal, optionally at the `--local` target", differing only in
/// which literal and which usage text. Parses argv and reads the real
/// current directory, then hands off to [`run_toggle_core`], which takes an
/// explicit `cwd` so the actual filesystem work is testable without a
/// process or a real working directory.
fn run_toggle(args: &[String], usage: &str, enabled: bool) -> Result<ExitCode> {
    let local = match args {
        [] => false,
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{usage}");
            return Ok(ExitCode::SUCCESS);
        }
        [flag] if flag == "--local" => true,
        _ => return Ok(usage_error(usage, "pass no arguments, or exactly `--local`")),
    };

    let cwd = std::env::current_dir().context("reading the current directory")?;
    run_toggle_core(&cwd, local, enabled)
}

fn run_toggle_core(cwd: &Path, local: bool, enabled: bool) -> Result<ExitCode> {
    let cwd_root = git::cwd_repo_root(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let target_root = if local {
        main_checkout_or_self(&cwd_root)
    } else {
        cwd_root.clone()
    };

    write_locked(|| {
        write_plugin_key(&target_root, local, &[PLUGIN_KEY, "enabled"], Value::Bool(enabled))
    })?;

    if enabled {
        scratchpad::ensure_state_ignored(&cwd_root).context("gitignoring .superset/.magic/")?;
    }

    println!(
        "{}",
        style::ok(format!(
            "Set `plugin.enabled` to `{enabled}` in {}",
            magic_file_label(local)
        ))
    );
    // R27 — one line pointing at `compact-window --recommend` when this
    // worktree configures no auto-compact window. Advice only: the tip names
    // the verb, and that verb writes nothing until `--set` is typed (R28).
    if enabled {
        if let Some(tip) = compact_window::enable_tip(&cwd_root) {
            println!("{}", style::info(tip));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `plugin config get|set …` — dispatches to the two sub-verbs.
pub fn run_config(args: &[String]) -> Result<ExitCode> {
    let Some((sub, rest)) = args.split_first() else {
        return Ok(usage_error(CONFIG_USAGE, "needs a `get` or `set` subcommand"));
    };
    match sub.as_str() {
        "-h" | "--help" => {
            println!("{CONFIG_USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "get" => run_config_get(rest),
        "set" => run_config_set(rest),
        other => Ok(usage_error(
            CONFIG_USAGE,
            &format!("unknown `config` subcommand `{other}`"),
        )),
    }
}

/// `plugin config get <dotted-key>`.
fn run_config_get(args: &[String]) -> Result<ExitCode> {
    let [key] = args else {
        return Ok(usage_error(CONFIG_USAGE, "`get` takes exactly one dotted key"));
    };
    let segments = match validate_plugin_key(key) {
        Ok(segments) => segments,
        Err(message) => return Ok(usage_error(CONFIG_USAGE, &message)),
    };

    let cwd = std::env::current_dir().context("reading the current directory")?;
    let value = run_config_get_core(&cwd, &segments)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(ExitCode::SUCCESS)
}

/// The read half of `config get`, split out from printing so it is testable
/// against a plain `Value` rather than captured stdout — mirroring
/// `status.rs`'s split between computing a report and printing it.
fn run_config_get_core(cwd: &Path, segments: &[&str]) -> Result<Value> {
    let cwd_root = git::cwd_repo_root(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let cfg = superset_files::load_overlaid(&cwd_root)
        .context("reading the overlaid magic.json")?
        .unwrap_or_default();

    Ok(navigate(cfg.extras.get(PLUGIN_KEY), &segments[1..])
        .cloned()
        .unwrap_or(Value::Null))
}

/// `plugin config set <dotted-key> <value> [--local]`.
fn run_config_set(args: &[String]) -> Result<ExitCode> {
    let (key, raw_value, local) = match args {
        [key, value] => (key, value, false),
        [key, value, flag] if flag == "--local" => (key, value, true),
        _ => {
            return Ok(usage_error(
                CONFIG_USAGE,
                "`set` takes a dotted key, a value, and an optional `--local`",
            ))
        }
    };
    let segments = match validate_plugin_key(key) {
        Ok(segments) => segments,
        Err(message) => return Ok(usage_error(CONFIG_USAGE, &message)),
    };

    let value = parse_value(raw_value);
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let outcome = run_config_set_core(&cwd, key, &segments, value, local)?;
    println!("{}", style::ok(outcome));
    Ok(ExitCode::SUCCESS)
}

fn run_config_set_core(
    cwd: &Path,
    key: &str,
    segments: &[&str],
    value: Value,
    local: bool,
) -> Result<String> {
    let turns_on = turns_plugin_on(segments, &value);
    let cwd_root = git::cwd_repo_root(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let target_root = if local {
        main_checkout_or_self(&cwd_root)
    } else {
        cwd_root.clone()
    };

    write_locked(|| write_plugin_key(&target_root, local, segments, value))?;

    if turns_on {
        scratchpad::ensure_state_ignored(&cwd_root).context("gitignoring .superset/.magic/")?;
    }

    Ok(format!("Set `{key}` in {}", magic_file_label(local)))
}

/// Split a dotted key into segments and check it is one `config` may touch:
/// rooted at `"plugin"`, with no empty segment (a stray leading, trailing or
/// doubled dot).
fn validate_plugin_key(key: &str) -> Result<Vec<&str>, String> {
    let segments: Vec<&str> = key.split('.').collect();
    if segments.first() != Some(&PLUGIN_KEY) || segments.iter().any(|s| s.is_empty()) {
        return Err(format!(
            "`{key}` is not a plugin configuration key; `config` only reads and writes keys \
             rooted at `plugin` (e.g. `plugin.enabled`, `plugin.gate.threshold_lines`)"
        ));
    }
    Ok(segments)
}

/// Walk `path` (dotted-key segments AFTER the leading `"plugin"`) into
/// `value`, returning `None` as soon as a segment is missing or the current
/// value is not an object to index into. An empty `path` returns `value`
/// itself unchanged — the "get/set the whole plugin block" case.
fn navigate<'a>(value: Option<&'a Value>, path: &[&str]) -> Option<&'a Value> {
    let mut current = value?;
    for seg in path {
        current = current.as_object()?.get(*seg)?;
    }
    Some(current)
}

/// Parse a CLI-supplied value as JSON when it parses as one (`true`, `3000`,
/// `["docs/**"]`, the literal `null`, ...); anything that fails to parse —
/// ordinary unquoted text like `docs/**` — is taken as a plain JSON string
/// instead, so a caller never has to hand-quote everyday values.
fn parse_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// Whether writing `value` at `segments` (a [`validate_plugin_key`]-checked,
/// plugin-rooted dotted key) turns the plugin ON — the trigger for R40's lazy
/// ignore-rule write. Recognizes both the ordinary `plugin.enabled` spelling
/// and a whole-block replacement (`plugin` set to an object carrying
/// `"enabled": true`), since both mean "the plugin is now enabled" from the
/// ignore rule's point of view.
fn turns_plugin_on(segments: &[&str], value: &Value) -> bool {
    match segments {
        [PLUGIN_KEY, "enabled"] => value.as_bool() == Some(true),
        [PLUGIN_KEY] => value.get("enabled").and_then(Value::as_bool) == Some(true),
        _ => false,
    }
}

/// Load-modify-write one key on the plugin configuration file at
/// `target_root` (`magic.local.json` when `local`, else `magic.json`).
/// `path` is the FULL key path INCLUDING the leading `"plugin"` segment
/// (e.g. `&["plugin", "enabled"]`, or just `&["plugin"]` for the whole
/// block); `value` replaces whatever was there. Every other key on the file —
/// every other top-level key, and every other key under `plugin` — survives
/// unchanged (KTD8): this loads the file, walks/creates just the requested
/// path inside `extras`, and writes the whole `MagicConfig` back. A malformed
/// existing file is a hard error (propagated, not swallowed) rather than
/// being silently rebuilt from nothing.
///
/// It also refuses, as an error, a target that resolves OUTSIDE the
/// repository through a symlink ([`Landing::Outside`]). This sits inside the
/// one writer rather than in each verb so that no caller – `enable`,
/// `disable`, `config set`, the seed, or a verb added later – can reach the
/// file without passing it. The hazard it closes is a repository that commits
/// `.superset/magic.json` (or the `.superset` directory) as a link to a JSON
/// file the person owns, say a harness settings file: a verb they run in that
/// checkout would load THAT file, fold a `plugin` key into it, and write it
/// back through the link. A link that stays inside the repository is fine and
/// is written through, onto its resolved target.
fn write_plugin_key(target_root: &Path, local: bool, path: &[&str], value: Value) -> Result<()> {
    debug_assert_eq!(path.first(), Some(&PLUGIN_KEY), "path must be plugin-rooted");

    if landing(target_root, local)? == Landing::Outside {
        bail!(
            "refusing to write {}: it resolves outside the repository at {} through a \
             symlink (on the file, or on .superset). Nothing was written.",
            magic_file_label(local),
            target_root.display()
        );
    }

    let existing = if local {
        superset_files::load_magic_local_json(target_root)
    } else {
        superset_files::load_magic_json(target_root)
    }
    .with_context(|| format!("reading the existing {}", magic_file_label(local)))?;

    let mut cfg: MagicConfig = existing.unwrap_or_default();
    set_nested(&mut cfg.extras, path, value);

    if local {
        superset_files::write_magic_local_json(target_root, &cfg)
    } else {
        superset_files::write_magic_json(target_root, &cfg)
    }
    .with_context(|| format!("writing {}", magic_file_label(local)))
}

/// Where a write to `.superset/magic.json` (or `magic.local.json`) under a
/// root would actually land, decided by [`landing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    /// The file exists and resolves inside the repository – possibly through
    /// a symlink that stays in the tree, which every writer follows.
    Existing,
    /// The file does not exist yet. It would be created under a `.superset`
    /// that resolves inside the repository, or beneath the root when
    /// `.superset` does not exist either; nothing on the path can redirect it.
    Fresh,
    /// The file, or the `.superset` directory above it, resolves OUTSIDE the
    /// repository through a symlink. No writer may proceed.
    Outside,
}

/// Decide where the configuration file at `target_root` really is before a
/// writer follows the path to it.
///
/// `canonicalize` on both sides is what makes the comparison meaningful: it
/// resolves every symlink and every `..` in one step, so a link anywhere along
/// the path is caught, and canonicalizing the ROOT too keeps the check honest
/// on a platform where the repository itself sits behind a link (macOS's
/// `/tmp` -> `/private/tmp` is the everyday case). The deepest EXISTING
/// component decides: an existing file must itself resolve inside; a missing
/// file is created in whatever `.superset` resolves to, so the directory
/// decides; when neither exists, both are created beneath the root that was
/// just resolved. A root that cannot be resolved is an error rather than a
/// pass – the unknown answer must be the refusing one.
fn landing(target_root: &Path, local: bool) -> Result<Landing> {
    let real_root = target_root
        .canonicalize()
        .with_context(|| format!("resolving the repository root {}", target_root.display()))?;
    let (existing, resolved) = match target_root.join(magic_file_label(local)).canonicalize() {
        Ok(real_file) => (true, real_file),
        Err(_) => match target_root.join(".superset").canonicalize() {
            Ok(real_dir) => (false, real_dir),
            Err(_) => return Ok(Landing::Fresh),
        },
    };
    if !resolved.starts_with(&real_root) {
        return Ok(Landing::Outside);
    }
    Ok(if existing { Landing::Existing } else { Landing::Fresh })
}

/// Set the value at `path` (a non-empty list of dotted-key segments, e.g.
/// `["plugin", "gate", "threshold_lines"]`) inside `extras`, creating any
/// missing intermediate object along the way. An intermediate value that
/// exists but is not itself an object (e.g. a stray `"plugin": "oops"` left
/// by hand-editing) is replaced with a fresh empty object rather than
/// rejected — the same fail-safe posture `gate_from_value`/`enabled_from_value`
/// already take on the READ side, applied here to writing: `set` always
/// succeeds instead of erroring over a malformed value it is about to fix.
fn set_nested(extras: &mut Map<String, Value>, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return; // an empty path has nothing to set; every caller here passes
                 // at least `["plugin"]`
    };
    let mut current = extras;
    for seg in parents {
        let entry = current
            .entry((*seg).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("just ensured this is an object");
    }
    current.insert((*last).to_string(), value);
}

/// The relative path a message names for the file a write just touched.
fn magic_file_label(local: bool) -> &'static str {
    if local {
        ".superset/magic.local.json"
    } else {
        ".superset/magic.json"
    }
}

/// Report a usage mistake and hand back the exit code for one: the message,
/// then `usage`. `2` matches the rest of the crate's convention for "the
/// command as typed cannot be carried out" (see e.g.
/// `checklist::verbs::refused`).
fn usage_error(usage: &str, message: &str) -> ExitCode {
    eprintln!("{}", style::err(format!("error: {message}")));
    eprintln!("{usage}");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests;
