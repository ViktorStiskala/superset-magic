# Recon: cli-sync – CLI reverse sync engine and merge model

Partition files: `crates/ss-magic/src/sync/mod.rs`, `crates/ss-magic/src/sync/reverse_sync.rs`,
`crates/ss-magic/src/sync/merge.rs`. Test siblings: `sync/reverse_sync/tests.rs`,
`sync/merge/tests.rs`.

## Map

### `crates/ss-magic/src/sync/mod.rs` (12 lines)

Purpose: module root – declares the two interactive sync submodules and re-exports the pure
half from `ss-magic-core`.

- (no items) – `pub(crate) mod merge;` (line 9), `pub(crate) mod reverse_sync;` (line 10),
  `pub(crate) use ss_magic_core::sync::{apply, pattern, repo_scan, under_excluded_tree};` (line 12).

### `crates/ss-magic/src/sync/reverse_sync.rs` (1410 lines)

Purpose: the bidirectional worktree↔main reconcile engine – candidate computation, diff
classification, the backup-first apply seam, and the two batch entry points (interactive cockpit
and non-interactive bulk push).

Public API (used outside this file):
- `run (pub) – fn(worktree_root: &Path, main_root: &Path) -> Result<ExitCode>` (299) – interactive
  unified Sync entry: compute reconcile set, launch the cockpit, apply decisions.
- `run_bulk (pub) – fn(worktree_root: &Path, main_root: &Path, no_backup: bool) -> Result<ExitCode>`
  (534) – non-interactive `ss-magic reverse-sync`: bulk-push every untracked candidate.
- `backup_forward_targets (pub) – fn(main_root: &Path, cwd_root: &Path, patterns: &[String]) -> Result<()>`
  (615) – pre-copy backup pass for forward `ss-magic sync`.
- `ensure_backups_ignored (pub(crate)) – fn(root: &Path) -> Result<()>` (670) – gitignore the
  `.superset/backups` tree once, shared by the lazy first-sync path and the eager init/migrate
  bootstrap.
- `DiffStatus (pub enum)` (65) – `Differs` / `WorktreeOnly` / `MainOnly` / `Identical`; consumed by
  `tui/cockpit.rs`.
- `ApplyContext<'a> (pub struct)` (1127) – per-batch roots + backups root + timestamp + backup
  toggle threaded through `apply_decision`.
- `Baseline (pub struct)` (1148) – one file's review-time `(wt, main)` metadata + `source_untracked`
  flag.
- `apply_decision (pub) – fn(ctx: &ApplyContext, rel: &Path, decision: &Decision, baseline: Baseline) -> Result<ApplyOutcome>`
  (1181) – the safety-gated write seam (push/pull/merge/delete).

Public items with **no caller outside this file's own functions and its `tests.rs`** (candidates
for `pub(crate)` narrowing, or plain `fn`/private, since the module is already `pub(crate)` in
`sync/mod.rs`):
- `compute_candidates (pub)` (106), `Candidate (pub struct)` (153), `compute_reconcile_set (pub)`
  (178), `classify (pub)` (242), `FileMeta (pub struct)` (861), `meta_of (pub)` (880),
  `WriteDirection (pub enum)` (974), `ApplyResult (pub struct)` (989), `ApplyOutcome (pub enum)`
  (1002).

Private helpers, smallest/most generic first (candidates to compare against similar helpers
elsewhere in the workspace – see Leads):
- `is_safe_rel (private) – fn(rel: &Path) -> bool` (85) – rejects absolute / non-normal components;
  generic path-safety check.
- `write_bytes (private) – fn(target: &Path, bytes: &[u8]) -> Result<()>` (1075) – temp-file-in-
  same-dir → write → preserve-mode → fsync → `persist` atomic write. **Structurally the same
  pattern as the plugin's `atomic::write_atomically` and core's private
  `superset_files::write_atomically`** – a third independent implementation (see Leads L1).
  distinct because it copies the *existing* target's mode rather than taking an explicit `mode`.
- `format_timestamp (private) – fn(secs: u64) -> String` (707) – Hinnant civil-from-days,
  `YYYYmmdd-HHMMSS`. Same algorithm family as `format_rfc3339` in
  `crates/ss-magic-plugin/src/scratchpad.rs` and the inverse `days_from_civil` in
  `crates/ss-magic-plugin/src/checklist/schema.rs` (see Leads L2) – the plugin's own comment at
  scratchpad.rs already calls out this file's copy by name as deliberately separate.
  Not reused elsewhere in this crate either (`tui/cockpit.rs:719-722` hand-rolls its own
  `secs / 86_400` "Nd ago" formatter for a different purpose – relative age, not a calendar date).
