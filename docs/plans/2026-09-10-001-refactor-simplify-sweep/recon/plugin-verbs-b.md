# Recon: plugin-verbs-b (status, spill_index)

## Map

### `crates/ss-magic-plugin/src/status.rs` (2347 lines)
Purpose: `ss-magic-plugin status` – builds and renders the "why is the plugin doing nothing" diagnostic report; read-only, never writes state.

Public / crate-visible items first:

- `SCHEMA_VERSION` (pub const, u32 = 2) – line 74 – version of the `--json` shape.
- `DECLARED_EVENTS` (pub const, `[&str; 5]`) – line 82 – the events the shipped manifest registers.
- `PIN_FILE` (pub(crate) const, &str) – line 106 – filename of the plugin's version pin.
- `read_pin_file` (pub(crate) fn) – `(plugin_root: &Path) -> Result<(String, PathBuf), String>` – line 113 – reads and trims the pin file, `Err` carrying a note.
- `Field` (pub struct) – line 165 – a value/source/note triple; `None` value always paired with a note. Small, generic, candidate for reuse (see Leads).
  - `Field::found` (private) – line 173 – constructor for a determined value.
  - `Field::missing` (private) – line 182 – constructor for an undetermined value.
  - `Field::render` (private) – line 191 – text-row rendering.
- `Located` (pub struct) – line 209 – like `Field` but keeps a `PathBuf` to join onto.
  - `Located::found` (private) – line 216 – same shape as `Field::found`.
  - `Located::missing` (private) – line 224 – same shape as `Field::missing`.
  - `Located::as_field` (private) – line 232 – converts to `Field`.
