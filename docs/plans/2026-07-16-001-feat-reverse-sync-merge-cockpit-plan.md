---
title: Reverse-Sync Merge Cockpit - Plan
type: feat
date: 2026-07-16
topic: reverse-sync-merge-cockpit
artifact_contract: ce-unified-plan/v1
artifact_readiness: requirements-only
product_contract_source: ce-brainstorm
execution: code
---

# Reverse-Sync Merge Cockpit - Plan

## Goal Capsule

- **Objective:** Replace the re-printing reverse-sync picker with a full-screen `ratatui` "merge cockpit" that fixes the stacking bug, shows a live side-by-side diff, and lets a developer reconcile git-untracked secret files between a worktree and main — push, pull, or interactive per-hunk merge — without footguns.
- **Product authority:** Owner (Viktor). UX shaped by the `ce-ideate` candidate set and a Fable/Codex/Opus design council; final call recorded in Key Decisions.
- **Open blockers:** None. New crates (`ratatui`, `similar`) are additive; `crossterm`/`console`/`tempfile` are already in the tree.

## Product Contract

### Summary

A full-screen reverse-sync cockpit: a file list beside a live side-by-side diff (*Local file* vs *Main branch*). The developer moves with arrows, sets each file's direction with explicit keys (`p` push / `l` pull / `m` merge / `u` undecided), and reconciles hunk-by-hunk when merging. It is deliberately conservative with secrets: nothing destructive is pre-selected, applying is gated by one batched confirm, and every overwrite writes a timestamped backup first.

### Problem Frame

Reverse sync writes git-untracked secrets (`.env`, `.dev.vars`, `config.local.*`) from a worktree back into the shared main checkout. The current picker re-invokes `inquire::Select` in a loop, so every keypress leaves a permanent "Untracked files to push back to main:" line — the screen stacks (the reported bug). The diff is a `git diff --no-index` pager: unified, no side-by-side, no titles. And the only reconcile direction is "push"; a developer who edited a secret in both places, or wants main's version, has no in-tool path. Because these files are untracked, a wrong overwrite has **no git undo** — so the redesign must be both richer and safer than what it replaces.

### Key Decisions

- **KD1. `ratatui` for this one screen** *(session-settled: user-directed — chosen over hand-rolled crossterm and embedding scm-record during ideation: richest fit for a navigable cockpit, and `crossterm 0.29` is already the resolved backend via `inquire`, so only the `ratatui` crate is added).* `inquire` stays for every other prompt (menu, bootstrap, config edit).
- **KD2. `similar` as the single diff engine** *(session-settled: user-approved — over `diffy`/`imara-diff`: `grouped_ops(3)` gives context-folded hunks and `iter_inline_changes` gives intra-line highlighting, and one diff pass drives both the view and the merge decisions).* These files have no common ancestor, so merge is a **base-less 2-way reconcile**, never a 3-way merge.
- **KD3. Interaction: always-visible split-pane (layout) + explicit letter-key decisions (input).** Final call from the Fable/Codex/Opus council: two of three flagged arrow-cycling of destructive directions as the design's one real footgun (a stray `←→` silently flips push↔pull). Keep the live diff pane; set decisions with `p`/`l`/`m`/`u`; reserve arrows for navigation/scroll.
- **KD4. Conservative safety posture.** Nothing destructive is pre-selected; differing files start *undecided*; applying is gated by one batched confirm naming every overwrite; a timestamped **pre-write backup** of the losing bytes precedes every destructive write (council-unanimous keystone). The existing TOCTOU guard and `ensure_gitignored_in_main` secret-safety step are preserved.
- **KD5. Robustness guards ship in v1** (council pitfalls): narrow terminals fall back to a unified diff; binary/non-UTF-8 files are whole-file only; oversized diffs are capped; no-TTY/piped/CI invocation refuses to launch. A destructive TUI for the typical workflow is not safe without these.
- **KD6. `mtime` is a labeled hint only** *(reasoned: verified unreliable — `std::fs::copy` mtime is platform-divergent and `git checkout`/`worktree add` reset it).* It is shown per file, marked unreliable inline, and never drives a default or implies which side is authoritative.

### Requirements

