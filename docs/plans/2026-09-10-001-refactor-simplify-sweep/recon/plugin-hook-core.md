# Recon: plugin-hook-core

Partition: `crates/ss-magic-plugin/src/hook/{mod,event,session_start,session_end,pre_compact}.rs`

## Map

### `hook/mod.rs` – the one pipeline: decode stdin, gate, dispatch, encode stdout, heartbeat, always exit 0.

- `route (pub)` – `(event: &HookEvent) -> Option<Route>` – the routing table; `None` for `Unknown`/`Missing`.
- `run (pub)` – `(event: &HookEvent) -> Result<ExitCode>` – real-I/O entry point, always returns `SUCCESS`.
- `HookContext<'a> (pub struct)` – fields `event`, `envelope`, `repo_root`, `config_root`, `main_root` (unused, `#[allow(dead_code)]`), `config`, `now`, `diagnostics` (private `RefCell`) – everything a handler may read.
  - `cwd (pub)` – `(&self) -> &Path` – envelope's cwd as a `Path`.
  - `diagnostic (pub)` – `(&self, message: impl Into<String>)` – queue a stderr line.
- `Outcome (pub struct)` – `{ response: Response, detail: Option<String> }`.
  - `silent (pub)` – `() -> Self`.
  - `new (pub)` – `(response: Response) -> Self`.
  - `with_detail (pub)` – `(mut self, detail: impl Into<String>) -> Self` – same name/shape as `heartbeat::Row::with_detail` (crates/ss-magic-plugin/src/heartbeat.rs:176); unrelated types, likely coincidental convention not a duplicate.
- `Handler (pub type)` – `fn(&HookContext<'_>) -> Result<Outcome>`.
- `Route (pub struct, Copy)` – `{ module, handler, writes_state }`.
- `quiet_mode (pub(crate))` – `(envelope: &Envelope, entrypoint: Option<&OsStr>) -> Option<&'static str>` – the R31 "is anyone watching" verdict; small, generic, single real caller (`session_start.rs:176`) – candidate for being folded into `Surroundings`/`session_start` if it never grows a second caller, but currently justified as pipeline-owned since the doc says notices "consult" it as a shared verdict.
- `ENTRYPOINT_ENV (pub(crate) const)` – see Constants.
- private: `HookIo<'a>` (I/O+store bundle for tests), `run_with`, `run_with_route`, `pipeline`, `base_row`, `gate_and_dispatch`, `with_discovery_note`, `state_tree_refusal`, `flush_diagnostics`, `panic_message`.
  - `state_tree_refusal` – `(root: &Path) -> Option<String>` – the ignored-tree gate; small, generic shape (probe + two-branch fail-closed message) – compare against `scratchpad.rs`'s own ignored-tree probe (same git call, same `STATE_QUERY` idea, different message wording) and `status.rs`'s third variant (dynamic query, different message again). Three near-identical "ask git if the state tree is ignored, render a sentence" bodies.
  - `panic_message` – `(payload: &(dyn Any + Send)) -> String` – small, generic string-recovery helper; no duplicate found elsewhere in the workspace.
  - `base_row` – `(event: &HookEvent, cwd: &str, now: u64) -> Row` – tiny row constructor, local use only.

### `hook/event.rs` – the hook wire format: decode stdin into `Envelope`, encode `Response` to stdout. Pure data, no I/O.

- `Common (pub struct, Deserialize)` – shared envelope fields (`session_id`, `transcript_path`, `cwd`, `hook_event_name`, `prompt_id`, `permission_mode`).
- `SessionStart / PreToolUse / PreCompact / SubagentStop / SessionEnd / FileChanged (pub structs, Deserialize)` – per-event payload shapes.
- `Payload (pub enum)` – wraps one of the above.
- `Envelope (pub struct)` – `{ common, payload, raw }`.
- `DecodeError (pub enum)` – `NoInput | Malformed(String) | NotAnEnvelope(String) | Unroutable(String)`.
  - `class (pub)` – `(&self) -> &'static str` – stable heartbeat-row class string.
  - `Display` impl – human message.
