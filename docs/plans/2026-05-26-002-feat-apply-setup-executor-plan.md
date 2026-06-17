---
title: "feat: execute config.json setup commands on apply"
type: feat
status: active
date: 2026-05-26
deepened: 2026-05-26
---

# feat: execute config.json setup commands on apply

## Summary

Adds an executor in the Rust `projects/superset-setup/` CLI that runs the
`setup` commands from `.superset/config.json` after apply-mode file copy
completes. Execution semantics match upstream Superset
(`superset-sh/superset`): commands joined with ` && ` into one
`$SHELL -lc` invocation so user-shell init (nvm/pnpm/asdf shims) is on
PATH; working directory is the worktree being applied into; two
`SUPERSET_*` env vars exposed to scripts. Apply mode gains a new
confirm-before-run prompt (separate from the existing file-copy
confirmation) that shows the commands and the exact shell invocation
before asking. Empty `setup: []` falls back to running
`<main_checkout>/.superset/setup.sh` directly so the canonical
setup-script path keeps working when `config.json` is absent or empty.

After this PR plus the deferred teardown/run executors (PR-#18 items 2
and 3), `superset-setup` evolves from contract-writer + file-copier into
a CLI lifecycle manager parity with upstream's host-service for the
documented script contract. Scope discipline here keeps the executor
small and additive — picker output already exists, this just consumes it.

---

## Requirements

