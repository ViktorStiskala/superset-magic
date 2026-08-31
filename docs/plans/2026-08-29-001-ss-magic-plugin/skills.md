# Skills – content and invocation

Companion to [the plan](../2026-08-29-001-feat-ss-magic-claude-plugin-plan.md).

Three skills ship inside the plugin. Because the manifest `name` is `ss-magic`, they are invoked
`/ss-magic:scratchpad`, `/ss-magic:operator-checklist` and `/ss-magic:setup-github-ci` – which is why
the directories are **not** prefixed (`skills/scratchpad/`, not `skills/ss-scratchpad/`).

A skill is Markdown with YAML frontmatter; `SKILL.md` is always scanned inside a plugin's `skills/`
directory. Editing one does **not** hot-reload – the whole plugin is snapshotted at session start
(measured), so a changed skill needs `/reload-plugins` or a restart.

## Every skill body invokes the wrapper

A skill body runs its commands through the **Bash tool**, and `${CLAUDE_PLUGIN_DATA}` is not exported
there – it reaches hook and MCP/LSP processes only. A skill therefore can never name the bootstrapped
binary's path. What it can name is `ss-magic-plugin`, the wrapper the plugin ships on its own `bin/`,
which **is** on the Bash tool's `PATH` while the plugin is enabled (R75). The wrapper resolves the
pinned binary and injects the `plugin` verb, so a skill writes `ss-magic-plugin status --json`, never
`ss-magic plugin status --json` and never `$CLAUDE_PLUGIN_DATA/bin/ss-magic …`. See
[plugin-assets.md](./plugin-assets.md) for the wrapper itself.

Two rules fall out, and both are review-checkable by grep:

- **No skill body contains the string `CLAUDE_PLUGIN_DATA`.** It would expand to nothing in the Bash
  tool and produce a path at the filesystem root.
- **No skill body invokes bare `ss-magic`.** That resolves to whatever the user happens to have on
  `PATH`, at whatever version – the exact drift the pin exists to prevent (R69). It would also route
  through the update gate and, for a bare invocation, the TUI.

## `skills/scratchpad/SKILL.md`

The one the user invokes right after `/ce-brainstorm`, `/ce-plan`, `/ce-ideate`, `/ce-work` and
similar. Its job is small and mechanical: resolve identity, scaffold state, then hand back a
discipline for keeping it current.

```markdown
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
```

## `skills/operator-checklist/SKILL.md`

The checklist is **committed repository content**, not scratchpad state: one JSON document per
action at `docs/actions/<YYYY-MM-slug>.checklist.json` (R82). ss-magic owns its schema, its
validator, its canonical ordering and its Markdown renderer – all inside the binary
(`src/plugin/checklist/`, KTD19) – so nothing about the format lives in a Markdown file an editor can
drift from.

The skill is thin for a reason that has changed since the first draft. It is no longer thin because
the hook supplies a pointer; it is thin because **the CLI is the format**. The file's shape does not
need to be recited in prose, because `ss-magic-plugin checklist verify` is the authority and every
write goes through a verb that re-validates.

