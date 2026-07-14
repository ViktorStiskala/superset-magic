# Bugbot Review Rules for ss-magic

Review with maximum thoroughness. `ss-magic` moves per-developer secrets
(`.env`, `.dev.vars`, `.superset/magic.local.json`, and similar) between a
main git checkout and its worktrees, and packs them into archives — treat
secret handling, gitignore safety, and filesystem writes with extra scrutiny.
Trace data flow across the git-checkout boundary, verify glob/path edge cases,
and check that destructive or overwriting filesystem operations are guarded.

This document is self-contained: it restates the conventions rather than
pointing at other docs, so it must be re-synchronised whenever those
conventions change.

## Tech Stack

Standalone interactive Rust CLI (binary: `ss-magic`; repo:
`ViktorStiskala/superset-magic`). Edition 2021. Key dependencies: `anyhow`
(errors), `inquire` (interactive prompts), `globset` + `walkdir` (pattern
matching), `serde`/`serde_json` (config I/O), `tempfile` (atomic staging),
`tar` + `bzip2` (pack archives), `self_update` + `ureq` + `fd-lock`
(self-update), `supports-color` (palette). No `clap` (the arg parser is
hand-rolled) and no `git2` (all git/gh is shelled out). Release binaries are
built by cargo-dist (`dist-workspace.toml`) and self-update from GitHub
Releases. Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.

## No External Process Libraries

- **All `git` and `gh` interaction shells out via `std::process::Command`** —
  there is NO `git2`/`libgit2`. Flag any addition of `git2`, `gix`, or another
  git-binding crate to `Cargo.toml`. The shared entry point is the `git_raw`
  helper in `src/git/mod.rs` (surfaces stderr verbatim); `git` and `git_optional`
  are thin one-liners on top. Flag new git/gh calls that spawn `Command`
  directly instead of routing through these helpers.
- **The CLI arg parser is hand-rolled in `src/cli.rs`** — there is NO `clap`.
  `parse(&[String]) -> Parsed` selects the command from the first non-flag
  token. Flag any addition of `clap`/`structopt`/`argh`, or command dispatch
  logic added outside `cli.rs`.

## Architecture: Layering (pure logic vs interactive layer)

The codebase is deliberately layered so the pure logic is unit-testable in
isolation from the interactive TUI. Preserve this boundary.

- Pure/testable modules: `git/mod.rs` (probes + mutating primitives), `cli.rs`
  (arg parsing), `sync/pattern.rs` (glob syntax checks), `sync/apply.rs` (glob/copy
  engine), `workspace/superset_files.rs` (`.superset/` I/O), `sync/repo_scan.rs` (working-tree
  scan), `git/gitignore.rs` (`.gitignore` helpers).
- Interactive/side-effecting: `tui/menu.rs`, `tui/ui.rs` (`inquire` wrappers),
  `tui/style.rs` (palette), the finishing-action prompts in `workspace/migrate.rs` /
  `sync/reverse_sync.rs`.
- `main.rs` composes: `tui::style::init` → `cli::parse` → [auto-update gate for
  `Bare`/`Sync`/`Pack`] → `dispatch`.

Flag business/pure logic (glob expansion, config merge, path resolution) added
directly into `tui/menu.rs`/`tui/ui.rs`/`main.rs` instead of a testable module, and
flag interactive `inquire` calls introduced into the pure modules.

## The Event-Stream Pattern

`sync/apply.rs` (`run`) and `pack.rs` (`pack_core`) emit a stream of typed events
(`apply::Event`, `pack::PackEvent`) through a **caller-supplied closure**, so
tests can collect events while production (`main.rs`) prints them. Flag new
engine code that prints directly to stdout/stderr (`println!`/`eprintln!`)
from inside the pure engine instead of emitting an event — that breaks the
test seam. User-facing rendering belongs in `main.rs`'s `print_event` /
`print_pack_event`.

## Glob and Path Semantics (owned by `sync/apply.rs` + `sync/pattern.rs`)

`pattern::check_syntax` is the single source of truth for "is this pattern
structurally valid". The engine's rules:

- **Absolute patterns and any pattern containing a `..` segment are rejected**
  (counted as skipped). Flag any expansion/copy path that accepts an absolute
  or parent-traversal pattern, or that resolves a matched path outside the
  source tree.
- Literal (non-glob) patterns must exist on disk — a missing literal is a
  counted skip; a glob with zero matches is non-fatal and uncounted.