- `Status` (pub struct) – line 245 – the whole report, field order = read order.
- `Repo`, `IdentityRow`, `Enablement`, `SsMagicLayer`, `HarnessLayer`, `Registration`, `StateTree`, `StateDirs`, `Gate`, `Compaction`, `Bootstrap`, `Binary`, `BootstrapOutcome`, `Markers`, `Versions`, `Hooks`, `EventStatus`, `Counts` (pub structs) – lines 270–526 – the report's sub-shapes, one per section. `Gate` (line 375) mirrors `config::GateConfig`'s three fields verbatim plus provenance (see Leads).
- `Inputs` (pub struct) – line 537 – everything `collect` would otherwise read from the environment, injected for testability.
- `Probes` (pub struct) – line 562 – the harness listing plus the version-probe closure.
- `HarnessListing` (pub enum) – line 572 – `Loaded(Vec<Registration>)` or `Unavailable(String)`.
- `parse_listing` (pub fn) – `(text: &str) -> Result<Vec<Registration>, String>` – line 614 – parses `claude plugin list --json` output, tolerant of the array-or-`{plugins:[...]}` shape.
- `snippet` (private fn, small/generic) – `(text: &str) -> String` – line 672 – first-line truncation for an error message. Candidate for reuse if another module truncates text for an error (none found in this partition).
- `probe_harness` (pub fn) – `() -> HarnessListing` – line 689 – runs `claude plugin list --json` bounded.
- `probe_binary_version` (pub fn) – `(bin: &Path) -> Result<String, String>` – line 702 – runs `<bin> --version` bounded.
- `parse_version_line` (pub fn) – `(text: &str) -> Option<String>` – line 716 – `"ss-magic 0.10.0"` → `"0.10.0"`.
- `run_bounded` (private fn) – `(cmd: Command, timeout: Duration, what: &str) -> Result<String, String>` – line 733 – spawn + poll + kill-on-timeout subprocess runner, output via a temp file (never a pipe, to avoid deadlock). No duplicate elsewhere in the workspace (only `try_wait` caller in the crate).
- `locate_data_dir` (pub fn) – `() -> Located` – line 793 – resolves `${CLAUDE_PLUGIN_DATA}`, falling back to the tmproot `data-root` pointer, then the documented layout.
- `read_data_root_pointer` (private fn) – `() -> Option<(String, PathBuf)>` – line 830 – reads the bootstrap's `data-root` file under both `/tmp` and `$TMPDIR`.
- `locate_plugin_root` (pub fn) – `(harness: &HarnessListing) -> Located` – line 853 – resolves `${CLAUDE_PLUGIN_ROOT}`, falling back to the harness registration's `installPath`.
- `non_empty_env` (pub(crate) fn, small/generic) – `(name: &str) -> Option<String>` – line 878 – env lookup that treats `""` as absent. Shared with `compact_window::Sources::from_process` (cross-module use, see below) – a small generic helper that is a natural home for further env-lookup duplication.
- `collect` (pub fn) – `(inputs: &Inputs, probes: &Probes) -> Status` – line 885 – builds the whole report; never fails.
- `collect_compaction` (private fn) – line 985 – wraps `compact_window::recommend_report` into the `Compaction` section and raises the R27 problem.
- `collect_identity` (private fn) – line 1070.
- `collect_enablement` (private fn) – line 1096 – both enablement layers plus registrations.
- `collect_state_tree` (private fn) – line 1238.
- `collect_bootstrap` (private fn) – line 1327.
- `read_marker` (private fn, small/generic) – `(dir: &Path, name: &str) -> Option<String>` – line 1441 – trimmed marker-file read, `None` on empty/missing. Generic enough to be reused for any "read a small marker file" need.
- `derive_outcome` (private fn) – line 1453.
- `collect_versions` (private fn) – line 1539.
- `collect_newest_release` (private fn) – line 1638 – reads the plugin release cache file only, never the network.
- `parse_semver` (private fn) – `(text: &str) -> Option<(u64,u64,u64)>` – line 1690 – thin wrapper over `release::parse_bare_triple`.
- `collect_hooks` (private fn) – line 1697 – per-event history from the heartbeat log, worktree/machine scoped.
- `outcome_name` (private fn, small/generic) – `(outcome: Outcome) -> &'static str` – line 1848.
- `STATUS_USAGE` (private const, &str) – line 1860.
- `run` (pub fn) – `(args: &[String]) -> Result<ExitCode>` – line 1877 – the verb entry point.
- `heartbeat_store_if_present` (private fn) – line 1932 – non-creating store lookup, mirrors `heartbeat::existing_store_dir` shared with `compact-window --recommend`.
- `emit` (private fn) – `(status: &Status, json: bool) -> Result<()>` – line 1941.
- `LABEL_WIDTH` (private const, usize = 22) – line 1953.
- `unknown` (private fn, small/generic) – `(note: Option<&str>) -> String` – line 1958 – "unknown – {reason}" rendering. Duplicated in spirit in `release_check.rs` (see Leads).
- `row` (private fn, small/generic) – `(out: &mut String, label: &str, value: impl AsRef<str>)` – line 1966 – padded `label: value` line. Near-identical to a local closure in `release_check.rs::render_text` (see Leads).
- `row_opt` (private fn) – line 1971.
- `row_bool` (private fn) – line 1979.
- `render_text` (private fn) – `(out: &mut String, status: &Status)` – line 1990 – the whole text report.

### `crates/ss-magic-plugin/src/spill_index.rs` (482 lines)
Purpose: `ss-magic-plugin spill-index` – lists the harness's own oversized tool-output ("spill") files for this worktree; strictly read-only.

- `SpillFile` (pub struct) – line 53 – one spilled file's path/name/bytes/modified/note.
- `SpillSession` (pub struct) – line 72 – one session's `tool-results/` directory.
- `Index` (pub struct) – line 87 – the whole answer.
- `TOOL_RESULTS_DIR` (private const, &str) – line 47.
- `projects_root` (pub fn) – `() -> Option<PathBuf>` – line 116 – `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`. Its own doc comment calls out that this is "deliberately the same resolution `plugin::ledger` uses for transcripts" – but the logic is copy-pasted, not shared (see Leads, L1).
- `encode_root` (private fn) – `(root: &Path, keep_underscore: bool) -> String` – line 137 – the harness's directory-name encoding for a cwd.
- `project_dir_for` (private fn) – `(projects_root: &Path, root: &Path) -> Option<PathBuf>` – line 154 – tries both `encode_root` spellings.
- `collect` (pub fn) – `(projects_root: &Path, root: &Path) -> Index` – line 173 – builds the whole index; never fails.
- `newest` (private fn, small/generic) – `(session: &SpillSession) -> u64` – line 244 – max `modified_ts` for sort ordering.
- `collect_session` (private fn) – `(session_id: &str, dir: &Path) -> SpillSession` – line 254.
- `SPILL_INDEX_USAGE` (private const, &str) – line 323.
- `run` (pub fn) – `(args: &[String]) -> Result<ExitCode>` – line 336 – the verb entry point.
- `emit` (private fn) – `(index: &Index, json: bool) -> Result<()>` – line 386.
- `plural` (private fn, small/generic) – `(n: usize) -> &'static str` – line 459 – `""`/`"s"`. **Byte-identical** to `ledger.rs:1525` (see Leads, L2).
- `human_bytes` (private fn, small/generic) – `(bytes: u64) -> String` – line 469 – coarse KiB/MiB rendering, with local `KIB`/`MIB` consts at lines 470–471. No duplicate found elsewhere in the workspace.

