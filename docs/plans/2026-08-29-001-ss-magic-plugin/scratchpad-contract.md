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
dir      = <worktree-root>/.scratchpad/.ss-magic-plugin/sessions/<repo>-<branch>/
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
<worktree-root>/.scratchpad/
├── .gitignore                       *  / !.gitignore / !README.md
├── README.md                        what this tree is; the only committed content
└── .ss-magic-plugin/
    ├── current.json                 pointer to the active session dir (replaces the symlink)
    ├── conclusions/<key>.md         hash-keyed Explore conclusions for the Read gate
    └── sessions/<repo>-<branch>/
        ├── CONTEXT.md               context expensive to rediscover; topic-grouped, not a timeline
        ├── DECISIONS.md             settled decisions with provenance
        ├── LEARNINGS.md             append-only; `## <timestamp> - <label>` blocks, never edited
        ├── STATUS.md                newest block first; older blocks demoted to history, never pruned
        ├── TASKS.md                 the task list and current status
        └── research-<topic>/*.md    durable research artifacts
```

**No symlink.** `current.json` is a plain file:

```json
{ "slug": "superset-magic-ss-magic-plugin",
  "dir": ".scratchpad/.ss-magic-plugin/sessions/superset-magic-ss-magic-plugin",
  "repo": "superset-magic", "branch": "ss-magic-plugin",
  "resolved_at": "2026-08-29T07:00:00Z" }
```

ss-magic creates no symlink anywhere today — forward sync explicitly *skips* them (`sync/apply.rs:315`) and pack only classifies them no-follow. Introducing one would mean teaching three code paths a primitive they currently drop, or accepting that the registry vanishes from every sync and every archive. A regular file is copied, packed and diffed correctly by all of them.

### Machine-level state

`ledger.jsonl` and `offsets.json` are not part of the worktree tree above — they live machine-level, in ss-magic's existing `ProjectDirs` cache root, alongside the heartbeat (KTD7):

```plaintext
<ProjectDirs cache root>/plugin/
├── ledger.jsonl                     one append-only row per ended session, labeled with the resolved worktree root and branch
├── offsets.json                     {path -> (offset, inode, size)} rotation guard for the ledger
└── hooks.jsonl                      the heartbeat; one row per hook invocation, including the fail-open path (R50)
```

Per-worktree state (the scratchpad session files under `sessions/<repo>-<branch>/`, `current.json`, and the conclusion cache under `conclusions/`) lives inside `.scratchpad/.ss-magic-plugin/` and is deleted along with the worktree. Machine-level state (the ledger, the offsets store, and the heartbeat) lives in the `ProjectDirs` root instead, because R46 requires `cost` to keep reporting a worktree's sessions after that worktree is gone — state that must outlive the worktree cannot be stored inside it.

The conclusion cache is the one exception kept per-worktree on purpose: its keys fingerprint `(realpath, size, mtime)` (KTD3), and a realpath is worktree-specific, so a cache entry from one worktree would never hit in another.

## The state files, and why these ones

Grounded in a real, in-use scratchpad tree rather than invented. Observed there: **STATUS.md at 594 lines / 88.7 KB**, refreshed every 20–40 minutes by a `/loop` cron; LEARNINGS and LINEAR strictly append-only with `## <timestamp> - <label>` blocks that are never edited; CONTEXT.md headed *"Context that would be expensive to rediscover"* and grouped by topic rather than time.

**The file set is a floor, not a schema.** In the observed tree one sibling session had `DECISIONS.md` but no `LINEAR.md`; another had only `REPORT.md` and `findings/`. `ss-magic plugin scratchpad ensure` scaffolds the five files if absent and **never rewrites an existing one** — the model owns their content.

STATUS.md follows the demote-never-delete pattern verbatim, because it is what made the observed tree survivable across compactions:

```markdown
## CURRENT STATE - <timestamp> (read this block first; everything below is history)
...
## HISTORY - <timestamp> block (superseded, kept for the audit trail)
```

## The gitignore, and the landmine it arms

```plaintext
*
!.gitignore
!README.md
```

**This file is exactly what triggers the pre-existing rule-lifting bug.** With a broad reverse-sync pattern, `ensure_path_ignored` lifts the pattern text `*` out of this nested file and re-anchors it at the **main checkout's root** `.gitignore`, blinding git to the entire shared checkout — reproducible in three commands, covered by no test.

→ **The gitignore fix and the enumeration-layer exclusion are hard prerequisites**, sequenced before the plugin writes its first byte into `.scratchpad/`. See [architecture.md](./architecture.md#two-prerequisite-fixes-outside-srcplugin).

Gitignoring the tree is **not** sufficient protection on its own: ss-magic's untracked probe is `git ls-files --others` *without* `--exclude-standard` (deliberately — reverse sync must push gitignored secrets), and `walk_source` has no gitignore filter at all. `.scratchpad/` must be excluded at the **enumeration layer**, the way `.superset/backups/` already is.
