# Recon: plugin-verbs-a (main dispatch, config, release_check, setup_ci)

## Map

### `crates/ss-magic-plugin/src/main.rs`
Crate root: argv parse into `Parsed`/`Invocation`, the two closed vocabularies (`HookEvent`, `HumanVerb`), and the dispatch table to every sibling module's `run`.

- `HookEvent` (pub enum) – `SessionStart | PreToolUse | PreCompact | SubagentStop | SessionEnd | FileChanged | Unknown(String) | Missing` – the manifest event channel; `Unknown`/`Missing` are values, not errors.
  - `from_token (pub) – (token: &str) -> Self` – total mapping, unknown token becomes `Unknown`.
  - `as_str (pub) – (&self) -> &str` – wire name for heartbeat rows/diagnostics.
- `HumanVerb` (pub enum, Copy) – the 17 verb tokens (`Status`, `Cost`, ..., `Checklist`).
  - `from_token (pub) – (token: &str) -> Option<Self>`.
  - `as_str (pub) – (&self) -> &'static str` – **no production caller** (comment says so); kept for round-trip tests only.
  - `writes_config (pub) – (&self) -> bool` – `Enable|Disable|Config|SeedConfig`; no production caller, asserted only by `tests.rs`.
  - `can_set_enabled (pub) – (&self) -> bool` – `Enable|Disable|Config`; the actual hook-safety invariant, also only read by `tests.rs`.
