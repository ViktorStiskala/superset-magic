---
name: setup-github-ci
description: >
  Add or update this repository's GitHub Actions workflow that renders the operator checklist into a
  pull-request comment. Use when setting up checklist CI for the first time, when the workflow's
  pinned ss-magic version is stale, or when the workflow was hand-edited and no longer matches.
---

# Set up checklist CI

`ss-magic-plugin setup-github-ci` writes the workflow, to
`.github/workflows/ss-magic-checklist.yml`. This skill decides *whether* to write it, and reports
what changed. Never hand-write or hand-edit that file – it pins and checksum-verifies the ss-magic it
installs, and a hand edit is how that pin goes stale silently.

Start with a dry run, which reports and writes nothing:

    ss-magic-plugin setup-github-ci --check

Its first line is `state: <token>`. Branch on the token, not on the prose after it:

1. **`state: absent`** – no workflow present. Say what will be added: the file path, the
   `pull_request` trigger, the permissions each job requests, and the ss-magic version it will pin.
   Ask for confirmation, then run `ss-magic-plugin setup-github-ci`.
2. **`state: identical`** – already exactly what would be written. Nothing to do. Say so and stop; do
   not run the write.
3. **`state: differs`** – present, and changed locally. `--check` has already printed the diff; show
   it to the user rather than describing it. A local edit may be deliberate – a changed job name, an
   added step, a repository-specific runner – so ask whether to overwrite or keep the local version.
   Only on an explicit yes, run `ss-magic-plugin setup-github-ci --force`. A run without `--force`
   refuses this case on purpose, so never reach for the flag before asking.
4. **`state: pin-stale`** – this exact workflow, naming an older ss-magic. `--check` reports which
   version it pins and which it would move to. Ask, then run `ss-magic-plugin setup-github-ci`.

Confirmation is required in every branch that writes. There are exactly two ways this ends: the
workflow is written, or the user declined – and when they declined, say at which step.

## What the workflow does, and what it deliberately does not

- Triggers on `pull_request`, **never** `pull_request_target`.
- Splits into two jobs so no single job both runs pull-request code and holds a write token. `render`
  checks out the pull request with `contents: read` and uploads the rendered Markdown as an artifact;
  `comment` holds `pull-requests: write`, checks out nothing, and posts the downloaded artifact.
  Everything else is denied by a workflow-level `permissions: {}`.
- Installs the pinned ss-magic from its GitHub release and verifies the published SHA-256 before
  running it.
- Runs `checklist verify`, then posts `checklist render-md` as a pull-request comment, rewriting the
  same comment on each push rather than adding a new one.
- Passes every checklist-derived value to the forge CLI through a file (`--body-file`) – never
  interpolated into a shell step, because checklist prose is repository-controlled text.
- Skips the comment on pull requests opened **from a fork**. GitHub issues fork pull requests a
  read-only token no matter what the workflow asks for, so the comment cannot be posted there. The
  `render` job still runs, so an invalid checklist is still caught. Mention this if the repository
  takes outside contributions.

If a repository has no checklist yet, the verb says so and writes the workflow anyway – it stays
quiet until `docs/actions/` holds one. Run `ss-magic-plugin checklist init <slug>` to create it.
