## Source of truth for magic.sh

`assets/magic.sh` is the canonical wrapper script, embedded into the
binary via `include_str!`. Migration and init write that body to
`.superset/magic.sh`. Edit `assets/magic.sh` and re-run migration/init
to propagate. (The legacy `assets/setup.sh` was deleted in U13 — the
binary is the sole file-copy implementation.)

## Conventions

- All git and gh COMMANDS shell out via `std::process::Command`, through the
  `git_raw` helper in core's `git/mod.rs` – in every crate, the plugin included.
  `git::discover` is the ONE filesystem-only
  reduction of two read-only probes (`--show-toplevel` and
  `--git-common-dir`), wired into the plugin crate alone – the hook pipeline
  plus the two ledger-attribution verbs named in Architecture (it reaches core
  across a crate boundary, which does not widen the rule), and it
  must never grow ref, index, or write handling – anything beyond "where are
  the two roots" is a subprocess. No git-binding crate (`git2`, `gix`) is
  added.
- Glob semantics (originally derived from the retired `setup.sh`):
  absolute / `..` rejected, literals must exist, glob-zero-match
  non-fatal, `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at
  any depth. Now owned by `sync/apply.rs` + `sync/pattern.rs`.
- Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.
  Final-action git ops and the interactive menu/pickers have no unit
  tests — validated by manual smoke. The unified Sync merge cockpit
  (`tui/cockpit.rs`) is a partial exception: its event loop and terminal
  lifecycle are manual-smoke too, but its render path (`draw`) and pure key
  dispatch (`handle_key`) ARE unit-tested by driving
  `ratatui::backend::TestBackend` with synthetic key events.
- Test layout: each module declares `#[cfg(test)] mod tests;` with the
  body in a sibling child file (`<module>/tests.rs`), keeping private-item
  access – in all three crates, the plugin crate included. The CLI's
  crate-root tests live in `crates/ss-magic/src/tests/` (`sync.rs`,
  `reverse_sync_flow.rs`, `update_gate.rs`); the shared helpers are
  `ss_magic_core::testutil` (`crates/ss-magic-core/src/testutil.rs`, the former
  `src/tests/support.rs`), compiled under `cfg(test)` for core's own suite and
  behind core's `testutil` feature for a binary's – which the binary enables
  from its `[dev-dependencies]` ONLY, so no release build contains it. Every
  helper there is `pub`: `git_run`, `neutralize_global_excludes`,
  `exit_code_to_u8`, `init_main_repo`, `write_magic`, `write_file`,
  `make_worktree`, `run_ignored_test_in_child`,
  `run_ignored_test_in_child_from` and `test_path_in_binary`. A test whose
  subject is the process environment – `PATH`, a `GIT_*` variable, or the
  process's own working directory – runs its assertion in a CHILD process via
  `testutil::run_ignored_test_in_child` (or `…_from(cwd, …)` for a cwd) (an
  `#[ignore]`d child test, named through `testutil::test_path_in_binary`,
  spawned from `current_exe()` with the variable set in the child only): the
  suite runs multithreaded, and a variable set with `set_var` is inherited by
  every `git` any other thread spawns during the window, which a mutex around
  the setter does nothing to prevent. Note that `cargo test` starts a test
  binary in its PACKAGE root (`crates/<name>`), not the repository root, so a
  fixture that needs a repository-level directory such as `docs/` to exist in
  the cwd builds a scratch tree and uses the `_from` variant. `set_var` under
  `ENV_LOCK` remains acceptable only for a variable no concurrent test's child
  could misread (`HOME` in the checklist-deny tests). CI
  (`.github/workflows/ci.yml`) runs `cargo test --workspace --locked` on every
  PR commit and gates cargo-dist releases as the `custom-ci` job registered
  under `local-artifacts-jobs` (see dist-workspace.toml and the release-gate
  rule below). As part of that gate it also refuses a release tag matching
  neither `^v[0-9]+\.[0-9]+\.[0-9]+$` nor
  `^ss-magic-plugin-v[0-9]+\.[0-9]+\.[0-9]+$`, since a prefixed CLI tag such as
  `ss-magic-v0.11.1` would publish a release the updater's anchored filter and
  every installed binary ignore.
