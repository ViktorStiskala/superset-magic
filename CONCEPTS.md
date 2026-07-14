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
candidates. Each candidate is shown to the user for confirmation before anything
is written into the main checkout.