- `decode (pub)` – `(event: &HookEvent, input: &str) -> Result<Envelope, DecodeError>`.
- `from_raw (private)` – `<T: DeserializeOwned>(raw: &Value) -> Result<T, DecodeError>` – small, generic; the one deserialize-with-error-mapping helper, no duplicate found.
- `cwd_hint (pub)` – `(input: &str) -> Option<String>` – best-effort `cwd` extraction from raw JSON, used only on the pre-decode error paths.
- `PermissionDecision (pub enum)` – `Deny` only.
  - `as_str (private)` – `(self) -> &'static str`.
- `PreToolUseResponse (pub struct, Default)` – `{ decision, reason, additional_context, system_message }`.
  - `is_empty (private)` – `(&self) -> bool`.
- `Response (pub enum)` – `Silent | SessionStart{..} | PreToolUse(..) | SubagentStopBlock{..}`.
- `SessionStartOut/-Specific, PreToolUseOut/-Specific, SubagentStopOut (private structs, Serialize)` – wire shapes, one per response variant.
- `encode (pub)` – `(response: &Response) -> Result<Option<String>, serde_json::Error>`.

### `hook/session_start.rs` – `SessionStart` handler: scaffold the scratchpad, inject guidance, decide three operator notices.

- `Surroundings (pub(crate) struct)` – injected environment (plugin root, override flag, entrypoint, cache-dir closure, lock-root closure, spawn-refresh closure).
  - `from_process (private)` – `() -> Self` – same name as `compact_window.rs:362`'s `from_process` (also a real-environment constructor for an injected-surroundings struct) – same pattern repeated, not literally shared code; worth checking whether the two structs could share a "read these 2-3 env vars" helper.
- `handle (pub(crate))` – `(ctx: &HookContext<'_>) -> Result<Outcome>` – wired handler, calls `handle_with`.
- `handle_with (pub(crate))` – `(ctx, surroundings: &Surroundings) -> Result<Outcome>` – the testable core.
- `CompactionAdvice (private struct)` – `{ message, detail }`.
- `compaction_advice (private)` – `(repo_root, source, quiet, override_present, marker_dir, now) -> CompactionAdvice` – R27 once-per-machine nudge.
- `claim_marker (private)` – `(marker: &Path, now: u64) -> io::Result<()>` – `create_new` exclusive marker write; same "exclusive marker via `create_new`, 0600" shape as `pre_compact.rs::append_note`'s new-file branch and as `claim.rs::take`'s rename-claim (different primitive, same intent) – compare, don't merge (claim.rs is deliberately rename-based per the pinned hard rule).
- `join_system_messages (private)` – `<const N: usize>(parts: [Option<String>; N]) -> Option<String>` – small, generic; no duplicate found, but note it is the only join point for three notices that could as easily be a `Vec`.
- `version_drift_notice (private)` – `(plugin_root: Option<&Path>) -> Option<String>`.
- `read_pin (private)` – `(plugin_root: Option<&Path>) -> Option<String>` – thin wrapper over `crate::status::read_pin_file`.
- `ReleaseAdvice (private struct)` – `{ message, details }`.
- `release_suggestion (private)` – `(surroundings, source, quiet, now) -> ReleaseAdvice` – R29-R31 notice + R30 background refresh.
- `MAX_LISTED_TRACKED_PATHS (private const)` – see Constants.
- `render_refusal (private)` – `(refusal: &Refusal) -> String` – caps `Refusal::TrackedPaths` display; every other variant passes through `Display`.
- `display_rel (private)` – `(root: &Path, path: &Path) -> String` – **byte-identical body** to `checklist/verbs.rs:1374`'s `display_rel` (same name, same signature, same `strip_prefix`/fallback logic, only a cosmetic `.unwrap_or_else` vs `.unwrap_or` difference). Strong dedup candidate.
- `build_guidance (private)` – `(repo_root: &Path, report: &Report) -> String` – assembles the whole `additionalContext` body; the one place `STATE_FILE_NOTES` and `CHECKLIST_VERBS`/`CHECKLIST_POINTER_NAME` are rendered.
- Constants: `CHECKLIST_VERBS`, `CHECKLIST_POINTER_NAME`, `STATE_FILE_NOTES`, `COMPACT_ADVICE_MARKER`, `COMPACT_ADVICE_TEXT` – see Constants.

