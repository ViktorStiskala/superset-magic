---
title: assemble() fused the last local line onto the first main line on keep-both
date: 2026-07-16
category: logic-errors
module: merge
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "Choosing keep-both on a hunk near the end of a file silently merges two unrelated lines into one, e.g. two `KEY=value` pairs from a .env file become a single corrupted line"
  - "The corruption is written to BOTH the worktree and main copies, since Decision::Merge writes the assembled text to both sides"
  - "Only reproduces when the LOCAL text does not end with a trailing newline — files freshly hand-edited in an editor that doesn't append a final newline (routine for .env / .dev.vars)"
root_cause: missing_edge_case
resolution_type: code_fix
tags:
  - merge
  - reverse-sync
  - merge-cockpit
  - trailing-newline
  - text-assembly
  - dotenv
  - secrets
---

# assemble() fused the last local line onto the first main line on keep-both

## Problem

`src/sync/merge.rs` implements the reverse-sync cockpit's per-hunk interactive
merge: `merge_segments` splits two texts (local/worktree vs main) into a list
of `Equal` runs and `Diff` regions, and `assemble` walks that list applying a
per-hunk `MergeChoice` (`Local`, `Main`, or `Both`) to build the final text
that gets written to both sides.

Every line text in a `MergeSegment` retains its own trailing `\n` — that's how
`Equal` segments reproduce the file byte-for-byte — which means the **only**
line in a text that can lack a trailing `\n` is the file's very last line. The
`Both` arm of `assemble` originally just concatenated the two candidate
strings with nothing between them:

```rust
// BEFORE (src/sync/merge.rs)
MergeChoice::Both => {
    out.push_str(local);
    out.push_str(main);
}
```

If the hunk being kept-both happens to be (or end at) the last line of the
local text, `local` has no trailing `\n`, so `main`'s first line gets appended
directly onto it with no separator — the two lines fuse into one. Concretely,
local `"a\nb"` (no final newline) merged keep-both against main `"a\nB\n"`
produced `"a\nbB\n"`: `b` and `B`, two distinct lines, became the single
corrupted line `bB`. Because reverse sync exists specifically to reconcile
hand-edited secret files (`.env`, `.dev.vars`), and those routinely lack a
final newline (most editors don't force one on a quick edit), this was not an
exotic corner case — it was the common shape for exactly the files this
feature was built to merge. And because `Decision::Merge` writes the
assembled bytes to **both** the worktree and main copies, the corruption
wasn't confined to one side to recover from — both copies of the secret were
damaged in the same apply.

## Symptoms

- Merging a file with keep-both on a hunk touching the final line silently
  joins two independent `KEY=value` lines into one malformed line — e.g.
  `API_KEY=abc` and `DB_URL=postgres://...` on adjacent files becomes
  `API_KEY=abcDB_URL=postgres://...`.
- No error, no warning — the assembled preview and the written file both show
  the fused line as if it were intentional merge output.
- Both the worktree and main copies of the file receive the corrupted text,
  since a `Decision::Merge` write goes to both sides.

## Solution

Insert a `'\n'` between the local and main sides in the `Both` arm, but only
when it's actually needed — a non-empty local side that doesn't already end in
`\n`:

```rust
// AFTER (src/sync/merge.rs)
MergeChoice::Both => {
    out.push_str(local);
    // Keep the two sides on distinct lines: a local side
    // without a trailing newline would otherwise fuse its
    // last line onto main's first line. (An empty local
    // side — a pure insert — needs no separator.)
    if !local.is_empty() && !local.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(main);
}
```

The `!local.is_empty()` guard matters on its own: a pure-insert hunk (nothing
on the local side, only an insertion on main) has `local == ""`, and inserting
a `'\n'` there would prepend a spurious blank line to `main`'s content even
though there was never a "last local line" to separate from anything.

## Why This Works

`merge_segments` guarantees every line keeps its own `\n` except possibly the
file's final line. So the only place two concatenated regions can fuse is
exactly the boundary this fix targets: local's last line missing its
terminator, glued onto main's first line. Adding the separator there, and only
there, restores the invariant that every line in the assembled output is
either a complete original line (with its own `\n`) or the file's genuinely
final line (correctly still lacking one, when `main` itself doesn't end in
`\n` either). The empty-local guard preserves the pure-insert case, where
`Both` is really just "emit main verbatim" and no synthetic separator should
appear.

## Prevention

Any text-assembly operation that concatenates two independently-sourced
regions — not just this merge cockpit, any "splice region A and region B
together" logic — must treat "region A doesn't end with the line terminator"
as an explicit case, not an accident to concatenate through. The bug only
shows up when the FIRST side lacks a trailing newline, which is exactly the
"last hunk of a file with no final newline" shape — so any test suite for this
kind of code must include a fixture whose first side lacks a trailing
newline, not just fixtures where both sides are terminated. A test that always
uses `"...\n"`-terminated fixtures will pass while the real, common case (a
hand-edited file with no trailing newline) silently corrupts.

## Related Issues

- Originating plan: [docs/plans/2026-07-16-001-feat-reverse-sync-merge-cockpit-plan.md](../../plans/2026-07-16-001-feat-reverse-sync-merge-cockpit-plan.md)
  (the base-less 2-way merge model and its `assemble`/`merge_segments` design).
- Fixed in commit `92a858d` (`fix(sync,tui): address code-review findings on
  the merge cockpit`), alongside the TOCTOU-baseline fix — see
  [docs/solutions/logic-errors/toctou-guard-review-time-baseline.md](./toctou-guard-review-time-baseline.md).
- `src/sync/reverse_sync.rs`'s `Decision::Merge` arm in `apply_decision` is the
  consumer that writes `assemble`'s output to both the worktree and main
  targets — the reason this bug corrupted two files, not one.
