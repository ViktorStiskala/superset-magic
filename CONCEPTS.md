# Concepts

Shared domain vocabulary for this project – entities, named processes, and
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
materialize it in one step. The interactive flows ask a finishing question
first, so an aborted run leaves the previous contract intact; the scripted
`ss-magic init [PATTERN...]` has no prompt, writes the layout straight away
and leaves the files uncommitted.

Initialization and migration here concern the workspace contract only. They
are not the plugin's Repository migration, which moves a branch's operator
checklist from hand-written Markdown to the typed document (see that term
under the Claude Code plugin).

### Main checkout
The primary git checkout that linked worktrees branch from and share a common
git directory with – the canonical tree reverse sync writes back into and the
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
Backups live under a gitignored `.superset/backups/` in the root each flow
designates – the worktree for the merge cockpit (which keeps main's losing
bytes there too, under the batch's `main/` side) and for forward sync, main for the direct
`ss-magic reverse-sync` subcommand – one directory per apply batch,
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
sync patterns alongside forward and reverse sync – a portable snapshot of the
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
is but does not tell the Bash tool, so the wrapper a skill runs finds the
binary by reading the handoff rather than by guessing. The shim a hook runs
through uses the harness's variable directly and falls back to the handoff only
when it is empty. When the variable is unset, the bootstrap falls back to the
harness's documented default location,
`${CLAUDE_CONFIG_DIR:-$HOME/.claude}/plugins/data/ss-magic-ss-magic`; the status
verb tries the handoff first and only then that location.

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
to letting the read happen – and it has deliberate escape hatches: anything
inside the plugin's own state tree (so scratchpad notes can always be re-read
after a compaction), a bounded window of the file, a subagent's own reads,
non-text files, configured exemption patterns, and a one-shot bypass claim.
The gate is also called the page-fault gate in the plugin's manifest and
configuration code.

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
ordered, validated, and renderable to Markdown that is byte-identical across
machines and timezones. One renderer serves both the plugin's `list` view and the
`render-md` output a CI job posts as the pull-request comment. `list` carries a
fixed byte budget; `render-md` is unbounded unless given `--max-bytes`, which
bounds the whole body, and the CI workflow always passes one so the comment
fits the forge's size limit.

`verify` and `render-md` take the active checklist by default, or an
explicit list of documents (`list` takes none). CI uses the explicit form: it renders and verifies
only the checklists the pull request added, modified or changed the type of
(a deletion selects nothing, a rename is selected at its new path), posting one
comment that holds each in turn, and does nothing when the pull request
touches none. An explicit path must be a checklist document directly under
`docs/actions/`, inside the repository and not a symlink; anything else is
refused rather than read.

### Repository migration
What the plugin's migrate-repository skill does for a repository that keeps a
hand-written Markdown checklist per branch: it preflights, asks whether to
enable the plugin, converts the *current branch's* checklist into the typed
Operator checklist document by driving the ordinary checklist verbs, sets up
the CI workflow, and retires the repository's old hand-written checklist skill
and rules. It converts one branch and one branch only – every earlier branch's
checklist, and any standing one, stays Markdown as history, and other open
branches run the skill themselves after merging. The skill decides and
confirms; the verbs write, so the format's rules are enforced by validation
rather than restated. It asks before each commit, before enabling, before
discarding an uncommitted document, before writing the workflow, and before
retiring the old skill and rules – the last as one consolidated diff, so
retirement is one decision rather than many.

This is not the sync model's initialization or migration, which converts a
repository's workspace contract from the old setup-script layout to the current
one. The two are distinct, and the order is fixed: the skill stops unless a
workspace contract (`.superset/magic.json`) already exists, so a repository
still on the old setup-script layout goes through the sync model's init or
migrate first.

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
one line's tag publish the other line's release. The plugin only advises about a
newer release of its own line (see Operator notice); it never downloads or
installs one.

### Version pin
The plugin's declared version, which fixes both the binary its hooks run and the
skills and Markdown shipped beside it. A `SessionStart` bootstrap installs the
pinned binary and does nothing when it already matches, so updating the plugin is
what updates the binary; no plugin invocation ever self-updates, because a
mid-session swap would leave the binary and the shipped assets describing
different behavior. When the binary actually running a hook does not match the
pin, the mismatch is reported to the operator at session start as one of the
operator notices below – never as model-facing context – and the status verb
names it as one reason the plugin may be doing nothing.

### Operator notice
A one-line message the session-start hook sends to the person rather than the
model, on the harness's operator channel. There are three. The version-drift
notice appears on every kind of session start when the running binary is not the
version the loaded plugin pins. The compaction advice appears once per machine,
on a fresh start only, when an auto-compaction override is set and no compaction
window is configured. The release suggestion appears once per release, on a
fresh start only, when the cached release list shows a newer plugin release than
the pin; the cache is refreshed in the background by a detached process, so
session start never waits on the network. The last two are suppressed in quiet
mode. The release suggestion is recorded before it is shown and is withheld when
it cannot be recorded; the compaction advice is recorded best-effort, so on a
machine whose cache directory refuses writes it can repeat.

### Quiet mode
The verdict that nobody is watching the session, so an operator notice would go
unread: the permission mode is one that never waits for a person (bypass or
don't-ask), or the harness was launched by an embedding other than the terminal
(an SDK or an IDE extension). Absent signals mean not quiet, deliberately, since
a wrongly shown line costs far less than a wrongly hidden one.

### Compaction window
The absolute token count at which the harness compacts a conversation, set in the
project's settings (the tracked file or the per-machine, gitignored local one;
the plugin writes only the local one). The plugin recommends one from
the peak context size of the repository's recent sessions in the cost ledger
(1.25 times the largest peak, rounded up to a multiple of 10,000 and clamped
between 100,000 and 1,000,000 tokens), reports it read-only, and writes it only
when the person explicitly asks. It never edits the user's own settings.

### Session identity
The deterministic `<repo>-<branch>` slug that names a worktree's session
scratchpad directory. It comes from git alone – the origin remote (or a
directory-name fallback) and the current branch, or a detached-head marker – and
never from the Superset workspace name, which can be renamed silently. Outside a
git repository there is no identity and the plugin does nothing.

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
worse than not enforcing. Independently of the block decision, a stopping agent
that ended without a final message (absent or blank) has the text it did produce
recovered from its transcript to disk, since a lost transcript is irreversible
while a block is retriable. An agent that reported a result needs no salvage, and
the step is skipped on a re-entered stop, outside a repository, and when the
state tree refuses.

### Commit nudge
The advisory context a before-tool-call hook adds when the model is about to
commit, push, or open a pull request while an operator checklist is absent from
the commit: named by the active-checklist pointer but never written, or reported
by git as untracked or having unstaged edits. It is a fixed reminder to update
the checklist first, with no checklist content in it. It never blocks the
command and says so, and a clean checklist produces nothing.

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
