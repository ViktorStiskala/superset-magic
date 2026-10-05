# Worked example

A run of the [migrate-repository skill](./SKILL.md) on a reference consumer repository, described
by structure only. Every name, item and date below is a placeholder; the rules each step applies
are in [reference.md](./reference.md).

## The repository before

**Legacy checklists.** One `docs/actions/<YYYY-MM-branch>/CHECKLIST.md` per branch: about a dozen
folders, from 2.5 KB to about 380 KB, holding from 4 to about 375 items each. Every file starts
with a mandatory header:

```markdown
# <Title>

Last updated: <YYYY-MM-DD>\
Branch name: `<branch>`
```

**Sections.** Four time-ordered top-level sections, `Pre-Deploy Requirements`,
`Post-Deploy Requirements`, `Post-Deploy Verification` and `Visual/Functional Checks`, plus ad-hoc
ones in some files. Inside them, `### <Area>` subsections named after a console or an owner.

**Items.** Two styles, an older and a newer one. `<P>` stands for the priority emoji some rows
carry:

```markdown
- [ ] **<action>** **PASS:** <expected result>
- [ ] <P> <action> – <expected result> (<why>)
- [x] <action> – <expected result> – done <YYYY-MM-DD>
- [ ] **Decision: <question>?** Default: <the option taken if nobody decides>
  > Superseded <YYYY-MM-DD>: <what changed>
```

**Governance and tooling.**

- A standing checklist outside `docs/actions/`, which stays Markdown.
- A project skill of the same name as the plugin's, `operator-checklist`, with a shell resolver
  that derives the folder from the branch name.
- A `docs/actions/CLAUDE.md` describing the folder layout and an "update before every commit" rule.
- A docs-reference checker that fails on dead backticked paths.
- Domain `CLAUDE.md` files telling agents to add a row to the checklist.
- Code comments linking a historical checklist.
- A `.scratchpad/.plugin/` reservation in the root `CLAUDE.md`.

## The run

The operator is in a linked worktree on the placeholder branch `area-a-rollout` and asks to migrate
the repository.

### Preflight

```plaintext
Preflight (read only)
  binary: installed, ss-magic-plugin 1.1.0, versions aligned
  repository: linked worktree; main checkout at <main checkout path>
  branch: area-a-rollout (default branch: main, compared against origin/main)
  plugin.enabled: false (main checkout overlay); acting: false
  state tree ignored: false – enable adds the rule
  seed diff: .superset/magic.json has an unstaged `plugin.gate` block from the session-start bootstrap
  legacy checklists: about a dozen folders under docs/actions/
  this branch: docs/actions/2026-09-area-a-rollout/CHECKLIST.md – 14 KB, 31 items, 9 done
  legacy skill: .claude/skills/operator-checklist/ (here and on origin/main)
  project rules: several matches across a handful of files
  CI workflow: state: absent
  problems:
    - git does not ignore .superset/.magic/ — every state-writing hook refuses to write anything while that is true. `ss-magic-plugin enable` adds the rule.
    - `plugin.enabled` is not true in <main checkout path> — every hook no-ops, whatever the harness has loaded. Turn it on with `ss-magic-plugin enable`.
```

Not the default branch, not yet migrated: full migration. The file is under 80 items and 60 KB, so
no size question will be asked. Preflight read it whole with the Read tool, before enabling.

### Enable

The skill asks committed, `--local`, both or no, and recommends both: `enabled` is read from the
main checkout, so a committed-only enable written in this worktree acts only once the commit
reaches it; `--local` acts now in every worktree on this machine. The operator picks both.

The bootstrap already seeded the `plugin` key, so `seed-config` is skipped.

```bash
ss-magic-plugin enable
ss-magic-plugin enable --local
ss-magic-plugin status --json    # enablement.ss_magic.enabled: true, acting: true, state_tree.ignored: true
```

The preflight listed no changed `.gitignore`; now the root `.gitignore` shows up, so that is the
one the enable changed. Commit gate: `.superset/magic.json` and `.gitignore`, as "Enable
ss-magic-plugin". Yes:

```bash
git add -- .superset/magic.json .gitignore
git commit -m 'Enable ss-magic-plugin' -- .superset/magic.json .gitignore
```

