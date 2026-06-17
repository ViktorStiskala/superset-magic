# superset-setup

Interactive Rust CLI for the Superset workspace contract. See README.md
for user-facing docs.

## Build

```
make build     # cargo build --release
make install   # cargo install --path .
make clean     # cargo clean
```

Rust toolchain is provided by `rustup` (cargo on `~/.cargo/bin`).

## Architecture

Layered to keep the pure logic unit-testable in isolation from the
interactive layer:

- `git.rs` — read-only probes (`probe`, `is_worktree`, `main_branch_name`)
  and mutating primitives (`stage_paths`, `commit`, `push`,
  `create_branch`, `pr_create`, `timestamp_branch_suffix`,
  `gh_available`). All `git`/`gh` invocations shell out via a shared
  `git_raw` helper that surfaces stderr verbatim; `git` and
  `git_optional` are thin one-liners on top.
- `superset_files.rs` — `.superset/{setup.sh, config.json,
  setup_config.json}` and `.envrc` I/O. `load_config` reads
  `config.json`; `merge_setup_into_config` builds a new `Config` from
  picker-output setup commands while preserving `teardown` and `run`
  from disk; `write_config_json` always rewrites pretty-printed.
  `existing_unknown_entries` (generic over patterns and commands) is
  the preservation rule shared by both bootstrap pickers.
  `copy_into_repo` materializes the staged tree atomically (config.json
  is always overwritten — preservation happens upstream of the write).
- `repo_scan.rs` — `matches_for_patterns(root, &[&str])` walks the
  working tree once with a multi-pattern `GlobSet` and returns a bool
  vector aligned to the input. `pattern_matches_any` is the single-
  pattern shortcut used when the user adds a custom pattern in the
  bootstrap picker.
- `repo_detect.rs` — `detect_for_options(root)` returns a bool vector
  aligned to a fixed `OPTIONS` array of preconfigured setup-command
  rows, driven by root lockfiles (pnpm/npm/yarn/uv, pnpm wins
  coexistence) and a conservative whitelist of recognized
  `package.json` `scripts` (just `cf-typegen` in v1).
- `pattern.rs` — shared syntax checks for both the apply-mode
  expansion path and the bootstrap UI validator: `has_glob_meta`,
  `has_parent_segment`, `SyntaxError`, `check_syntax`. One source of
  truth for "is this pattern structurally valid".
- `apply.rs` — re-implements `setup.sh`'s expansion/copy semantics.
  Delegates syntax checks to `pattern::check_syntax`. Emits an `Event`
  stream via a caller-supplied closure so tests can collect events
  while production prints them.
- `exec.rs` — runs the `setup` array from `config.json` after apply's
  file copy completes. `run(workspace_root, main_root, commands,
  on_event)` joins commands with ` && ` and invokes `$SHELL -lc`
  (`/bin/sh -c` when `$SHELL` is unset, because POSIX `sh` lacks `-l`).
  `run_setup_sh` is the direct-invocation fallback for the empty-array
  case — uses `Command::new("bash").arg(setup_sh)` so paths with spaces
  are safe. Injects `SUPERSET_ROOT_PATH` and `SUPERSET_WORKSPACE_PATH`;
  child inherits stdout/stderr for real-time progress. `Event { Begin,
  Complete }` for caller-side display; `format_exit` renders signal-
  killed exits as `"signal"` so error messages stay readable.
- `style.rs` — palette mirrored from `setup.sh` (gray info, bold green
  ok, bold orange/xterm 208 warn, bold red err, bold cyan header). One
  `OnceLock<bool>` captures the color decision (NO_COLOR + supports-color).
  `inquire`'s global `RenderConfig` is installed from the same palette.
- `ui.rs` — `inquire` wrappers. `pick_with_actions` is the shared
  `Select`-loop driver used by both `pick_patterns` and
  `pick_setup_commands`; the shared `Row` shape carries
  `dim_suffix: Option<&'static str>` so each picker passes its own
  copy (`"(no matches)"` vs `"(not detected)"`). `confirm_envrc`,
  `confirm_apply`, `pick_final_action`, `print_pattern_list`, and the
  two validators (`validate_pattern` delegating to
  `pattern::check_syntax`; `validate_command` rejecting empty +
  duplicate-of-taken only) round out the module. See
  `docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`
  for why the pickers are `Select` loops rather than a `MultiSelect`.
- `main.rs` — composes everything: `style::init` → `git::probe` → branch
  to `bootstrap_flow` / `apply_flow` / error. Bootstrap captures all
  decisions, stages writes to a tempdir, materializes via
  `superset_files::copy_into_repo` after the final-action prompt.

## Source of truth for setup.sh

`assets/setup.sh` is the canonical bash script, embedded into the binary
via `include_str!`. Bootstrap mode writes that body to
`.superset/setup.sh` on every run. Edit `assets/setup.sh` and re-run
bootstrap to propagate.

## Conventions

- No `git2` — all git/gh interactions shell out via `std::process::Command`.
- Glob semantics are derived from `setup.sh`: absolute / `..` rejected,
  literals must exist, glob-zero-match non-fatal, `DEFAULT_EXCLUDES`
  (`node_modules`, `.venv`) drop matches at any depth.
- Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.
  Final-action git ops have no unit tests — validated by manual smoke.

## Documented Solutions

`docs/solutions/` — documented solutions to past problems (bugs, best
practices, design patterns, workflow learnings), organized by category
with YAML frontmatter (`module`, `tags`, `problem_type`, `component`).
Relevant when implementing or debugging in documented areas.
