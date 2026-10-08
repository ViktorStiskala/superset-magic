---
name: setup-github-ci
description: >
  Add or update this repository's GitHub Actions workflow that renders the operator checklist into a
  pull-request comment. Use when setting up checklist CI for the first time, when the workflow's
  pinned ss-magic-plugin version is stale, or when the workflow was hand-edited and no longer matches.
---

# Set up checklist CI

`ss-magic-plugin setup-github-ci` writes the workflow, to
`.github/workflows/ss-magic-checklist.yml`. This skill decides *whether* to write it, and reports
what changed. Never hand-write or hand-edit that file – it pins and checksum-verifies the
ss-magic-plugin it installs, and a hand edit is how that pin goes stale silently.

Start with a dry run, which reports and writes nothing:

    ss-magic-plugin setup-github-ci --check

Its first line is `state: <token>`. Branch on the token, not on the prose after it:

1. **`state: absent`** – no workflow present. Say what will be added: the file path, the
   `pull_request` trigger, the permissions each job requests, and the ss-magic-plugin version it
   will pin.
   Ask for confirmation, then run `ss-magic-plugin setup-github-ci`.
2. **`state: identical`** – already exactly what would be written. Nothing to do. Say so and stop; do
   not run the write.
3. **`state: differs`** – present, and changed locally. `--check` has already printed the diff; show
   it to the user rather than describing it. A local edit may be deliberate – a changed job name, an
   added step, a repository-specific runner – so ask whether to overwrite or keep the local version.
   Only on an explicit yes, run `ss-magic-plugin setup-github-ci --force`. A run without `--force`
   refuses this case on purpose, so never reach for the flag before asking.
4. **`state: pin-stale`** – a workflow this tool wrote and nobody edited since, but not the one this
   build would write. It is either the current workflow at a different pin, or an untouched workflow
   from an earlier template generation, at any version (the earliest generation pins the `ss-magic`
   CLI under `SS_MAGIC_VERSION`, not ss-magic-plugin). `--check` reports which version
   it pins and which it would move to, names the generation when it is an earlier one, and prints a
   diff – for an earlier generation that diff is more than the pin line, so show it rather than
   describing it. No local edit is at risk, so this advances without `--force`: ask, then run
   `ss-magic-plugin setup-github-ci`. A hand-edited workflow, even one derived from an earlier
   generation, is `state: differs` instead.

Confirmation is required in every branch that writes. There are exactly two ways this ends: the
workflow is written, or the user declined – and when they declined, say at which step.

## What the workflow does, and what it deliberately does not

- Triggers on `pull_request`, **never** `pull_request_target`.
- Splits into two jobs so no single job both runs pull-request code and holds a write token. `render`
  checks out the pull request with `contents: read` and uploads the rendered Markdown as an artifact;
  `comment` holds `pull-requests: write`, checks out nothing, and posts the downloaded artifact.
  Everything else is denied by a workflow-level `permissions: {}`.
- Installs the pinned ss-magic-plugin from its GitHub release (the `ss-magic-plugin-vX.Y.Z` line, not
  the `ss-magic` sync CLI's) and verifies the published SHA-256 before running it.
- Verifies and renders only the checklists the pull request adds or changes: it lists the top-level
  `docs/actions/*.checklist.json` files that differ from the branch the pull request targets (deleted
  files left out, a renamed one counted at its new path) and passes those names to `checklist verify`
  and `checklist render-md` as arguments. A pull request that touches no checklist renders and posts
  nothing.
- Posts the rendered Markdown as a pull-request comment, rewriting the same comment on each push
  rather than adding a new one. The body is capped (`--max-bytes 60000`) below GitHub's comment limit;
  a note names any checklist left out.
- Passes every checklist-derived value to the forge CLI through a file (`--body-file`) – never
  interpolated into a shell step, because checklist prose is repository-controlled text. The
  selected file names reach the verbs as quoted arguments read back from a NUL-separated file, never
  spliced into a command line.
- Skips the comment on pull requests opened **from a fork**. GitHub issues fork pull requests a
  read-only token no matter what the workflow asks for, so the comment cannot be posted there. The
  `render` job still runs, so an invalid checklist the pull request changes is still caught. Mention
  this if the repository takes outside contributions.

If a repository has no checklist yet, the verb says so and writes the workflow anyway – it stays
quiet until a pull request adds or changes one under `docs/actions/`. Run
`ss-magic-plugin checklist init <slug>` to create it.
