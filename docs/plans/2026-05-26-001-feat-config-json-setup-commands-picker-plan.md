---
title: "feat: bootstrap picker for .superset/config.json setup commands"
type: feat
status: active
date: 2026-05-26
---

# feat: bootstrap picker for .superset/config.json setup commands

## Summary

Adds a new bootstrap step between the patterns picker and the `.envrc`
prompt that lets the user pre-select which commands go into
`.superset/config.json`'s `setup` array. A lockfile-driven scan plus
root `package.json` script parsing auto-pre-selects detected commands;
everything else stays visible but unchecked with a dim `(not detected)`
suffix. The patterns picker's action-loop is extracted into a shared
`ui` helper so the two pickers do not duplicate the loop.

---

## Requirements

- R1. Add a bootstrap step that captures setup commands for
  `.superset/config.json`'s `setup` array. Runs after `pick_patterns`
  and before the `.envrc` prompt.
- R2. Picker reuses the action-loop pattern (`Toggle` / `AddNew` /
  `Done`) via a shared helper extracted from the existing
  `pick_patterns`. Both pickers become thin wrappers; no
  loop-body duplication.
- R3. Detect setup-command candidates from cheap repo signals:
  - `pnpm-lock.yaml` at repo root → `pnpm -r install`
  - `package-lock.json` at repo root (no pnpm-lock) → `npm ci`
  - `yarn.lock` at repo root (no pnpm-lock, no package-lock) →
    `yarn install --frozen-lockfile`
  - `uv.lock` at repo root → `uv sync`
  - Root `package.json` `scripts` map parsed with `serde_json`; a
    recognized script name preselects `<pm> run <name>` flavored to the
    detected package manager (pnpm uses `pnpm -r run`).
  - `pnpm` wins when multiple JS lockfiles coexist.
- R4. The known-options row set is fixed and rendered every run; rows
  that did not detect carry a dim `(not detected)` suffix mirroring the
  patterns picker's `(no matches)` suffix.
- R5. Preselect logic:
  - `./.superset/setup.sh` row preselected by default (but deselectable).
  - Detected rows preselected.
  - Rows already present verbatim in the existing `config.json`'s
    `setup` array preselected.
- R6. Existing custom entries (entries in `config.json`'s `setup` that
  are not in the known-option set) are preserved across edit-mode
  re-runs and surface as deselectable rows above the sentinels, mirroring
  `existing_unknown_patterns` for `setup_config.json`.
- R7. `+ Add new command…` opens a sub-prompt with non-empty +
  duplicate-of-taken validation. Any non-empty trimmed string is
  accepted (no shell-syntax validation).
- R8. `.superset/config.json` is always rewritten in bootstrap mode from
  the picker output. `teardown` and `run` arrays are preserved verbatim
  from the existing on-disk `config.json` when present, or default to
  empty arrays on first run.
- R9. The Phase-0.7 stage-then-materialize discipline is preserved: no
  writes hit the working tree until after the final-action prompt.

---

## Scope Boundaries

- Picker covers the `setup` array only. `teardown` and `run` are
  preserved-from-disk, never solicited via the picker.
- No walking of workspace packages to discover scripts beyond the root
  `package.json`.
- No custom validation on command strings beyond non-empty and
  duplicate-of-taken. Shell-syntax, executable-on-PATH, and
  package-manager-availability checks are out of scope.
- Python detection limited to `uv.lock`. No `requirements.txt`, Poetry,
  or Pipenv heuristics.

---

## Context & Research

### Relevant Code and Patterns

- `projects/superset-setup/src/ui.rs` — current `pick_patterns`
  action-loop, the canonical reference for the row-as-action shape. The
  shared helper extraction targets the `loop { … }` body in this
  function.
- `projects/superset-setup/src/repo_scan.rs` — `matches_for_patterns`
  and `pattern_matches_any` shape the I/O surface for filesystem
  preselect signals. `repo_detect` mirrors the "input array → bool
  vector aligned to input" shape so the picker can consume the result
  the same way.
- `projects/superset-setup/src/superset_files.rs` —
  `existing_unknown_patterns`, `write_setup_config_json`,
  `copy_into_repo` carry the preservation idiom that
  `config.json` now adopts. `Config { setup, teardown, run }` already
  exists with serde derives.
