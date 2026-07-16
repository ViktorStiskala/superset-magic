# ss-magic

Keep gitignored files — `.env` secrets, local overrides, machine-specific
config — in sync across git worktrees: automatically when a
[Superset](https://superset.sh) workspace is created, on demand from the
command line anywhere else.

## The problem

Git worktrees share your repo's history, branches, and objects — but not its
gitignored files. Create a new worktree and every file git ignores stays
behind in the original checkout: `.env`, `.dev.vars`, local database configs,
per-developer overrides. The new tree is "clean" in the worst way — nothing
runs until you hand-copy your secrets in, and you re-do that copy for every
worktree you create.

The same asymmetry bites in reverse: add or rotate a secret *inside* a
worktree and it's stranded there. Gitignored files never travel through a
merge, so the main checkout — and every future worktree created from it —
silently misses the update.

## What ss-magic does

You declare the files once, as glob patterns in a committed
`.superset/magic.json`. ss-magic then gives you three operations over that one
file set:

- **Forward sync** (`ss-magic sync`) — copy every matching file from the
  repo's main checkout into the current worktree. Under Superset this runs
  automatically the moment a workspace is created, via the setup-script hook;
  without Superset it's one command to run in a fresh worktree.
- **Reverse sync** (interactive menu) — reconcile git-untracked files that
  changed or appeared in a worktree against the main checkout, through a
  full-screen merge cockpit: a file list beside a live side-by-side diff, where
  you set each file's direction (push to main / pull from main / per-hunk merge /
  undecided) and apply the batch behind one confirmation, with a timestamped
  backup taken before every overwrite. This is how a new secret created in a
  worktree reaches everywhere else.
- **Pack** (`ss-magic pack`) — snapshot the whole configured file set into a
  single `ss-magic-<repo>.tar.bz2` for backup, machine migration, or handing
  to a teammate.

```plaintext
main checkout                              linked worktree
  .env                -- forward sync -->    .env
  config/local.json   <-- reverse sync --    config/local.json
      \
       `-- pack --> ss-magic-<repo>.tar.bz2
```

Tracked files are deliberately out of scope — they already travel through
normal git commits and merges. And ss-magic is not a secrets manager: the
files remain ordinary files on disk, and you decide which paths may be copied
or packed.

If you work with git worktrees and carry per-developer gitignored files, this
tool is for you. It is built for Superset's workspace lifecycle, but forward
sync, pack, and init are ordinary CLI commands that work in any worktree
setup.

## How it works with Superset

Superset workspaces are isolated git worktrees. When Superset creates one, it
runs the `setup` commands from `.superset/config.json` sequentially inside the
new worktree (see the
[setup & teardown scripts docs](https://docs.superset.sh/setup-teardown-scripts)).
ss-magic's init writes this hook for you:

```json
{
  "setup": ["./.superset/magic.sh sync"]
}
```

`magic.sh` is a small committed wrapper: it `exec`s the installed `ss-magic`
binary, and if the binary isn't installed it prints an install hint and exits
0 — a missing ss-magic never blocks Superset's setup pipeline. ss-magic's role
in the hook is the file copy only; dependency installation, migrations, and
dev servers stay in Superset's own `config.json` commands. The result: every
new workspace starts with your secrets and local config already in place.

## Install

### One-line installer (recommended — macOS & Linux)

```sh
curl -sSfL https://github.com/ViktorStiskala/superset-magic/releases/latest/download/ss-magic-installer.sh | sh
```

It fetches the right prebuilt binary for your platform and puts `ss-magic` on
your `PATH`. From then on the binary keeps itself current — see
[Self-update](#self-update).

**Supported platforms:** macOS (Apple Silicon and Intel) and Linux (x86-64 and
arm64). Windows is not in the release matrix yet.

### Manual download

Grab the archive for your platform plus its `.sha256` from the
[latest release](https://github.com/ViktorStiskala/superset-magic/releases/latest),
verify the checksum, extract, and move `ss-magic` onto your `PATH`.

Building from source is covered in [CONTRIBUTING.md](./CONTRIBUTING.md).

### Verify a release

Releases after v0.2.0 attest the platform archives (`ss-magic-<target>.tar.gz`)
with signed build provenance. This repo is public, so attestations are recorded
in Sigstore's public [Rekor](https://docs.sigstore.dev/logging/overview/)
transparency log. Verify a downloaded archive with the
[GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify ss-magic-aarch64-apple-darwin.tar.gz -R ViktorStiskala/superset-magic
```

