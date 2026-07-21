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
Pushing a worktree's files that match the sync patterns back into the main
checkout. The direct `ss-magic reverse-sync` subcommand bulk-pushes every
git-untracked match – the path by which gitignored secrets created in a
worktree reach the shared checkout, since they cannot travel through a git
merge. The interactive merge cockpit, opened from the worktree menu's unified
Sync entry, can also push a tracked candidate's worktree bytes into main on
request; that push skips the gitignore step, since a tracked file is not a
secret and already reaches main through a normal git merge.

### Merge cockpit
The full-screen interactive UI the worktree menu's unified Sync entry opens
to reconcile candidates in either direction: a file list beside a live diff
(side-by-side or unified, depending on terminal width), where the developer
sets each candidate's reconcile decision explicitly and applies the whole
batch behind one confirmation. Binary, oversized, or unreadable candidates
fall back to a whole-file notice instead of a diff.

### Reconcile decision
The direction chosen for one candidate in the merge cockpit: push (worktree
→ main), pull (main → worktree), merge (a per-hunk reconciled result written
to both sides), delete (removed from both sides, whichever exist), or
undecided (nothing written for that file). The unified Sync cockpit
pre-selects nothing – every candidate opens undecided, and the developer
picks a decision per file before applying the batch.

### Pre-write backup
A timestamped copy of a file's losing bytes, taken immediately before an
apply overwrites or deletes it, so a mistaken decision is recoverable.
Backups live under a gitignored `.superset/backups/` of the root being
overwritten – the worktree for the merge cockpit and forward sync, main for
the direct `ss-magic reverse-sync` subcommand – one `YYYYmmdd-HHMMSS` (UTC)
directory per apply batch, with `worktree/` and `main/` namespaces inside it
for the side the bytes came from, and are never committed. Taking backups is
opt-out (`--no-backup`/`-n` on the direct subcommands) and, when skipped,
leaves an overwritten or deleted file with no recovery path. Retention keeps
the 10 newest batches; older ones are pruned after each apply.

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
A sync-pattern match whose worktree and main copies differ: present only in
the worktree (worktree-only), present only in main (a main-only candidate –
see below), or present on both sides with different bytes (differing).
Patterns are expanded against both the worktree and main checkout, so a
main-only file is visible even though a worktree-only walk would never see
it. A candidate byte-identical on both sides is hidden – nothing to
reconcile; every other candidate is offered in the merge cockpit for a
reconcile decision before anything is written into either tree.

Only a candidate with worktree bytes (worktree-only or differing) can be
pushed; a main-only candidate has no worktree source, so push is unavailable
and it can only be pulled or deleted. Pushing a worktree-untracked
candidate into main also gitignores it there – the secret-safety gate, since
only an untracked file is treated as a secret needing that protection.
Pushing a tracked candidate skips that gitignore step: it lands as an
ordinary working-tree copy in main, recoverable through the pre-write backup
and git.

### Main-only candidate
A candidate present in main but absent from the worktree. Pulling it creates
the file locally; deleting it removes main's copy; push is unavailable,
since there is no worktree copy to push.
