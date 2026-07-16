# Concepts

Shared domain vocabulary for this project — entities, named processes, and
status concepts with project-specific meaning. Seeded with core domain
vocabulary, then accretes as ce-compound and ce-compound-refresh process
learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Sync model

### Main checkout
The primary git checkout that linked worktrees branch from and share a common
git directory with — the canonical tree reverse sync writes back into and the
source forward sync copies from.

### Sync patterns
The glob patterns that drive both forward and reverse sync, formed by overlaying
a committed, shared pattern list with an optional per-checkout local list (union,
de-duplicated). They select which local or untracked files cross between the main
checkout and a worktree.

### Forward sync
Copying the files matching the sync patterns from the main checkout into a
worktree, so a freshly created worktree gains the local/untracked files (secrets,
local overlays) that never travel through git.

### Reverse sync
Pushing a worktree's git-untracked files that match the sync patterns back into
the main checkout — the path by which gitignored secrets created in a worktree
reach the shared checkout, since they cannot travel through a git merge. Only
untracked files move; tracked files reach the main checkout through a normal
merge.

### Merge cockpit
The full-screen interactive UI reverse sync opens to reconcile candidates: a
file list beside a live diff (side-by-side or unified, depending on terminal
width), where the developer sets each candidate's reconcile decision
explicitly and applies the whole batch behind one confirmation. Binary,
oversized, or unreadable candidates fall back to a whole-file notice instead
of a diff.

### Reconcile decision
The direction chosen for one reverse-sync candidate in the merge cockpit:
push (worktree → main), pull (main → worktree), merge (a per-hunk reconciled
result written to both sides), delete (removed from both sides, whichever
exist), or undecided (nothing written for that file). Undecided is the
conservative default for any candidate that exists on both sides; only a
worktree-only candidate defaults to push, since that direction is never
destructive.

### Pre-write backup
A timestamped copy of a file's losing bytes, taken immediately before the
merge cockpit overwrites or deletes it on apply, so a mistaken decision is
recoverable. Backups live under a gitignored `.superset/backups/` in the
worktree — one `YYYYmmdd-HHMMSS` (UTC) directory per apply batch, with
`worktree/` and `main/` namespaces inside it for the side the bytes came from
— and are never committed. Retention keeps the 10 newest batches; older ones
are pruned after each apply.

### Pack
Bundling the files matching the sync patterns from the current git repo root
into a single `ss-magic-<repo>.tar.bz2` archive at that root, preserving each
file's repo-relative path. The `<repo>` stem is derived from the normalized
`origin` remote (owner/path segments joined with `_`), falling back to the
primary worktree's basename when no origin exists. A third operation on the
sync patterns alongside forward and reverse sync — a portable snapshot of the
configured file set (for backup, machine transfer, or handoff) rather than a
copy between trees.

### Candidate
A worktree file eligible for reverse sync: it matches the sync patterns and is
git-untracked (whether or not it is gitignored). Tracked files are never
candidates. A candidate byte-identical to main's copy is hidden — nothing to
reconcile; every other candidate is offered in the merge cockpit for a
reconcile decision before anything is written into the main checkout.