- `projects/superset-setup/src/main.rs:bootstrap_flow` — composes the
  capture-then-stage-then-materialize sequence the new step plugs into.

### Institutional Learnings

- `projects/superset-setup/docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`
  — canonical write-up of the Select-loop pattern. The shared helper
  must keep the documented contract: cursor persistence across
  toggles, "land on the new row" after `AddNew`, "back to AddNew" on
  Esc-during-sub-prompt, `with_starting_cursor` clamped against the
  current actions length, `with_help_message` carrying the single-key
  model hint.

---

## Key Technical Decisions

- **Shared action-loop helper** lives in `ui.rs` as a free function
  parameterized by: prompt label, help message, initial row vector,
  sub-prompt label, sub-prompt help, sub-prompt validator closure
  (returns `Result<(), String>` for inline inquire validation), and a
  closure that yields the dim-suffix flag for a newly added row. Both
  `pick_patterns` and `pick_setup_commands` become thin wrappers.
  Rationale: the loop body is identical across pickers; only the
  validation rule and the dim-suffix label vary. A free function with
  closures is the smallest abstraction that removes the duplication
  without introducing a trait or builder.
- **`Row` shape carries `Option<&'static str>` for the dim suffix**
  rather than a `bool + fixed_label` pair. Each picker passes its own
  suffix literal (`"(no matches)"` vs `"(not detected)"`); `None`
  means no suffix. Keeps the helper agnostic to per-picker copy.
- **`config.json` write semantic shifts from "preserve whole file" to
  "always rewrite, preserving teardown/run".** Preservation moves up
  into the staging step (read disk → merge → stage merged Config). The
  `copy_into_repo` branch that skipped `config.json` when present is
  dropped; the staged file is now always authoritative.
- **`MaterializeReport::wrote_config` is removed.** With the new write
  semantic the field is always true and uninformative. `bootstrap_flow`
  derives the "Wrote vs Updated" message from
  `existing.superset_dir_present` instead — which it already has.
- **`Config::default_contract()` is dropped from the write path.**
  Bootstrap composes the `Config` from picker output + preserved
  teardown/run. The default-contract concept becomes the empty
  defaults the merge step uses on first run (inline literals).
- **Recognized setup-shaped script names** are a small fixed const
  array in `repo_detect.rs`. Starter list: just `cf-typegen` — a
  workspace-specific signal whose presence is a strong indication the
  user wants it in `setup`. `setup` and `codegen` are deliberately
  excluded for v1 because they are among the most-recycled npm-script
  slot names and would carry a high false-positive rate (a `setup`
  script may install husky hooks; a `codegen` script may take minutes
  and hit remote services). Preselecting either of those by name alone
  would be a strong endorsement the picker can't honestly make. The
  `+ Add new command…` path covers the cases v1 doesn't auto-detect.

---

## Open Questions

### Resolved During Planning

- Where does package-manager detection live? In `repo_detect.rs`,
  surfaced as a `PackageManager` enum (`Pnpm` / `Npm` / `Yarn` / `None`)
  that the script-flavoring logic consumes.
- How does `cf-typegen` get preselected? Solely from the root
  `package.json` `scripts` map (script presence drives preselect). No
  wrangler-file detection — the brainstorm originally floated that
  signal but parsing scripts is now in scope and is the more reliable
  source.

### Deferred to Implementation

- Final wording of the new bootstrap prompt label and help message
  (e.g., "Setup commands to run:" vs "Commands for `.superset/config.json` setup:").
  Pick the closest match to the patterns picker's tone during
  implementation.
- Final wording of the dim suffix for not-detected rows
  (`(not detected)` is the working draft).
- Whether the existing `config_json_is_preserved_on_rerun` test gets
  deleted outright or replaced with a "teardown/run preserved across
  re-runs" test. Decided during U2 implementation.

### From 2026-05-26 doc review

