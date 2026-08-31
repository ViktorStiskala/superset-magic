# Contributing to ss-magic

Thanks for your interest in improving ss-magic. This document covers building
from source, running the tests, and what a good PR looks like here. One thing
to keep in mind throughout: the files this tool moves commonly contain
secrets, so changes to path handling, overwrite behavior, gitignore rules,
archives, or self-update deserve particular care.

## Building from source

You need a Rust toolchain, provided by [rustup](https://rustup.rs/) (CI builds
on stable), and `git` on `PATH`. The GitHub CLI (`gh`) is optional — it's only
needed for the interactive finishing action that opens a PR, and for verifying
release attestations. Working on the Claude Code plugin additionally needs
`python3` (the packaging and release-assertion script) and `bash` – the
bootstrap suite is written for bash 3.2, the version macOS ships, so no
associative arrays, no `mapfile`, no `${var^^}`.

Install straight from git without cloning:

```sh
cargo install --git https://github.com/ViktorStiskala/superset-magic
```

…or from a clone of this repo:

```sh
make build     # cargo build --release
make install   # cargo install --path .
make clean     # cargo clean
```

Both install paths drop `ss-magic` into `$CARGO_HOME/bin` (usually
`~/.cargo/bin`). ss-magic is not yet published to crates.io.

**Tip:** the binary self-updates on bare / `sync` / `reverse-sync` / `pack`
invocations. While testing a local build, export `SS_MAGIC_NO_UPDATE=1` so the
auto-updater doesn't replace your development binary with the latest GitHub
release.

## Code layout

Source is layered so the pure logic stays unit-testable in isolation from the
interactive layer, and grouped by purpose under `src/`:

- `git/` — git plumbing (read-only probes and mutating primitives; all git/gh
  interaction shells out via `std::process::Command` — **no `git2`**).
- `sync/` — pattern validation and the glob/exclude/copy engine shared by
  forward sync, reverse sync, and pack; `merge.rs` owns the reverse-sync
  push/pull/merge decision model and per-hunk merge assembly (`similar`-based
  diffing); `reverse_sync.rs` owns the backup-first, TOCTOU-guarded apply seam
  that writes a cockpit decision to disk.
- `tui/` — the interactive layer: `inquire` menus and pickers, styling, the
  pure diff/decision models (`diffmodel`, also built on `similar`), and the
  full-screen `ratatui`
  reverse-sync merge cockpit (`cockpit`, on the `crossterm` backend).
- `workspace/` — `.superset/` contract I/O and the init/migration lifecycle.
- `update/` — the self-update check and apply paths.
- `plugin/` – the Claude Code plugin's verb tree: `hook/` (the stdin decode,
  gates, per-event handlers and JSON envelope), `checklist/` (the typed
  document, its ordering, validator, renderer and verbs), and the state and
  reporting modules beside them. Two rules shape it: a hook answers the harness
  with JSON on stdout and must always exit 0, while a human verb reports on
  stderr and exits non-zero – and only human verbs may write configuration, so
  a repository cannot arrange its own enablement by getting a hook to fire.
  Nothing here touches the update gate or the TUI.
