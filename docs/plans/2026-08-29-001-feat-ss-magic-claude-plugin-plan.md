---
title: ss-magic Claude Code Plugin - Plan
type: feat
date: 2026-08-29
topic: ss-magic-claude-plugin
artifact_contract: ce-unified-plan/v1
artifact_readiness: requirements-only
product_contract_source: ce-brainstorm
execution: code
---

# ss-magic Claude Code Plugin - Plan

## Goal Capsule

**Objective.** Make `ss-magic` ship and install a Claude Code plugin named `ss-magic` that carries
a durable per-worktree session scratchpad, a context page-fault gate on `Read`, a cost ledger, and
subagent artifact enforcement — installed and refreshed from `ss-magic sync` whenever
`magic.json` sets `plugin.enabled`.

**Product authority.** The requirements below are settled. Every design fork was closed by
measurement against Claude Code 2.1.251 and the `ss-magic` crate at v0.9.0; the evidence is in
[validation-evidence.md](./2026-08-29-001-ss-magic-plugin/validation-evidence.md). Six pieces of
the original request were rewritten or dropped on that evidence, each with a named replacement —
see [Key Decisions](#key-decisions).

**Open blockers.** None. Every question is ruled.

## Companion documents

The plan stays standalone-readable; these carry the implementation detail it would otherwise bury.
Each is in scope for review alongside this file.

- [architecture.md](./2026-08-29-001-ss-magic-plugin/architecture.md) — the full `src/plugin/`
  tree, module dependency rules, the three helper extractions that stop the plugin restating logic
  ss-magic already has, and the two prerequisite fixes that live outside `src/plugin/`.
- [hook-contract.md](./2026-08-29-001-ss-magic-plugin/hook-contract.md) — every shipped hook event
  with its channel, the measured 10,000-character cliff, the uncapped deny channel, the concurrent
  last-write-wins rewrite race, and the two validation tiers with opposite fail postures.
- [page-fault.md](./2026-08-29-001-ss-magic-plugin/page-fault.md) — what the harness already does
  to large output, why the Bash half is dropped, and the deny-with-inline-conclusion mechanism
  with its cache-key rules.
- [scratchpad-contract.md](./2026-08-29-001-ss-magic-plugin/scratchpad-contract.md) — identity
  derivation and its traps, the directory layout, the state files, and why the gitignore inside it
  arms a pre-existing bug.
- [plugin-assets.md](./2026-08-29-001-ss-magic-plugin/plugin-assets.md) — `plugin.json`,
  `hooks.json` and the installed layout verbatim, plus install verification.
- [skills.md](./2026-08-29-001-ss-magic-plugin/skills.md) — the three shipped skills, their
  frontmatter and body, and their invocation names.
- [cost-ledger.md](./2026-08-29-001-ss-magic-plugin/cost-ledger.md) — measured scan feasibility,
  attribution rules, and why the harness's own priced records come before any price table.

## Product Contract

### Summary

`ss-magic` gains a `plugin` block in `magic.json` and a `ss-magic plugin <verb>` subcommand family.
When enabled, `ss-magic sync` installs a Claude Code plugin into `~/.claude/skills/ss-magic/` and
keeps it current. The plugin is pure JSON and Markdown; every hook calls the `ss-magic` binary by
name, so behavior versions with the tool's existing self-update. It gives each worktree a durable
scratchpad that survives compaction, blocks oversized `Read` calls and answers them from a cached
conclusion instead, records per-session cost, and stops a subagent from exiting without its
contracted artifact.

### Problem Frame

Agent sessions lose work to autocompaction, and the loss is invisible until something has to be
redone. Two measurements frame the cost. In one 34-hour session, cost tracked
`requests × steady-state context` at ~440K tokens re-read per request, and 10.5% of tool results
carried 69.5% of all tool-result text. In this session, 8.5% of results carried 52.4% — the same
shape, a different magnitude.

Claude Code already solves most of the raw-output half: a generic persistence layer turns 200 KB
of command output into 2,302 characters and a file path. What it does not solve is `Read`, which
never spills at all — an 8,000-line read cost 60,066 cache-creation tokens against ~6,600 for any
spilled Bash output. Nor does it solve continuity: when the window is cleared, nothing authored
survives unless something wrote it down, and the 1,303 spill files already on this machine sit in
92 directories under unguessable names with no index.

ss-magic already owns the per-worktree contract, already runs on every worktree, and already
self-updates. It is the natural carrier.

### Key Decisions

- **Install to personal scope, never project scope.** A project-scope plugin is gated on
  `hasTrustDialogAccepted`, and every Superset worktree is a new realpath — untrusted by
  construction, which is exactly ss-magic's domain. *Governs R10, R11.*
- **The plugin is a manifest; the binary is the behavior.** Hooks call `ss-magic` by bare name on
  PATH, so nothing is vendored and hook behavior rides the existing self-update. *Governs R12, R13.*
- **Drop the Bash page-fault half entirely.** The harness already spills every tool's output to a
  named file with a size label and preview, `BASH_MAX_OUTPUT_LENGTH` provably cannot raise the
  30,000-char literal, and a rewrite would race the user's live rtk hook. Replaced by a read-only
  spill manifest. *Governs R20, R25.*
  (session-settled: user-directed — the user's own ideation named this as idea 3; measurement showed
  the mechanism already exists, so the manifest is what survives.)
- **The gate denies; it never rewrites.** `updatedInput` works but the transcript keeps the original
  tool call, so the model is never told its input changed — and rewrites race last-write-wins.
  `permissionDecisionReason` is uncapped, so the cached conclusion rides inline instead.
  *Governs R21, R22, R23.*
  (session-settled: user-directed — chosen over the brief's "deny and tell the model to range-read
  or grep": that leaves the model to re-derive, and its own note flagged the looping risk.)
- **Route denied reads to an Explore agent and cache the conclusion by file identity.** The saving
  is not the agent's tokens, it is every later request that never re-reads the payload.
  *Governs R21, R24.*
  (session-settled: user-directed.)
- **Key the scratchpad on `<repo>-<branch>` from git, never on the Superset workspace name.** This
  overturns a direction confirmed earlier in the session: the name is user-mutable, 31 of 36 live
  workspaces have `name != branch`, and five are named `main`, so a rename would silently orphan the
  scratchpad. *Governs R14, R15.*
- **No symlink registry.** ss-magic creates no symlink anywhere today — forward sync skips them and
  pack only classifies them — so a plain `current.json` pointer is used instead. *Governs R16.*
- **Fix two pre-existing defects first.** Reverse sync can append `*` to the main checkout's root
  `.gitignore`, and the sync/pack enumeration layer is gitignore-blind. The plugin's own state lives
  under `.scratchpad/`, so shipping without these turns a private scratchpad into a leak surface.
  *Governs R1, R2, R3.*
- **Every hook is advisory, never a security gate.** Three independent fail-open paths: a missing
  binary, a timed-out `PreToolUse` hook, and an envelope-level typo. The real secret boundary stays
  in the sync engine. *Governs R26.*
- **Ship the cost ledger on `SessionEnd`.** The "1.5 s rules out a full scan" premise was wrong by
  a factor of three — the worst session tree in a 2.61 GiB corpus scans in 0.87 s. *Governs R27, R28.*
- **Tune the compaction window with `autoCompactWindow`, not `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`.**
  The env var is a percent bound to a field named `testPctOverride`, absent from `/autocompact` and
  `/config`, and can only lower the window. *Governs R30, R31.*
- **Effort-tiering stays out of scope.** ~20% of the measured bill and config-only, but it is
  guidance rather than mechanism.
  (session-settled: user-directed — offered and declined.)

### Requirements

**Prerequisites — these land before the plugin writes a byte**

- R1. `ensure_path_ignored` must not re-anchor a covering rule at a broader scope than it had in
  the source tree; reuse a pattern only when the target has a `.gitignore` at the same relative
  directory, else fall through to an anchored literal.
- R2. The sync engine's enumeration layer must exclude `.superset/backups`, `.scratchpad` and
  `.git` as whole trees, in `walk_source` and in `pack`, including when a directory match is an
  ancestor of an excluded subtree.
- R3. `pack` must report the number of unique paths it archived, not the number of tar entries.

**Configuration**

- R4. `MagicConfig` must round-trip unknown top-level keys through every write path, so `init`,
  `migrate` and the edit-config menu no longer delete configuration they do not understand.
- R5. `magic.json` accepts a `plugin` object; `plugin.enabled` defaults to false when the key is
  absent.
- R6. For every non-`files` key, `magic.local.json` overrides the base value whole; an absent key
  inherits, and an explicit null means off.
- R7. Enabling or disabling the plugin per machine is done in the main checkout's
  `magic.local.json`, because a worktree's overlay is itself forward-synced.

**CLI surface**

- R8. `ss-magic plugin <verb>` provides the hook verbs `session-start`, `pre-tool-use`,
  `pre-compact`, `subagent-stop`, `session-end`, and the human verbs `install`, `uninstall`,
  `status`, `cost`, `spill-index`.
- R9. `ss-magic plugin` never runs the auto-update gate, never opens the TUI, and on any internal
  error prints nothing and exits 0.

**Packaging and install**

- R10. The plugin installs to the user's `~/.claude/skills/ss-magic/`, resolved from the home
  directory and honouring `CLAUDE_CONFIG_DIR`.
- R11. Install verifies itself against the harness's own plugin listing and surfaces any reported
  errors and notes verbatim, ignoring the listing's exit code.
- R12. The installed tree contains only JSON and Markdown; hooks invoke `ss-magic` by bare name.
- R13. Install is content-addressed: identical bytes write nothing, and changed bytes print one
  notice naming the reload command.

**Session scratchpad**

- R14. The scratchpad directory name is derived from the git repository and branch alone, and is
  stable across sessions, days, and workspace renames.
- R15. A branch name that cannot be resolved falls back to a detached-HEAD form; outside a git
  repository the plugin does nothing.
- R16. The active session is recorded in a plain JSON pointer file, not a symlink.
- R17. `session-start` scaffolds any missing state file and never rewrites one that exists.
- R18. The scratchpad tree is gitignored, and its contents are never committed.
- R19. `SessionStart` injects operating guidance and the checklist pointer, staying within the
  channel's 10,000-character limit.

**The Read gate**

- R20. `ss-magic` ships no hook on `PreToolUse[Bash]` and emits no tool-input rewrite on any event.
- R21. A `Read` whose target exceeds the configured size is denied, and the denial names the cache
  path and instructs the model to route the work to an Explore agent.
- R22. When a conclusion exists for that file, the denial carries the conclusion inline, verbatim.
- R23. The inline conclusion is bounded by ss-magic's own byte budget, because the channel imposes
  none.
- R24. The cache key is derived from the file's identity, not from the read's offset or limit.
- R25. `ss-magic plugin spill-index` lists the harness's own spill files for the current worktree,
  read-only.
- R26. Every hook fails open: on timeout, malformed output, or a missing binary the session
  proceeds unchanged, and the plan documents the gate as a context measure rather than a boundary.

**Cost ledger**

- R27. `SessionEnd` appends one idempotent row per session id, scanning that session's own
  transcript tree, within the default hook timeout.
- R28. Cost is read from the harness's own priced records where present, falling back to a
  versioned price table snapshotted at ingest.
- R29. `ss-magic plugin cost` reports the ledger and can backfill a session whose `SessionEnd`
  never ran.

**Compaction window**

- R30. On explicit opt-in only, ss-magic writes an absolute auto-compact window into the
  repository's local, gitignored settings file, and adds the ignore rule in the same step.
- R31. ss-magic never overwrites a window the user already set, and never writes to the
  git-tracked settings file.

**Subagent artifacts**

- R32. `SubagentStop` blocks a stop at most once when the subagent's contracted output file is
  missing or empty, and the block names the file.
- R33. When a subagent's transcript ends with no reported result, its transcript is salvaged into a
  file marked as incomplete.

**Documentation and release**

- R34. `CLAUDE.md`, `README.md`, `CONTRIBUTING.md` and `.cursor/BUGBOT.md` describe the new
  behavior, and the crate version bumps a minor.

### Key Flows

- F1. **Enabling the plugin.** **Trigger:** a repo sets `plugin.enabled` and runs `ss-magic sync`.
  The plugin step runs after the configuration loads and before the empty-`files` early return, so
  a repo that syncs no files still gets the plugin. It renders the tree, compares to disk, writes
  only on change, verifies the install, and prints a reload notice if bytes changed. *Covers R5,
  R10, R11, R13.*
- F2. **A session starts.** **Trigger:** any of the five `SessionStart` sources. The hook resolves
  the slug from git, creates the session directory and pointer file, scaffolds missing state files,
  and injects guidance plus the checklist pointer. On the `compact` source this is what restores
  orientation after the window was cleared. *Covers R14, R16, R17, R19.*
- F3. **An oversized read is intercepted.** **Trigger:** the model calls `Read` on a file over the
  threshold. On a cache miss the call is denied with routing instructions; the model dispatches an
  Explore agent that reads the file in its own window and writes a conclusion. The model retries,
  and the second denial carries the conclusion inline. *Covers R21, R22, R24.*
- F4. **A session ends.** **Trigger:** `SessionEnd`. The hook walks that session's transcript tree,
  reads the harness's priced records where available, and appends one row keyed on session id.
  *Covers R27, R28.*

### Acceptance Examples

- AE1. `plugin.enabled` is true and `files` is empty. **Covers R5.** `ss-magic sync` still installs
  the plugin, then reports that there is nothing to sync.
- AE2. A `magic.json` written by a newer ss-magic contains a `plugin` block; the user runs
  `ss-magic init`. **Covers R4.** The `plugin` block survives the rewrite.
- AE3. Base `magic.json` sets `plugin.enabled` true; the main checkout's `magic.local.json` sets it
  false. **Covers R6, R7.** The plugin is not installed.
- AE4. The workspace is renamed in Superset. **Covers R14.** The scratchpad directory is unchanged.
- AE5. `HEAD` is detached. **Covers R15.** A detached-HEAD directory name is used and the session
  proceeds.
- AE6. The same `Read` is issued twice for a file with no cached conclusion and none is written in
  between. **Covers R21.** Both calls are denied with routing instructions; neither returns file
  content, and neither succeeds silently.
- AE7. A conclusion exists and the model re-issues the same `Read` with a different `limit`.
  **Covers R24.** The cached conclusion is still used.
- AE8. A cached conclusion exceeds ss-magic's byte budget. **Covers R23.** The denial carries a
  bounded excerpt and the conclusion's path rather than the whole file.
- AE9. The `ss-magic` binary is absent from PATH. **Covers R26.** Every hook is a no-op and the
  session behaves normally.
- AE10. The hook emits malformed JSON. **Covers R26.** The tool call proceeds unchanged.
- AE11. `SessionEnd` runs twice for one session id. **Covers R27.** The ledger holds one row.
- AE12. The CLI is killed and `SessionEnd` never runs. **Covers R29.** `ss-magic plugin cost`
  backfills the row from the transcript.
- AE13. A reverse sync matches a file under `.scratchpad/` whose nested `.gitignore` contains `*`.
  **Covers R1, R2.** The file is not pushed, and the main checkout's root `.gitignore` is unchanged.
- AE14. `pack` runs with a `**` pattern. **Covers R2, R3.** `.git/`, `.scratchpad/` and the backups
  tree are absent from the archive, and the reported count equals the number of unique paths.
- AE15. The user already set an auto-compact window. **Covers R31.** ss-magic leaves it alone.
- AE16. A subagent finishes without writing its contracted output file. **Covers R32.** Its stop is
  blocked once with the file named; if it stops again without the file, it is allowed to end.
- AE17. A subagent's transcript ends with no reported result. **Covers R33.** A salvage file is
  written and marked incomplete, and the parent reads that instead of re-running the agent.
- AE18. A pattern would match a path inside the scratchpad during forward sync. **Covers R18.** The
  scratchpad tree is skipped at enumeration, whatever the pattern's breadth.

### Success Criteria

- A session resumed after compaction can re-orient from the scratchpad alone, without re-reading
  the work that produced it.
- The ledger makes the Read gate's value measurable per workload rather than assumed — this
  session's own profile inverts the one that motivated the feature, so the threshold must be
  tunable against evidence rather than fixed.
- `cargo test` stays green, and the three prerequisite defects gain regression tests, none of which
  exist today.

### Scope Boundaries

**Deferred for later**

- Effort tiering, and any change to session or subagent effort settings.
- `PostToolUse` subagent cost attribution as a fast path.
- A `Stop` hook, gated on re-entry, if the SessionStart and SubagentStop pair proves insufficient.
- `Grep` and `Glob` gating as active behavior — the matcher ships, but neither tool exists in this
  environment, so nothing may depend on it firing.

**Outside this work**

- Injecting `/compact` into a terminal. Hooks have no controlling terminal, terminal-send submits
  by default, and the goal is met declaratively by the window setting.
- A `FileChanged` hook and any direnv workstream.
- Any hook acting as a security or policy gate.
- Exact billing reconciliation. The ledger is a relative signal, not an invoice.

### Dependencies / Assumptions

- Claude Code 2.1.251. Every measurement is against that build; the plugin loading path, hook
  channels, and spill thresholds are not contractual across versions, and `ss-magic plugin status`
  exists so drift is detectable.
- The harness's transcript JSONL is append-only. Confirmed empirically over one session, not
  documented — the ledger therefore keeps a rotation guard and can fall back to a full rescan.
- Transcript completeness at `SessionEnd` is measured for normal exit only; kill, crash and logout
  are untested, which is why rows must be idempotent and backfillable.
- Managed settings do not restrict the personal-scope plugin scan on the target machine. Where they
  do, the per-session plugin-directory flag is the documented fallback.

### Sources / Research

- [validation-evidence.md](./2026-08-29-001-ss-magic-plugin/validation-evidence.md) — the ruling
  record: 8 live probes, 86 adversarial refutations, and a final ruling pass, with commands and raw
  output for every claim.
- Existing insertion points: the forward-sync path and its pre-copy backup pass in `src/main.rs`,
  the configuration reader in `src/workspace/superset_files.rs`, the gitignore primitive in
  `src/git/gitignore.rs`, and the enumeration walk in `src/sync/apply.rs`.
- The koolman plugin plan and its reference spec, from which the packaging shape, the reserved
  machine-file directory, and the session-identity discipline port; its operator-checklist domain,
  Node runtime, and CI renderer do not.
