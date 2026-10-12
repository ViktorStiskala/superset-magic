## Source of truth for magic.sh

`assets/magic.sh` is the canonical wrapper script, embedded into core at
compile time as `superset_files::MAGIC_SH` (`include_str!`). Migration and
init write that body to `.superset/magic.sh`. Edit `assets/magic.sh` and
re-run migration/init to propagate. The binary is the sole file-copy
implementation: no shell copy script ships in `assets/`, and migration deletes
a repository's retired `.superset/setup.sh`.

## Conventions

- Every `git` and `gh` invocation shells out via `std::process::Command` from
  core's `git/mod.rs`, for every crate, the plugin included; no production
  code outside that module spawns git or gh (test code is the exception: the
  `testutil::git_run` helper and several test modules run `git` directly to
  build fixtures, see the test bullet below). Most git probes go through the
  private `git_raw`, which returns the raw `Output`; `git` (trimmed stdout, and a
  non-zero exit becomes an error carrying the verbatim stderr) and
  `git_optional` (a non-zero exit becomes `None`) are thin wrappers on it. The
  exceptions spawn their own `Command`: `nothing_to_commit` (it needs only the
  exit status of `git diff --cached --quiet`) and the two `gh` calls,
  `gh_available` and `pr_create`. (`timestamp_branch_suffix` also lives there
  and spawns `date`, which is neither.) `git::discover` is the ONE
  filesystem-only reduction of two read-only probes (`--show-toplevel` and
  `--git-common-dir`), wired into the plugin crate alone – the hook pipeline
  plus two read-only verbs: `compact-window --recommend` (its own root lookup,
  and the ledger-row attribution in `ledger::rows_for_repository`) and
  `status`, whose compaction section builds the same report through that
  same `rows_for_repository` (see the plugin map,
  `.claude/rules/architecture-plugin.md`). The `cost` verb does not use it:
  `cost --here` resolves its root with the `git::cwd_repo_root` subprocess
  probe. (`tmproot.rs` imports only `discover::effective_uid`, not the walk.)
  It reaches core across a crate boundary, which does not widen the rule, and
  it must never grow ref, index, or write handling – anything beyond "where are
  the two roots" is a subprocess. No git-binding crate (`git2`, `gix`) is added.