This proves the archive was built by this repository's release workflow from a
specific commit — provenance, not a security audit of the contents. Only the
`.tar.gz` archives are attested; the installer script and `.sha256` files are
not (the installer's integrity path is TLS plus the checksummed archives it
downloads). Release notes also include a `gh attestation verify --bundle`
variant generated by cargo-dist — both commands are equivalent checks.

## Getting started

In your repo's **main checkout** (the primary checkout your worktrees are
linked to), run:

```sh
ss-magic
```

and pick **init** from the menu. It walks you through selecting the file
patterns to sync and writes the [`.superset/` contract](#the-superset-contract)
— `config.json` with the setup hook, the `magic.sh` wrapper, `magic.json` with
your patterns, and a gitignored `magic.local.json` overlay. You then choose a
finishing action: commit and push, open a PR, or leave the changes on disk.

For scripted provisioning there's a non-interactive form:

```sh
ss-magic init '.env' '**/.dev.vars' 'config/local/*'
```

Quote glob patterns so your shell doesn't expand them before ss-magic sees
them. The non-interactive init leaves the generated files uncommitted on disk.

Once the contract is committed, every worktree created through Superset.sh app
gets the matching files copied in automatically. In a worktree created any
other way, run `ss-magic sync` yourself.

## Commands

```plaintext
ss-magic            # interactive operation menu (location-aware)
ss-magic sync       # non-interactive forward copy: main → current worktree
ss-magic pack       # archive the configured files into ss-magic-<repo>.tar.bz2
ss-magic update     # force a self-update to the latest release
ss-magic init [PATTERN...]   # non-interactively seed .superset (magic.json
                             # layout); extra args become magic.json `files`
ss-magic --help     # usage
```

Reverse sync is an option in the worktree menu opened by bare `ss-magic` —
there is no separate reverse-sync subcommand. `SS_MAGIC_NO_UPDATE=1` disables
the auto-update gate (the explicit `ss-magic update` ignores it and always
checks).

### `ss-magic` — the interactive menu

The bare invocation opens a menu whose options depend on where you run it:

- **Main checkout** — one lifecycle operation, chosen from the detected state:
  init the contract, migrate an old `setup.sh` layout, or edit the
  synced-files config.
- **Worktree** — forward sync (main → here), or reverse sync (push untracked
  files from here back to main).
- **Pack** is offered wherever an initialized `magic.json` exists (any
  worktree, or the main checkout once set up).

Nothing runs until you pick it; Esc / Ctrl-C leaves the tree untouched.

#### Init and migration (main checkout)

From the main-checkout menu, ss-magic branches on `config.json`'s `setup`:

- An entry referencing the old `./.superset/setup.sh` → **migrate**: rename
  `setup_config.json` → `magic.json` (carrying its patterns along), write
  `magic.sh`, replace the `setup.sh` entry in place with
  `./.superset/magic.sh sync`, delete `setup.sh`, bootstrap
  `magic.local.json` + its `.gitignore` entry.
- A `magic.sh` / `ss-magic` marker only → **edit config**.
- Neither marker (or absent `config.json`) → **init** the contract.

Both flows preserve `config.json`'s `teardown` and `run` arrays verbatim. All
changes are staged into a tempdir and materialized only after the
finishing-action prompt returns a non-cancel choice, so picking "done" or
aborting leaves the old layout intact — never a half-migrated tree. Migration
warns that worktrees created before the migration keep the old `setup.sh` /
`setup_config.json` and should be recreated.

After files are staged you pick a finishing action:

1. Commit and push to the main branch.
2. Create a feature branch, commit, push, then `gh pr create --fill`.
3. Done for now (no git operations).

If nothing on disk changed, the commit step is skipped automatically.

### `ss-magic sync` — forward sync (main → worktree)

Non-interactive, files-only — the command the Superset app setup hook runs:

1. Resolve the main checkout root (parent of `git --git-common-dir`).
2. Require `.superset/magic.json` there (hard error, non-zero exit, if absent
   or malformed — a visible failure beats a silent no-copy inside Superset
   setup).
3. Load the overlaid config (`magic.json` + `magic.local.json`) from main.
4. Copy every match into the current working tree, following the
   [pattern semantics](#pattern-semantics) below. Matched directories are
   copied recursively; existing files in the destination are overwritten.

No git/gh operations, no setup commands — setup commands live in Superset's
own `config.json` and are run by Superset.

Forward sync is also offered from the worktree menu, because main may have
gained files since the worktree was created.

### Reverse sync (worktree → main, via the menu)

From a worktree's menu, reconcile **git-untracked** files matching the overlaid
patterns against the main checkout. "Untracked" includes **gitignored** files
— that is the point, since the files worth pushing are secrets like `.env` /
`.dev.vars` (and the gitignored `magic.local.json`), which never merge via
git. Tracked files are excluded — they reach main via merge. The flow:

- Collects differing / worktree-only candidates (identical files are hidden)
  and opens a full-screen merge cockpit: a file list beside a live diff
  (side-by-side on a wide terminal, unified when narrow; binary / oversized
  files show a whole-file notice instead of a diff).
- Nothing destructive is pre-selected — a file that differs starts *undecided*.
  You set each file's direction with explicit keys: `p` push to main, `l` pull
  from main, `m` interactive merge, `u` undecided (arrows/`j`/`k` navigate,
  `PgUp`/`PgDn`/`Space` scroll the diff, `?` toggles help). Each row's mtimes are
  shown only as an unreliable hint.
- `m` on a differing text file opens a per-hunk merge overlay: walk the hunks
  with the arrows and cycle each between keep-local / keep-main / keep-both
  (`←`/`→` or `h`/`l`) while a live preview assembles the result; `Enter` accepts
  it and `Esc` cancels. The accepted bytes are written to **both** sides on apply
  so they stop differing. Merge is unavailable for binary / oversized / new
  files (which offer only push/pull).
- `Enter` applies: one batched confirmation lists every existing-target
  overwrite (defaulting to No). Before each destructive write, the losing bytes
  are copied to a timestamped backup under a gitignored `.superset/backups/`,
  whose path is printed so a mistaken overwrite is recoverable. A file changed
  on either side since you reviewed it is skipped rather than clobbered.
- Gitignore-safety: if a path pushed into main isn't already gitignored there,
  ss-magic copies the worktree's covering `.gitignore` rule (resolved via
  `git check-ignore -v --no-index`) into main's root `.gitignore` (creating it
  if absent), falling back to the literal path when no covering rule exists.
  This is the guard that prevents a reverse-synced secret (e.g. `.dev.vars`)
  from becoming committable in main.