```markdown
---
name: operator-checklist
description: >
  Track the operational steps a change needs before it is safe to ship – verification, rollout,
  decisions still open, and the follow-ups that outlive the code change. Use when work has
  real-world consequences beyond the diff, and whenever a checklist item is completed or a decision
  lands.
---

# Operator checklist

The checklist is committed repository content: one JSON document per action at
`docs/actions/<YYYY-MM-slug>.checklist.json`. It is reviewed on the pull request like any other file,
and CI renders it into a PR comment on every push to the PR.

**You cannot read or edit it directly.** `Read`, `Edit`, `Write` and notebook edits on a checklist
path are denied, and the denial names the verb to use instead. This is deliberate: the file is
machine-ordered and machine-validated, and a hand edit that reorders items or drops a required field
is invisible until the render breaks. Go through the CLI.

## The verbs

    ss-magic-plugin checklist init <slug>            # create docs/actions/<YYYY-MM-slug>.checklist.json
    ss-magic-plugin checklist add-item <section> <id>
    ss-magic-plugin checklist add-entry <id>         # a changelog entry
    ss-magic-plugin checklist set <id> <dotted-key> <value>
    ss-magic-plugin checklist done <id>
    ss-magic-plugin checklist list                   # rendered, in canonical order
    ss-magic-plugin checklist verify                 # exits non-zero on any violation
    ss-magic-plugin checklist render-md              # the Markdown CI posts

Multi-line bodies – an action step, a `why`, a description – are read from **stdin** rather than
passed as an argument, so newlines and quoting survive intact. Dotted keys follow the same
convention as `ss-magic-plugin config set`.

`init` also records the active checklist under `.superset/.magic/`, so later sessions and the
commit-time nudge know which document is live.

## The discipline

- **Ids are permanent.** Kebab-case, starting with a letter, unique across the changelog and every
  section jointly. Never rename one – references and history hang off it.
- **`done` means done and verified**, not "probably fine". An item that turned out to be unnecessary
  is set to a non-blocking state with a stated reason, never deleted.
- **Every item needs at least one action step**, and every reference must be an absolute URL.
- **Record decisions as they land**, not at the end. A decision-blocking item that is still open is
  the most useful thing the render shows a reviewer.
- **Run `verify` before you commit.** It catches what the schema leaves implicit: a done item with no
  completion timestamp, a null `expected` on an item whose kind requires one, a duplicate id, a
  relative reference URL.

Ordering is not yours to choose. The document is stored in canonical order and re-sorted on every
write, so a diff shows what changed rather than where things moved.

See [reference.md](./reference.md) for the section model, the priority vocabulary, and the field
shapes the verbs accept.
```

`reference.md` carries the section model and field shapes. It is **project-agnostic by requirement**
(R83): `sections` is an ordered array of `{ id, title, items }` with a binary-owned default set,
replacing the source format's four fixed deploy-lifecycle keys; there is no fixed trailing
release-approval block; and `priority` is domain-neutral – **blocking**, **decision-blocking**,
**follow-up** – rather than "blocks the deploy". A project that declares its own section set gets
that order; one that declares none gets the default (AE69).