- **Main picker prompt label is user-model territory, not cosmetic.**
  The picker label and its help message are the only affordance
  telling a user what these commands are for and when they execute. A
  label like "Setup commands to run:" leaves the timing ambiguous; a
  label like "Commands for `.superset/config.json` setup:" leaks
  implementation detail. Pick a phrasing that establishes WHEN these
  commands run (on `superset apply` into a worktree) so a first-time
  user can pattern-match against intent before they start toggling.
- **Generalized unknown-entries helper vs inline filter.** U3's
  cmd_options construction says "via a small inline filter mirroring
  `existing_unknown_patterns`, or — if reused — a generalized helper
  added in U2." U2 doesn't currently ship such a helper. Decide
  during implementation: (a) widen `existing_unknown_patterns` to
  generic `existing_unknown_entries(existing: &[String], options: &[&str]) -> Vec<String>`
  and reuse for both pickers, or (b) inline the filter in
  `bootstrap_flow`. Both are valid; (a) reduces duplication, (b)
  keeps the helper tightly scoped to its original caller.
- **Silent-removal-via-dim-misread risk.** A pre-existing
  `config.json` setup entry that's *also* a known option (e.g.,
  `pnpm -r install`) preselects checked, but if the lockfile that
  motivated it is gone, the row carries the dim `(not detected)`
  suffix. A user scanning quickly might uncheck it on the
  intuition that "not detected" means "doesn't apply here." The
  setup command is silently removed. Possible mitigations:
  visually distinguish "checked & in existing config but not
  detected" from "checked & detected"; surface a confirmation
  before writing when a known-option that *was* in the existing
  config is being deselected; or accept the risk and rely on the
  Risks table mitigation. Pick during U3 smoke testing.
- **Bun and multi-lockfile precedence.** Detection encodes only
  pnpm / npm / yarn lockfiles; `bun.lockb`-only repos fall through
  to `PackageManager::None`. The pnpm-wins-when-coexisting rule
  is asserted with no rationale for why pnpm over npm specifically.
  Decide: add `bun.lockb` + `bun install` to the OPTIONS set; tighten
  the precedence rationale; or document the scope decision to skip
  bun explicitly.
- **Script-flavored row coverage for npm/yarn.** OPTIONS contains
  only `pnpm -r run cf-typegen`; a repo with `package-lock.json` +
  a `cf-typegen` script can never preselect a flavored row because
  no `npm run cf-typegen` row exists. Decide: surface flavored
  variants in OPTIONS (cluttering the picker), generate the
  flavored row dynamically based on detected package manager, or
  accept that script-flavored preselect is pnpm-only and require
  users on other PMs to use `+ Add new command…`.

---

## Implementation Units

- U1. **Repo detection module**

**Goal:** Add a new `repo_detect` module that computes which
preconfigured setup commands should be preselected for the picker,
driven by root lockfiles and the root `package.json` scripts map.

**Requirements:** R3, R5

**Dependencies:** None

**Files:**
- Create: `projects/superset-setup/src/repo_detect.rs`
- Modify: `projects/superset-setup/src/main.rs` (add `mod repo_detect;`)
- Modify: `projects/superset-setup/Cargo.toml` (no new deps —
  `serde_json` is already present)
