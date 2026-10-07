---
title: Docs Sync and Repository Migration - Plan
type: feat
date: 2026-10-05
artifact_contract: ce-unified-plan/v1
execution: code
product_contract_source: ce-plan-bootstrap
deepened: 2026-10-05
---

# Docs Sync and Repository Migration - Plan

## Goal Capsule

- **Objective:** Every current-state document in this repository describes the code as it is today, an agent working here loads a short index and the always-applicable rules plus only the per-crate maps for the files it touches, the checklist CI job stays green once a repository has merged more than one checklist, and an existing repository that still runs a hand-written Markdown operator checklist can be brought onto the plugin from inside a session, by asking.
- **Means:** a drift sweep against the verified work list, a split of `CLAUDE.md` into an index plus `.claude/rules/*.md` (KTD1, KTD2), a diff-scoped checklist CI template with explicit-file verbs and legacy-template recognition (KTD5, KTD6, KTD7), and a new plugin skill `migrate-repository` released as plugin 1.1.0 (KTD8, KTD9).
- **Authority:** the session-settled Key Decisions below and the user's global constraints in this plan's Implementation Constraints outrank research evidence; [drift-findings.md](./2026-10-05-1610-feat-docs-sync-and-repository-migration/drift-findings.md) is the authoritative sweep list; the code is the authority for every fact a document states.
- **Stop conditions:** a drift fix that would require a behavior change (list it under Code suspects, do not change the code); any edit to `docs/plans/`, `docs/solutions/` or `docs/brainstorms/` beyond adding this plan's own files; anything client-identifying from the reference consumer repository about to land in a committed file; a plugin-group version surface that `--check` enumerates and this plan did not name.
- **Execution profile:** `ce-work`, one commit per unit on the current branch `ViktorStiskala/docs-update`, never switching branches; no tag is pushed and no release is cut by this plan.
- **Who finishes:** the implementing agent lands U1 through U8 and runs the Verification Contract; the user reviews the Code suspects list and decides the release.

---

## Product Contract

### Summary

Fix every confirmed documentation drift in the six current-state documents and the shipped skills, rewrite historical narration as present-tense fact, and fix stale code comments without changing behavior.
Turn the 1,752-line `CLAUDE.md` into a short always-loaded index with three unconditional rule files and four path-scoped architecture maps, with CI guards extended so the new files are as protected as the old one.
Fix the one plugin bug the sweep surfaced: the shipped CI workflow renders the wrong checklist with one merged document and fails outright with two, so it will instead verify and render exactly the checklists a pull request touches, and `setup-github-ci` will upgrade every untouched workflow it ever wrote without `--force`.
Ship a fourth skill, `/ss-magic:migrate-repository`, that takes an installed-but-not-enabled repository onto the plugin: preflight, the enable decision, conversion of the active branch's legacy checklist through the existing verbs, retirement of the consumer's hand-written rules, CI setup and a final report.
All plugin edits ride one minor bump to 1.1.0.

### Problem Frame