- R1. New `exec` module exposes `run(workspace_root, main_root, commands, on_event)` that executes setup commands as one ` && `-joined `$SHELL -lc` invocation, returning the child's exit status. `$SHELL` falls back to `/bin/sh` when unset.
- R2. Apply mode prompts the user for confirmation *before* executing setup commands, with the prompt showing: a banner, a bulleted list of commands, the exact `$SHELL -lc "x && y"` invocation that will run, the working directory, and a one-line note that the file copy is already complete (declining leaves files in place; commands won't run). Default is **Yes** for ergonomic re-applies; tab-through risk is acknowledged in Risks.
- R3. Empty `setup: []` array falls back to `bash <main_checkout>/.superset/setup.sh` when that file exists, executed via `Command::new("bash").arg(path)` directly (no `sh -c` wrapping, so paths with spaces work); otherwise the user sees an info line and apply exits successfully. Same fallback fires when `config.json` is absent.
- R4. Executor injects two env vars: `SUPERSET_ROOT_PATH` (main-checkout absolute path) and `SUPERSET_WORKSPACE_PATH` (worktree absolute path). `SUPERSET_WORKSPACE_NAME` is **not** injected — upstream's value is a SQLite-stored logical name the Rust CLI cannot honestly reproduce; better to not advertise a contract we can't keep than to inject the basename and silently drift.
- R5. Failure handling: non-zero exit from the joined invocation surfaces as a CLI error naming the exit code and giving concrete recovery steps (fix the issue, then either re-run setup directly or re-run `superset-setup` declining the file-copy step). When the child is killed by signal (`ExitStatus::code()` returns `None`), render as `signal` and exit non-zero. No retry, no rollback of the preceding file copy — matches upstream's exit-code-recorded-no-rollback model.
- R6. Streaming output: child process inherits the parent's stdout/stderr so commands like `pnpm install` produce real-time progress. No capture, no prefixing in production.

---

## Scope Boundaries

- **Teardown execution.** Explicit user directive: no teardown in this PR.
- **`run` array execution.** Different lifecycle (long-running, possibly resolves against `~/.superset` presets per upstream's `docs/done/20260509-run-script-presets-design.md`). Deferred to its own plan.
- **`config.local.json` overlay support.** Upstream supports a `{before, after}` merge shape and full-replace per-key; the Rust CLI's picker doesn't emit it and the executor reads only the canonical `config.json`.
- **Per-machine `~/.superset/projects/<id>/config.json` override.** Same reason as above.
- **`cwd` field in `SetupConfig`.** Upstream only honors it for `run`, not setup. Safe to ignore here.
- **`SUPERSET_WORKSPACE_NAME` env var.** Upstream value is a logical workspace name from their SQLite store; the Rust CLI has no DB-backed name to map to. Documented divergence — setup scripts that depend on this var won't work without modification.
- **Bootstrap-mode auto-run.** Bootstrap remains a contract-write flow operating on the parent git repo. Setup execution is apply-mode-only (runs in worktrees).
- **New top-level subcommands** (`superset-setup destroy`, `superset-setup run`, `--setup-only`, etc.) — out until teardown/run are planned. Recovery from a failed setup is "re-run and decline file copy", not a dedicated flag.
- **Per-command timeouts, sandboxing, privilege checks, retries.** Accept indefinite hang; Ctrl-C is the cancel path.
- **Windows support.** Unix-only, consistent with the rest of the CLI's `chmod 0755` path.

### Deferred to Follow-Up Work

- **Item 2 from PR #18 follow-ups (teardown executor).** Separate plan; trigger surface needs scoping (new subcommand vs. flag vs. worktree-removal hook).
- **Item 3 from PR #18 follow-ups (run executor).** Separate plan; lifecycle differs and may need preset resolution.

---

## Context & Research

### Relevant Code and Patterns

- `projects/superset-setup/src/apply.rs` — current apply-mode flow. The `run(src, dest, patterns, on_event)` signature and `Event { Copy, Skip }` shape are the template the new executor mirrors. `Summary { copied, skipped }` is the precedent for returning a small struct vs. raw exit codes.
- `projects/superset-setup/src/main.rs:apply_flow` — composes `apply::run` with `confirm_apply`, `load_main_config`, and the post-summary print. The new opt-in prompt + executor wiring slots in after `apply::run` returns.
- `projects/superset-setup/src/ui.rs::confirm_apply` — Yes/No `inquire::Confirm` wrapper. The new `confirm_run_setup_commands` follows the same shape (default Yes, contextualized prompt). Banner + bullet list + joined preview are printed by `apply_flow` *before* the call — the new `ui` function itself only wraps the Y/N Confirm.
- `projects/superset-setup/src/superset_files.rs::load_config` — **already exists** from the prior picker plan; returns `Option<Config>`. Apply mode reads from the main checkout's `.superset/config.json`. No new function needed in `superset_files.rs`.
- `projects/superset-setup/src/style.rs` — `style::header` for the `── Setup commands ──` banner; `style::info`/`ok`/`err` mirror apply mode's existing output.

### Upstream Reference (`superset-sh/superset`, cloned during planning)

The Rust CLI is an alternative consumer of the same `.superset/config.json` contract that the upstream host-service consumes. Three parallel agents read the upstream repo and produced the citations below; the plan's execution model honors these decisions with the noted divergences.

- **Authoritative contract** (TypeScript): `packages/host-service/src/runtime/setup/config.ts:11-27`
  ```typescript
  export interface SetupConfig {
    setup?: string[]; teardown?: string[]; run?: string[]; cwd?: string;
  }
  ```
- **Execution model** (commands joined with ` && `, single shell invocation): `packages/host-service/src/trpc/router/workspace-creation/shared/setup-terminal.ts:96-99` (`commands.join(" && ")`) and `plans/20260505-setup-teardown-scripts-v2.md:96-104` ("run the commands joined with `&&` so a failure short-circuits"). Upstream writes the joined string to a PTY running `$SHELL` interactively — the Rust CLI mirrors the `$SHELL` choice via `$SHELL -lc` to keep nvm/pnpm/asdf shims on PATH.
- **Working directory** (worktree, not main repo): `packages/host-service/src/trpc/router/workspace-creation/shared/setup-terminal.ts` resolves cwd to `workspace.worktreePath`. `cwd` field in `SetupConfig` is parsed but only honored for `run`.
- **Empty-array fallback**: same v2 plan, lines 96-104 — "Else fall back to `bash <repoPath>/.superset/setup.sh` (resolved against the main repo, **not** the worktree)."
- **Documented env vars** (script contract surface): `README.md:177-209` lists `SUPERSET_WORKSPACE_NAME` and `SUPERSET_ROOT_PATH`. `SUPERSET_WORKSPACE_PATH` is added at the runtime env builder (`env.ts:184-197`) — not in the README but equal to the child's `cwd`. The Rust CLI keeps `SUPERSET_ROOT_PATH` (documented) and `SUPERSET_WORKSPACE_PATH` (universal cwd reference) but drops `SUPERSET_WORKSPACE_NAME` because the upstream value is DB-backed and the Rust CLI has no equivalent — see Scope Boundaries.
- **No opt-in UX upstream**: setup runs automatically on workspace creation in the Electron app. No confirmation prompt, no dry-run flag, no preview. The Rust CLI's confirm-before-run prompt is a deliberate UX divergence (CLI users invoke explicitly; explicit-action UX fits a CLI better than auto-run-on-open fits an Electron app).
- **Failure UX upstream**: terminal stays visible, exit code recorded to SQLite, error toast "Workspace opened, but setup command failed." Recovery is manual — user re-runs in the terminal. The Rust CLI matches the no-rollback model but provides explicit recovery guidance in the error message (see R5).

### Institutional Learnings

- `projects/superset-setup/docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md` — irrelevant to this plan (action-loop is picker-side; apply uses simple Confirm prompts).

---

## Key Technical Decisions

- **Match upstream where the script *contract* requires it (execution semantics, env vars, fallback); diverge where the *delivery context* requires it (UX, recovery affordances).** This is the load-bearing parity principle for the plan. The `&&`-join, working-directory choice, fallback-on-empty, and no-rollback decisions match upstream because setup-script authors depend on these. The confirm-before-run prompt, the `$SHELL -lc` invocation (vs. upstream's PTY in interactive mode), the dropped `SUPERSET_WORKSPACE_NAME`, and the concrete error-message recovery diverge because the Rust CLI is invoked from a terminal, not from a desktop app's renderer process.
- **Join setup commands with ` && ` into one shell invocation, not N separate spawns.** Shell short-circuit is the documented contract; shared environment between commands (env vars set by cmd1 visible to cmd2; `cd subdir && next` semantics) is load-bearing. Spawning N processes silently changes failure semantics and breaks `cd`-style patterns. Verified in upstream's v2 plan: *"the runner does `.join(' && ')`, so collapsing into one newline-separated string would silently change failure semantics."*
- **Use `$SHELL -lc "joined"` (default `/bin/sh` when `$SHELL` is unset), not `sh -c`.** Setup commands authored in the user's terminal commonly depend on shim PATHs from shell rc files (nvm, pnpm, asdf, mise, pyenv, Homebrew on Apple Silicon at `/opt/homebrew/bin`). `sh -c` invokes `/bin/sh` (dash on Debian, BSD sh on macOS) without sourcing any rc file; the child inherits the parent's PATH only. A user who tested `pnpm install` in their terminal would see "pnpm: command not found" under `sh -c`. `$SHELL -lc` matches the upstream PTY-in-user-shell semantic. Trade-off: slower startup (shell init), some rc-file side effects (e.g., prompt customizations, motd) appear in stderr; both accepted.
- **Inherit stdout/stderr in production; capture in tests.** Real-time progress for users (the `pnpm install` use case); inherited stdio matches upstream's PTY-inheritance behavior. Tests use a separate `Command::output()` path or assert via side-effects (marker files in a `tempfile::TempDir`).
- **Inject two env vars (`SUPERSET_ROOT_PATH`, `SUPERSET_WORKSPACE_PATH`), drop `SUPERSET_WORKSPACE_NAME`.** Upstream's `SUPERSET_WORKSPACE_NAME` comes from a SQLite-stored logical name set at workspace creation time in the Electron UI. The Rust CLI has no DB-backed equivalent — using `Path::file_name()` would silently drift from upstream's value and break scripts that depend on the exact contract (container names, namespace prefixes, etc.). Better to drop it entirely and document the divergence than to inject a fake value. The other 17+ host-service env vars (`SUPERSET_TERMINAL_ID`, `SUPERSET_AGENT_HOOK_*`, etc.) are daemon-internal and not part of the script contract.
- **Confirm-before-run prompt defaulting to Yes.** Preserves apply-flow ergonomics: the common case is the user wants to apply *and* run setup. The bullet list + joined `$SHELL -lc` preview + working directory + "file copy already complete" note are printed above the prompt as the description; the prompt itself is "Run setup commands? [Y/n]". Tab-through risk (user dismisses by pressing Enter and runs arbitrary shell) is documented in Risks; mitigated by the description being above the prompt rather than buried in help text.
- **Show both the bulleted command list AND the exact `$SHELL -lc "x && y"` joined preview before the prompt.** The bullet list is readable; the joined preview is the contract — it shows shell metacharacter interactions, multi-line entries, and exactly what the shell will parse. Hiding the joined string would make the prompt feel safer than it is; showing both gives users readable comprehension and exact-execution honesty.
- **Empty-array fallback runs `bash <main_checkout>/.superset/setup.sh` via direct `Command::new("bash").arg(path)`, not via `sh -c "bash <path>"`.** The direct-invocation form is immune to path-with-spaces / metacharacter bugs. A user-decision was made to keep the upstream invariant: `setup: []` falls back to `setup.sh` rather than treating empty as "skip". This honors upstream parity and the fact that the picker's preselect of `./.superset/setup.sh` makes empty-array a hand-edit case where the user almost certainly still wants the canonical setup.
- **No rollback on setup failure.** Matches upstream's "exit-code recorded, no rollback" model. The file copy has already completed before setup runs; reverting it would surprise the user. Setup is independent of the file-copy outcome. The error message names concrete recovery steps (see R5) so users aren't left wondering what to do.

---

## Open Questions

### Resolved During Planning

- Sequential or parallel execution? **Sequential, via shell ` && `.**
- Working directory for setup commands? **Worktree (apply destination).**
- Which env vars to inject? **`SUPERSET_ROOT_PATH` and `SUPERSET_WORKSPACE_PATH`. `SUPERSET_WORKSPACE_NAME` dropped (Rust CLI has no DB-backed name).**
- Shell choice for joined invocation? **`$SHELL -lc` (default `/bin/sh`).**
- Fallback on empty array? **Yes, to `<main_checkout>/.superset/setup.sh` when it exists, via direct `Command::new("bash").arg(path)`.**
- Show bullet list or `$SHELL -lc` joined preview at opt-in? **Both — bullets for readability, joined for honesty.**
- Default for the opt-in prompt? **Yes (ergonomic confirm).** Tab-through risk acknowledged in Risks.
- Should bootstrap mode auto-run setup? **No, user directive. Bootstrap operates on the parent git repo; apply mode runs setup in worktrees.**

### Deferred to Implementation

- **Exact wording of the opt-in prompt's header lines and help text.** The required content (banner, bullet list, joined preview, cwd, no-rollback note) is anchored in R2 and U2 Approach; exact phrasing finalizes during U2 smoke.
- **Whether `style::header` (cyan) or a different style helper best marks the `── Setup commands ──` banner.** Pick during U2 to match the existing apply-mode "── Apply Superset config ──" header.

---

## Implementation Units

- U1. **Executor module (`exec.rs`)**

**Goal:** Add a pure executor that runs setup commands as one ` && `-joined `$SHELL -lc` invocation, with documented env-var injection and child stdio inheritance. Emits Begin/Complete events through a caller-supplied closure (parity with `apply::run`'s shape) so production prints them and tests collect them.

**Requirements:** R1, R4, R5, R6

**Dependencies:** None

**Files:**
- Create: `projects/superset-setup/src/exec.rs`
- Modify: `projects/superset-setup/src/main.rs` (add `mod exec;`)
- Test: `projects/superset-setup/src/exec.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Public surface mirrors apply's signature:
  ```rust
  pub fn run<F>(
      workspace_root: &Path,
      main_root: &Path,
      commands: &[String],
      on_event: F,
  ) -> Result<ExitStatus>
  where F: FnMut(&Event)
  ```
- `Event { Begin { shell: String, joined: String }, Complete { status: ExitStatus } }`. Tests assert on Begin's `joined` payload and the shell name; main.rs prints both for the opt-in description and the post-run summary.
- Implementation:
  1. Validate `workspace_root` exists and is a directory; bail with a clear error if not (defensive against the apply-into-empty-dir edge case).
  2. Build `joined = commands.join(" && ")`. If `commands.is_empty()`, this becomes `""` which the caller (U2) is responsible for never producing — drop the defensive `bail!` and corresponding test from earlier drafts; U2 routes empty input to the fallback before `exec::run` sees it.
  3. Determine the shell: `let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());`. Emit `Event::Begin { shell: shell.clone(), joined: joined.clone() }`.
  4. `Command::new(&shell).arg("-lc").arg(&joined)` plus `.current_dir(workspace_root)`, `.env("SUPERSET_ROOT_PATH", main_root)`, `.env("SUPERSET_WORKSPACE_PATH", workspace_root)`.
  5. Inherit stdout/stderr — don't override `Stdio`. `Command::status()` returns when the child exits.
  6. Emit `Event::Complete { status }`.
  7. Return `Ok(status)` — caller decides whether non-zero is fatal.
- Exit-code rendering helper (used by U2): `fn format_exit(status: ExitStatus) -> String { match status.code() { Some(n) => n.to_string(), None => "signal".to_string() } }`. Lives in `exec.rs` as `pub(crate)` since it's exec-specific.
- Public fallback helper for U2: `pub fn run_setup_sh<F>(workspace_root: &Path, main_root: &Path, setup_sh: &Path, on_event: F) -> Result<ExitStatus>` invokes `Command::new("bash").arg(setup_sh)` directly (no shell wrapping, no path-quoting concerns) with the same `.current_dir`/`.env`/`Stdio` setup, and emits the same `Event::Begin/Complete` pair (with `shell: "bash"` and `joined: format!("bash {}", setup_sh.display())` for display).
- Tests use a `tempfile::TempDir` as `workspace_root` and `main_root` and rely on side-effects (touched marker files) plus `Command::output()` to assert env-var visibility. Tests do not depend on inherited stdio.

**Patterns to follow:**
- `projects/superset-setup/src/apply.rs::run` — the `on_event` closure shape, `Event` enum, `Summary`-style return, `tempfile`-based test harness.

**Test scenarios:**
- Happy path: `["touch a", "touch b"]` in a tempdir → both files exist; exit status success; one `Begin` event with `joined == "touch a && touch b"` followed by one `Complete` with success.
- Edge case: single command `["touch a"]` → no `&&` artifacts in the joined string; file exists.
- Error path: `["false"]` → exit status reflects non-zero; `Complete` event carries the non-zero status; no marker side-effects.
- Error path: `["touch a", "false", "touch b"]` → `a` exists, `b` does NOT (short-circuit verified); exit non-zero.
- Edge case: `["cd subdir && touch x"]` in a tempdir containing `subdir/` → `subdir/x` exists (shared shell state verified).
- Integration: `["sh -c 'echo $SUPERSET_ROOT_PATH > out_root'", "sh -c 'echo $SUPERSET_WORKSPACE_PATH > out_ws'"]` → both marker files contain the expected absolute paths.
- Integration: `SUPERSET_WORKSPACE_NAME` is NOT set in the child env (assert via `sh -c 'env | grep ^SUPERSET_WORKSPACE_NAME=' > out; test ! -s out`).
- Error path: `workspace_root` does not exist → returns an error mentioning the path; `on_event` never fires.
- Fallback path: `run_setup_sh` against a tempdir-created `setup.sh` that touches a marker → marker exists; exit success; works even when the path contains a space (e.g., `<tempdir>/My Code/.superset/setup.sh`).
- Signal handling: `format_exit(ExitStatus)` returns `"signal"` for a status with `code() == None` (constructed via `ExitStatusExt::from_raw` on Unix in tests).

**Verification:** `cargo test -p superset-setup exec::` passes.

---

- U2. **Apply-mode wiring: confirm-before-run prompt + empty-array fallback**

**Goal:** Compose the executor into `apply_flow` after the file copy completes, behind a new confirm-before-run Y/N prompt (default Yes) that prints the commands, the joined preview, the working directory, and a no-rollback note. Handle the empty-array fallback to `bash <main_checkout>/.superset/setup.sh` via the dedicated `exec::run_setup_sh` helper (no path-quoting issues).

**Requirements:** R2, R3, R5, R6

**Dependencies:** U1

**Files:**
- Modify: `projects/superset-setup/src/main.rs`
- Modify: `projects/superset-setup/src/ui.rs` (add `confirm_run_setup_commands`)
- Test: `projects/superset-setup/src/main.rs` — none for interactive prompts (matches the module's existing convention).

**Approach:**
- After `apply::run` succeeds in `apply_flow`, call `superset_files::load_config(main_checkout)` (already exists from the picker plan — no new I/O code needed).
- Handle parse errors: if `load_config` returns `Err`, emit `style::warn("Could not read .superset/config.json from main checkout: <error>. File copy completed; skipping setup execution.")` and exit successfully. Don't abort — the file copy is already done and a malformed config shouldn't undo that.
- Resolve which path to run:
  - `Some(cfg)` with `cfg.setup` non-empty → use `cfg.setup.clone()` for the commands list; will call `exec::run`.
  - `Some(cfg)` with `cfg.setup` empty AND `<main_checkout>/.superset/setup.sh` exists → will call `exec::run_setup_sh`. Show the fallback shape in the prompt: a single bullet `bash <path-to-setup.sh>`.
  - `Some(cfg)` with `cfg.setup` empty AND no `setup.sh` → emit `style::info("No setup commands configured.")` and exit 0.
  - `None` (no `config.json`) AND `<main_checkout>/.superset/setup.sh` exists → same as the empty-array-with-setup.sh case (run the fallback).
  - `None` AND no `setup.sh` → emit `style::info("No .superset/config.json or .superset/setup.sh in main checkout; nothing to run.")` and exit 0.
  - Note: `setup: missing` and `setup: []` produce the same outcome (both deserialize to an empty `Vec` via `#[serde(default)]`); upstream's "both fall through" behavior is preserved.
- Print the opt-in description in this order (the lines printed by `apply_flow` *before* the `confirm_run_setup_commands` call):
  1. `style::header("── Setup commands ──")`
  2. A bulleted list of the commands using `ui::print_pattern_list` (already accepts `&[String]`).
  3. A "Will run as:" line followed by `style::info(format!("  {} -lc \"{}\"", shell, joined))` (or `"  bash {path}"` in the fallback case) — the exact invocation, visually indented and dim.
  4. `style::info(format!("Working directory: {}", workspace_root.display()))`.
  5. `style::info("File copy is already complete. Declining leaves files in place; commands will not run.")`.
  6. Env vars line: `style::info("Env vars exposed to commands: SUPERSET_ROOT_PATH, SUPERSET_WORKSPACE_PATH")`.
- `ui::confirm_run_setup_commands() -> Result<bool>` is a thin `Confirm::new("Run setup commands?").with_default(true).with_help_message("Y to run · N to skip (files stay copied)")` wrapper. The function does NOT print the description — that's the caller's job (`apply_flow` prints lines 1-6 above, then calls this).
- User Yes → call `exec::run` or `exec::run_setup_sh` (depending on path). Surface the result:
  - Exit 0 → `style::ok("Setup complete.")`.
  - Non-zero → `bail!("Setup failed (exit {}). The file copy completed and is not rolled back. Fix the issue, then either run the setup commands directly or re-run `superset-setup` and decline the file-copy step.", exec::format_exit(status))`. The error becomes anyhow → main's exit code.
- User No → `style::info("Skipped setup commands. Files are in place; run setup manually when ready.")` and exit 0.
- A small `print_exec_event` closure in `main.rs` prints the executor's Begin (the joined command, dim) and Complete (ok/err depending on exit status) events.

**Patterns to follow:**
- `projects/superset-setup/src/main.rs:apply_flow` — confirmation-then-action shape used for the file-copy step.
- `projects/superset-setup/src/ui.rs::confirm_apply` — `inquire::Confirm` wrapper with `with_default(true)` and a context-anchored help message.
- `projects/superset-setup/src/main.rs::print_event` — apply's per-event print closure is the template for `print_exec_event`.

**Test scenarios:**
- Test expectation: none for `confirm_run_setup_commands` and the `apply_flow` wiring — both are interactive `inquire` paths the module does not unit-test (`confirm_apply`, `pick_patterns`, etc. follow the same convention). Validate manually during smoke.
- Manual smoke A: apply with a non-empty `setup` array → file-copy prompt → setup-confirm prompt (default Yes; press Enter) → commands execute; exit 0 on success.
- Manual smoke B: apply with `setup: []` and a `setup.sh` present → fallback prompt offers `bash <path>`; pressing Enter runs it.
- Manual smoke C: apply with `setup: []` and no `setup.sh` → info line, no prompt, exit 0.
- Manual smoke D: apply, decline the setup confirm (press N) → files stay copied, info line, exit 0.
- Manual smoke E: apply with a failing setup command → file-copy already completed, error message names the exit code and the concrete recovery, CLI exit non-zero.
- Manual smoke F: apply with malformed `config.json` in main checkout → warn line, no setup execution, exit 0 (file copy already happened).
- Manual smoke G: apply with the main checkout path containing a space (e.g., `/Users/.../My Repo/`) and `setup: []` → fallback runs `setup.sh` successfully (direct `bash` invocation bypasses sh quoting).

**Verification:**
- `cargo test -p superset-setup` end-to-end passes.
- Manual smoke (A-G above) walks through the new branches.

---

- U3. **Documentation (README + CLAUDE.md)**

**Goal:** Document the new apply-side setup execution behavior so users and future agents understand the contract — including the deliberate divergences from upstream.

**Requirements:** R2 (the description users see during opt-in is the runtime contract; the docs are the durable version).

**Dependencies:** U1, U2

**Files:**
- Modify: `projects/superset-setup/README.md`
- Modify: `projects/superset-setup/CLAUDE.md`

**Approach:**
- README "Apply mode" section gains a paragraph after the file-copy description:
  - Names the new confirm-before-run prompt and the lines it shows.
  - Documents the two injected env vars (`SUPERSET_ROOT_PATH`, `SUPERSET_WORKSPACE_PATH`).
  - **Explicit divergence note:** `SUPERSET_WORKSPACE_NAME` is not injected; setup scripts that depend on it will need modification.
  - **Shell choice note:** commands run under `$SHELL -lc` (defaults to `/bin/sh`); shell rc files are sourced so nvm/pnpm/asdf shims are on PATH.
  - **No-rollback note:** if setup fails, the file copy is not rolled back. Recovery is: fix the issue and either run the setup commands directly or re-run `superset-setup` and decline the file-copy step.
  - **Side-effect note:** setup commands may write outside `workspace_root` (e.g., to `$SUPERSET_ROOT_PATH`) and may spawn backgrounded daemons that outlive the CLI — both match upstream and are by design.
  - **Trust note:** apply mode only on branches whose `.superset/` contents and copied files you trust — the file copy happens before the setup confirm, so a malicious branch's payload lands on disk regardless of whether you decline setup.
- README "Re-run behavior" section: note that re-running apply re-prompts for the file copy (which can clobber local edits in the worktree). The intended post-failure flow is: fix the issue, decline the file copy on re-run, accept the setup confirm.
- CLAUDE.md source-modules list gains `exec.rs` with a one-paragraph summary mirroring the existing per-module style: `run(workspace_root, main_root, commands, on_event)` joins commands with ` && ` and invokes `$SHELL -lc`; `run_setup_sh` is the direct-invocation fallback for the empty-array case; two `SUPERSET_*` env vars injected; child stdio inherited; `Event { Begin, Complete }` for observability.

**Test scenarios:** none — documentation.

**Verification:** Docs read cleanly; cross-references to `exec.rs` symbols are correct.

---

## System-Wide Impact

- **Interaction graph:** `apply_flow` gains a post-copy executor step with its own confirm prompt. Bootstrap mode is untouched.
- **Error propagation:** Executor non-zero exit becomes `anyhow::Error` → `main`'s err exit code. File copy is already committed at that point; failure does not roll it back. `config.json` parse errors (post-copy) become a warn-and-skip rather than an abort.
- **State lifecycle risks:** Half-finished setup state stays on disk. Matches upstream's no-rollback model; documented in README. Setup commands may also spawn backgrounded daemons that outlive the CLI (`docker compose up -d`, etc.) — accepted; setup-script authors are responsible.
- **Working directory pre-check:** `exec::run` validates that `workspace_root` exists and is a directory before invoking the shell; protects against the apply-into-empty-dir edge case where file-copy produced nothing.
- **`main_root == workspace_root` edge case:** When `superset-setup` is invoked from the main checkout itself (not a worktree), `git::probe` should already route to bootstrap mode rather than apply. If it ever doesn't, the env vars `SUPERSET_ROOT_PATH` and `SUPERSET_WORKSPACE_PATH` collapse to the same value, which is well-defined but probably unintended. The mode-routing logic in `git::probe` is the right guard; no exec-level check needed.
- **API surface parity:** Bootstrap mode signature unchanged. `apply_flow` signature unchanged (still takes `cwd_root`, `main_checkout`). New module `exec.rs` is private to the binary; no public Rust API surface.
- **Integration coverage:** U1's tests cover the executor end-to-end with real subprocesses against `tempfile` fixtures. Apply-flow integration is covered by manual smoke per the module's existing convention for interactive paths.
- **Unchanged invariants:** Bootstrap flow, apply file-copy semantics, picker UX, config.json read/write/merge semantics, `setup.sh` embedded body, `.envrc` offering rule. The new executor is additive — it consumes data the picker already writes, without changing the picker or the file shape.

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| **Malicious branch + file-copy-before-setup**. A developer applies a worktree tracking an unreviewed branch. Files land on disk before the setup confirm prompt. Declining setup doesn't undo the file copy; an attacker's payload in the copied files (e.g., modified `.envrc`) is on disk regardless. | README "Trust note" tells users to only apply branches whose `.superset/` contents have been reviewed. Not a code-level mitigation — accepted as the local-trust threat model. |
| **Tab-through Default-Yes runs arbitrary shell**. User who hits Enter through the apply flow runs setup without reading the joined preview. | Description prints *above* the prompt (not in help text), so it's hard to miss. README documents the prompt's structure. Acknowledged trade-off for ergonomic re-applies. |
| **User Ctrl-C mid-setup leaves a half-finished worktree state** (e.g., `pnpm install` aborted halfway). | `sh` propagates SIGINT to its children when in the foreground (same process group as the Rust parent), matching upstream's PTY behavior. README documents that recovery is manual: re-run setup directly or re-run `superset-setup` and decline the file-copy step. |
| **Shim PATH drift between user's terminal and apply-time shell**. Setup commands testing OK in the user's terminal may fail in `$SHELL -lc` if shell init logic is conditional on interactive mode. | Using login mode (`-lc`) sources rc files; matches upstream's behavior. Rare edge cases (rc files that check `[[ -t 0 ]]` and skip non-interactive) are out of scope. |
| **`$SHELL` is unset or points to an exotic shell**. Environment without `$SHELL` falls back to `/bin/sh`. A user with `$SHELL=/usr/local/bin/fish` (non-bash-compatible) running a bash-syntax setup array would fail. | Fall back to `/bin/sh` when `$SHELL` is unset; document that setup commands should be portable shell or bash. |
| **Long-running setup commands feel like the CLI hung**. | Inherited stdout/stderr surfaces real-time progress for any command that prints. Documented in README; Ctrl-C is the cancel path. |
| **Backgrounded daemons survive CLI exit**. `setup: ["docker compose up -d"]` leaves the daemon running after the CLI returns. | Accepted; matches upstream. README documents this so users aren't surprised. |
| **Setup commands writing outside `workspace_root`**. Scripts can write to `$SUPERSET_ROOT_PATH` (the main checkout) from a worktree. | Accepted; matches upstream (their env builder also exposes both paths). README documents the contract. |
| **`config.json` becomes malformed after a successful file copy**. Reading it during apply errors. | `load_config` error becomes a warn-and-skip in `apply_flow`; file copy stays committed. README documents that fix is to repair `config.json` and re-run. |
| **Empty `setup: []` hand-edit silently runs `setup.sh`**. A user explicitly emptying the array still triggers the fallback. | Documented (Scope Boundaries + README): upstream invariant preserved. Users wanting "no setup at all" must also remove `setup.sh` from the main checkout. |

---

## Documentation / Operational Notes

- README "Apply mode" gains the paragraphs described in U3.
- README "Re-run behavior" gets the post-failure flow explainer.
- CLAUDE.md source-modules list gains `exec.rs`.
- No CHANGELOG (the repo doesn't keep one).

---

## Sources & References

- **Upstream contract** (`superset-sh/superset`, cloned during planning, summarized via three parallel Sonnet research agents):
  - TypeScript interface: `packages/host-service/src/runtime/setup/config.ts:11-27`
  - Execution model (` && `-join, single PTY write): `packages/host-service/src/trpc/router/workspace-creation/shared/setup-terminal.ts:96-99`
  - Env builder: `packages/host-service/src/runtime/setup/env.ts:184-197`
  - Documented contract: `README.md:177-209` (env vars `SUPERSET_WORKSPACE_NAME` (not honored by Rust CLI), `SUPERSET_ROOT_PATH`)
  - Original v2 plan: `plans/20260505-setup-teardown-scripts-v2.md` (especially lines 96-104 on execution model and 126-130 on editor-save semantics)
  - Test invariants: `apps/desktop/src/lib/trpc/routers/workspaces/utils/setup.test.ts`
  - Run-key follow-up plan: `docs/done/20260509-run-script-presets-design.md`
- Upstream prior art for opt-in UX: none (upstream auto-runs in the Electron app; Rust CLI's confirm-before-run is a deliberate divergence).
- Related code in this repo:
  - `projects/superset-setup/src/apply.rs::run` — `on_event` template
  - `projects/superset-setup/src/ui.rs::confirm_apply` — `Confirm` template
  - `projects/superset-setup/src/superset_files.rs::load_config` — config reader (already exists)
  - `projects/superset-setup/src/main.rs:apply_flow` — composition site
- Related plans:
  - `projects/superset-setup/docs/plans/2026-05-25-001-feat-superset-setup-cli-plan.md` — original CLI plan
  - `projects/superset-setup/docs/plans/2026-05-26-001-feat-config-json-setup-commands-picker-plan.md` — picker that emits the `setup` commands consumed here
- Upstream PR for this work: `https://github.com/ViktorStiskala/monorepo-general/pull/18`