- `pack.rs`, `hashing.rs`, `cli.rs`, `main.rs` – the pack engine, the shared
  content-hash primitives (FNV-1a for cache keys, a hand-rolled SHA-256 the
  plugin's shell bootstrap has to reproduce with `shasum`), the hand-rolled arg
  parser (**no `clap`** – this is also where the `-n`/`--no-backup` flag for
  `sync`/`reverse-sync` is parsed), and composition (update gate, dispatch,
  event rendering).

Outside `src/`, the plugin ships as a packaged tree: `plugin/` (its manifest,
hooks, bootstrap script, wrapper and skills), `.claude-plugin/marketplace.json`
(which pins that tree's zip by SHA-256), `scripts/build-plugin-zip.py` (the
reproducible builder and the release assertions), `scripts/test-bootstrap.sh`,
and `assets/workflow/checklist.yml` (embedded into the binary by
`plugin/setup_ci.rs`).

`assets/magic.sh` is the canonical wrapper script, embedded into the binary
via `include_str!` — edit it there, never in a repo's generated `.superset/`
copy. Domain vocabulary (main checkout, forward/reverse sync, sync patterns,
candidates) is defined in [CONCEPTS.md](./CONCEPTS.md).

A few boundaries to preserve:

- Pattern syntax checks live in `sync/pattern.rs` and expansion (with the
  default `node_modules` / `.venv` excludes) in `sync/apply.rs` — don't add a
  second glob implementation with divergent semantics.
- The sync and pack engines emit typed events through caller-supplied
  closures; rendering and terminal side effects belong in `main.rs` / `tui/`,
  which also keeps the engines testable.
- Keep new logic out of the interactive layer where possible so it stays
  unit-testable.
- The excluded-trees filter (`sync::under_excluded_tree` over
  `sync::EXCLUDED_TREES`) must be applied at every point of **final
  enumeration** – each directory walk – never only on an upstream match list. A
  later step that re-walks the filesystem would otherwise re-admit an excluded
  subtree through an ancestor directory match.
- `ss-magic plugin` must stay outside the auto-update gate and must never
  construct the interactive menu. `should_run_update_gate` is an inclusion list
  over `Command`, and `plugin` is not a `Command` at all – keep it that way.

## Tests

`cargo test` is no longer the whole suite. Run all four the way CI does:

```sh
cargo test --locked                              # the Rust suite
python3 scripts/build-plugin-zip.py --selftest   # the plugin builder's own tests
python3 scripts/build-plugin-zip.py --check      # the release assertions
/bin/bash scripts/test-bootstrap.sh              # the bootstrap's failure paths
```

The last three cover code `cargo test` cannot reach:

- `--selftest` exercises the builder's reproducibility guarantees (sorted
  entries, fixed 1980-01-01 timestamps, normalized modes, stored not deflated)
  and its loud refusals (a symlink or a non-ASCII filename, either of which
  would make the digest depend on which platform built it).
- `--check` asserts the three release invariants: the marketplace entry actually
  carries a `sha256` key, the four version surfaces agree, and the committed
  digest matches the current `plugin/` tree. Run it after any change under
  `plugin/` – re-pin first with `--update-manifest`.
- `test-bootstrap.sh` drives `plugin/hooks/bootstrap.sh` through the failure
  modes that matter (offline, corrupted download, hostile version pin,
  unwritable data directory, unsupported platform, concurrent sessions) using a
  `curl` shim, asserting for each that it exits 0, prints nothing on stdout, and
  leaves any pre-existing binary untouched. Pass `-v` for per-assertion output.

Conventions worth knowing:

- Each module declares `#[cfg(test)] mod tests;` with the body in a sibling
  child file (`<module>/tests.rs`), keeping private-item access – including
  every module under `src/plugin/`. Crate-root
  integration tests and shared helpers live in `src/tests/` (`sync.rs`,
  `reverse_sync_flow.rs`, `update_gate.rs`, `support.rs`).
- Tests use `tempfile` plus shell-invoked `git init` / `git worktree add` to
  build real repos — no git mocking. They must not depend on or mutate your
  real repositories, global git config, clipboard, or installed `ss-magic`.
- The interactive menu/pickers and the final-action git operations
  (commit/push/PR) have no unit tests; they are validated by manual smoke
  testing. If your change touches one of those surfaces, describe the manual
  path you exercised in the PR.
- The reverse-sync merge cockpit (`tui/cockpit.rs`) is a partial exception:
  its event loop and terminal lifecycle are manual-smoke like the rest of the
  interactive layer, but its render path (`draw`) and pure key dispatch
  (`handle_key`) ARE unit-tested by driving `ratatui::backend::TestBackend`
  with synthetic key events — no real terminal required. Prefer extending
  those tests over adding new manual-smoke-only cockpit behavior.
- Test an exclusivity property by actually racing it, never sequentially. A
  "consume exactly once" claim that is checked by calling it twice in a row
  proves the state machine, not the exclusion – see
  [docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md](./docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md)
  for the version of that mistake this repo shipped and measured.
- Give any parser over line-oriented command output a **single-line** fixture.
  A defect that corrupts only the first line hides completely behind a
  multi-line one; that is exactly how a trimmed `git status --porcelain` went
  unnoticed.