The cockpit needs an interactive terminal; run piped or in CI it refuses to
launch and writes nothing (`ss-magic sync` is forward-only). Pressing `Esc` —
or applying with everything undecided — leaves both sides fully untouched.

### `ss-magic pack` — archive the configured files

Snapshot the files defined by the config into a single portable archive —
useful for backup, transfer to a new machine, or handing the bundle to a
teammate. Non-interactive, and also offered from the menu wherever an
initialized `magic.json` exists. The flow, all relative to the current git
repo root:

1. Resolve the current repo root; require `.superset/magic.json` there (hard
   error, non-zero exit, if absent or malformed).
2. Load the overlaid config (`magic.json` + `magic.local.json`) and expand the
   patterns with the same [pattern semantics](#pattern-semantics) as forward
   sync (matched directories included recursively, de-duped).
3. Write every match — preserving its repo-relative path — into
   `ss-magic-<repo>.tar.bz2` at the git root. Compression is bzip2; the
   archive is a standard `.tar.bz2` any `tar` can read.

The archive name identifies the repo: with an `origin` remote it is derived
from the normalized remote URL — `ss-magic-viktorstiskala_upx-cz.tar.bz2` for
`github.com/ViktorStiskala/upx.cz`, identical whether origin uses `https://`,
`ssh://`, or the `git@host:` form (GitLab nested groups keep every path
segment). Without an origin, the primary worktree's directory basename is used
instead (`ss-magic-upx-cz.tar.bz2` for a checkout at `.../upx.cz`). After
packing, ss-magic prints the `tar -xjvf` extraction command and copies the
archive's full path to the clipboard (`pbcopy`, `wl-copy`, `xclip`, or `xsel`,
whichever is available — "full path copied to clipboard" confirms it).

The archive is built to a temp file and atomically renamed into place, and
never packs itself (a stale archive at the root — current or pre-0.3
`ss-magic-files.tar.bz2` name — is excluded even if a broad pattern would
match it). Symlinks are stored as symlink entries, never followed — a matched
link (even to a directory) is recorded as a link, so it can't pull in a target
outside the repo. An empty config, no matches, or a match set that contains
nothing packable is a success with no archive written — and an existing
archive is left untouched rather than replaced by an empty one.

### `ss-magic init [PATTERN...]` — scripted init

The scriptable form of the interactive init: it writes the `.superset/`
contract without prompts (for CI / automated provisioning) and leaves the
changes uncommitted on disk. Extra arguments become the `files` patterns in
`magic.json`. It preserves an existing `magic.local.json`, performs no git/gh
operations, and skips the auto-update gate.

### `ss-magic update` — force a self-update

