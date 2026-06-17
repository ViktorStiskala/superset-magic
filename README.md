# ss-magic

Interactive Rust CLI that bootstraps the monorepo's Superset workspace
contract in the main checkout and applies the configured file copies
from inside a linked worktree.

## Install

```sh
make install
```

Builds via `cargo install --path .` and drops `ss-magic` in
`$CARGO_HOME/bin` (usually `~/.cargo/bin`).

## Modes

The mode is chosen automatically based on where you run it from.

### Bootstrap mode (main checkout, on `main`)

Writes the `.superset/` contract at the repo root:

- `.superset/setup.sh` — always overwritten with the embedded canonical
  body (source: `projects/superset-setup/assets/setup.sh`).
- `.superset/config.json` — always rewritten in bootstrap mode from the
  setup-commands picker output. `teardown` and `run` arrays are preserved
  verbatim from the existing file when present, or default to empty.
- `.superset/setup_config.json` — rewritten from your multi-select.

Two pickers run back-to-back during bootstrap. The first selects files
to copy on `superset apply` (writes `setup_config.json`); the second
selects setup commands to run on `superset apply` (writes the `setup`
array in `config.json`).

Both pickers are action loops. Every row is an action — pressing Enter
on it does one definite thing:

- A row (`[x] .env`, `[ ] pnpm -r install`) toggles its checkbox and
  re-renders, leaving the cursor on the same row.
- `+ Add new pattern…` / `+ Add new command…` opens a text prompt; on
  confirm the entry is inserted above the sentinel as a checked row
  (deselect later if you change your mind).
- `✔ Done` commits the checked rows and continues.

The patterns picker's row set is the four preconfigured patterns
(`.env`, `**/.env`, `.env.local`, `**/.dev.vars`) plus any existing
custom patterns already in `setup_config.json` (preselected). Each row
is preselected when its pattern matches at least one file under the
repo root, or when it's already present in `setup_config.json`. Rows
with zero current filesystem matches carry a dim orange `(no matches)`
suffix.

The setup-commands picker's row set is a fixed preconfigured list —
`./.superset/setup.sh`, `pnpm -r install`, `pnpm -r run cf-typegen`,
`npm ci`, `yarn install --frozen-lockfile`, `uv sync` — plus any
existing custom entries already in `config.json`'s `setup` array
(preselected). Each row is preselected when its detection signal trips
or when it's already present in `setup`. Detection reads root
lockfiles (`pnpm-lock.yaml` → pnpm; `package-lock.json` → npm;
`yarn.lock` → yarn; `pnpm` wins when JS lockfiles coexist; `uv.lock` →
`uv sync`) plus the root `package.json` `scripts` map for
`cf-typegen`. Rows that didn't trip detection carry a dim orange
`(not detected)` suffix. `./.superset/setup.sh` is treated as detected
by default (deselectable).

Inline validation on the pattern text prompt rejects absolute paths,
`..` segments, uncompilable globs, and duplicates with a one-line
reason. Custom patterns use the same glob syntax as `setup.sh` (`*`,
`?`, `[abc]`, `**` for any depth). The command text prompt rejects
only empty strings and duplicates — no shell-syntax checks.

The CLI captures every decision (multi-select selection, the `.envrc`
choice, and the finishing action) before writing anything to the
working tree. Writes are staged in a tempdir as the prompts run and
copied into `.superset/` and `.envrc` only after the finishing action
prompt completes. Ctrl-C / Esc at any prompt leaves the working tree
untouched.

When `.env` exists at the repo root and `.envrc` does not, the tool
offers to create `.envrc` with the body `dotenv_if_exists` (compatible
with `direnv`).

After files are written you pick a finishing action:

1. Commit and push to main branch (`origin/main`).
2. Create feature branch (`chore/superset-setup-YYYYMMDD-HHMMSS`),
   commit, push, then `gh pr create --fill --base main`.
3. Done for now (no git operations).

If nothing on disk changed (edit-mode re-run with the same selections),
the commit step is skipped automatically.

### Apply mode (worktree or non-main branch)

Loads the main checkout's `.superset/setup_config.json` and copies the
configured files into the current working tree, then offers to run the
setup commands from the main checkout's `.superset/config.json`. Never
writes inside the worktree's `.superset/` and never invokes git or `gh`.

