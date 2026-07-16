---
title: TOCTOU guard took both snapshots back-to-back, so it never spanned the review window
date: 2026-07-16
category: logic-errors
module: reverse_sync
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "A file changed by a teammate or forward sync DURING the (potentially minutes-long) interactive cockpit review is silently overwritten on apply, with never-reviewed bytes"
  - "The 'freshness' guard always reports Unchanged for a target nobody touched in the last few microseconds — it can never actually observe a concurrent edit that happened during review"
  - "A non-NotFound `fs::metadata` error (e.g. a permission error) on the pre-write snapshot is read as 'file is missing', which skips the mandatory pre-overwrite backup for a target that in fact exists"
root_cause: race_condition
resolution_type: code_fix
tags:
  - reverse-sync
  - toctou
  - race-condition
  - merge-cockpit
  - metadata-baseline
  - mtime
  - fs-metadata
---

# TOCTOU guard took both snapshots back-to-back, so it never spanned the review window

## Problem

The reverse-sync merge cockpit (`ss-magic`'s "push/pull/merge untracked secrets
back into main" flow) opens a full-screen `ratatui` UI in `cockpit::run_cockpit`
and lets the user review file-by-file for as long as they like — realistically
minutes. Only after the user confirms does `reverse_sync::run` walk the
decisions and write bytes via `apply_decision`.

The guard meant to catch "something changed the target while I was staring at
the diff" took its **two** metadata snapshots inside `apply_decision` itself,
back-to-back, with a no-op hook between them:

```rust
// BEFORE (src/sync/reverse_sync.rs)
fn check_target(target: &Path, hook: &mut dyn FnMut()) -> Guard {
    let pre = match fs::metadata(target) {
        Ok(m) => m,
        Err(_) => return Guard::Missing,
    };
    hook();
    match fs::metadata(target) {
        Ok(now) if pre.len() == now.len() && pre.modified().ok() == now.modified().ok() => {
            Guard::Unchanged
        }
        _ => Guard::Changed,
    }
}
```

In production `hook` is a literal no-op, so `pre` and the re-read `now` are
taken microseconds apart, at *apply* time — long after the user actually
looked at the file. The window this guard needed to cover was "user opened the
cockpit → user confirmed", not "just before I write → right before I write".
Structurally, it could never fire: nothing meaningfully changes a file in the
gap between two adjacent `fs::metadata` calls, so `Guard::Unchanged` was
guaranteed to win every real race. A file edited by a teammate, or overwritten
by an unrelated forward `ss-magic sync`, while the user was reviewing the
cockpit was silently clobbered on apply — the pre-write backup was the only
safety net, and only because it fires unconditionally on the (always-true)
`Unchanged` branch, not because the guard detected anything.

A second, independent bug rode along in the same function: `Err(_) =>
Guard::Missing` treated *any* `fs::metadata` failure — including a permission
error on a target that plainly exists — as "the file doesn't exist yet". That
skips the backup path entirely (`Missing` means "fresh write, nothing to lose"
to the caller), so a stat error on an existing file meant the write proceeded
with zero backup of the file it was about to overwrite.

## Symptoms

- Editing (or forward-syncing into) a candidate file at any point after
  opening the merge cockpit, and before confirming, has the change silently
  discarded — the applied write uses the bytes the user reviewed a moment
  ago, not the current on-disk bytes, with no warning that anything moved.
- The guard reports `Unchanged` on effectively every apply, because its two
  observations are never more than a function call apart.
- A stat error (permissions, I/O) on an existing target is read as "missing",
  so the file's prior bytes are never backed up before being overwritten.

## Solution

Capture a **review-time baseline** — `(len, mtime)` per candidate, on both the
worktree and main side — before the cockpit ever opens, and thread it through
to the apply seam so `check_target` compares "what the user saw" against
"what's on disk right now", not two readings of "right now".

**1. A lightweight, explicitly-optional metadata snapshot (`FileMeta` / `meta_of`)**
that treats "missing" and "other I/O error" as distinct outcomes:

```rust
// src/sync/reverse_sync.rs
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// The file's length in bytes.
    pub len: u64,
    /// The file's modification time, when the platform / filesystem reports one.
    pub mtime: Option<SystemTime>,
}

/// Returns `Ok(None)` ONLY when the path does not exist (`ErrorKind::NotFound`);
/// `Ok(Some(..))` when it exists; and propagates any OTHER io error (permissions,
/// I/O) via `?`. A non-`NotFound` stat error must NEVER be silently read as
/// "missing" — doing so would skip the mandatory pre-overwrite backup for a
/// target that actually exists.
pub fn meta_of(path: &Path) -> Result<Option<FileMeta>> {
    match fs::metadata(path) {
        Ok(m) => Ok(Some(FileMeta {
            len: m.len(),
            mtime: m.modified().ok(),
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading metadata of {}", path.display())),
    }
}
```

**2. The baseline is captured in `run()` right before `cockpit::run_cockpit` is invoked** —
this is the load-bearing ordering fix, not the struct shapes:

```rust
// src/sync/reverse_sync.rs, in run()
let mut baseline: HashMap<PathBuf, (Option<FileMeta>, Option<FileMeta>)> = HashMap::new();
for (rel, _status) in &offered {
    let wt_meta = meta_of(&worktree_root.join(rel))?;
    let main_meta = meta_of(&main_root.join(rel))?;
    baseline.insert(rel.clone(), (wt_meta, main_meta));
}

// Full-screen cockpit: the user sets each file's direction and either
// cancels (main untouched) or confirms a batch of decisions.
let decisions = match cockpit::run_cockpit(worktree_root, main_root, &offered)? { .. };
```

**3. `check_target` compares the baseline against current metadata**, and a stat
error at apply time now fails safe (`Changed`, i.e. skip) instead of silently
becoming `Missing`:

```rust
// src/sync/reverse_sync.rs
fn check_target(target: &Path, baseline: Option<&FileMeta>) -> Guard {
    let current = match meta_of(target) {
        Ok(c) => c,
        Err(_) => return Guard::Changed,
    };
    match (baseline, current) {
        (None, None) => Guard::Missing,
        (None, Some(_)) | (Some(_), None) => Guard::Changed,
        (Some(b), Some(c)) => {
            if b.len == c.len && b.mtime == c.mtime {
                Guard::Unchanged
            } else {
                Guard::Changed
            }
        }
    }
}
```

**4. The baseline is threaded into `apply_decision` via `Baseline` and `ApplyContext`**
(the per-batch roots/backup-dir/timestamp bundle), one `Baseline` per file:

```rust
// src/sync/reverse_sync.rs
pub struct ApplyContext<'a> {
    pub worktree_root: &'a Path,
    pub main_root: &'a Path,
    pub backups_root: &'a Path,
    pub ts: &'a str,
}

pub struct Baseline {
    /// The worktree side's metadata at review time (`None` if it didn't exist).
    pub wt: Option<FileMeta>,
    /// The main side's metadata at review time (`None` if it didn't exist).
    pub main: Option<FileMeta>,
}

pub fn apply_decision(
    ctx: &ApplyContext,
    rel: &Path,
    decision: &Decision,
    baseline: Baseline,
) -> Result<ApplyOutcome> { .. }
```

and each direction's write path guards its destination against the matching
half of the baseline, e.g. `Decision::Push` guards main's side with
`baseline.main.as_ref()`, `Decision::Pull` guards the worktree side with
`baseline.wt.as_ref()`, and `Decision::Merge` guards both sides before writing
either.

## Why This Works

A TOCTOU (time-of-check-to-time-of-use) guard is only meaningful if its
"check" happens at the START of the window it is trying to protect, and its
"use" (or a final re-check) happens at the END. The original code's window was
"just before writing → immediately before writing" — zero real elapsed time,
so the check could never observe anything. Moving the baseline capture to
before `run_cockpit` is called makes the window exactly what the design
requires: "what did the user see when they started reviewing" vs "what is
actually on disk when we're about to overwrite it", spanning the entire
(possibly multi-minute) interactive session in between.

Separating "missing" (`Ok(None)`, `NotFound` only) from "stat failed"
(`Err`, propagated) closes the second bug: a permission or I/O error on an
existing target no longer masquerades as "there's nothing here to back up".

## Prevention

A freshness/TOCTOU guard must anchor its baseline to **when the user last saw
the state**, not to a second read taken right before the write. A guard whose
two observations are adjacent in time looks structurally correct — the code
compiles, the types line up, there's a "before" and an "after" — but it can
never actually fire, because nothing meaningfully changes a file in the
microseconds between two consecutive syscalls. When reviewing a "detect
concurrent modification" guard, find where the review/decision window
*actually* begins in the caller and confirm the baseline snapshot is taken
there, not adjacent to the write.

Related mtime caveat: mtime itself is an unreliable *authority* — the merge
cockpit's own plan notes `std::fs::copy`'s mtime behavior is platform-divergent
and `git checkout` / `git worktree add` reset it, which is why the cockpit only
ever shows mtime as a labeled, non-authoritative hint (KD6) and never uses it to
pick a default direction. That is a separate concern from this fix: comparing
`(len, mtime)` between two same-machine, same-process snapshots is a valid
change *detector* even though mtime is a poor cross-context *authority* — the
TOCTOU baseline only ever compares itself to itself on the same filesystem, so
KD6's caveat about `mtime` as a UI hint does not undermine it as a guard input.

## Related Issues

- Originating plan: [docs/plans/2026-07-16-001-feat-reverse-sync-merge-cockpit-plan.md](../../plans/2026-07-16-001-feat-reverse-sync-merge-cockpit-plan.md)
  (KD4 backup-first apply, KD6 mtime-as-hint-only, R13–R15 concurrent-edit safety).
- Fixed in commit `92a858d` (`fix(sync,tui): address code-review findings on
  the merge cockpit`); the parameter-list version of `apply_decision` was
  later folded into the `ApplyContext`/`Baseline` structs in `131f5f3`
  (`refactor(sync,tui): dedup + tidy per simplification sweep`) without
  changing this fix's behavior.
- [docs/solutions/logic-errors/reverse-sync-untracked-probe-excludes-gitignored-secrets.md](./reverse-sync-untracked-probe-excludes-gitignored-secrets.md) —
  another reverse-sync data-safety bug in the same module, also caught after
  tests passed on an unrepresentative shape.
