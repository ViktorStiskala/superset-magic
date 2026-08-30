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

use std::path::Path;

use serde_json::Value;

use crate::git;
use crate::workspace::superset_files;

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
// consumed by U14 (the page-fault gate), U19 (`config`/`enable`/`disable`)
// and U28 (`status`)
#[allow(dead_code)]
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
    let root = git::main_checkout_root(cwd_root).unwrap_or_else(|_| cwd_root.to_path_buf());
    enabled_from_value(plugin_value(&root).as_ref())
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

#[cfg(test)]
mod tests;
