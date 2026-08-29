# Scratchpad contract — identity, layout, and the state files

Companion to [the plan](../2026-08-29-001-feat-ss-magic-claude-plugin-plan.md).

## Identity — deterministic, from git only

**The slug is `<repo>-<branch>`, derived entirely from git. It is never derived from the Superset workspace name.** This overturns a direction confirmed earlier in the session, on measurement.

Why the workspace name fails:

- `superset ws update --name <string>` exists — renaming is a one-command, zero-friction operation.
- The schema carries `is_unnamed integer DEFAULT false`, which only makes sense if `name` starts as a placeholder and is later changed.
- **31 of 36** live workspaces have `name != branch`; **five** are named `main`; names contain spaces, `+` and mixed case.
- `worktreePath` tracks the **branch**, never the name — workspace `"Invoices"` lives at `…/worktrees/<projectId>/computer-control-invoices`.

A name-derived directory silently orphans the whole scratchpad on a rename, with no error and no migration signal. Pairing the name with the immutable `createdAt` does not rescue it — the *mutable* half is the name half, and it additionally breaks on workspace re-create.

### The derivation

```plaintext
<repo>   = pack::archive_file_name's origin-derived name (already shipped: scheme/userinfo/host
           stripped, segments sanitized, joined with '_'), falling back to
           basename(git::main_checkout_root)
<branch> = git symbolic-ref --quiet --short HEAD, '/' -> '-', unsafe chars -> '-',
           runs collapsed, leading/trailing '-' trimmed, lowercased, truncated to 40
dir      = <worktree-root>/.superset/.magic/sessions/<repo>-<branch>/
```

Four traps, each measured:

- **Do not use `basename(git rev-parse --git-common-dir)`.** It is cwd-relative and returns `.git` or `../.git` in any ordinary checkout — absolute only inside a linked worktree — so outside Superset every repo would slug to `.git-<branch>`.
- **Do not use `git rev-parse --abbrev-ref HEAD`.** It returns the literal string `HEAD` under detached HEAD. `symbolic-ref` exits 1 instead, which is a usable signal: fall back to `detached-<short-sha>`. Outside a repo (exit 128) the plugin does nothing at all.
- **Do not append a path hash.** In the `worktrees/<projectId>/<branch>` layout it is merely a function of (project, branch), adds no independent identity, and breaks on directory rename. It is unnecessary anyway — the scratchpad lives *inside* the worktree and a worktree has exactly one branch, so the slug only has to be unique within one directory.
- **Sanitizing `/` is load-bearing.** A cross-repo PR workspace produces `localBranchName = <forkOwner>/<headRefName>`.

**Implement slugify in Rust, not shell.** The spike's shell version used `sed -e 's/[^a-z0-9]\+/-/g'`, which **silently no-ops on BSD `sed`** (macOS default): `"Magic plugin"` kept its space and produced an invalid directory name, with no error. Two further cases the Rust implementation must handle, both found by running it: a name of `"---"` slugs to the **empty string**, and `"Ünïcödé Nàme"` mangles to `n-c-d-n-me`. Guard the empty result by falling through to the next identity source; NFD-decompose-and-strip for the accents.

**Superset is read for display only, never for identity.** `superset ws get` costs 0.73–0.98 s warm and 1.19 s cold versus **5–8 ms** for the whole git probe, which alone disqualifies it from any hook path. If a human-readable name is wanted for a printed line, read `~/.superset/host/<orgId>/host.db` (~4 ms) as a best-effort fast path — it is an undocumented internal schema, so any failure means "no name".

Because identity never touches Superset, **there is one code path inside and outside a Superset workspace** — nothing to test for and nothing to fall back from.

### Which directory the hook resolves against

Read the **`cwd` field from the hook's stdin JSON**. Not `${CLAUDE_PROJECT_DIR}` — documented and directly load-bearing: *"`${CLAUDE_PROJECT_DIR}` stays put … `cwd` follows Claude"* when Claude enters a worktree. Not the process cwd either. Feed that `cwd` into `git::is_worktree` / `git::main_checkout_root`, not `resolve_sync_roots`' implicit cwd.

## Layout

