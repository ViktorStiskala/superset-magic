---
title: A retention/cleanup sweep must recognize every on-disk layout the tool ever wrote, not just the current one
date: 2026-07-16
category: logic-errors
module: reverse_sync
problem_type: logic_error
component: sync-engine
severity: low
symptoms:
  - "Retention reports pruning to the keep budget, but disk usage keeps rows written by an older build forever"
  - "Old backup trees under a superseded directory layout are invisible to the new pruning pass — stale secret pre-images retained indefinitely while the docs promise a bounded backup dir"
root_cause: incomplete_migration
resolution_type: code_fix
tags:
  - retention
  - backups
  - layout-migration
  - reverse-sync
  - prune
---

# Retention must know every layout the tool ever wrote

## Problem

v0.5.0 restructured backup batches from `<epoch>/<rel>` + top-level `local/<epoch>/<rel>` + `main/<epoch>/<rel>` (the unreleased 0.4.0 merge layout) to a single `<YYYYmmdd-HHMMSS>/{worktree,main}/<rel>` directory per batch, and added keep-10 retention. The new `prune_old_backups` matched only batch-shaped names at the top level — so pre-upgrade merge backups under the top-level `local/` and `main/` dirs failed the name check and were never counted against the budget, never pruned, and survived forever. The conservative name filter ("never delete a directory we did not name") is exactly right; the gap was forgetting that *we ourselves* had named directories differently one release earlier.

## Root cause

The retention pass was written against the current layout only. Changing an on-disk layout in the same change that adds cleanup makes the old layout's artifacts invisible to that cleanup — the safest-looking filter (exact-shape matching) quietly excludes the tool's own history.

## Fix

Fold legacy artifacts into the same budget, keyed by the batch identity they share:

- A batch is keyed by its timestamp NAME; one batch may own several directories (`<ts>/`, plus legacy `local/<epoch>/` and `main/<epoch>/`).
- Children of top-level `local`/`main` are folded in only when their names match the batch shapes; anything else under those dirs is foreign and untouched.
- An emptied legacy side dir is removed only when *this run* pruned from it — an unrelated dir merely named `local`/`main` is never deleted, not even when empty.

## Prevention

- When changing an on-disk layout, enumerate every shape previous builds wrote (`git show <old-rev>:<file>` is the authority) and either migrate, clean up, or explicitly document each one — "the new code no longer writes there" is not "nothing lives there".
- Seed retention tests with artifacts in every historical layout, not just the current one.
- Keep the deletion filter allow-listed (exact shapes the tool wrote); widen it per known legacy shape rather than loosening the match.

## Where

- `src/sync/reverse_sync.rs` — `prune_old_backups`, `is_backup_batch_name`
- `src/sync/reverse_sync/tests.rs` — `prune_old_backups_folds_legacy_merge_layout_into_batches`
