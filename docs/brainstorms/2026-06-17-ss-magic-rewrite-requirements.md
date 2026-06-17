---
date: 2026-06-17
topic: ss-magic-rewrite
---

# ss-magic: self-updating rewrite of superset-setup

## Summary

Rewrite `superset-setup` into a single self-updating Rust binary named
`ss-magic`. Bare invocation opens a menu of location-appropriate operations
(init/migrate/edit config in the main checkout; forward/reverse sync from a
worktree); `ss-magic sync` replaces the embedded `setup.sh` file copy; and
`ss-magic update` (plus an automatic check on every run) keeps the binary
current from GitHub Releases. Per-repo config moves to `magic.json`
(committed) with `magic.local.json` (gitignored) overriding it, both copied
into worktrees on sync. A committed `.superset/magic.sh` wrapper lets
Superset setup degrade gracefully when `ss-magic` isn't installed. The
project is being prepared to split out of the monorepo and ship open-source.

## Problem Frame

The current tool ships file-copy logic in two places: the embedded
`assets/setup.sh` (bash, written verbatim to every repo's `.superset/`) and
`src/apply.rs`, which already re-implements the same expansion/copy
semantics in Rust. The bash copy carries real cost — it needs `bash >= 4`
and `jq`, it is the thing `config.json`'s `setup` array points at, and it
duplicates logic the binary already owns. Collapsing onto the binary
removes the duplication and the runtime dependencies.

Distribution and upgrades are the second pain point. The tool installs via
`cargo install` from a local checkout, so every machine that wants a fix has
to rebuild from source. As the tool moves toward an open-source, standalone
release, it needs a real distribution channel (GitHub Releases) and a way to
keep itself current without a manual rebuild — ideally invisibly, so a
workspace created tomorrow runs the version released today.

The third gap is directional and is the one with teeth. File sync only flows
main → worktree today. When a developer edits a synced file inside a
worktree — most dangerously an *untracked* secret file like `.dev.vars` that
gained a new key — there is no supported path to get that change back to the
main checkout. Tracked config (`magic.json`) is recoverable: it is
version-controlled and lands in main through the normal merge. Untracked
files are not: if the worktree is deleted after merge, a new `.dev.vars` key
that was never copied back is simply lost. Reverse sync exists to rescue
exactly those untracked files before the worktree goes away.

## Actors

- A1. **Developer** — runs `ss-magic` interactively and picks an operation
  from the menu: in the main checkout (init, migration, config edits), and
  from a worktree (forward sync, reverse sync). Also runs `ss-magic update`
  on demand.
- A2. **Superset.app** — runs `config.json`'s `setup` array during workspace
  creation, which (post-migration) invokes `./.superset/magic.sh sync`
  non-interactively inside the new worktree.
- A3. **GitHub Releases** — the update source. Multi-arch binaries plus a
  release manifest published by CI; queried for "is there a newer version"
  and downloaded from on update.

## Key Decisions

- **Thin `sync` — files only.** `ss-magic sync` copies the configured files
  and nothing else (a literal `setup.sh` replacement). Setup *commands*
  (`pnpm -r install`, `uv sync`, …) stay as separate entries in Superset's
  `config.json` `setup` array, run by Superset after sync. `config.json`
  keeps owning orchestration; `magic.json` stays a plain file list.

- **`magic.sh` wrapper for graceful degradation.** `config.json` invokes
  `./.superset/magic.sh sync`, not `ss-magic` directly. The committed wrapper
  delegates to `ss-magic` when installed; when it isn't, it prints a bold-red
  error with install instructions and exits 0, so a teammate without the
  binary doesn't crash Superset's workspace setup.

- **Menu-driven mode, not location-auto.** Bare `ss-magic` no longer picks
  bootstrap-vs-apply silently by location. It presents a menu of the
  operations valid for where it runs, then a submenu. Forward sync is an
  explicit choice even inside a worktree, because the main checkout may have
  gained new `.dev.vars` (or other synced files) from another source since
  the worktree was created.

- **Reverse sync moves untracked files only.** It pushes back exactly the
  git-untracked files that match the worktree's `magic.json` +
  `magic.local.json` (including `magic.local.json` itself). Tracked files
  reach main through the normal git merge, so reverse sync never touches them
  and there is no "sync `magic.json` back" operation.

- **Auto-update on every invocation, including `sync`.** Always run the
  newest version, accepting one network check (5s timeout) on the first run
  per day. The updater must not hand control back to its caller until the
  replacement binary has finished, so Superset never advances to the next
  setup command mid-swap; offline/failed checks fall through silently to the
  installed version.

- **Migration is interactive and main-checkout-only.** The destructive
  rewrite runs only when the developer runs bare `ss-magic` in the main
  checkout and detects the old layout; it lands as ordinary working-tree
  edits routed through the existing finishing-action prompt — never
  auto-committed, never triggered from a worktree `sync`.

- **`magic.local.json` overlays `magic.json` per key, unioning arrays.** When
  both are present, `files` arrays are unioned and de-duplicated (local
  patterns add to the shared set); scalar/object keys in local win. Local
  cannot remove a shared pattern in v1.

- **GitHub Releases via cargo-dist, not R2/S3.** Distribution mirrors the
  `cli-release.yml` reference: cargo-dist builds the multi-arch matrix and
  publishes binaries + manifest + checksums to a GitHub Release on a version
  tag.

## Requirements

### Binary, commands, and the wrapper

- R1. The binary is renamed `superset-setup` → `ss-magic` (Cargo `[[bin]]`
  name, `Makefile`, README, and references all follow).
- R2. Bare `ss-magic` (no subcommand) presents a menu of the operations valid
  for the current location, then a submenu for the chosen operation: in the
  main checkout — init, migration, config editing; from a worktree — forward
  sync and reverse sync. Nothing runs until the developer selects it.
- R3. `ss-magic sync` performs a non-interactive forward file copy (main
  checkout → current working tree) using the glob/exclude semantics
  `apply.rs` implements today, and exits without invoking git, `gh`, or any
  setup commands.
- R4. `ss-magic update` forces a self-update to the latest GitHub Release
  regardless of the 1-day check cache, and reports the resulting version (or
  "already latest").
- R5. A committed `.superset/magic.sh` wrapper is what `config.json`'s `setup`
  invokes (`./.superset/magic.sh sync`). When `ss-magic` is on `PATH` it
  delegates (`ss-magic "$@"`); when it is not, it prints a bold-red error with
  install instructions and exits 0 so Superset setup continues.

### Config model

- R6. Per-repo config lives in `.superset/magic.json` (committed), holding at
  least `{ "files": [pattern, …] }`, replacing `setup_config.json`.
- R7. `.superset/magic.local.json` (gitignored) overlays `magic.json` per the
  overlay decision above; it is optional and developer-managed.
- R8. The tool creates `magic.local.json` if absent — an empty object plus an
  explanatory comment — adds it to the git-root `.gitignore` (adding the entry
  if missing, leaving an existing one untouched), and includes it as a default
  pattern in `magic.json` `files` so forward sync copies it into worktrees.
- R9. Forward `sync` resolves its file patterns from the *main checkout's*
  `magic.json` overlaid with the main checkout's `magic.local.json`.

### Migration and init

- R10. On bare `ss-magic` in the main checkout, branch on `config.json`'s
  `setup`: if it references the old `setup.sh`, migrate (R11); if it already
  references the `magic.sh`/`ss-magic sync` marker, proceed normally; if it
  references neither, run the interactive init flow (first-time bootstrap).
- R11. Migration renames `setup_config.json` → `magic.json`, writes
  `.superset/magic.sh`, sets `config.json`'s `setup` to invoke the wrapper,
  deletes `.superset/setup.sh`, and applies R8.
- R12. Migration replaces the old `setup.sh` entry with the wrapper invocation
  without reordering other `setup` entries, and preserves `config.json`'s
  `teardown` and `run` arrays verbatim.
- R13. Migration is idempotent: re-running against an already-migrated repo is
  a no-op that reports nothing changed.
- R14. Migration and init changes are surfaced to the developer and routed
  through the existing finishing-action prompt (commit & push to main /
  feature branch + PR / done); nothing is auto-committed.

### Self-update

- R15. Every invocation (including `sync` via the wrapper) checks for a newer
  release before doing its work, gated by a check cache with a 1-day TTL.
- R16. The check cache lives in the OS cache directory — `~/Library/Caches/ss-magic`
  on macOS, the XDG cache dir (`~/.cache/ss-magic`) on Linux, the platform
  equivalent on Windows — recording the last-check time and latest version
  seen so a fresh cache needs no network.
- R17. The network check has a 5-second timeout; on timeout, offline, or any
  error, the tool proceeds silently on the currently installed version.
- R18. When a newer version is found, the tool downloads it, verifies its
  checksum, atomically replaces the running binary, and re-executes the new
  binary with the original argv.
- R19. The updater must not return control to its caller until the
  re-executed binary has finished; it propagates the new binary's exit code.
  This guarantees Superset never advances to the next `setup` command while a
  swap is in flight (prefer exec-replacement where the OS supports it, else
  spawn-and-wait).
- R20. The download-and-replace step is serialized with a file lock; a second
  concurrent invocation that cannot take the lock skips the update and
  proceeds on its current version. The lock carries an expiration / stale-lock
  detection so a crashed updater cannot deadlock future runs.
- R21. The re-executed process does not re-trigger the update check (guarded
  by an inherited marker), preventing re-exec loops.

### Reverse sync (worktree → main)

- R22. From a worktree, the menu offers a reverse-sync operation that pushes
  the worktree's untracked synced files back to the main checkout.
- R23. Candidates are files matching the worktree's `magic.json` +
  `magic.local.json` patterns that are git-untracked in the worktree —
  including `magic.local.json` itself. Tracked files (e.g., `magic.json`) are
  excluded; they reach main through the normal git merge.
- R24. The picker is diff-aware: it lists candidates (including worktree-only
  new files), and each row offers a "show diff" action that shells out to
  `diff` with colored output (worktree vs main).
- R25. On copy into main, the tool creates missing parent directories, and
  ensures each copied path is gitignored in main; when a path is not already
  ignored there, it copies the matching `.gitignore` line from the worktree so
  secrets are never accidentally committed in main.
- R26. Reverse sync writes into main only after the developer's explicit
  selection/confirmation; declining leaves main untouched.

### Distribution and packaging

- R27. GitHub Actions builds `ss-magic` for the target matrix — macOS arm64 +
  x86_64, Linux x86_64 + arm64 — via cargo-dist, and publishes the binaries,
  release manifest, and checksums to a GitHub Release triggered by a version
  tag.
- R28. The embedded `assets/setup.sh` and its `include_str!` source-of-truth,
  the bash-script write path, and the `exec.rs` "empty array → run setup.sh"
  fallback are removed; the binary is the sole file-copy implementation.

## Key Flows

- F1. Interactive menu in the main checkout
  - **Trigger:** A1 runs bare `ss-magic` in the main checkout.
  - **Steps:** Auto-update check (F3) → branch on `config.json` `setup` (R10):
    old layout → migrate (R11–R13); marker present → normal; neither → init →
    run the chosen flow's pickers, writing `magic.json`, `magic.local.json`,
    `magic.sh`, and `config.json` → finishing-action prompt (R14).
  - **Outcome:** `.superset/` on the new layout; changes committed or left for
    review per the developer's choice.
  - **Covered by:** R2, R5, R8, R10–R14.

- F2. Workspace creation runs the wrapper
  - **Trigger:** A2 runs `config.json`'s `setup` in a new worktree; one entry
    is `./.superset/magic.sh sync`.
  - **Steps:** Wrapper finds `ss-magic` (or prints install help and exits 0,
    R5) → auto-update check (F3) → resolve patterns from main's `magic.json` +
    `magic.local.json` (R9) → copy matching files into the worktree (R3) →
    exit; Superset runs the remaining `setup` commands.
  - **Outcome:** Worktree populated by the newest `ss-magic`, or setup
    continues uninterrupted when the binary is absent.
  - **Covered by:** R3, R5, R9, R15–R21.

- F3. Auto-update + re-exec (cross-cutting)
  - **Trigger:** Any invocation whose check-cache TTL (R16) has expired and
    which is not a re-executed child (R21).
  - **Steps:** Query A3 for latest (5s timeout, R17) → if newer: take the file
    lock (R20), download, verify checksum, atomic swap (R18) → re-exec with
    original argv and the no-recheck marker, waiting for the child and
    propagating its exit code (R19) → otherwise proceed on current version.
  - **Outcome:** Work runs under the latest version (caller blocked until it
    finishes), or the current version when the check fails/offline.
  - **Covered by:** R15–R21.

- F4. Reverse sync from a worktree
  - **Trigger:** A1 runs bare `ss-magic` in a worktree and selects reverse
    sync.
  - **Steps:** Compute untracked candidates from the worktree's `magic.json` +
    `magic.local.json` (R23) → diff-aware picker with per-file diffs (R24) → on
    confirm, copy selected files into main, creating directories and ensuring
    each is gitignored (R25–R26).
  - **Outcome:** Selected untracked worktree files (incl. `magic.local.json`)
    land safely in main; nothing written on decline.
  - **Covered by:** R22–R26.

## Acceptance Examples

- AE1. Offline `sync`.
  - **Given** a stale check cache and no network, **when** `./.superset/magic.sh sync`
    runs, **then** the check times out at 5s, nothing about updates is logged,
    and the file copy proceeds on the installed version. (R17)

- AE2. Concurrent updates.
  - **Given** two invocations start together and a newer release exists,
    **when** both reach the update step, **then** one takes the lock and
    swaps; the other skips the update and runs on its current version rather
    than waiting. (R20)

- AE3. Updater blocks the caller.
  - **Given** Superset runs `magic.sh sync` followed by `uv sync`, **when**
    `ss-magic` updates and re-execs mid-run, **then** Superset does not start
    `uv sync` until the re-executed `ss-magic` has finished and its exit code
    has propagated. (R19)

- AE4. No re-exec loop.
  - **Given** an update just swapped the binary and re-executed it, **when**
    the child starts, **then** it sees the no-recheck marker and runs the work
    directly without checking again. (R21, R18)

- AE5. Init when neither marker is present.
  - **Given** a repo whose `config.json` `setup` references neither `setup.sh`
    nor the wrapper/`ss-magic sync`, **when** bare `ss-magic` runs in the main
    checkout, **then** it runs the interactive init flow rather than migrating
    or proceeding. (R10)

- AE6. Idempotent migration.
  - **Given** a repo already on the new layout, **when** bare `ss-magic` runs
    in the main checkout, **then** no files are renamed/deleted, the wrapper
    entry is not duplicated, and it reports nothing changed. (R13)

- AE7. Local overlay union.
  - **Given** `magic.json` `files` is `["**/.env"]` and `magic.local.json`
    `files` is `["**/.dev.vars"]`, **when** `sync` resolves patterns, **then**
    it copies matches for both (union), not just the local set. (R7, R9)

- AE8. Wrapper without the binary.
  - **Given** a machine where `ss-magic` is not installed, **when** Superset
    runs `./.superset/magic.sh sync`, **then** the wrapper prints a bold-red
    error with install instructions and exits 0, and Superset setup
    continues. (R5)

- AE9. Reverse sync respects tracking and gitignore.
  - **Given** a worktree with a modified tracked `magic.json` and a new
    untracked `apps/api/.dev.vars`, **when** reverse sync runs, **then**
    `magic.json` is not offered (it merges via git), `.dev.vars` is, and on
    copy the tool creates `apps/api/` in main if missing and ensures the path
    is gitignored there (copying the worktree's `.gitignore` line if not).
    (R23, R25)

## Scope Boundaries

### Deferred for later

- Windows in the build matrix (cargo-dist makes it a small addition when
  wanted).
- A local override that *removes* a shared pattern (v1 union only adds).
- Binary signing / notarization beyond checksum verification.

### Outside this work

- R2/S3 distribution — replaced by GitHub Releases.
- A "sync `magic.json` back to main" operation — `magic.json` is tracked and
  merges via git; reverse sync handles untracked files only.
- The actual repo split and open-source release mechanics (LICENSE, public
  README, repo move) — this brainstorm designs the tool; separation is its
  own task.
- `SUPERSET_WORKSPACE_NAME` injection — still not provided by this tool;
  unchanged limitation from today.

## Dependencies / Assumptions

- Self-update rides cargo-dist's GitHub Releases output and its updater
  surface (e.g., `axoupdater`) or an equivalent GitHub-Releases-backed update
  crate; the 1-day cache, file lock, wait-for-child re-exec, and stale-lock
  expiry are custom on top.
- The "latest version" check uses a cheap GitHub endpoint (release manifest
  or `releases/latest`), not a full binary download.
- Initial install (before self-update can take over) is the cargo-dist
  installer script or `cargo install`; the exact bootstrap is settled in
  planning.
- The OS cache directory is resolved via a standard crate (`dirs`/`directories`)
  to honor `~/Library/Caches`, XDG, and the Windows equivalent.

## Outstanding Questions

### Deferred to planning

- How `magic.local.json` carries a comment given strict JSON (`_comment` key
  vs a JSONC parser).
- The `magic.sh` wrapper contract — exact `ss-magic` detection, the install
  instructions it prints, and that it always exits 0 on the missing-binary
  path.
- How "untracked" is determined for reverse sync (e.g., `git status` /
  `git check-ignore`) and how the matching `.gitignore` line is located and
  copied into main.
- Exact self-update crate/mechanism (`axoupdater` vs `self_update` vs
  hand-rolled), the wait-for-child re-exec per platform, and the
  atomic-swap + stale-lock strategy.
- Version-tag format for the release trigger now that the project is
  standalone (e.g., `vX.Y.Z`).

## Sources / Research

- `projects/superset-setup/src/apply.rs` — existing Rust re-implementation of
  the `setup.sh` copy semantics (the basis for `ss-magic sync`).
- `projects/superset-setup/src/superset_files.rs` — current `.superset/`
  contract I/O, `config.json` `{setup, teardown, run}` shape, and the
  `setup_config.json` → migration starting point.
- `projects/superset-setup/assets/setup.sh` — canonical bash copy logic being
  retired (glob/exclude rules to preserve).
- `md-cli`'s `.github/workflows/cli-release.yml` — cargo-dist → GitHub
  Releases build matrix used as the packaging reference (R2/S3 step dropped).