- `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at ANY depth. Flag
  code that bypasses `is_excluded` when materialising matches.
- Matches are de-duplicated by relative path; matched directories are copied
  recursively.
- `globset`'s `*` crosses path separators (unlike POSIX shell glob) — do not
  introduce code that assumes `*` matches a single path component.

Flag any second, divergent glob/exclude implementation — expansion must go
through `sync/apply.rs` (`run`/`match_paths`) and syntax checks through
`pattern::check_syntax`.

## Security: Secret Handling and Gitignore Safety

The files this tool moves are secrets. The main-checkout copy must never become
committable and must never leak.

- **Reverse sync moves git-untracked (including gitignored) files** from a
  worktree back to the main checkout (`git ls-files --others` in `git/mod.rs`;
  `sync/reverse_sync.rs`). When a copied path is not already gitignored in the main
  checkout, `ss-magic` copies the covering `.gitignore` rule (resolved via
  `git check-ignore -v`, negations excluded) into main's root `.gitignore`,
  falling back to the literal path when no covering rule exists
  (`git/gitignore.rs`). Flag any reverse-sync change that writes a secret into the
  main checkout WITHOUT ensuring it is gitignored there, or that removes the
  verified-then-literal fallback.
- `git/gitignore.rs::ensure_entry` appends a line only if no exact match exists,
  creates the file if absent, and never reorders. Flag changes that reorder or
  dedupe existing `.gitignore` content.
- **Pack must not dereference symlinks.** `pack::write_archive` sets
  `tar::Builder::follow_symlinks(false)` — the tar default (`true`)
  dereferences symlinks and embeds the TARGET file's bytes, which leaks an
  out-of-repo secret (e.g. a link to `~/.aws/credentials`) into the archive and
  hard-aborts on a broken link. Flag any removal of `follow_symlinks(false)`,
  or a new archive-building path that omits it. Note `Path::is_file()` follows
  symlinks, so a top-level `is_file()` guard does NOT substitute for this.
- **Pack must never archive itself or the whole tree.** `pack_core` drops any
  match equal to the derived output name (`pack::archive_file_name` — from the
  normalized `origin` remote, falling back to the primary worktree basename),
  any match equal to the legacy `ss-magic-files.tar.bz2` name, and any match
  that resolves to the repo root itself (a `.` pattern) before archiving. Flag
  removal of any of these guards.
- **Clipboard stays out of the pack engine.** The archive-path clipboard copy
  (`pack::copy_to_clipboard`) and the extraction-hint output hang off
  `PackEvent::Done` in `main.rs`'s rendering layer. Flag any clipboard or
  extra printing side effect added inside `pack_core`/`write_archive` — tests
  drive those directly and must never mutate the developer's clipboard.
- **Pack classifies matches with `symlink_metadata` (lstat), not `is_dir()`.**
  `Path::is_dir()`/`is_file()` follow symlinks, so a matched symlink to a
  directory would make `append_dir_all` walk the link's TARGET tree (outside
  the repo). Each match must be classified no-follow: a symlink → a single
  symlink entry; a real dir → `append_dir_all`; a real file →
  `append_path_with_name`; anything else (socket/fifo/vanished) → skipped. Flag
  a pack that classifies a top-level match with `is_dir()`/`is_file()` (which
  follow links) instead of `symlink_metadata`.
- **Pack must not write an empty archive or clobber a good one.** When nothing
  is actually added (every match was a special file or vanished after
  expansion), `write_archive` must discard the temp file and leave any existing
  `ss-magic-files.tar.bz2` untouched — never rename an empty tarball over a
  prior good backup, and never report "Packed 0 entries" as success. Flag a
  pack path that persists the temp archive when the added count is zero.
- Overwrite safety: reverse sync requires a per-file diff + explicit user
  confirmation before overwriting a file that already exists in the main
  checkout (`tui/ui.rs::confirm_overwrite_with_diff`). Flag a reverse-sync path
  that overwrites an existing main-checkout file without that confirm.

## Filesystem Writes: Atomic Staging

- `.superset/` materialisation stages the whole tree in a tempdir and copies it
  into place only after the user confirms the finishing action
  (`superset_files::copy_into_repo`, driven by `workspace/migrate.rs`). `*.sh` files are
  chmod `0755`; a `delete` set strips retired files (e.g. the old `setup.sh`).
  Flag partial in-place writes to `.superset/` that bypass this staging.
- `pack::write_archive` writes the archive to a `NamedTempFile` in the git root
  and renames it into place atomically only after the tar+bzip2 stream is fully
  finalised (`into_inner()` then `finish()`). Flag an archive path that writes
  the final `ss-magic-files.tar.bz2` directly, or that renames before both
  stream layers are flushed.

## Config Files (`workspace/superset_files.rs`)

- `config.json` is Superset-owned (`{ setup, teardown, run }`);
  `merge_setup_into_config` builds a new `Config` from a new `setup` array
  while **preserving `teardown` and `run` from disk**. Flag a merge that drops
  or reorders `teardown`/`run`.
- `magic.json` (committed) is overlaid with `magic.local.json` (gitignored,
  per-machine) via `load_overlaid`: `files` are UNION + DEDUPE with
  `magic.json` order first. Flag overlay changes that reorder base entries or
  drop the dedupe.
- `setup_config.json` / `SetupConfig` is a READ-ONLY legacy migration path
  (its `files` are carried into `magic.json`); it is never written. Flag any
  code that writes `setup_config.json`.
- Malformed `magic.json` / `magic.local.json` / `config.json` must be a HARD
  error with a non-zero exit that names the offending path — never a silent
  fallback to empty/default. Flag a config read that swallows a parse error.

## `magic.sh` Source of Truth

`assets/magic.sh` is the canonical wrapper script, embedded into the binary via
`include_str!` and written to `.superset/magic.sh` by migration/init. Flag a
change to the `.superset/magic.sh` body made anywhere OTHER than
`assets/magic.sh` (a hard-coded wrapper string elsewhere would drift from the
embedded source of truth).

## Self-Update Safety (`update/`)

- The daily-cached "latest release" check (`update/check.rs`) uses `ureq` with
  an ETag and a short timeout, and must fall through SILENTLY on any offline /
  non-200 / timeout result — a failed update check must never block or slow a
  normal invocation. Flag an update-check change that surfaces a hard error or
  removes the timeout.
- The apply path (`update/apply.rs`) takes an advisory `fd-lock`
  (skip-on-contention), downloads over TLS, atomically swaps the binary, then
  re-execs and blocks on the child. The re-exec loop guard (`SS_MAGIC_UPDATED`
  / `SS_MAGIC_NO_UPDATE`) must prevent infinite re-exec — flag changes to
  `should_run_update_gate` / `guard_active` that could let a re-exec'd child
  re-enter the gate.
- The auto-update gate fires for `Bare`, `Sync`, and `Pack`; `Update` uses its
  own force path and bypasses the daily-cache gate. Keep this consistent when a
  new command is added.

## Style / Output

- All colored output goes through `tui/style.rs` (gray info, bold green ok, bold
  orange warn, bold red err, bold cyan header). The color decision (NO_COLOR +
  supports-color) is captured once in a `OnceLock<bool>`. Flag raw ANSI escape
  codes emitted outside `tui/style.rs`, or output that ignores the NO_COLOR
  decision.
- Interactive prompts must be inert on Esc / Ctrl-C (leave the tree untouched
  and exit success) — `tui/menu.rs` and the pickers follow this. Flag an
  interactive path where cancellation mutates the filesystem.

## Version Bump Discipline (REQUIRED)

The binary self-updates from GitHub Releases keyed on the crate version, so a
stale version means users never receive the change. **Any change that alters
CLI behavior — a fix, a new/changed command or flag, or different output —
MUST bump `version` in `Cargo.toml` AND the matching `ss-magic` entry in
`Cargo.lock`.** Bug fixes bump patch; new/changed user-visible behavior bumps
minor (pre-1.0). Flag a behavior-changing PR that does not bump both
`Cargo.toml` and `Cargo.lock`, or that bumps only one of the two.

## Test Requirements

- Tests use `tempfile` for scratch trees and shell-invoked `git init` /
  `git worktree add` for git fixtures. Pure modules (`cli.rs`, `sync/pattern.rs`,
  `sync/apply.rs`, `pack.rs`, `workspace/superset_files.rs`, `git/mod.rs` probes, `tui/menu.rs`
  routing via `operations_for`) have unit tests; the interactive
  menu/pickers and final-action git ops are validated by manual smoke, not
  unit tests.
- New behavior in a pure module (a new command in `cli.rs`, a new
  `operations_for` entry, new glob/exclude/pack behavior) MUST come with tests
  covering the happy path and key edge cases (empty input, error/hard-fail
  paths, exclusions). Flag a behavior-adding PR to a pure module with no test
  changes.
- Bug fixes SHOULD include a test that reproduces the issue before the fix.
- Test layout: every module declares `#[cfg(test)] mod tests;` with the body
  in a dedicated child file (`<module>/tests.rs`); crate-root tests and shared
  helpers live in `src/tests/` (`sync.rs`, `update_gate.rs`, `support.rs`).
  Flag a PR that adds an inline `mod tests { ... }` block to a source file
  instead of a sibling test file.