Two prior sweeps left the docs accurate at their time, and the workspace split and the plugin's growth since then produced 62 verified wrong claims, 70 passages that narrate history instead of stating current fact, and a `CLAUDE.md` whose 124 KB loads whole into every session, larger than the plugin's own default Read gate.
The reference consumer repository that motivated the plugin still runs the predecessor design (a per-branch `docs/actions/<YYYY-MM-branch>/CHECKLIST.md`, a project skill of the same name as the plugin's, a shell resolver and an "update before every commit" prose rule), and the plugin's original design note deliberately excluded migrating it (`docs/plans/2026-08-29-001-ss-magic-plugin/skills.md`, "The `CHECKLIST.md` migration"). That exclusion is reversed by this plan; the historical note is left as written.
Surveying that repository also exposed a bug that any adopter hits on its second merged checklist: CI has no pointer, `resolve_active` refuses when `docs/actions/` holds several documents, and with exactly one merged document every later PR renders that old one.

### Requirements

**Docs sweep**

- R1. Every current-state document (`README.md`, the contributor instructions wherever they live after the split, `CONCEPTS.md`, `CONTRIBUTING.md`, `.cursor/BUGBOT.md`, `docs/runbooks/forge-tag-and-release-protection.md`, `plugin/skills/*`) states what the code does now, applying every fix in [drift-findings.md](./2026-10-05-1610-feat-docs-sync-and-repository-migration/drift-findings.md) (its own ids C1–C26, R1–R11, N1–N6, K1–K5, B1–B13 and F1, unrelated to this plan's R-numbers) and re-deriving every number it touches from the code rather than from the findings.
- R2. Each of the 70 historical-narration passages listed in the findings is rewritten as current state, keeping every bracketed fact the findings mark as load-bearing.
- R3. Stale code comments S3–S9 and S12–S14 are fixed with no behavior change; the CLI version stays `0.11.2` because no CLI output or behavior changes.
- R4. Behavior-level suspects (S1, S2, S10, and B6's "port the checks" half) are listed for the user under Code suspects and are not changed.
- R5. `docs/plans/`, `docs/solutions/` and `docs/brainstorms/` are not rewritten; the only additions under `docs/plans/` are this plan and its sibling directory.
- R6. `.cursor/BUGBOT.md` stays self-contained: it restates every convention inline, contains no Markdown link, and names no individual rule file (a `.claude/rules/<name>.md` path), though it may name the `.claude/rules/` directory as the subject of a review rule; it is re-synchronised in the same unit as every convention it mirrors.
- R7. Every `.md` file this plan creates or edits follows the user's Markdown rules (en dashes, `./`-prefixed relative links with plain link text, `plaintext` fences, trailing `\` hard breaks, mermaid over ASCII art), with one exception owned by R10.

**Contributor-instructions layout**

- R8. `CLAUDE.md` becomes an index under 200 lines: what the repository is, the three-crate layout with its mermaid diagram, a table of every rule file with its `paths` globs and a one-line "read this before touching X", a three-to-five line summary per crate, the cross-crate invariants, and the pointer to `CONCEPTS.md` and `docs/solutions/`.
- R9. Hard rules, conventions and build/release rules live in unconditional rule files that load every session; each crate's module map lives in a `paths:`-scoped rule file that loads on a Read, Write or Edit of a matching file (KTD1).
- R10. The two hard-rules sections (`## Secret-safety constraints (hard rules)` and `## Plugin constraints (hard rules)`, lines 1651–1741 at `7f6c62d`) move into `.claude/rules/hard-rules.md` byte-identically except for re-based link targets, provable at the U1 commit by the diff in the Verification Contract. After U1 the file receives exactly three listed edits in U2: the C22 wording fix, the line-1657 narration rewrite ("the real incident this run fixed"), and the line-1664 prose em dash.
- R11. Every relative link in `CLAUDE.md`, `.claude/rules/*.md`, `README.md`, `CONTRIBUTING.md`, `CONCEPTS.md`, `docs/runbooks/*.md`, `docs/solutions/**/*.md` and `plugin/skills/**/*.md` resolves to an existing file.
- R12. The CI document guards cover the rule files: the retired `ss-magic plugin ` spelling grep scans `.claude/rules/`, BUGBOT is asserted link-free and free of individual rule-file paths, relative links are checked, every rule file's frontmatter is either absent or a well-formed `paths:` list, and the always-loaded set (`CLAUDE.md` plus every `.claude/rules/**/*.md` without a `paths:` list, derived rather than named) stays under 50,000 bytes; the guards run in the release-gating `plugin` job and on the macOS `/bin/bash` 3.2 leg; `CONTRIBUTING.md` and BUGBOT describe the guards as they are.
- R13. The "after every implementation change" convention names the rule files as the targets (a new or changed module goes in the owning architecture map, a convention in `conventions.md`, a build or release fact in `build-release.md`), and `CONTRIBUTING.md`'s "Code layout" stays the contributor-facing overview with a pointer to the maps rather than a third copy of them.

**CI multi-checklist fix**

- R14. The checklist workflow verifies and renders exactly the top-level `docs/actions/*.checklist.json` files the pull request adds or modifies relative to the base branch tip: none means no comment and a green job, one means that one, several means one comment holding each document's own render in turn, a deleted document is excluded, a renamed document appears at its new path, a nested file under `docs/actions/<dir>/` is never selected, and a selection step that fails (no merge commit, a failed `git diff`) fails the job instead of reading as zero checklists.
- R15. `ss-magic-plugin checklist verify [FILE...]` and `checklist render-md [FILE...]` accept explicit paths; with no argument both behave exactly as today. Each path is refused (exit 2) when absolute, when it contains `..`, when it does not satisfy the same `matches_convention` predicate the checklist deny uses (so a nested path is refused), when `lstat` reports a symlink, or when its canonical form leaves the canonicalized repository root; the read uses the validated canonical path; an unknown `-`-prefixed token is an error, never a path; the pointer is never read or written on the explicit-path route.
- R16. `render-md --max-bytes <N>` bounds the whole posted body, marker included, to at most N bytes: a later document is cut at a document boundary, while a first document that alone exceeds the budget is truncated inside its own envelope (KTD6), and a fixed-text trailing marker names the omitted documents with their paths escaped and the list capped ("… and N more"); the workflow passes 60000 so the body stays under GitHub's 65,536-character comment limit.
- R17. The template keeps every existing security property (`pull_request` trigger, two-job split, `permissions: {}`, `persist-credentials: false`, the comment posted from a file, fork PRs skipped) and the two template tests' invariants: no `${{` inside a `run:` block and the literal `docs/actions/*.checklist.json` glob present; changed-file names reach the binary as separate argv elements read NUL-separated, never through a shell variable the checklist authored.
- R18. `setup-github-ci` classifies an untouched workflow written by any template generation ever released (the `v0.10.0`/`v0.11.0` shape keyed `SS_MAGIC_VERSION:` and the `ss-magic-plugin-v1.0.1` shape) as `pin-stale` and advances it without `--force`; the token stays `pin-stale`, the report names the generation found, a diff is printed for this state because the change is no longer only the pin, and a locally edited old workflow still classifies `differs`.
- R19. The upgrade path is documented: README's "Keeping the plugin current" says what a 1.1.0 upgrader does about an installed workflow, and the `setup-github-ci` skill's `pin-stale` branch reflects R18.

**Migration skill**

- R20. A skill directory `plugin/skills/migrate-repository/` ships `SKILL.md`, `reference.md` and `example.md`; the skill is invoked as `/ss-magic:migrate-repository` or by the operator asking, its description names the legacy-checklist migration and says it is not the `ss-magic` workspace migration, every command it names is spelled `ss-magic-plugin <verb>`, and no file in it contains `CLAUDE_PLUGIN_DATA`.
- R21. Preflight requires parseable JSON from `ss-magic-plugin status --json` (an empty stdout means the binary is not installed yet: stop and say so), a git repository, an attached HEAD, and a `.superset/magic.json`, then builds the read-only inventory (the bootstrap's seed diff, legacy checklist folders, the legacy skill, project rules, the CI workflow state). On the default branch the skill ends with that inventory report plus "create and switch to a branch, then re-run": it makes no commit there and never switches a branch itself. When the legacy skill directory is present in this worktree but absent from the default branch tip, another branch has already migrated the repository, and the skill stops with "merge the default branch first, then re-run". `ss-magic-plugin seed-config` runs only after an affirmative enable answer, and only when the `plugin` key is still absent.
- R22. Enabling is the operator's decision: the skill asks before any `enable`, offers committed, `--local`, or both, recommends both in a linked worktree with the reason stated (`enabled` is read from the main checkout, so a committed-only enable acts only after merge), re-reads `status --json` afterwards and branches on `enablement.ss_magic.enabled`. A decline ends in a read-only inventory report: it shows any seed diff the bootstrap already left, touches nothing itself, and adds no `.gitignore` rule by hand.
- R23. Conversion covers only the current branch's legacy checklist, through the existing verbs, with ids derived per KTD10. A conversion completes or is restarted, never resumed item by item: when an uncommitted document for the stem already exists (an interrupted run, or a legacy source edited since), the skill offers to discard it and convert again from the legacy file, and a committed document for the stem means this branch is already converted, so conversion is skipped. Mapping: the four default sections retitled positionally to the legacy file's top-level sections, legacy `###` areas carried as item-title prefixes, done state and dates preserved via `done` plus `set <id> completed`, increasing `created` stamps so source order holds within each done/priority group (the canonical sort is `(done, priority, created)`), `steps` set as a whole list, references kept only as absolute URLs. A failing `verify` shows its findings, is fixed once through `set`, and otherwise stops with a report; nothing is committed until `verify` is green, and nothing is pushed.
- R24. A legacy checklist over 80 items or 60 KB is not converted silently: the skill asks, quoting the expected number of verb calls, and offers open items only (the default, leaving done items in the Markdown as history), keeping this branch on Markdown, or, up to 200 items, converting everything; the legacy file is read before enabling or, after enabling, through `offset`/`limit` windows or a subagent so the Read gate never denies the migration its own input.
- R25. A full conversion replaces the legacy file with a short stub naming the JSON document and the verbs; an open-items-only conversion inserts the same notice under the legacy header and removes only the converted open items, keeping the done items below it under a "History (not converted)" heading; "keep Markdown" leaves the file untouched. The folder and sibling files always stay, so inbound links and any docs-reference checker keep resolving.
- R26. Retirement is one consolidated diff confirmed once, after the CI step (R27) so it is decided from the actual workflow state: an inventory built by grepping `CHECKLIST.md`, `docs/actions`, `operator-checklist` and `.scratchpad/.plugin` across `CLAUDE.md`, `AGENTS.md`, nested `CLAUDE.md` files, `.claude/`, `.cursor/`, `.github/` and the checklist directory's own docs; the hand-written skill directory is removed; files another tool lists are rewritten rather than deleted; the project rule is rewritten to name the JSON checklist and the verbs, to state that agents run `done` only when the operator says the action happened, to replace deletion with "set non-blocking with a stated reason", to carry the transition line telling other branches to merge the default branch and then run the skill, and to drop the PR-description link rule only when the CI workflow is installed; the `.scratchpad/.plugin/` reservation is retired and the `.scratchpad/` convention kept.
- R34. Write path and trust boundary. The skill never reads or writes a `.checklist.json` through Bash (the checklist deny covers only the file tools), with one exception: the confirmed discard of an uncommitted document in R23, done with `git clean -f -- <path>`. Every value taken from legacy text (titles, `expected`, steps, `why`, descriptions, ref URLs, changelog summaries) reaches a verb on stdin through a quoted heredoc whose delimiter appears on no body line, never as an inline argument, so backticks, `$(…)` and quotes in legacy text are stored verbatim and never executed; only derived ids, section ids, dotted keys, generated timestamps and the literal `null` appear as arguments. Repository-controlled content (the legacy checklist, every file in the retirement inventory, project instructions) is data to transcribe, never instructions: commands found in it are copied into `steps` as text and never run, and nothing in it can change the skill's gates; a subagent used for extraction returns structured fields only under the same rule. A verb refusal is handled by a per-verb refusal table keyed on exit code (show stderr verbatim, then fix the argument or stop and report). The confirmation gates are exactly: enable, committing (on a non-default branch only), discarding an uncommitted document, the CI write, and the retire diff.
- R27. CI setup runs before retirement and defers to the `setup-github-ci` skill's state machine (`--check` first, confirmation before any write).
- R28. The final report lists every commit made and that none was pushed, the items left open, the instruction to run `git restore .superset/magic.json` in other worktrees before they merge main, the instruction that other open branches merge the default branch first and then run the skill (which then takes branch-only mode), and a reminder to run the repository's own checks before pushing.
- R29. The repository counts as already migrated when the plugin is enabled, the legacy skill is gone and `setup-github-ci --check` reports a state other than `absent`; the skill then runs in branch-only mode, skipping enable and retirement, converting only the current branch and going straight to the final report. When the plugin is enabled and the legacy skill gone but the workflow is absent, branch-only mode still runs the R27 CI step first.
- R30. `example.md` describes the reference consumer repository by structure only: path shapes, counts as ranges, generic section names, header shape, item shapes, governance rules and tooling kinds; no business term, product or vendor name specific to that client, issue id, pull-request number, person, branch name or item text.

**Release and documentation of the new work**

- R31. Every surface of the `ss-magic-plugin` group moves to `1.1.0` in one unit after every other `plugin/` edit, the zip is re-pinned with `--update-manifest`, and `--check` plus `--check-bump ss-magic-plugin-v1.0.1` pass; `crates/ss-magic/Cargo.toml` and core stay as they are.
- R32. The new work is documented where readers look: README gains a skills inventory naming all four skills with the verbs each drives, `CONCEPTS.md` gains a "Repository migration" term disambiguated from the workspace contract's init/migrate, BUGBOT gains inline rules for the legacy-template list, the explicit-path verbs, the comment budget and the rule-file guards, and the architecture maps describe the changed `setup_ci.rs`, `checklist/verbs.rs` and template.
- R33. Units land in the order U1 → U8 with one commit each, so the split is a pure move, the sweep edits the new files, and the single bump covers every `plugin/` edit.

### Key Decisions

- KD1. **Restructure `CLAUDE.md` into an index plus `.claude/rules/*.md`** (session-settled: user-directed – chosen over fixing facts in place with no restructure: the file is larger than the plugin's own Read gate and loads whole into every session). Governs R8, R9, R10, R11, R12, R13.
- KD2. **The migration guide ships inside the plugin as a skill applied after installation** (session-settled: user-directed – chosen over a guide document committed in this repository: the guide must reach a repository that has installed the plugin, not someone reading this repo). Governs R20–R30, R34.
- KD3. **Unconditional rule files for hard rules, conventions and build/release; `paths:`-scoped files for per-crate module maps** (session-settled: user-approved – chosen over all-unconditional rule files: that would save no context; the accepted cost is that an answer given without opening a crate file does not see that crate's map). Governs R9, R12.
- KD4. **Fix the CI multi-checklist bug in this plan** (session-settled: user-approved – chosen over a skill-side manual archive workaround: any adopter hits the bug on its second merged checklist). Governs R14–R19.
- KD5. **Convert only the active branch's checklist via existing verbs; history and the standing checklist stay Markdown; retitle the four default sections; no import or add-section verb** (session-settled: user-approved – chosen over a bulk-import verb and converting history: a Markdown-to-JSON converter is heuristic against real files, and importing history is exactly what breaks CI). Governs R23, R24, R25.
- KD6. **A model-invoked skill started by the operator, asking before `enable`, with no SessionStart notice** (session-settled: user-approved – chosen over a SessionStart detection notice: hooks do not run in a not-yet-enabled repository, so a notice cannot fire where it is needed). Governs R20, R21, R22, R34.
- KD7. **Doc mismatches are fixed in the doc; stale code comments fixed without behavior change; behavior-level suspects listed, not fixed** (session-settled: user-approved). Governs R1, R3, R4.
- KD8. **Historical records are not rewritten** (session-settled: user-approved). Governs R5.
- KD9. **Retire the consumer's hand-written checklist skill, its "update before every commit" rule and the `.scratchpad/.plugin/` reservation; keep `.scratchpad/`** (session-settled: user-approved). Governs R26.
- KD10. **Plugin changes ship as one minor bump, 1.0.1 → 1.1.0** (session-settled: user-approved – a new skill and a CI behavior change are new user-visible behavior). Governs R31, R33.

### Scope Boundaries

#### Deferred to Follow-Up Work

- Porting the shim-token and two-argument checks from `scripts/test-bootstrap.sh` into `check_hooks_shim` (S1): a script-only change needing no plugin bump, listed under Code suspects for the user's call.
- Pinning or dropping the `releases/latest` install hint in `assets/magic.sh` (S2): changes CLI output, so it needs a CLI bump and is outside this plan's "CLI unchanged" boundary.
- Narrowing `ensure_entry` to `pub(crate)` (S10): an API visibility change in core, not a comment fix.
- The 24 pre-split `src/...` paths in `docs/solutions/**` "Where" sections: historical records under KD8.
- A `checklist import` or `add-section` verb (KD5), triggered by the first real migration whose conversion takes over an hour or by demand to convert a checklist over 200 items whole (R24).

#### Considered and not built

- A read-only `status` field reporting the CI workflow's state: an outdated workflow fails visibly on the next pull request ("holds several"), the README upgrade note (R19) and the migrate skill's CI step cover upgraders, and the field would add a status section, tests and three doc updates; build it if upgraders report not noticing the red check.
- A second diff against the legacy render ("your local changes") for a locally edited old workflow: the skill shows the full diff and asks before `--force`, and git history holds the overwritten file, so a wrong overwrite is recoverable at no cost; revisit on a report of a lost local edit.
- A hidden marker in the posted comment so a later run can find it: `--edit-last` edits the last comment by `github-actions[bot]`, so another workflow commenting as the same identity can overwrite the checklist comment or have its own overwritten, and a stale comment remains after a reverted change. Both are visible and recoverable through comment edit history, and neither is new; build the marker on the first report of a comment overwritten across workflows.
- Handle-based or `O_NOFOLLOW` reading on the explicit-path route, and hardening the no-argument route against a symlinked checklist: CI no longer uses the no-argument route, the explicit route reads the validated canonical path, and the only local actor is the operator.
- Lifting every imperative in the path-scoped maps into the unconditional set: the binding ones (bootstrap stdout silence, the shim manifest, `--version` line shape, hook-reachable verbs) are enforced by `test-bootstrap.sh`, `--check` and `no_hook_invoked_verb_can_set_enabled`; the one untested posture rule is lifted into the index (KTD3).
- Ids that survive an edit of the legacy source between runs, or a persisted id map: every conversion starts from an empty document (R23), so ids only need to be deterministic within one run.
- Resuming an interrupted conversion item by item: a resume must re-derive identical ids across sessions, unescape the ids `render-md` prints, skip existing changelog entries and detect a changed source, and every one of those is a place a model-executed run goes wrong; the document is uncommitted until `verify` passes, so a restart costs only time (R23). Build it if operators report reconversion time on large checklists as a problem.
- Running the consumer repository's own check and test commands from the skill: the retire diff is reviewed once, the stub and folder keep inbound links resolving (R25), and a dead reference surfaces in the repository's normal CI; the final report reminds the operator to run the checks before pushing (R28).
- An automated `claude -p` evaluation harness for the skill: the conversion is model-executed and every write is confirmed by the operator or checked by `verify`; the manual rehearsal in the Verification Contract covers the flows. Build it on the first report of a wrong conversion or retirement.
- A panic-free path for non-UTF-8 file names passed to the verbs: `std::env::args()` panics, exit 101 fails the CI job closed, and git-selected names under `docs/actions/` are ASCII in practice.
- `disable-model-invocation` on the new skill: the operator starts the migration by asking, which is a model invocation, so the flag would block the intended path; a narrow description does the scoping (KTD8).
- A SessionStart line pointing at the skill once a repository is enabled: it spends the model-facing guidance budget every session to reach a repository that has, by then, already acted.

### Code suspects for the user

Listed per KD7; none is changed by this plan.

| Id | Where | What | Suggested call |
|---|---|---|---|
| S1 | `scripts/build-plugin-zip.py` `check_hooks_shim` | Does not assert the shim event token, the two-argument shape or that the target exists; only `test-bootstrap.sh` does | Port the checks; BUGBOT is reworded now to say which half each gate covers (B6) |
| S2 | `assets/magic.sh` | Install hint uses the unpinned `releases/latest/download/` URL that can 404 after a plugin release | Template the writing binary's tag or drop the line; needs a CLI bump |
| S10 | `crates/ss-magic-core/src/git/gitignore.rs` | `ensure_entry` is `pub` with one in-module caller | `pub(crate)` in a later change |
| S11 | `crates/ss-magic-core/src/git/mod.rs` | `nothing_to_commit`, `gh_available`, `pr_create` spawn `Command` directly | Resolved as a doc fix (C1): the rule is relaxed to match the code |

### Acceptance Examples

- AE1. **Covers R14.** Given a PR that modifies `docs/actions/2026-10-a.checklist.json` while `docs/actions/2026-08-b.checklist.json` is unchanged on both sides, when the workflow runs, then `verify` and `render-md` receive exactly `docs/actions/2026-10-a.checklist.json` and the comment shows that document alone.
- AE2. **Covers R14, R16.** Given a PR that adds two checklists whose renders together exceed 60,000 bytes, when the body is rendered with `--max-bytes 60000`, then the first renders whole, the second is omitted entirely, and the body ends with a marker naming the omitted file.
- AE3. **Covers R15.** Given `checklist verify docs/actions/x.checklist.json` where that path is a symlink to a file outside the repository, when the verb runs, then it refuses with a message naming the symlink and exits 2, and nothing is read through the link.
- AE4. **Covers R18.** Given a repository whose `.github/workflows/ss-magic-checklist.yml` is byte-identical to the 1.0.1 template rendered at `1.0.1`, when `setup-github-ci --check` runs under 1.1.0, then the first line is `state: pin-stale`, the report names the earlier template generation, a diff is printed, and a run without `--check` writes the new workflow.
- AE5. **Covers R18.** Given the same file with `runs-on: self-hosted` substituted, when `--check` runs, then the state is `differs` and a write without `--force` is refused.
- AE6. **Covers R22.** Given a linked worktree where the operator chooses "committed only", when the skill re-reads `status --json`, then `enablement.ss_magic.enabled` is still `false` and the skill says hooks start after the commit reaches the main checkout, offering `--local` for this machine.
- AE7. **Covers R23.** Given an uncommitted document left by an interrupted run, when the skill is re-run on the same branch, then it offers to discard and reconvert, and on yes produces the same document a fresh run produces and `verify` passes; given a committed document for the stem, conversion is skipped.
- AE9. **Covers R34.** Given a legacy item whose title contains a backtick code span holding a command and an apostrophe, when it is converted, then the title is stored verbatim and nothing in it runs.
- AE8. **Covers R22, R21.** Given the operator declines enable, when the skill ends, then `git status --porcelain` is identical before and after the run.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Rule-file set and globs.** `.claude/rules/hard-rules.md`, `conventions.md` and `build-release.md` carry no frontmatter and load every session. `architecture-core.md` (`crates/ss-magic-core/**`), `architecture-cli.md` (`crates/ss-magic/**`), `architecture-plugin.md` (`crates/ss-magic-plugin/**`, `assets/workflow/**`) and `plugin-assets.md` (`plugin/**`, `scripts/**`, `.claude-plugin/**`, `.github/**`, `dist-workspace.toml`, `docs/runbooks/**`) carry `paths:` lists. Flat directory, one topic per file, names that read as what they hold. Inherits KD1 and KD3; governs R8, R9.
- KTD2. **The split is a pure move that lands first, then the sweep edits the new files.** U1 moves text by section or by module bullet without rewording, which makes "nothing was lost or paraphrased" a mechanical check against `7f6c62d:CLAUDE.md`, and the hard-rules proof (R10) is one `diff`. The sweep then has smaller files to edit and locates each finding by its quoted claim rather than by a `CLAUDE.md` line number. Chosen over sweeping first: a sweep-then-move would make the move's review a diff of a diff.
- KTD3. **The index mitigates Bash-based reads and answers given without reading.** Path-scoped rules load only on Read, Write or Edit, and this environment routes many reads through `cat`/`sed`; so the index itself lists every rule file with its globs and a "read this before touching X" line, carries a short summary per crate, and states the cross-crate invariants (`STATE_REL` equals the `.superset/.magic` entry of `EXCLUDED_TREES`; `git::discover` is wired into the plugin crate only; no hook reaches a verb that can set `plugin.enabled`; the two binaries never depend on each other). The unconditional set holds every rule no test enforces; the maps carry module descriptions plus contracts that tests enforce (bootstrap stdout silence and the shim manifest by `test-bootstrap.sh` and `--check`, hook-reachable verbs by `no_hook_invoked_verb_can_set_enabled`). The one untested posture rule, "skills and model-facing text spell `ss-magic-plugin <verb>`; nobody types a verb in a terminal", is added to the index's invariants. Always-loaded budget: 50,000 bytes, enforced by the guard script (measured: 27,175 bytes of moved unconditional text plus about 8 KB of index, about 35 KB).
- KTD4. **Document guards move into `scripts/check-docs.sh`, called by CI and runnable locally.** It carries the three greps now inline in `ci.yml` (retired spelling over the extended list including `.claude/rules/`, no `CLAUDE_PLUGIN_DATA` under `plugin/skills/`, no `releases/latest/download/` in README) plus the four new checks (BUGBOT has no `](` and names no individual `.claude/rules/<name>.md` file, relative links resolve, rule frontmatter is absent or a well-formed `paths:` list, and the always-loaded byte budget over `CLAUDE.md` plus every `.claude/rules/**/*.md` without `paths:`, derived so a new file cannot escape it), each printing one `ok`/`FAIL` line, with a `--selftest` driven through `scripts/lib/test-harness.sh` on bash 3.2. It runs where the three greps ran, in the release-gating `plugin` job, so the new link and budget checks become release-blocking too; `--selftest` and the guards also run under `/bin/bash` on the macOS leg beside the other two bash 3.2 suites, which is what proves the BSD-grep and bash-3.2 claim. Chosen over growing `ci.yml` inline: the greps were CI-only, and a guard nobody can run locally is one nobody runs before pushing. `docs/plans/` stays out of every guard's scope.
- KTD5. **Changed-checklist selection by diffing the merge commit against the base tip, failing closed.** The `render` job checks out the default `refs/pull/N/merge` with `fetch-depth: 2`, asserts the shape with `git rev-parse --verify HEAD^2` (a non-merge HEAD fails the job), and runs `git diff --name-only -z --no-renames --diff-filter=AM HEAD^1 HEAD -- ':(glob)docs/actions/*.checklist.json'` as its own command writing to a file under `$RUNNER_TEMP`, so `set -e` sees its exit status; each later step that needs the names (verify, render) re-reads that file with `while IFS= read -r -d ''` into a bash array, because every Actions step is a fresh shell and an array from an earlier step would expand to nothing under `set -u`, silently falling back to the zero-argument route; a step whose array comes out empty while `present` is true fails rather than calling a verb with no paths. Reading through `< <(git diff …)` is forbidden because a process-substitution failure is invisible to `set -e` and would read as zero checklists, silently turning the gate off. `HEAD^1` is the base branch tip, so changes that arrived by merging main into the branch are excluded; `--no-renames` makes a renamed document appear as added at its new path; the `:(glob)` magic stops `*` matching `/`, so nested files are not selected, and the pathspec still contains the literal `docs/actions/*.checklist.json` the template test asserts; no `${{` appears in any `run:` block. Every selected name starts with `docs/actions/`, so none can be read as a flag, which is why no `--` separator is passed. `present` becomes "at least one name". Chosen over `base...head` (needs the base ref and history the default checkout lacks) and over passing a base SHA (needs an `env:` interpolation).
- KTD6. **Explicit paths on `verify` and `render-md`, multi-document rendering, and a byte budget on `render-md`.** Both verbs take zero or more `FILE` arguments; zero keeps today's `resolve_active` route untouched. Validation per R15 happens in one helper shared by both verbs (lexical refusal via the existing `contained_join` shape, the `matches_convention` predicate, `symlink_metadata` refusal, canonical containment against the canonicalized root as `config.rs`'s `landing` does) and returns the canonical path the read then uses. `render-md` with several files emits each document's existing `render()` output in argument order joined by fixed separator text, adding no raw heading of its own, because `render()` already wraps every repository-authored string in one `cache::envelope` and emits the escaped title inside it. `--max-bytes <N>` (default unbounded) bounds the whole body, marker included: when the first selected document alone does not fit, it is rendered through `render()` with `Budget::Bytes` set to what remains after the marker reserve, so `cache::envelope` truncates inside the envelope with its own "the whole text is at …" notice and the operator still sees the document's head; otherwise it stops before a document that would cross the budget and appends fixed marker text listing omitted paths escaped through `prose_inline`, capped with "… and N more" so the marker itself cannot overflow. Chosen over one path per call with shell concatenation: the budget must see every document to bound the sum, and a Rust loop is testable where a shell loop is not. The existing `Budget`/`envelope` machinery stays for `list`; the new budget is a document-level cut, so no envelope is split mid-text.
- KTD7. **Legacy templates are embedded fixtures with their own placeholder and pin key.** `assets/workflow/legacy/checklist-0.11.0.yml` (byte-identical to `v0.10.0`/`v0.11.0`, keyed `SS_MAGIC_VERSION:` with placeholder `@SS_MAGIC_VERSION@`) and `assets/workflow/legacy/checklist-1.0.1.yml` (byte-identical to `v0.11.1` through `ss-magic-plugin-v1.0.1`) are `include_str!`-ed into a `LEGACY_TEMPLATES` table of `(generation label, body, pin key, placeholder)`. `classify` tries the current template first, then each legacy generation: a legacy body rendered at the version its own key names reproducing the file is `PinStale` regardless of whether that version equals the current one. `pinned_version` learns the legacy key. The `pin-stale` token and `State::token()` stay; `State::PinStale` gains the generation so `report_state` can name it and `print_diff` runs for it. Provenance of each fixture is recorded in a comment and in the Verification Contract. Follows `docs/solutions/logic-errors/retention-must-know-every-layout-ever-written.md`.
- KTD8. **The skill keeps the name `migrate-repository`.** It is the user's vocabulary, the `/ss-magic:` namespace already separates it from the CLI's TUI menu entry, and the collision is one of words, which the description and the `CONCEPTS.md` term resolve. Chosen over `adopt-repository`: a different word would make the skill harder to find for the person who asked for a "migration guide". No `disable-model-invocation`: the operator asks, the model invokes (KD6); the description's "Use when / Not for" clause is what keeps it from firing in unrelated repositories.
- KTD9. **The skill orchestrates existing verbs and ordinary git and file tools; it ships no helper script and adds no verb.** A `.sh` beside the skill cannot be addressed from the Bash tool (no exported plugin-root variable), and every write the skill needs already has a verb or is a plain file edit the model performs. The skill's only state is the repository and the conversation; an interrupted conversion is restarted, not resumed (R23), because the document stays uncommitted until `verify` passes, so a restart loses nothing but time and needs none of the id-matching a resume would.
- KTD10. **Conversion mapping rules.** Ids: one normalization, total over any input, so every derived id passes `is_well_formed_id`: ASCII-fold, lowercase, map every other character to `-`, collapse runs of `-`, trim leading and trailing `-`, keep the first six words, use `item` when nothing is left, and prefix `i-` when the first character is not a letter. Because a conversion always starts from an empty document (R23), collisions are resolved within that one run against every id `check_new_id` refuses (ids already assigned in this run, the four section ids, changelog entry ids and the reserved `document`), suffixed `-2`, `-3` in source order. Changelog entry ids follow a fixed scheme: `migrated-from-legacy` for the first entry (which records the legacy stem and the R24 mode) and `note-<item-id>[-n]` for a dated annotation. Sections: the four default ids are retitled positionally (`verification`, `rollout`, `decisions`, `follow-ups` in render order) to the legacy file's first four top-level sections in source order; further legacy sections fold into the nearest phase with their heading as an item-title prefix; `###` areas become `<Area>: ` title prefixes. Items: title from the clause before the first ` – ` or bold pass marker, `expected` from the clause after it, `steps` from imperative sentences and commands (at least one, else the action sentence itself), `why` from a trailing parenthetical, `kind` `decision` for decision-marked rows, `record` for tick-less fact rows, `priority` `blocking` for rows carrying the legacy priority marker and `decision-blocking` for decision rows. Done rows: `done <id>` then `set <id> completed <date>T12:00:00<local offset>` when a date is in the prose. Order: `set <id> created <base + n seconds>` where `base` is the legacy file's first-commit time from `git log --diff-filter=A --format=%aI`. Dated annotations become changelog entries naming the item id. Inline links become `refs` only when absolute or resolvable against `https://<host>/<owner>/<repo>/blob/<default-branch>/`, a base built from the `origin` remote with userinfo, port and any `.git` suffix removed and ssh/scp forms rewritten to https (the normalization `reponame::stem_from_origin` already applies for naming); when the remote URL carries userinfo the skill tells the operator and keeps relative links in prose, so no credential reaches a committed document or a PR comment; everything else stays in prose. The slug passed to `init` is the whole legacy stem so its `YYYY-MM` survives `split_date_prefix`; the part after `YYYY-MM-` goes through the same normalization as ids (branch-derived folder names can carry uppercase, `_` or `.`), with `b-` prefixed when it does not start with a letter, and the original recorded in `migrated-from-legacy`.
- KTD11. **Enable recommendation in a worktree is committed plus `--local`.** `enabled` is read from the main checkout's overlay while a committed `enable` writes the current checkout, so committed-only leaves hooks off until merge; `--local` writes the main checkout's `magic.local.json` and acts at once in every worktree of this repository on this machine, including branches not yet converted, where whole Reads of large legacy checklists are then routed through the size gate. Both together act now and for teammates after merge. The skill states this in two sentences and lets the operator pick.
- KTD12. **Thresholds.** Large: more than 80 items or more than 60 KB triggers the R24 question, which quotes the expected cost (about six verb calls per item, each a locked whole-document rewrite); "convert everything" is offered only up to 200 items, because past that the call count runs into the thousands and the deferred import verb is the right tool. Read gate: the legacy file is read before `enable` when possible; after enabling, a file over the configured threshold (default 120,000 bytes) is read in `offset`/`limit` windows of at most 1,500 lines or extracted by a subagent whose own reads are not gated, optionally declared with `expect-artifact`; `bypass` is not used because it hands the whole file to the main context.
- KTD13. **Markdown normalisation.** Files the sweep rewrites also get their prose em dashes replaced with en dashes and bare fences tagged `plaintext`, except any dash that quotes a code string verbatim (the cockpit's own messages use an em dash, drift-findings B10) and except `hard-rules.md`, which receives only the three edits R10 lists (its single prose em dash is one of them). Files not otherwise edited are not normalised.
- KTD14. **`.claude/settings.json` stays tracked and `.claude/rules/` is tracked.** The old simplify-plan constraint "never commit `.claude/settings.json`" is stale (the file is tracked today); this plan records the current rule in `conventions.md`: `.claude/settings.json` and `.claude/rules/` are committed, `.claude/settings.local.json` and `.claude/skills/` are not.
- KTD15. **CONTRIBUTING keeps "Code layout" and "Tests" as the contributor overview.** They are shorter restatements for a different audience; U3 adds one sentence pointing at the architecture maps and removes nothing, so no fact is maintained in three places beyond the crate list.

### High-Level Technical Design

Rule-file layout and what loads when:

```mermaid
flowchart TB
  subgraph always["Loaded every session"]
    idx["CLAUDE.md – index: crate layout, rule table, summaries, invariants"]
    hr[".claude/rules/hard-rules.md – two hard-rules sections, verbatim"]
    cv[".claude/rules/conventions.md – conventions, magic.sh source, docs-sync rule"]
    br[".claude/rules/build-release.md – build, two release lines, version surfaces"]
  end
  subgraph scoped["Loaded on Read/Write/Edit of a matching file"]
    ac["architecture-core.md – paths: crates/ss-magic-core/**"]
    acli["architecture-cli.md – paths: crates/ss-magic/**"]
    ap["architecture-plugin.md – paths: crates/ss-magic-plugin/**, assets/workflow/**"]
    pa["plugin-assets.md – paths: plugin/**, scripts/**, .claude-plugin/**, .github/**, dist-workspace.toml, docs/runbooks/**"]
  end
  subgraph guards["CI guards – scripts/check-docs.sh"]
    g1["retired spelling grep incl. .claude/rules/"]
    g2["BUGBOT link-free and rule-path-free"]
    g3["relative links resolve"]
    g4["frontmatter shape and always-loaded byte budget"]
  end
  idx --> ac
  idx --> acli
  idx --> ap
  idx --> pa
  always --> guards
  scoped --> guards
```

Checklist selection in the fixed workflow:

```mermaid
flowchart TB
  subgraph render["render job – contents: read"]
    co["checkout refs/pull/N/merge, fetch-depth 2, persist-credentials false"]
    mg{"git rev-parse --verify HEAD^2 succeeds?"}
    fail["job fails – never read as zero checklists"]
    df["git diff --name-only -z --no-renames --diff-filter=AM HEAD^1 HEAD -- ':(glob)docs/actions/*.checklist.json' into a file; its own exit status checked"]
    n{"names found?"}
    skip["present=false – nothing to render"]
    inst["install the pinned ss-magic-plugin, sha256 verified"]
    vf["checklist verify NAME... – any error fails the PR"]
    rm["checklist render-md --max-bytes 60000 NAME... > checklist.md"]
    up["upload artifact"]
  end
  subgraph comment["comment job – pull-requests: write, no checkout"]
    fork{"same-repo PR and present?"}
    post["gh pr comment --body-file --edit-last --create-if-none"]
    none["skipped"]
  end
  co --> mg
  mg -- no --> fail
  mg -- yes --> df
  df -- "git diff fails" --> fail
  df --> n
  n -- "0" --> skip
  n -- "1 or more" --> inst --> vf --> rm --> up --> fork
  fork -- yes --> post
  fork -- no --> none
```

`setup-github-ci` classification after KTD7:

```mermaid
flowchart TB
  f{"workflow file present?"}
  a["absent"]
  cur{"equals current template at this version?"}
  id["identical"]
  curpin{"equals current template at the version the file pins?"}
  ps["pin-stale – current generation"]
  leg{"equals a legacy generation rendered at the version its own key names?"}
  psl["pin-stale – legacy generation, diff printed"]
  d["differs – --force required"]
  f -- no --> a
  f -- yes --> cur
  cur -- yes --> id
  cur -- no --> curpin
  curpin -- yes --> ps
  curpin -- no --> leg
  leg -- yes --> psl
  leg -- no --> d
```

The migrate-repository skill's decision flow:

```mermaid
flowchart TB
  subgraph pre["Preflight – read only"]
    s0["operator asks / invokes /ss-magic:migrate-repository"]
    s1{"status --json parses?"}
    s1x["stop: binary not installed yet; retry after bootstrap or a new session"]
    s2{"git repo, attached HEAD, magic.json?"}
    s2x["stop and explain; for a missing magic.json suggest ss-magic init"]
    s5["inventory: bootstrap seed diff, legacy dirs, legacy skill, project rules, CI state"]
    s5m{"legacy skill here but gone from the default branch tip?"}
    s5mx["stop: merge the default branch first, then re-run"]
    s3{"on the default branch?"}
    s3b["inventory report + create and switch to a branch, then re-run; no commit"]
    s6{"already migrated? enabled, legacy skill gone, workflow not absent"}
  end
  subgraph en["Enablement"]
    e1{"enabled?"}
    e2{"ask: committed / --local / both / no"}
    e2x["inventory report showing any seed diff; nothing written"]
    e3["seed-config if plugin key absent; run enable (and/or --local); re-read status --json; commit seed + enable + .gitignore"]
  end
  subgraph conv["Conversion – current branch only"]
    c1{"legacy checklist for this branch?"}
    c1m["several matches: ask which"]
    c2{"document for the stem exists?"}
    c2c["committed: already converted, skip"]
    c2u{"uncommitted: discard and reconvert?"}
    c4{"over 80 items or 60 KB?"}
    c4a["ask, quoting the call count: open items only (default) / keep Markdown / convert all (up to 200 items)"]
    c5["init stem; retitle sections; add items per KTD10, legacy text on stdin only"]
    c6{"verify green?"}
    c6f["show findings; fix once through set; re-verify"]
    c6x["stop and report; nothing committed"]
    c7["stub or annotate the legacy file per R25; commit"]
  end
  subgraph fin["CI, retirement, report"]
    r2["setup-github-ci skill state machine"]
    r1["consolidated retire diff decided from the CI state; one confirmation; commit"]
    r3["final report"]
  end
  s0 --> s1
  s1 -- no --> s1x
  s1 -- yes --> s2
  s2 -- no --> s2x
  s2 -- yes --> s5 --> s5m
  s5m -- yes --> s5mx
  s5m -- no --> s3
  s3 -- yes --> s3b
  s3 -- no --> s6
  s6 -- "yes: branch-only mode" --> c1
  s6 -- "enabled, skill gone, workflow absent" --> c1
  s6 -- no --> e1
  e1 -- yes --> c1
  e1 -- no --> e2
  e2 -- no --> e2x
  e2 -- "any enable" --> e3 --> c1
  c1 -- several --> c1m --> c2
  c1 -- one --> c2
  c1 -- none --> r2
  c2 -- committed --> c2c --> r2
  c2 -- uncommitted --> c2u
  c2u -- no --> c6x
  c2u -- "yes: git clean the file" --> c4
  c2 -- none --> c4
  c4 -- yes --> c4a --> c5
  c4 -- no --> c5
  c5 --> c6
  c6 -- "no, first time" --> c6f --> c6
  c6 -- "no, after the fix" --> c6x
  c6 -- yes --> c7 --> r2
  r2 -- "branch-only mode" --> r3
  r2 -- "full migration" --> r1 --> r3
```

In branch-only mode `r2` runs only when the workflow is absent; otherwise the run passes straight from conversion to the final report.

### Implementation Constraints

- Work on `ViktorStiskala/docs-update`; never switch branches; one commit per unit; nothing is tagged or pushed by this plan.
- Every `plugin/` edit is followed by `python3 scripts/build-plugin-zip.py --update-manifest` then `--check`; the version bump is U8 and the only one; `--check` enumerates the surfaces and this plan's list in R31 is not a substitute for running it.
- Skills spell `ss-magic-plugin <verb>`, never bare `ss-magic` as a command and never `ss-magic plugin <verb>`; no skill file contains `CLAUDE_PLUGIN_DATA`; skill links are `./`-relative within the skill directory; frontmatter is `name` plus a folded `description`; ASCII file names only, no symlinks under `plugin/`.
- The CI template tests `no_run_step_interpolates_an_expression` and `the_workflow_globs_the_checklist_convention` remain green and unmodified.
- Comments explain the rule, not only an id: a plan or requirement id may stay, with one clause saying what it means.
- Test files follow the sibling `<module>/tests.rs` layout; no test embeds a literal version or date where `CARGO_PKG_VERSION` or a derived value serves (lesson in `docs/solutions/logic-errors/cargo-dist-plan-job-failure-publishes-empty-release.md`).
- Nothing from the private reference consumer repository is copied into any committed file; `example.md` is written from the structural description in U6 and nothing else.
- The retired spelling `ss-magic plugin ` must not reappear in any guarded file, including the new rule files and skill.

### Sequencing

1. U1 – split `CLAUDE.md` (pure move) and land the document guards in the same commit, so the rule files are never unguarded.
2. U2 – sweep the rule files, the index and the code comments.
3. U3 – sweep README, CONTRIBUTING, CONCEPTS, BUGBOT and the runbook.
4. U4 – explicit-path verbs and the render budget (binary only).
5. U5 – workflow template rewrite and legacy-template classification (binary plus `assets/`).
6. U6 – the `migrate-repository` skill.
7. U7 – documentation of the new work across README, CONCEPTS, BUGBOT, the architecture maps and the shipped skills, including their drift fix.
8. U8 – plugin 1.1.0 on every surface, re-pin, full verification.

The branch is not pushed between U6 and U8: CI's "Content change requires a version bump" step runs `--check-bump` on every push, and from U6 the `plugin/` digest has moved while the version is still 1.0.1, so the first push after U1 is the U8 head. U8 is last because `--check-bump` compares the `plugin/` digest against `ss-magic-plugin-v1.0.1` and every `plugin/` edit (U6, and U7's skill edits including the `reference.md` section-set fix) must precede the one bump.

### Risks

- A move that silently drops a line of `CLAUDE.md`: mitigated by the multiset move diff in the Verification Contract, which lists every original line absent from the new files and every line duplicated into two of them.
- A legacy fixture that is not byte-identical to the released template makes every 1.0.1 workflow read as `differs`: mitigated by extracting the fixtures with `git show <tag>:assets/workflow/checklist.yml` and by the provenance diff in the Verification Contract.
- A selection step that fails without failing the job would turn the verify gate off for every PR: mitigated by KTD5 (the `HEAD^2` assertion, `git diff` as its own command) and pinned by the U5 template test and the "CI selection fails closed" check.
- `fetch-depth: 2` on a PR whose merge ref cannot be created (conflicting PR) fails at checkout: unchanged from today, where checkout fails the same way, and the failure names the cause.
- The skill is run in a repository with several worktrees holding the unstaged seed diff: covered by the final report's `git restore` instruction (R28), not by the skill touching other worktrees.
- Context cost of the architecture maps when an agent touches all three crates: accepted under KD3; the maps load only when their files are touched.

---

## Implementation Units

### U1. Split CLAUDE.md into an index plus rule files, with guards

**Goal:** `CLAUDE.md` becomes the index; every section moves byte-for-byte into its rule file; the document guards cover the new files in the same commit.

**Requirements:** R8, R9, R10, R11, R12, R13 (the convention text itself is rewritten in U2), R33.

**Dependencies:** none.

**Files:** `CLAUDE.md`; create `.claude/rules/hard-rules.md`, `.claude/rules/conventions.md`, `.claude/rules/build-release.md`, `.claude/rules/architecture-core.md`, `.claude/rules/architecture-cli.md`, `.claude/rules/architecture-plugin.md`, `.claude/rules/plugin-assets.md`; create `scripts/check-docs.sh`; `.github/workflows/ci.yml`; `CONTRIBUTING.md` (the CI description paragraph only); `.cursor/BUGBOT.md` (the CI grep list sentence and the "contributor-instructions file" wording only).

**Approach:**

1. Move sections per the Appendix mapping table, keeping every moved line byte-identical; the only text U1 authors is the new index body, the `paths:` frontmatter blocks, and the re-based link targets (`./docs/` → `../../docs/` in the five links at `CLAUDE.md` 1621, 1667, 1687, 1703, 1710 at `7f6c62d`).
2. Split the interleaved module bullets (lines 181–697) by owning crate: core modules to `architecture-core.md`, CLI modules to `architecture-cli.md`; the `update/` bullet stays in the CLI map and the `release.rs` sentence inside it is left as written for U2 to tidy.
3. Write the index per R8 and KTD3; keep the crate-layout mermaid; list the rule files in a table with globs and one line each.
4. Write `scripts/check-docs.sh` per KTD4 (bash 3.2, sourcing `scripts/lib/test-harness.sh` for `--selftest`); in `ci.yml`, replace the three inline grep steps of the `plugin` job with one "Documentation guards" step running it, and add the guards plus `--selftest` under `/bin/bash` to the macOS leg next to the other two bash 3.2 suites; keep the step comments' reasoning inside the script.
5. Update the `CONTRIBUTING.md` CI paragraph and the two BUGBOT sentences so they describe the guards as they now are, with BUGBOT still naming no rule-file path (it may say "the contributor-instructions file and the rule files it indexes").

**Patterns to follow:** `scripts/test-mark-latest.sh` and `scripts/lib/test-harness.sh` for the shell style; the `ok  `/`FAIL` line shape of `build-plugin-zip.py --check`; rule-file frontmatter shape from `rules-mechanism` (a YAML list under `paths:`).

**Test scenarios** (`scripts/check-docs.sh --selftest`, against a fixture tree in a tempdir):

- A fixture file containing `ss-magic plugin status` under `.claude/rules/` fails the spelling guard; the same text under `docs/plans/` passes.
- A BUGBOT fixture containing `](./x.md)` fails the link-free guard; one containing `.claude/rules/foo.md` fails the rule-path guard; one naming the `.claude/rules/` directory as a rule's subject passes; a plain restatement passes.
- A new rule file without frontmatter is counted in the always-loaded budget without being named anywhere; one nested in a subdirectory is counted too.
- A relative link to a missing file fails the link guard; a link with a `#anchor` to an existing file passes.
- A rule file whose frontmatter opens with `---` but has no `paths:` list fails the frontmatter guard; a file with no frontmatter passes; a well-formed `paths:` list passes.
- An always-loaded set one byte over the budget fails; one at the budget passes.
- Running the script against the real repository prints only `ok` lines and exits 0.

**Verification:** the hard-rules diff prints nothing and the multiset move diff prints only the index's authored lines, the frontmatter and the retired header lines; `/context` in a fresh session lists `CLAUDE.md` and the three unconditional rule files only; a Read of `crates/ss-magic-plugin/src/main.rs` makes `architecture-plugin.md` appear; `scripts/check-docs.sh` passes locally and in CI.

### U2. Sweep the rule files, the index and the code comments

**Goal:** every `CLAUDE.md`-derived fact and passage in the drift findings is fixed in its new file, and the ten stale code comments are corrected without behavior change.

**Requirements:** R1 (C1–C26), R2 (the 44 CLAUDE.md passages), R3, R6 (the moved BUGBOT-mirrored facts are handled in U3), R7, R13.

**Dependencies:** U1.

**Files:** `CLAUDE.md`, `.claude/rules/*.md`; `crates/ss-magic-plugin/src/hook/mod.rs` (S3, S4), `crates/ss-magic-plugin/src/config.rs` (S5), `crates/ss-magic-plugin/src/checklist/validate.rs` (S6), `crates/ss-magic-plugin/src/checklist/render.rs` (S7), `crates/ss-magic/src/update/mod.rs` (S8), `crates/ss-magic/src/cli.rs` (S9), `crates/ss-magic/src/sync/merge.rs` and `crates/ss-magic/src/tui/cockpit.rs` (S12), `crates/ss-magic/src/main.rs` (S13), `crates/ss-magic-plugin/src/status.rs` (S14).

**Approach:**

1. Apply C1–C26 in the owning rule file, locating each by its quoted claim; `hard-rules.md` receives exactly the three edits R10 lists.
2. Rewrite the twelve historical-narration groups as present-tense fact, keeping each bracketed fact; group 12's docs-sync convention is rewritten per R13 and KTD14 in `conventions.md`.
3. Move the description of core's `release.rs` (`Line`, `parse_line_tag`, `Cache.suggested`, `refresh_cache`, now inside the CLI map's `update/` bullet) into a new `release.rs` bullet in `architecture-core.md`, leaving a one-line pointer in the `update/` bullet; add short bullets for the core files the old text never described separately (`reponame.rs`, `state_tree.rs`, `testutil.rs`) so every file in `crates/ss-magic-core/src/` has a home in the core map.
4. Repoint positional cross-references that now cross files: grep the moved text for `below`, `above`, `see <Section>`, `in Architecture` and `this doc`, and name the target rule file in each hit (known hits at `7f6c62d`: lines 174, 874, 1468, 1626, 1639).
5. Normalise prose em dashes and bare fences in every file this unit edits (KTD13).
6. Fix the ten code comments in place; a comment that cites a plan id keeps the id and gains one clause of meaning.
7. Re-run `scripts/check-docs.sh` and `cargo test --workspace --locked`; the CLI version stays `0.11.2`.

**Patterns to follow:** the workspace-split plan's R34 method, "read each file end to end and rewrite the stale passages rather than appending"; comment style from the user's rule (explain, do not only cite).

**Test expectation:** none – comment-only Rust changes and documentation; verified by the full test suite staying green and the greps below.

**Verification:** `grep -rn 'U13\|used to\|no longer\|formerly\|moved wholesale' CLAUDE.md .claude/rules/` returns only lines the findings keep by design; `grep -rnE 'below|above|in Architecture|this doc' CLAUDE.md .claude/rules/` returns no cross-file positional reference; `grep -c '—' .claude/rules/*.md CLAUDE.md` is 0 outside quoted code strings; `cargo test --workspace --locked` green; `git diff --stat` shows no `.rs` hunk outside comment lines.

### U3. Sweep README, CONTRIBUTING, CONCEPTS, BUGBOT and the runbook

**Goal:** the five remaining current-state documents describe the code as it is; the shipped skills' drift is fixed in U7 so every `plugin/` edit lands between U6 and U8.

**Requirements:** R1 (the drift findings' README items R1–R11, CONTRIBUTING items N1–N6, CONCEPTS items K1–K5, BUGBOT items B1–B13 and runbook item F1), R2 (the 26 remaining passages), R6, R7.

**Dependencies:** U2 (so BUGBOT mirrors the already-corrected conventions).

**Files:** `README.md`, `CONTRIBUTING.md`, `CONCEPTS.md`, `.cursor/BUGBOT.md`, `docs/runbooks/forge-tag-and-release-protection.md`.

**Approach:**

1. README: apply the findings' README items R1–R11, keeping the headings that in-README anchors point at (`#the-claude-code-plugin`, `#self-update`, `#the-superset-contract`, `#pattern-semantics`) and exactly one `releases/download/v0.11.2/ss-magic-installer.sh` line for the `--check` README surface; the skills inventory itself is U7's.
2. CONTRIBUTING: N1–N6 plus the cross-referenced C11 (`--check-bump` as a runnable command) and R7; add the one-sentence pointer from "Code layout" to the architecture maps (KTD15).
3. CONCEPTS: K1–K5 and the cross-referenced C7, C9, C15.
4. BUGBOT: B1–B13 and the cross-referenced C1 (relaxed git/gh rule per S11), C2, C9, C15, C11, K1, N3; the 15 narration passages; B6 reworded to say which half each gate asserts.
5. Runbook: F1 and its three passages.
6. Normalise em dashes and fences in the edited files (KTD13), keeping the cockpit strings quoted verbatim where B10 requires.

**Patterns to follow:** the findings' per-item "Fix" text; the user's Markdown rules.

**Test expectation:** none – documentation; `python3 scripts/build-plugin-zip.py --check` guards the README surface and `scripts/check-docs.sh` guards links and spellings.

**Verification:** `grep -n 'only network access' README.md` finds nothing; `grep -c 'ok   ' CONTRIBUTING.md` sample block shows eight lines; `grep -n 'workspace/superset_files.rs' .cursor/BUGBOT.md` finds nothing; `grep -c '](' .cursor/BUGBOT.md` is 0; `scripts/check-docs.sh` passes; `python3 scripts/build-plugin-zip.py --check` passes (U3 touches nothing under `plugin/`, so the committed digest still matches).

### U4. Explicit-path verbs and the render budget

**Goal:** `checklist verify` and `checklist render-md` accept validated explicit paths and `render-md` can bound its body, so a CI job with no pointer can name the documents it means.

**Requirements:** R15, R16.

**Dependencies:** none (U1–U3 ordering is by convention, not by code).

**Files:** `crates/ss-magic-plugin/src/checklist/verbs.rs`, `crates/ss-magic-plugin/src/checklist/verbs/tests.rs`, `crates/ss-magic-plugin/src/checklist/mod.rs` (re-exports if the validator lives beside `read_document`).

**Approach:**

1. Extend `parse` so `verify` and `render-md` take zero or more trailing paths and `render-md` takes `--max-bytes <N>`; any other `-`-prefixed token is an error (exit 2), never a path; the `USAGE` string and the three usage sites in the session-start guidance and the skill stay accurate (U7 updates the prose).
2. Add one path validator per R15 and KTD6 returning the canonical path or a refusal naming the rule tripped; the caller reads through that canonical path; it never consults the pointer.
3. `run_verify` over several paths reports findings per document and exits 1 if any has an error; `run_render` over several paths joins each document's own `render()` output with fixed separator text and applies the R16 budget, marker included.
4. Zero arguments keeps the existing `load_active` route untouched.

**Patterns to follow:** `contained_join` for the lexical refusal; `read_document` for the regular-file check; the `Budget` enum's naming for the new flag's type; exit code 2 for "the command as typed cannot be carried out".

**Test scenarios** (`crates/ss-magic-plugin/src/checklist/verbs/tests.rs`):

- `verify <one valid path>` on a valid document exits 0 and prints "is valid".
- `verify <a> <b>` where `b` has an error exits 1 and names `b`.
- `render-md <a> <b>` emits both documents' renders, each inside its own envelope, in argument order, with no text between them other than the fixed separator.
- `render-md --max-bytes N <a> <b>` where only `a` fits emits `a` whole plus a marker naming `b`; where neither fits emits the marker alone and exits 0; with 100 long-named files omitted the whole body, marker included, is at most N bytes and the marker ends "… and N more".
- A file name containing a backtick and a newline, omitted by the budget, cannot open a code block or a new Markdown line in the marker.
- An absolute path, a path with `..`, a path without the suffix, a nested `docs/actions/sub/x.checklist.json`, and a symlinked path are each refused with exit 2 and the reason named; nothing is read through the symlink (assert the target file's content never appears in output).
- An uppercase `DOCS/actions/x.checklist.json` is accepted or refused exactly as `matches_convention` (and so the checklist deny) treats it.
- A path whose canonical form leaves the repository through a symlinked directory is refused; a repository root reached through a symlink (as macOS `/tmp` is) still accepts its own files.
- An unknown flag such as `--bogus` is refused with exit 2 rather than treated as a path.
- `render-md --max-bytes N <big>` where `big` alone renders past N emits its truncated render with the envelope's "the whole text is at …" notice, and the body is at most N bytes.
- With no arguments and no pointer and two documents present, the verbs still refuse with "holds several" (unchanged behavior pinned).
- With no arguments and a pointer, the pointed document is used (unchanged behavior pinned).
- Covers AE3.

**Verification:** `cargo test -p ss-magic-plugin --locked checklist` green; `ss-magic-plugin checklist verify --help` and the usage error list the new forms.

### U5. Diff-scoped workflow template and legacy-template classification

**Goal:** the shipped workflow renders exactly the pull request's changed checklists, and `setup-github-ci` upgrades every untouched workflow it ever wrote.

**Requirements:** R14, R17, R18.

**Dependencies:** U4.

**Files:** `assets/workflow/checklist.yml`; create `assets/workflow/legacy/checklist-0.11.0.yml` and `assets/workflow/legacy/checklist-1.0.1.yml`; `crates/ss-magic-plugin/src/setup_ci.rs`, `crates/ss-magic-plugin/src/setup_ci/tests.rs`.

**Approach:**

1. Before editing the template, copy it to `assets/workflow/legacy/checklist-1.0.1.yml` and extract `git show v0.11.0:assets/workflow/checklist.yml` to `checklist-0.11.0.yml`. The fixtures stay byte-exact (a leading comment would break the byte comparison), so each generation's provenance tags are recorded in the comment on `setup_ci.rs`'s `LEGACY_TEMPLATES` table instead.
2. Rewrite the `render` job per KTD5: `fetch-depth: 2`, the `HEAD^2` assertion, the `:(glob)` diff written to a file under `$RUNNER_TEMP` as its own command, `present` from the file's entry count, the verify and render steps each re-reading that file into a bash array with the empty-array guard, `verify "${names[@]}"`, `render-md --max-bytes 60000 "${names[@]}"`; update the header prose that explains the selection; leave the `comment` job as is.
3. Add `LEGACY_TEMPLATES` and the generation-aware `classify`, `pinned_version` and `report_state` per KTD7; `print_diff` runs for `PinStale` too; the `--check` "would" line for a legacy pin-stale says it would replace the file with the current workflow.
4. Keep `State::token()` values unchanged.

**Patterns to follow:** `include_str!` as used for `TEMPLATE`; the existing `classify` tests' shape; `docs/solutions/logic-errors/retention-must-know-every-layout-ever-written.md` for seeding tests with every historical layout.

**Test scenarios** (`crates/ss-magic-plugin/src/setup_ci/tests.rs`):

- Every entry of `LEGACY_TEMPLATES` rendered at a sample version classifies `PinStale`, never `Differs` (loop over the table, so a future generation added to it is covered automatically).
- The 0.11.0 generation rendered at `0.11.0` pins `0.11.0` through its own `SS_MAGIC_VERSION:` key and classifies `PinStale` under the current version.
- A legacy generation rendered at the current version string still classifies `PinStale` (the version equality rule applies only to the current template).
- A legacy render with `runs-on: self-hosted` substituted classifies `Differs`.
- The current template at another version stays `PinStale` and at the same version `Identical` (existing tests kept).
- `run_core` with `check` on a legacy file prints `state: pin-stale`, a diff, and writes nothing; without `check` it writes the current render and exits 0 without `force`.
- The template test `no_run_step_interpolates_an_expression` and `the_workflow_globs_the_checklist_convention` pass unmodified against the new template.
- The `render` job's checkout step contains `fetch-depth: 2` and `persist-credentials: false`; the selection step contains `rev-parse --verify HEAD^2` and the `:(glob)` pathspec, and never reads `git diff` through `< <(`; the verify and render steps each re-read the names file and fail when the array is empty while `present` is true; the verify and render steps pass `"${names[@]}"` and the render step passes `--max-bytes`.
- Covers AE4, AE5.

**Verification:** `cargo test -p ss-magic-plugin --locked setup_ci` green; `diff <(git show v0.11.0:assets/workflow/checklist.yml) assets/workflow/legacy/checklist-0.11.0.yml` and `diff <(git show ss-magic-plugin-v1.0.1:assets/workflow/checklist.yml) assets/workflow/legacy/checklist-1.0.1.yml` both print nothing; `actionlint` or a manual read confirms the YAML is well-formed; a dry run of the diff command on this repository against a fabricated merge commit lists only changed `.checklist.json` paths.

### U6. The migrate-repository skill

**Goal:** an operator in an installed-but-not-enabled repository can ask for the migration and get preflight, the enable decision, conversion, retirement, CI setup and a report, with every step reversible or confirmed.

**Requirements:** R20–R30, R34.

**Dependencies:** U4, U5 (the skill's CI step relies on the fixed workflow and the skill's prose names the verbs' final forms).

**Files:** create `plugin/skills/migrate-repository/SKILL.md`, `plugin/skills/migrate-repository/reference.md`, `plugin/skills/migrate-repository/example.md`.

**Approach:**

1. `SKILL.md` (thin, under 150 lines): frontmatter `name: migrate-repository` and a folded description of the shape "Take a repository that already runs a hand-written Markdown operator checklist onto the ss-magic plugin: preflight, the enable decision, conversion of this branch's legacy checklist through the checklist verbs, retirement of the old rules, and CI setup. Use when the operator asks to migrate or adopt the plugin's operator checklist. Not for the `ss-magic` workspace migration from `setup.sh` to `magic.sh`, and not for creating a checklist in a repository that never had a Markdown one." Then the decision flow from the High-Level Technical Design as numbered steps keyed on `status --json` fields (`enablement.ss_magic.enabled`, `enablement.acting`, `repo.is_worktree`, `repo.main_checkout_root`, `state_tree.ignored`, `versions`), the confirmation gates of R34, the rule that checklist JSON is never touched through Bash, the branch-only mode, and the final report shape; link `./reference.md` and `./example.md`.
2. `reference.md`: the preflight decision table; the enable modes with the worktree sentence (KTD11); the seed diff explanation; the conversion rules of KTD10 as tables (field mapping, section mapping, id normalization and collision set, changelog entry ids, done and date handling, order within done/priority groups, refs, annotations); the size thresholds, call-count estimate and Read-gate ordering (KTD12); the restart rule of R23 (a committed document skips, an uncommitted one is discarded with `git clean -f -- <path>` after confirmation and reconverted); the trust boundary and stdin-only rule of R34 with one worked heredoc example; the ref-base derivation from `origin` with userinfo stripped (KTD10); the per-verb refusal table keyed on exit code (R34); the stub text and the open-items-only annotation of R25; the retirement inventory grep list, the replacement project-rule paragraph (R26), and the rule to rewrite rather than delete files other tools list; the CI step; the final-report template including the other-worktrees instruction and the other-branches transition line.
3. `example.md`: a worked example on "a reference consumer repository" by structure only (R30): a `docs/actions/<YYYY-MM-branch>/CHECKLIST.md` per branch (about a dozen folders, 2.5 KB to about 380 KB, 4 to about 375 items each), a mandatory header with a title line, a `Last updated` line with a trailing backslash and a `Branch name` line, four time-ordered sections (`Pre-Deploy Requirements`, `Post-Deploy Requirements`, `Post-Deploy Verification`, `Visual/Functional Checks`) plus ad-hoc ones, `### <Area>` subsections named by console or owner, two item styles (an older bold pass-marker style and a newer `action – expected (why)` style), a priority emoji, bold decision rows with a stated default, dated "superseded" annotations, a standing checklist outside `docs/actions/`, a project skill of the same name with a shell resolver that derives the folder from the branch, a `docs/actions/CLAUDE.md`, a docs-reference checker that fails on dead backticked paths, domain `CLAUDE.md` files telling agents to add a row, code comments linking a historical checklist, and a `.scratchpad/.plugin/` reservation; then the walk-through: what preflight reported, the enable choice in a worktree, the section retitle table, three converted items shown as the verb calls that produce them, the stub, the retire diff summary, the CI state token, and the final report. No vendor, product, person, issue id, PR number, branch name or item text from the real repository appears.
4. Every command is `ss-magic-plugin <verb>`; git commands are plain `git`; `gh` is used only through the `setup-github-ci` skill.

**Patterns to follow:** `plugin/skills/setup-github-ci/SKILL.md` for the ask-then-act state machine keyed on a token; `plugin/skills/operator-checklist/reference.md` for "thin skill, the CLI is the format" (no schema restated); the folded-description frontmatter of the three shipped skills.

**Test expectation:** none – Markdown; guarded by the CI greps (`CLAUDE_PLUGIN_DATA`, retired spelling), `scripts/check-docs.sh`'s link check over `plugin/skills/**`, and a manual anonymisation grep.

**Verification:** `grep -rn 'CLAUDE_PLUGIN_DATA\|ss-magic plugin ' plugin/skills/migrate-repository/` finds nothing; a case-insensitive grep of `plugin/skills/migrate-repository/` for the reference consumer repository's identifying terms (its name, organization, vendors, tracker ids and `#<number>` PR references; the denylist is held by the operator and never committed, because naming the terms here would publish them) finds nothing; every `](./…)` link in the three files resolves; `python3 scripts/build-plugin-zip.py --selftest` still passes (ASCII names, no symlinks); the migration rehearsal in the Verification Contract passes every scenario, run once U4 and U5 are in and repeated against the built 1.1.0 binary in U8, with the outcome recorded in the PR description.

### U7. Document the new work

**Goal:** README, CONCEPTS, BUGBOT, the architecture maps and the `setup-github-ci` skill describe the four skills, the changed verbs and template, the legacy-template rule and the rule-file guards.

**Requirements:** R19, R32, R1 (the shipped skills' drift, including the section-set claim in `operator-checklist/reference.md`), R6, R7, R13.

**Dependencies:** U4, U5, U6.

**Files:** `README.md`, `CONCEPTS.md`, `.cursor/BUGBOT.md`, `.claude/rules/architecture-plugin.md`, `.claude/rules/plugin-assets.md`, `.claude/rules/conventions.md`, `plugin/skills/setup-github-ci/SKILL.md`, `plugin/skills/operator-checklist/SKILL.md` (verb list gains the optional `[FILE...]`), `plugin/skills/operator-checklist/reference.md` (the section-set drift fix, moved here from U3).

**Approach:**

1. README: a "Skills" subsection under "The Claude Code plugin" listing `/ss-magic:scratchpad`, `/ss-magic:operator-checklist`, `/ss-magic:setup-github-ci` and `/ss-magic:migrate-repository` with the verbs each drives; the verb block gains `checklist verify [FILE...]` and `checklist render-md [--max-bytes N] [FILE...]` (flag before the paths, the order the U4 tests and the U5 workflow use); "Keeping the plugin current" gains an "Upgrading to 1.1.0" paragraph: re-run `/ss-magic:setup-github-ci`, an untouched workflow reports `pin-stale` and advances without `--force`, and the workflow now renders only the pull request's changed checklists.
2. CONCEPTS: a "Repository migration" term under the plugin glossary (what the skill does, that it converts one branch, that history stays Markdown) with a sentence distinguishing it from the sync model's init/migrate of the workspace contract, and a cross-reference from that existing paragraph.
3. BUGBOT: inline rules – every template generation ever released stays in `LEGACY_TEMPLATES` and a template change without a new fixture is flagged; explicit-path verbs refuse absolute, `..`, non-suffix, symlink and out-of-root paths and never touch the pointer; `render-md`'s budget bounds the whole body including the marker, cuts later documents at boundaries and truncates an oversized first document inside its envelope; the workflow passes names as argv and never interpolates; rule-file guards and the always-loaded budget; the skill-authoring rules restated for the fourth skill.
4. Architecture maps: `setup_ci.rs` (legacy table, generation-aware classify), `checklist/verbs.rs` (explicit paths, budget), the template's selection step, `scripts/check-docs.sh`; `conventions.md`'s checks list grows by the doc-guard script.
5. `operator-checklist/reference.md`: replace "a project that declares its own section set gets exactly that order" with the fact that `init` writes the four default sections and `set <section-id> title` is the only section edit.
6. `setup-github-ci/SKILL.md`: the `pin-stale` branch says a diff may be printed and the report names an earlier template generation; the "What the workflow does" list says it renders the pull request's changed checklists.

**Patterns to follow:** README's existing verb block; CONCEPTS' definition-then-boundary paragraph shape; BUGBOT's "Flag …" sentence shape.

**Test expectation:** none – documentation; guarded by `scripts/check-docs.sh` and `--check`.

**Verification:** `grep -c '/ss-magic:' README.md` is at least 4; `grep -n 'Repository migration' CONCEPTS.md` finds the term; `grep -n 'LEGACY_TEMPLATES' .cursor/BUGBOT.md .claude/rules/architecture-plugin.md` finds both; `scripts/check-docs.sh` passes.

### U8. Plugin 1.1.0 on every surface

**Goal:** the plugin group reads `1.1.0` everywhere `--check` enumerates, the zip digest is re-pinned, and the full gate is green.

**Requirements:** R31, R33.

**Dependencies:** U6, U7 (every `plugin/` edit).

**Files:** `crates/ss-magic-plugin/Cargo.toml`, `Cargo.lock` (the `ss-magic-plugin` entry), `plugin/.claude-plugin/plugin.json`, `plugin/ss-magic-plugin.version`, `.claude-plugin/marketplace.json` (tag and asset name in the url, then the digest), the `artifacts` literal in `crates/ss-magic-plugin/Cargo.toml`'s `[[package.metadata.dist.extra-artifacts]]`.

**Approach:**

1. Bump the surfaces; run `cargo build --release --workspace` so `Cargo.lock` follows.
2. `python3 scripts/build-plugin-zip.py --update-manifest`, then `--check` (eight `ok` lines), then `--check-bump ss-magic-plugin-v1.0.1`.
3. Run the whole Verification Contract.

**Patterns to follow:** the release procedure in `CONTRIBUTING.md` ("Release procedure, per line"), bump-then-merge-then-tag; no tag is pushed here.

**Test expectation:** none – version surfaces; `--check` is the test.

**Verification:** `--check` prints eight `ok` lines; `--check-bump ss-magic-plugin-v1.0.1` passes; `grep -rn '1\.0\.1' plugin/ .claude-plugin/ crates/ss-magic-plugin/Cargo.toml` finds only the legacy fixture reference and prose about the earlier release; CLI version still `0.11.2`.

---

## Verification Contract

| Check | Command | Applies to | Pass signal |
|---|---|---|---|
| Rust suite | `cargo test --workspace --locked` | U2, U4, U5, U8 | green |
| Release build | `cargo build --release --workspace` | U8 | builds |
| Builder selftest | `python3 scripts/build-plugin-zip.py --selftest` | U6, U8 | passes |
| Release assertions | `python3 scripts/build-plugin-zip.py --update-manifest && python3 scripts/build-plugin-zip.py --check` | U6, U7, U8 | eight `ok` lines |
| Bump check | `python3 scripts/build-plugin-zip.py --check-bump ss-magic-plugin-v1.0.1` | U8 | passes |
| Dependency absence | `for c in self_update inquire ratatui; do cargo tree --locked -p ss-magic-plugin -i "$c"; done` | U8 | each reports no match |
| Bootstrap suite | `/bin/bash scripts/test-bootstrap.sh` | U8 | all pass |
| Latest-mark suite | `/bin/bash scripts/test-mark-latest.sh` | U8 | all pass |
| Document guards | `/bin/bash scripts/check-docs.sh && /bin/bash scripts/check-docs.sh --selftest` | U1–U3, U6–U8 | only `ok` lines; selftest passes |
| Hard rules moved verbatim | `diff <(git show 7f6c62d:CLAUDE.md \| sed -n '1651,1741p') <(sed 's#](\.\./\.\./docs/#](./docs/#g' .claude/rules/hard-rules.md)` run at the U1 commit | U1 | empty |
| Nothing lost or duplicated in the move | `diff <(git show 7f6c62d:CLAUDE.md \| sort) <(cat CLAUDE.md .claude/rules/*.md \| sed 's#](\.\./\.\./docs/#](./docs/#g' \| sort)` at the U1 commit (multiset, both directions) | U1 | only the index's authored lines, the `paths:` frontmatter blocks and the retired header lines named in the U1 commit |
| Always-loaded budget | the `check-docs.sh` budget line, which derives the set as `CLAUDE.md` plus every `.claude/rules/**/*.md` without `paths:` | U1, U2 | under 50,000 |
| CI selection fails closed | in a scratch repository: a non-merge HEAD makes the selection step exit non-zero; a merge commit touching `docs/actions/a.checklist.json` and `docs/actions/sub/b.checklist.json` selects only the first | U5 | as stated |
| Merge-ref shape on GitHub (manual) | in a scratch GitHub repository (never this one), a workflow holding only the new checkout and selection steps that prints the selected names; one PR behind its base and one up to date | U5 | `HEAD^2` resolves in both and only each PR's own top-level checklist names are printed |
| Migration rehearsal (manual) | a script-generated fixture repository in the session scratchpad (never committed, never under `plugin/`): main checkout plus a linked worktree, `magic.json` without a `plugin` key, an `origin` remote of the form `https://x:secret@host/o/r.git`, a synthetic `docs/actions/<YYYY-MM-x>/CHECKLIST.md` of about 15 items covering both item styles, a `###` area, a decision row, a dated done row, the priority marker, a dated annotation, a relative and an absolute link, a title starting with a digit, a title with no letters or digits, two titles sharing their first six words, a title equal to a section id, a title holding a backtick command span and an apostrophe, and a line phrased as an instruction to the agent, plus a hand-written `operator-checklist` skill, a project rule, and a second legacy file padded past 80 items. The session loads the working tree with `claude --plugin-dir ./plugin` and the locally built `target/release/ss-magic-plugin` is placed where `plugin/hooks/bootstrap.sh` installs the binary, with the install markers `already_installed` checks set to the pin, because a 1.1.0 release does not exist yet | U6, U8 | (a) committed-only enable in the worktree: AE6; (b) interrupt mid-conversion, re-run: AE7; (c) decline enable: AE8; (d) on the default branch: no commit; (e) already migrated: branch-only mode, no enable question, straight to the report; (f) large file: the R24 question with a call count; (g) the backtick and apostrophe title: AE9; (h) `secret` appears in no file and no output the skill writes; (i) the instruction-shaped line is transcribed, not followed |
| Legacy fixture provenance | `diff <(git show v0.11.0:assets/workflow/checklist.yml) assets/workflow/legacy/checklist-0.11.0.yml; diff <(git show ss-magic-plugin-v1.0.1:assets/workflow/checklist.yml) assets/workflow/legacy/checklist-1.0.1.yml` | U5 | both empty |
| Skill anonymisation | `grep -rniE '<operator-held denylist>' plugin/skills/migrate-repository/` (terms never committed, see U6) | U6 | no match |
| Retired spelling | `grep -rn 'ss-magic plugin ' plugin/skills/ docs/solutions/ README.md CONTRIBUTING.md CONCEPTS.md CLAUDE.md .claude/rules/ .cursor/BUGBOT.md` | all | no match |
| Loading behaviour (manual) | `/context` in a fresh session, then Read one file per crate | U1 | the three unconditional rule files load at start; each map appears only after its file is read |
| CI dry run (manual) | push the branch and open a draft PR, only at the U1 head and the U8 head (Sequencing) | U1, U8 | `ci.yml` green including the documentation-guards step |

---

## Definition of Done

- Every item in [drift-findings.md](./2026-10-05-1610-feat-docs-sync-and-repository-migration/drift-findings.md) is either applied (R1, R2, R3) or listed under Code suspects (R4); no historical record was edited (R5).
- `CLAUDE.md` is an index under 200 lines; the seven rule files exist with the globs of KTD1; the hard-rules diff and the multiset move diff pass; `scripts/check-docs.sh` passes locally and in CI (R8–R13).
- The workflow template selects changed checklists; the verbs accept validated paths and the budget; `setup-github-ci` reports `pin-stale` for both legacy generations; all tests named in U4 and U5 exist and pass (R14–R18).
- `plugin/skills/migrate-repository/` ships its three files, passes the greps, and its example carries nothing client-identifying (R20–R30).
- README names four skills and the upgrade path; CONCEPTS carries the disambiguated term; BUGBOT restates every new rule inline and links nowhere (R19, R32, R6).
- Every plugin-group surface reads `1.1.0`, the digest is re-pinned, and every row of the Verification Contract passes (R31).
- Eight commits on `ViktorStiskala/docs-update`, one per unit, nothing pushed or tagged by the plan; no abandoned experiment, scratch file or dead code remains in the diff.

---

## Appendix

### Section-to-file mapping for U1 (line numbers at `7f6c62d:CLAUDE.md`)

| Lines | Section | Destination |
|---|---|---|
| 1–6 | Header | `CLAUDE.md` (rewritten as the index opening) |
| 7–115 | `## Build`, `### Two release lines`, `### The packaged plugin tree and its version surfaces` | `.claude/rules/build-release.md` |
| 116–180 | `## Architecture` intro, crate list, mermaid, re-export notes | `CLAUDE.md` (crate layout) |
| 181–697 | Module bullets | `architecture-core.md` for core modules (`git/mod.rs`, `git/discover.rs`, `superset_files.rs`, `sync/mod.rs`, `sync/repo_scan.rs`, `sync/pattern.rs`, `sync/apply.rs`, `style.rs`, `git/gitignore.rs`, `hashing.rs`; `release.rs`, `reponame.rs`, `state_tree.rs` and `testutil.rs` gain their own bullets in U2); `architecture-cli.md` for CLI modules (`tui/theme.rs`, `tui/ui.rs`, `tui/cockpit.rs`, `cli.rs`, `tui/menu.rs`, `workspace/migrate.rs`, `sync/reverse_sync.rs`, `pack.rs`, `update/` (its `release.rs` text moves to the core map in U2), `main.rs`) |
| 1651–1741 | the two hard-rules sections | `hard-rules.md` (verbatim, links re-based; line 1742 is a trailing blank line) |
| 698–737 | `## The Claude Code plugin` intro | `architecture-plugin.md` (opening) |
| 738–1352 | Entry point, State, Hooks, Human verbs, The operator checklist | `architecture-plugin.md` |
| 1353–1452 | Non-Rust assets | `plugin-assets.md` |
| 1453–1460 | `## Source of truth for magic.sh` | `conventions.md` |
| 1461–1650 | `## Conventions` | `conventions.md` |
| 1743–1752 | `## Documented Solutions` | `CLAUDE.md` (orientation pointer) |

### Template generations the classifier must know

| Generation | Released on | Pin key | Placeholder | Fixture |
|---|---|---|---|---|
| 0.11.0 shape | `v0.10.0`, `v0.11.0` (byte-identical) | `SS_MAGIC_VERSION:` | `@SS_MAGIC_VERSION@` | `assets/workflow/legacy/checklist-0.11.0.yml` |
| 1.0.1 shape | `v0.11.1`, `v0.11.2`, `ss-magic-plugin-v1.0.0`, `ss-magic-plugin-v1.0.1` (byte-identical) | `SS_MAGIC_PLUGIN_VERSION:` | `@SS_MAGIC_PLUGIN_VERSION@` | `assets/workflow/legacy/checklist-1.0.1.yml` |
| 1.1.0 shape | this plan | `SS_MAGIC_PLUGIN_VERSION:` | `@SS_MAGIC_PLUGIN_VERSION@` | `assets/workflow/checklist.yml` |

`v0.9.0` and earlier carry no template.

### Status fields the skill's preflight reads

`enablement.ss_magic.enabled`, `enablement.acting`, `repo.root`, `repo.main_checkout_root`, `repo.is_worktree`, `state_tree.ignored`, `bootstrap.binary`, `versions`, and `problems` – all from `ss-magic-plugin status --json` (schema `2`), which exits 0 whenever a report was produced.

### Execution amendments (2026-10-07)

Recorded after implementation (PR #10); the sections above are left as planned.

- **KTD5 and R14, selection filter:** the shipped workflow uses `--diff-filter=AMT`, not `AM`. A checklist that changed between a regular file and a symlink is a modification, so it is selected and the verb's symlink refusal fails the job; `AM` would have skipped it and left the gate green.
- **R16, budget floor:** `render-md --max-bytes N` refuses N below 2048, the cap on the omitted-documents marker, because the "at most N bytes, marker included" bound cannot hold below it. `--max-bytes` without a FILE applies the budget to the active checklist.
- **R23, staged document:** an uncommitted document that is staged stops the run with a `git restore --staged` instruction instead of offering the discard, because `git clean -f` cannot remove an index entry. An untracked one is still offered the discard.
- **R29, CI step in branch-only mode:** branch-only mode runs the CI step when `setup-github-ci --check` reports `absent` or `pin-stale`, and a declined `pin-stale` upgrade does not count as an installed workflow for retirement. A `pin-stale` workflow of an earlier template generation still renders the wrong checklist and fails once `docs/actions/` holds two, so treating it as working would skip the one upgrade that fixes the repository (code-review finding #3).
- **R33 and Definition of Done, commit count:** the branch carries ten commits, not eight: U1 through U8, then a simplify pass over the U4 code and the review-fix commit for findings #1 to #11.