```plaintext
<worktree-root>/.superset/.magic/
├── README.md                    what this tree is; the only committed content
├── current.json                 pointer to the active session dir (replaces the symlink)
├── checklist.json               pointer to the active operator checklist (R89); the checklist
│                                ITSELF is committed repo content at docs/actions/, never here
├── conclusions/<key>.md         hash-keyed Explore conclusions for the Read gate
├── bypass/<key>                 one-shot Read-gate bypass token per file (R42, KTD10); the gate consumes it on the next matching over-threshold Read
├── expect-artifact/<key>        a dispatching agent's pending subagent-output-file declaration (R51); consumed by SubagentStop
└── sessions/<repo>-<branch>/
    ├── CONTEXT.md               context expensive to rediscover; topic-grouped, not a timeline
    ├── DECISIONS.md             settled decisions with provenance
    ├── LEARNINGS.md             append-only; `## <timestamp> - <label>` blocks, never edited
    ├── OPERATOR-CHECKLIST.md    the model's own running notes on operational steps (R17) – NOT
    │                            the R82 checklist, which is committed repo content; see below
    ├── STATUS.md                newest block first; older blocks demoted to history, never pruned
    ├── TASKS.md                 the task list and current status
    └── research-<topic>/*.md    durable research artifacts
```

The sync / reverse-sync / pack exclusion for this tree matches the exact two-component path `.superset/.magic` – it must never widen to `.superset` itself, which would also swallow the contract files ss-magic already writes there (`config.json`, `magic.sh`, `magic.json`).

**No symlink.** `current.json` is a plain file:

```json
{ "slug": "superset-magic-ss-magic-plugin",
  "dir": ".superset/.magic/sessions/superset-magic-ss-magic-plugin",
  "repo": "superset-magic", "branch": "ss-magic-plugin",
  "resolved_at": "2026-08-29T07:00:00Z" }
```

ss-magic creates no symlink anywhere today — forward sync explicitly *skips* them (`sync/apply.rs:315`) and pack only classifies them no-follow. Introducing one would mean teaching three code paths a primitive they currently drop, or accepting that the registry vanishes from every sync and every archive. A regular file is copied, packed and diffed correctly by all of them.

### The checklist pointer – `checklist.json`, and nothing in `.scratchpad/`

`ss-magic plugin checklist init` records the active operator checklist at **`.superset/.magic/checklist.json`** – inside the single state root this document defines, alongside `current.json`. **Nothing is written into `.scratchpad/`.** That tree belongs to other tooling, ss-magic does not own it, and R2's enumeration exclusion keeps it out of sync and pack precisely because it is not ours to manage.

**Two different things share the word "checklist", and they must not be confused.**
`sessions/<repo>-<branch>/OPERATOR-CHECKLIST.md` is scratchpad state the *model* owns and freely edits (R17), scaffolded empty and never rewritten by ss-magic. The **operator checklist** of R82-R90 is committed **repository** content at `docs/actions/<YYYY-MM-slug>.checklist.json`, written only through the `checklist` verbs and denied to `Read`/`Edit`/`Write` by R88. `checklist.json` in the state root is the pointer to the second of those, and is what the `SessionStart` injection (R19) names.

The same preference as `current.json` applies, for the same reasons: **a manifest file wherever a plain file suffices**, because every existing code path already handles a regular file correctly. The pointer records the intended checklist path (and does so whether or not that file exists yet), so `checklist init` on a branch with no checklist is a complete operation rather than a half-state.

Where a symlink genuinely is required, three rules bind it:

- It is **the one symlink ss-magic creates**, anywhere. That is a deliberate, countable exception, not a precedent.
- It is created **only after the containment and ignored-tree checks pass on the resolved parent** – R56's "stays inside the worktree, never follows a symlink out of it" and R63's "git reports `.superset/.magic/` ignored". Resolving the parent first is what stops a pre-planted symlink from turning a containment check into a write outside the tree.
- **A dangling pointer is classified as a checklist path** by the R88 gate, not followed through to a stat. A pointer whose target does not exist is still a checklist reference, so the read is denied with the "use the `ss-magic plugin checklist` verb" text rather than falling through the gate's size machinery and surfacing as a bare missing-file error. See [page-fault.md](./page-fault.md#why-the-checklist-deny-precedes-every-exemption).

### Machine-level state

`ledger.jsonl` and `offsets.json` are not part of the worktree tree above — they live machine-level, in ss-magic's existing `ProjectDirs` cache root, alongside the heartbeat (KTD7):

```plaintext
<ProjectDirs cache root>/plugin/
├── ledger.jsonl                     one append-only row per ended session, labeled with the resolved worktree root and branch
├── offsets.json                     {path -> (offset, inode, size)} rotation guard for the ledger
└── hooks.jsonl                      the heartbeat; one row per hook invocation, including the fail-open path (R50)
```

Per-worktree state (the scratchpad session files under `sessions/<repo>-<branch>/`, `current.json`, the conclusion cache under `conclusions/`, and the one-shot `bypass/` and `expect-artifact/` claims) lives inside `.superset/.magic/` and is deleted along with the worktree. Machine-level state (the ledger, the offsets store, and the heartbeat) lives in the `ProjectDirs` root instead, because R46 requires `cost` to keep reporting a worktree's sessions after that worktree is gone — state that must outlive the worktree cannot be stored inside it.

The conclusion cache is the one exception kept per-worktree on purpose: its keys fingerprint `(realpath, size, mtime)` (KTD3), and a realpath is worktree-specific, so a cache entry from one worktree would never hit in another.

### Volatile coordination state – the temporary root

There is a third tier, and it holds **nothing durable**. Lock files and anything else that guards a race live under a private per-machine temporary root (R80):

```plaintext
/tmp/ss-magic-plugin/<frozen-identifier>/          preferred
$TMPDIR/ss-magic-plugin/<frozen-identifier>/       fallback where /tmp cannot host a
                                                   writable private root – e.g. a sandbox
                                                   that allowlists only $TMPDIR
```

Four properties, each load-bearing:

- **Created owner-only.** A world-writable lock directory is a lock anyone can steal.
- **The identifier is stable for the machine and encodes no repository path.** Stable, because two sessions in *different* worktrees still contend for the one thing the lock protects – the single pinned binary under `${CLAUDE_PLUGIN_DATA}`, which is machine-global. Path-free, because a temporary root is world-readable metadata by nature and the name of a private repository is not something to publish in it.
- **Nothing durable is kept there.** Anything that must survive a reboot belongs in the `ProjectDirs` root above; anything scoped to the worktree belongs in `.superset/.magic/`. A cleared `/tmp` must cost nothing but a re-taken lock.
- **It is outside the worktree, deliberately.** This is what lets two hooks on one event coordinate – notably the synchronous bootstrap and any asynchronous sibling – without either writing into a repository, and without depending on `.superset/.magic/` already existing or already being ignored (R63 forbids a hook writing there until git reports the tree ignored, which is exactly the moment a bootstrap lock is needed).

This is the mechanism the hook contract's second concurrency rule names: ss-magic's own concurrent handlers coordinate through this lock, **never** through ordering assumptions, because handlers on one event fire concurrently and completion order is not stable (see [hook-contract.md](./hook-contract.md#concurrency--handlers-do-not-chain-and-rewrites-are-folded-last-write-wins)).

## The state files, and why these ones

Grounded in a real, in-use scratchpad tree rather than invented. Observed there: **STATUS.md at 594 lines / 88.7 KB**, refreshed every 20–40 minutes by a `/loop` cron; LEARNINGS and LINEAR strictly append-only with `## <timestamp> - <label>` blocks that are never edited; CONTEXT.md headed *"Context that would be expensive to rediscover"* and grouped by topic rather than time.

**The file set is a floor, not a schema.** In the observed tree one sibling session had `DECISIONS.md` but no `LINEAR.md`; another had only `REPORT.md` and `findings/`. `ss-magic plugin scratchpad ensure` scaffolds the six files (including `OPERATOR-CHECKLIST.md`, empty – the model's own notes, not the R82 checklist) if absent and **never rewrites an existing one** — the model owns their content.

STATUS.md follows the demote-never-delete pattern verbatim, because it is what made the observed tree survivable across compactions:

```markdown
## CURRENT STATE - <timestamp> (read this block first; everything below is history)
...
## HISTORY - <timestamp> block (superseded, kept for the audit trail)
```

## The gitignore rule, and the landmine it no longer arms

The tree is ignored by a single `.superset/.magic/` `Dir` rule, written through `gitignore::ensure_path_ignored` exactly like the existing `.superset/backups/` rule. It lands in the closest EXISTING `.gitignore` among the path's ancestors – the repository root in the ordinary case, but `.superset/.gitignore` where a repo already carries one. The contract is that git reports the tree ignored, not that any particular file was edited.

**Who writes it (R40): only explicit `ss-magic` invocations.** `ensure_bootstrap_gitignores` covers it eagerly on `init`/`migrate`, the same eager pairing `reverse_sync::ensure_backups_ignored` already uses for the backups tree. **No hook verb ever writes a `.gitignore`.**

Note what the 2026-08-30 delivery change did to the *lazy* half: with the marketplace as the only delivery path, `ss-magic sync` runs no plugin step at all (R66), so there is no sync-time writer left to add the rule on first use. Only `init`, `migrate` and an explicit `ss-magic plugin` verb that is not a hook can write it. A repository that flips `plugin.enabled` without re-running one of those therefore has no ignore rule, which is precisely the case the fail-closed check below covers.

**What replaces the old design's create-and-ignore atomicity (R63).** A hook verb writes NO state while git does not report `.superset/.magic/` ignored – it records the refusal and its reason in its heartbeat row instead. The old nested `*` gitignore made the tree ignored the instant it existed, as a side effect of its own creation; with the rule owned elsewhere and written only by `ss-magic`, that instant-ignored property no longer falls out for free, so the fail-closed check is what restores it.

Because this plan no longer creates a nested `*` gitignore, it no longer arms the covering-rule re-anchor defect: with the rule already anchored at a repo root (or `.superset/.gitignore`), `ensure_path_ignored` lifts a root rule to a root rule – nothing moves, nothing blinds. That defect (fixed by R1/U1) stays worth fixing on its own merits – `.scratchpad/.gitignore` containing `*` already exists in Superset worktrees, written by the planning tooling – but it is now an independent pre-existing fix, not something this plan arms, and U8 no longer depends on U1. See [architecture.md](./architecture.md#two-prerequisite-fixes-outside-srcplugin) for where that fix lives.