- `apply_timestamp (private) – fn() -> String` (695) – `SystemTime::now()` → `format_timestamp`.
- `backup (private) – fn(target: &Path, dest: &Path) -> Result<PathBuf>` (1056) – mkdir parent +
  `fs::copy`. Small and generic – a plain non-atomic file copy-to-backup helper.
- `backup_if_exists (private) – fn(root: &Path, target: &Path, dest: &Path) -> Result<Option<PathBuf>>`
  (592) – file-or-dir backup dispatch used only by `backup_forward_targets`.
- `backup_if_unchanged (private) – fn(target: &Path, guard: Guard, dest: &Path, take_backup: bool) -> Result<Option<PathBuf>>`
  (1112) – the "back up iff enabled and unchanged" rule shared by all four `apply_decision` arms.
- `backups_root_for (private) – fn(root: &Path, ensure_ignore: bool) -> Result<PathBuf>` (681).
- `is_backup_batch_name (private) – fn(name: &str) -> bool` (736) – recognizes the current
  `YYYYmmdd-HHMMSS` or legacy all-digits epoch directory name shape.
- `prune_old_backups (private) – fn(backups_root: &Path, keep: usize, protect: Option<&str>) -> Result<Vec<PathBuf>>`
  (766) – retention over backup batch dirs, folding the legacy `local/<epoch>`/`main/<epoch>` layout.
- `finish_batch (private) – fn(summary: BatchSummary, backups_root: &Path, ts: &str, backup: bool, label: &str) -> ExitCode`
  (395) – shared tail: print backups, prune, print applied/skipped/failed, pick exit code.
- `apply_batch (private) – fn(ctx: &ApplyContext, decisions: &[(PathBuf, Decision)], baseline: &HashMap<PathBuf, Baseline>) -> BatchSummary`
  (469) – drives `apply_decision` over a whole batch, one summary line per file.
- `ensure_gitignored_in_main (private) – fn(worktree_root: &Path, main_root: &Path, rel: &Path) -> Result<bool>`
  (272) – strict re-verify wrapper over `gitignore::ensure_path_ignored`, the secret boundary.
- `review_baseline (private) – fn(worktree_root: &Path, main_root: &Path, rel: &Path, status: DiffStatus) -> (Option<FileMeta>, Option<FileMeta>)`
  (919).
- `baseline_side (private) – fn(meta: Result<Option<FileMeta>>) -> Option<FileMeta>` (948) – folds
  a stat error to `None` (fail-closed).
- `metas_match (private) – fn(b: &FileMeta, c: &FileMeta) -> bool` (959) – length + mtime, hash
  fallback.
- `check_target (private) – fn(target: &Path, baseline: Option<&FileMeta>) -> Guard` (1036).
- `BatchSummary (private struct)` (455), `Guard (private enum)` (1012).
- Consts: `BACKUPS_REL (private, &str = ".superset/backups")` (661), `BACKUP_BATCHES_KEPT
  (private, usize = 10)` (730).

### `crates/ss-magic/src/sync/merge.rs` (277 lines)

Purpose: pure per-hunk merge model (base-less 2-way diff/assemble) and the backup-path naming
primitive, kept `ratatui`-free so it stays unit-testable and is driven by both `reverse_sync.rs`
and `tui/cockpit.rs`.

- `Decision (pub enum)` (34) – `Undecided`/`Push`/`Pull`/`Merge(String)`/`Delete`; consumed by
  `reverse_sync::apply_decision` and `tui/cockpit.rs`.
- `FileState (pub enum)` (51) – `ExistsBoth`/`WorktreeOnly`/`MainOnly`; consumed only by
  `tui::cockpit::file_state` + `default_decision`.
- `default_decision (pub) – fn(_state: FileState) -> Decision` (68) – always returns `Undecided`
  now; the `_state` parameter is unused (doc comment explains: kept for the cockpit's exhaustive
  match and "future use") – see Leads L7.
