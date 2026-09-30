# Recon: plugin-hook-gates (pre_tool_use, subagent_stop, file_changed)

## Map

### `crates/ss-magic-plugin/src/hook/pre_tool_use.rs` (1570 lines)
Purpose: `PreToolUse` handler – the R88 checklist deny, the R21-R24/R41-R43/R52 oversized-`Read` page-fault gate, and the R91 commit-time checklist nudge on `Bash`, in one decision function.

- `handle(ctx: &HookContext<'_>) -> Result<Outcome>` (pub(crate), :642) – entry point, calls `gate(ctx, classify_checklist)`.
- `Classification` (pub(crate) enum, :120) – `Ordinary` / `Checklist { reason }`, the checklist classifier's verdict.
- `Classifier` (private type alias, :140) – `fn(&HookContext, &Path) -> Classification`, injected for testability.
- `classify_checklist(ctx, target) -> Classification` (private, :155) – the shipped classifier; resolves target then checks every candidate root.
- `Target` (private enum, :254) – `Rooted(PathBuf)` / `Unrootable(PathBuf)`.
- `resolve_target(cwd: &Path, raw: &Path) -> Target` (private, :272) – the tilde-expand → lexical-normalize → process-view pipeline (R88 move 1+2). **Name collides** with `file_changed.rs:253 resolve_target(raw: &str) -> Result<PathBuf, String>` – unrelated logic, same name (see L15).
- `absolute_cwd(cwd) -> PathBuf` (private, small/generic, :328) – `canonicalize` or fall back to the input.
- `own_home() -> Option<PathBuf>` (private, small/generic, :342) – reads `$HOME`, requires absolute.
- `classification_roots(ctx, target) -> Vec<PathBuf>` (private, :364) – actor root + target's own walked root (R88 move 3).
- `actor_root(ctx) -> PathBuf` (private, :380) – memoized `worktree_root` or `ctx.config_root` canonicalized.
- `resolve_for_classification(target: &Path) -> PathBuf` (private, :405) – canonicalize-or-deepest-existing-ancestor-plus-tail. Same shape as `pre_tool_use.rs`'s own doc calls out as "looks redundant with a caller's resolved value but is not" – internal note at :949-966.
- `paths_equal_ignoring_case(a, b) -> bool` (private, small/generic, :444) – ASCII case-fold path compare, fallback to `==` on non-UTF-8. Candidate for reuse: shape is generic enough it could live in `pathnorm.rs` alongside `ends_with_ignoring_case`.
- `is_checklist_path(root, path) -> bool` (private, :454) – convention match or pointer match (case-folded, then canonicalized-and-refolded).
- `is_checklist_under_any_root(pointer_root, path) -> bool` (private, :503) – the root-independent variant, used only for `Target::Unrootable`.
- `has_checklist_shape(pointer_root, path) -> bool` (private, :527) – ancestor-wise `matches_convention` scan + pointer-suffix test.
- `ends_with_ignoring_case(path, tail) -> bool` (private, small/generic, :567) – component-wise case-folded suffix match. Generic utility, candidate for `pathnorm.rs`.
- `tool_label(ctx) -> &str` (private, :588) – tool name from the envelope, `"tool call"` fallback.
- `checklist_reason(tool, shown) -> String` (private, :612) – the R88 denial text; hardcodes every `ss-magic-plugin checklist …` command line (see Constants section).
- `GateTool` (private enum, :648) – `Read | Mutating | Search | Bash | Other`.
- `GateTool::from_name(name) -> Self` (:668) – tool-name dispatch table.
- `gate(ctx, classify) -> Result<Outcome>` (private, :698) – the 13-step decision order documented at :679-697; this is the whole module's spine.
- `allow(detail) -> Outcome` (private, small/generic, :859) – `Outcome::silent().with_detail(...)`. **Name collides** with the concept of "allow" helpers implicit elsewhere but not a literal duplicate name found.
- `deny(reason, detail) -> Outcome` (private, small/generic, :864) – builds a `PreToolUseResponse` deny.
- `target_path(payload) -> Option<PathBuf>` (private, :877) – reads `file_path` or `notebook_path` from `tool_input`.
- `subagent_label(payload) -> Option<&str>` (private, :896) – `agent_type` or `agent_id`, non-empty.
- `read_u64(payload, key) -> Option<u64>` (private, small/generic, :911) – tolerant JSON-number-or-string reader.
- `in_state_tree(path) -> bool` (private, :928) – adjacent-components test against `STATE_REL`'s two parts.
- `non_text_extension(path) -> Option<String>` (private, :937) – lowercased extension membership in `NON_TEXT_EXTENSIONS`.
- `configured_exemption(patterns, root, path) -> Option<&str>` (private, :967) – glob match against relative-then-absolute path; deliberately re-canonicalizes rather than reusing the caller's resolved target (documented at :949-966).
- `threshold_bytes(threshold_lines) -> u64` (private, small/generic, :988) – `lines * BYTES_PER_LINE`.
- `window_bytes(size, offset, limit) -> u64` (private, :1000) – R41 window-cost estimate, ceiling division twice.
- `worktree_root(cwd) -> Option<PathBuf>` (private, :1029) – process-lifetime memoized wrapper over `walk_for_root`, using a `static OnceLock<Mutex<HashMap<...>>>` (`MEMO`, :1030).
- `walk_for_root(cwd) -> Option<PathBuf>` (private, :1050) – bounded ancestor walk for `ROOT_MARKER`.
- `display_path(root, path) -> String` (private, small/generic, :1065) – `strip_prefix` or raw. **Name collides conceptually** with similar "display relative to root, fallback to raw" helpers in this same file's siblings (`rel_to_repo` in pre_compact.rs, `rel_to_state` in subagent_stop.rs – see L1/L2).
- `kb(size) -> u64` (private, small/generic, :1074) – `div_ceil(1024)`.
- `grouped(n) -> String` (private, small/generic, :1081) – thousands-separator formatting.
- `hit_preamble(shown, size, threshold_lines) -> String` (private, :1097) – cache-hit denial prefix.
- `hit_epilogue(shown) -> String` (private, :1115) – cache-hit denial suffix, embeds the `ss-magic-plugin bypass`/`conclude` command text.
- `miss_reason(shown, size, threshold_lines, entry) -> String` (private, :1138) – cache-miss denial, embeds the same command family again (near-duplicate prose vs `hit_epilogue`, see L3).
- `shipping_action(command) -> bool` (private, :1241) – top of the R91 matcher pipeline.
- `shipping_action_words(command) -> bool` (private, :1264) – single-pass quote/`$()`-aware tokenizer + trailing-window check.
- `flush_word(word, w0, w1, w2) -> bool` (private, :1323) – word-boundary shift-and-test.
- `matches_shipping_action(w0, w1, w2) -> bool` (private, :1347) – `git commit` / `git push` / `gh pr create` trailing-window test.
- `is_word_char(c) -> bool` (private, small/generic, :1362) – bash bare-word charset.
- `strip_heredocs(command) -> String` (private, :1379) – line-based heredoc-body stripper.
- `heredoc_delimiters(line) -> Vec<(String, bool)>` (private, :1404) – `<<[-]DELIM` scanner.
- `commit_nudge(ctx, payload) -> Outcome` (private, :1447) – R91 entry past the pre-filter; reads command, checks `shipping_action`, resolves checklist candidates, checks staleness.
- `checklist_candidates(root) -> Option<Vec<PathBuf>>` (private, :1488) – pointer target, or convention-matching files under `docs/actions/`.
- `staleness(root, candidates) -> Option<PathBuf>` (private, :1510) – first candidate missing or git-dirty-and-unstaged, via `git::status_porcelain`.
- `nudge_text(shown) -> String` (private, :1556) – the `additionalContext` advisory text.

