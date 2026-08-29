# Skills — content and invocation

Companion to [the plan](../2026-08-29-001-feat-ss-magic-claude-plugin-plan.md).

Three skills ship inside the plugin. Because the manifest `name` is `ss-magic`, they are invoked `/ss-magic:scratchpad`, `/ss-magic:operator-checklist` and `/ss-magic:setup` — which is why the directories are **not** prefixed (`skills/scratchpad/`, not `skills/ss-scratchpad/`).

A skill is Markdown with YAML frontmatter; `SKILL.md` is always scanned inside a plugin's `skills/` directory. Editing one does **not** hot-reload — the whole plugin is snapshotted at session start (measured), so a changed skill needs `/reload-plugins` or a restart.

## `skills/scratchpad/SKILL.md`

The one the user invokes right after `/ce-brainstorm`, `/ce-plan`, `/ce-ideate`, `/ce-work` and similar. Its job is small and mechanical: resolve identity, scaffold state, then hand back a discipline for keeping it current.

```markdown
---
name: scratchpad
description: >
  Open or resume this worktree's durable session scratchpad and keep it current, so work survives
  autocompaction. Use immediately after starting a substantial task — a brainstorm, a plan, an
  implementation run — and whenever context is about to be lost. Not for one-off questions.
---

# Session scratchpad

Run `ss-magic plugin scratchpad ensure` first. It resolves this worktree's deterministic
slug, creates `.scratchpad/.ss-magic-plugin/sessions/<repo>-<branch>/`, scaffolds any missing state
file, and prints the resolved directory. It never rewrites a file that already exists — the content
is yours.

A dispatched agent that receives no injected context — an Explore subagent, for instance — can run
`ss-magic plugin status --json` instead, to recover the resolved slug, session directory, and gate
thresholds directly. When such an agent's findings are what a blocked `Read` should hand back, write
them with `ss-magic plugin conclude <original-path>` so the Read gate serves that conclusion in
place of the oversized file.

If the directory already has content, **read `STATUS.md` first** and continue from it rather than
starting over.

## The files, and what belongs in each

- **STATUS.md** — where the work stands *right now*. Newest block first under
  `## CURRENT STATE - <timestamp> (read this block first; everything below is history)`. When you
  update it, demote the previous block to `## HISTORY - <timestamp> (superseded, kept for the audit
  trail)`. Never delete history; never edit a demoted block.
- **TASKS.md** — the task list with status, including user follow-ups folded in as they arrive.
- **DECISIONS.md** — decisions that are settled, each with the alternative it beat and why.
- **LEARNINGS.md** — append-only. New entries as `## <timestamp> - <label>`. Never edit an old one.
- **CONTEXT.md** — context that would be expensive to rediscover, grouped by topic, not by time.
- **research-<topic>/** — durable artifacts worth keeping: extracted findings, measurements, notes.

## Keeping it current

Update the scratchpad when something lands, not on a timer: a decision settles, a task changes
state, a measurement comes back, the user redirects the work. Write the *conclusion*, not the
transcript.

Before a compaction and before ending a work session, reconcile: is STATUS.md's current block
actually current? Are any tasks silently abandoned? If something looks incomplete or hung, verify
its real state and resume it rather than assuming it finished.

Promote anything durable out of the scratchpad and into the repo — `docs/`, a plan, a solution
write-up. The scratchpad is working state, and it is never committed.
```

## `skills/operator-checklist/SKILL.md`

Initialised by the `SessionStart` hook's `additionalContext`, which is why the skill itself stays thin — it describes the discipline, and the hook supplies the live pointer.

```markdown
---
name: operator-checklist
description: >
  Track the operational steps a change needs before it is safe to ship — verification, rollout,
  and the follow-ups that outlive the code change. Use when work has real-world consequences
  beyond the diff.
---

# Operator checklist

The checklist lives in this worktree's scratchpad as `OPERATOR-CHECKLIST.md`. `ss-magic plugin scratchpad ensure` creates it empty on first run and the SessionStart hook points at it each session.

See [reference.md](./reference.md) for the item shape and the section conventions.

Keep it honest: an item is only checked when it has actually been done and verified, and an item
that turned out to be unnecessary is struck with a reason rather than deleted.
```

`reference.md` carries the item shape and section conventions. It is deliberately **generic** — the koolman original's four fixed sections (`pre-deploy`, `post-deploy`, `verification`, `visual`) and its GSC/release-approval domain do not transfer to a CLI tool and are dropped.

## `skills/setup/SKILL.md`

```markdown
---
name: setup
description: >
  Set up, verify, or repair the ss-magic plugin install and this repo's magic.json plugin block.
  Use when hooks are not firing, the scratchpad is not being created, or after upgrading ss-magic.
---

# ss-magic plugin setup

Diagnose first, then act. `ss-magic plugin status` reports where the plugin is installed, whether
the harness actually sees it, which hooks are registered, and what the slug resolves to here.

Common findings and what they mean:

- **Installed but not seen.** Run `claude plugin list --json` and read `errors[]` and `notes[]` —
  ignore the exit code, it is 0 even on total failure. A project-scope directory is suppressed
  unless the workspace has been trusted, which is why ss-magic installs to `~/.claude/skills/`.
- **Hooks registered but not firing.** Nothing hot-reloads; the plugin is snapshotted at session
  start. Run `/reload-plugins`, or restart the session.
- **`ss-magic` not on PATH.** The plugin is pure JSON and calls the binary by bare name. A missing
  binary is silently non-fatal — the hooks simply never run.
- **Plugin config disappeared from `magic.json`.** Older ss-magic versions dropped unknown keys on
  rewrite. Re-add the block and upgrade.

To enable, add to `.superset/magic.json`:

    { "plugin": { "enabled": true }, "files": [ … ] }

To opt out on one machine, set `{"plugin": {"enabled": false}}` in the **main checkout's**
`.superset/magic.local.json` — non-`files` keys are local-wins, whole-value. Setting it in a
worktree's overlay is clobbered by the next `ss-magic sync`, because that file is itself forward-synced.
`ss-magic plugin disable --local` does the same edit from the command line.

`disable` only flips that toggle — the installed tree at `~/.claude/skills/ss-magic/` stays in
place, so re-enabling costs nothing. `ss-magic plugin uninstall` is what actually removes it.
```

## What does not ship

The koolman original's `ss-operator-checklist` carried an entire e-commerce release domain — four fixed checklist sections, Google Search Console verification, release-approval instructions, a CI job rendering the checklist into a PR comment, and a `CHECKLIST.md` migration. None of it transfers: ss-magic has no per-branch operator artifact reviewed on a PR, and no predecessor file to migrate. What ports is the *shape* — a per-worktree checklist the session hook keeps pointing at.
