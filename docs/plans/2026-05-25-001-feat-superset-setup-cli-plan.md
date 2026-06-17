---
title: "feat: superset-setup interactive Rust CLI"
type: feat
status: completed
date: 2026-05-25
---

# feat: superset-setup interactive Rust CLI

## Summary

A new monorepo project `projects/superset-setup/` providing an interactive Rust CLI with two modes:

- **Bootstrap mode** (main checkout, on `main`): emits `.superset/config.json`, `.superset/setup.sh`, and `.superset/setup_config.json`; lets the user multi-select copy patterns (with preselects driven by filesystem scan); optionally writes `.envrc` (`dotenv_if_exists`) when `.env` exists without one; finishes with a commit-and-push / feature-branch+PR / done menu. Re-runs operate in edit mode against the existing `setup_config.json`.
- **Apply mode** (worktree or non-`main` branch): does NOT write to `.superset/` in the current checkout. Instead, locates the main checkout's `.superset/setup_config.json`, confirms with the user, then copies the configured files from the main checkout into the current working tree — the same effect `setup.sh` would have, but driven from the worktree.

Color palette mirrors `.superset/setup.sh` for visual continuity: gray for info/details, bold green for success, bold red for errors, bold orange (256-color 208) for warnings, cyan for section headers.

---

## Problem Frame

The monorepo's Superset workspace contract requires three files under `.superset/` (`config.json`, `setup.sh`, `setup_config.json`) in the source repo's main branch. Today these are hand-authored, which is repetitive and easy to get wrong (missing patterns, mismatched setup.sh versions, accidentally committing from a worktree). A first-class interactive bootstrapper removes that friction and gives a single canonical source for the `setup.sh` body.

---

## Requirements

- R1. Tool is a Rust binary under `projects/superset-setup/` built with `cargo`.
- R2. Detects whether the cwd is the main checkout or a linked worktree, and which branch HEAD points at, to choose between bootstrap mode and apply mode (no hard fail on worktree / non-`main`).
- R3. **Bootstrap mode** is entered when invoked from the main checkout AND HEAD is on `main` (or `master` fallback). Apply mode is entered otherwise (worktree, or non-main branch in the main checkout).
- R4. Bootstrap mode: creates `.superset/` if absent; if present, enters edit mode against the existing `setup_config.json`.
- R5. Emits `.superset/setup.sh` from a canonical copy embedded in the binary (sourced from the existing `.superset/setup.sh` at the repo root, moved into the project as a build-time asset).
- R6. Emits `.superset/config.json` with the contract shape (`setup: ["./.superset/setup.sh"]`, `teardown: []`, `run: []`) when not already present; preserves the existing one on re-run.
- R7. Presents a multi-select menu for copy patterns with the preconfigured options `.env`, `**/.env`, `.env.local`, `**/.dev.vars`; preselects each option that matches at least one file in the repo at scan time.
- R8. Writes the chosen patterns to `.superset/setup_config.json` under the `files` array.
- R9. If `.env` exists at the repo root and `.envrc` does not, prompts whether to create `.envrc` containing `dotenv_if_exists`.
- R10. After configuration, presents a final action menu: "Commit and push to main branch", "Create feature branch, commit and open a PR", "Done for now". Acts on the selection (shells out to `git` and `gh`).
- R11. Edit-mode re-runs preselect patterns from the existing `setup_config.json` (union with filesystem-driven preselects) and skip the final commit step when no on-disk changes resulted.
- R12. Provides a `Makefile` with `clean`, `build`, and `install` targets, with recipes inlined (no auxiliary shell scripts).
- R13. **Apply mode:** when entered, locates the main checkout's `.superset/setup_config.json`. If it does not exist, fails with a clear hint to run bootstrap mode first. If it does exist, shows the configured patterns and asks the user to confirm copying.
- R14. **Apply mode:** on confirm, copies the files matched by `setup_config.json` from the main checkout into the current working tree, applying the same glob/exclude semantics as `.superset/setup.sh` (no shell-out — re-implemented in Rust, embedded `setup.sh` is the spec). Never writes to `.superset/` in the current worktree. Never invokes git in apply mode.
- R15. CLI styling: consistent palette across both modes (gray info, bold green success, bold red error, bold orange (208) warning, cyan section headers). Honors `NO_COLOR` and non-TTY stdout (disables ANSI escapes), matching `setup.sh`'s behavior.

---

## Scope Boundaries