- `MergeChoice (pub enum)` (74) – `Local`/`Main`/`Both`.
- `MergeSegment (pub enum)` (91) – `Equal(String)` / `Diff { local, main }`.
- `Pending (private enum)` (105) + `impl Pending::into_segment` (111) – coalescing accumulator
  used only inside `merge_segments`.
- `merge_segments (pub) – fn(local: &str, main: &str) -> Vec<MergeSegment>` (128) – `similar`
  line-diff → coalesced segments.
- `diff_count (pub) – fn(segments: &[MergeSegment]) -> usize` (195).
- `assemble (pub) – fn(segments: &[MergeSegment], choices: &[MergeChoice]) -> String` (215).
- `BackupSide (pub enum)` (251) – `Worktree`/`Main`; `dir_name (private) – fn(self) -> &'static str`
  (259) maps to `"worktree"`/`"main"`.
- `backup_rel_path (pub) – fn(ts: &str, side: BackupSide, rel: &Path) -> PathBuf` (272) – pure path
  join, no I/O.

## Constants and literals

| Item | Location | Value | What it denotes | Elsewhere in workspace? |
|---|---|---|---|---|
| `BACKUPS_REL` | reverse_sync.rs:661 | `".superset/backups"` | repo-relative backups tree path | **Yes, in a different representation.** Core's `EXCLUDED_TREES[0]` (`crates/ss-magic-core/src/sync/mod.rs:40`) encodes the same path as component array `&[".superset", "backups"]`. Two independent encodings of one path – see Lead L3. Also appears as a plain string in many test files (`index/literals.md` line 140: 18 occurrences) and in `crates/ss-magic/src/tests/sync.rs:192,226,249,268` and `reverse_sync_flow.rs:58`. |
| `BACKUP_BATCHES_KEPT` | reverse_sync.rs:730 | `10` | retention: newest N backup batch dirs kept | Not found elsewhere in `index/consts.md`. Comparable-purpose retention constants exist elsewhere with different values and names: `ROWS_KEPT` (heartbeat, 2000) and `RECOMMEND_ROWS` (compact_window, 20) – different domains, not the same constant, just the same "keep newest N" shape. |
| `86_400` (seconds/day) | reverse_sync.rs:708,709 | `86_400` | day length in seconds, inside `format_timestamp` | **Yes**, same literal appears in `crates/ss-magic-plugin/src/scratchpad.rs:656,657`, `crates/ss-magic-plugin/src/release_check.rs:564` (×2), `crates/ss-magic-plugin/src/checklist/schema.rs:587`, and `crates/ss-magic/src/tui/cockpit.rs:719,722` (a different, relative-age use). Four independent from-epoch-seconds date/time formatters exist in the workspace (`reverse_sync.rs`, `scratchpad.rs`, `release_check.rs`, `checklist/schema.rs` – the last computing the inverse). See Lead L2. |
| `146_097` (days per 400-year era) | reverse_sync.rs:714,715 | `146_097` | Hinnant civil-from-days era length | Same literal in `crates/ss-magic-plugin/src/scratchpad.rs:662,663` and `crates/ss-magic-plugin/src/checklist/schema.rs:679`. Part of the same algorithm family as `86_400` above. |
| `719_468` (epoch shift to 0000-03-01) | reverse_sync.rs:713 | `719_468` | Hinnant algorithm epoch offset | Same literal in `scratchpad.rs:661` and `checklist/schema.rs:679`. Same family. |
| `1_460` / `36_524` / `146_096` / `365` (year-of-era arithmetic) | reverse_sync.rs:716-717 | as written | Hinnant civil-from-days interior constants | Same shape (not necessarily identical spelling) in `scratchpad.rs:664-665` and the inverse form in `checklist/schema.rs:673-679`. Below the `numbers.md` >=60 threshold in places, but part of the same triplicated algorithm. |
| `.superset/backups/` (with trailing slash, distinct literal) | test files only | – | same tree, test-side literal | `index/literals.md:356` – only in `tests.rs` files (not in the three source files under review), listed here because it is the same concept as `BACKUPS_REL` (no trailing slash) and a reviewer diffing literals should not conflate the two spellings as identical hits. |

No `#[cfg(test)]`-only consts from `reverse_sync/tests.rs` are in scope here except as noted
(`TS = "20260716-000000"` at `reverse_sync/tests.rs:315`, per `index/consts.md:267` – a test fixture
timestamp, not a duplicate of the production format).

