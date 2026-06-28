---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
type: fix
title: "fix: Reverse sync finds gitignored untracked files (secrets)"
date: 2026-06-28
origin: docs/brainstorms/2026-06-17-ss-magic-rewrite-requirements.md
plan_depth: standard
---

# fix: Reverse sync finds gitignored untracked files (secrets)

## Summary

Reverse sync reports "No untracked files match the configured patterns" even
when the worktree holds untracked secret files that match `magic.json`. The
candidate probe `git::untracked_files` runs `git ls-files --others
**--exclude-standard**`, and `--exclude-standard` drops gitignored paths — but
the files reverse sync exists to push (`.env`, `.dev.vars`, and the gitignored
`magic.local.json`) are *exactly* the gitignored ones. So in any real repo that
gitignores its secrets, the candidate set is always empty.

The fix removes `--exclude-standard` from the probe so it lists every untracked
file (tracked files stay excluded — they merge via git). `git ls-files
--others` lists untracked **files** only, so directory-pattern matches are still
naturally excluded and the single-file copy path is unaffected. The
candidate-intersection logic in `compute_candidates` needs no change. The bulk
of the work is correcting the test that pinned the wrong behavior and adding the
gitignored-secret regression coverage that was missing — the gap that let this
ship.

---

## Problem Frame

`reverse_sync::compute_candidates` builds the candidate set as the intersection
of two probes (`src/reverse_sync.rs:101-114`):

- `apply::match_paths(worktree_root, &cfg.files)` — files matching the overlaid
  `magic.json` + `magic.local.json` patterns.
- `git::untracked_files(worktree_root)` — `git ls-files --others
  --exclude-standard -z` (`src/git.rs:213-235`).

`--exclude-standard` honors `.gitignore`, `.git/info/exclude`, and global
excludes, so an untracked-**and-ignored** file is omitted from the list. The
entire purpose of reverse sync is to push back untracked secrets that never
merge via git — and those secrets are gitignored by definition. The downstream
machinery confirms the intent: `copy_candidate_into_main` →
`ensure_gitignored_in_main` (`src/reverse_sync.rs:235-270`) goes to great
lengths to copy the worktree's covering `.gitignore` rule into main so the
pushed secret stays ignored there. That safety apparatus is unreachable in
production because the candidate was filtered out one step earlier.

Empirically confirmed in a scratch repo: with `**/.dev.vars` and `.env`
gitignored, `git ls-files --others --exclude-standard` returns only the
non-ignored `newfile.txt`; `git ls-files --others` returns `.env`,
`apps/api/.dev.vars`, and `newfile.txt`. The two committed tests that "cover"
candidate selection
(`reverse_sync::tests::ae9_untracked_is_candidate_tracked_is_not`,
`magic_local_json_is_candidate_when_matched_and_untracked`) both write their
secret files **without** gitignoring them, so they never exercise the real
production shape — false confidence that masked the defect.

**Symptom → cause chain:**

1. User's worktree has untracked `.env` / `.dev.vars` matching `magic.json`,
   and those paths are gitignored (the normal case).
2. `untracked_files` omits them (`--exclude-standard`).
3. `matched ∩ untracked` is empty.
4. `run` prints "No untracked files match the configured patterns" and returns
   without opening the picker (`src/reverse_sync.rs:294-302`).

---

## Requirements

- **R-fix-1.** `compute_candidates` must include untracked files that are
  gitignored in the worktree, as long as they match the overlaid patterns. This
  restores R23 / AE9 / KTD10 (origin:
  `docs/brainstorms/2026-06-17-ss-magic-rewrite-requirements.md`), which call
  for pushing untracked secrets like `apps/api/.dev.vars`.
- **R-fix-2.** Tracked files must remain excluded (they merge via git) —
  including tracked-but-modified files. No regression to AE9's "tracked
  `magic.json` is not offered" behavior.
- **R-fix-3.** The gitignored `.superset/magic.local.json` must surface as a
  candidate when present and untracked (it is gitignored by bootstrap, so this
  is the same defect class).