**Cockpit shell & interaction**

- R1. The reverse-sync screen is a full-screen `ratatui` app with a left file-list pane and a right live diff pane (column titles "Local file" and "Main branch"); it redraws in place and never stacks output.
- R2. `↑`/`↓` (and `j`/`k`) move between files; the diff pane updates live for the focused file. `PgUp`/`PgDn` (and `Space`) scroll the diff. No keypress re-prints.
- R3. Per-file decisions are set with explicit keys — `p` push-to-main, `l` pull-from-main, `m` interactive-merge, `u` undecided — not by arrow-cycling. Each row shows its decision as a distinct badge with an unambiguous direction label (e.g. `worktree → main`) using color plus text, never position alone.
- R4. Differing files start *undecided*; files absent in main default to *push*; nothing destructive is pre-selected. Each row shows both mtimes, labeled as an unreliable hint.
- R5. A persistent footer shows the key legend; `?` opens a help overlay. `Esc` aborts from any screen (including mid-merge) leaving both worktree and main byte-for-byte untouched, with no partial writes.

**Diff rendering**

- R6. The diff is computed with `similar` and renders per-side line numbers, ~3 lines of folded context around each change, ANSI color, and intra-line (word-level) highlighting.
- R7. When the terminal is too narrow for a legible two-column split (a column would fall below ~40 columns), render a unified full-width diff instead — never silently truncate both columns.
- R8. Oversized diffs are capped by a line/byte threshold with a clear "diff truncated / too large to render" notice; whole-file push/pull remain available for such files.
- R9. Binary / non-UTF-8 files are detected and shown as "binary — differs (size/hash)" rather than raw bytes; only whole-file push/pull are offered and interactive merge is disabled for them.

**Interactive merge (base-less 2-way)**

- R10. Choosing `m` opens a per-hunk walk over the `similar` diff: for each differing hunk the user picks keep-local / keep-main / keep-both while equal context passes through; the assembled result is previewed before it is committed as the file's decision.
- R11. Applying a merged file writes the reconciled bytes to **both** the worktree file and main so they stop differing. Merge is unavailable for binary files (R9).

**Apply & safety**

- R12. `Enter` (apply) collects all non-undecided decisions and shows ONE batched confirmation listing every destructive overwrite (path, direction, ± line counts), defaulting to No; `d` re-opens a listed file's diff. Non-destructive creates (new file → push) do not require a diff re-show.
- R13. Before any overwrite or destructive write to either side, the to-be-overwritten (losing) bytes are copied to a timestamped backup whose path is reported in the outcome summary.
- R14. If a file changed on either side since it was reviewed, that file is skipped with a warning rather than clobbering a concurrent edit (the existing TOCTOU guard, extended to both directions).
- R15. Every write into main funnels through the existing `ensure_gitignored_in_main` step so reverse-synced secrets stay ignored in main.
- R16. When there is no interactive TTY (piped, CI, hook), the cockpit refuses to launch with a clear error pointing to the non-interactive path; it never falls through to writing files.

**Scope & integration**

- R17. The cockpit replaces only the reverse-sync screen. Forward sync stays non-interactive; the menu, bootstrap, and config-edit prompts stay on `inquire`; delete is out of scope for v1.

### Key Flows

- F1. **Reconcile.** Open cockpit (candidates already filtered to differing/new by `classify`) → review live diffs, moving with arrows → set each file's decision with `p`/`l`/`m`/`u` → `Enter` → batched confirm of destructive overwrites → per-file: TOCTOU re-check, backup losing bytes, gitignore-safe write → outcome summary (copied / skipped / backup paths).
- F2. **Interactive merge sub-flow.** `m` on a differing text file → per-hunk keep-local/keep-main/keep-both walk → preview assembled result → accept sets the file's decision to "merge (assembled)"; on apply the assembled bytes go to both sides (R11). `Esc` in the sub-flow returns to the cockpit with the file still undecided.
- F3. **Guard flows.** No-TTY → refuse + point to non-interactive path (R16). Binary → whole-file push/pull only (R9). Narrow width → unified diff (R7). Oversized → capped diff + push/pull (R8).

### Acceptance Examples

