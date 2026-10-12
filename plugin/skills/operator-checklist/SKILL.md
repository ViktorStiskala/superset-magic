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
and CI renders it into a PR comment whenever the pull request adds or changes it.

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
    ss-magic-plugin checklist verify [FILE...]       # exits non-zero on any error; warnings do not fail
    ss-magic-plugin checklist render-md [--max-bytes N] [FILE...]   # the Markdown CI posts

With no `FILE`, `verify` and `render-md` work on the active checklist. A `FILE` is a
repository-relative `docs/actions/<stem>.checklist.json` – the form CI passes for each checklist a
pull request changes; an absolute path, a `..`, a nested or differently named file, a symlink or a
path outside the repository is refused with exit 2. Several files are verified one by one (exit 1 if
any is invalid) or rendered one after another. `--max-bytes N` (at least 2048) bounds the whole
rendered body and names any document it had to leave out. Neither verb moves the active-checklist
pointer.

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