## Constants and literals

| Item | file:line | value | denotes | elsewhere in workspace | owning definition |
|---|---|---|---|---|---|
| `SCHEMA_VERSION` | status.rs:74 | `2` | `--json` shape version for `status` | `release_check.rs:60` also declares a `SCHEMA_VERSION = 1` for its own report (different shape, same name, per consts.md `dup-name`); `compact_window.rs:115` has `RECOMMEND_SCHEMA_VERSION = 1` | each is a distinct report's own version; not a true duplicate value, just a repeated naming pattern across three report shapes in the same crate |
| `DECLARED_EVENTS` | status.rs:82-88 | 5 event name strings | events the shipped manifest registers | the same 5 strings (`session-start`, `pre-tool-use`, `pre-compact`, `subagent-stop`, `session-end`) are literal in `main.rs:138-156` (the dispatch table) and repeated in `tests.rs` files and `test-bootstrap.sh` per literals.md lines 36,59,64,75,76 | `main.rs`'s `route()`/dispatch table is the actual source of truth for what fires; `DECLARED_EVENTS` is a second, hand-kept copy of the same 5 names |
| `MANIFEST_NAME` | status.rs:94 | `"ss-magic"` | plugin manifest name to match harness registrations on | literals.md line 22 shows `"ss-magic"` used 23x across the workspace (release.rs, heartbeat.rs, apply.rs, build-plugin-zip.py, etc.) | each use is contextual (binary name, manifest name, archive stem); no single owning const across crates, but within this file it is the one place the harness-match name is defined |
| `PIN_FILE` | status.rs:106 | `"ss-magic-plugin.version"` | plugin version-pin filename | literals.md line 74: also literal in `tests.rs` and `build-plugin-zip.py` (4 files, 9 occurrences) | doc comment explicitly says this is a deliberate single Rust-side copy; the shell/py side is a separate, acknowledged spelling |
| `BINARY_REL` | status.rs:129 | `"bin/ss-magic-plugin"` | bootstrapped binary path under data dir | literals.md line 87: also in `tests.rs:516` and `build-plugin-zip.py:882,1073` | doc-noted intentional single Rust copy, shell/py side separate |
| `MARKER_INSTALLED` | status.rs:133 | `".ss-magic-installed"` | bootstrap install-completion marker | not found elsewhere in consts/literals indices for this partition | owned here; presumably mirrored in `bootstrap.sh` (outside this partition) |
| `MARKER_DISCLOSED` | status.rs:135 | `".ss-magic-disclosed"` | one-time disclosure marker | same as above | same |
| `MARKER_UNSUPPORTED` | status.rs:138 | `".ss-magic-unsupported"` | unsupported-platform marker | same as above | same |
| `DATA_ROOT_FILE` | status.rs:144 | `"data-root"` | bootstrap's data-root pointer filename | literals.md line 122: also `tmproot.sh:48` | doc-noted bridge file; shell side owns the write, Rust side reads |
| `HARNESS_TIMEOUT` | status.rs:149 | `Duration::from_secs(5)` | timeout for `claude plugin list --json` | same literal value as `VERSION_TIMEOUT` (status.rs:152) and `release.rs:57 HTTP_TIMEOUT` (unrelated HTTP client) | three independently-named 5s timeouts across the workspace, none sharing a definition |
| `VERSION_TIMEOUT` | status.rs:152 | `Duration::from_secs(5)` | timeout for `<binary> --version` | see above | same |
| `POLL_INTERVAL` | status.rs:154 | `Duration::from_millis(20)` | subprocess poll interval | not found elsewhere | owned here |
| `LABEL_WIDTH` | status.rs:1953 | `22` | text-report label column width | `release_check.rs:576` has a local `const WIDTH: usize = 22` with the same value, used the same way (see Leads, L3) | genuine duplicate value + duplicate purpose |
| `TOOL_RESULTS_DIR` | spill_index.rs:47 | `"tool-results"` | harness's spill subdirectory name | not found elsewhere in the indices | owned here; the harness's own convention, no other reader in this codebase |
| `KIB`/`MIB` | spill_index.rs:470-471 | `1024` / `1024*KIB` | byte-formatting units | other `1024`-based consts exist elsewhere (`ledger.rs:555 READ_BUF_BYTES`, `heartbeat.rs:91 PRUNE_TRIGGER_BYTES`, `hook/subagent_stop.rs:70,75`) but for unrelated buffer/budget sizes, not byte-formatting | no other human-readable byte formatter exists in the workspace; not a duplicate |
| `CLAUDE_CONFIG_DIR` env name | spill_index.rs:117 | `"CLAUDE_CONFIG_DIR"` | harness config dir override | literals.md line 33: also `compact_window.rs:364`, `ledger.rs:1163`, `status.rs:805` – 4 occurrences | see Leads L1: `spill_index::projects_root` and `ledger::transcript_root` are the same function body twice |
| `".claude"` path segment | spill_index.rs:123 | `".claude"` | default harness config dir under `$HOME` | literals.md line 26: also `compact_window.rs:366`, `status.rs:807` (4 total) | same three call sites resolve `$HOME/.claude` independently |
| `"projects"` path segment | spill_index.rs:119,123 | `"projects"` | harness's per-project session directory | literals.md line 85: also `ledger.rs:1165,1169` | direct duplicate with `ledger::transcript_root`, see L1 |
| `CLAUDE_PLUGIN_ROOT` env name | status.rs:854 | `"CLAUDE_PLUGIN_ROOT"` | installed plugin tree | literals.md line 51: also `session_start.rs:136`, `release_check.rs:419` | each resolves it independently (no shared "locate plugin root" helper reused by `release_check.rs`, which instead calls `status::locate_plugin_root` for the fallback but reads the env var itself first – see cross-module refs) |
| `CLAUDE_PLUGIN_DATA` env name | status.rs:794 | `"CLAUDE_PLUGIN_DATA"` | installed data dir | literals.md line 264: also `test-bootstrap.sh:657` | fine, one Rust reader |
| `"error: unexpected argument `{other}`"` | status.rs:1891 | literal format string | arg-parse error | literals.md line 124: identical string also in `spill_index.rs:348` | both verbs hand-roll the same tiny arg parser loop (`-h`/`--help`, `--json`, else error) – see Leads L4 |
| number `117`/`120` | status.rs:674-675 | `120` char cutoff, `117` char take | `snippet()`'s truncation | numbers.md lists only these two occurrences, both in `status.rs` | not duplicated elsewhere |