- Not a generic project scaffolder — only writes the `.superset/` contract and optional `.envrc` in bootstrap mode, and only copies configured files in apply mode.
- Does not modify `setup.sh` content per-invocation; the embedded copy is the source of truth.
- Does not implement the workspace runtime that consumes `.superset/config.json` (that's the existing Superset app).
- Does not support non-`main` default branches beyond a `master` fallback.
- Does not manage `gh auth` — assumes `gh` is authenticated when the PR option is chosen.
- Does not support custom user-typed glob patterns in v1 (only toggling the four preconfigured options); pre-existing entries in `setup_config.json` are preserved verbatim across the multi-select.
- Apply mode does not invoke `.superset/setup.sh` as a subprocess — file-copy semantics are re-implemented in Rust so the tool stays portable (no bash 4 / jq runtime requirement on the user side) and matches the embedded `setup.sh` spec.

### Deferred to Follow-Up Work

- Custom user-entered glob patterns beyond the four preconfigured options.
- Non-interactive / `--yes` flag mode for CI.
- Teardown/run command authoring in `config.json`.

---

## Context & Research

### Relevant Code and Patterns

- `.superset/setup.sh` (repo root) — canonical bash setup script, will be relocated to `projects/superset-setup/assets/setup.sh` and embedded via `include_str!`.
- `.superset/config.json`, `.superset/setup_config.json` (repo root) — shape references for emitted files.
- `projects/workon/` — sibling Rust-adjacent project (Go) demonstrating monorepo conventions: `Makefile`, `README.md`, `CLAUDE.md`, source under `src/`.
- `scripts/new-project.py` — creates `projects/<name>/` and registers in the workspace file; will be used once to scaffold the directory entry.
- `CLAUDE.md` (repo root) — monorepo conventions: projects independent, no shared deps.

### Institutional Learnings

- None directly applicable; this is a greenfield Rust project in a monorepo where most projects are bash/Go/Python.

### External References

- `inquire` crate docs — `MultiSelect` supports default-selected indices via `.with_default()`, `Select` for the final menu, `Confirm` for the `.envrc` prompt.
- `globset` / `glob` crates — for evaluating the four preconfigured patterns against the working tree to drive preselects.

---

## Key Technical Decisions

- **Interactive TUI: `inquire`.** Modern ergonomics, first-class `MultiSelect` with default-selected indices, no terminal-mode juggling.
- **Git/PR ops: shell out to `git` and `gh`.** No `git2` dependency. Matches monorepo conventions; `gh pr create` for the PR path.
- **`.envrc` body: `dotenv_if_exists`.** Safer than bare `dotenv` for checkouts where `.env` may be absent.
- **Re-run posture: edit mode.** Load existing `setup_config.json`, preselect from union of `existing patterns ∩ four options` and `filesystem matches`. Preserve unknown entries in `files` verbatim.
- **`setup.sh` source of truth: embed at compile time** via `include_str!("../assets/setup.sh")`. Single canonical body; no fetch at runtime.
- **Worktree detection:** compare `git rev-parse --git-dir` to `git rev-parse --git-common-dir`; if they differ, this is a worktree. The main checkout's path is the parent of `git-common-dir`. Also infer worktree when `<repo>/.git` is a file rather than a directory.
- **Mode routing:** main checkout + on `main` (or `master` fallback) → bootstrap. Worktree OR non-main HEAD in the main checkout → apply. Detached HEAD in the main checkout → error with a clear message (ambiguous intent).
- **Main-branch detection:** read `git symbolic-ref --short HEAD`; prefer `main`, fall back to `master` only if `main` does not exist as a local ref in the main checkout.
- **Apply-mode glob semantics:** re-implement `setup.sh`'s walk in Rust using `globset` plus the same `DEFAULT_EXCLUDES` (`node_modules`, `.venv`). Reject absolute paths and `..` segments, same as the bash version. Same skip-counting and reporting (copied / skipped tallies).
- **Styling: `owo-colors` for ad-hoc output + `inquire::ui::RenderConfig` for prompt theming.** Single palette module (`style.rs`) exposes constants matching `setup.sh`: `INFO` (gray/dim), `OK` (bold green), `ERR` (bold red), `WARN` (bold orange = 256-color 208), `HEADER` (cyan), `PATH` (default; unstyled, easy to copy). `inquire` prompts use cyan for prompt prefix, green for selected items, dim gray for help text. Auto-disable when stdout is not a TTY or `NO_COLOR` is set (`owo-colors` supports `supports-color` integration; `inquire` already respects `NO_COLOR`).
- **Layered architecture:** pure logic (git probes, pattern scanning, JSON read/write, `.envrc` writer, apply-mode walk) isolated from the interactive layer so the pure functions are unit-testable.
- **Build system: `Makefile` with inlined recipes** wrapping `cargo` (`build` → `cargo build --release`; `install` → `cargo install --path .`; `clean` → `cargo clean`). No shell scripts.

---

## Open Questions

### Resolved During Planning

- TUI crate → `inquire`.
- Git/PR ops → shell out to `git` + `gh`.
- `.envrc` content → `dotenv_if_exists`.
- Re-run behavior → edit mode.

### Deferred to Implementation

- Exact `inquire` API surface for preselected `MultiSelect` defaults (set via `.with_default(&[indices])`); will be confirmed against the installed version's docs.
- Whether to colorize output via `console`/`owo-colors` or rely on `inquire`'s built-in styling.
- Exact phrasing of prompt strings.

---

## Output Structure

    projects/superset-setup/
    ├── Cargo.toml
    ├── Makefile
    ├── README.md
    ├── CLAUDE.md
    ├── .gitignore
    ├── assets/
    │   └── setup.sh                 # canonical, embedded via include_str!
    └── src/
        ├── main.rs                  # CLI entry, mode routing
        ├── git.rs                   # worktree / branch / main-checkout discovery / commit / push / PR
        ├── repo_scan.rs             # glob matching for bootstrap preselects
        ├── apply.rs                 # apply-mode: walk main checkout, copy matched files into cwd
        ├── superset_files.rs        # read/write config.json, setup_config.json, setup.sh, .envrc
        ├── style.rs                 # color palette + inquire RenderConfig
        └── ui.rs                    # inquire wrappers (MultiSelect, Confirm, Select)

---

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification.*

Mode routing:

    main()
      ├─ git::probe()                       # { mode_repo, current_branch, main_checkout_path, is_worktree }
      └─ match mode:
           Bootstrap  → bootstrap_flow(repo_root)
           Apply      → apply_flow(cwd, main_checkout_path)
           Error      → exit non-zero with hint (e.g. detached HEAD)

Bootstrap flow:

      ├─ superset_files::load_or_init()     # detect .superset/, load setup_config.json if present
      ├─ repo_scan::matches_for_options()   # which of the 4 patterns match >=1 file
      ├─ ui::pick_patterns(preselects)      # MultiSelect, returns chosen Vec<String>
      ├─ superset_files::write_all()        # emit setup.sh, config.json, setup_config.json
      ├─ if .env exists && !.envrc:
      │     ui::confirm_envrc() → write .envrc
      └─ ui::pick_final_action()
            ├─ CommitPushMain  → git::commit_all + git::push("main")
            ├─ FeatureBranchPR → git::create_branch + git::commit_all + git::push_upstream + gh::pr_create
            └─ Done            → exit 0

Apply flow:

      ├─ load main_checkout/.superset/setup_config.json (error with hint if missing)
      ├─ render configured patterns to user (HEADER + dimmed list)
      ├─ ui::confirm_apply(dest = cwd, source = main_checkout)
      └─ if confirmed:
            apply::run(source = main_checkout, dest = cwd, patterns)
            # walks source, applies DEFAULT_EXCLUDES, copies into dest
            # prints "Copied: <path>" (dim) / "Skipped (...)" lines and a summary

Color palette (`style.rs`):

    INFO    = dim / gray            # paths, "Copied:" details, help text
    OK      = bold green            # success summary, confirmations
    WARN    = bold orange (208)     # non-fatal skips that count
    ERR     = bold red              # failures, rejected patterns
    HEADER  = bold cyan             # section titles ("Apply Superset config", "Bootstrap")
    PATH    = unstyled              # file paths in prompts (terminal-default for easy copy)

    Disabled when: NO_COLOR set, stdout is not a TTY, or terminal lacks color support.
    `inquire::ui::RenderConfig` is built from the same palette so prompts and ad-hoc output
    feel like one UI rather than two.

Preselect computation:

    preselected = union(
      filesystem_matches(four_options),
      existing_setup_config.files ∩ four_options
    )

Unknown entries already in `setup_config.json.files` (outside the four options) are preserved across the rewrite.

---

## Implementation Units

- U1. **Project scaffold and Makefile**

**Goal:** Create the `projects/superset-setup/` directory, register it in the monorepo workspace file, and add a buildable Rust skeleton with a `Makefile`.

**Requirements:** R1, R12

**Dependencies:** None

**Files:**
- Create: `projects/superset-setup/Cargo.toml`
- Create: `projects/superset-setup/src/main.rs` (placeholder `fn main()`)
- Create: `projects/superset-setup/Makefile`
- Create: `projects/superset-setup/README.md`
- Create: `projects/superset-setup/.gitignore` (`/target`)
- Modify: `personal.monorepo-general.code-workspace` (add folder entry via `scripts/new-project.py`)

**Approach:**
- Run `scripts/new-project.py superset-setup` to scaffold the directory and register it; then overwrite the generated `README.md` and add the Rust files.
- `Cargo.toml`: edition 2021, deps `inquire`, `serde`, `serde_json`, `globset`, `walkdir`, `owo-colors`, `supports-color`, `anyhow`.
- `Makefile` targets (recipes inlined, no scripts):
  - `build:` → `cargo build --release`
  - `install:` → `cargo install --path .`
  - `clean:` → `cargo clean`
  - `.PHONY: build install clean`

**Patterns to follow:**
- `projects/workon/Makefile` for monorepo Makefile style.

**Test scenarios:**
- Happy path: `make build` produces a binary; `cargo check` is clean.
- Happy path: `make clean` removes `target/`.

**Verification:**
- `cargo build` succeeds; binary runs and exits 0 (still a stub).

---

- U2. **Git environment probe and mode routing**

**Goal:** Read-only git probes that return a structured `Mode` (Bootstrap, Apply, or a hard error) plus the paths needed by each mode.

**Requirements:** R2, R3

**Dependencies:** U1

**Files:**
- Create: `projects/superset-setup/src/git.rs`
- Modify: `projects/superset-setup/src/main.rs` (wire `probe()` → `match` at startup)
- Test: `projects/superset-setup/src/git.rs` (`#[cfg(test)]` module)

**Approach:**
- `cwd_repo_root()` — `git rev-parse --show-toplevel` (the working tree containing cwd; may be the main checkout or a linked worktree).
- `is_worktree()` — true when `git rev-parse --git-dir` ≠ `git rev-parse --git-common-dir`, or when `<cwd_repo_root>/.git` is a regular file.
- `main_checkout_root()` — derived from `git rev-parse --git-common-dir`: take its parent (the directory containing the shared `.git`). Canonicalize.
- `current_branch_in(path)` — `git -C <path> symbolic-ref --short HEAD`; returns `None` on detached HEAD.
- `main_branch_name(main_root)` — returns `"main"` if it exists as a local ref, else `"master"` if it does, else error.
- `probe() -> Mode` where:
  - `Mode::Bootstrap { repo_root }` — not a worktree AND current branch == main branch name.
  - `Mode::Apply { cwd_root, main_checkout }` — worktree, OR same repo but branch ≠ main branch name. `main_checkout` is `main_checkout_root()`.
  - `Mode::Error(message)` — detached HEAD in the main checkout (ambiguous), or cwd is not inside a git repo.
- All git invocations route through a small `Command` helper that surfaces stderr verbatim in errors.

**Patterns to follow:**
- Keep the `Command` invocation helper local; do not pull in `git2`.

**Test scenarios:**
- Happy path: main checkout on `main` → `Mode::Bootstrap`.
- Happy path: linked worktree on a feature branch → `Mode::Apply { main_checkout = parent of git-common-dir }`.
- Happy path: main checkout on a feature branch → `Mode::Apply { main_checkout = repo root }`.
- Edge case: detached HEAD in main checkout → `Mode::Error` mentioning "detached".
- Edge case: repo has only `master` (no `main`); on `master` → `Mode::Bootstrap`.
- Error path: cwd not inside a git repo → `Mode::Error`.

**Verification:**
- Unit tests use `tempfile` + shell-invoked `git init` and `git worktree add` to construct fixtures.

---

- U3. **`.superset/` file emission and edit-mode load**

**Goal:** Read existing `.superset/` artifacts (if any), and emit `config.json`, `setup.sh`, `setup_config.json` correctly on both first-run and edit-mode paths.

**Requirements:** R4, R5, R6, R8, R11

**Dependencies:** U2

**Files:**
- Create: `projects/superset-setup/src/superset_files.rs`
- Create: `projects/superset-setup/assets/setup.sh` (copy of the existing repo-root `.superset/setup.sh`)
- Modify: `projects/superset-setup/src/main.rs`
- Test: `projects/superset-setup/src/superset_files.rs` (`#[cfg(test)]` module)

**Approach:**
- `SETUP_SH: &str = include_str!("../assets/setup.sh");` — single source of truth.
- Define typed structs (`serde`) for `Config { setup, teardown, run }` and `SetupConfig { files: Vec<String> }`.
- `load_existing(root: &Path) -> ExistingState` — returns presence flags and parsed content when files exist.
- `write_setup_sh` — always writes (and `chmod 0755` via `std::os::unix::fs::PermissionsExt`); overwrite ensures the embedded canonical body is current.
- `write_config_json` — only writes when missing (preserve user edits on re-run).
- `write_setup_config_json(patterns: Vec<String>)` — pretty-printed (`serde_json::to_string_pretty`) with trailing newline.
- Preservation rule: when rewriting `setup_config.json`, keep entries from the existing `files` that are NOT in the four preconfigured options, appended after the user's selection in their original order.

**Patterns to follow:**
- Reference shape from repo-root `.superset/config.json` and `.superset/setup_config.json`.

**Test scenarios:**
- Happy path: fresh repo emits all three files with expected content; `setup.sh` is executable.
- Happy path: re-run with existing `config.json` does NOT overwrite it.
- Edge case: existing `setup_config.json.files` contains a non-preconfigured entry (e.g. `apps/*/config`) — it survives a rewrite.
- Edge case: malformed `setup_config.json` returns a clean error instead of panicking.
- Error path: `.superset/` exists as a regular file (not a directory) → clear error.

**Verification:**
- Unit tests using `tempfile` for first-run, re-run-preserve, and unknown-entry-preservation cases.

---

- U4. **Filesystem scan for pattern preselects**

**Goal:** Determine which of the four preconfigured glob patterns have ≥1 match in the repo, to drive multi-select defaults.

**Requirements:** R7

**Dependencies:** U2

**Files:**
- Create: `projects/superset-setup/src/repo_scan.rs`
- Test: `projects/superset-setup/src/repo_scan.rs` (`#[cfg(test)]` module)

**Approach:**
- Constant `OPTIONS: [&str; 4] = [".env", "**/.env", ".env.local", "**/.dev.vars"];`
- `matches_any(root: &Path, pattern: &str) -> bool` using `globset::Glob` + a bounded walker.
- Walker: `walkdir` (or hand-rolled `std::fs::read_dir` recursion) that skips `node_modules`, `.venv`, `.git`, `target` to match `setup.sh`'s `DEFAULT_EXCLUDES` spirit and keep scans fast.
- Returns `[bool; 4]` aligned to `OPTIONS`.

**Test scenarios:**
- Happy path: temp repo with `.env` at root → `.env` and `**/.env` both true; `.env.local` and `**/.dev.vars` false.
- Happy path: `apps/api/.env` only → `**/.env` true, `.env` false.
- Edge case: `node_modules/foo/.env` is ignored.
- Edge case: empty repo → all false.

**Verification:**
- Unit tests cover the four options across single and nested layouts.

---

- U5. **Color palette and `inquire` theming**

**Goal:** Centralize colors and prompt theming in one module so the whole CLI feels like a single, consistent UI.

**Requirements:** R15

**Dependencies:** U1

**Files:**
- Create: `projects/superset-setup/src/style.rs`
- Modify: `projects/superset-setup/src/main.rs` (initialize style once at startup; install `inquire` global render config)

**Approach:**
- Color decision: `supports_color::on(Stream::Stdout)` AND `std::env::var_os("NO_COLOR").is_none()` → enabled, else disabled. Set process-wide once.
- Palette helpers using `owo-colors` (call-site ergonomics: `s.info()`, `s.ok()`, `s.warn()`, `s.err()`, `s.header()`):
  - `INFO` — dim/gray (`BrightBlack`).
  - `OK` — bold green.
  - `WARN` — bold orange (`Color::Rgb` or `XtermColors::DarkOrange`, 256-color 208 fallback).
  - `ERR` — bold red.
  - `HEADER` — bold cyan.
  - `PATH` — unstyled (terminal-default; preserves copy-paste).
- `inquire::ui::RenderConfig`:
  - `prompt_prefix` — cyan `?`.
  - `answered_prefix` — green `✓`.
  - `selected_option` — green.
  - `highlighted_option_prefix` — cyan `›`.
  - `help_message` — dim gray.
  - `error_message` — bold red.
  - When color is disabled: pass `RenderConfig::empty()`.
- Section header helper: prints a one-line cyan banner like `── Bootstrap mode ──` between major phases.
- All emitted output goes through this module so palette tweaks are one-file changes.

**Patterns to follow:**
- `.superset/setup.sh` palette (gray / bold red / bold green / bold orange 208) — match those tones for visual continuity.

**Test scenarios:**
- Happy path: with `NO_COLOR=1`, all helpers produce escape-free strings; `RenderConfig::empty()` is used.
- Happy path: with a TTY and no `NO_COLOR`, helpers emit ANSI escapes for each role.
- Edge case: piped stdout disables color even without `NO_COLOR`.

**Verification:**
- Unit tests against the palette helpers using a forced-on / forced-off flag.

---

- U6. **Apply mode: read source config and copy files**

**Goal:** Implement apply-mode end-to-end — load the main checkout's `setup_config.json`, confirm with the user, then copy matched files from the main checkout into cwd.

**Requirements:** R13, R14

**Dependencies:** U2, U3, U5

**Files:**
- Create: `projects/superset-setup/src/apply.rs`
- Modify: `projects/superset-setup/src/main.rs` (wire `Mode::Apply` branch)
- Test: `projects/superset-setup/src/apply.rs` (`#[cfg(test)]` module)

**Approach:**
- `load_main_config(main_root) -> Result<SetupConfig>` — reuses `superset_files::load_setup_config`; missing file returns a tailored error: "no `.superset/setup_config.json` in <main_root>; run from main checkout to bootstrap first".
- `expand(main_root, patterns) -> Vec<Match>` where `Match { rel_path, kind: File | Dir }`:
  - Same semantics as `setup.sh`: reject absolute and `..` patterns; literal patterns must exist; glob patterns may yield zero matches (logged "Skipped (no matches)", non-fatal).
  - Walk uses `walkdir` + `globset::GlobSet`. Apply `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) as a segment match; matches inside excluded dirs are dropped (logged "Skipped (excluded)" in INFO/dim, not counted).
  - De-duplicate by relative path.
- `copy_all(main_root, dest_root, matches) -> Summary { copied, skipped }`:
  - Files: `mkdir -p` parent of dest, `std::fs::copy`.
  - Dirs: recursive copy (mirrors `cp -R src/. dst/`); preserve permissions where possible.
  - Each result prints `Copied: <rel>` (INFO/dim) or `Skipped (<reason>): <rel>` (ERR/bold-red for missing, WARN/orange for other failures); summary line at end uses OK when `skipped == 0`, WARN otherwise — mirrors `setup.sh`.
- UI orchestration in `main.rs`:
  - HEADER banner "Apply Superset config".
  - Print source (`main_root`) and dest (`cwd_root`) on dim lines.
  - Print the configured patterns as a dim bulleted list.
  - `inquire::Confirm` "Copy these files into the current worktree?" (default Yes).
  - On confirm → `copy_all`. On decline → exit 0 with an INFO line.
- Apply mode never touches git and never writes inside the dest's `.superset/`.

**Patterns to follow:**
- `.superset/setup.sh`'s loop structure, exclusion handling, and summary semantics — preserve them as the spec.

**Test scenarios:**
- Happy path: source has `.env` at root, dest empty → `.env` is copied to dest, no other files touched.
- Happy path: pattern `apps/*/config` matches two directories → both are copied recursively.
- Edge case: pattern `**/.dev.vars` ignores `node_modules/foo/.dev.vars` (excluded) and copies the rest.
- Edge case: glob with zero matches is non-fatal (logged, not counted as skipped).
- Edge case: dest already contains a copy of one of the files → overwritten in place (matches `cp` semantics).
- Error path: absolute pattern (`/etc/foo`) → logged as skipped (red), counted; non-fatal continue.
- Error path: pattern with `..` segment → logged as skipped (red), counted; non-fatal continue.
- Error path: `setup_config.json` missing in main checkout → clear error mentioning bootstrap.

**Verification:**
- Unit tests using `tempfile` for source+dest layouts covering each scenario above.

---

- U7. **Interactive prompts (`inquire` wrappers)**

**Goal:** Provide thin UI functions for the bootstrap multi-select, the `.envrc` confirm, the apply-mode confirm, and the final-action select.

**Requirements:** R7, R9, R10, R13

**Dependencies:** U3, U4, U5

**Files:**
- Create: `projects/superset-setup/src/ui.rs`
- Modify: `projects/superset-setup/src/main.rs`

**Approach:**
- `pick_patterns(options: &[&str], preselected: &[usize], existing_unknown: &[String]) -> Result<Vec<String>>` — wraps `inquire::MultiSelect::new("Files to copy automatically:", options.to_vec()).with_default(preselected)`. Result is the user's selection in option order, with `existing_unknown` appended unchanged.
- `confirm_envrc() -> Result<bool>` — `inquire::Confirm` with default `true`, help text "writes `dotenv_if_exists` to `.envrc`".
- `confirm_apply(src: &Path, dest: &Path) -> Result<bool>` — `inquire::Confirm` with default `true`.
- `pick_final_action() -> Result<FinalAction>` — `inquire::Select` over `[CommitPushMain, FeatureBranchPR, Done]` with display strings matching the spec verbatim.
- All prompts use the global `RenderConfig` from U5.

**Test scenarios:**
- Test expectation: none -- thin TUI wrapper; logic is in callers and pure modules.

**Verification:**
- Manual smoke: run the binary in a scratch repo and a worktree; walk each prompt branch.

---

- U8. **`.envrc` handling**

**Goal:** When `.env` exists at the repo root and `.envrc` does not, ask the user, and write `.envrc` with `dotenv_if_exists` on confirm.

**Requirements:** R9

**Dependencies:** U3, U7

**Files:**
- Modify: `projects/superset-setup/src/superset_files.rs` (add `write_envrc(root)`)
- Modify: `projects/superset-setup/src/main.rs`

**Approach:**
- Check `root.join(".env").is_file() && !root.join(".envrc").exists()`.
- On confirm, write `.envrc` with the single line `dotenv_if_exists\n`.

**Test scenarios:**
- Happy path: `.env` present, no `.envrc` → write produces a file with body `dotenv_if_exists\n`.
- Edge case: `.envrc` already exists → prompt is skipped entirely (no overwrite).
- Edge case: `.env` absent → prompt is skipped.

**Verification:**
- Unit test for `write_envrc`; orchestration covered by manual smoke in U7.

---

- U9. **Final action: commit + push / PR / done**

**Goal:** Execute the chosen finishing action, shelling out to `git` and `gh`.

**Requirements:** R10, R11

**Dependencies:** U2, U3, U8

**Files:**
- Modify: `projects/superset-setup/src/git.rs` (`stage_paths`, `commit`, `push`, `create_branch`, `current_remote`, `pr_create`)
- Modify: `projects/superset-setup/src/main.rs`

**Approach:**
- `stage_paths(&[".superset/", ".envrc"])` — only stages paths that exist on disk; uses `git add -- <path>`.
- `nothing_to_commit()` — `git diff --cached --quiet`; if true, skip commit step and report "no changes".
- `CommitPushMain` → commit with `"chore(superset): bootstrap workspace contract"` (overridable later; not a CLI flag in v1) then `git push origin main`.
- `FeatureBranchPR` → derive branch name `chore/superset-setup-YYYYMMDD-HHMMSS`, `git switch -c <branch>`, commit, `git push -u origin <branch>`, then `gh pr create --fill --base main`. Print the resulting PR URL captured from stdout.
- `Done` → no git operations; print a one-line summary of what was written.

**Test scenarios:**
- Test expectation: none -- thin process-spawn wrappers; correctness validated by manual smoke runs.

**Verification:**
- Manual smoke in a scratch repo for each of the three final actions, including the "nothing to commit" no-op path in edit mode.

---

- U10. **Wire-up, error UX, and README**

**Goal:** Compose all units in `main.rs` (mode dispatch + both flows), ensure user-facing errors are actionable, and document usage.

**Requirements:** R1–R15

**Dependencies:** U1–U9

**Files:**
- Modify: `projects/superset-setup/src/main.rs`
- Modify: `projects/superset-setup/README.md`
- Create: `projects/superset-setup/CLAUDE.md` (brief: build with `make build` / `make install`; layered architecture note)

**Approach:**
- `main.rs` initializes `style` first, then `match git::probe()`:
  - `Mode::Bootstrap { repo_root }` → load existing → scan → multi-select → write all → `.envrc` step → final action.
  - `Mode::Apply { cwd_root, main_checkout }` → load main config → confirm → copy → exit 0.
  - `Mode::Error(msg)` → print bold-red error and exit 1.
- Map `anyhow::Error` to a non-zero exit with a single concise message; no panics on user-recoverable conditions.
- README covers: both modes, install (`make install`), what bootstrap writes, what apply copies, re-run behavior, the `NO_COLOR` env var.

**Test scenarios:**
- Happy path (manual, bootstrap): fresh repo with `.env` and `apps/foo/.dev.vars` → preselects `.env`, `**/.env`, `**/.dev.vars`; `.envrc` prompt appears; final action "Done" leaves working tree dirty.
- Happy path (manual, apply): from a linked worktree → tool prints HEADER "Apply Superset config", shows main checkout path and configured patterns, copies on confirm.
- Happy path (manual, apply): from a feature branch in the main checkout → routed to apply mode (no `.superset/` edits).
- Happy path (manual): edit-mode re-run in bootstrap preserves unknown entries in `setup_config.json.files`.
- Edge case (manual): `NO_COLOR=1 superset-setup` produces escape-free output and unstyled prompts.
- Error path (manual): apply mode with no `.superset/setup_config.json` in main checkout → clear actionable error.
- Error path (manual): detached HEAD in main checkout → clear error, no writes.

**Verification:**
- End-to-end manual run for bootstrap (fresh + edit-mode), apply (worktree + non-main branch), and the error paths above.

---

## System-Wide Impact

- **Interaction graph:** Bootstrap mode shells out to `git` and `gh`; reads/writes under `<repo>/.superset/` and optionally `<repo>/.envrc`. Apply mode reads from the main checkout's `.superset/` and copies files into cwd; does not invoke git or `gh`. No background processes, no daemons.
- **Error propagation:** All fallible operations return `anyhow::Result`; surfaced from `main` as exit code 1 with a single-line message in bold red. No partial writes on probe failures (probe runs first).
- **State lifecycle risks:** Bootstrap edit-mode rewrites `setup_config.json` — must preserve unknown entries (test in U3). `setup.sh` is always overwritten with the embedded canonical body — documented behavior. Apply mode may overwrite files in cwd that exist there already (matches `cp` and `setup.sh` semantics); flagged in README.
- **API surface parity:** None — this is a new binary; nothing else in the repo invokes it.
- **Integration coverage:** Multi-process integration (git/gh) is validated by U10 manual smokes, not unit tests. Apply-mode glob semantics are covered by U6 unit tests against the `setup.sh` spec.
- **Unchanged invariants:** The `.superset/setup.sh` semantics (jq config, globstar/nullglob/dotglob, `DEFAULT_EXCLUDES`) are preserved verbatim by embedding the existing file AND mirrored in the Rust apply-mode implementation.

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Embedded `setup.sh` drifts from the repo-root copy. | Move the canonical file into `projects/superset-setup/assets/setup.sh` and replace the repo-root copy with a pointer in U3. Single source of truth thereafter. |
| Rust apply-mode logic drifts from `setup.sh` semantics. | `setup.sh` is the spec — apply-mode tests in U6 explicitly cover absolute-path rejection, `..` rejection, `DEFAULT_EXCLUDES`, glob-zero-match-is-non-fatal, and dir-recursive-copy to lock parity. |
| `inquire` defaults API differs across versions. | Pin a known-good version in `Cargo.toml`; deferred-implementation note already flags the API check. |
| `gh` not installed or not authenticated. | Detect missing `gh` before `pr_create` and print an actionable hint; fall back to printing the push output so the user can open the PR manually. |
| Filesystem scan slow on huge repos. | Walker skips `node_modules`, `.venv`, `.git`, `target` (matches `setup.sh` exclude spirit). |
| Mode probe misses an edge case (e.g., `git worktree add` on a bare repo, submodule). | Belt-and-suspenders: `.git`-is-file check AND `git-dir` vs `git-common-dir` comparison. Tested in U2 with `git worktree add` fixtures. |
| Color palette unreadable on light terminals or basic 8-color terminals. | Use bright/bold variants for ERR/OK/WARN/HEADER so they remain legible on both light and dark; orange falls back gracefully on 8-color terminals (`owo-colors` downgrades). `NO_COLOR` always honored. |
| Apply mode silently overwrites user-edited files in cwd. | Confirm prompt shows source/dest paths and pattern list; README documents `cp`-style overwrite. Considered (and rejected for v1): per-file diff prompt — adds complexity beyond `setup.sh` parity. |

---

## Documentation / Operational Notes

- `projects/superset-setup/README.md` — install + usage.
- `projects/superset-setup/CLAUDE.md` — build commands and layered-architecture pointer for future agent edits.
- After U3 lands, the repo-root `.superset/setup.sh` is no longer the source of truth; either delete it (workspace doesn't need it at the root) or replace with a comment pointing at `projects/superset-setup/assets/setup.sh`.

---

## Sources & References

- Repo-root `.superset/setup.sh`, `.superset/config.json`, `.superset/setup_config.json` — shape references.
- `scripts/new-project.py` — project scaffolding entry point.
- `projects/workon/Makefile` — Makefile style reference.
- `inquire` crate: https://docs.rs/inquire
- `globset` crate: https://docs.rs/globset