### `hook/session_end.rs` – `SessionEnd` handler: append one cost-ledger row from the ending session's transcript.

- `handle (pub(crate))` – `(ctx: &HookContext<'_>) -> Result<Outcome>` – resolves the heartbeat store, then calls `handle_in_store`.
- `handle_in_store (pub(crate))` – `(ctx: &HookContext<'_>, store: &Path) -> Result<Outcome>` – the testable core; builds `ledger::Ingest` and matches `ledger::Recorded`.
- No consts in this file.

### `hook/pre_compact.rs` – `PreCompact` handler: append one entry to a tool-owned `PRE-COMPACT.md`, always silent.

- `handle (pub(crate))` – `(ctx: &HookContext<'_>) -> Result<Outcome>`.
- `rel_to_repo (private)` – `(repo_root: &Path, path: &Path) -> String` – **same body shape** as `session_start.rs::display_rel` and `checklist/verbs.rs::display_rel` (strip_prefix + lossy fallback), different name (`rel_to_repo` vs `display_rel`). Third copy of the same three-line idiom.
- `is_tracked (private)` – `(report: &Report, rel: &str) -> bool` – checks whether `rel` appears in any `Refusal::TrackedPaths` the scratchpad already reported. Same *name* as `scratchpad.rs:363`'s `is_tracked(&self, path: &Path) -> bool` (checks a `HashSet` of git-tracked paths) but different semantics/signature – naming collision, not logic duplication; low-priority rename candidate (`is_tracked` here answers "already reported as tracked", not "is tracked").
- `append_note (private)` – `(session_dir: &Path, now: u64, trigger: &str, custom_instructions: Option<&str>) -> Result<()>` – `create_new`-then-fallback-append idiom, same shape as `session_start.rs::claim_marker`'s exclusive-create pattern (both use `create_new` + `mode(0o600)`, but this one *also* falls back to a plain append on `AlreadyExists` rather than treating it as "someone else already did it").
- Constants: `NOTE_NAME`, `HEADER`, `FILE_MODE` – see Constants.

## Constants and literals

