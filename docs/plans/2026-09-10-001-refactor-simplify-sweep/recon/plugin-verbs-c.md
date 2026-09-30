# Recon: plugin-verbs-c – compact_window, ledger

## Map

### `crates/ss-magic-plugin/src/compact_window.rs`
Purpose: `compact-window` verb – read-only `--recommend` report on the auto-compact window, plus the single opt-in `--set` write to `.claude/settings.local.json`.

- `run (pub) – fn(args: &[String]) -> Result<ExitCode>` – dispatches parsed args to `run_core` / `run_recommend` / usage.
- `ParsedArgs (private enum, 176)` – `Set(u64) | Recommend{json} | Help | Error(String)`.
- `parse_args (private, 193) – fn(&[String]) -> ParsedArgs` – hand-rolled arg parser for this verb only.
- `run_core (private, 222) – fn(root: &Path, window: u64) -> Result<ExitCode>` – the one write path.
- `ExistingSettings (private enum, 272)` – `Absent | Object(Map) | NotAnObject | Malformed(String)`.
- `read_settings_object (private, 296) – fn(&Path) -> Result<ExistingSettings>` – generic "read a JSON-object settings file, report shape problems as values" helper. Small and generic; candidate for reuse/duplication check (see Leads).
- `write_settings_object (private, 319) – fn(&Path, &Map) -> Result<()>` – pretty-print + preserve mode + atomic write.
- `Sources (pub struct, 349)` + `impl Sources::from_process (pub, 362)` – injected environment/paths for the recommend report (testability seam, pinned).
- `managed_settings_path (private, 377) – fn() -> Option<PathBuf>` – per-OS path constant lookup.
- `Report / OverrideReport / OverrideLocation / Windows / WindowSetting / Recommendation / CommandLine (pub structs, 394-486)` – the `--recommend` JSON/text shape.
- `recommend (pub(crate), 494) – fn(peaks: &[u64]) -> Recommendation` – R25 arithmetic (1.25x, round to 10k, clamp).
- `no_recommendation (private, 528) – fn(note: &str) -> Recommendation` – the "no number" shape, small generic-looking helper.
- `detect_override (pub(crate), 548) – fn(root, sources) -> OverrideReport` – scans process env + every settings file's `env` block for `OVERRIDE_ENV`.
- `env_value_in (private, 608) – fn(&Path) -> Option<String>` – small, generic-shaped (read one key out of a JSON `env` block).
- `read_window (pub(crate), 620) – fn(root, rel) -> WindowSetting` – reads `autoCompactWindow` from one file.
- `window_configured (pub(crate), 645) – fn(root) -> bool` – true if either settings file has a window.
- `enable_tip (pub(crate), 654) – fn(root) -> Option<String>` – one-line nudge printed by `config::enable`.
- `recommend_report (pub, 670) – fn(root, main_root, store, sources) -> Report` – assembles the whole report, calling into `ledger::rows_for_repository`.
- `run_recommend (private, 730) – fn(cwd, json) -> Result<ExitCode>` – real-process wiring for `recommend_report`.
- `render_text (pub(crate), 754) – fn(&Report) -> String` – human-readable rendering into a buffer.
- `usage_error (private, 837) – fn(&str) -> ExitCode` – prints usage + error, exit 2.
- `fail (private, 846) – fn(String) -> ExitCode` – prints error, exit 2. Near-duplicate of `usage_error` (see Leads) and of `checklist/verbs.rs::fail`.

### `crates/ss-magic-plugin/src/ledger.rs`
Purpose: `cost.jsonl` ledger (one row per session id, incrementally scanned from transcripts) and the `cost` verb that reports it.