- AE1. **Covers R7.** Given a 90-column terminal and a file with 120-char lines, when the cockpit renders the diff, then it shows a unified full-width diff (not two ~40-col columns), and a wider terminal shows the two-column view.
- AE2. **Covers R9, R11.** Given a binary secret that differs, when the file is focused, then the pane shows "binary — differs (size/hash)", `m` is disabled, and only `p`/`l` are offered.
- AE3. **Covers R12, R13.** Given two files marked push and one file marked pull (all overwriting existing bytes), when the user presses `Enter`, then one batched prompt lists all three overwrites with directions defaulting to No; on confirm, each overwritten file's prior bytes are backed up and the summary prints the backup paths.
- AE4. **Covers R14.** Given a file reviewed as push, when main's copy changes on disk before apply, then that file is skipped with a "changed since review" warning and main's current bytes are untouched.
- AE5. **Covers R16.** Given `ss-magic` reverse sync invoked with no TTY (piped), when it starts, then it prints a clear error and the non-interactive alternative and writes nothing.
- AE6. **Covers R5.** Given decisions set on several files, when the user presses `Esc`, then no file on either side is modified.

### Success Criteria

- The screen redraws in place — the stacking "Untracked files to push back to main:" repetition is gone.
- The `p`/`l`/`m`/`u` + arrows + `PgUp/PgDn` + `Enter`/`Esc`/`?` model works as specified; direction is never ambiguous.
- The side-by-side diff shows line numbers, ~3-line folded context, color, and intra-line highlighting; narrow/oversized/binary cases degrade gracefully per R7–R9.
- Interactive merge produces the exact assembled bytes and both sides converge (no phantom re-diff next run).
- No destructive write happens without an explicit decision, the batched confirm, and a pre-write backup; no-TTY refuses.
- Pure logic (diff-to-side-by-side model, per-hunk merge assembly, decision→outcome truth table, backup-path naming, width/binary/size classification) is unit-tested; the interactive cockpit is manual-smoke, per repo convention.
- The crate version is bumped (minor, pre-1.0) so the self-updater ships the change; README, CLAUDE.md, CONTRIBUTING, and `.cursor/BUGBOT.md` reflect the new behavior and dependencies.

### Dependencies / Assumptions

- New crates: `ratatui` (0.30) and `similar` (3.1). `crossterm 0.29`, `console`, and `tempfile` are already present (verified against `Cargo.lock`), so `ratatui` adds no new terminal backend.
- Candidate computation, `classify` (identical filtered out), `copy_candidate_into_main`, `ensure_gitignored_in_main`, and the TOCTOU guard already exist in `src/sync/reverse_sync.rs` and are reused/extended, not rewritten.
- The cockpit is reverse-sync only; forward sync (`sync_core`) remains a non-interactive scripted copy.

### Outstanding Questions (deferred to planning)

- Exact backup location and retention (e.g. `.superset/backups/<ts>/` vs a session tempdir) — planning decides, keeping it inside `.superset` or a gitignored path.
- Concrete thresholds: narrow-width column minimum, oversized-diff line/byte cap.
- Whether trailing-newline/CRLF normalization is applied before diffing to avoid spurious hunks (council raised; likely a small helper, deferred).
- Help-overlay contents and whether `d`-from-confirm reuses the cockpit diff renderer or a pager.

### Sources / Research

- `docs/ideation/2026-07-15-interactive-diff-merge-ui-ideation.html` — the five-architecture candidate set; this plan builds idea 1 (ratatui cockpit).
- Fable/Codex/Opus design council (this session) — interaction model, batched confirm, both-sides merge, and the pre-write-backup keystone.
- Verified crate research — `similar` `grouped_ops`/`iter_inline_changes`; `crossterm`/`console` already resolved; `ratatui 0.30 + crossterm 0.29` proven by `gitui`.
- Current contracts to preserve: `src/sync/reverse_sync.rs` (`run`, `classify`, `copy_candidate_into_main`, `ensure_gitignored_in_main`, TOCTOU guard), `src/tui/ui.rs` (`pick_reverse_sync`, `confirm_overwrite_with_diff`), `src/tui/menu.rs` (routing).