- Test: `projects/superset-setup/src/repo_detect.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Public surface: `OPTIONS: [&str; N]` mirroring `repo_scan::OPTIONS` —
  the preconfigured command rows in display order. Starter set: 
  `./.superset/setup.sh`, `pnpm -r install`, `pnpm -r run cf-typegen`,
  `npm ci`, `yarn install --frozen-lockfile`, `uv sync`.
- `detect_for_options(root: &Path) -> Result<Vec<bool>>` — returns a
  bool vector aligned to `OPTIONS`, mirroring
  `repo_scan::matches_for_patterns` so the caller can consume it the
  same way.
- Internally split into:
  - `PackageManager` enum (`Pnpm` / `Npm` / `Yarn` / `None`) computed
    by checking the three JS lockfiles at root with `pnpm` winning.
  - `parse_root_package_scripts(root) -> Result<HashSet<String>>` —
    reads `<root>/package.json`, parses with `serde_json`, returns the
    keys of the `scripts` object (or an empty set when the file is
    absent, unparseable, or has no `scripts` field; do not bubble parse
    errors for an optional signal).
  - A const `RECOGNIZED_SCRIPTS: [&str; 1]` = `["cf-typegen"]`.
    See Key Technical Decisions for why `setup` and `codegen` are
    deliberately excluded from v1.
  - Per-option logic that maps each option string to a detection
    predicate using the package manager and scripts set.
- The `./.superset/setup.sh` row is always treated as detected
  (preselected by default, deselectable). Encode this by making its
  predicate `|_, _| true`.

**Patterns to follow:**
- `repo_scan.rs` — input array, bool vector aligned to input,
  permission errors treated as "no signal" rather than aborts.

**Test scenarios:**
- Happy path: empty repo → all preconfigured options return `false`
  except `./.superset/setup.sh` (always true).
- Happy path: only `pnpm-lock.yaml` at root → `pnpm -r install` true,
  `npm ci` and `yarn install --frozen-lockfile` false.
- Happy path: only `package-lock.json` at root → `npm ci` true, the
  pnpm and yarn rows false.
- Happy path: only `yarn.lock` at root → `yarn install --frozen-lockfile` true,
  the pnpm and npm rows false.
- Happy path: `pnpm-lock.yaml` + `package-lock.json` both present →
  `pnpm -r install` true, `npm ci` false (pnpm wins).
- Happy path: `uv.lock` at root → `uv sync` true, JS rows independent.
- Happy path: root `package.json` with `scripts.cf-typegen` + `pnpm-lock.yaml` →
  `pnpm -r run cf-typegen` true.
- Edge case: root `package.json` exists with no `scripts` object →
  script-driven rows all false, lockfile-driven rows independent.
- Edge case: root `package.json` is unparseable (`{not json`) →
  script-driven rows all false; no error bubbled (parse failure for an
  optional signal is silent).
- Edge case: nested `package.json` inside `apps/api/` with
  `cf-typegen` script + no root `package.json` → `pnpm -r run cf-typegen`
  false (only root is parsed).

**Verification:**
- `cargo test -p superset-setup repo_detect::` passes.
- Adding a temp `pnpm-lock.yaml` and root `package.json` with a
  `cf-typegen` script returns the expected bool vector aligned to
  `repo_detect::OPTIONS`.

---

- U2. **Config.json read/write/merge semantic shift**

**Goal:** Move `config.json` preservation from "skip-when-present in
`copy_into_repo`" to "read disk → merge picker output with preserved
teardown/run → stage merged Config → always copy". Adds the
`load_config` reader, renames the writer to always-rewrite shape, and
adjusts `copy_into_repo` and `MaterializeReport` accordingly.

**Requirements:** R6, R8

**Dependencies:** None (independent of U1)

**Files:**
- Modify: `projects/superset-setup/src/superset_files.rs`
- Test: `projects/superset-setup/src/superset_files.rs` (inline
  `#[cfg(test)] mod tests`)

**Approach:**
- Add `pub fn load_config(root: &Path) -> Result<Option<Config>>`
  mirroring `load_setup_config` — `Ok(None)` when absent, error when
  malformed.
- Extend `ExistingState` with `config_json: Option<Config>` and
  populate it in `load_existing` alongside `setup_config_json`.
- Replace `write_config_json_if_absent(root) -> Result<bool>` with
  `pub fn write_config_json(root: &Path, cfg: &Config) -> Result<()>`
  that always writes pretty-printed JSON with a trailing newline.
  Callers that previously wanted the "if absent" semantic now do the
  read-then-merge upstream.
- Add `pub fn merge_setup_into_config(existing: Option<&Config>, new_setup: Vec<String>) -> Config`
  — a pure helper that returns a new `Config` with `setup = new_setup`,
  `teardown` and `run` cloned from `existing` (or empty when `existing`
  is `None`).
- Update `copy_into_repo`:
  - Remove the "copy only when absent" branch for `config.json`.
  - Copy the staged `config.json` unconditionally (same shape as
    `setup_config.json`).
- Update `MaterializeReport`: drop `wrote_config`. Keep `wrote_envrc`.
- Drop `Config::default_contract()` (no remaining call sites after
  `bootstrap_flow` updates in U3); leave a note in the module-level
  doc if removal would be confusing for readers.

**Patterns to follow:**
- `write_setup_config_json` + `load_setup_config` — the always-rewrite
  + preservation-up-front idiom this module already encodes for
  `setup_config.json`.
- `existing_unknown_patterns` — the pure-merge-helper shape.

**Test scenarios:**
- Happy path: `load_config` returns `Ok(None)` when
  `.superset/config.json` is absent.
- Happy path: `load_config` round-trips a hand-written
  `config.json` with non-default `setup`, `teardown`, `run` arrays.
- Error path: malformed `config.json` surfaces as an error mentioning
  `config.json` and `malformed JSON` (mirrors the
  `malformed_setup_config_returns_clean_error` test for the sibling file).
- Happy path: `merge_setup_into_config` with `existing = None` returns
  a Config with the new setup and empty teardown/run.
- Happy path: `merge_setup_into_config` with an existing Config
  carrying non-empty teardown/run returns a Config with the new setup
  and verbatim-preserved teardown/run.
- Happy path: `write_config_json` writes pretty-printed JSON with a
  trailing newline and is round-trip-parseable by `load_config`.
- Integration: `copy_into_repo` over-writes an existing
  `config.json` with the staged content (the previous
  `copy_into_repo_preserves_existing_config_json` test is replaced
  with this).
- Integration: a full bootstrap simulation — pre-existing
  `config.json` with `teardown: ["./drop.sh"]`, picker output
  `["./.superset/setup.sh", "uv sync"]` → final on-disk
  `config.json` has the new setup and the original teardown preserved.

**Verification:**
- `cargo test -p superset-setup superset_files::` passes; the test
  count matches the deletions/additions plan; no test silently
  references the dropped `wrote_config` field.

---

- U3. **Shared action-loop helper, new `pick_setup_commands`, bootstrap wiring**

**Goal:** Extract the `pick_patterns` action-loop body into a shared
`ui` helper, add `pick_setup_commands` as a second wrapper over it, and
wire the new step into `bootstrap_flow` so it runs between the patterns
picker and the `.envrc` prompt.

**Requirements:** R1, R2, R4, R5, R7, R9

**Dependencies:** U1, U2

**Files:**
- Modify: `projects/superset-setup/src/ui.rs`
- Modify: `projects/superset-setup/src/main.rs`
- Test: `projects/superset-setup/src/ui.rs` (inline
  `#[cfg(test)] mod tests`)

**Approach:**
- In `ui.rs`, lift the `loop { … }` body of `pick_patterns` into a
  new free function: `pick_with_actions(prompt, help, rows,
  add_prompt_label, add_prompt_help, validator, dim_for_new_row) ->
  Result<Vec<String>>`. The function owns the cursor state, the
  `Action` enum, and the row-rendering. Callers supply:
  - The initial `Vec<Row>` (a shared shape with `raw: String`,
    `checked: bool`, `dim_suffix: Option<&'static str>`).
  - The `+ Add new …` prompt label and help.
  - A validator closure (`Fn(&str, &[String]) -> Result<(), String>`)
    that wraps inquire's validation contract.
  - A closure (`Fn(&str) -> Result<Option<&'static str>>`) that decides
    whether a newly added row carries a dim suffix and what label it
    uses.
- The existing `PatternRow` struct in `ui.rs` is **replaced** by the
  shared `Row` shape (`raw: String`, `checked: bool`,
  `dim_suffix: Option<&'static str>`). The `Action` enum and
  `render_row` move alongside the helper. The wrapper rewrites of
  `pick_patterns` and `pick_setup_commands` use the new `Row` directly;
  `PatternRow` does not survive the refactor.
- **Esc behavior** for the new picker mirrors `pick_patterns`: the
  top-level `Select` propagates the inquire error via
  `.context("setup command selection cancelled")?`, which exits the
  process (same semantic as Ctrl+C). Esc inside the
  `+ Add new command…` sub-prompt returns to the AddNew row (already
  handled by `prompt_skippable`).
- **Cursor on entry** mirrors `pick_patterns`: start on the first
  unchecked row; when every row is preselected (a well-configured repo
  with detected pnpm + uv + a `cf-typegen` script), the cursor lands
  past the rows onto `✔ Done`. This is intentional — the user can
  commit the picker immediately with Enter.
- Rewrite `pick_patterns` as a thin wrapper that:
  - Builds the initial rows from `(options, preselected,
    fs_match)` and passes `"(no matches)"` as the dim suffix where
    `!fs_match[i]`.
  - Passes the existing `validate_pattern` (glob-syntax + duplicate)
    as the validator.
  - Passes a closure that calls `repo_scan::pattern_matches_any` to
    decide the dim suffix for the new row.
- Add `pub fn pick_setup_commands(options: &[String], preselected: &[usize], detected: &[bool]) -> Result<Vec<String>>`:
  - Builds initial rows; dim suffix is `Some("(not detected)")` where
    `!detected[i]`, `None` otherwise.
  - Validator is `validate_command` — trim, reject empty, reject
    duplicate-of-taken. No glob/shell checks.
  - The new-row dim-suffix closure always returns `Ok(None)` —
    user-typed commands carry no detection signal.
  - Sub-prompt help message reads: `e.g. make setup or ./scripts/build.sh — runs from the workspace root on superset apply`.
- In `main.rs:bootstrap_flow`:
  - After `existing` is loaded (now also populated with
    `config_json`), compute the command-picker inputs:
    - `cmd_options` = `repo_detect::OPTIONS` extended with
      `existing_unknown_setup_commands` (computed via a small inline
      filter mirroring `existing_unknown_patterns`, or — if reused —
      a generalized helper added in U2). Custom entries appear
      **after** the known options (mirroring the pattern picker's
      precedent, where unknowns sit between the preconfigured set and
      the `+ Add new… / ✔ Done` sentinels). Custom entries carry no
      distinguishing visual marker — they look identical to known
      options with `None` dim-suffix; the user identifies them by
      content.
    - `cmd_detected` from `repo_detect::detect_for_options`, extended
      with `false` entries for the unknown-tail.
    - `cmd_preselected` = indices where detection is true OR the
      option already appears in the existing `config.json` setup
      array.
  - Call `ui::pick_setup_commands` after `ui::pick_patterns` and
    before `superset_files::should_offer_envrc`.
  - Build the merged Config via
    `superset_files::merge_setup_into_config(existing.config_json.as_ref(), chosen_commands)`
    and stage it with `write_config_json` (replacing
    `write_config_json_if_absent`).
  - Adjust the post-materialize messaging to use
    `existing.superset_dir_present` as the "Wrote vs Updated"
    discriminator now that `MaterializeReport::wrote_config` is gone.
  - When the staged `config.json` is byte-identical to the existing
    on-disk file (i.e., the merge produced no semantic change), emit a
    "Setup commands unchanged — config.json rewritten with no changes"
    info line instead of the standard "Updated" message, so users
    re-running bootstrap with no toggle changes get a clear no-op
    signal. The byte-equality check happens against the existing on-disk
    content captured by `existing.config_json` after pretty-printing the
    merged Config the same way `write_config_json` will.

**Patterns to follow:**
- `inquire-action-loop-2026-05-26.md` — the documented contract for
  the Select-loop shape. The shared helper must preserve cursor
  persistence on toggle, land-on-new-row after `AddNew`,
  back-to-AddNew on Esc, and the `with_starting_cursor` clamp.
- `pick_patterns` pre-extraction — anything the wrapper does today
  that the new helper does not handle (e.g., row construction from
  options + preselected + fs-match) stays in the wrapper.

**Test scenarios:**
- Happy path: `validate_command` accepts a typical command string
  like `pnpm -r install`.
- Happy path: `validate_command` accepts a single-token command like
  `make`.
- Edge case: `validate_command` rejects an empty string with a
  message that mentions "empty".
- Edge case: `validate_command` rejects whitespace-only input the
  same way (caller trims first; verify the wrapper or validator
  matches the trim discipline).
- Edge case: `validate_command` rejects a duplicate-of-taken with a
  message that mentions "already".
- Pattern-picker regression: existing `validate_pattern` tests still
  pass after the wrapper rewrite (they target a wrapper-internal
  function, not the helper).
- Test expectation: none for `pick_with_actions` and
  `pick_setup_commands` themselves -- both are interactive `inquire`
  loops that the existing module does not unit-test (`pick_patterns`
  is similarly not tested). Validate manually during smoke per the
  module's existing convention.

**Verification:**
- `cargo test -p superset-setup` passes end-to-end.
- Manual smoke in a checkout with `pnpm-lock.yaml` + root
  `package.json` containing `scripts.cf-typegen` shows the
  `pnpm -r run cf-typegen` row preselected.
- Manual smoke in a checkout with `uv.lock` shows `uv sync`
  preselected.
- Manual smoke in an empty checkout shows only `./.superset/setup.sh`
  preselected; all other rows visible with `(not detected)`.
- Esc at the new picker leaves the working tree untouched (the
  staging discipline carries through).

---

## System-Wide Impact

- **Interaction graph:** `bootstrap_flow` gains one new prompt between
  the patterns picker and the `.envrc` prompt. Apply mode is
  untouched.
- **State lifecycle risks:** The shift from "preserve config.json
  on-disk" to "always rewrite from merged Config" means a corrupted or
  malformed pre-existing `config.json` now errors at the `load_existing`
  step rather than being silently preserved. This is the same error
  surface `setup_config.json` already exposes — symmetric and the
  cleaner default.
- **API surface parity:** The shared helper is a private symbol within
  `ui.rs`; external callers see only the two wrapper functions.
- **Integration coverage:** A bootstrap-simulation test in U2 covers
  the read-merge-write cycle end-to-end.
- **Unchanged invariants:** `setup.sh` embedded body, `setup_config.json`
  preservation, `.envrc` offering rule, final-action menu, and the
  capture-then-stage-then-materialize discipline. Apply-mode behavior is
  unchanged.

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Existing `config.json` files with hand-edited `setup` arrays get rewritten in a way users don't expect. | Edit-mode preselects ensure any current `setup` entry surfaces as a checked row by default — re-running bootstrap with no toggle changes produces the same `setup` array. Custom user-typed entries surface as deselectable rows above the sentinels. |
| Malformed pre-existing `config.json` becomes a hard error where it used to be silently preserved. | Document in the bootstrap section of `README.md`. The same error surface already applies to `setup_config.json`; symmetry is the point. |
| Recognized-scripts list (just `cf-typegen` in v1) misses common script names users would expect. | The list is a const array in `repo_detect.rs`; extending it is a one-line change. The `+ Add new command…` path covers anything not in the list. Conservative-by-default trades a known under-detection for protection against high false-positive rate on generic names like `setup` and `codegen`. |

---

## Documentation / Operational Notes

- Update `projects/superset-setup/README.md`:
  - Add a bullet to the bootstrap-mode section describing the new
    setup-commands picker, the detection signals, and the
    preservation rule for `teardown` / `run`.
  - Note that `config.json` is now always rewritten in bootstrap
    mode (with teardown/run preserved); update the "Re-run behavior"
    section accordingly.
- Update `projects/superset-setup/CLAUDE.md`:
  - `superset_files.rs` summary picks up `load_config`,
    `write_config_json`, `merge_setup_into_config`; drop
    `write_config_json_if_absent`.
  - Add a one-liner for `repo_detect.rs` mirroring the existing
    `repo_scan.rs` line.
  - `ui.rs` summary picks up `pick_setup_commands` and the shared
    `pick_with_actions` action-loop helper; note that `PatternRow`
    was replaced by the shared `Row` shape.

---

## Sources & References

- Existing CLI plan: `projects/superset-setup/docs/plans/2026-05-25-001-feat-superset-setup-cli-plan.md`
- Action-loop pattern: `projects/superset-setup/docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`
- Related code:
  - `projects/superset-setup/src/ui.rs:pick_patterns`
  - `projects/superset-setup/src/repo_scan.rs:matches_for_patterns`
  - `projects/superset-setup/src/superset_files.rs:Config`,
    `existing_unknown_patterns`, `write_config_json` (replaces
    `write_config_json_if_absent`), `copy_into_repo`
  - `projects/superset-setup/src/main.rs:bootstrap_flow`
