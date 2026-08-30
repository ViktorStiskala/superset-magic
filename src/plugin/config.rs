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
//! problem (see [`crate::plugin::run_hook`]'s doc comment). "Safe" means the
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

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use crate::git;
use crate::plugin::scratchpad;
use crate::tui::style;
use crate::workspace::superset_files::{self, MagicConfig};

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
    PluginConfig {
        enabled: resolve_enabled(cwd_root),
        gate: gate_from_value(plugin_value(cwd_root).as_ref()),
    }
}

/// R7's per-machine toggle: `enabled` read from the main checkout's overlay,
/// falling back to resolving against `cwd_root` itself when no main
/// checkout can be found at all (e.g. `cwd_root` is not inside a git
/// repository, or the `git` invocation otherwise fails) — still the safest
/// available answer, and strictly better than refusing to resolve.
fn resolve_enabled(cwd_root: &Path) -> bool {
    enabled_from_value(plugin_value(&main_checkout_or_self(cwd_root)).as_ref())
}

/// The main checkout's root, or `cwd_root` itself when none can be found
/// (outside any git repository, or the `git` invocation otherwise fails) —
/// the same fallback [`resolve_enabled`] uses, shared here because the
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
//   `git::main_checkout_root` fallback [`resolve_enabled`] uses, because a
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

const ENABLE_USAGE: &str = "\
Usage: ss-magic plugin enable [--local]

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

const DISABLE_USAGE: &str = "\
Usage: ss-magic plugin disable [--local]

Stop the plugin's hooks from acting on this repository by setting
`plugin.enabled` to `false`. The installed tree (binary, hooks, skills) is
left in place — this only flips the switch.

Without --local, writes .superset/magic.json (committed). With --local,
writes the main checkout's .superset/magic.local.json instead (R7).

Never removes the .superset/.magic/ gitignore rule `enable` may have added.";

const CONFIG_USAGE: &str = "\
Usage: ss-magic plugin config get <plugin.DOTTED.KEY>
       ss-magic plugin config set <plugin.DOTTED.KEY> <VALUE> [--local]

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

    write_plugin_key(&target_root, local, &[PLUGIN_KEY, "enabled"], Value::Bool(enabled))?;

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

    write_plugin_key(&target_root, local, segments, value)?;

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
fn write_plugin_key(target_root: &Path, local: bool, path: &[&str], value: Value) -> Result<()> {
    debug_assert_eq!(path.first(), Some(&PLUGIN_KEY), "path must be plugin-rooted");

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