- **R-fix-4.** Directory matches must not become candidates (the copy path is
  single-file `fs::copy`); only regular files are pushed.
- **R-fix-5.** Regression tests must exercise the **gitignored** shape so this
  defect cannot silently return.

---

## Key Technical Decisions

**KTD-fix-1. Drop `--exclude-standard`; do not invert to a tracked-set.**
The correct membership predicate is "matched **and not tracked**" (untracked,
regardless of ignore status). Two implementations were considered:

- *(chosen)* Remove `--exclude-standard` from the existing probe so it runs
  `git ls-files --others -z`. This lists all untracked **files** (git lists
  files, not directories, without `--directory`), so the existing
  `matched ∩ untracked` intersection keeps directory matches out for free and
  the single-file copy path is untouched. One-flag change; the probe's behavior
  now matches its name (`untracked_files` = untracked, period).
- *(rejected)* Invert to a tracked set via `git ls-files` and keep matched
  files that are absent from it. Equivalent for files, but `git ls-files`
  returns no directory entries, so a matched **directory** would fall through as
  "not tracked" and become a candidate that `fs::copy` then fails on — a new
  bug. Rejected to avoid reintroducing the directory case the intersection
  currently handles implicitly.

**KTD-fix-2. No logic change in `compute_candidates`.** Broadening the probe is
sufficient; the intersection, de-dupe, sort, and `is_safe_rel` guard all stay.
This keeps the blast radius to one git flag plus tests/docs.

**KTD-fix-3. Performance is acceptable and unchanged in character.** `git
ls-files --others` (no `--exclude-standard`) descends into ignored directories
(e.g. `target/`, `node_modules/`), so the returned list can be large. This is
acceptable: `apply::match_paths` already walks the entire worktree via
`walkdir`, and `DEFAULT_EXCLUDES` plus the pattern intersection mean ignored-dir
paths never become candidates. No new asymptotic cost relative to the walk the
tool already performs.

---

## Scope Boundaries

**In scope**

- Broaden `git::untracked_files` to include gitignored untracked files.
- Correct the git probe test and add gitignored-secret regression coverage in
  `reverse_sync`.
- Sync the doc comments, `CLAUDE.md` architecture note, and the README
  reverse-sync section to the corrected semantics.

**Out of scope — Outside this product's identity**

- A "sync `magic.json` itself back to main" operation. `magic.json` is tracked
  and reaches main via git merge; the origin requirements (R23) and plan
  explicitly exclude a reverse-sync path for it. If the user's intent was to
  propagate `magic.json` *config edits* (not the files those patterns match)
  from a never-merged worktree branch to main, that is a separate feature
  decision for `ce-brainstorm`, not this bug fix. See Assumptions.

**Deferred to Follow-Up Work**

- None.

---

## Assumptions

Resolved in headless/pipeline mode without a confirming user:

