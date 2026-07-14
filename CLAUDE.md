# ss-magic

Interactive Rust CLI for the Superset workspace contract (standalone repo:
`ViktorStiskala/superset-magic`; binary: `ss-magic`). See README.md for
user-facing docs.

## Build

```
make build     # cargo build --release
make install   # cargo install --path .
make clean     # cargo clean
```

Rust toolchain is provided by `rustup` (cargo on `~/.cargo/bin`).

Release binaries are published to GitHub Releases via cargo-dist
(`dist-workspace.toml`); the binary self-updates from there. End-user install
instructions (the installer script, prebuilt-binary download, and from-source
builds) live in README.md.

## Architecture

Layered to keep the pure logic unit-testable in isolation from the
interactive layer:

- `git.rs` — read-only probes (`is_worktree`, `main_checkout_root`,
  `cwd_repo_root`, `main_branch_name`, plus the reverse-sync probes
  `untracked_files` (`git ls-files --others` — untracked *including*
  gitignored, since reverse sync pushes gitignored secrets),
  `is_ignored`, `check_ignore_pattern`,
  `diff_no_index_paged`) and mutating primitives (`stage_paths`,
  `commit`, `push`, `push_upstream`, `create_branch`, `pr_create`,
  `timestamp_branch_suffix`, `gh_available`). All `git`/`gh` invocations
  shell out via a shared `git_raw` helper that surfaces stderr verbatim;
  `git` and `git_optional` are thin one-liners on top. (The bare
  location-auto `probe`/`Mode` dispatch was removed in U13 — routing is
  now the menu via `is_worktree` + `main_checkout_root`.)