CI (`.github/workflows/ci.yml`) runs the Rust suite on Ubuntu and macOS for
every PR commit and every push to `main`, plus a `plugin package` job carrying
the builder selftest, the release assertions, a check that a content change
under `plugin/` came with a version bump, a check that no skill body names
`CLAUDE_PLUGIN_DATA`, and the bootstrap failure-path suite. The same workflow
gates releases: the cargo-dist release pipeline invokes it as a plan job, so a
release cannot ship with a red suite.

## Pull requests

- Make sure `cargo test --locked` passes locally; add or update tests for
  behavior-bearing changes (bug fixes should include a test that reproduces
  the issue).
- Make sure the three non-Rust suites above pass too, if your change touches
  `plugin/`, `scripts/`, or anything they assert about.
- **Bump the crate version** (`version` in `Cargo.toml` and the matching
  `ss-magic` entry in `Cargo.lock`) on any change that alters CLI behavior — a
  fix, a new/changed command or flag, or different output. The installed
  binary self-updates from GitHub Releases keyed on version, so a change
  without a version bump never reaches users. Pre-1.0 rules: bug fixes bump
  patch; new or changed user-visible behavior bumps minor.
- **A change under `plugin/` bumps four version surfaces, not one**, and
  re-pins the digest. Run `python3 scripts/build-plugin-zip.py
  --update-manifest`, then `--check`, which asserts that `Cargo.toml`,
  `plugin/.claude-plugin/plugin.json`, `plugin/ss-magic.version` and the release
  URL in `.claude-plugin/marketplace.json` all agree and that the committed
  digest matches the tree. The resolved *version*, not the digest, is what tells
  the Claude Code client to update: changing the zip and its `sha256` without
  bumping the version leaves every installed user silently on the cached copy.
- Update the docs in the same PR: `README.md` must describe the tool as it is
  after your change, and `CLAUDE.md` / `.cursor/BUGBOT.md` must reflect any
  architecture or convention change. `.cursor/BUGBOT.md` has to stay
  self-contained – restate a convention inline there rather than linking to it.
- Keep the secret-safety invariants intact unless the change is explicitly
  about them: absolute / `..` patterns rejected, reverse-synced paths always
  gitignored in main, no overwrite of an existing main-checkout file without a
  diff + explicit confirm, pack never following symlinks or packing itself,
  staged/atomic writes for `.superset/` and archives, and the excluded trees
  (`.superset/backups`, `.superset/.magic`, `.scratchpad`, `.git`) pruned during
  every directory walk.
- Keep the plugin's two postures intact: a hook always exits 0 and never prints
  anything but its JSON envelope on stdout, while the gates that protect secrets
  refuse on "could not determine" as well as on "no". Do not simplify either
  half toward the other.

## Releases and versioning

Releases are built and published to GitHub Releases by
[cargo-dist](https://opensource.axo.dev/cargo-dist/) (configured in
`dist-workspace.toml`), which also generates the one-line installer script and
per-archive checksums. The pipeline runs the locked test suite before building
macOS (arm64/x86-64) and Linux (arm64/x86-64) archives, and attests the
per-target `.tar.gz` archives with signed build provenance (Sigstore/Rekor);
users can verify them with `gh attestation verify` as described in the README.
The self-updater itself trusts the TLS-authenticated download plus cargo-dist
checksums — it does not consume the attestations.

The Claude Code plugin ships on the same release, as an extra asset:
`scripts/build-plugin-zip.py` packs `plugin/` into
`ss-magic-plugin-v<version>.zip` byte-reproducibly, and
`.claude-plugin/marketplace.json` pins that zip by SHA-256 with a release-asset
URL. Reproducibility is what lets the digest be computed and committed *before*
the release exists and re-derived identically by CI; `.gitattributes` marks
`plugin/**` as `-text` so a checkout's line-ending conversion can never move it.

Because the binary self-updates from the latest release, the version number is
the release mechanism: publishing a release with a higher version rolls it out
to every installed binary within a day (or immediately via `ss-magic update`).
The plugin's binary is the exception – it is pinned by
`plugin/ss-magic.version` and updated only when the plugin itself is, so the
shipped hooks, skills and Markdown never describe a different binary than the
one running them.

## License

ss-magic is dual-licensed under [MIT](./LICENSE-MIT) and
[Apache-2.0](./LICENSE-APACHE). Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in the work by you, as
defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