- **Interpretation.** "Reverse sync should sync files added to magic.json in the
  worktree" is read as: *the files matched by patterns in `magic.json` should be
  offered as reverse-sync candidates.* The reported symptom ("No untracked files
  match the configured patterns") is fully explained by the gitignore-exclusion
  defect, and the matched files in a real repo (`.env`, `.dev.vars`) are
  gitignored. This is the interpretation the plan fixes. The alternative reading
  — pushing `magic.json`'s own tracked content — is handled as Out of scope
  above, consistent with the existing design decision.

---

## Implementation Units

### U1. Broaden `untracked_files` to include gitignored untracked files

**Goal:** Make the git probe list every untracked file (ignored or not), so
reverse-sync candidates include gitignored secrets. Tracked files remain
excluded.

**Requirements:** R-fix-1, R-fix-2, R-fix-4.

**Dependencies:** none.

**Files:**
- `src/git.rs` — modify `untracked_files` (drop `--exclude-standard`); update
  its doc comment (`src/git.rs:201-235`); update the existing test
  `untracked_files_lists_only_untracked_unignored` (`src/git.rs:416-453`).

**Approach:**
- Change the args from `["ls-files", "--others", "--exclude-standard", "-z"]`
  to `["ls-files", "--others", "-z"]`. Keep the `-z` NUL parsing and the
  defensive `..`/absolute drop exactly as-is.
- Rewrite the doc comment: the probe now returns untracked files **including**
  gitignored ones; tracked files are still excluded; it lists files (not
  directories). State plainly that including ignored files is required because
  reverse sync pushes gitignored secrets.

**Patterns to follow:** the existing NUL-split + defensive-drop loop in
`untracked_files`; no structural change.

**Test scenarios** (in `src/git.rs` tests):
- Rename/repurpose `untracked_files_lists_only_untracked_unignored` →
  `untracked_files_lists_untracked_including_ignored`: given a tracked
  `README.md`, an untracked-not-ignored `new.txt`, an untracked secret
  `apps/api/.dev.vars`, and a gitignored-and-untracked `ignored.txt` (with a
  tracked `.gitignore` naming it), assert the result **contains** `new.txt`,
  `apps/api/.dev.vars`, **and** `ignored.txt`, and does **not** contain the
  tracked `README.md`. (This flips the one assertion that pinned the defect.)
- Edge case: a gitignored secret under a subdir (`apps/api/.dev.vars` with
  `**/.dev.vars` in a tracked `.gitignore`) is listed. Covers AE9 (candidate
  side) for the realistic gitignored shape.

**Verification:** `cargo test git::` passes; the renamed test asserts a
gitignored untracked file is now returned.

### U2. Reverse-sync regression coverage for gitignored candidates

**Goal:** Prove `compute_candidates` now surfaces gitignored secrets and the
gitignored `magic.local.json`, and lock the behavior against regression. Correct
the module doc note that describes the candidate set.

**Requirements:** R-fix-1, R-fix-3, R-fix-5.

**Dependencies:** U1.

**Files:**
- `src/reverse_sync.rs` — add tests in the existing `tests` module
  (`src/reverse_sync.rs:431-506`); update the module-level doc comment
  (`src/reverse_sync.rs:11-18`) and the inline note that references
  `--exclude-standard` (`src/reverse_sync.rs:15`).

**Approach:**
- No production-logic change in `compute_candidates` — U1 makes the existing
  intersection correct. This unit is regression coverage + doc accuracy.
- Update the doc comment so the "What moves, and what doesn't" section states
  candidates include gitignored untracked files (not just non-ignored ones), and
  drop the stale `--exclude-standard` reference.

**Patterns to follow:** the existing `compute_candidates` tests
(`ae9_untracked_is_candidate_tracked_is_not`,
`magic_local_json_is_candidate_when_matched_and_untracked`) and their
`write_magic` / `write` / `git_run` helpers — but with the secret **gitignored**
this time.

**Test scenarios** (in `src/reverse_sync.rs` tests):
- `gitignored_secret_is_a_candidate`: worktree with a tracked `.gitignore`
  containing `**/.dev.vars`, `magic.json` matching `**/.dev.vars`, and an
  untracked gitignored `apps/api/.dev.vars`. Assert `compute_candidates`
  contains `apps/api/.dev.vars`. (This is the exact production shape the old
  tests omitted — Covers AE9.)
- `gitignored_magic_local_json_is_a_candidate`: worktree with a tracked
  `.gitignore` containing `.superset/magic.local.json`, `magic.json` matching
  `.superset/magic.local.json`, and that file present + untracked. Assert it is
  a candidate. (R-fix-3.)
- Regression guard `modified_tracked_secret_is_not_a_candidate`: a file that
  matches a pattern, is gitignored, **and** is tracked (force-added with `git
  add -f`) and then modified — assert it is **not** a candidate (tracked ⇒
  merges via git). Confirms U1 did not start pulling tracked files in. (R-fix-2.)

**Verification:** `cargo test reverse_sync::` passes, including the three new
tests; the pre-existing candidate tests still pass.

### U3. Sync documentation to the corrected semantics

**Goal:** Align user- and contributor-facing docs with the fix so the
"git-untracked" wording is unambiguous about gitignored secrets.

**Requirements:** R-fix-1 (documentation traceability).

**Dependencies:** U1.

**Files:**
- `CLAUDE.md` — the `git.rs` architecture bullet lists `untracked_files` as a
  reverse-sync probe; note it lists untracked files **including gitignored**
  ones.
- `README.md` — the "Reverse sync (worktree → main)" section
  (`README.md:123-141`): clarify that "git-untracked files" includes gitignored
  secrets (e.g. `.dev.vars`), which is what the gitignore-safety step on copy is
  for. The section already describes the secret-safety copy; the candidate
  wording is what needs tightening.

**Approach:** prose-only edits; no behavior. Keep the existing tone and brevity.

**Patterns to follow:** existing CLAUDE.md bullet style and README section
voice.

**Test scenarios:** Test expectation: none — documentation-only unit.

**Verification:** `README.md` and `CLAUDE.md` describe candidate selection as
"untracked, including gitignored" with no remaining `--exclude-standard`-implied
wording; `grep -rn "exclude-standard" src/ README.md CLAUDE.md` shows no stale
references claiming ignored files are excluded.

---

## Risks & Mitigations

- **Secret-leak boundary (highest sensitivity).** This change makes gitignored
  secrets *flow into* the candidate path that writes to main. The
  gitignore-safety step (`ensure_gitignored_in_main`) is what keeps a pushed
  secret ignored in main; it is unchanged and already unit-tested
  (`ae9_copy_creates_dirs_and_appends_covering_rule`,
  `copy_falls_back_to_literal_when_covering_rule_is_subdir_anchored`,
  `magic_local_json_lands_gitignored_in_main`). Mitigation: do not touch the
  copy/gitignore logic; only broaden candidate discovery. The copy still
  guarantees the secret is ignored in main before writing bytes.
- **Over-inclusion of ignored build artifacts.** Broadening the probe means a
  broad user pattern could now match an untracked ignored artifact (e.g. under
  `target/`). Mitigation: this matches forward-sync's existing trust model
  (patterns are user-authored and specific), `DEFAULT_EXCLUDES` still drops
  `node_modules`/`.venv` matches, and the diff-aware picker + per-file confirm
  remain the human gate before anything is written to main.
- **Performance in large repos.** See KTD-fix-3 — bounded by the full-tree walk
  the matcher already performs; no new order of cost.

---

## Definition of Done

- `git::untracked_files` lists untracked files including gitignored ones; the
  git-probe test asserts the gitignored case.
- `compute_candidates` returns gitignored secrets and gitignored
  `magic.local.json` as candidates; tracked (and tracked-modified) files are
  excluded — all proven by new tests.
- `cargo test` passes; `cargo clippy` clean.
- README, CLAUDE.md, and in-code doc comments describe candidate selection as
  "untracked, including gitignored," with no stale `--exclude-standard`
  framing.

---

## Sources & Research

- Code read: `src/reverse_sync.rs` (`compute_candidates`, `run`,
  `ensure_gitignored_in_main`), `src/git.rs` (`untracked_files`, `is_ignored`,
  `check_ignore_pattern`), `src/apply.rs` (`match_paths`, `expand_patterns`),
  `src/superset_files.rs` (`load_overlaid`).
- Origin requirements: `docs/brainstorms/2026-06-17-ss-magic-rewrite-requirements.md`
  (R22–R26, AE9).
- Origin plan / KTD10 and U10–U13:
  `docs/plans/2026-06-17-001-feat-ss-magic-self-updating-cli-plan.md`.
- Empirical confirmation (scratch git repo): `git ls-files --others
  --exclude-standard` omits gitignored `.env` / `apps/api/.dev.vars`; `git
  ls-files --others` includes them; `git ls-files` (tracked) lists neither.