- `superset_files.rs` — `.superset/{config.json, magic.sh, magic.json,
  magic.local.json}` I/O (plus the legacy `setup_config.json` reader).
  `load_config` reads Superset-owned `config.json`;
  `merge_setup_into_config` builds a new `Config` from a new `setup`
  array while preserving `teardown` and `run` from disk;
  `write_config_json` always rewrites pretty-printed. `load_overlaid`
  reads `magic.json` and overlays `magic.local.json` (union+dedupe
  `files`, base order first); `write_magic_json`, `write_magic_sh`,
  `bootstrap_magic_local_json`, and `default_magic_files` are the
  init/migration writers. `load_setup_config` / `SetupConfig` survive as
  a READ-ONLY legacy path: migration reads the old `setup_config.json`
  `files` to carry them into `magic.json`. `existing_unknown_entries`
  preserves user-typed patterns across re-runs. `copy_into_repo`
  materializes the staged `.superset/` tree atomically (files always
  overwritten — preservation happens upstream of the write; `*.sh` are
  chmod 0755'd; a `delete` set strips the retired `setup.sh`).
- `repo_scan.rs` — `matches_for_patterns(root, &[&str])` walks the
  working tree once with a multi-pattern `GlobSet` and returns a bool
  vector aligned to the input. `pattern_matches_any` is the single-
  pattern shortcut used when the user adds a custom pattern in the
  bootstrap picker.
- `pattern.rs` — shared syntax checks for both the apply/sync
  expansion path and the picker UI validator: `has_glob_meta`,
  `has_parent_segment`, `SyntaxError`, `check_syntax`. One source of
  truth for "is this pattern structurally valid".
- `apply.rs` — the glob/exclude/copy engine reused by forward `sync`
  (and, via `match_paths`, by reverse sync). Delegates syntax checks to
  `pattern::check_syntax`. Emits an `Event` stream via a caller-supplied
  closure so tests can collect events while production prints them.
  (`load_main_config`, the old interactive apply path, was removed in
  U13.)
- `style.rs` — palette (gray info, bold green ok, bold orange/xterm 208
  warn, bold red err, bold cyan header). One `OnceLock<bool>` captures
  the color decision (NO_COLOR + supports-color). `inquire`'s global
  `RenderConfig` is installed from the same palette.
- `ui.rs` — `inquire` wrappers. `pick_with_actions` is the shared
  `Select`-loop driver behind `pick_patterns`; the shared `Row` shape
  carries `dim_suffix: Option<&'static str>` for the `(no matches)`
  flag. `pick_final_action`, `print_pattern_list`, `validate_pattern`
  (delegating to `pattern::check_syntax`), and the reverse-sync picker
  (`pick_reverse_sync`, `confirm_overwrite_with_diff`) round out the
  module. (The setup-command picker/validator and the `.envrc`/apply
  confirms were removed in U13.) See
  `docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`
  for why the pickers are `Select` loops rather than a `MultiSelect`.
- `cli.rs` — hand-rolled arg parser (no `clap`). `parse(&[String]) ->
  Parsed` selects `Command::{Bare, Sync, Update}` from the first non-flag
  arg (absent → `Bare`), short-circuits `--help`/`-h` to `Parsed::Help`,
  and returns `Parsed::Error(token)` for an unknown subcommand.
  `init [PATTERN...]` parses to `Parsed::Init(patterns)` (carried apart
  from the `Copy` `Command` enum). Pure and unit-testable without
  spawning the process.
- `menu.rs` — bare-invocation operation menu. Location-gated: main
  checkout offers init / migrate / edit config; a worktree offers
  forward sync / reverse sync. `Pack` is offered wherever an initialized
  `magic.json` exists (any worktree, or main on a `Normal` branch), so it
  appears in both location lists. Routes selections to their handlers via
  the `Select` driver; Esc/Ctrl-C is inert.
- `migrate.rs` — detect + migrate/init branching off `config.json`'s
  `setup` (old `setup.sh` reference → migrate; `magic.sh` marker →
  normal; neither → init). Stages renames/writes/deletes into a tempdir
  and materializes via `copy_into_repo` only after the finishing-action
  prompt. `run_init_noninteractive` is the TUI-free init behind
  `ss-magic init` (writes the layout from CLI patterns, no prompt, not
  gated by auto-update).
- `reverse_sync.rs` — push git-untracked worktree files matching the
  overlaid patterns back to main, with a diff-aware picker,
  parent-dir creation, and gitignore-safety (`gitignore.rs`).
- `pack.rs` — `ss-magic pack`: expand the overlaid `magic.json` patterns
  against the current git repo root (via `apply::match_paths`) and write
  the matches — repo-relative — into `ss-magic-files.tar.bz2` at that root.
  Everything (config source, match target, archive destination) is the one
  `cwd_repo_root`. `pack_core(cwd, on_event)` mirrors `main::sync_core`'s
  control flow (resolve root → probe magic.json → load overlaid → empty
  guard → work) and emits a `PackEvent` stream. `write_archive` tars into a
  bzip2 stream (`bzip2` crate, pure-Rust `libbz2-rs-sys` backend — no C
  toolchain) via a `NamedTempFile` in the root, then persists atomically.
  Safety: it never packs the output archive into itself (nor a `.` match that
  resolves to the repo root); it classifies each match with
  `symlink_metadata` (no-follow) so a matched symlink — including one to a
  directory — is stored as a single symlink entry rather than followed
  (`Path::is_dir()` would follow it and archive the target tree); and it
  discards the temp file without touching an existing archive when nothing was
  actually added, so a prior good backup is never replaced by an empty tarball.
- `gitignore.rs` — `.gitignore` helpers at a git root: `ensure_entry`
  appends a line iff no exact match exists (creates the file if absent,
  never reorders); `find_covering_rule` resolves the rule covering a path
  via `git check-ignore -v` (negations excluded). Used by `migrate.rs`
  (bootstrap) and `reverse_sync.rs` (secret-safety; verified-then-literal
  fallback so a copied secret is always ignored in main).
- `update/` — every-invocation self-update: `check.rs` does the
  daily-cached GitHub latest check (ureq, ETag, 5 s timeout, silent
  fall-through); `apply.rs` does the fd-lock / download / atomic swap /
  spawn-and-wait re-exec via the `self_update` crate. Integrity rests on
  TLS + cargo-dist checksums (no SHA-256-vs-asset-digest check — see the
  KTD5 conformance notes in `update/apply.rs`); `bin_path_in_archive`
  matches cargo-dist's `<bin>-<target>/` tarball layout.
- `main.rs` — composes everything: `style::init` → `cli::parse` →
  [auto-update gate for `Bare`/`Sync`/`Pack`] → `dispatch`. `Bare` routes to
  `menu::run`; `Sync` runs the non-interactive forward copy
  (`sync_core`); `Pack` runs `pack::pack_core` (`run_pack_flow` +
  `print_pack_event`); `Update` forces a self-update. `print_event` renders
  the `apply::Event` stream.

## Source of truth for magic.sh

`assets/magic.sh` is the canonical wrapper script, embedded into the
binary via `include_str!`. Migration and init write that body to
`.superset/magic.sh`. Edit `assets/magic.sh` and re-run migration/init
to propagate. (The legacy `assets/setup.sh` was deleted in U13 — the
binary is the sole file-copy implementation.)

## Conventions

- No `git2` — all git/gh interactions shell out via `std::process::Command`.
- Glob semantics (originally derived from the retired `setup.sh`):
  absolute / `..` rejected, literals must exist, glob-zero-match
  non-fatal, `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at
  any depth. Now owned by `apply.rs` + `pattern.rs`.
- Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.
  Final-action git ops and the interactive menu/pickers have no unit
  tests — validated by manual smoke.
- Always bump the crate version (`version` in `Cargo.toml`, and the
  matching `ss-magic` entry in `Cargo.lock`) on any change that alters
  CLI behavior — a fix, a new/changed command or flag, or different
  output. The binary self-updates from GitHub Releases keyed on version
  (see Build), so a stale version means users never receive the change.
  Bug fixes bump patch; new/changed user-visible behavior bumps minor
  (pre-1.0).
- After every implementation change, update `CLAUDE.md` and `README.md`
  to match the current state before the change is considered done. A
  new/changed command, flag, module, or behavior must be reflected in the
  README (command list + relevant prose) and in this doc's Architecture +
  Conventions sections — the two docs are expected to describe the code as
  it is now, not as it was.
- `.cursor/BUGBOT.md` holds the Cursor Bugbot review rules. It must stay
  **self-contained**: it cannot reference this `CLAUDE.md`,
  `docs/solutions/`, `.cursor/rules`, or any skill/rule — restate the
  relevant conventions inline instead. Keep it **synchronised on every
  change**: whenever a convention here or a behavior in the code changes,
  update `.cursor/BUGBOT.md` in the same change so its rules never describe
  stale conventions.

## Documented Solutions

`docs/solutions/` — documented solutions to past problems (bugs, best
practices, design patterns, workflow learnings), organized by category
with YAML frontmatter (`module`, `tags`, `problem_type`, `component`).
Relevant when implementing or debugging in documented areas.

`CONCEPTS.md` (repo root) — shared domain vocabulary (the sync model:
main checkout, forward/reverse sync, sync patterns, candidates).
Relevant when orienting to the codebase or discussing domain concepts.