Checks GitHub for the latest release regardless of the daily cache and reports
the resulting version or "already latest". See [Self-update](#self-update).

## The `.superset/` contract

A repo using ss-magic carries:

- `.superset/config.json` — Superset-owned `{ setup, teardown, run }`. Its
  `setup` array runs `./.superset/magic.sh sync` during workspace creation.
  `teardown` and `run` are preserved verbatim by ss-magic.
- `.superset/magic.sh` — the committed wrapper Superset invokes. It runs
  `command -v ss-magic` then `exec ss-magic "$@"` (propagating the binary's
  real exit code); if the binary is absent it prints a bold-red install hint
  and exits 0, so Superset's setup pipeline is never blocked.
- `.superset/magic.json` — committed `{ files: [pattern, ...] }`. The glob
  patterns of files to sync from main into each worktree.
- `.superset/magic.local.json` — gitignored local overlay of the same shape.
  Patterns here are unioned with `magic.json` (de-duped, `magic.json` order
  first) at sync time, so a developer can add machine-specific patterns
  without committing them.

A typical `magic.json`:

```json
{
  "files": [
    ".superset/magic.local.json",
    ".env",
    "**/.dev.vars"
  ]
}
```

`magic.json` itself is tracked and travels via git; `magic.local.json` is
ignored (ss-magic bootstraps it and adds the `.gitignore` entry).
`.superset/magic.local.json` is a default `magic.json` pattern, so the local
overlay is itself copied into each worktree.

## Pattern semantics

Forward sync, reverse-sync candidate selection, and pack all expand the same
overlaid pattern list with the same rules:

- Patterns are repo-relative. Absolute patterns (`/etc/foo`) and patterns
  containing a `..` segment are rejected (counted as skipped).
- Literal patterns must exist (counted as skipped when missing); invalid glob
  syntax is also a counted skip.
- Glob patterns with zero matches are non-fatal and uncounted.
- Matches inside `node_modules` or `.venv` are dropped at any depth
  (uncounted, logged gray as "excluded").
- Matches are de-duplicated by relative path. Matched directories are
  copied/archived recursively by forward sync and pack; reverse sync offers
  individual untracked files only, so a directory match yields no
  reverse-sync candidate of its own.
- Existing files in the destination are overwritten (forward sync; reverse
  sync instead intersects the matches with git-untracked files and reconciles
  them in the merge cockpit — a batched confirm and a pre-write backup gate
  every overwrite).
- Matching uses [`globset`](https://docs.rs/globset): unlike a POSIX shell
  glob, `*` can cross path separators. Quote patterns on the command line so
  your shell doesn't expand them first.

## Self-update

Every invocation of `ss-magic` (bare), `sync`, and `pack` runs a cheap,
daily-cached check for a newer GitHub release (`init` and `--help` skip the
gate; the `update` subcommand forces its own path instead):

- The version cache lives in the OS cache dir; if it's fresh (< 24 h) no
  network call is made.
- Otherwise `GET /releases/latest` runs with an ETag and a 5 s timeout. Any
  offline / non-200 / timeout response falls through silently on the installed
  version.
- When a newer release is found, ss-magic acquires an advisory lock
  (skip-on-contention), downloads the release archive over TLS, atomically
  swaps the running binary, then re-execs the original command on the new
  binary and blocks until it finishes (propagating its exit code). Integrity
  rests on the TLS-authenticated GitHub download plus cargo-dist's published
  per-archive checksums; there is no separate SHA-256-vs-GitHub-digest check
  and the updater does not consume the release attestations — binary signing
  is a deferred future item.

The gate also covers the non-interactive `sync` inside Superset's pipeline —
the bounded timeouts and block-until-child contract keep it from ever slowing
or breaking an unattended caller.

Escape hatches:

- `SS_MAGIC_NO_UPDATE=1` — skip the auto-update gate entirely (`ss-magic
  update` still checks — it's an explicit request).
- `SS_MAGIC_UPDATED=1` — set internally on the re-exec'd child to prevent
  re-check loops.
- `ss-magic update` — force a check regardless of the 24 h cache and report
  the resulting version or "already latest".

## Environment variables

| Variable | Effect |
| --- | --- |
| `NO_COLOR` | Disable ANSI color output. Stdout is also checked for TTY support and color is auto-disabled when piping. |
| `SS_MAGIC_NO_UPDATE` | Disable the self-update gate. |
| `SS_MAGIC_UPDATED` | Internal re-exec guard preventing update loops — not meant to be set by hand. |

## Contributing

Bug reports and PRs are welcome. Building from source, running the test suite,
and the release/versioning rules are covered in
[CONTRIBUTING.md](./CONTRIBUTING.md). Domain vocabulary (main checkout,
forward/reverse sync, candidates) is defined in [CONCEPTS.md](./CONCEPTS.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