- Glob semantics, owned by core's `sync/apply.rs` and `sync/pattern.rs`
  (`pattern::check_syntax` is the one syntax check that both the expansion and
  the picker's validator use): an absolute pattern or one with a `..` segment
  is rejected; a literal must exist, and a missing one is a counted skip; a
  glob matching nothing is logged, not counted, and never fatal;
  `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at any depth.
- Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.
  Final-action git ops and the interactive menu/pickers have no unit
  tests – validated by manual smoke. The unified Sync merge cockpit
  (`tui/cockpit.rs`) is a partial exception: its event loop and terminal
  lifecycle are manual-smoke too, but its render path (`draw`) and pure key
  dispatch (`handle_key`) ARE unit-tested by driving
  `ratatui::backend::TestBackend` with synthetic key events.
- Test layout: each module declares `#[cfg(test)] mod tests;` with the
  body in a sibling child file (`<module>/tests.rs`), keeping private-item
  access – in all three crates, the plugin crate included. The CLI's
  crate-root tests live in `crates/ss-magic/src/tests/` (`sync.rs`,
  `reverse_sync_flow.rs`, `update_gate.rs`); the shared helpers are
  `ss_magic_core::testutil` (`crates/ss-magic-core/src/testutil.rs`), compiled
  under `cfg(test)` for core's own suite and behind core's `testutil` feature
  for a binary's – which the binary enables from its `[dev-dependencies]`
  ONLY, so no release build contains it. Every helper there is `pub`:
  `git_run`, `neutralize_global_excludes`, `exit_code_to_u8`,
  `init_main_repo`, `write_magic`, `write_file`, `make_worktree`,
  `run_ignored_test_in_child`, `run_ignored_test_in_child_from` and
  `test_path_in_binary`. A test whose subject is the process environment –
  `PATH`, a `GIT_*` variable, or the process's own working directory – runs
  its assertion in a CHILD process via `testutil::run_ignored_test_in_child`
  (or `…_from(cwd, …)` for a cwd) (an `#[ignore]`d child test, named through
  `testutil::test_path_in_binary`, spawned from `current_exe()` with the
  variable set in the child only): the suite runs multithreaded, and a
  variable set with `set_var` is inherited by every `git` any other thread
  spawns during the window, which a mutex around the setter does nothing to
  prevent. Note that `cargo test` starts a test binary in its PACKAGE root
  (`crates/<name>`), not the repository root, so a fixture that needs a
  repository-level directory such as `docs/` to exist in the cwd builds a
  scratch tree and uses the `_from` variant. `set_var` under an `ENV_LOCK`
  mutex remains acceptable only for a variable no concurrent test's child
  could misread (`HOME` in the checklist-deny tests, the updater's
  `SS_MAGIC_UPDATED` / `SS_MAGIC_NO_UPDATE` in `update/apply`'s tests). CI
  (`.github/workflows/ci.yml`) runs `cargo test --workspace --locked` on Linux
  and macOS for every PR commit and every push to `main`, and gates cargo-dist
  releases as the `custom-ci` job registered under `local-artifacts-jobs` (see
  `dist-workspace.toml` and the release-gate rule below). As part of that gate
  it also refuses a release tag matching neither `^v[0-9]+\.[0-9]+\.[0-9]+$`
  nor `^ss-magic-plugin-v[0-9]+\.[0-9]+\.[0-9]+$`, since a prefixed CLI tag
  such as `ss-magic-v0.11.1` would publish a release the updater's anchored
  filter and every installed binary ignore.
- **`cargo test --workspace` is not the whole suite.** Seven commands cover
  ground `cargo test` cannot reach; each runs locally, and CI runs all seven:
  - `python3 scripts/build-plugin-zip.py --selftest` – the builder's own
    reproducibility and refusal tests.
  - `python3 scripts/build-plugin-zip.py --check` – prints EIGHT assertion
    lines: `R101 marketplace sha256 key` (the marketplace entry carries a
    well-formed `sha256` key, without which it installs unpinned),
    `R95 version surfaces (ss-magic)`, `R95 version surfaces
    (ss-magic-plugin)` (each release line's version surfaces agree, the
    README installer pin compared `<=` rather than `==`),
    `distinct release lines`, `hooks spawn through the shim` (the hooks-shim
    manifest guard), `workspace shape` (no `self_update`/`inquire`/`ratatui`
    in the plugin or core manifest, plus `publish = false` on core),
    `release gate blocks publishing` (the generated `release.yml`'s `host`
    job waits for and checks the test gate, see the release-gate rule below)
    and `R96 committed digest pin` (the committed digest matches the
    reproducibly built zip).
  - `python3 scripts/build-plugin-zip.py --check-bump <REF>` – the R98
    version-bump check: it rebuilds the `plugin/` tree as of `<REF>` and fails
    when the zip's digest changed but `plugin.json`'s version did not, or when
    the version moved backwards. Run it before a PR that touches `plugin/`,
    with `<REF>` the newest release tag of either shape reachable from `HEAD`
    (CI picks it by tag creation date, skipping a tag on `HEAD` itself).
  - `cargo tree --locked -p ss-magic-plugin -i <crate>` for each of
    `self_update`, `inquire` and `ratatui`, which must report no match – a
    build proves a dependency PRESENT and can never prove one ABSENT, so this
    is the only evidence for that requirement.
  - `/bin/bash scripts/test-bootstrap.sh` – the bootstrap's failure paths
    (offline, corrupted download, hostile pin, an archive without the binary,
    a staged binary whose `--version` differs from the pin, unwritable data
    dir, unsupported platform, concurrent sessions), each asserting exit 0,
    empty stdout, and an untouched pre-existing binary; PLUS the hook shim's
    own inertness contract, which drives `plugin/hooks/run-hook.sh` directly
    and asserts exit 0 with both streams empty and the binary un-invoked when
    it cannot resolve one; PLUS the `bin/ss-magic-plugin` wrapper (resolving
    through the handoff file, a removed handoff, a directory or a +x
    non-loadable file where the binary should be, no binary at all); PLUS the
    `seed-config` call sites (R3a, the bootstrap's one-time seeding of the
    `plugin` block: a second repository on an already-provisioned machine IS
    seeded, a failed install or upgrade seeds nothing); PLUS the one-time
    install disclosure on stderr (AE67: the first install names the binary,
    version, release and hooks, a later one is silent); PLUS a manifest
    invariant asserted over EVERY `hooks.json` entry rather than the bootstrap
    group alone. Written for bash 3.2, so no associative arrays, no `mapfile`,
    no `${var^^}`; its `passed`/`failed` counters, the `pass`/`fail` recorders
    and every `assert_*` helper live in `scripts/lib/test-harness.sh`, sourced
    by all three shell suites (this one, `test-mark-latest.sh` and
    `check-docs.sh --selftest`) so they cannot drift.
  - `/bin/bash scripts/test-mark-latest.sh` – the latest-mark selection behind
    the post-announce job, against a fake `gh` over a fixture release list
    (AE1, holding every trap in creation order): the plugin tag, a draft, a
    prerelease and a wrongly-prefixed CLI tag are never chosen, `v0.11.10`
    beats `v0.11.3` numerically, a `v*` tag never invokes `gh`, a dry run
    edits nothing, and a mark that did not take is exit 1 – also bash 3.2,
    where a `case` arm cannot sit inside `$( … )` and a `read` loop must
    handle an unterminated last line or drop the newest release when it is
    listed last.
  - `/bin/bash scripts/check-docs.sh` (and `--selftest`) – the documentation
    guards, seven `ok`/`FAIL` lines: `retired subcommand spelling` (no
    current-state document, `.claude/rules/` included, spells the old
    `ss-magic` + `plugin` + verb form), `skills name no CLAUDE_PLUGIN_DATA`,
    `README pins the installer` (no `releases/latest/download/` URL),
    `BUGBOT is self-contained` (no Markdown link, no individual rule-file
    path), `relative links resolve`, `rule frontmatter` (absent, or a
    well-formed `paths:` list) and `always-loaded budget` (`CLAUDE.md` plus
    every rule file without `paths:`, at most 50,000 bytes). A check also
    fails when a tool it runs writes to stderr; `docs/plans/` and
    `docs/brainstorms/` are out of every guard's scope.

  Where CI runs them: the `plugin` job (Linux, part of the release gate) runs
  all seven, plus two CI-only steps – the release-tag-shape refusal above (on
  a tag build only) and a build of the exact asset cargo-dist will publish
  (the zip filename the plugin crate's `extra-artifacts` entry declares, with
  STORED entries only, `.claude-plugin/plugin.json` at the zip root and a
  digest equal to the committed pin). The `test` job runs `cargo test` on
  Linux and macOS, and its macOS leg re-runs `test-bootstrap.sh`,
  `test-mark-latest.sh` and `check-docs.sh` (with `--selftest`) under
  `/bin/bash` 3.2 itself.
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
  cached copy, which is what `--check-bump` (above) catches.
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
  does not name that tag so a token that cannot edit shows up red. Its
  environment interface: `MARK_LATEST_TAG` names the announced tag (falling
  back to `GITHUB_REF_NAME`), `MARK_LATEST_REPO` the `OWNER/REPO` (default
  `ViktorStiskala/superset-magic`), and `MARK_LATEST_DRY_RUN=1` prints the
  chosen tag instead of editing; it exits 2 when no tag is given, 1 when the
  mark did not take, and 0 with a note when no `v*` release exists. The
  workflow is also `workflow_dispatch`-able with the tag as input (the
  documented fallback); it asks for `contents: write`, which `release.yml`
  grants at workflow level. `scripts/test-mark-latest.sh` drives the script
  against a fake `gh` (see the checks list above); `dist generate --check`
  must stay green after any `dist-workspace.toml` change, and the
  `release.yml` is REGENERATED, never hand-edited.
- **The release test gate must block the PUBLISH, not only the builds.**
  cargo-dist's `host` job (the one that runs `gh release create`) publishes
  when `plan` succeeded and each build job succeeded OR WAS SKIPPED. A job
  upstream of the builds whose result `host` never checks can therefore fail,
  skip every build, and still let `host` publish a release with no assets.
  That is exactly what a `plan-jobs` entry is: upstream of
  `build-local-artifacts`, absent from `host`'s `needs` and `if`. The gate is
  therefore `local-artifacts-jobs = ["./ci"]`, never `plan-jobs`: cargo-dist
  adds such a job to `host`'s `needs` AND checks its result in `host`'s `if`,
  so a red or cancelled suite skips `host`, `announce` and
  `custom-mark-latest`. The builds run alongside the tests, so a red gate
  still spends build minutes and leaves attested archives in Actions storage,
  but uploads nothing. `ci.yml` declares the optional `plan` input cargo-dist
  passes to such a job (GitHub rejects an undeclared one). cargo-dist 0.33
  keeps the same `host` condition, so an upgrade is no substitute.
  `--check`'s `release gate blocks publishing` asserts the shape on the
  GENERATED `release.yml`: `custom-ci` must call `ci.yml`, `host` must need it
  and check its result, and every job `build-local-artifacts` waits on must
  appear as `needs.<job>.result` in `host`'s `if`, so any future `plan-jobs`
  entry fails there rather than in a release. An empty release cannot be
  repaired: release immutability forbids adding assets, GitHub never lets a
  deleted immutable release's tag be reused, and the tag ruleset forbids
  moving or deleting the tag, so the version is lost for good. `v0.11.1` and
  `ss-magic-plugin-v1.0.0` are two such releases, holding only
  `dist-manifest.json`; they are marked as pre-releases (so neither updater
  selects them) and their titles end in "(broken – no assets, do not use)".
  Independently of the gate, push a release tag only after CI on `main` is
  green for that exact commit: a red release still burns its version number,
  since nothing is published but the tag is protected and cannot move. See
  [docs/solutions/logic-errors/cargo-dist-plan-job-failure-publishes-empty-release.md](../../docs/solutions/logic-errors/cargo-dist-plan-job-failure-publishes-empty-release.md).
- Version bumps follow the GROUP, never the repository. Bump
  `crates/ss-magic/Cargo.toml` and the matching `ss-magic` entry in `Cargo.lock`
  on any change that alters CLI behavior – a fix, a new/changed command or flag,
  or different output; that binary self-updates from GitHub Releases keyed on
  version (see `.claude/rules/build-release.md`), so a stale version means users
  never receive the change. Bump the plugin group (above) on any change under
  `plugin/` or in `crates/ss-magic-plugin/`. `crates/ss-magic-core/Cargo.toml`
  is not a surface at all. The two groups' versions must never be EQUAL –
  `--check`'s `distinct release lines` assertion refuses that, because a bare
  `vX.Y.Z` tag would then announce both packages. Bug fixes bump patch;
  new/changed user-visible behavior bumps minor (pre-1.0 on the CLI line; the
  plugin line follows ordinary semver from 1.0 – its first release with assets
  is `1.0.1`, since `ss-magic-plugin-v1.0.0` was published empty, see the
  release-gate rule above).
- After every implementation change, update the docs to match the current
  state before the change is considered done; they describe the code as it is
  now, not as it was. Each fact has one home:
  - a new or changed module, or a module's changed behavior: the owning
    architecture map – `.claude/rules/architecture-core.md`,
    `architecture-cli.md` or `architecture-plugin.md` by crate, and
    `plugin-assets.md` for the non-Rust assets (`plugin/`, the scripts, CI and
    the release config);
  - a convention: this file, `.claude/rules/conventions.md`;
  - a build or release fact: `.claude/rules/build-release.md`;
  - a hard rule: `.claude/rules/hard-rules.md`;
  - the crate layout or a cross-crate invariant: the index, `CLAUDE.md`;
  - a new or changed command, flag or user-visible behavior: `README.md`
    (command list + relevant prose);
  - a build, test or release-workflow fact: `CONTRIBUTING.md` as well, whose
    "Code layout" stays the contributor overview pointing at the maps rather
    than a third copy of them;
  - any convention or behavior BUGBOT mirrors: `.cursor/BUGBOT.md` (next
    bullet).

  `.claude/settings.json` and `.claude/rules/` are committed;
  `.claude/settings.local.json` (per-machine) and `.claude/skills/` are not,
  and the repository's own `.gitignore` covers neither, so keep them out of a
  commit by hand.
- `.cursor/BUGBOT.md` holds the Cursor Bugbot review rules. It must stay
  **self-contained**: it cannot reference `CLAUDE.md`, the rule files,
  `docs/solutions/`, `.cursor/rules` or any skill – restate the relevant
  conventions inline instead. It contains no Markdown link and names no
  individual rule file path (a `.claude/rules/<name>.md`), though it may name
  the `.claude/rules/` directory as the subject of a review rule;
  `check-docs.sh`'s `BUGBOT is self-contained` check enforces both. Keep it
  **synchronised on every change**: whenever a convention here or a behavior
  in the code changes, update `.cursor/BUGBOT.md` in the same change so its
  rules never describe stale conventions.