- CI (`.github/workflows/ci.yml`) runs `cargo test --locked` on every PR
  commit and gates cargo-dist releases via `plan-jobs` in
  `dist-workspace.toml`. Flag hand edits to the generated
  `.github/workflows/release.yml` (regenerate with the pinned `dist` version
  instead) and flag `allow-dirty = ["ci"]` additions.
- Release archives are attested (`github-attestations = true` in
  `dist-workspace.toml` → `actions/attest` in the release workflow's
  build-local-artifacts job, signing same-job build output before it
  transits Actions artifact storage). Flag removal of the
  `github-attestations` key, removal of the attest step, or a
  `github-attestations-phase` change away from `build-local-artifacts` —
  a host/announce-phase attest signs a `download-artifact` merge directory
  that any job in the run can inject into, so a phase change requires
  explicit security review, not routine approval.

## Documentation Sync (REQUIRED)

`README.md` (user-facing) and `CLAUDE.md` (architecture/conventions) must
reflect the current state after every implementation change — a new command,
flag, module, or changed behavior. Flag a behavior- or architecture-changing PR
that leaves `README.md` or `CLAUDE.md` describing the old state (e.g. a new
subcommand not listed in the README command list or the `main.rs`/`cli.rs`
descriptions, or a new module absent from the `CLAUDE.md` architecture list).
This `.cursor/BUGBOT.md` must likewise be re-synchronised whenever the
conventions above change.
