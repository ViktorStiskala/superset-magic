---
title: Deriving "is this a secret?" by ABSENCE from a set is fail-open; determine it positively
date: 2026-07-17
category: logic-errors
module: reverse_sync
problem_type: logic_error
component: security
severity: high
symptoms:
  - "A push into main could skip the gitignore-in-main step for a genuinely-untracked secret, leaving it committable"
  - "A non-UTF-8 or Unicode-normalization-mismatched (macOS NFD/NFC) filename silently classified as 'tracked' and bypassed the secret gate"
  - "The gate was fail-OPEN: absence from an 'untracked files' set was treated as 'tracked = safe to skip'"
root_cause: wrong_default_direction
resolution_type: code_fix
tags:
  - reverse-sync
  - secrets
  - gitignore
  - fail-closed
  - git-ls-files
  - tracked-files
  - unicode-normalization
  - security-default
---

# Deriving "is this a secret?" by ABSENCE from a set is fail-open; determine it positively

## Problem

When the unified sync cockpit gained TRACKED files (not just untracked secrets),
`apply_decision` needed a per-file gate: fire `ensure_gitignored_in_main` (which
keeps a pushed secret gitignored in main) ONLY for an untracked source; a tracked
file is already committed and must NOT gain a `.gitignore` rule.

The tempting derivation was: compute the untracked set (`git ls-files --others`)
and set `source_untracked = untracked.contains(rel)`. This is **fail-open**.
Absence from the untracked set was treated as "tracked ⇒ skip the gitignore
step." But a genuinely-untracked secret can be absent from that set WITHOUT being
tracked:

- A non-UTF-8 filename is dropped by `filter_map(|p| p.to_str())` before the
  probe, so it is never queried → absent → mis-classified "tracked".
- On macOS, git porcelain output vs. `walkdir`/`globset` `PathBuf`s can differ by
  Unicode normalization (NFD vs. NFC), so `set.contains` misses.

In those cases `source_untracked == false` → the entire gitignore step (including
its strict re-verify + bail) is skipped → the secret is written into main
UN-IGNORED → a committable secret. The design's own stated bias ("when
tracked-ness can't be determined, treat as secret") was silently violated.

## Symptoms

- A reverse-synced secret with an unusual filename lands in main without a
  `.gitignore` rule; a subsequent `git add` in main would stage the secret.
- No error — the leak is silent, because the skipped step is exactly the one that
  would have caught it.

## What Didn't Work

`source_untracked = untracked.contains(rel)`, where `untracked` came from
`git ls-files --others` scoped to `to_str()`-able pathspecs. It reads as correct
("is it in the untracked list?") but encodes the wrong default: a lookup MISS
(for any reason — unenumerable name, normalization mismatch, a dropped pathspec)
resolves to "tracked ⇒ safe," which is the fail-open direction for a secret
boundary. The prior, unconditional `ensure_gitignored_in_main` was accidentally
fail-CLOSED here (its `to_str()?` errored the push on a non-UTF-8 path, writing
nothing); the new gate converted fail-closed into fail-open.

## Solution

Determine tracked-ness **POSITIVELY** and bias the undeterminable case toward
"secret." Compute the TRACKED set and negate:

```rust
// git/mod.rs — the mirror of untracked_files
pub fn tracked_files(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<PathBuf>> {
    let mut args = vec!["ls-files", "--cached", "-z", "--"];
    args.extend_from_slice(pathspecs);
    Ok(parse_ls_files_z(&git(&args, Some(repo_root))?))
}

// compute_reconcile_set — positive determination, negated
let wt_untracked = !tracked.contains(&rel);
```

Anything NOT positively known-tracked — a non-UTF-8 name, an oddly-normalized
name, an unenumerable path — is `wt_untracked = true`, so the gitignore step runs.
That step's own `is_ignored` / strict re-verify then fail-closed (a `to_str()`
error on a non-UTF-8 path errors the push, writing nothing). The flag is carried
once on the `Candidate`, threaded into `Baseline::source_untracked`, and the
`apply_batch` default on a bookkeeping miss is also `source_untracked: true`.

## Why This Works

For a SECURITY default, the question must be phrased so the unknown answer is the
SAFE one. "Is it positively tracked?" defaults an unknown to `false` ⇒ treated as
a secret ⇒ gated ⇒ fail-closed. "Is it in the untracked set?" defaults an unknown
to `false` ⇒ treated as tracked ⇒ ungated ⇒ fail-open. Same boolean, opposite
safety posture — the difference is entirely which set you enumerate and which
direction the miss falls.

## Prevention

**Lesson — for a security gate, never derive the protected state by ABSENCE from
a set; determine the SAFE state positively so an enumeration gap fails closed.**
Whenever code decides "is this dangerous?" from `!set.contains(x)`, ask: what
happens when `x` can't be represented / enumerated / normalized into the set? If
the miss lands on the permissive side, invert the question to enumerate the safe
state instead.

Pin it with a regression test in the shape of the historical hole — a source not
enumerable by the probe must STILL be gitignored (or the push must fail), never
written un-ignored. Keep the strict re-verify + bail in
`ensure_gitignored_in_main` as the last-line enforcement:

```rust
// A secret whose name is absent from the tracked probe must fail-closed:
// gitignore_appended == true, or the push errors — never a silent un-ignored write.
```

## Related Issues

- Surfaced in the Task-5 adversarial DESIGN review (before code was written),
  which is where fail-open secret defaults are cheapest to catch.
- The pack-side sibling gotcha from the same run: [pack-backups-exclusion-must-guard-the-directory-walk.md](./pack-backups-exclusion-must-guard-the-directory-walk.md)
- `git/mod.rs::tracked_files` / `parse_ls_files_z`; `sync/reverse_sync.rs`
  `compute_reconcile_set` + `apply_decision`'s `source_untracked` gate.