### `crates/ss-magic-plugin/src/hook/subagent_stop.rs` (571 lines)
Purpose: `SubagentStop` handler – enforce a declared `expect-artifact` output file (R32/R51) and salvage a resultless agent's transcript (R33/R54).

- `handle(ctx: &HookContext<'_>) -> Result<Outcome>` (pub(crate), :89) – entry point: `stop_hook_active` short-circuit, repo-root check, `scratchpad::ensure`, salvage, then take-and-check the declaration.
- `block_reason(expectation, unmet, salvage) -> String` (private, :164) – the block text shown to the agent.
- `Salvaged` (private struct, :198) – `{ path: Option<PathBuf>, note: String }`.
- `salvage_path_of(salvage) -> Option<&Path>` (private, small/generic, :206) – accessor.
- `salvage_note(salvage) -> Option<&str>` (private, small/generic, :211) – accessor.
- `suffix(salvage) -> String` (private, small/generic, :216) – `"; {note}"` or empty, for detail-string concatenation.
- `salvage(ctx, payload, report) -> Option<Salvaged>` (private, :227) – the whole salvage decision: skip if a real message exists, else read+recover+render+write.
- `read_bounded(path) -> Result<String>` (private, :290) – lossy UTF-8 read capped at `MAX_TRANSCRIPT_BYTES`.
- `recovered_body(raw, transcript) -> (String, usize)` (private, :311) – pulls assistant blocks or falls back to raw lines; tail-keeps to budget.
- `assistant_blocks(raw) -> Vec<String>` (private, :345) – JSONL scan for `type:"assistant"` message content.
- `push_text(out, text)` (private, small/generic, :376) – append-if-non-blank.
- `keep_tail(blocks, budget) -> (Vec<String>, usize)` (private, small/generic, :385) – generic "keep the last N that fit a byte budget" reducer. Candidate for reuse – same shape recurs conceptually wherever a byte budget truncates a list (e.g. cache/spill-index rendering in other partitions); worth checking against those.
- `salvage_header(payload, transcript, now, dropped) -> String` (private, :405) – the R54 provenance header.
- `write_salvage(report, payload, now, rendered) -> Result<PathBuf>` (private, :437) – R17 tracked-path re-check, `mkdir`, then `create_new` retry loop for a unique name.
- `compact_stamp(now) -> String` (private, small/generic, :498) – `YYYYmmdd-HHMMSS` derived from `scratchpad::format_rfc3339`.
- `agent_slug(payload) -> String` (private, small/generic, :512) – sanitize agent id to a filename-safe slug, bounded at 40 chars.
- `rel_to_state(report, path) -> String` (private, small/generic, :543) – path relative to the worktree root (derived from `report.state_root`'s grandparent). **Near-duplicate** of `pre_compact.rs`'s `rel_to_repo` (see L1).
- `tracked_under(report, rel) -> bool` (private, :559) – exact-or-prefix match against `Refusal::TrackedPaths`. **Near-duplicate** of `pre_compact.rs`'s `is_tracked` (exact-only) (see L2).

### `crates/ss-magic-plugin/src/hook/file_changed.rs` (389 lines)
Purpose: `FileChanged` handler (unwired – no manifest entry, see module doc) – gate a `.env`/`.envrc` change through `direnv status --json` (read-only trust check) before exporting via `direnv export bash` into the harness's `CLAUDE_ENV_FILE`.

- `handle(ctx: &HookContext<'_>) -> Result<Outcome>` (pub(crate), :128) – reads `CLAUDE_ENV_FILE` from the environment, calls `handle_with`.
- `handle_with(ctx, program, raw_target) -> Result<Outcome>` (private, :134) – the whole testable pipeline: rc-path match, `unlink` short-circuit, absolute-dir check, target resolution, boundary check, direnv status, direnv export, locked append.
- `first_rc_path(payload) -> Option<String>` (private, :228) – first of `file_path`/`file_paths` matching `RC_NAMES`.
- `is_rc_path(path) -> bool` (private, small/generic, :238) – filename membership in `RC_NAMES`.
- `resolve_target(raw: &str) -> Result<PathBuf, String>` (private, :253) – validates+canonicalizes `CLAUDE_ENV_FILE`'s parent. **Name collides** with `pre_tool_use.rs:272 resolve_target(cwd, raw) -> Target` (unrelated; see L15).
- `within(path, boundary) -> bool` (private, small/generic, :283) – canonicalize-and-`starts_with`, fail-closed (unresolvable boundary counts as containing). **Same shape** recurs at `config.rs:893`, `expect_artifact.rs:388`, `scratchpad.rs:349`, `status.rs:1771/1778` with varying fail directions (see L16, cross-partition).
- `Status` (private struct, :290) – `{ allowed: Option<i64>, rc_path: Option<String> }`.
- `Status::rc_display(&self, changed) -> String` (:301) – direnv's path or the changed path.
- `direnv_status(program, dir) -> Result<Option<Status>>` (private, :315) – runs `direnv status --json`, `Ok(None)` = not installed.
- `parse_status(stdout) -> Status` (private, :333) – picks `state.foundRC.{allowed,path}` out of loose JSON.
- `direnv_export(program, dir) -> Result<Option<Option<String>>>` (private, :348) – runs `direnv export bash`.
- `run_direnv(program, dir, args) -> Result<Option<Output>>` (private, small/generic, :362) – the shared `Command::new(program)...output()` wrapper distinguishing "not found" from a real error. Generic subprocess-runner shape; only one caller pattern here but structurally similar to other `Command::new(...).output()` sites elsewhere in the workspace (not verified as duplicated – worth a spot-check by the converge pass).
- `append_export(target, script) -> Result<()>` (private, :376) – owner-only-mode append-with-leading-newline write.

## Constants and literals

| item | file:line | value | denotes | elsewhere in workspace? |
|---|---|---|---|---|
| `BYTES_PER_LINE` | pre_tool_use.rs:79 | `40` | avg bytes/line, converts line threshold to byte threshold | not found duplicated (per numbers.md/consts.md) |
| `NON_TEXT_EXTENSIONS` | pre_tool_use.rs:88 | 11-entry `&str` array | extensions never gated | not duplicated elsewhere |
| `MAX_WALK_DEPTH` | pre_tool_use.rs:95 | `64` | worktree-walk bound | not duplicated (index/consts.md has no other `MAX_WALK_DEPTH`) |
| `ROOT_MARKER` | pre_tool_use.rs:100 | `".superset/magic.json"` | worktree-root marker file | same STRING literal is `MAGIC_JSON`'s value joined with `SUPERSET_DIR` conceptually (`superset_files.rs:26,30` = `".superset"` + `"magic.json"`); no shared const – this file hardcodes the joined path independently. Candidate to derive from core's constants instead of a fresh literal (L4). |
| `"ss-magic-plugin"` prefix in every deny/nudge string | pre_tool_use.rs:614,622-634,1099,1117-1166,1140-1166,1558-1563 | literal text | model-facing command prose | Same wrapper-name convention appears in `session_start.rs` (per CLAUDE.md's own note that it "spells the same family the same way") – see L5, cross-partition prose consistency, not a code constant to extract but worth a shared-template lead. |
| `SALVAGE_DIR` | subagent_stop.rs:64 | `"research-salvage"` | salvage subdirectory name | not duplicated |
| `SALVAGE_BYTE_BUDGET` | subagent_stop.rs:70 | `64 * 1024` | salvage size cap | not duplicated (distinct from `spill_index.rs`'s `KIB`/`MIB` and `ledger.rs`'s `READ_BUF_BYTES = 256 * 1024`) |
| `MAX_TRANSCRIPT_BYTES` | subagent_stop.rs:75 | `16 * 1024 * 1024` | transcript read cap | not duplicated |
| `MAX_NAME_ATTEMPTS` | subagent_stop.rs:80 | `20` | salvage filename retry bound | dup-name-only collision with unrelated `MAX_NAME_ATTEMPTS`-shaped consts elsewhere? consts.md shows none by this exact name elsewhere – no duplicate |
| `DIR_MODE` | subagent_stop.rs:85 | `0o700` | dir perm | **dup-name across 8 files** per consts.md (bypass.rs, cache.rs, expect_artifact.rs, heartbeat.rs, ledger.rs, scratchpad.rs, tmproot.rs, subagent_stop.rs) – same value everywhere, each hand-copied. This file's own doc comment at :82-84 explicitly says "repeated here rather than imported ... which keeps its own copies private" – a **deliberate** duplication acknowledged in-file, but the pin doc (structure-pins.md) doesn't cover this one; flag as a genuine finding candidate for the converge pass (L6, cross-partition). |
| `FILE_MODE` | subagent_stop.rs:86 | `0o600` | file perm | same pattern, **dup-name across 9 files** including `file_changed.rs:95` (also `0o600`) – see L6/L7 |
| `FILE_MODE` | file_changed.rs:95 | `0o600` | secret-export file perm | same as above; this file's own doc (:91-94) also explains the "at creation only" caveat, independent copy of the same literal |
| `LOCK_NAME` | file_changed.rs:106 | `"file-changed.lock"` | lock file name under tmproot | **dup-name** (3 files use `LOCK_NAME` for different lock filenames: checklist/verbs.rs `"checklist.lock"`, file_changed.rs `"file-changed.lock"`, release_check.rs `"release-check.lock"`) – same name, different string values, not a collision risk since each lives in a differently-named tmproot subdir, but worth the converge pass noting the naming pattern is consistent (not a bug) |
| `DIRENV` | file_changed.rs:85 | `"direnv"` | subprocess program name | not duplicated |
| `ENV_FILE_VAR` | file_changed.rs:89 | `"CLAUDE_ENV_FILE"` | harness env var name | not duplicated |
| `ALLOWED` | file_changed.rs:114 | `0` (`i64`) | direnv "allowed" status code | not duplicated; note the const name `ALLOWED` is generic/short for a file-scope grep |
| `RC_NAMES` | file_changed.rs:120 | `[".envrc", ".env"]` | direnv-recognized rc filenames | core's `sync/repo_scan.rs:14 OPTIONS` and `superset_files/tests.rs:63 OPTIONS` both hold `[".env", "**/.env", ".env.local", "**/.dev.vars"]` – a **different but overlapping** list (glob patterns for `.env`-family files used by the sync bootstrap picker) vs this file's exact-filename list for direnv. Related domain, different purpose – flag as a candidate for the converge pass to confirm they should stay separate (L8, cross-partition). |
| hardcoded `"status"`, `"--json"`, `"export"`, `"bash"` | file_changed.rs:316,349 | direnv subcommand args | not duplicated |
| `40` chars slug cap | subagent_stop.rs:522 (`take(40)`) | bare number, not a const | filename length bound | not indexed in numbers.md (under the >=60 threshold) but worth naming as a magic number – L9 |

## Cross-module references

**Imports into these three files:**
- `pre_tool_use.rs`: `crate::bypass`, `crate::cache`, `crate::checklist`, `crate::pathnorm`, `crate::hook::event::{Payload, PermissionDecision, PreToolUse, PreToolUseResponse, Response}`, `crate::hook::{HookContext, Outcome}`, `crate::scratchpad::STATE_REL`, plus `globset::Glob`.
- `subagent_stop.rs`: `crate::cache::{self, Budget}`, `crate::expect_artifact::{self, Expectation, Unmet}`, `crate::hook::event::{Payload, Response, SubagentStop}`, `crate::hook::{HookContext, Outcome}`, `crate::scratchpad::{self, Refusal, Report}`.
- `file_changed.rs`: `crate::hook::event::{FileChanged, Payload}`, `crate::hook::{HookContext, Outcome}`, `crate::tmproot`.

**Where these files' own public/pub(crate) symbols are used elsewhere** (all three `handle` functions are called only from `hook/mod.rs::route`, at line 324/326/332 respectively – see functions.md's `handle` group). No other item in any of the three files is `pub` (crate-visible items are `Classification`/`Classifier` in `pre_tool_use.rs`, used only inside that file and its own `tests.rs`). **Everything else is unused outside its own file.**

**What these files pull FROM elsewhere that other partitions also pull from** (candidates for the converge pass to cross-reference against other partitions' recon):
- `checklist::matches_convention`, `checklist::pointer_target`, `checklist::ACTIONS_REL` – used only by `pre_tool_use.rs` among hook handlers (per grep), defined in `checklist/verbs.rs:191,150,94`.
- `scratchpad::format_rfc3339` – used by 8 files across the plugin crate (cache.rs, release_check.rs, spill_index.rs, heartbeat.rs, status.rs, pre_compact.rs, session_start.rs, subagent_stop.rs, checklist/render.rs, checklist/schema.rs) – a genuinely shared, correctly-reused helper, not a lead.
- `cache::envelope` – used by both `subagent_stop.rs` and `checklist/render.rs` – correctly shared (per module docs, deliberate: "both are ss-magic-generated text ... get the same treatment").
- `git::status_porcelain` – used by `pre_tool_use.rs` (staleness) and defined/tested in `ss-magic-core/src/git/mod.rs`; no other plugin caller.

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | copy-paste-variant | `subagent_stop.rs:543-554` (`rel_to_state`) vs `pre_compact.rs:132-136` (`rel_to_repo`) | Both compute "path relative to the worktree root, in `git::tracked_files` form" with an identical `strip_prefix(...).map(...).unwrap_or_else(...)` body; `rel_to_state` derives the root from `report.state_root`'s grandparent while `rel_to_repo` takes `repo_root` directly – same operation, two names, two derivations of the same root value. | `crates/ss-magic-plugin/src/hook/pre_compact.rs:130-136` |
| L2 | copy-paste-variant | `subagent_stop.rs:559-568` (`tracked_under`) vs `pre_compact.rs:140-144` (`is_tracked`) | Both scan `report.refusals` for `Refusal::TrackedPaths` matching a `rel` string; `tracked_under` additionally matches a directory-prefix (`p.starts_with(&prefix)`) because it guards a directory (`research-salvage/`) while `is_tracked` only guards one file. The core scan-and-match logic is duplicated; only the prefix branch differs. Both exist because "R17's re-check" is spelled out as a rule each handler must repeat (see CLAUDE.md's fail-open/fail-closed section) – so some duplication is expected, but the exact-match half could be one shared helper with an optional prefix mode. | `crates/ss-magic-plugin/src/hook/pre_compact.rs:138-144`; also check `scratchpad.rs`'s own `is_tracked`-shaped method at scratchpad.rs:363 (`fn is_tracked(&self, path: &Path) -> bool`, different signature/purpose – verify it isn't a third variant) |
| L3 | duplication | `pre_tool_use.rs:1115-1126` (`hit_epilogue`) vs `pre_tool_use.rs:1138-1174` (`miss_reason`) | Both end with the identical bypass-invocation paragraph: `"To read the raw bytes in THIS window instead, run exactly:\n\n    ss-magic-plugin bypass {shown}\n\n and read the file again. That lets exactly the next Read of this file through, once; the one after it is gated again.\n"` – verbatim repeated string within one file. A shared `fn bypass_paragraph(shown: &str) -> String` would remove the duplicate prose (functions.md doesn't show this as a separate function today). | same file, `pre_tool_use.rs:1121-1124` and `:1165-1168` |
| L4 | hardcoded-value | `pre_tool_use.rs:100` (`ROOT_MARKER = ".superset/magic.json"`) | Hardcodes the join of `SUPERSET_DIR` (`".superset"`) and `MAGIC_JSON` (`"magic.json"`), both defined in core's `superset_files.rs:26,30`, as one fresh literal rather than referencing those constants (core is a dependency of the plugin crate). If either changes, this literal silently drifts. | `crates/ss-magic-core/src/superset_files.rs:26,30` |
| L5 | naming/quality | `pre_tool_use.rs:607-635,1108-1174,1556-1565` | Every model-facing command in this file is manually re-spelled `ss-magic-plugin ...` in each of `checklist_reason`, `hit_epilogue`, `miss_reason`, `nudge_text` – four independent format! strings hand-typing the same command family (`checklist list/render-md/verify/init/add-item/add-entry/set/done`, `bypass`, `conclude`). A single source of the command names (even just constants) would prevent one of these four prose blocks drifting out of sync with the checklist verbs' actual argument order in `checklist/verbs.rs`. | `crates/ss-magic-plugin/src/checklist/verbs.rs` (verb definitions) and `crates/ss-magic-plugin/src/hook/session_start.rs:49` (`CHECKLIST_VERBS` const, which appears to be exactly this shared text already extracted once – worth checking whether `pre_tool_use.rs`'s copies should instead reuse that const) |
| L6 | const-location | `subagent_stop.rs:85-86` (`DIR_MODE`/`FILE_MODE` = `0o700`/`0o600`) | Same two constants, same two values, independently declared in 8-9 files across the plugin crate (bypass.rs, cache.rs, expect_artifact.rs, heartbeat.rs, ledger.rs, scratchpad.rs, tmproot.rs, subagent_stop.rs, file_changed.rs). The doc comment at `subagent_stop.rs:82-84` explicitly defends the duplication ("keeps its own copies private"), but this specific pattern (mode constants, not the containment-check pattern) is not listed in structure-pins.md's "deliberate duplications" section – worth the converge pass confirming whether this is meant to be pinned too, or is a genuine opportunity for one `crate::modes::{DIR_MODE, FILE_MODE}` pair. | every file in consts.md's `DIR_MODE`/`FILE_MODE` dup-name rows |
| L7 | const-location | `file_changed.rs:95` (`FILE_MODE = 0o600`) | Same as L6, specific to this file: the file's own doc (:91-94) re-explains the owner-only rationale independently of every other copy's rationale comment. | same set as L6 |
| L8 | reuse (cross-partition) | `file_changed.rs:120` (`RC_NAMES = [".envrc", ".env"]`) | Overlaps in subject matter (`.env`-family filenames) with core's `sync/repo_scan.rs:14 OPTIONS = [".env", "**/.env", ".env.local", "**/.dev.vars"]`, used by the bootstrap file-picker. Different purpose (exact filename match for direnv vs glob patterns offered to the user) – likely correctly separate, but the converge pass should confirm no shared list was intended. | `crates/ss-magic-core/src/sync/repo_scan.rs:14-16` |
| L9 | hardcoded-value | `subagent_stop.rs:522` (`.take(40)`) | Bare `40` (slug length cap) with no named constant, unlike every other magic number in this file's neighborhood (`MAX_NAME_ATTEMPTS`, `SALVAGE_BYTE_BUDGET`, etc. are all named). Minor naming/quality nit. | none – self-contained |
| L10 | efficiency | `pre_tool_use.rs:975-976` (`configured_exemption`) vs `:405-433` (`resolve_for_classification`) | The module's own doc at :949-966 already explains why these two `canonicalize()` calls on the same file cannot be merged (different fallback semantics needed for different callers) – flagging only so the converge pass does NOT propose merging them; this is a documented non-finding, include as a "settled, do not re-flag" note rather than a fix. | n/a – self-referential, informational only |
| L11 | duplication (cross-partition) | `pre_tool_use.rs:444-451` (`paths_equal_ignoring_case`) and `:567-582` (`ends_with_ignoring_case`) | Two generic, self-contained ASCII-case-folding path helpers defined private to this file. Both are exactly the kind of small generic helper the brief calls out as a candidate to be shared – check whether `pathnorm.rs` (which already owns `normalize`/`home_relative`/`process_view`) is the natural home, since it's the module already responsible for path-shape reasoning in this crate. | `crates/ss-magic-plugin/src/pathnorm.rs` (no case-insensitive helpers found there today) |
| L12 | naming | `pre_tool_use.rs:272` `resolve_target(cwd: &Path, raw: &Path) -> Target` vs `file_changed.rs:253` `resolve_target(raw: &str) -> Result<PathBuf, String>` | Same function name, unrelated signatures and purposes, in sibling files of the same `hook/` module. Not a bug (both are private, no ambiguity), but a reader grepping `resolve_target` across the module gets two unrelated hits – naming clarity nit. | both files, both private |
| L13 | duplication (cross-partition) | `file_changed.rs:283-286` (`within`) | Canonicalize-then-`starts_with` containment check, fail-CLOSED (unresolvable boundary treated as containing – refuses the export). The **same shape** (canonicalize + `starts_with`) recurs at `config.rs:893`, `expect_artifact.rs:388`, `scratchpad.rs:349`, `status.rs:1771` and `:1778`, each in a different partition, each with its own fail direction hand-decided locally. Given the hard-rule emphasis in CLAUDE.md on "phrase every gate so the unknown answer is the safe one," a shared `contains(path, boundary, on_error: Safe|Unsafe)`-style helper (or at minimum a documented convention) could prevent the fail-direction choice from being silently re-derived five times. Flag for the converge pass to weigh against each site's specific fail-direction requirement (this is a security-adjacent area – do not thin any of them). | `crates/ss-magic-plugin/src/config.rs:893`; `crates/ss-magic-plugin/src/expect_artifact.rs:388`; `crates/ss-magic-plugin/src/scratchpad.rs:349`; `crates/ss-magic-plugin/src/status.rs:1771,1778` |
| L14 | dead-code (context, not a bug) | `file_changed.rs` (whole file, 389 lines) + `file_changed/tests.rs` (922 lines) | The module's own doc (:1-13) states this handler is shipped but **unwired** – no `FileChanged` entry in `plugin/hooks/hooks.json` – so none of this code runs in a real session today. This is documented as deliberate (CLAUDE.md's Architecture section repeats it), so NOT a finding to fix, but flagging so the converge pass does not mistake the module's size (389 lines of logic + 922 lines of tests for code that never executes) for waste; it's pre-built for a future manifest change. | `plugin/hooks/hooks.json` (no `FileChanged` entry) |
| L15 | efficiency/duplication (cross-partition) | `subagent_stop.rs:385-398` (`keep_tail`) | Generic "keep the last N items that fit a byte budget, report how many were dropped" reducer. Worth checking whether `cache.rs` or `spill_index.rs` (both of which manage byte-budgeted rendering, per their consts like `ENTRIES_KEPT`, `LIST_BYTE_BUDGET`) already implement the same tail-keep-with-drop-count pattern under a different name – if so, this is a third independent implementation. | `crates/ss-magic-plugin/src/cache.rs` and `crates/ss-magic-plugin/src/spill_index.rs` (not verified in this partition; flagged for the converge pass to check) |
| L16 | quality | `pre_tool_use.rs:1078-1091` (`grouped`) | Hand-rolled thousands-separator formatter (`103000` → `"103,000"`), used exactly once (`miss_reason`). Check whether an equivalent already exists in `ss-magic-core` (e.g. near `ledger.rs`'s cost formatting, which also prints numbers for a human) before accepting this as a one-off. | `crates/ss-magic-plugin/src/ledger.rs` (cost-report formatting; not verified, flagged for converge pass) |

Notes for the converge pass: this partition's files are unusually well-documented in-line (extensive module/function doc comments explaining *why* something is NOT simplified – e.g. L10, L14). Per structure-pins.md and the review rules, do not re-propose anything the file's own doc comment already defends unless the lead specifically targets the defended choice itself (none of L1-L16 do; L10 and L14 are included only as "already settled, do not re-flag" markers for the converge pass's benefit).