Numeric literals below 60 (day/hour/minute arithmetic: `3_600`, `60`, `4`, `100`, `153`, `5`, `2`,
`9`, `10`, `1`, `3`, `12`) at reverse_sync.rs:710,716-721 are the small interior constants of the
same Hinnant algorithm as above; not separately tracked in `index/numbers.md` (below its ≥60
threshold) but relevant to Lead L2 as a whole.

## Cross-module references

**Imports into these files** (reverse_sync.rs, lines 44-60):
- `std::{collections, fs, io::Write, path, process::ExitCode, time}` – stdlib.
- `anyhow::{Context, Result}`.
- `crate::hashing` – re-export of `ss_magic_core::hashing` (per CLAUDE.md's re-export pin); used
  once, `hashing::hash_file` at line 885.
- `crate::sync::apply` – core's `sync::apply` (match_paths, copy_dir_recursive), re-exported via
  `sync/mod.rs:12`.
- `crate::sync::merge::{backup_rel_path, BackupSide, Decision}` – sibling module in this partition.
- `crate::git` and `crate::git::gitignore::{self, Ignored, PathKind}` – probes + the gitignore
  writer (core, re-exported under `crate::` per the CLAUDE.md pin).
- `crate::tui::cockpit::{self, CockpitOutcome}` – the interactive merge cockpit (outside this
  partition, in `crates/ss-magic/src/tui/cockpit.rs`).
- `crate::tui::style` – palette (core's `style.rs` via the CLI's `tui::style` re-export).
- `crate::workspace::superset_files` – core's config I/O, re-exported.
- `crate::sync::under_excluded_tree` (called via `crate::sync::under_excluded_tree(rel)` at
  106/628, not a `use`) – core's `sync::under_excluded_tree`.

merge.rs imports only `std::path::{Path, PathBuf}` and `similar::{ChangeTag, TextDiff}` (line 27) –
no dependency on `reverse_sync.rs` or on `tui`.

**Where these files' public symbols are used elsewhere** (grep verified; ≤5 callers each):
- `reverse_sync::run` → `crates/ss-magic/src/tui/menu.rs:148` (`MenuOp::Sync => reverse_sync::run(...)`).
  Unused outside this call site (plus doc-comment mentions).
- `reverse_sync::run_bulk` → `crates/ss-magic/src/main.rs:407`.
- `reverse_sync::backup_forward_targets` → `crates/ss-magic/src/main.rs:353`.
- `reverse_sync::ensure_backups_ignored` → `crates/ss-magic/src/workspace/migrate.rs:96` (one call,
  behind `ensure_bootstrap_gitignores`).
- `reverse_sync::DiffStatus` → `crates/ss-magic/src/tui/cockpit.rs:50` (`use ...::DiffStatus;`),
  then used at cockpit.rs:146,300,381,388,412,433,481 and more (>5 total; only `use` + first 4 body
  sites shown per the cap).
- `reverse_sync::compute_candidates`, `compute_reconcile_set`, `classify`, `apply_decision`,
  `meta_of`, `Candidate`, `FileMeta`, `ApplyContext`, `Baseline`, `ApplyResult`, `ApplyOutcome`,
  `WriteDirection`, `BackupSide` (merge.rs) – **unused outside this file** except in
  `sync/reverse_sync/tests.rs` (and, for `BackupSide`, inside `merge.rs`/`merge/tests.rs` itself).
  See the Map section's "no external caller" list.
- `merge::Decision`, `merge::FileState`, `merge::default_decision`, `merge::MergeChoice`,
  `merge::merge_segments`, `merge::diff_count`, `merge::assemble` → all imported and used by
  `crates/ss-magic/src/tui/cockpit.rs:47` (`use crate::sync::merge::{assemble, default_decision,
  diff_count, merge_segments, Decision, FileState, MergeChoice, ...}`), then bodies at cockpit.rs
  lines 78-651 (capped at 5 shown per symbol above under `classify`/`FileState`/etc. searches).
- `merge::backup_rel_path`, `merge::BackupSide` → used only by `reverse_sync.rs` (635, 1240, 1278,
  1320, 1327, 1378, 1385) and `merge/tests.rs`; **unused outside this partition**.

## Leads

| # | Category | File:line(s) | Hypothesis | Compare against |
|---|---|---|---|---|
| L1 | duplication | `reverse_sync.rs:1075-1103` (`write_bytes`) | A third independent hand-rolled "temp file in same dir → write → preserve/chmod → fsync → persist" atomic-write implementation, alongside the plugin's `atomic::write_atomically` and core's private `superset_files::write_atomically` (already flagged as non-pinned duplication in structure-pins). This one is bytes-based and preserves the *target's existing mode* rather than taking an explicit mode – a real behavioral difference, not a pure copy, so a merge would need to support both "explicit mode" and "copy existing target's mode" callers. | `crates/ss-magic-plugin/src/atomic.rs:51` (`write_atomically`); `crates/ss-magic-core/src/superset_files.rs:180` (private `write_atomically`). Core cannot depend on the plugin crate, so a shared version (if any) would need to live in core and be usable from all three crates. |
| L2 | duplication | `reverse_sync.rs:695-724` (`apply_timestamp`/`format_timestamp`) | The Hinnant civil-from-days/from-civil algorithm and its "seconds since epoch → UTC string" wrapper appears independently FOUR times across the workspace: here (`YYYYmmdd-HHMMSS`), `scratchpad.rs` (`format_rfc3339`, `YYYY-MM-DDTHH:MM:SSZ`), `release_check.rs` (its own from-epoch formatter), and `checklist/schema.rs` (`days_from_civil`, the inverse direction, for ISO-8601 parsing). `scratchpad.rs`'s own doc comment explicitly acknowledges this file's copy and argues against sharing it (different output format for a different audience) – so this is a "compare and judge, not an automatic merge" lead, but a shared `days_from_secs`/`secs_from_days` core primitive, with each caller only supplying its own `format!` string, was not considered and might resolve all four without conceding the format-independence argument. | `crates/ss-magic-plugin/src/scratchpad.rs:648-670` (`format_rfc3339`, with the comment at 649-654 already discussing this exact file); `crates/ss-magic-plugin/src/release_check.rs:560-575ish` (grep hit at 564); `crates/ss-magic-plugin/src/checklist/schema.rs:663-679` (`days_from_civil`, the inverse). |
| L3 | const-location | `reverse_sync.rs:661` (`BACKUPS_REL = ".superset/backups"`) | This string is a second, independent encoding of the SAME path core already owns as a component array: `EXCLUDED_TREES[0] = &[".superset", "backups"]`. If the backups directory name ever changed, two representations in two crates would need to change in lockstep with no compiler check tying them together. A shared core constant (or a `path_from_components`-style helper feeding both) would remove the duplicate spelling. | `crates/ss-magic-core/src/sync/mod.rs:40` (`EXCLUDED_TREES`). |
| L4 | reuse (visibility too broad) | `reverse_sync.rs:106,153,178,242,861,880,974,989,1002` (`compute_candidates`, `Candidate`, `compute_reconcile_set`, `classify`, `FileMeta`, `meta_of`, `WriteDirection`, `ApplyResult`, `ApplyOutcome`) | All nine are declared `pub` but have zero callers outside this same file and its sibling `tests.rs` (verified by grep across the whole workspace). Since `sync::reverse_sync` is already `pub(crate)` at the module level (`sync/mod.rs:10`), and a child `tests.rs` module can see a parent's private items in Rust without `pub` at all, none of these needs to be `pub`. Not a behavior bug, but the visibility overstates the module's actual public surface and could mislead a reader hunting for the real integration points (`run`, `run_bulk`, `backup_forward_targets`, `ensure_backups_ignored`, `DiffStatus`, `ApplyContext`, `Baseline`, `apply_decision`). | Compare against the module's real cross-file callers listed in "Cross-module references" above – `DiffStatus`, `ApplyContext`, `Baseline`, `apply_decision` genuinely need `pub(crate)`/`pub` because `tui/cockpit.rs` and `tui/menu.rs`/`main.rs` reach them; the nine above do not. |
| L5 | reuse (visibility too broad, merge.rs) | `merge.rs:251` (`BackupSide`), `merge.rs:272` (`backup_rel_path`) | Both are `pub` but used only from `reverse_sync.rs` (same partition) and `merge/tests.rs` – never from `tui/cockpit.rs` or any other crate module. `pub(crate)` (or even file-private, if merge.rs stayed the sole caller) would state the real surface. Lower priority than L4 since these two ARE used cross-file within the partition, just not beyond it. | `reverse_sync.rs:55` (`use crate::sync::merge::{backup_rel_path, BackupSide, Decision};`) – the only outside user. |
| L6 | efficiency / naming | `reverse_sync.rs:592-606` (`backup_if_exists`) vs `reverse_sync.rs:1056-1064` (`backup`) vs `reverse_sync.rs:1112-1123` (`backup_if_unchanged`) | Three functions named `backup*` with overlapping but distinct responsibilities (dir-or-file dispatch; the raw file copy; the guard-gated wrapper around the raw copy) sit in the same file. Not a bug, but the naming makes it easy to reach for the wrong one – e.g. a future caller might call `backup_if_exists` where `backup_if_unchanged`'s TOCTOU guard was actually needed, since only the doc comments (not the names) distinguish "does this file exist" from "is this file unchanged since review". Worth a naming pass, not necessarily a merge. | Internal only – compare the three doc comments at 587-591, 1105-1111, and the call sites at 636 (`backup_if_exists`, forward-sync pre-pass, no TOCTOU concern) vs 1236/1274/1316/1323/1374/1381 (`backup_if_unchanged`, TOCTOU-guarded). |
| L7 | dead-code / naming | `merge.rs:61-70` (`default_decision`) | `default_decision(_state: FileState) -> Decision` always returns `Decision::Undecided` regardless of `_state`; the parameter is unused (leading underscore) and the doc comment says it is "retained for the cockpit's exhaustive `file_state` mapping and future use." Since the function ignores its only argument today, `default_decision()` (no parameter) would say the same thing more plainly – the current signature invites a reader to assume the state matters. Not proposing removal of `FileState` itself (cockpit.rs's `file_state` mapping genuinely uses it), just the now-vestigial parameter on this one function. | `crates/ss-magic/src/tui/cockpit.rs:526` (`let decision = default_decision(file_state(status));`) – the sole caller, which computes `file_state(status)` only to hand it to a function that discards it. |
| L8 | duplication (cross-partition, algorithm shape) | `reverse_sync.rs:960-970` (`metas_match`) | Length-then-mtime-then-hash-fallback "is this the same file" comparison. Worth checking whether the plugin crate's own TOCTOU-style checks (e.g. `heartbeat.rs`/`ledger.rs`'s incremental byte-offset scan, or `checklist/verbs.rs`'s read-modify-write staleness handling) reach for an equivalent shape independently – not confirmed by this partition's own grep (no cross-file caller found), flagged for the converge pass to check against the plugin partition's recon. | Plugin partition's recon (not yet cross-checked from here) – `crates/ss-magic-plugin/src/heartbeat.rs`, `crates/ss-magic-plugin/src/ledger.rs`. |
| L9 | naming | `reverse_sync.rs:830` (`for side in ["local", "main"]`) | The bare string literals `"local"` and `"main"` here name the SAME two legacy backup-side directories that `merge.rs`'s `BackupSide::dir_name` (line 259-265) maps to `"worktree"`/`"main"` for the CURRENT layout – note `"local"` here is the LEGACY name for what `BackupSide::Worktree` calls `"worktree"` today (see the doc comment at reverse_sync.rs:751-753 explaining the rename). The bare strings in the pruning loop are correct (they target the old, pre-rename directory names on purpose) but nothing ties them to `BackupSide`'s naming, so a future rename of `BackupSide::dir_name`'s outputs could silently make this loop's comment stale without a compiler nudge. Documentation/robustness note, not a functional bug. | `merge.rs:258-265` (`BackupSide::dir_name`). |
| L10 | quality (comment convention) | `reverse_sync.rs:25` (module doc, `## Push vs pull scope (R23, KTD10)`) and similar plan-id-only headers at lines 34, 98, 149, 229 | Per structure-pins' review rules, "a comment citing a plan id alone is a finding (category quality)". These section headers cite `R23`/`KTD10`/`R24` etc. alongside a full prose explanation in the following lines, so most are compliant (id + explanation), but flagging the pattern here so the converge pass can spot-check whether every `(R\d+…)`/`(KTD\d+…)` tag in this file is followed by enough explanation for a reader who has never seen the plan, per the CLAUDE.md code-comments rule. | N/A – self-contained within this partition; compare against CLAUDE.md's "Code comments and docs references" section. |