- **`cargo test --workspace` is no longer the whole suite.** FIVE checks cover
  ground `cargo test` cannot reach, and CI runs all five:
  `python3 scripts/build-plugin-zip.py --selftest` (the builder's own
  reproducibility and refusal tests); `python3 scripts/build-plugin-zip.py
  --check`, which now prints EIGHT assertion lines – `R101 marketplace sha256
  key`, `R95 version surfaces (ss-magic)`, `R95 version surfaces
  (ss-magic-plugin)`, `distinct release lines`, `hooks spawn through the shim`,
  `workspace shape`, `release gate blocks publishing` and `R96 committed digest
  pin` (the hooks-shim manifest guard; the "no `self_update`/`inquire`/`ratatui`
  in the plugin or core manifest plus `publish = false` on core" guard; the
  generated `release.yml`'s host job waiting for and checking the test gate, see
  the release-gate rule below; and the digest);
  `cargo tree --locked -p ss-magic-plugin -i <crate>` for each of `self_update`,
  `inquire` and `ratatui`, which must report no match – a build proves a
  dependency PRESENT and can never prove one ABSENT, so this is the only evidence
  for that requirement; and
  `/bin/bash scripts/test-bootstrap.sh` (the bootstrap's failure paths –
  offline, corrupted download, hostile pin, unwritable data dir, unsupported
  platform, concurrent sessions – each asserting exit 0, empty stdout, and an
  untouched pre-existing binary; PLUS the hook shim's own inertness contract,
  which drives `plugin/hooks/run-hook.sh` directly and asserts exit 0 with both
  streams empty and the binary un-invoked when it cannot resolve one; PLUS a
  manifest invariant asserted over EVERY `hooks.json` entry rather than the
  bootstrap group alone. Written for bash 3.2, so no associative arrays, no
  `mapfile`, no `${var^^}`; its `pass`/`fail` counters and every `assert_*`
  helper live in `scripts/lib/test-harness.sh`, sourced by both shell suites
  so the two cannot drift); and `/bin/bash scripts/test-mark-latest.sh` (the
  latest-mark selection behind the post-announce job, against a fake `gh` over
  the AE1 tag list: the plugin tag, a draft, a prerelease and a wrongly-prefixed
  CLI tag are never chosen, `v0.11.10` beats `v0.11.3` numerically, a `v*` tag
  never invokes `gh`, a dry run edits nothing, and a mark that did not take is
  exit 1 – also bash 3.2, where a `case` arm cannot sit inside `$( … )` and a
  `read` loop must handle an unterminated last line or drop the newest
  release when it is listed last).