- `LEDGER_FILE_NAME / OFFSETS_FILE_NAME / PRICES_DIR_NAME (pub consts)` – file/dir names inside the machine store.
- `price_for (private, 140) – fn(model: &str) -> Option<(f64,f64)>` – longest-prefix match into `PRICES`.
- `price_tokens (private, 151) – fn(model, &Tokens) -> Option<f64>` – prices one `Tokens` value.
- `price_table_snapshot (private, 166) – fn() -> Value` – self-describing JSON of the price table.
- `Tokens (pub struct, 193)` + `impl Tokens::add (private, 205)` – token-count accumulator.
- `Basis (pub enum, 217)` + `impl Basis::label (private, 228)`.
- `Row (pub struct, 239)` – the ledger line shape.
- `Mark (private struct, 318)`, `Offsets (private type alias, 331)`.
- `read_offsets (private, 335)`, `write_offsets (private, 348)` – offsets-store I/O.
- `ledger_path (pub, 368) – fn(store) -> PathBuf`.
- `read (pub, 376) – fn(store) -> Result<Vec<Row>>` – tolerant line-by-line JSONL read.
- `rows_for_repository (pub, 407) – fn(store, main_root, limit) -> Result<Vec<Row>>` – population for the compact-window recommendation.
- `root_belongs_to (private, 436) – fn(root, main_root) -> bool`.
- `transcript_tree (pub, 454) – fn(transcript) -> Vec<PathBuf>` – main file + subagent `.jsonl`s under the sibling dir.
- `collect_jsonl (private, 474) – fn(dir, out)` – recursive, sorted, symlink-safe walk. Small and generic (candidate for reuse/comparison, see Leads).
- `Scan (private struct, 499)` – accumulator for one scan pass.
- `USAGE_NEEDLE / COST_STATE_NEEDLE / HEAD_SCAN_BYTES / READ_BUF_BYTES (private consts, 541-555)`.
- `scan_tree (private, 569)`, `plan_starts (private, 641)`, `scan_file (private, 671)`, `read_line (private, 709)`, `read_cost_state (private, 733)`, `read_usage (private, 748)`, `contains (private, 807)` – the transcript-scanning pipeline. `contains` (naive byte substring search, 807) is small and fully generic – a duplication candidate.
- `Ingest (pub struct, 816)`, `Recorded (pub enum, 835)`.
- `record (pub, 851) – fn(store, &Ingest) -> Result<Recorded>` – the public entry point the `SessionEnd` hook calls.
- `commit (private, 885)`, `find_row (private, 925)`, `build_row (private, 931)`, `resolve_roots (private, 1014)`, `dominant (private, 1070)` – the locked commit path and row assembly.
- `ensure_store (private, 1080)`, `append_row (private, 1093)`, `replace_row (private, 1112)`, `snapshot_prices (private, 1137)` – writers.
- `transcript_root (private, 1162) – fn() -> Option<PathBuf>` – `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`. Near-duplicate of `spill_index::projects_root` (see Leads – that function's own doc comment names this one).
- `find_transcript (private, 1175) – fn(root, session_id) -> Option<PathBuf>`.
- `COST_USAGE (private const, 1190)`.
- `run (pub, 1210) – fn(args) -> Result<ExitCode>` – hand-rolled arg parser (different shape/style from `compact_window::parse_args`).
- `usage_error (private, 1250) – fn(&str) -> Result<ExitCode>` – note: returns `Result<ExitCode>`, unlike every other `usage_error` in the crate which returns bare `ExitCode` (see Leads – naming/signature inconsistency).
- `run_backfill (private, 1263)`, `row_summary (private, 1347)`.
- `Group (private struct, 1359)`, `group_rows (private, 1370)`, `report (private, 1402)`.
- `plural (private, 1525) – fn(n: usize) -> &'static str` – exact duplicate of `spill_index::plural` (see Leads).
- `compact (private, 1534) – fn(n: u64) -> String` – token-count abbreviation (`k`/`M`); no duplicate found elsewhere in the workspace at time of writing, but same shape as many "compact a count" helpers – worth a grep in later partitions.

## Constants and literals

| Item | file:line | Value | Elsewhere in workspace? | Owning definition |
|---|---|---|---|---|
| `SETTINGS_LOCAL_REL` | compact_window.rs:89 | `.claude/settings.local.json` | Also appears literally in `checklist/verbs.rs` region per literals.md (`tests.rs:627`,`1192` are this file's own tests) – no other production file. | Here; correctly the one owner (R31). |
| `SETTINGS_PROJECT_REL` | compact_window.rs:93 | `.claude/settings.json` | `status.rs` reads/writes a *different* meaning of "settings.json" (its own literal, see numbers/literals index row for `settings.json` – 10 occurrences total across files, mostly in tests) | This file for the compact-window use; status.rs likely has its own copy for a different purpose – check status.rs partition. |
| `OVERRIDE_ENV` | compact_window.rs:100 | `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` | Used in `hook/session_start.rs:29,137` via import, not re-literalized. Good – one owner. | Here. |
| `RECOMMEND_ROWS` | compact_window.rs:105 | `20` | Not found duplicated elsewhere. | Here. |
| `HIGH_CONFIDENCE_ROWS` | compact_window.rs:108 | `3` | Not duplicated. | Here. |
| `ROUNDING` | compact_window.rs:111 | `10_000` | Not duplicated. | Here. |
| `RECOMMEND_SCHEMA_VERSION` | compact_window.rs:115 | `1` | Schema-version pattern repeated elsewhere in the crate (e.g. `status.rs`'s `SCHEMA_VERSION = 2`) but each is its own surface's version – not a duplicate value, a repeated *pattern*. | Here. |
| `WINDOW_KEY` | compact_window.rs:118 | `autoCompactWindow` | literals.md shows this string at 8 sites total, all within compact_window.rs and its own tests.rs. No leak elsewhere. | Here. |
| `WINDOW_MIN` / `WINDOW_MAX` | compact_window.rs:123,125 | `100_000` / `1_000_000` | Restated as plain numbers in `USAGE` text at compact_window.rs:143 and in doc comments (lines 34, 511, 535, 709) instead of via `{WINDOW_MIN}`/`{WINDOW_MAX}` interpolation in a couple spots – actually most uses DO interpolate (`format!` with `{WINDOW_MIN}-{WINDOW_MAX}`), but the doc-comment prose at line 34 (`100,000–1,000,000`) and the literal `USAGE` string at line 143 (`100000-1000000`) are hand-typed copies that must be kept in sync by hand. | Here. |
| `NEW_FILE_MODE` | compact_window.rs:132 | `0o644` | Same value/name pattern at `checklist/verbs.rs:111`. Two independent consts, same name, same value, same "brand-new file gets ordinary permissions" rationale. | Candidate for a shared helper/const if a "harness-owned file mode" concept is wanted – flagged in Leads, not clearly wrong per structure-pins (each module owns its own consts). |
| `USAGE` | compact_window.rs:134 | multi-line usage string | Pattern repeated per-verb across the crate (every verb has its own `USAGE`/`*_USAGE` const) – deliberate, not a finding. | Here. |
| `LEDGER_FILE_NAME` | ledger.rs:82 | `cost.jsonl` | literals.md: 2 occurrences total (here + its own tests.rs). | Here, pub but unused outside this file (see Cross-module references). |
| `OFFSETS_FILE_NAME` | ledger.rs:86 | `transcript-offsets.json` | Not duplicated. | Here, pub but unused outside file. |
| `PRICES_DIR_NAME` | ledger.rs:90 | `prices` | Not duplicated (generic name, but only referenced here). | Here, pub but unused outside file. |
| `LOCK_FILE_NAME` | ledger.rs:96 | `cost.lock` | Same *name*, different value, at `heartbeat.rs:67` (`hooks.lock`) and `ss-magic/src/update/apply.rs:80` (`update.lock`) – a repeated per-module naming convention, not a duplicated value. | Here. |
| `DIR_MODE` | ledger.rs:100 | `0o700` | Same name+value repeated in `bypass.rs:67`, `cache.rs:106`, `expect_artifact.rs:92`, `heartbeat.rs:71`, `hook/subagent_stop.rs:85`, `scratchpad.rs:119`, `tmproot.rs:101` – 8 modules total, all `0o700`. | No single owner; each module redeclares it. Flagged as a const-location lead. |
| `FILE_MODE` | ledger.rs:101 | `0o600` | Same name+value in `bypass.rs:68`, `cache.rs:107`, `expect_artifact.rs:93`, `heartbeat.rs:73`, `hook/file_changed.rs:95`, `hook/pre_compact.rs:67`, `hook/subagent_stop.rs:86`, `scratchpad.rs:122` – 8 more modules, all `0o600`. `setup_ci.rs:84` uses the same name for `0o644` (different value – a naming collision, not a value duplicate). | Same as above. |
| `PRICE_TABLE_VERSION` | ledger.rs:109 | `2026-06-24` | Not duplicated; used in tests. | Here. |
| `PRICES` | ledger.rs:119 | model→(input,output) table | Not duplicated elsewhere in the workspace (grepped model id strings). | Here – sole home of pricing. |
| `CACHE_READ_MULT` / `CACHE_WRITE_5M_MULT` / `CACHE_WRITE_1H_MULT` | ledger.rs:132,134,137 | `0.10` / `1.25` / `2.00` | Not duplicated. | Here. |
| `USAGE_NEEDLE` | ledger.rs:541 | `"usage"` | Not duplicated. | Here. |
| `COST_STATE_NEEDLE` | ledger.rs:546 | `b"\"cost-state\""` | Not duplicated. | Here. |
| `HEAD_SCAN_BYTES` | ledger.rs:551 | `256` | Not duplicated. | Here. |
| `READ_BUF_BYTES` | ledger.rs:555 | `256 * 1024` | Not duplicated as a named const; numbers.md flags a bare `64 * 1024` at ledger.rs:683 (the per-line `buf` capacity) as a related, smaller, unnamed buffer size in the same function family. | Here; the 683 literal could be named or explained relative to this one (see Leads). |
| `COST_USAGE` | ledger.rs:1190 | multi-line usage string | Per-verb pattern, not a finding. | Here. |

## Cross-module references

**Imports into `compact_window.rs`:** `crate::git`, `crate::git::gitignore::{self, PathKind}`, `crate::{atomic, heartbeat, ledger}`, `ss_magic_core::style`, and `crate::status::non_empty_env` (aliased locally as `non_empty` inside `Sources::from_process`, line 363).

**Imports into `ledger.rs`:** `crate::git`, `crate::atomic`, `crate::heartbeat`, `crate::scratchpad::{format_rfc3339, now_secs}`, `crate::tmproot`, `ss_magic_core::style`.

**Public symbols and their outside callers:**

- `compact_window::run` – `main.rs:539` (`HumanVerb::CompactWindow => compact_window::run(args)`). Only caller.
- `compact_window::recommend_report` – `status.rs:993`. Only caller outside this file.
- `compact_window::Sources` / `Sources::from_process` – `status.rs:553` (field type), `status.rs:1912` (constructed). Only caller.
- `compact_window::window_configured` – `hook/session_start.rs:260`. Only caller.
- `compact_window::enable_tip` – `config.rs:650`. Only caller.
- `compact_window::OVERRIDE_ENV` – `status.rs:1056`, `hook/session_start.rs:29,137`. Two callers.
- `compact_window::recommend`, `detect_override`, `read_window`, `render_text` (`pub(crate)`) – used within the crate; `recommend`/`detect_override`/`read_window` are exercised primarily from this file's own tests plus `recommend_report`; `render_text` is called only from `run_recommend` in this file (740) – "unused outside this file" as a *caller*, though it is `pub(crate)` and covered by `tests.rs`.
- `compact_window::RECOMMEND_ROWS` – used inside this file only (line 691, `ledger::rows_for_repository(store, main_root, RECOMMEND_ROWS)`). Unused outside this file.
- `compact_window::RECOMMEND_SCHEMA_VERSION` – used inside this file only (line 716). Unused outside this file.

- `ledger::run` – `main.rs:530` (`HumanVerb::Cost => ledger::run(args)`). Only caller.
- `ledger::record` – `hook/session_end.rs:93`. Only caller.
- `ledger::Ingest` / `ledger::Recorded` – imported and used at `hook/session_end.rs:47,85,94,102,105`. Only caller module.
- `ledger::rows_for_repository` – `compact_window.rs:691`. Only caller.
- `ledger::read` – no production caller outside this file; used only from `hook/session_end/tests.rs` (151,185,201,222,244,264,281) and this file's own tests/internal calls (421,862,886,1403). **Unused outside this file in production code** – its only outside consumers are tests.
- `ledger::ledger_path` – no production caller outside this file; used from `ledger/tests.rs` only. **Unused outside this file in production code.**
- `ledger::LEDGER_FILE_NAME`, `OFFSETS_FILE_NAME`, `PRICES_DIR_NAME`, `PRICE_TABLE_VERSION`, `transcript_tree` (all `pub`) – grepped with no non-test, non-self match. **All unused outside this file** (pub visibility appears broader than actual use; check their tests.rs for direct construction before calling this dead code – some may be intentionally `pub` only for the sibling tests module, which is allowed access anyway via `mod tests;` and would not need `pub`).

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `ledger.rs:1525-1531` vs `spill_index.rs:459-465` | `fn plural(n: usize) -> &'static str` is byte-for-byte identical in both files (`if n == 1 { "" } else { "s" }`). | `crates/ss-magic-plugin/src/spill_index.rs:459` |
| L2 | copy-paste-variant | `compact_window.rs:846-849` vs `checklist/verbs.rs:1456-1459` | Both define `fn fail(message: String) -> ExitCode` with the identical doc-comment rationale ("a refusal that is not a usage mistake ... same exit code") and near-identical bodies; `checklist`'s calls a local `refused()` while `compact_window`'s inlines `ExitCode::from(2)`. | `crates/ss-magic-plugin/src/checklist/verbs.rs:1438-1459` |
| L3 | naming / copy-paste-variant | `ledger.rs:1250` vs `compact_window.rs:837,846` vs `checklist/verbs.rs:1438,1456` vs `cache.rs:1006` vs `config.rs:938` | Five `usage_error`/`fail`-shaped helpers across the crate with inconsistent signatures: `ledger::usage_error(&str) -> Result<ExitCode>`, `compact_window::usage_error(&str) -> ExitCode`, `config::usage_error(usage: &str, message: &str) -> ExitCode`, `cache::usage_error(message: &str, usage: &str) -> Result<ExitCode>` (note the swapped argument order vs config's). A reader moving between verbs must recheck argument order and return type each time. | `crates/ss-magic-plugin/src/cache.rs:1006`, `crates/ss-magic-plugin/src/config.rs:938` |
| L4 | duplication | `ledger.rs:1162-1170` (`transcript_root`) vs `spill_index.rs:116-123` (`projects_root`) | Near-identical bodies resolving `$CLAUDE_CONFIG_DIR/projects` else `~/.claude/projects`; `spill_index::projects_root` is `pub` and its own doc comment (spill_index.rs:113-114) explicitly says it is "Deliberately the same resolution `plugin::ledger` uses" – i.e. the duplication is acknowledged but not actually shared. `ledger::transcript_root` could call `spill_index::projects_root` instead of recomputing. | `crates/ss-magic-plugin/src/spill_index.rs:109-123` |
| L5 | dead-code | `ledger.rs:82,86,90,109` (`LEDGER_FILE_NAME`, `OFFSETS_FILE_NAME`, `PRICES_DIR_NAME` unused outside file; `PRICE_TABLE_VERSION` used only in tests) and `ledger.rs:368` (`ledger_path`), `ledger.rs:376` (`read`), `ledger.rs:454` (`transcript_tree`) | These are `pub` (crate-visible-and-beyond) but have no caller outside `ledger.rs` in production code – only this file's own code and `tests.rs` files use them. Either they should be `pub(crate)`/private, or there is a planned external consumer not yet wired up. | grep confirms zero non-test external call sites as of this recon |
| L6 | const-location | `ledger.rs:100-101` (`DIR_MODE`/`FILE_MODE` = `0o700`/`0o600`) | The identical name+value pair is redeclared in 8 other plugin modules (`bypass.rs:67-68`, `cache.rs:106-107`, `expect_artifact.rs:92-93`, `heartbeat.rs:71,73`, `hook/subagent_stop.rs:85-86`, `scratchpad.rs:119,122`, `tmproot.rs:101`, plus `hook/file_changed.rs:95` and `hook/pre_compact.rs:67` for `FILE_MODE` alone). Ten-plus copies of "owner-only dir/file mode" across the crate; a single `state_mode` module-level pair (or reuse of an existing one) would remove the repetition. Not explicitly pinned in structure-pins, so worth flagging even though each individual const is legitimate on its own. | every module listed above; the full list belongs in a cross-partition summary since it spans many partitions |
| L7 | naming | `ledger.rs:96` (`LOCK_FILE_NAME = "cost.lock"`) vs `heartbeat.rs:67` (`LOCK_FILE_NAME = "hooks.lock"`) vs `ss-magic/src/update/apply.rs:80` (`LOCK_FILE_NAME = "update.lock"`) | Same constant *name* reused per-module for a different literal value each time. Not a bug (each is module-private), but a reader grepping `LOCK_FILE_NAME` gets three unrelated hits; consider a `Lock` newtype or per-module rename if this pattern grows further. | cross-partition, informational only |
| L8 | hardcoded-value | `compact_window.rs:143` (`USAGE` text: `100000-1000000 tokens`) vs `compact_window.rs:123,125` (`WINDOW_MIN`/`WINDOW_MAX`) | The `USAGE` const's prose hand-types the bound values instead of using `concat!`/`format!` with `{WINDOW_MIN}`/`{WINDOW_MAX}` the way the runtime error messages do (e.g. line 199-200). If the bounds ever change, this line is easy to miss. | `compact_window.rs:196-204` (the `parse_args` error messages, which DO interpolate) |
| L9 | duplication | `ledger.rs:683` (`let mut buf = Vec::with_capacity(64 * 1024);`) vs `ledger.rs:555` (`const READ_BUF_BYTES: usize = 256 * 1024;`) | Two different, unrelated buffer-size literals in the same scan pipeline (one for the `BufReader`, one for the per-line `Vec` inside `scan_file`) with no named constant for the second. Not necessarily wrong, but the size relationship between the two (4x) is unexplained and the smaller one is a bare literal where the file otherwise names every tuning constant. | `ledger.rs:671-706` (`scan_file`) |
| L10 | reuse | `ledger.rs:1163-1166` (`transcript_root`'s hand-rolled `std::env::var("CLAUDE_CONFIG_DIR").ok().filter(...)`) vs `status.rs:878` (`pub(crate) fn non_empty_env`) already reused by `compact_window.rs:363-366` | `compact_window::Sources::from_process` reuses `status::non_empty_env` for the same "non-empty env var" idiom, but `ledger::transcript_root` (and `spill_index::projects_root`, see L4) reimplement it inline instead of calling the shared helper. | `crates/ss-magic-plugin/src/status.rs:878-884`, `crates/ss-magic-plugin/src/compact_window.rs:362-366` |
| L11 | naming | `compact_window.rs:132` (`NEW_FILE_MODE = 0o644`) vs `checklist/verbs.rs:111` (`NEW_FILE_MODE = 0o644`) | Same name, same value, same "brand-new committed file gets ordinary mode" rationale, declared independently in two files. Lower-priority than L6 (only two sites, and the two files write different kinds of files – one a harness settings file, one a checklist document) but worth the reviewer's eye alongside L6. | `crates/ss-magic-plugin/src/checklist/verbs.rs:105-115` |
| L12 | efficiency | `ledger.rs:407-434` (`rows_for_repository`) | For every distinct `root`/`also_roots` value across the whole ledger, `belongs` calls `git::discover::roots` (a filesystem walk) via the memoizing closure – memoization is per-call already (good), but the function first calls `read(store)?` which parses the ENTIRE ledger (every session ever recorded) before filtering, even though only `limit` (20 from `compact_window::RECOMMEND_ROWS`) rows are ultimately kept. On a long-lived machine with thousands of sessions across many repos this reads and JSON-parses the whole file every time `--recommend` runs. Not clearly wrong (the file is line-delimited and `read` is already tolerant), but flagged for the reviewer since `sort_by`+`truncate` happens only after the full read+filter. | `ledger.rs:376-386` (`read`), `ledger.rs:1402-1403` (`report`, which also reads the whole ledger – same shape, so this may just be the established pattern for this module, not a defect) |
| L13 | naming/quality | `ledger.rs:1210-1249` (`run`) vs `compact_window.rs:193-218` (`parse_args`) | Two different hand-rolled argv parsing styles for sibling verbs in the same crate: `ledger::run` parses inline with a `while let Some(arg) = rest.next()` loop mutating local `bool`/`Option` flags, while `compact_window` parses via `match args { [..] => ... }` slice patterns into a typed `ParsedArgs` enum. Neither is wrong, and CLAUDE.md's "no clap" convention applies to both, but the stylistic split is worth surfacing since a reviewer skimming both files for "how does this crate parse args" gets two different answers. | `compact_window.rs:193-218` |
