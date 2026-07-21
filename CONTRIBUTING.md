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
release attestations.

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
- `pack.rs`, `cli.rs`, `main.rs` — the pack engine, the hand-rolled arg parser
  (**no `clap`** — this is also where the `-n`/`--no-backup` flag for
  `sync`/`reverse-sync` is parsed), and composition (update gate, dispatch,
  event rendering).

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

## Tests

Run the suite the same way CI does:

```sh
cargo test --locked
```

Conventions worth knowing:

- Each module declares `#[cfg(test)] mod tests;` with the body in a sibling
  child file (`<module>/tests.rs`), keeping private-item access. Crate-root
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

CI (`.github/workflows/ci.yml`) runs the suite on Ubuntu and macOS for every
PR commit and every push to `main`. The same workflow gates releases: the
cargo-dist release pipeline invokes it as a plan job, so a release cannot ship
with a red suite.

## Pull requests

- Make sure `cargo test --locked` passes locally; add or update tests for
  behavior-bearing changes (bug fixes should include a test that reproduces
  the issue).
- **Bump the crate version** (`version` in `Cargo.toml` and the matching
  `ss-magic` entry in `Cargo.lock`) on any change that alters CLI behavior — a
  fix, a new/changed command or flag, or different output. The installed
  binary self-updates from GitHub Releases keyed on version, so a change
  without a version bump never reaches users. Pre-1.0 rules: bug fixes bump
  patch; new or changed user-visible behavior bumps minor.
- Update the docs in the same PR: `README.md` must describe the tool as it is
  after your change, and `CLAUDE.md` / `.cursor/BUGBOT.md` must reflect any
  architecture or convention change.
- Keep the secret-safety invariants intact unless the change is explicitly
  about them: absolute / `..` patterns rejected, reverse-synced paths always
  gitignored in main, no overwrite of an existing main-checkout file without a
  diff + explicit confirm, pack never following symlinks or packing itself,
  staged/atomic writes for `.superset/` and archives.

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

Because the binary self-updates from the latest release, the version number is
the release mechanism: publishing a release with a higher version rolls it out
to every installed binary within a day (or immediately via `ss-magic update`).

## License

ss-magic is dual-licensed under [MIT](./LICENSE-MIT) and
[Apache-2.0](./LICENSE-APACHE). Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in the work by you, as
defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
