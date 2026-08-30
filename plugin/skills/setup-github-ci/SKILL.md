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
