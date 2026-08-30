---
name: scratchpad
description: >
  Open or resume this worktree's durable session scratchpad and keep it current, so work survives
  autocompaction. Use immediately after starting a substantial task – a brainstorm, a plan, an
  implementation run – and whenever context is about to be lost. Not for one-off questions.
---

# Session scratchpad

Run `ss-magic-plugin scratchpad ensure` first. It resolves this worktree's deterministic
slug, creates `.superset/.magic/sessions/<repo>-<branch>/`, scaffolds any missing state
file, and prints the resolved directory. It never rewrites a file that already exists – the content
is yours.

A dispatched agent that receives no injected context – an Explore subagent, for instance – can run
`ss-magic-plugin status --json` instead, to recover the resolved slug, session directory, and gate
thresholds directly. When such an agent's findings are what a blocked `Read` should hand back, write
them with `ss-magic-plugin conclude <original-path>` so the Read gate serves that conclusion in
place of the oversized file.

If the directory already has content, **read `STATUS.md` first** and continue from it rather than
starting over.

## The files, and what belongs in each

- **STATUS.md** – where the work stands *right now*. Newest block first under
  `## CURRENT STATE - <timestamp> (read this block first; everything below is history)`. When you
  update it, demote the previous block to `## HISTORY - <timestamp> (superseded, kept for the audit
  trail)`. Never delete history; never edit a demoted block.
- **TASKS.md** – the task list with status, including user follow-ups folded in as they arrive.
- **DECISIONS.md** – decisions that are settled, each with the alternative it beat and why.
- **LEARNINGS.md** – append-only. New entries as `## <timestamp> - <label>`. Never edit an old one.
- **CONTEXT.md** – context that would be expensive to rediscover, grouped by topic, not by time.
- **research-<topic>/** – durable artifacts worth keeping: extracted findings, measurements, notes.

The operator checklist is **not** here. It is committed repository content under `docs/actions/`;
see the operator-checklist skill.

## Keeping it current

Update the scratchpad when something lands, not on a timer: a decision settles, a task changes
state, a measurement comes back, the user redirects the work. Write the *conclusion*, not the
transcript.

Before a compaction and before ending a work session, reconcile: is STATUS.md's current block
actually current? Are any tasks silently abandoned? If something looks incomplete or hung, verify
its real state and resume it rather than assuming it finished.

Promote anything durable out of the scratchpad and into the repo – `docs/`, a plan, a solution
write-up. The scratchpad is working state, and it is never committed.
