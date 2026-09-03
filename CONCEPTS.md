# Concepts

Shared domain vocabulary for this project — entities, named processes, and
status concepts with project-specific meaning. Seeded with core domain
vocabulary, then accretes as ce-compound and ce-compound-refresh process
learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Sync model

### Worktree
A linked git checkout that shares its repository's history with the main
checkout but has its own working tree, so anything that never enters git –
secrets, local overlays, generated state – is absent from it until something
copies it there. Superset creates one per workspace; forward sync, reverse
sync and the merge cockpit exist to move exactly those non-git files between
a worktree and the main checkout, and which side is source or target is
decided by where the command is run.

### Workspace contract
The directory a repository commits so Superset can create and tear down
workspaces for it: the workspace lifecycle configuration, the wrapper script
that runs ss-magic when a workspace is set up, the shared sync-pattern list,
and beside them the optional per-checkout local pattern list. It is the one
tree ss-magic owns inside a repository and is never itself excluded from
sync or pack – only the backup and plugin-state subtrees inside it are.

ss-magic writes it in two ways. Initialization lays it out fresh; migration
converts a repository that predates the current layout, carrying the
previously configured file list forward. Both stage the whole tree and
materialize it in one step behind a finishing prompt, so an aborted run
leaves the previous contract intact.

### Main checkout
The primary git checkout that linked worktrees branch from and share a common
git directory with — the canonical tree reverse sync writes back into and the
source forward sync copies from.

### Sync patterns
The glob patterns that drive both forward and reverse sync, formed by overlaying
a committed, shared pattern list with an optional per-checkout local list (union,
de-duplicated). They select which local or untracked files cross between the main
checkout and a worktree. The local list is itself gitignored and is itself a
forward-sync target, so it travels from the main checkout into a worktree like
any other local file – which is why a setting that must not be overridable per
worktree, such as whether the plugin is enabled, is always read from the main
checkout's copy.

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
the direct `ss-magic reverse-sync` subcommand – one directory per apply batch,
named by the batch's timestamp and keeping the copies from each side apart, and
are never committed. Taking backups is
opt-out (`--no-backup`/`-n` on the direct subcommands) and, when skipped,
leaves an overwritten or deleted file with no recovery path. A bounded number of the
newest batches is kept; older ones are pruned after each apply.

### Review baseline
The per-candidate snapshot of both sides' file metadata taken the moment the
merge cockpit opens, and compared against the disk again just before each
reconcile decision is applied, so a file that changed while the developer
was reviewing is skipped rather than overwritten with bytes the review never
saw.

The baseline is coherent with what was reviewed, not with the disk: a side
the review showed as absent is recorded as absent even if a copy appears
later, and a side that could not be read is recorded as absent too. Both
fail closed – an absent baseline against a present file counts as changed
and the file is skipped; only a genuinely absent target is ever written. The
direct non-interactive push has no review window, so its baseline is taken
immediately before the apply and guards only against a concurrent writer.

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
file's repo-relative path. The `<repo>` stem is derived from the
repository's origin remote, so the same repository yields the same archive name
from any clone URL form, falling back to the primary worktree's basename when no
origin exists. A third operation on the
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
pushed. Pushing a worktree-untracked
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

### Hook
An entry point the Claude Code harness runs at a session lifecycle event –
session start, before a tool call, before a context compaction, when a
subagent stops, and at session end – handing it an envelope on standard
input and reading a JSON answer on standard output. Everything the plugin
does inside a session happens through one.

A hook fails open: an error, a panic, a timeout, or a binary that is not
installed yet must all look exactly like a hook that decided to do nothing,
and it prints nothing but its answer, since a session-start hook's output
enters the model's context. A hook can never grant a capability – it may
deny a tool call or add context, never allow or rewrite one – and nothing
reachable from a hook can switch the plugin on for a repository.

### Human verb
A named plugin command a skill or the person's own request drives – status,
checklist, enable, config and the rest – as opposed to a hook. It reports
problems on standard error with a non-zero exit like any command-line tool.
Nobody is expected to type one at a shell prompt: the plugin's binary is
installed under the harness's plugin data directory and deliberately kept off
the user's `PATH`, so a verb is reached from inside a session, where a wrapper
of the binary's own name sits on the tool's `PATH`.

Only a human verb may set the per-repository enablement switch, which is what
keeps a repository from switching the plugin on by arranging for a hook to
fire. The rule is about that key specifically, not about configuration in
general: the bootstrap does invoke one verb that writes configuration – the
seed below – and that verb has no code path to the enablement key at all.

### Configuration seed
The one-time write the bootstrap makes after installing the binary: it folds a
block of the gate's default settings into an *existing* workspace contract
file, so the knobs are visible and editable in a file the repository already
tracks. It exists because there is no terminal path to the configuration verbs
at all, and documenting a command nobody can type would be worse than
surfacing the settings where a person is already looking.

It is bounded in six ways, each of them a test rather than a convention: it
never writes the enablement key, never stages the change, writes only when
there is no configuration block at all, never creates the file, never writes
through a symlink that leaves the repository, and preserves every other key in
the file. Turning the plugin on is therefore always a deliberate human edit to
the seeded block.

### Bootstrap
The session-start step that fetches the release binary named by the version
pin into the plugin's data directory, verifies it against the release's
published checksum, and installs it – doing nothing when the installed
binary already matches. It never fails a session: offline, a bad download,
an unwritable directory, or an unsupported platform all end in silence, and
a failed install never touches an existing binary.