| Name/value | Location | Denotes | Elsewhere in workspace? |
|---|---|---|---|
| `STATE_QUERY = ".superset/.magic/"` | `hook/mod.rs:93` | git query string for the ignored-tree gate | **Yes** – `scratchpad.rs:66` defines an identically-named, identically-valued private const with near-identical doc comment; `status.rs:1270` independently builds the same value via `format!("{}/", scratchpad::STATE_REL)`. Owning definition should arguably be `state_tree::STATE_REL` (core, `.superset/.magic`, no trailing slash) with the `/` appended at the one or two call sites, the way `status.rs` already does it – `mod.rs` and `scratchpad.rs` instead each hardcode the slash-suffixed literal independently. |
| `REASON_UNROUTABLE`/`REASON_STDIN`/`REASON_STDOUT`/`REASON_CWD_MISSING`/`REASON_DISABLED`/`REASON_NOT_IGNORED`/`REASON_HANDLER_ERROR`/`REASON_HANDLER_PANIC`/`REASON_ENCODE_FAILED` | `hook/mod.rs:103-119` | heartbeat "reason class" vocabulary | Not found duplicated elsewhere; `REASON_UNROUTABLE`'s value `"unroutable-event"` equals `DecodeError::Unroutable`'s `.class()` value in `event.rs:262` (`"unroutable-event"`) – same string, two independent owners (one a bare const, one a match arm); worth a single source of truth if the two are meant to name the same condition. |
| `ENTRYPOINT_ENV = "CLAUDE_CODE_ENTRYPOINT"` | `hook/mod.rs:128` | env var name | Only reader is `session_start.rs:138`; no duplicate literal found elsewhere. |
| `MAX_LISTED_TRACKED_PATHS = 20` | `session_start.rs:467` | cap on inline-listed tracked paths before "… and N more" | Same *pattern* (cap a repository-controlled list before it blows a fixed budget) as `cache.rs::bound` (byte/line budget for cached bodies) referenced in this file's own comment; no numeric duplicate of `20` found in the index. |
| `CHECKLIST_VERBS` (multi-line `&str`) | `session_start.rs:49-57` | the eight `ss-magic-plugin checklist …` command lines shown in guidance | Not duplicated verbatim; the individual verb names (`init`, `add-item`, …) are the same tokens `checklist/verbs.rs`'s CLI parser matches on – a drift risk if a verb is renamed there without updating this string, since nothing ties them together. |
| `CHECKLIST_POINTER_NAME = "checklist.json"` | `session_start.rs:62` | pointer file name inside the state root | Literal `.superset/.magic/checklist.json` / `checklist-pointer.json` appear in `checklist/verbs.rs` test fixtures (per literals.md) – the actual pointer filename used by `checklist/verbs.rs` should be checked for exact match; this module's own comment says "not yet written by anything in this codebase" as of this doc's writing, so treat as aspirational and verify against `checklist/verbs.rs`'s real constant before assuming agreement. |
| `STATE_FILE_NOTES` (6 pairs) | `session_start.rs:68-90` | the six state file names + one-line descriptions shown in guidance | The six *names* (`CONTEXT.md`, `DECISIONS.md`, `LEARNINGS.md`, `OPERATOR-CHECKLIST.md`, `STATUS.md`, `TASKS.md`) are typed out again here rather than derived from `scratchpad::STATE_FILES` (`scratchpad.rs:80`, same six strings, same order) – the doc comment explicitly warns this is a duplication risk ("a name here that U8 does not actually create would be guidance worse than none"), but the fix (zip `STATE_FILES` with a separate description array) was not taken. |
| `COMPACT_ADVICE_MARKER = "compact-advice-shown"` | `session_start.rs:94` | once-per-machine marker file name | Not duplicated elsewhere in the index. |
| `COMPACT_ADVICE_TEXT` | `session_start.rs:217-223` | the operator-facing compaction notice | Not duplicated; names `compact-window --recommend` and `--set`, which are real verb names in `compact_window.rs`. |
| `10,000` (character budget, in comments) | `event.rs:357,397`; `session_start.rs:464` (as "R19's 10,000-character budget") | `additionalContext` cliff documented by the harness | **Yes** – `config.rs:92` defines `GATE_INLINE_BYTE_BUDGET_DEFAULT: u32 = 10_000` for a *different* budget (the Read-gate's inline byte threshold), and `pre_tool_use.rs:1554` also references "10,000-character cliff" in a comment. None of these four sites derive from one shared const; they are independently-typed magic numbers describing what may be the same harness limit (`additionalContext` cliff) referenced from unrelated code paths – a comment-only duplication, not a logic bug, but a single named const (even doc-only) would prevent drift if the harness's real cliff ever changes. |
| `NOTE_NAME = "PRE-COMPACT.md"` | `pre_compact.rs:52` | compaction-log file name | Deliberately *not* in `scratchpad::STATE_FILES` (by design, per module doc) – not a duplicate, a deliberate exclusion. |
| `HEADER` (multi-line) | `pre_compact.rs:55-62` | one-time file header | Not duplicated. |
| `FILE_MODE = 0o600` | `pre_compact.rs:67` | owner-only file mode | **Yes, extremely** – identical name+value in `bypass.rs:68`, `cache.rs:107`, `expect_artifact.rs:93`, `heartbeat.rs:73`, `hook/file_changed.rs:95`, `hook/subagent_stop.rs:86`, `ledger.rs:101`, `scratchpad.rs:122` (nine copies of `const FILE_MODE: u32 = 0o600` across the plugin crate; `setup_ci.rs:87`'s own `FILE_MODE` is `0o644`, a different value under the same name). `pre_compact.rs`'s own doc comment explicitly defends the duplication ("Repeated here rather than imported... which keeps its own copy private") – flag for the reviewer to weigh against consolidating into one crate-level `const OWNER_ONLY_MODE: u32 = 0o600` used by all nine, since `atomic::write_atomically` (per CLAUDE.md) already centralizes the *write* primitive that takes a mode parameter – only the mode *value* is what is scattered. |

## Cross-module references

**Imports into this partition:**
- `hook/mod.rs` – `crate::git` (incl. `git::discover::roots`, `git::is_ignored_no_index_str`), `crate::config::{self, PluginConfig}`, `crate::heartbeat::{self, Outcome as RowOutcome, Row}`, `crate::scratchpad::now_secs`, `crate::HookEvent`; declares child modules `event` (pub), `file_changed`, `pre_compact`, `pre_tool_use`, `session_end`, `session_start`, `subagent_stop`.
- `hook/event.rs` – `serde`, `serde_json::Value`, `crate::HookEvent`. No crate-internal imports beyond that – pure data module as documented.
- `hook/session_start.rs` – `ss_magic_core::release::{self, PLUGIN_LINE}`, `crate::compact_window::{self, OVERRIDE_ENV}`, `crate::hook::event::{Payload, Response}`, `crate::hook::{self, HookContext, Outcome}`, `crate::release_check::{self, Decision, Recorded}`, `crate::scratchpad::{self, Refusal, Report}`, `crate::tmproot`.
- `hook/session_end.rs` – `crate::heartbeat`, `crate::hook::event::Payload`, `crate::hook::{HookContext, Outcome}`, `crate::ledger::{self, Ingest, Recorded}`.
- `hook/pre_compact.rs` – `crate::hook::event::Payload`, `crate::hook::{HookContext, Outcome}`, `crate::scratchpad::{self, Refusal, Report}`.

**Where this partition's public symbols are used elsewhere (grep-verified, max 5 callers each):**
- `hook::route` – used by every sibling handler module's doc comments/unreachable-arm comments (`pre_tool_use.rs:641,700`; `subagent_stop.rs:88,91`; `file_changed.rs:122,137`) and by every handler's own `tests.rs` (`subagent_stop/tests.rs:165`, `pre_compact/tests.rs:98`, `session_end/tests.rs:120`, `session_start/tests.rs:160`, `pre_tool_use/tests.rs:227`, `file_changed/tests.rs:213`). No caller outside `hook/`.
- `hook::run` – one caller: `main.rs:517` (`hook::run(event)`).
- `hook::quiet_mode` – one production caller: `session_start.rs:176`; documented (not called) by `release_check.rs:23`'s module comment; exercised directly by `hook/tests.rs` (lines 1116-1166).
- `hook::ENTRYPOINT_ENV` – one caller: `session_start.rs:138`.
- `event::decode` – one production caller: `hook/mod.rs:439`; one test call (`hook/tests.rs:1101`).
- `event::encode` – one production caller: `hook/mod.rs:569`; test callers in `pre_compact/tests.rs:111`, `session_end/tests.rs:109`, `pre_tool_use/tests.rs:329,2784`.
- `event::cwd_hint` – two production callers, both in `hook/mod.rs` (lines 431, 451); tested in `hook/event/tests.rs`.
- `session_start::Surroundings` – used only inside `session_start.rs` (`handle`, `handle_with`) and its own `tests.rs`; unused outside this file.
- `session_end::handle_in_store` – called only by `session_end::handle` and `session_end/tests.rs`; unused outside this file.
- `pre_compact::rel_to_repo`, `pre_compact::is_tracked`, `pre_compact::append_note` – private, used only within `pre_compact.rs` and exercised by `pre_compact/tests.rs`; unused outside this file.
- `HookContext::main_root` field – read nowhere except the pipeline that builds it (`hook/mod.rs:512,536`); every other hook module reads `ctx.config_root` instead (`pre_tool_use.rs:382,384,768,1458`). Confirms the `#[allow(dead_code)]`/doc claim that no handler reads it today.
- `status::read_pin_file` (imported by this partition, not defined here) – also called directly by `status.rs:1332` and `release_check.rs:434`; `session_start.rs::read_pin` is a thin wrapper adding `Option`-chaining, not a duplicate implementation.

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `hook/session_start.rs:495-499` vs `checklist/verbs.rs:1374-1379` | `display_rel` is defined twice with the same signature and effectively the same body (`strip_prefix` + lossy-string fallback); one private copy per file. | `crates/ss-magic-plugin/src/checklist/verbs.rs:1374` |
| L2 | duplication | `hook/pre_compact.rs:132-136` | `rel_to_repo` is a third copy of the same `strip_prefix`-plus-fallback idiom as L1, just renamed. | `hook/session_start.rs:495`; `checklist/verbs.rs:1374` |
| L3 | const-location | `hook/mod.rs:93` (`STATE_QUERY`) vs `scratchpad.rs:66` (`STATE_QUERY`) | Two private consts, same name, same value `".superset/.magic/"`, near-identical doc comment, each independently hardcoding the trailing slash instead of deriving from `state_tree::STATE_REL`. | `crates/ss-magic-core/src/state_tree.rs:35` (`STATE_REL`); `crates/ss-magic-plugin/src/status.rs:1270` (the one site that already derives it: `format!("{}/", scratchpad::STATE_REL)`) |
| L4 | duplication | `hook/mod.rs:619-631` (`state_tree_refusal`) | The "ask git if `.superset/.magic/` is ignored, render pass/fail-closed message" logic is written a third time here, with its own wording, after `scratchpad.rs` (own probe, own wording) and `status.rs:1271-1276` (own probe, own wording). All three call the same core function (`git::is_ignored_no_index_str`) on the same path. | `crates/ss-magic-plugin/src/scratchpad.rs:439`; `crates/ss-magic-plugin/src/status.rs:1271` |
| L5 | const-location | `hook/pre_compact.rs:67` (`FILE_MODE = 0o600`) | One of nine identically-named, identically-valued private `FILE_MODE` consts scattered across the plugin crate; the doc comment defends the duplication but a single shared `OWNER_ONLY_MODE` const would remove the risk of one copy drifting (e.g. `setup_ci.rs`'s same-named const is `0o644`, showing the name alone gives no protection). | `bypass.rs:68`; `cache.rs:107`; `expect_artifact.rs:93`; `heartbeat.rs:73`; `hook/file_changed.rs:95`; `hook/subagent_stop.rs:86`; `ledger.rs:101`; `scratchpad.rs:122`; `setup_ci.rs:87` (different value, same name – naming trap) |
| L6 | const-location | `hook/session_start.rs:68-90` (`STATE_FILE_NOTES`) | The six file-name strings are retyped by hand rather than built from `scratchpad::STATE_FILES`, which lists the same six names in the same order; the module's own comment flags this as a real drift risk it did not close. | `crates/ss-magic-plugin/src/scratchpad.rs:80` (`STATE_FILES`) |
| L7 | hardcoded-value | `hook/event.rs:357,397`; `hook/session_start.rs:464`; `hook/pre_tool_use.rs:1554` | The harness's `additionalContext` 10,000-character cliff is written as a bare number in four separate comments across two-plus files, never as a named const, while a differently-purposed `10_000` (`config.rs:92`, `GATE_INLINE_BYTE_BUDGET_DEFAULT`) exists for the *other* 10,000-character/byte budget in the same hook contract – easy to confuse the two limits or let one drift undocumented. | `crates/ss-magic-plugin/src/config.rs:92` |
| L8 | naming | `hook/pre_compact.rs:140` (`is_tracked(report: &Report, rel: &str) -> bool`) vs `scratchpad.rs:363` (`is_tracked(&self, path: &Path) -> bool`) | Same function name, different receiver/semantics (one re-checks a previously-reported refusal list, the other queries a live tracked-paths set) – a reader grepping `is_tracked` will find two unrelated answers. | `crates/ss-magic-plugin/src/scratchpad.rs:363` |
| L9 | copy-paste-variant | `hook/session_start.rs:297-304` (`claim_marker`, exclusive `create_new`, no fallback) vs `hook/pre_compact.rs:167-182` (`append_note`'s open, `create_new` **with** an `AlreadyExists` fallback to plain append) | Both are "exclusive-create a 0600 file, treat `AlreadyExists` specially" idioms in the same partition, but one treats the race as "someone else already did the whole job, stop" and the other treats it as "someone else made the file exist, so skip only the header and keep appending" – worth confirming both are intentionally different rather than one being an incomplete copy of the other. | (same partition, listed for the reviewer's judgment) |
| L10 | efficiency | `hook/mod.rs:465-466`, `502` (`gate_and_dispatch`) | `route.writes_state` gate and the `discovered`/`fallback` plumbing already separate "cheap path" from "git-probing path" per the module doc; no measured issue found, but the `base` closure is rebuilt three times (`pipeline`'s own `base`, then `gate_and_dispatch`'s own `base`) with the same body (`base_row(event, &envelope.common.cwd, now)`) re-declared as a local closure in two functions rather than one shared helper the caller passes down. | `hook/mod.rs:458` and `hook/mod.rs:504` (the two `let base = || base_row(...)` sites) |
| L11 | reuse | `hook/session_start.rs:134` (`Surroundings::from_process`) vs `compact_window.rs:362` (`Sources::from_process`, per `functions.md`) | Same method name on a different "read a few env vars / build closures over the real environment for testability" struct – same pattern intentionally repeated per-module (each struct is a documented testability seam per `structure-pins.md`), so likely NOT to be merged, but worth the reviewer's confirmation that no shared "read `CLAUDE_PLUGIN_ROOT`" helper would remove one line of duplication (`std::env::var_os("CLAUDE_PLUGIN_ROOT").map(PathBuf::from)` – check whether `compact_window.rs` repeats this exact line). | `crates/ss-magic-plugin/src/compact_window.rs:362` |
| L12 | dead-code | `hook/mod.rs:212` (`HookContext::main_root`, `#[allow(dead_code)]`) | Field is populated by the pipeline and never read by any handler (verified: every handler reads `ctx.config_root` or `ctx.repo_root` instead). The doc comment argues it should stay for a *future* handler, so this is a documented-intentional dead field, not an oversight – flagged only so the reviewer can decide if the justification still holds. | `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:382,384,768,1458` (the actual `config_root` callers) |
| L13 | naming | `hook/mod.rs:103` (`REASON_UNROUTABLE = "unroutable-event"`) vs `hook/event.rs:262` (`DecodeError::Unroutable.class() == "unroutable-event"`) | Same string literal, meaning the same condition, owned independently by a bare const in one file and a `match` arm in another – a rename of one would silently desync the heartbeat log's vocabulary from the decode-error class. | `crates/ss-magic-plugin/src/hook/event.rs:257-264` |
