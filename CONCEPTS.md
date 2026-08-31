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

### Excluded trees
The four directory trees no ss-magic operation may ever enumerate, whatever the
sync patterns say: `.superset/backups` (pre-write backups, which hold recovered
secrets), `.superset/.magic` (the Claude Code plugin's machine-local state),
`.scratchpad` (a tree ss-magic does not own but must never push into the shared
main checkout), and `.git`. Each is matched as an exact path rather than a name
or a prefix, so `.superset` itself stays includable and the contract files still
travel. The exclusion is applied during each directory walk, not to the match
list, so a pattern that matches an *ancestor* of one of them cannot re-admit it.

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

## Claude Code plugin

### Session scratchpad
The per-worktree directory of durable working state – `STATUS.md`, `TASKS.md`,
`DECISIONS.md`, `LEARNINGS.md`, `CONTEXT.md` and research artifacts – that
survives a context compaction because it lives on disk rather than in the
window. Its name is derived deterministically from the git repository and
branch, so the same worktree always resolves to the same directory. Working
state only: it is gitignored, never committed, and anything durable is promoted
into the repo.

### Read gate
The `PreToolUse` hook that denies a `Read` of a file over the size threshold
rather than letting its whole content enter the context window. `Read` is the
one tool the harness never spills to disk, so an unguarded large read is
re-read on every later request. The gate is advisory, not a security boundary:
a timeout, a malformed hook envelope, or a missing binary all leave the read
to proceed.

### Conclusion cache
The store of Explore-agent answers about oversized files, keyed by a file's
identity rather than by the read's offset or limit. A first denied read routes
the work to an agent that reads the file in its own context and writes back a
conclusion; a later read of the same file is denied again, but that denial
carries the conclusion inline. This is what keeps the gate from blocking the
model permanently.

### Operator checklist
The committed record of the operational steps a change needs before it is safe
to ship – verification, rollout, decisions still open, and follow-ups that
outlive the code change – as one typed JSON document per action under
`docs/actions/`. The plugin's own verbs are its only write path; direct reads and
edits of the file are denied, which is what keeps every write canonically
ordered, validated, and renderable to byte-identical Markdown wherever it
appears (the CLI, a commit-time nudge, or a pull-request comment).

### One-shot claim
A record whose consumption is its own exactly-once flag, used for the bypass
token that admits a single gated read and for the artifact a subagent is
required to produce. Claiming renames the record onto a private name rather than
deleting it, so exactly one of several concurrent callers can win – a deleting
claim is not exclusive under a real race, even though it looks like it.

### Version pin
The plugin's declared version, which fixes both the binary its hooks run and the
skills and Markdown shipped beside it. A `SessionStart` bootstrap installs the
pinned binary and does nothing when it already matches, so updating the plugin is
what updates the binary; no plugin invocation ever self-updates, because a
mid-session swap would leave the binary and the shipped assets describing
different behavior.

### Cost ledger
The append-only record of what each ended session cost, written once per
session id from that session's own transcript tree. It reads the harness's own
priced records where they exist and falls back to a versioned price table.
A relative signal for comparing branches, never an authoritative bill.

### Spill file
A tool result the harness itself judged too large, written whole to disk and
replaced in the model's context by a short envelope naming its path. Spill
files outlive the session but carry unguessable names and no index, so
`ss-magic plugin spill-index` lists the ones belonging to the current worktree.