### Sections

| Section id | Default title | Retitled to |
|---|---|---|
| `verification` | Verification | Pre-Deploy Requirements |
| `rollout` | Rollout | Post-Deploy Requirements |
| `decisions` | Open decisions | Post-Deploy Verification |
| `follow-ups` | Follow-ups | Visual/Functional Checks |

The file's fifth section, `Additional Checks`, folds into `follow-ups`; its rows get the title
prefix `Additional Checks: `.

```bash
set -e
ss-magic-plugin checklist init 2026-09-area-a-rollout
ss-magic-plugin checklist set document title <<'SSMAGIC_LEGACY'
Area A rollout
SSMAGIC_LEGACY
ss-magic-plugin checklist set verification title <<'SSMAGIC_LEGACY'
Pre-Deploy Requirements
SSMAGIC_LEGACY
ss-magic-plugin checklist add-entry migrated-from-legacy <<'SSMAGIC_LEGACY'
Migrated from docs/actions/2026-09-area-a-rollout/CHECKLIST.md (folder 2026-09-area-a-rollout), mode: full
SSMAGIC_LEGACY
```

The other three sections are retitled the same way. `base` is the legacy file's first commit,
`2026-09-02T09:30:00+02:00`.

### Three converted items

**An open row with the priority marker, in an area** – the 4th row of the file (`n = 3`):

```markdown
### Admin console
- [ ] <P> Rotate the staging API key – requests signed with the old key return 401 (the old key was pasted into a shared ticket)
  > Superseded 2026-09-10: rotate the production key in the same window
```

```bash
set -e
ss-magic-plugin checklist add-item verification rotate-the-staging-api-key <<'SSMAGIC_LEGACY'
Admin console: Rotate the staging API key
SSMAGIC_LEGACY
ss-magic-plugin checklist set rotate-the-staging-api-key expected <<'SSMAGIC_LEGACY'
requests signed with the old key return 401
SSMAGIC_LEGACY
ss-magic-plugin checklist set rotate-the-staging-api-key why <<'SSMAGIC_LEGACY'
the old key was pasted into a shared ticket
SSMAGIC_LEGACY
ss-magic-plugin checklist set rotate-the-staging-api-key steps <<'SSMAGIC_LEGACY'
Rotate the staging API key
SSMAGIC_LEGACY
ss-magic-plugin checklist set rotate-the-staging-api-key priority blocking
ss-magic-plugin checklist set rotate-the-staging-api-key created 2026-09-02T09:30:03+02:00
ss-magic-plugin checklist add-entry note-rotate-the-staging-api-key <<'SSMAGIC_LEGACY'
rotate-the-staging-api-key: Superseded 2026-09-10: rotate the production key in the same window
SSMAGIC_LEGACY
ss-magic-plugin checklist set note-rotate-the-staging-api-key created 2026-09-10T12:00:00+0200
```

**A done row in the older style, with a command and an apostrophe in its title** – the 7th row
(`n = 6`). The title is stored verbatim; the command is transcribed into `steps` and never run:

```markdown
- [x] **Run `./scripts/migrate --env staging` and check the operator's log** **PASS:** the log ends with the new revision – done 2026-09-05
```

```bash
set -e
ss-magic-plugin checklist add-item verification run-scripts-migrate-env-staging-and <<'SSMAGIC_LEGACY'
Run `./scripts/migrate --env staging` and check the operator's log
SSMAGIC_LEGACY
ss-magic-plugin checklist set run-scripts-migrate-env-staging-and expected <<'SSMAGIC_LEGACY'
the log ends with the new revision
SSMAGIC_LEGACY
ss-magic-plugin checklist set run-scripts-migrate-env-staging-and steps <<'SSMAGIC_LEGACY'
Run `./scripts/migrate --env staging` and check the operator's log
SSMAGIC_LEGACY
ss-magic-plugin checklist set run-scripts-migrate-env-staging-and created 2026-09-02T09:30:06+02:00
ss-magic-plugin checklist done run-scripts-migrate-env-staging-and
ss-magic-plugin checklist set run-scripts-migrate-env-staging-and completed 2026-09-05T12:00:00+0200
```

**A decision row with a stated default** – the 22nd row (`n = 21`), under the third legacy
section, so in `decisions`:

```markdown
- [ ] **Decision: keep the legacy export endpoint for one more release?** Default: keep it until the payments provider confirms the new format.
```

The bold and the leading `Decision:` are markup: the marker becomes `kind decision`, and the
question is stored as written, lowercase first letter included.

```bash
set -e
ss-magic-plugin checklist add-item decisions keep-the-legacy-export-endpoint-for <<'SSMAGIC_LEGACY'
keep the legacy export endpoint for one more release?
SSMAGIC_LEGACY
ss-magic-plugin checklist set keep-the-legacy-export-endpoint-for kind decision
ss-magic-plugin checklist set keep-the-legacy-export-endpoint-for priority decision-blocking
ss-magic-plugin checklist set keep-the-legacy-export-endpoint-for expected null
ss-magic-plugin checklist set keep-the-legacy-export-endpoint-for description <<'SSMAGIC_LEGACY'
Default: keep it until the payments provider confirms the new format.
SSMAGIC_LEGACY
ss-magic-plugin checklist set keep-the-legacy-export-endpoint-for steps <<'SSMAGIC_LEGACY'
keep the legacy export endpoint for one more release?
SSMAGIC_LEGACY
ss-magic-plugin checklist set keep-the-legacy-export-endpoint-for created 2026-09-02T09:30:21+02:00
```

After all 31 rows:

```plaintext
$ ss-magic-plugin checklist verify docs/actions/2026-09-area-a-rollout.checklist.json
docs/actions/2026-09-area-a-rollout.checklist.json is valid
```

### The stub

A full conversion, so `docs/actions/2026-09-area-a-rollout/CHECKLIST.md` becomes:

```markdown
# Area A rollout

This branch's operator checklist moved to `docs/actions/2026-09-area-a-rollout.checklist.json` on 2026-09-14.
It is managed with `ss-magic-plugin checklist` (`list`, `add-item`, `set`, `done`, `verify`) and
rendered into a comment on the pull request. Do not edit the JSON by hand.
```

The folder and all the other legacy folders stay. Commit gate: the document and the stub, as
"Convert the operator checklist to the plugin JSON format". Yes.

### CI

```plaintext
$ ss-magic-plugin setup-github-ci --check
state: absent
```

Following the `setup-github-ci` skill, the operator confirms the write; a second `--check` reports
`state: identical`. The workflow is installed.

### Retire diff

Shown once and confirmed once, then applied. The next question is the commit gate, naming the
subject "Retire the hand-written checklist rules" and every path below, the workflow included:

```plaintext
D  .claude/skills/operator-checklist/SKILL.md
D  .claude/skills/operator-checklist/<resolver script>
M  CLAUDE.md                project rule replaced by the plugin paragraph; PR-description link
                            rule dropped (workflow installed); .scratchpad/.plugin/ reservation
                            removed, .scratchpad/ kept
M  docs/actions/CLAUDE.md   describes the JSON documents; Markdown folders described as history
M  <domain>/CLAUDE.md (each) "add a row" now names ss-magic-plugin checklist add-item
A  .github/workflows/ss-magic-checklist.yml
```

Not changed: the docs-reference checker (no file it lists was deleted), the standing checklist, and
the code comments linking a historical checklist (their folders still exist).

### Final report

```markdown
## Migration report

Mode: full migration\
Enablement: plugin.enabled true (both), acting true\
Checklist: docs/actions/2026-09-area-a-rollout.checklist.json – 31 items converted (full), verify green\
CI workflow: state identical (written)

Commits (none pushed):
- <hash> Enable ss-magic-plugin
- <hash> Convert the operator checklist to the plugin JSON format
- <hash> Retire the hand-written checklist rules

Items left open:
- rotate-the-staging-api-key – Admin console: Rotate the staging API key
- keep-the-legacy-export-endpoint-for – keep the legacy export endpoint for one more release?
- … (20 more)

Next:
- In every other worktree of this repository, run `git restore .superset/magic.json` before it
  merges the default branch (the session-start bootstrap left an unstaged seed there).
- Other open branches merge the default branch first, then run `/ss-magic:migrate-repository`;
  it takes branch-only mode there.
- Run this repository's own checks before pushing.
```
