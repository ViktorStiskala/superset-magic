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

- **Forward sync** (`ss-magic sync`) – copy every matching file from the
  repo's main checkout into the current worktree, backing up every file it's
  about to overwrite first (skip with `-n`/`--no-backup`). Under Superset this
  runs automatically the moment a workspace is created, via the setup-script
  hook; without Superset it's one command to run in a fresh worktree.
- **Sync** (interactive, worktree menu) – reconcile every configured file
  against the main checkout in either direction, through a full-screen merge
  cockpit: a file list beside a live side-by-side diff, where you set each
  file's direction (push to main / pull from main / per-hunk merge / delete
  from both / undecided) and apply the batch behind one confirmation, with a
  timestamped backup taken before every overwrite or delete. It's how a new
  secret created in a worktree reaches everywhere else, and how a file added
  directly to main reaches a worktree. For scripted use, `ss-magic
  reverse-sync` non-interactively bulk-pushes every git-untracked candidate
  that differs from main.
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

ss-magic also ships a **Claude Code plugin** built on the same
`.superset/magic.json` contract – a durable per-worktree scratchpad, a gate that
keeps oversized file reads out of the context window, an operator checklist, and
a per-session cost ledger. It installs from a marketplace, is off until you
enable it per repository, and is described in
[its own section](#the-claude-code-plugin) below.

If you work with git worktrees and carry per-developer gitignored files, this
tool is for you. It is built for Superset's workspace lifecycle, but forward
sync, reverse sync, pack, and init are ordinary CLI commands that work in any
worktree setup.

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
ss-magic              # interactive operation menu (location-aware)
ss-magic sync         # non-interactive forward copy: main → current worktree
ss-magic reverse-sync # non-interactive bulk copy: current worktree → main,
                       # for git-untracked files matching the configured
                       # patterns
ss-magic pack         # archive the configured files into ss-magic-<repo>.tar.bz2
ss-magic update       # force a self-update to the latest release
ss-magic init [PATTERN...]   # non-interactively seed .superset (magic.json
                             # layout); extra args become magic.json `files`
ss-magic plugin <VERB>       # Claude Code plugin entry point (see below);
                             # never self-updates, never opens the menu
ss-magic --help       # usage
ss-magic --version    # print the version and exit
```

`sync` and `reverse-sync` both take a timestamped backup of every file they're
about to overwrite unless `-n`/`--no-backup` is given – skipping it leaves no
recovery path for an overwritten or deleted untracked secret. The worktree
menu opened by bare `ss-magic` offers a single interactive **Sync** entry that
reconciles files in both directions through the merge cockpit; there's no
separate forward/reverse choice there. `SS_MAGIC_NO_UPDATE=1` disables the
auto-update gate (the explicit `ss-magic update` ignores it and always
checks). `ss-magic plugin` is deliberately outside the gate entirely – it never
checks for an update and never opens the menu, because the plugin's binary is
pinned to the skills and hooks shipped alongside it.

### `ss-magic` — the interactive menu

The bare invocation opens a menu whose options depend on where you run it:

- **Main checkout** — one lifecycle operation, chosen from the detected state:
  init the contract, migrate an old `setup.sh` layout, or edit the
  synced-files config.
- **Worktree** – a single **Sync** entry: the interactive merge cockpit,
  reconciling every configured file against main in both directions.
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

Init and migration both gitignore two things up front: the per-machine
`magic.local.json` overlay and the tool's `.superset/backups/` tree (where a
sync stores the bytes it overwrites, which can include recovered secrets), so
the backup tree is protected before the first sync ever writes to it. Both flows
preserve `config.json`'s `teardown` and `run` arrays verbatim. All
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

By default every worktree file this is about to overwrite is backed up first,
under a gitignored `<worktree>/.superset/backups/<YYYYmmdd-HHMMSS>/…`; pass
`-n`/`--no-backup` to skip it. Forward sync is not offered from the worktree
menu – the menu's single **Sync** entry (below) covers pulling from main too.

### `ss-magic reverse-sync` – bulk push (worktree → main)

Non-interactive, worktree → main, files-only: pushes every configured file
that is git-**untracked** in the current worktree and differs from main.
"Untracked" includes **gitignored** files – that is the point, since this
command exists for secrets like `.env` / `.dev.vars` (and the gitignored
`magic.local.json`), which never merge via git. Tracked files are never
touched by this command – they reach main via a normal merge; use the
interactive **Sync** menu entry below if you want to push a tracked file's
local edits into main's working copy.

1. Resolve the current repo root and the main checkout root; hard error
   (non-zero exit) if run from the main checkout itself – there is nothing to
   push.
2. Compute the untracked candidates matching the overlaid patterns that differ
   from main (identical files are skipped, nothing to do).
3. For each: back up main's existing bytes first (unless `-n`/`--no-backup`)
   under a gitignored `<main>/.superset/backups/<YYYYmmdd-HHMMSS>/…`, ensure
   the path is gitignored in main (the same secret-safety gate the interactive
   cockpit uses – see below), then write the worktree's bytes into main,
   creating the file there if it was absent.

`-n`/`--no-backup` skips the pre-overwrite backup – the only recovery path for
an overwritten or deleted untracked secret, so use it deliberately. Exit code
is non-zero if any file failed to apply.

### Sync (worktree ↔ main, via the menu)

From a worktree's menu, the single **Sync** entry reconciles every configured
file against the main checkout **in either direction**, tracked or untracked.
Candidates come from expanding the overlaid patterns against *both* roots and
classifying each path into one of four situations (directory matches and
anything under the tool's own `.superset/backups/` tree are dropped before
classification, so a directory pattern or a recovered backup copy is never
offered):

- **Differs** – exists on both sides with different bytes.
- **Worktree-only** – absent in main; a push *creates* it there.
- **Main-only** – absent in the worktree; a pull *creates* it locally, a
  delete removes main's copy (push is unavailable – there is no worktree copy
  to push).
- **Identical** – hidden; nothing to reconcile.

The flow:

- Opens a full-screen merge cockpit: a file list – long paths wrap onto
  additional lines instead of clipping – beside a live diff, side-by-side on a
  wide terminal (with a faint divider between the two columns) or unified when
  narrow; binary / oversized files show a whole-file notice instead of a diff.
  In both the split and the unified view, local additions/changes render
  **green** and main additions/changes render **red**. A worktree-only or
  main-only file instead shows its content as numbered `+` lines under a
  colored header ("new file – will be created in main" in green, "main
  only – will be created in this worktree" in cyan). Diffs are EOL-normalized
  (CRLF → LF, trailing newline) so hunks reflect content changes only; a pair
  that differs *only* by line endings says so instead of showing an empty diff.
- Nothing is pre-selected – every file starts *undecided*, including a
  worktree-only file (a bare Enter never auto-pushes anything). You set each
  file's direction with explicit keys: `p` push to main, `l` pull from main,
  `m` interactive merge, `d` delete from both sides, `u` undecided
  (arrows/`j`/`k` navigate, `PgUp`/`PgDn`/`Space` scroll the diff, `←`/`→`
  scroll long lines horizontally, `?` toggles help). Lines wider than the pane
  are flagged in its title ("lines continue →") so a change past the right
  edge – a trailing comment, a long value – is never silently invisible. Each
  row's mtimes are shown only as an unreliable hint.
- `m` on a differing text file opens a per-hunk merge overlay: walk the hunks
  with the arrows and cycle each between keep-local / keep-main / keep-both
  (`←`/`→` or `h`/`l`) while a live preview assembles the result
  (`PgUp`/`PgDn`/`Space`/`b` scroll a long preview); `Enter` accepts
  it and `Esc` cancels. The accepted bytes are written to **both** sides on apply
  so they stop differing (normalized to LF + trailing newline). Merge is
  unavailable for binary / oversized / worktree-only / main-only files (which
  offer only push/pull as applicable).
- `Enter` opens one batched confirmation listing every existing-target
  overwrite and delete (a delete names exactly which side(s) it removes, e.g.
  "delete (main copy)" for a main-only file). `Enter` again applies; `Esc`
  backs out to the file list, changing nothing. Before each destructive write
  or unlink, the losing bytes are copied to a timestamped backup under the
  worktree's gitignored `.superset/backups/<YYYYmmdd-HHMMSS>/{worktree,main}/…`,
  whose path is printed so a mistaken decision is recoverable; the 10 newest
  backup batches are kept and older ones pruned after each apply. A file
  changed on either side since you reviewed it is skipped rather than
  clobbered.
- Gitignore-safety: a push into main only touches main's `.gitignore` when the
  worktree source is git-**untracked** – pushing a **tracked** file instead
  updates main's working copy in place with no `.gitignore` change (it's
  recoverable via the pre-write backup and ordinary `git restore`, like any
  other overwrite in the batch). Tracked-ness is determined positively; a path
  that can't be confirmed tracked is treated as a secret. When an untracked
  push isn't already gitignored in main, ss-magic adds a rule to the closest
  existing `.gitignore` among the file's ancestor directories (else main's
  root `.gitignore`, creating it if absent), preferring the worktree's own
  covering rule (e.g. `**/.dev.vars`) over a literal path – the guard that
  prevents a reverse-synced secret from becoming committable in main.

The cockpit needs an interactive terminal; run piped or in CI it refuses to
launch and writes nothing – use the non-interactive `ss-magic sync` (main →
worktree) or `ss-magic reverse-sync` (worktree → main, untracked-only)
instead. Pressing `Esc` – or applying with everything undecided – leaves both
sides fully untouched.

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

## The Claude Code plugin

An optional [Claude Code](https://claude.com/claude-code) plugin, published from
this repository and built on the same `.superset/` contract. It is off until you
turn it on, and it does four things:

- **A durable session scratchpad.** Each worktree gets
  `.superset/.magic/sessions/<repo>-<branch>/` with `STATUS.md`, `TASKS.md`,
  `DECISIONS.md`, `LEARNINGS.md`, `CONTEXT.md` and `OPERATOR-CHECKLIST.md`, so
  working state survives a context compaction by living on disk. The directory
  name is derived from git alone, so the same worktree always resolves to the
  same place. It is gitignored, never committed, and the plugin refuses to write
  anything until git confirms that.
- **A read gate with a conclusion cache.** A `Read` of a file past the
  configured size threshold is denied and routed to an Explore agent instead, so
  a large file never lands whole in the context window. The agent's answer is
  recorded, and any later read of the same file is answered with that conclusion
  inline. The gate is advisory, not a security boundary – a timeout, a malformed
  envelope, or a missing binary all leave the read to proceed.
- **An operator checklist.** One JSON document per action under
  `docs/actions/`, with the steps a change needs before it is safe to ship. The
  plugin's own verbs are the only write path (direct reads and edits of the file
  are denied), which is what keeps the document canonically ordered and valid,
  and a GitHub Actions workflow can render it into a pull-request comment.
- **A cost ledger.** One row per ended session, read from that session's own
  transcript, using the harness's priced records where they exist and a
  versioned price table otherwise. A relative signal for comparing branches,
  never an authoritative bill.

### Install and enable

The marketplace is the only delivery path – there is no `ss-magic plugin
install`, and `ss-magic sync` never installs anything. In Claude Code:

```plaintext
/plugin marketplace add ViktorStiskala/superset-magic
/plugin install ss-magic
```

The marketplace entry pins the plugin archive by SHA-256, so the client refuses
an archive whose bytes do not match. On the first session after install, a
`SessionStart` hook downloads the pinned `ss-magic` binary the hooks run,
verifying it against the release's published checksum before anything is moved
into place. That bootstrap never fails a session: offline, DNS failure, proxy,
404, checksum mismatch, unwritable data directory, or an unsupported platform
all end in "do nothing, one line on stderr, carry on", and an already-installed
binary is never touched by a failed install.

Then, in each repository you want it to act on:

```sh
ss-magic plugin enable          # writes plugin.enabled into .superset/magic.json
ss-magic plugin enable --local  # ...or into the gitignored magic.local.json
ss-magic plugin status          # what it sees, and why it is or isn't acting
```

Both layers must be on: the plugin must be installed and enabled in Claude Code,
**and** `plugin.enabled` must be true for the repository. `ss-magic plugin
status` is the one place that reports both, plus whether the state tree is
gitignored, whether the binary arrived, and whether its version matches the
plugin's pin – run it first whenever nothing seems to be happening.

**The first session after installing does nothing, and that is expected.** The
plugin ships without a binary; a `SessionStart` hook fetches the pinned release
in the background. Hooks on one event run at the same time, so that first session
is already underway before the download lands, and every ss-magic hook is
deliberately inert for it – silently, without failing anything. Start a new
session and the plugin is live.

For the same reason, `/reload-plugins` alone is not enough. It re-registers the
plugin but emits no session-start event, so nothing fetches or updates the
binary, and what you get depends on which case you are in:

- **After a fresh install**, there is no binary at all, so every hook stays
  inert until the next real session.
- **After a version bump**, the previously installed binary is still there and
  keeps serving hooks – so the session runs the *old* binary against the *new*
  manifest and skills until a fresh session's bootstrap swaps it.

`ss-magic plugin status` tells the two apart: it reports whether the binary
arrived at all, and whether its version matches the plugin's pin.

### Verbs

```plaintext
ss-magic plugin status            # what the plugin sees, and why it is or isn't acting
ss-magic plugin cost              # what recorded sessions cost, across every worktree
ss-magic plugin spill-index       # list the harness's own oversized-output files
ss-magic plugin scratchpad ensure # create/refresh this worktree's state tree
ss-magic plugin conclude <FILE>   # record a conclusion about a file
ss-magic plugin conclusions [KEY] # list or show recorded conclusions
ss-magic plugin gc                # prune expired plugin state
ss-magic plugin bypass <FILE>     # let the next Read of that file through, once
ss-magic plugin expect-artifact <FILE> [--note TEXT]
                                  # require the next subagent to produce a file
ss-magic plugin enable  [--local] # turn the hooks on for this repository
ss-magic plugin disable [--local] # stop them acting (leaves the install alone)
ss-magic plugin config get <plugin.DOTTED.KEY>      # e.g. plugin.gate.threshold_lines
ss-magic plugin config set <plugin.DOTTED.KEY> <VALUE> [--local]
ss-magic plugin compact-window --set <TOKENS>
                                  # opt into an absolute auto-compaction window
ss-magic plugin setup-github-ci [--check] [--force]
                                  # write the checklist PR-comment workflow
ss-magic plugin checklist <SUBVERB>
                                  # init, add-item, add-entry, set, done, list,
                                  # verify, render-md — the only write path
ss-magic plugin --help
```

`status`, `cost` and `spill-index` also take `--json`. Inside a Claude Code
session the shipped skills invoke these through a `ss-magic-plugin` wrapper on
the session's `PATH`, so `ss-magic-plugin checklist list` is the same command.

### Hooks

The plugin registers five hook events:

| Event | What it does |
| --- | --- |
| `SessionStart` | Installs/refreshes the pinned binary, scaffolds the scratchpad, and injects the operating guidance. |
| `PreToolUse` | The read gate, the checklist-file deny, and an advisory nudge to update the checklist before `git commit` / `git push` / `gh pr create`. |
| `PreCompact` | Records that a compaction is about to happen. Never blocks or slows it. |
| `SubagentStop` | Salvages a subagent's result text, and blocks the stop once if a declared artifact is missing. |
| `SessionEnd` | Writes the session's cost-ledger row. |

Every hook fails open: an error, a panic, or a timeout looks to Claude Code
exactly like a hook that decided to do nothing, because a tool that is only
advisory must never break a session in progress. The commit nudge in particular
never blocks the command it comments on.

### Configuration

The plugin reads a `plugin` block from the same overlaid `magic.json` /
`magic.local.json` pair the sync patterns live in:

```json
{
  "files": [".env"],
  "plugin": {
    "enabled": true,
    "gate": {
      "threshold_lines": 3000,
      "inline_byte_budget": 10000,
      "exemptions": ["docs/reference/*.md"]
    }
  }
}
```

`enabled` is always read from the **main checkout's** config, because a
worktree's own `magic.local.json` is itself a forward-sync target. Every field
is optional and every malformed value falls back to a safe default rather than
failing – an out-of-range number is clamped, not rejected. Read and write them
with `ss-magic plugin config get` / `set`, which preserve every other key in the
file.

### What it stores, and where

- `.superset/.magic/` in each worktree – the scratchpad, conclusion cache, and
  the one-shot bypass / expected-artifact records. Gitignored (the plugin
  refuses to write until git says so), owner-only, and excluded from sync and
  pack, so it never travels between trees or into an archive.
- A per-machine data directory outside any repository – the hook heartbeat log
  and the cost ledger, which have to outlive the worktrees they describe.

Nothing is sent anywhere: every file above is local, and the only network access
the plugin makes is the bootstrap's download of the pinned binary from this
repository's GitHub releases.

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

Forward sync, the Sync menu's reconcile set, `ss-magic reverse-sync`, and pack
all expand the same overlaid pattern list with the same rules:

- Patterns are repo-relative. Absolute patterns (`/etc/foo`) and patterns
  containing a `..` segment are rejected (counted as skipped).
- Literal patterns must exist (counted as skipped when missing); invalid glob
  syntax is also a counted skip.
- Glob patterns with zero matches are non-fatal and uncounted.
- Matches inside `node_modules` or `.venv` are dropped at any depth
  (uncounted, logged gray as "excluded").
- Matches are de-duplicated by relative path. Matched directories are
  copied/archived recursively by forward sync and pack; the Sync menu and
  `ss-magic reverse-sync` reconcile individual files only, so a directory
  match yields no candidate of its own.
- Four directory trees are excluded from every operation – forward sync's copy
  walk, the Sync menu's reconcile set, `ss-magic reverse-sync`, and pack:
  `.superset/backups/` (the tool's own copies of overwritten bytes, so a
  recovered secret is never re-offered or re-archived), `.superset/.magic/` (the
  Claude Code plugin's gitignored local state), `.scratchpad/`, and `.git/`.
  Each is matched as an exact path, so a sibling like `.superset/.magicked/` is
  unaffected – and `.superset/` itself is never excluded, so the contract files
  still sync and pack normally. The exclusion is applied during the directory
  walk, so a broad pattern such as `.superset` or `**` cannot re-admit the
  subtrees through an ancestor match.
- Existing files in the destination are overwritten (forward sync; the Sync
  menu instead classifies every match against both roots – differs /
  worktree-only / main-only / identical – and reconciles them in the merge
  cockpit, a batched confirm and a pre-write backup gating every overwrite;
  `ss-magic reverse-sync` narrows further to untracked-only candidates pushed
  straight into main).
- Matching uses [`globset`](https://docs.rs/globset): unlike a POSIX shell
  glob, `*` can cross path separators. Quote patterns on the command line so
  your shell doesn't expand them first.

## Self-update

Every invocation of `ss-magic` (bare), `sync`, `reverse-sync`, and `pack` runs
a cheap, daily-cached check for a newer GitHub release. `init`, `--help`,
`--version` and every `ss-magic plugin` verb skip the gate entirely; the
`update` subcommand forces its own path instead. (`plugin` skips it by design,
not by omission: the plugin's binary is pinned alongside the skills and hooks
shipped with it, so a silent mid-session swap would leave the two describing
different behavior.)

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
| `CLAUDE_CONFIG_DIR` | Read (not set) by `ss-magic plugin` to locate Claude Code's own state when it is not at `~/.claude`. |
| `CLAUDE_PLUGIN_ROOT` | Set by Claude Code for a running plugin; read to find the version pin. Not meant to be set by hand. |

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