## Cross-module references

**Imports into `status.rs`:** `crate::git`; `crate::heartbeat::{self, Outcome}`; `crate::{bypass, cache, checklist, compact_window, config, expect_artifact, identity, release_check, scratchpad, tmproot}`; `ss_magic_core::release`; `ss_magic_core::style`.

**Imports into `spill_index.rs`:** `crate::git`; `crate::scratchpad::format_rfc3339`; `ss_magic_core::style`.

**Public symbols defined here, used elsewhere:**

- `status::read_pin_file` – used by `release_check.rs:434` and `hook/session_start.rs:350`. Shared by design (doc comment at status.rs:108-112 names all three callers).
- `status::locate_plugin_root` – used by `release_check.rs:422,425`.
- `status::probe_harness` – used by `release_check.rs:425` (to build the `HarnessListing` it then hands to `locate_plugin_root`).
- `status::non_empty_env` – used by `compact_window.rs:363` (`Sources::from_process`).
- `status::PIN_FILE` – referenced only in doc comments in `hook/session_start.rs:318,344`, not as a value (the actual filename is re-derived via `read_pin_file`, not by joining `PIN_FILE` directly outside status.rs).
- `status::run` – called once, from `main.rs:533` (`HumanVerb::Status => status::run(args)`).
- `status::locate_data_dir`, `status::probe_binary_version`, `status::parse_listing`, `status::parse_version_line`, `status::collect`, `status::SCHEMA_VERSION`, `status::DECLARED_EVENTS`, `status::MANIFEST_NAME` – **unused outside this file** (called only from within `status.rs`'s own `run`/`collect`, apart from tests).
- `spill_index::run` – called once, from `main.rs:534` (`HumanVerb::SpillIndex => spill_index::run(args)`).
- `spill_index::projects_root`, `spill_index::collect`, `spill_index::SpillFile`, `spill_index::SpillSession`, `spill_index::Index` – **unused outside this file** (apart from tests). `projects_root` is `pub` despite this – its own doc comment argues it should logically be shared with `ledger::transcript_root`, but nothing actually imports it (see Leads L1).

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `spill_index.rs:116-124` vs `ledger.rs:1162-1169` | `projects_root()` and `transcript_root()` are the same function body – `$CLAUDE_CONFIG_DIR/projects` else `$HOME/.claude/projects` – under two different private names in two different files, and the doc comment on `projects_root` even says "deliberately the same resolution `plugin::ledger` uses" without the code actually being shared. `spill_index::projects_root` is `pub` but nothing outside this file calls it, so making one of the two call (or re-export) the other is free. | `crates/ss-magic-plugin/src/ledger.rs:1162-1169` (`fn transcript_root`) |
| L2 | duplication | `spill_index.rs:459-465` | `fn plural(n: usize) -> &'static str` is byte-identical to `ledger.rs:1525-1531`'s `plural`, including doc comment wording ("`\`""` or `"s"`""). Two private copies of the exact same one-liner in the same crate. | `crates/ss-magic-plugin/src/ledger.rs:1525-1531` |
| L3 | duplication / copy-paste-variant | `status.rs:1953,1966-1969` vs `release_check.rs:575-577` | `status.rs` has `const LABEL_WIDTH: usize = 22;` plus `fn row(out, label, value) { writeln!(out, "  {label:<LABEL_WIDTH$}{}", value.as_ref()) }`; `release_check.rs::render_text` has a local `const WIDTH: usize = 22;` and a local closure `let row = |out, label, value| writeln!(out, "  {label:<WIDTH$}{value}")` – same width, same format string, same purpose, just re-declared as a closure instead of reused as a function. `compact_window.rs:783` uses yet a third width (`{:<30}`) for the same kind of row. | `crates/ss-magic-plugin/src/release_check.rs:575-577` (and, as a softer three-way, `crates/ss-magic-plugin/src/compact_window.rs:781-783`) |
| L4 | duplication | `status.rs:1877-1896` vs `spill_index.rs:336-354` | Both verbs' `run()` hand-roll an identical tiny arg-parse loop: match `-h`/`--help` → print usage + return `SUCCESS`; `--json` → set flag; other → `eprintln!(style::err("error: unexpected argument `{other}`"))` + print usage + `ExitCode::from(2)`. `status.rs` additionally handles `--all` in the same loop, but the skeleton (help branch, error branch, exact error string) is copy-pasted. Other human verbs in the crate likely repeat this a third+ time (worth checking against other partitions' verb files, e.g. `release_check.rs`, `setup_ci.rs`). | `crates/ss-magic-plugin/src/spill_index.rs:336-354`; check also `crates/ss-magic-plugin/src/release_check.rs` and `crates/ss-magic-plugin/src/setup_ci.rs` argument loops in other partitions |
| L5 | copy-paste-variant | `status.rs:171-204` (`Field`) vs `status.rs:209-239` (`Located`) | `Field::found`/`Field::missing` and `Located::found`/`Located::missing` are structurally the same two-constructor "value-or-note" pattern, duplicated because `Located` needs a `PathBuf` instead of a `String`. `Located::as_field` already converts one into the other. A single generic `Provenance<T>` (or a trait) could unify the two constructor pairs, though the split may be intentional for clarity – flag for reviewer judgment rather than a strong claim. | within `status.rs` only; no other file in this partition uses the same pattern (`release_check.rs`'s report fields use inline `Option`/tuples, not this type) |
| L6 | const-location / naming | `status.rs:149,152` (`HARNESS_TIMEOUT`, `VERSION_TIMEOUT`) vs `release.rs:57` (`HTTP_TIMEOUT`, in core) | Three unrelated 5-second subprocess/HTTP timeouts, independently named and independently set to the same literal value across two crates. Not a behavioral bug (each is semantically distinct), but if a future change wants one timeout tunable it is easy to update one and miss the others silently drifting apart. Low-severity naming/consistency note only. | `crates/ss-magic-core/src/release.rs:57` |
| L7 | dead-code / unused-outside-file (informational) | `spill_index.rs:116` (`pub fn projects_root`) | `projects_root` is declared `pub` but has zero callers outside `spill_index.rs` (only `run()` in the same file, plus tests). Given L1, either it should stay `pub` because it is meant to be the shared implementation `ledger.rs` should call, or it should be `pub(crate)`/private since nothing external currently uses it. As-is it is publicly exposed but doing nothing for anyone else. | `crates/ss-magic-plugin/src/ledger.rs` (potential caller after L1's fix) |
| L8 | reuse | `status.rs:878-880` (`non_empty_env`) | Already shared with `compact_window::Sources::from_process` (`compact_window.rs:363`), which is the right pattern – flagged here only so the reviewer knows this one IS already deduplicated and should not be "fixed" again. Its `CLAUDE_CONFIG_DIR`+`.claude` resolution (status.rs:805-807, used for `locate_data_dir`'s fallback) is itself a fourth near-copy of the same `$CLAUDE_CONFIG_DIR` else `$HOME/.claude` pattern seen in L1's pair plus `compact_window.rs:364-366` – four sites doing the same two-line fallback with slightly different trailing joins (`projects`, `settings.json`, `plugins/data/...`). A single `fn claude_config_dir() -> Option<PathBuf>` in `status.rs` (already partially reused via `non_empty_env`) could anchor all four, but the trailing join differs per caller so this is a soft lead, not a strict duplicate. | `crates/ss-magic-plugin/src/compact_window.rs:363-368`; `crates/ss-magic-plugin/src/ledger.rs:1162-1169`; `crates/ss-magic-plugin/src/spill_index.rs:116-124` |
| L9 | naming / copy-paste-variant | `status.rs:375-382` (`struct Gate`) vs `config.rs` `GateConfig` | The report's `Gate` struct (`threshold_lines`, `inline_byte_budget`, `exemptions`, plus `source_root`/`note`) mirrors `config::GateConfig`'s three core fields field-for-field (per `cfg.gate.threshold_lines` etc. read at status.rs:929-931). This is an ordinary "DTO wraps domain type with provenance" pattern, not obviously wrong, but worth the reviewer's eye in case the two drift (a fourth field added to `GateConfig` would need a matching addition here, and nothing enforces that structurally). | `crates/ss-magic-plugin/src/config.rs` (`GateConfig`, referenced at status.rs:927-941) – file not in this partition, cross-check there |
| L10 | efficiency (low severity) | `status.rs:1177-1220` (`collect_enablement`'s harness-registration handling) | Builds `which: Vec<String>` via `.map(...).collect()` then immediately `.join(", ")`s it (lines 1183-1199) purely to produce one string; a `problems.push` built with a direct `fold`/`Iterator::intersperse`-style write would skip the intermediate `Vec`. Trivial, listed only because the sweep asked for efficiency candidates – not worth a change on its own merit given the small N (registrations list is at most a handful of entries). | none – self-contained, informational only |
| L11 | naming | `status.rs:1638` (`collect_newest_release`) vs `release_check.rs`'s own report-building function for the same cache | Both `status::collect_newest_release` and whatever builds `release_check::Report`'s "newest known release" field (`release_check.rs` around lines 560-610, per the `render_text` excerpt in L3) read the exact same `release::read_cache` / `cached_plugin_tag` and answer the same three questions (tag, checked-when, is-it-newer) independently, formatting the "unknown – {reason}" case in a slightly different sentence each time. Not a byte-identical duplicate (fields differ: `status` also computes `update_available: Option<bool>`), but the same read-cache-and-explain logic is grown twice. Cross-check against `release_check.rs`'s own recon partition for the exact function name/lines before proposing a merge. | `crates/ss-magic-plugin/src/release_check.rs` (its own report-building code around line ~440-560, outside this partition – verify exact lines there) |