- `Invocation` (pub enum) – `Hook { event, args }` / `Human { verb, args }`.
- `Parsed` (pub enum) – `Invocation | Version | Help | MissingVerb | UnknownVerb(String)`.
- `USAGE (pub const &str)` – the top-level usage banner (main.rs:341). **dup-name** with two other `USAGE` consts (release_check.rs:71, setup_ci.rs:92) – different text, same identifier, only a namespacing coincidence since each is module-private/pub(crate) except this one.
- `usage (pub) – () -> &'static str` – returns `USAGE`.
- `version_line (pub) – () -> String` – `"ss-magic-plugin {CARGO_PKG_VERSION}"`; contract-shaped (bootstrap.sh + status.rs parse the last field of line 1).
- `parse (pub) – (args: &[String]) -> Parsed` – pure, no I/O; first-token-only `-V`/`-h` recognition (deliberately, unlike the CLI's whole-argv scan).
- `run (pub) – (args: &[String]) -> Result<ExitCode>` – sets color mode then matches `Parsed`.
- `forces_no_color (private) – (parsed: &Parsed) -> bool` – true only for a `Hook` invocation. Small/generic-looking but genuinely one-off (depends on this crate's own `Parsed`), not a duplicate.
- `dispatch (private) – (inv: Invocation) -> Result<ExitCode>` – routes to `run_hook`/`run_human`. Single-call passthrough; could be inlined into `run`.
- `run_hook (private) – (event: &HookEvent) -> Result<ExitCode>` – thin wrapper around `hook::run`; single caller (`dispatch`), single callee.
- `run_human (private) – (verb: HumanVerb, args: &[String]) -> Result<ExitCode>` – the 17-arm verb→module table.
- `main () -> ExitCode` – argv collection, error print, exit code.

### `crates/ss-magic-plugin/src/config.rs`
Typed `plugin` key in the overlaid `magic.json`: read-side resolution (`resolve`/`resolve_with_roots`) for the `PreToolUse` gate and `status`, and the write-side verbs `enable`/`disable`/`config get|set`/`seed-config`.

- `CONFIG_LOCK_NAME (private const &str)` = `"magic-json.lock"` (config.rs:51) – dup-name family with other `LOCK_NAME`s (see Constants section).
- `GATE_THRESHOLD_LINES_DEFAULT/MIN/MAX (pub const u32)` = 3_000 / 500 / 20_000 (config.rs:82,84,86) – **used only inside this file** (and its own tests); not read from `status.rs` or elsewhere.
- `GATE_INLINE_BYTE_BUDGET_DEFAULT/MIN/MAX (pub const u32)` = 10_000 / 1_000 / 100_000 (config.rs:92,94,96) – same: unused outside this file.
- `PluginConfig (pub struct, Default)` – `{ enabled: bool, gate: GateConfig }`.
- `GateConfig (pub struct)` – `{ threshold_lines, inline_byte_budget, exemptions: Vec<String> }`, with a hand-written `Default` at config.rs:138.
- `resolve (pub) – (cwd_root: &Path) -> PluginConfig` – probes `git::main_checkout_root` itself; used by `status.rs:927,1102`.
- `resolve_with_roots (pub) – (cwd_root: &Path, main_root: Option<&Path>) -> PluginConfig` – used by `hook/mod.rs:513` (avoids a subprocess on the hot path).
- `main_checkout_or_self (private) – (cwd_root: &Path) -> PathBuf` – fallback wrapper around `git::main_checkout_root`; small/generic-looking, but it's the `--local` target resolution specific to this file.
- `plugin_value (private) – (root: &Path) -> Option<Value>` – merged `plugin` JSON value via `superset_files::load_overlaid`.
- `enabled_from_value (private) – (Option<&Value>) -> bool`.
- `gate_from_value (private) – (Option<&Value>) -> GateConfig` – clamps two numeric fields, filters non-string `exemptions`.
- `PLUGIN_KEY (private const &str)` = `"plugin"` (config.rs:318).
- `ENABLE_USAGE / DISABLE_USAGE / CONFIG_USAGE / SEED_CONFIG_USAGE (pub(crate) const &str)` – four usage banners (config.rs:320,336,348,366).
- `SeedOutcome (pub enum)` – `Seeded | AlreadyPresent | NotAWorkspace | OutsideRepository`.
- `seed_block (private) – () -> Value` – literal-map builder, structurally cannot emit `enabled`.
- `seed_config_at (pub) – (root: &Path) -> Result<SeedOutcome>` – pure-ish core of `seed-config`; calls `landing` itself (pinned decision per structure-pins, "seed needs an outcome, writer needs an error" – not a duplication finding).
- `run_seed_config (pub) – (args: &[String]) -> Result<ExitCode>` – verb entry; non-blocking lock (`try_with_lock`).
- `run_enable / run_disable (pub) – (args: &[String]) -> Result<ExitCode>` – both call `run_toggle`.
- `run_toggle (private) – (args, usage, enabled: bool) -> Result<ExitCode>` – argv parse (`match args { [] | [flag] ... }`), one more flag-parsing idiom in this crate (see Leads).
- `run_toggle_core (private) – (cwd: &Path, local: bool, enabled: bool) -> Result<ExitCode>` – testable core; calls `compact_window::enable_tip`.
- `run_config (pub) – (args: &[String]) -> Result<ExitCode>` – dispatches `get`/`set`.
- `run_config_get / run_config_get_core (private)` – read half.
- `run_config_set / run_config_set_core (private)` – write half; `turns_plugin_on` decides the R40 ignore-rule trigger.
- `validate_plugin_key (private) – (key: &str) -> Result<Vec<&str>, String>`.
- `navigate (private) – (value: Option<&Value>, path: &[&str]) -> Option<&Value>` – generic dotted-path walker; small and generic, a candidate to check against similar walkers elsewhere (none found in this partition; `checklist/schema.rs` has its own typed model instead of `Value` navigation).
- `parse_value (private) – (raw: &str) -> Value` – "parse as JSON, else treat as string."
- `turns_plugin_on (private) – (segments: &[&str], value: &Value) -> bool`.
- `write_plugin_key (private) – (target_root, local, path, value) -> Result<()>` – the one load-modify-write, calls `landing` a second time (pinned: seed needs an outcome, this needs an error).
- `Landing (private enum)` – `Existing | Fresh | Outside`.
- `landing (private) – (target_root: &Path, local: bool) -> Result<Landing>` (config.rs:882-897) – canonicalize root + deepest-existing-component, `starts_with` containment check. **Same shape as containment checks elsewhere in the crate** (see Leads L3).
- `set_nested (private) – (extras: &mut Map<String, Value>, path: &[&str], value: Value)` – generic nested-map setter; small/generic, no other caller.
- `magic_file_label (private) – (local: bool) -> &'static str`.
- `usage_error (private) – (usage: &str, message: &str) -> ExitCode` (config.rs:938) – **argument order is `(usage, message)`**, the reverse of every sibling `usage_error` in the crate (see Leads L1).

### `crates/ss-magic-plugin/src/release_check.rs`
The plugin release line's cache, the `SessionStart` once-per-tag suggestion decision, and the `release-check` verb (refresh/report).

- `LOCK_NAME (pub const &str)` = `"release-check.lock"` (release_check.rs:53) – **pub, but no caller outside this file** (own `record_suggested`/`refresh_with`); dup-name with `checklist/verbs.rs::LOCK_NAME` and `hook/file_changed.rs::LOCK_NAME` (different values, all private/pub-unused-externally).
- `REFRESH_ARGV (pub const [&str;3])` = `["release-check","--refresh","--quiet"]` (release_check.rs:57) – used only inside this file (`spawn_refresh`); the doc comment says it keeps "the hook's spawn and the verb's parse" in agreement, but the hook (`hook/session_start.rs`) never references this const directly – it calls `spawn_refresh()`, which uses it internally. Comment slightly overstates the direct sharing.
- `SCHEMA_VERSION (pub const u32)` = `1` (release_check.rs:60) – dup-name with `status.rs::SCHEMA_VERSION = 2`; unrelated schemas (this one is the `release-check --json` report, the other is `status --json`), so not a bug, but worth the reviewer knowing both exist.
- `REMEDY (pub const &str)` (release_check.rs:68) – used at :228 and :522, and by `hook/session_start.rs` via `release_check::notice`.
- `USAGE (private const &str)` (release_check.rs:71) – dup-name, see main.rs entry.
- `cache_file (pub) – () -> Option<PathBuf>`.
- `existing_cache_file (pub) – () -> Option<PathBuf>` – used by `status.rs:1916`.
- `write_cache (pub) – (path: &Path, cache: &Cache) -> Result<()>` – atomic write via `crate::atomic::write_atomically`.
- `Decision (pub enum)` – `Suggest{..} | Silent(Silence)`.
- `Silence (pub enum)` – 7 reasons, ordered as checked.
  - `detail (pub) – (&self) -> Option<String>`.
- `suggestion (pub) – (pinned, cache, source, quiet) -> Decision` – the KTD12 pure decision; used by `hook/session_start.rs:404`.
- `notice (pub) – (tag: &str, pinned: &str) -> String`.
- `Recorded (pub enum)` – `Written | AlreadyRecorded | Superseded | Busy`.
- `record_suggested (pub) – (lock_root, cache_file, tag) -> Result<Recorded>` – used by `hook/session_start.rs:410`.
- `spawn_detached (pub) – (exe: &Path, args: &[&str]) -> io::Result<u32>`.
- `spawn_refresh (pub) – () -> Result<u32, String>` – used by `hook/session_start.rs:141` (as an injected `Fn`).
- `RefreshReport (pub enum)` – `Ran(RefreshOutcome) | Busy`.
- `refresh_with (pub) – <C: ReleaseClient>(client, lock_root, cache_file, now) -> Result<RefreshReport>` – only called from `run_refresh` in this file.
- `Report / LineReport / CacheReport / PinReport / UpdateReport / RefreshField (pub structs, Serialize)` – the `--json` shape.
- `locate_pin (private) – () -> PinReport` – reuses `status::locate_plugin_root` / `status::probe_harness` / `status::read_pin_file` (good reuse, not a duplicate).
- `cached_plugin_tag (pub) – (cache: &Cache) -> Result<&str, String>` – used by `status.rs:1661`.
- `report (pub) – (cache_file, pin, running, now, refresh) -> Report`.
- `format_age (private) – (secs: u64) -> String` (release_check.rs:563) – `3h 2m`/`45s`/`2d 1h` day/hour/min/sec breakdown. No equivalent elsewhere in the crate (checked `status.rs`, `compact_window.rs`, `heartbeat.rs` – none re-implement this breakdown), so NOT a duplicate; flagged only because it's a small generic time-formatter that could move to `ss_magic_core::style` or similar if a second caller ever needs it.
- `render_text (private) – (out: &mut String, report: &Report)` (release_check.rs:575) – local `const WIDTH: usize = 22;` plus an inline `row` closure doing `{label:<WIDTH$}{value}` – the same left-aligned label/value row shape `status.rs` already has as named helpers `row`/`row_opt`/`row_bool` (status.rs:1966-1990, with its own `LABEL_WIDTH`). See Leads L9/L11.
- `Flags (private struct)`, `ParsedArgs (private enum)`, `parse_args (private) – (&[String]) -> ParsedArgs` – one of several different flag-parsing idioms in this crate (see Leads L5).
- `run (pub) – (args: &[String]) -> Result<ExitCode>` – used by `main.rs:540`.
- `run_refresh (private) – (cache_file: Option<&Path>, now: u64) -> RefreshField` – the one place an `UreqReleaseClient` is constructed.

### `crates/ss-magic-plugin/src/setup_ci.rs`
`setup-github-ci`: write the embedded GitHub Actions workflow into the consuming repository, with a 4-state classify/diff/force flow.

- `TEMPLATE (pub const &str)` = `include_str!("../../../assets/workflow/checklist.yml")` (setup_ci.rs:62).
- `VERSION_PLACEHOLDER (pub const &str)` = `"@SS_MAGIC_PLUGIN_VERSION@"` (setup_ci.rs:68).
- `WORKFLOW_REL (pub const &str)` = `".github/workflows/ss-magic-checklist.yml"` (setup_ci.rs:75).
- `PIN_KEY (private const &str)` = `"SS_MAGIC_PLUGIN_VERSION:"` (setup_ci.rs:80).
- `FILE_MODE (private const u32)` = `0o644` (setup_ci.rs:84) – dup-name with 10+ other `FILE_MODE`/`NEW_FILE_MODE` consts crate-wide (see Leads L2).
- `DIFF_LINE_BUDGET (private const usize)` = `120` (setup_ci.rs:89).
- `USAGE (private const &str)` (setup_ci.rs:92) – dup-name, see main.rs entry.
- `render (pub) – (version: &str) -> String` – single `str::replace`.
- `pinned_version (pub) – (text: &str) -> Option<String>` – scans for the `env:` line rather than parsing YAML.
- `State (pub enum)` – `Absent | Identical | PinStale{found} | Differs`.
  - `token (pub) – (&self) -> &'static str`.
  - `needs_force (private) – (&self) -> bool`.
- `classify (pub) – (existing: Option<&str>, version: &str) -> State` – used by `setup_ci/tests.rs` extensively; production caller is `run_core`.
- `run (pub) – (args: &[String]) -> Result<ExitCode>` – manual `for arg in args { match ... }` flag loop (yet another idiom, see Leads L5); used by `main.rs:531`.
- `run_core (pub) – (cwd: &Path, version: &str, check: bool, force: bool) -> Result<ExitCode>`.
- `report_state (private) – (state: &State, version: &str)`.
- `would (private) – (state: &State, force: bool) -> String`.
- `has_checklist (private) – (root: &Path) -> bool` – reuses `checklist::{ACTIONS_REL, CHECKLIST_SUFFIX}` (good reuse).
- `print_diff (private) – (current: &str, proposed: &str)` – uses the `similar` crate directly; the only user of `similar` in this partition.
- `write_workflow (private) – (path: &Path, body: &str) -> Result<()>` – `atomic::write_atomically(..., Some(FILE_MODE), true)`.

## Constants and literals

| Const/literal | File:line | Value | Elsewhere in workspace? | Owning definition |
|---|---|---|---|---|
| `CONFIG_LOCK_NAME` | config.rs:51 | `"magic-json.lock"` | Not reused; dup-name family with `LOCK_NAME` in release_check.rs:53, checklist/verbs.rs:105, hook/file_changed.rs:106, and `INSTALL_LOCK_NAME`/`POINTER_LOCK_NAME` elsewhere in the crate (all different values, one per subsystem's own lock) | This file; the "one lock name per subsystem under the shared tmproot" pattern is deliberate (each subsystem locks its own resource) |
| `GATE_THRESHOLD_LINES_DEFAULT/MIN/MAX` | config.rs:82,84,86 | 3_000 / 500 / 20_000 | Not found elsewhere in the workspace (grepped `crates/`) | This file; genuinely unused outside it + tests – candidate to shrink visibility from `pub` |
| `GATE_INLINE_BYTE_BUDGET_DEFAULT/MIN/MAX` | config.rs:92,94,96 | 10_000 / 1_000 / 100_000 | Not found elsewhere | Same as above |
| `PLUGIN_KEY` | config.rs:318 | `"plugin"` | Not reused as a literal elsewhere (checked `status.rs`, `hook/`) | This file |
| `ENABLE_USAGE` | config.rs:320 | usage banner text | N/A (unique text) | This file |
| `DISABLE_USAGE` | config.rs:336 | usage banner text | N/A | This file |
| `CONFIG_USAGE` | config.rs:348 | usage banner text | N/A | This file |
| `SEED_CONFIG_USAGE` | config.rs:366 | usage banner text | N/A | This file |
| `LOCK_NAME` | release_check.rs:53 | `"release-check.lock"` | dup-name (see above); `pub` but no external caller | This file |
| `REFRESH_ARGV` | release_check.rs:57 | `["release-check","--refresh","--quiet"]` | Not reused; `pub` but only self-referenced | This file |
| `SCHEMA_VERSION` | release_check.rs:60 | `1` | dup-name with `status.rs:74`'s `SCHEMA_VERSION = 2` (different report schemas – not a conflict, just a namespace collision) | This file |
| `REMEDY` | release_check.rs:68 | remedy sentence | Reused via `notice()`/report – not duplicated text elsewhere | This file |
| `USAGE` (release_check) | release_check.rs:71 | usage banner text | dup-name with main.rs:341 and setup_ci.rs:92 (unrelated text, coincidental identifier) | This file |
| `WIDTH` | release_check.rs:576 | `22` (usize, local to `render_text`) | Analogous to `status.rs`'s `LABEL_WIDTH` (not found by exact grep in this partition's index, but same row-formatting purpose) | This file; candidate to share status.rs's row helpers instead |
| `TEMPLATE` | setup_ci.rs:62 | `include_str!(...)` | Sole embed of `assets/workflow/checklist.yml` | This file |
| `VERSION_PLACEHOLDER` | setup_ci.rs:68 | `"@SS_MAGIC_PLUGIN_VERSION@"` | Must match the literal token inside `assets/workflow/checklist.yml` (not in this partition; cross-check there) | This file |
| `WORKFLOW_REL` | setup_ci.rs:75 | `".github/workflows/ss-magic-checklist.yml"` | Not reused elsewhere in Rust; referenced by name in CLAUDE.md prose and presumably in shell/CI docs (outside this partition) | This file |
| `PIN_KEY` | setup_ci.rs:80 | `"SS_MAGIC_PLUGIN_VERSION:"` | Must match the `env:` key inside `assets/workflow/checklist.yml` | This file |
| `FILE_MODE` (setup_ci) | setup_ci.rs:84 | `0o644` | Same value/name as `checklist/verbs.rs`'s `NEW_FILE_MODE` and `compact_window.rs`'s `NEW_FILE_MODE`; same NAME `FILE_MODE` as ~9 other files using `0o600` for state-tree files (bypass.rs:68, cache.rs:107, ledger.rs:101, scratchpad.rs:122, heartbeat.rs:73, expect_artifact.rs:93, hook/file_changed.rs:95, hook/subagent_stop.rs:86, hook/pre_compact.rs:67) | No shared owner today; see Leads L2 |
| `DIFF_LINE_BUDGET` | setup_ci.rs:89 | `120` | Not reused; comparable in spirit to `checklist/verbs.rs`'s `LIST_BYTE_BUDGET = 24_000` (different unit – lines vs bytes – for a similar "don't dump the whole thing" truncation) | This file |
| `USAGE` (setup_ci) | setup_ci.rs:92 | usage banner text | dup-name, see above | This file |

## Cross-module references

**Imports into this partition:**
- `main.rs` imports `ss_magic_core::style`; re-exports `ss_magic_core::{git, hashing}` as `pub(crate) use` so every sibling module's `crate::git::…` still resolves (pinned in structure-pins as deliberate, not duplication).
- `config.rs` imports `crate::git`, `crate::{compact_window, scratchpad, tmproot}`, `ss_magic_core::style`, `ss_magic_core::superset_files::{self, MagicConfig}`.
- `release_check.rs` imports `crate::atomic`, `crate::scratchpad::format_rfc3339`, `crate::status`, `crate::tmproot`, `ss_magic_core::release::{self, Cache, RefreshOutcome, ReleaseClient, UreqReleaseClient, PLUGIN_LINE}`, `ss_magic_core::style`.
- `setup_ci.rs` imports `crate::git`, `crate::atomic`, `crate::checklist::{ACTIONS_REL, CHECKLIST_SUFFIX}`, `ss_magic_core::style`.

**Where this partition's public symbols are used elsewhere (grepped, max 5 callers):**
- `HookEvent` – used only in `hook/event.rs` and `hook/mod.rs` (the routing table). Consistent with main.rs's own doc comment.
- `HumanVerb` – **used only within main.rs and its own tests.rs**; no other module references it (dispatch is entirely local to main.rs).
- `config::resolve` – `status.rs:927`, `status.rs:1102`.
- `config::resolve_with_roots` – `hook/mod.rs:513`.
- `config::PluginConfig` / `GateConfig` – `hook/pre_tool_use.rs`, `hook/mod.rs` (field access, not constructed there).
- `config::run_enable/run_disable/run_config/run_seed_config` – `main.rs:535-538` only.
- `config::SeedOutcome`, `seed_config_at` – used only inside config.rs (production) plus `config/tests.rs`; unused outside this file.
- `release_check::cached_plugin_tag` – `status.rs:1661`.
- `release_check::existing_cache_file` – `status.rs:1916`.
- `release_check::spawn_refresh`, `release_check::suggestion`, `release_check::record_suggested`, `release_check::notice` – all in `hook/session_start.rs` (lines 141, 404, 410, 412).
- `release_check::run` – `main.rs:540` only.
- `release_check::LOCK_NAME`, `REFRESH_ARGV`, `SCHEMA_VERSION` – **unused outside release_check.rs** despite being `pub`.
- `setup_ci::run` – `main.rs:531` only. `setup_ci::{TEMPLATE, VERSION_PLACEHOLDER, WORKFLOW_REL, render, pinned_version, classify, State, run_core}` – used only within setup_ci.rs and setup_ci/tests.rs; no other production module references setup_ci's internals.
- `status::locate_plugin_root`, `status::probe_harness`, `status::HarnessListing`, `status::read_pin_file` – imported INTO this partition by `release_check.rs:422-434` (reverse direction: status.rs is the owner, release_check.rs is the caller) – legitimate reuse, not duplication.

## Leads

| # | Category | File:line(s) | Hypothesis | Compare against |
|---|---|---|---|---|
| L1 | copy-paste-variant | config.rs:938-942 vs cache.rs:1006-1009 vs ledger.rs:1250-1253 vs compact_window.rs:837-840 vs checklist/verbs.rs:1438-1442 | Five near-identical `usage_error` functions ("print `error: {message}`, print usage, return exit-2"), but with **inconsistent argument order**: `config.rs`'s is `fn usage_error(usage: &str, message: &str)` while all four others are `fn usage_error(message: &str[, usage: &str])`. A caller moved between files would silently swap which string prints first. | All five sites; candidate for one shared helper (e.g. in `ss_magic_core::style` or a new small crate-local module) with one fixed argument order. |
| L2 | const-location | setup_ci.rs:84 (`FILE_MODE = 0o644`) and bypass.rs:68, cache.rs:107, ledger.rs:101, scratchpad.rs:122, heartbeat.rs:73, expect_artifact.rs:93, hook/file_changed.rs:95, hook/subagent_stop.rs:86, hook/pre_compact.rs:67 (`FILE_MODE = 0o600`), plus compact_window.rs:132 and checklist/verbs.rs:111 (`NEW_FILE_MODE = 0o644`) | Every writer module hand-declares its own `FILE_MODE`/`NEW_FILE_MODE` constant rather than sharing two named constants (one for owner-only state, one for committed/world-readable content) from a common module (`atomic.rs`, which already owns the shared `write_atomically`). | `atomic.rs`'s `write_atomically` signature (`Option<u32>` mode param) – the mode values themselves are the natural constants to hoist there. |
| L3 | reuse | config.rs:869-897 (`landing`) | Canonicalize-root, canonicalize-deepest-existing-component, `starts_with` containment check – the same shape recurs in `scratchpad.rs`'s `verify_contained` (~scratchpad.rs:341-360) and in `expect_artifact.rs`'s path resolution (~expect_artifact.rs:349-400). Each is independently hand-rolled for its own "does this resolve inside the root" question. | `scratchpad.rs::verify_contained`, `expect_artifact.rs` containment logic (both outside this partition – cross-partition lead). |
| L4 | naming | release_check.rs:53 (`pub const LOCK_NAME`), checklist/verbs.rs:105 (`const LOCK_NAME`), hook/file_changed.rs:106 (`const LOCK_NAME`) | Three unrelated lock-name constants share the identifier `LOCK_NAME` crate-wide (plus `CONFIG_LOCK_NAME`, `INSTALL_LOCK_NAME`, `POINTER_LOCK_NAME` elsewhere using distinct names for the same role). Not a bug – each is scoped to its own module – but the inconsistent naming convention (`LOCK_NAME` vs `<THING>_LOCK_NAME`) makes grepping for "all the plugin's lock files" unreliable. | Every `*_LOCK_NAME`/`LOCK_NAME` const in `crates/ss-magic-plugin/src/`. |
| L5 | copy-paste-variant | release_check.rs:646-668 (`Flags`/`ParsedArgs`/`parse_args`) vs setup_ci.rs:199-220 (manual `for arg in args { match ... }`) vs config.rs:608-621 (`run_toggle`'s `match args { [] => ..., [flag] if ... => ..., _ => ... }`) | Three different idioms for "parse this verb's flags, handle `-h`/`--help`, reject anything else with usage+exit 2" inside the SAME crate. None is wrong, but a reviewer fixing a flag-parsing bug in one has no reason to notice the other two use a different pattern. | `compact_window.rs`'s and `checklist/verbs.rs`'s own flag parsers (outside this partition – cross-partition lead) – worth checking whether ALL verb parsers in the crate could converge on one of these three shapes. |
| L6 | dead-code | main.rs:247-267 (`HumanVerb::as_str`), main.rs:285-290 (`writes_config`), main.rs:295-297 (`can_set_enabled`) | All three are `pub` with doc comments stating "no production caller" / "asserted only by tests"; `HumanVerb` itself (the whole enum) is referenced nowhere outside `main.rs` and `main.rs`'s own `tests.rs`. Confirmed by grep: no other module imports `HumanVerb`. | `main.rs`'s `tests.rs` (the sole consumer) – decide whether these should be `#[cfg(test)]`-gated instead of `pub`. |
| L7 | dead-code | config.rs:82-96 (`GATE_THRESHOLD_LINES_DEFAULT/MIN/MAX`, `GATE_INLINE_BYTE_BUDGET_DEFAULT/MIN/MAX`) | All six are `pub` but grepping the whole `crates/` tree finds no reader outside `config.rs` and `config/tests.rs`/`status/tests.rs`. `status.rs` reads the resolved `GateConfig` struct fields instead of these raw bounds. | Whole-workspace grep already run; nothing else references them – candidate to demote to `pub(crate)` or private. |
| L8 | reuse | release_check.rs:575-635 (`render_text`, local `const WIDTH: usize = 22` + inline `row` closure) | Re-implements the exact "left-pad a label column, print value" idiom that `status.rs` already has as three named, reusable helpers: `row` (status.rs:1966), `row_opt` (status.rs:1971), `row_bool` (status.rs:1979) – with their own `LABEL_WIDTH`. `release_check.rs` could call those instead of re-deriving the pattern with a different local width (22 vs status's own constant). | `status.rs:1966-1990` (outside this partition's file set but named directly in this partition's code via `status::locate_plugin_root` etc., so the two files already know about each other). |
| L9 | dup-name / const-location | release_check.rs:60 (`SCHEMA_VERSION = 1`) vs status.rs:74 (`SCHEMA_VERSION = 2`) | Two unrelated `--json` report schemas share the bare identifier `SCHEMA_VERSION` crate-wide. Not a correctness bug (each is scoped to its module and versions its own report), but a reader searching "the schema version" will find two different meanings under the same name with no qualifying prefix. | `status.rs:74` (its own partition) – consider `RELEASE_CHECK_SCHEMA_VERSION` / `STATUS_SCHEMA_VERSION` naming, or leave as-is and just document the collision (per structure-pins, do not propose collapsing them into one shared version). |
| L10 | efficiency / naming | main.rs:501-518 (`dispatch`, `run_hook`) | `dispatch` has exactly one call site (`run`) and `run_hook` has exactly one call site (`dispatch`) and exactly one callee (`hook::run`) with no added logic beyond the doc comment. Both are one-line pass-throughs; inlining `run_hook`'s body into `dispatch` (or `dispatch` into `run`) would not lose any behavior or testability that isn't already covered by testing `run`/`hook::run` directly. Low priority – the indirection also documents the fail-open contract at the call site, which has value. | N/A (self-contained to main.rs); reviewer judgment call on whether the doc-comment value outweighs the indirection. |
| L11 | hardcoded-value | setup_ci.rs:89 (`DIFF_LINE_BUDGET = 120`) vs checklist/verbs.rs:~116 (`LIST_BYTE_BUDGET = 24_000`) | Both are "don't dump the whole rendered thing, truncate and say how much more there is" budgets guarding a similar failure mode (a large generated/rendered document flooding stdout or a model's context), but measured in different units (lines vs bytes) with no shared naming convention or shared truncation helper. | `checklist/verbs.rs`'s `LIST_BYTE_BUDGET` usage and truncation logic (outside this partition – cross-partition lead); consider whether a shared "print up to N lines/bytes then summarize" helper is warranted given `status.rs` likely has similar concerns for its own long sections. |
| L12 | duplication | config.rs:823-851 (`write_plugin_key`) calls `landing()` at :826, and `seed_config_at` (config.rs:465-499) calls `landing()` at :474 | This double-call is EXPLAINED by the module doc as deliberate (seed needs an outcome, the writer needs an error) and is listed in structure-pins as a non-finding. Recorded here only so the reviewer does not re-flag it; **do not report as a new lead**. | structure-pins.md, "Deliberate duplications" section, `seed_config_at calls landing() before write_plugin_key calls it again`. |