Glob semantics mirror `.superset/setup.sh` exactly:

- Absolute patterns (`/etc/foo`) and patterns containing a `..` segment
  are rejected (counted as skipped).
- Literal patterns must exist (counted as skipped when missing).
- Glob patterns with zero matches are non-fatal and uncounted.
- Matches inside `node_modules` or `.venv` are dropped (uncounted, logged
  gray as "excluded").
- Matched directories are copied recursively.
- Existing files in the destination are overwritten.

After the file copy completes, apply mode reads
`<main_checkout>/.superset/config.json` and offers to run the `setup`
array as one ` && `-joined shell invocation. The confirm-before-run
prompt shows:

- A bulleted list of the commands.
- The exact shell invocation that will execute (e.g.,
  `/bin/zsh -lc "pnpm install && uv sync"`).
- The working directory (the worktree being applied into).
- A note that the file copy has already completed; declining setup
  leaves the files in place but skips the commands.
- The env vars exposed to commands: `SUPERSET_ROOT_PATH` (main checkout
  absolute path) and `SUPERSET_WORKSPACE_PATH` (worktree absolute path).

**Shell.** Commands run under `$SHELL -lc` so shell rc files are sourced
and nvm / pnpm / asdf / mise / pyenv shims land on `PATH`. When `$SHELL`
is unset the executor falls back to `/bin/sh -c` (no `-l`, because POSIX
`sh` does not support `-l`).

**Empty array.** When `config.json` is absent or its `setup` array is
empty, the executor offers to run `<main_checkout>/.superset/setup.sh`
directly via `bash <path>` (no shell wrapping — paths with spaces are
safe).

**`SUPERSET_WORKSPACE_NAME` divergence.** The upstream Electron app
injects `SUPERSET_WORKSPACE_NAME` from its workspace database. This Rust
CLI has no equivalent and does not inject the variable. Setup scripts
that reference it will need modification.

**Failure.** A non-zero exit fails the CLI with a message naming the
exit code. The file copy is NOT rolled back. Recovery: fix the issue,
then either run the failing commands directly in the worktree, or
re-run `ss-magic` and decline the file-copy step on the second
prompt so you only re-run setup.

**Side effects to be aware of.** Setup commands may write outside the
worktree (e.g., into `$SUPERSET_ROOT_PATH`) and may spawn backgrounded
daemons (`docker compose up -d`, `pnpm dev &`) that outlive the CLI.
Both match upstream and are by design.

**Trust.** The file copy lands `.superset/` content from the worktree's
branch onto your main checkout *before* the setup-confirm prompt. Only
apply branches whose `.superset/` contents you trust — declining setup
does not undo the file copy.

## Environment

- `NO_COLOR` — set to disable ANSI color output. Stdout is also checked
  for TTY support and color is auto-disabled when piping.

## Re-run behavior

Bootstrap mode is safe to re-run. `.superset/setup.sh` is always
overwritten with the embedded canonical body. `.superset/config.json`
is always rewritten from the merged Config — the picker selection
drives `setup`, while `teardown` and `run` are preserved verbatim from
the existing file. `.superset/setup_config.json` is rewritten from your
new selection, with non-preconfigured entries preserved verbatim.

A re-run whose `config.json` byte-output matches the existing file
emits a "Setup commands unchanged" info line. A malformed pre-existing
`config.json` is a hard error (same surface as `setup_config.json`).
A re-run with no changes skips the commit step.

Apply-mode re-runs after a failed setup re-prompt for the file copy
first. If you've started fixing things locally in the worktree,
**decline the file-copy prompt on the re-run** so your edits aren't
clobbered — then accept the setup-confirm prompt to retry the
commands.

## Commands

The bare invocation chooses bootstrap or apply mode automatically based
on where you run it (see Modes below). Two non-interactive subcommands
are also available:

- `ss-magic sync` — non-interactive forward file copy (main → current
  worktree).
- `ss-magic update` — force a self-update to the latest release.

Run `ss-magic --help` for the full list.

## Make targets

```
make build    # cargo build --release
make install  # cargo install --path .
make clean    # cargo clean
```