Because hooks on one event run concurrently, the first session after
installing is already underway before the download lands, so every other
hook is deliberately inert for that session – each is launched through a
small shim that exits silently when the binary is not there yet, rather than
naming a path that does not exist. Reloading plugins re-registers the plugin
but emits no session-start event, so it does not run the bootstrap; a new
session does.

### Temp root
A private per-machine directory, derived from the user's home directory
alone, where the plugin's shell scripts and its binary coordinate before any
repository or session context exists: the install lock that serializes
concurrent bootstraps, the locks the plugin's writers take, and the handoff
file. Each level of it must be a real directory owned by the current user
with owner-only permissions; a predictable path is not proof of ownership,
so anything else makes the root unusable rather than trusted.

The handoff file records where the bootstrap installed the binary. It exists
because the harness tells hook processes where the plugin's data directory
is but does not tell the Bash tool, so the wrapper a skill runs, and the
shim a hook runs through, both find the binary by reading the handoff rather
than by guessing.

### State tree
The gitignored directory inside the workspace contract where the plugin
keeps everything machine-local for one worktree: the session scratchpads,
the conclusion cache, the pending one-shot claims, and the pointer to the
active session. It is written only after git confirms the tree is ignored –
refusing both when git says no and when git cannot be asked – and it is one
of the excluded trees, so nothing in it is ever synced, packed, or re-
offered as a candidate.

### Session scratchpad
The per-worktree directory inside the state tree holding durable working state –
`STATUS.md`, `TASKS.md`, `DECISIONS.md`, `LEARNINGS.md`, `CONTEXT.md`,
`OPERATOR-CHECKLIST.md` and research artifacts – that survives a context
compaction because it lives on disk rather than in the window. Its
`OPERATOR-CHECKLIST.md` is the model's own running notes on operational steps and
is distinct from the Operator checklist below, which is committed repository
content managed only through the plugin's verbs. Its name is derived deterministically from the git repository and
branch, so the same worktree always resolves to the same directory. Working
state only: it is gitignored, never committed, and anything durable is promoted
into the repo.

### Read gate
The `PreToolUse` hook that denies a `Read` of a file over the size threshold
rather than letting its whole content enter the context window. `Read` is the
one tool the harness never spills to disk, so an unguarded large read is
re-read on every later request. The gate is advisory, not a security boundary:
a timeout, a malformed hook envelope, or a missing binary all leave the read
to proceed. It can only deny, never allow – every uncertain lookup falls through
to letting the read happen – and it has deliberate escape hatches: a bounded
window of the file, a subagent's own reads, non-text files, configured exemption
patterns, and a one-shot bypass claim.

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
claim is not exclusive under a real race, even though it looks like it. A claim
also carries an age limit: an expired bypass is still consumed but does not open
the gate, so a stale claim can never admit a read indefinitely.

### Release line
One of the two independent streams of releases this repository publishes: the
sync CLI's, and the plugin's. Each has its own tag shape, its own version
number, its own archives, and its own way of reaching an installed machine –
the CLI polls the release list and replaces itself, while the plugin is
replaced by the marketplace client. The two versions are deliberately never
equal, because the release tooling reads a tag without a package prefix as
"every releasable package sitting at that version", so equal numbers would let
one line's tag publish the other line's release.

### Version pin
The plugin's declared version, which fixes both the binary its hooks run and the
skills and Markdown shipped beside it. A `SessionStart` bootstrap installs the
pinned binary and does nothing when it already matches, so updating the plugin is
what updates the binary; no plugin invocation ever self-updates, because a
mid-session swap would leave the binary and the shipped assets describing
different behavior. When the binary actually running a hook does not match the
pin, the mismatch is reported to the operator at session start – never as
model-facing context – and the status verb names it as one reason the plugin may
be doing nothing.

### Heartbeat log
The append-only, machine-level record of every hook invocation – including
the ones that did nothing and the ones that failed – kept outside any
worktree so its rows survive that worktree's deletion. It is what lets the
plugin's status verb answer whether a hook is actually firing, since a hook
that fails open looks from the outside identical to one that had nothing to
do.

It is bounded: the newest rows are kept and older ones dropped once the file
grows past a trigger size, and a row stamped in the future by a clock jump
is kept rather than dropped.

### Artifact contract
A declaration, recorded before a subagent is dispatched, that the subagent
must produce a named file. When that agent stops, the pending declaration is
consumed and, if the file is missing or empty, the stop is blocked once with
an explanation; with nothing declared, nothing is ever blocked.

Blocking happens at most once per declaration because consuming the record
is itself the one-shot flag. A declaration left waiting too long is dropped
rather than enforced – blocking an unrelated agent hours later would be
worse than not enforcing. Independently of the block decision, the stopping
agent's own text is salvaged to disk, since a lost transcript is
irreversible while a block is retriable.

### Commit nudge
The advisory context a before-tool-call hook adds when the model is about to
commit, push, or open a pull request while an operator checklist in the
repository is untracked or has unstaged edits: a reminder to update the
checklist first. It never blocks the command and says so; it fires only when
git reports the checklist as stale, so a clean checklist produces nothing.

### Cost ledger
The append-only record of what each ended session cost, written once per
session id from that session's own transcript tree. It reads the harness's own
priced records where they exist and falls back to a versioned price table.
A relative signal for comparing branches, never an authoritative bill.

### Spill file
A tool result the harness itself judged too large, written whole to disk and
replaced in the model's context by a short envelope naming its path. Spill
files outlive the session but carry unguessable names and no index, so the
plugin's `spill-index` verb lists the ones belonging to the current worktree.