Two things `reference.md` must **not** do, because they would drift the moment the binary changes:
restate the JSON schema field-by-field (the schema is the binary's, and `verify` is its enforcement),
or give an example document to copy (an example is a template, and templates become the format).
Point at `checklist init` and `checklist verify` instead.

## `skills/setup-github-ci/SKILL.md`

Replaces the removed `/ss-magic:setup` (R93). The split is the point: **a Markdown skill cannot write
a workflow file**, so `ss-magic plugin setup-github-ci` owns the bytes – pinned, least-privilege,
golden-file tested (R94) – and the skill owns the conversation around it.

```markdown
---
name: setup-github-ci
description: >
  Add or update this repository's GitHub Actions workflow that renders the operator checklist into a
  pull-request comment. Use when setting up checklist CI for the first time, when the workflow's
  pinned ss-magic version is stale, or when the workflow was hand-edited and no longer matches.
---

# Set up checklist CI

`ss-magic-plugin setup-github-ci` writes the workflow. This skill decides *whether* to write it, and
reports what changed. Never hand-write the workflow file – it pins and checksum-verifies the
ss-magic it installs, and a hand edit is how that pin goes stale silently.

Start with a dry run:

    ss-magic-plugin setup-github-ci --check

It reports one of four states. Take the matching branch:

1. **No workflow present.** Say what will be added – the file path, the `pull_request` trigger, the
   permissions it requests, and the ss-magic version it will pin. Ask for confirmation, then run
   `ss-magic-plugin setup-github-ci`.
2. **Present and identical.** Nothing to do. Say so and stop; do not write.
3. **Present and differing.** Show the difference before proposing anything. A local edit may be
   deliberate – a changed job name, an added step, a repository-specific runner. Ask whether to
   overwrite or keep the local version. If the answer is keep, stop and say what stays stale.
4. **Pin stale.** The workflow is otherwise current but names an older ss-magic. Say which version it
   pins and which it would move to, then ask.

Confirmation is required in every branch that writes. There are exactly two ways this ends: the
workflow is written, or the user declined – and when they declined, say at which step.

## What the workflow does, and what it deliberately does not

- Triggers on `pull_request`, **never** `pull_request_target`, and never checks out pull-request head
  code in a job holding write permissions.
- Declares an explicit least-privilege `permissions:` block.
- Installs a pinned ss-magic and verifies its checksum before running it.
- Runs `checklist verify`, then posts `checklist render-md` as a PR comment, updating the same
  comment on each push rather than adding a new one.
- Passes every checklist-derived value to the forge CLI through a file or stdin – never interpolated
  into a shell step, because checklist prose is repository-controlled text.

If a repository has no checklist yet, run `ss-magic-plugin checklist init <slug>` first; a workflow
with nothing to render is noise on every pull request.
```

## Where the old setup skill's diagnostics went

`/ss-magic:setup` is gone (R93). It existed to diagnose a local install that no longer exists – there
is no install verb, no personal-scope tree, and no sync-time plugin step to repair.

Its genuinely useful half was a symptom-to-cause table, and that content is now **`ss-magic plugin
status`'s human-readable output**, which R65 already makes the single answer to "why is the plugin not
acting". A table in a skill body is a copy that drifts; `status` reads the live state and can state
which layer is the one saying no. The cases it must cover, each of which the old skill carried as
prose:

| symptom | what `status` reports |
|---|---|
| plugin installed but not acting | both enablement layers side by side – the harness's scope, registration id and enabled flag, and ss-magic's own overlaid `plugin.enabled` for this repository (R65) |
| hooks registered but not firing | nothing hot-reloads; the plugin is snapshotted at session start, so `/reload-plugins` or a restart is needed |
| binary missing or version-mismatched | the pinned version, the resolved binary path under the plugin data directory, and the last bootstrap outcome (R77) |
| first session after a pin bump behaves as if the plugin were absent | expected: that session runs with ss-magic's hooks inert, because sibling hooks on one event fire concurrently and the bootstrap does not finish first (R77, R81) |
| plugin config vanished from `magic.json` | older ss-magic versions dropped unknown keys on rewrite; fixed by R4's round-trip |
| state tree not ignored, so hooks refuse to write | R63's refusal, with its reason – re-run `init`, `migrate` or `sync` to add the rule |

Enabling and disabling stay CLI operations rather than skill prose: `ss-magic plugin enable` /
`disable`, with `--local` targeting the **main checkout's** `magic.local.json` (R7 – a worktree's
overlay is itself forward-synced, so setting it there is clobbered by the next `ss-magic sync`).
`disable` flips the overlay only; removing the plugin from the machine is `claude plugin uninstall
ss-magic@ss-magic`, which also deletes `${CLAUDE_PLUGIN_DATA}` unless `--keep-data` is passed.

## What the koolman original contributed, and what it did not

Earlier drafts of this document recorded that the koolman original's CI job – the one that renders the
checklist into a pull-request comment – "does not transfer", on the reasoning that ss-magic has no
per-branch operator artifact reviewed on a PR. **That is backwards, and it is corrected here.** The
checklist is exactly such an artifact: committed under `docs/actions/`, reviewed on the pull request,
and rendered into a PR comment by CI on every push (R82, R93-R95). The CI job is one of the parts that
ports *best*, and `/ss-magic:setup-github-ci` exists to install it.

What genuinely does not transfer is the original's **domain and runtime**, not its shape:

- **The four fixed deploy-lifecycle sections** (`pre-deploy`, `post-deploy`, `verification`,
  `visual`) and the trailing release-approval block. Replaced by R83's ordered, declarable `sections`
  array with a binary-owned default.
- **The e-commerce specifics** – Google Search Console verification and the rest. Nothing generic
  survives them.
- **`priority` defined as "blocks the deploy".** Replaced by the domain-neutral blocking /
  decision-blocking / follow-up vocabulary.
- **The Node runtime.** The original ran `npx tsx`; ss-magic ships no ambient Node toolchain, so the
  schema, the validator and the renderer are Rust inside the binary (KTD19).
- **The renderer's project coupling** – shelling out to a forge CLI for a repository URL, a
  hard-coded timezone and locale date format, a grep into a plan file. All dropped: the URL comes
  from the `origin` remote the crate already reads for pack naming, and dates render through
  ss-magic's existing UTC formatter with no date crate added (R85).
- **The `CHECKLIST.md` migration.** There is no predecessor file here to migrate from.