- The plugin's packaged tree is **content-pinned**. Any change under `plugin/`
  moves the zip's digest, so it must be followed by `python3
  scripts/build-plugin-zip.py --update-manifest` and then `--check`, and by a
  version bump on every surface of the `ss-magic-plugin` GROUP –
  `crates/ss-magic-plugin/Cargo.toml`, the `ss-magic-plugin` entry in
  `Cargo.lock`, `plugin/.claude-plugin/plugin.json`,
  `plugin/ss-magic-plugin.version`, both the tag and the asset name in
  `.claude-plugin/marketplace.json`'s release URL, and the literal zip filename
  in the plugin crate's `[[package.metadata.dist.extra-artifacts]]`.
  `--check` enumerates them; do not work from a count. The resolved VERSION, not
  the digest, is the harness's update signal: changing the zip and its `sha256`
  without bumping the version leaves every installed user silently on the
  cached copy.
- After a PLUGIN release, the repository-wide `releases/latest` mark must name
  a bare `v*` release again (R12): `gh release create` marks whatever it just
  published as latest, and every `ss-magic` binary released before the
  per-line check (0.11.0) polls that mark and parses only a bare `vX.Y.Z`, so
  a plugin release left holding it makes those installs report "up to date"
  until the next CLI release. cargo-dist's post-announce job does it:
  `dist-workspace.toml` sets `post-announce-jobs = ["./mark-latest"]`, so the
  generated `release.yml` gains `custom-mark-latest` (needs `plan` and
  `announce`, `uses: ./.github/workflows/mark-latest.yml` with the plan
  manifest as `plan`, `secrets: inherit`), whose one step reads
  `announcement_tag` from the plan and runs `scripts/mark-latest.sh`. The
  script exits 0 at once for a bare `v*` tag; otherwise it lists releases
  (`--limit 200` – the default 30 would truncate), DROPS drafts and
  prereleases, keeps only exact `v` + triple tags, picks the numerically
  greatest (`v0.11.10` over `v0.11.3`, sorted on the bare triple – a
  mid-field sort key was not numeric on BSD sort), runs `gh release edit
  <tag> --latest`, and reads `releases/latest` back, FAILING the job if it
  does not name that tag so a token that cannot edit shows up red.
  `MARK_LATEST_DRY_RUN=1` prints the chosen tag instead. The workflow is also
  `workflow_dispatch`-able with the tag as input (the documented fallback);
  it asks for `contents: write`, which `release.yml` grants at workflow level.
  `scripts/test-mark-latest.sh` drives the script against a fake `gh` over
  the AE1 tag list (bash 3.2, run by CI's `plugin` job under Linux and again
  on the macOS leg of the `test` job under `/bin/bash` 3.2 itself, like the
  bootstrap suite); `dist generate
  --check` must stay green after any `dist-workspace.toml` change, and the
  `release.yml` is REGENERATED, never hand-edited.
- **The release test gate must block the PUBLISH, not only the builds.**
  cargo-dist's `host` job (the one that runs `gh release create`) publishes
  when `plan` succeeded and each build job succeeded OR WAS SKIPPED. A job
  upstream of the builds whose result `host` never checks can therefore fail,
  skip every build, and still let `host` publish a release with no assets. That
  is exactly what a `plan-jobs` entry is: upstream of `build-local-artifacts`,
  absent from `host`'s `needs` and `if`. On 2026-09-30 the gate (then
  `plan-jobs = ["./ci"]`) failed on a date-dependent test, and `host` published
  `v0.11.1` and `ss-magic-plugin-v1.0.0` holding only `dist-manifest.json`.
  Release immutability forbids adding assets, and GitHub never lets a deleted
  immutable release's tag be reused. The tag ruleset forbids moving or deleting
  the tags. So both versions were lost for good, and the fix shipped as 0.11.2
  and 1.0.1. The two empty releases remain, marked as pre-releases (so neither
  updater selects them) and titled "broken – no assets, do not use". The gate is now
  `local-artifacts-jobs = ["./ci"]`: cargo-dist adds such a job to `host`'s
  `needs` AND checks its result in `host`'s `if`, so a red or cancelled suite
  skips `host`, `announce` and `custom-mark-latest`. The builds run alongside the
  tests, so a red gate still spends build minutes and leaves attested archives in
  Actions storage, but uploads nothing. `ci.yml` declares the optional `plan`
  input cargo-dist passes to such a job (GitHub rejects an undeclared one).
  cargo-dist 0.33 keeps the same `host` condition, so an upgrade is no
  substitute. `--check`'s `release gate blocks publishing` asserts the shape on
  the GENERATED `release.yml`. `custom-ci` must call `ci.yml`, and `host` must
  need it and check its result. Every job `build-local-artifacts` waits on must
  appear as `needs.<job>.result` in `host`'s `if`, so any future `plan-jobs`
  entry fails there rather than in a release. Independently of the gate, push a
  release tag only after CI on `main` is green for that exact commit. A red
  release still burns its version number: nothing is published, but the tag is
  protected and cannot move. See
  [docs/solutions/logic-errors/cargo-dist-plan-job-failure-publishes-empty-release.md](../../docs/solutions/logic-errors/cargo-dist-plan-job-failure-publishes-empty-release.md).
- Version bumps follow the GROUP, never the repository. Bump
  `crates/ss-magic/Cargo.toml` and the matching `ss-magic` entry in `Cargo.lock`
  on any change that alters CLI behavior – a fix, a new/changed command or flag,
  or different output; that binary self-updates from GitHub Releases keyed on
  version (see Build), so a stale version means users never receive the change.
  Bump the plugin group (above) on any change under `plugin/` or in
  `crates/ss-magic-plugin/`. `crates/ss-magic-core/Cargo.toml` is not a surface
  at all. The two groups' versions must never be EQUAL – `--check`'s `distinct
  release lines` assertion refuses that, because a bare `vX.Y.Z` tag would then
  announce both packages. Bug fixes bump patch; new/changed user-visible
  behavior bumps minor (pre-1.0 on the CLI line; the plugin line follows
  ordinary semver from 1.0 – its first release with assets is `1.0.1`, since
  `ss-magic-plugin-v1.0.0` was published empty, see the release-gate rule
  above).
- After every implementation change, update `CLAUDE.md` and `README.md`
  to match the current state before the change is considered done. A
  new/changed command, flag, module, or behavior must be reflected in the
  README (command list + relevant prose) and in this doc's Architecture +
  Conventions sections; `CONTRIBUTING.md` must likewise be updated when
  build, test, or release-workflow facts change — the docs are expected to
  describe the code as it is now, not as it was.
- `.cursor/BUGBOT.md` holds the Cursor Bugbot review rules. It must stay
  **self-contained**: it cannot reference this `CLAUDE.md`,
  `docs/solutions/`, `.cursor/rules`, or any skill/rule — restate the
  relevant conventions inline instead. Keep it **synchronised on every
  change**: whenever a convention here or a behavior in the code changes,
  update `.cursor/BUGBOT.md` in the same change so its rules never describe
  stale conventions.
