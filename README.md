# ss-magic

Self-updating Rust CLI for the Superset workspace contract. It keeps
per-developer files (env files, secrets, local overrides) in sync between
a repo's main checkout and its linked worktrees, and bootstraps/migrates
the `.superset/` contract a repo needs for Superset.

## Install

### Released build (recommended — macOS & Linux)

Install the latest release with the one-line installer:

```sh
curl -sSfL https://github.com/ViktorStiskala/superset-magic/releases/latest/download/ss-magic-installer.sh | sh
```

It fetches the right prebuilt binary for your platform and puts `ss-magic` on
your `PATH`. From then on the binary keeps itself current — see
[Self-update](#self-update).

Prefer to download by hand? Grab the archive for your platform plus its
`.sha256` from the
[latest release](https://github.com/ViktorStiskala/superset-magic/releases/latest),
verify the checksum, extract, and move `ss-magic` onto your `PATH`.

**Supported platforms:** macOS (Apple Silicon and Intel) and Linux (x86-64 and
arm64). Windows is not in the release matrix yet.

### From source

With a Rust toolchain (via `rustup`):

```sh
cargo install --git https://github.com/ViktorStiskala/superset-magic
```

…or from a clone of this repo:

```sh
make install   # cargo install --path .
```

Both drop `ss-magic` in `$CARGO_HOME/bin` (usually `~/.cargo/bin`). ss-magic is
not yet published to crates.io.

## Commands

```
ss-magic            # interactive operation menu (location-aware)
ss-magic sync       # non-interactive forward copy: main → current worktree
ss-magic update     # force a self-update to the latest release
ss-magic init [PATTERN...]   # non-interactively seed .superset (magic.json
                             # layout); extra args become magic.json `files`
ss-magic --help     # usage
```

`ss-magic init` is the scriptable form of the interactive init: it writes the
`.superset/` contract without prompts (for CI / automated provisioning) and
leaves the changes uncommitted on disk. `SS_MAGIC_NO_UPDATE=1` disables the
auto-update check for any invocation.

The bare invocation opens a menu whose options depend on where you run it:

- **Main checkout** — init the contract, migrate an old `setup.sh` layout,
  or edit the synced-files config.
- **Worktree** — forward sync (main → here), or reverse sync (push
  untracked files from here back to main).

Nothing runs until you pick it; Esc / Ctrl-C leaves the tree untouched.

## The `.superset/` contract

A repo using ss-magic carries:

- `.superset/config.json` — Superset-owned `{ setup, teardown, run }`.
  Its `setup` array runs `./.superset/magic.sh sync` during workspace
  creation. `teardown` and `run` are preserved verbatim by ss-magic.
- `.superset/magic.sh` — the committed wrapper Superset invokes. It runs
  `command -v ss-magic` then `exec ss-magic "$@"` (propagating the
  binary's real exit code); if the binary is absent it prints a bold-red
  install hint and exits 0, so Superset's setup pipeline is never blocked.
- `.superset/magic.json` — committed `{ files: [pattern, ...] }`. The glob
  patterns of files to sync from main into each worktree.
- `.superset/magic.local.json` — gitignored local overlay of the same
  shape. Patterns here are unioned with `magic.json` (de-duped,
  `magic.json` order first) at sync time, so a developer can add
  machine-specific patterns without committing them.

`magic.json` itself is tracked and travels via git; `magic.local.json` is
ignored (ss-magic bootstraps it and adds the `.gitignore` entry).
`.superset/magic.local.json` is a default `magic.json` pattern, so the
local overlay is itself copied into each worktree.

## Forward sync (`ss-magic sync`)

Non-interactive, files-only, main → current worktree:

1. Resolve the main checkout root (parent of `git --git-common-dir`).
2. Require `.superset/magic.json` there (hard error, non-zero exit, if
   absent or malformed — a visible failure beats a silent no-copy inside
   Superset setup).
3. Load the overlaid config (`magic.json` + `magic.local.json`) from main.
4. Copy every matching file into the current working tree.

No git/gh operations, no setup commands — setup commands live in
Superset's own `config.json` and are run by Superset.

Glob semantics:

- Absolute patterns (`/etc/foo`) and patterns containing a `..` segment
  are rejected (counted as skipped).
- Literal patterns must exist (counted as skipped when missing).
- Glob patterns with zero matches are non-fatal and uncounted.
- Matches inside `node_modules` or `.venv` are dropped (uncounted, logged
  gray as "excluded").
- Matched directories are copied recursively.
- Existing files in the destination are overwritten.

The forward sync is also offered from the worktree menu, because main may
have gained files since the worktree was created.

## Reverse sync (worktree → main)

From a worktree menu, push **git-untracked** files matching the overlaid
patterns back to the main checkout. Tracked files are excluded — they
reach main via merge. The flow:

- Builds a diff-aware picker of differing / worktree-only candidates, each
  with a "show diff" action (paged via `git diff --no-index`).
- On copy: creates missing parent dirs in main; a candidate that already
  exists in main requires a per-file diff + explicit confirm before
  overwrite.
- Gitignore-safety: if a copied path isn't already gitignored in main,
  ss-magic copies the worktree's covering `.gitignore` rule (resolved via
  `git check-ignore -v --no-index`) into main's root `.gitignore`
  (creating it if absent), falling back to the literal path when no
  covering rule exists. This is the guard that prevents a reverse-synced
  secret (e.g. `.dev.vars`) from becoming committable in main.

Declining at the picker leaves main fully untouched.

## Init / migration (main checkout)

From the main-checkout menu, ss-magic branches on `config.json`'s `setup`:

- An entry referencing the old `./.superset/setup.sh` → **migrate**:
  rename `setup_config.json` → `magic.json`, write `magic.sh`, replace the
  `setup.sh` entry in place with `./.superset/magic.sh sync`, delete
  `setup.sh`, bootstrap `magic.local.json` + its `.gitignore` entry.
- A `magic.sh` / `ss-magic` marker only → **edit config**.
- Neither marker (or absent `config.json`) → **init** the contract.

All changes are staged into a tempdir and materialized only after the
finishing-action prompt returns a non-cancel choice, so picking "done" or
aborting leaves the old layout intact — never a half-migrated tree.
Migration warns that worktrees created before the migration keep the old
`setup.sh` / `setup_config.json` and should be recreated.

After files are staged you pick a finishing action:

1. Commit and push to the main branch.
2. Create a feature branch, commit, push, then `gh pr create --fill`.
3. Done for now (no git operations).

If nothing on disk changed, the commit step is skipped automatically.

## Self-update

Every invocation (except the explicit `update` subcommand's own path)
runs a cheap, daily-cached check for a newer GitHub release:

- The version cache lives in the OS cache dir; if it's fresh (< 24 h) no
  network call is made.
- Otherwise `GET /releases/latest` runs with an ETag and a 5 s timeout.
  Any offline / non-200 / timeout response falls through silently on the
  installed version.
- When a newer release is found, ss-magic acquires an advisory lock
  (skip-on-contention), downloads the release archive over TLS, atomically
  swaps the running binary, then re-execs the original command on the new
  binary and blocks until it finishes (propagating its exit code). Integrity
  rests on the TLS-authenticated GitHub download plus cargo-dist's published
  per-archive checksums; there is no separate SHA-256-vs-GitHub-digest check,
  and binary signing is a deferred future item.

The gate runs on `bare`, `sync`, and `update` — including the
non-interactive `sync` inside Superset's pipeline. The bounded timeouts
and block-until-child contract keep this reach from ever slowing or
breaking an unattended caller.

Escape hatches:

- `SS_MAGIC_NO_UPDATE=1` — skip the update check entirely.
- `SS_MAGIC_UPDATED=1` — set internally on the re-exec'd child to prevent
  re-check loops.
- `ss-magic update` — force a check regardless of the 24 h cache and
  report the resulting version or "already latest".

## Environment

- `NO_COLOR` — set to disable ANSI color output. Stdout is also checked
  for TTY support and color is auto-disabled when piping.
- `PAGER` — pager for reverse-sync diffs (default `less -R`).
- `SS_MAGIC_NO_UPDATE` — disable the self-update gate.

## Make targets

```
make build    # cargo build --release
make install  # cargo install --path .
make clean    # cargo clean
```
