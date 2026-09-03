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

Rust, edition 2021, repo `ViktorStiskala/superset-magic`. The repository is a
Cargo **workspace with three members**: the root `Cargo.toml` is a virtual
manifest (no `[package]`) that owns `[workspace.package]` and both build
profiles (`[profile.release]` with `opt-level = "z"` – a measured size/speed
decision that applies to every member, flag a per-crate override – and
`[profile.dist]` inheriting it), and the crates live under `crates/`:

- `crates/ss-magic-core` – library `ss-magic-core`, crate name
  `ss_magic_core`, `publish = false` plus `[package.metadata.dist] dist =
  false`, version `0.1.0`, never a release surface and never tagged.
- `crates/ss-magic` – binary `ss-magic`, the interactive sync CLI. Releases on
  bare `vX.Y.Z` tags.
- `crates/ss-magic-plugin` – binary `ss-magic-plugin`, the Claude Code plugin's
  hook runtime and verb tree. Releases on `ss-magic-plugin-vX.Y.Z` tags.

**The plugin's inability to self-update or open a TUI is STRUCTURAL.** Neither
`crates/ss-magic-plugin/Cargo.toml` nor `crates/ss-magic-core/Cargo.toml` may
declare `self_update`, `inquire` or `ratatui`, in any dependency table, under
any rename – core counts because the plugin links it, so a dependency added
there reaches the plugin transitively and the guarantee is gone without the
plugin's own manifest changing a line. Flag any of the three appearing in
either manifest. A `cargo build` proves a dependency is PRESENT and can say
nothing about one being ABSENT, so the absence is asserted mechanically by
`python3 scripts/build-plugin-zip.py --check` and by
`cargo tree --locked -p ss-magic-plugin -i <crate>` in CI – flag a change that
drops either assertion.

**`ss-magic` has NO `plugin` subcommand.** The token was deleted outright when
the plugin became its own binary: no alias, no redirect, no deprecation shim.
`ss-magic plugin` now takes the ordinary unknown-subcommand path. Flag any
reintroduction of the token in `cli.rs`, and flag any prose or model-facing
string spelling a plugin verb as a subcommand of `ss-magic`.

Module paths below are written relative to a crate's `src/`. Core owns `git/`,
`hashing.rs`, `style.rs` (palette and color decision only), the pure half of
`sync/` (`mod.rs`, `pattern.rs`, `repo_scan.rs`, `apply.rs`),
`superset_files.rs`, `reponame.rs`, `state_tree.rs`, `release.rs` and
`testutil.rs`. The CLI re-exports core's modules under the names it used before
the split (`crate::git`, `crate::hashing`, `crate::tui::style`,
`crate::sync::apply`, `crate::workspace::superset_files`); the plugin crate
does the same for `crate::git` and `crate::hashing` only, and reaches the rest
by their real paths – `ss_magic_core::style` (never `crate::tui::style`, which
does not exist there) and `ss_magic_core::reponame::repo_name_stem` (never
`crate::pack`). Flag a plugin-crate path that reaches into the CLI's module
names; the two binaries share core, not each other.

Key dependencies: `anyhow` (errors), `inquire` (interactive prompts, CLI only),
`ratatui` + `crossterm` (the full-screen bidirectional sync merge cockpit, CLI
only; `crossterm` also backs `inquire`), `similar` (line/word diffing for the
diff model and merge assembly), `globset` + `walkdir` (pattern matching),
`serde`/`serde_json` (config I/O), `tempfile` (atomic staging), `tar` + `bzip2`
(pack archives), `self_update` (CLI only) + `ureq` + `fd-lock` (self-update and
the plugin's advisory locks), `directories` (the plugin's machine-level data
dir), `supports-color` (palette). No `clap` (the arg parsers are hand-rolled)
and no `git2` (every git/gh COMMAND is shelled out; the one filesystem-only
exception is core's `git/discover.rs`, below). Hashing is in-crate (core's
`hashing.rs`: FNV-1a plus a hand-rolled SHA-256) – flag the addition of a
hashing crate for these uses. Release binaries are built by cargo-dist
(`dist-workspace.toml` plus per-package `[package.metadata.dist]` tables); only
`ss-magic` self-updates from GitHub Releases. Tests use `tempfile` +
shell-invoked `git init` / `git worktree add`.

The plugin also ships as a packaged tree under `plugin/`, pinned by SHA-256 in
`.claude-plugin/marketplace.json`. Two non-Rust pieces are part of the product,
not tooling: `scripts/build-plugin-zip.py` (python3; the reproducible zip
builder and the release assertions) and `plugin/hooks/bootstrap.sh` +
`plugin/hooks/run-hook.sh` + `plugin/bin/ss-magic-plugin` +
`plugin/lib/*.sh` + `scripts/test-bootstrap.sh` (bash 3.2 – macOS's version, so
no associative arrays, no `mapfile`, no `${var^^}`).

## No External Process Libraries

- **All `git` and `gh` COMMANDS shell out via `std::process::Command`** –
  there is NO `git2`/`libgit2`. Flag any addition of `git2`, `gix`, or another
  git-binding crate to `Cargo.toml`. The shared entry point is the `git_raw`
  helper in core's `git/mod.rs` (surfaces stderr verbatim); `git` and `git_optional`
  are thin one-liners on top. Flag new git/gh calls that spawn `Command`
  directly instead of routing through these helpers.
- **Core's `git/discover.rs` is the ONE filesystem-only reduction of git
  behavior, and it is bounded.** It answers exactly two read-only probes –
  `git rev-parse --show-toplevel` and `--git-common-dir` – by walking the
  filesystem, for the plugin's hook pipeline only; the CLI's own commands keep
  the subprocess probes. Flag: (1) any new caller outside the plugin crate's `hook/`;
  (2) anything in `discover.rs` that reads refs, the index, or writes –
  resolving `HEAD` to a branch, reading `packed-refs`, parsing more of the
  config than the format check (`core.bare`, `core.worktree`,
  `repositoryformatversion`, `[extensions]`) – that belongs in a subprocess;
  (3) any change that turns an `Undecided` into a `Found`, or alters what a
  `Found` returns, without a matching real-git scenario added to the
  equivalence matrix in core's `git/discover/tests.rs`. The invariant is that a
  fast answer is byte-equal to git's or there is none: widening the
  `Undecided` set is always safe, narrowing it never is without measurement.
- **A test that changes the process environment must not use `set_var` for a
  variable a concurrent test's `git` child could inherit** (`PATH`, `GIT_*`).
  The suite runs multithreaded and a lock around the setter does not stop
  other threads' children from inheriting the value. Flag such tests unless
  they run the assertion in a child process via
  `ss_magic_core::testutil::run_ignored_test_in_child`. `HOME` under `ENV_LOCK`
  in the checklist-deny tests is the tolerated exception (no concurrent child
  misreads it).
- **Both arg parsers are hand-rolled** – there is NO `clap` in either binary.
  The CLI's is `cli.rs`: `parse(&[String]) -> Parsed` selects the command from
  the first non-flag token; `Command` is `{ Bare, Sync { no_backup },
  ReverseSync { no_backup }, Pack, Update }`, and `Parsed` additionally carries
  `Init(Vec<String>)`, `Version`, `Help` and `Error(token)` – there is no
  `Plugin` variant any more. `sync`/`reverse-sync` read the `-n`/`--no-backup`
  flag via a full-argv scan (`has_no_backup`, position-independent – before OR
  after the subcommand), an intentional asymmetry with `-h`/`--help` (a terminal
  short-circuit recognized only BEFORE the subcommand). Flag any addition of
  `clap`/`structopt`/`argh`, command dispatch logic added outside `cli.rs`, or a
  "fix" that makes `has_no_backup` and the `--help` scan match each other.
- **The CLI's `--version` scan runs PAST a subcommand token, and must keep
  doing so.** `ss-magic sync --version` prints the version; without that, an
  unrecognized `--version` is skipped as an unknown flag and falls through to
  `Command::Bare`, which IS update-gated and opens the interactive menu –
  exactly wrong when a script shells out to identify the binary. The scan used
  to STOP at a `plugin` token so a `-V` belonging to a plugin verb was not
  swallowed; that token no longer exists, and re-adding either the token or the
  stop is a regression. Flag a change that stops the `--version` scan at the
  subcommand, or that reintroduces `plugin` handling in `cli.rs`.
- **The plugin binary's own parse is separate and deliberately stricter.**
  `crates/ss-magic-plugin/src/main.rs::parse` recognizes `-V`/`--version` and
  `-h`/`--help` only as the FIRST token, because every verb parses its own
  arguments and a whole-argv scan would swallow one of them. `-V`/`--version`
  must print `ss-magic-plugin <version>` on ONE line and exit 0, ahead of verb
  parsing: `hooks/bootstrap.sh` gates every install on
  `"$staged_bin" --version | head -1 | awk '{print $NF}'` equalling the pin, and
  `status` probes the same shape for drift. A flag that answered with usage text
  or a non-zero exit would make the bootstrap discard every download it made.
  Flag a change to that line's shape, to the version's position on it, or to the
  first-token-only rule.

## Architecture: Layering (pure logic vs interactive layer)

The codebase is deliberately layered so the pure logic is unit-testable in
isolation from the interactive TUI. Preserve this boundary.

- Pure/testable modules: `git/mod.rs` (probes + mutating primitives), `cli.rs`
  (arg parsing), `sync/pattern.rs` (glob syntax checks), `sync/apply.rs` (glob/copy
  engine), `workspace/superset_files.rs` (`.superset/` I/O), `sync/repo_scan.rs` (working-tree
  scan), `git/gitignore.rs` (`.gitignore` helpers), `sync/merge.rs` (the
  push/pull/merge decision model and per-hunk merge assembly), `tui/diffmodel.rs`
  (the diff-to-rows model powering the cockpit's diff pane), `hashing.rs`
  (FNV-1a + SHA-256), `reponame.rs` (the repo-name stem), `state_tree.rs`
  (the `.superset/.magic` constant and its gitignore rule; a core test pins the
  constant equal to the `sync::EXCLUDED_TREES` entry – flag a change to one
  side without the other), and effectively all of the plugin crate – its parse
  (the crate root `main.rs`), its state modules, its hook handlers, and the whole
  `checklist/` family (schema, canonical ordering, validator, renderer,
  verbs) are unit-tested with no terminal and no harness involved.
- Interactive/side-effecting: `tui/menu.rs`, `tui/ui.rs` (`inquire` wrappers),
  `tui/theme.rs` (installs the `inquire` render config from core's `style`
  color decision; `render_config(false)` must stay `RenderConfig::empty()`),
  the finishing-action prompts in `workspace/migrate.rs` /
  `sync/reverse_sync.rs`, `tui/cockpit.rs` (the full-screen reverse-sync merge
  cockpit — its event loop and terminal lifecycle are manual-smoke like the
  rest of this list, but its render path (`draw`) and key dispatch
  (`handle_key`) are unit-tested via `ratatui::backend::TestBackend`, so a
  regression there IS expected to be caught by `cargo test`).
- The CLI's `main.rs` composes: `cli::parse` → `tui::style::init` +
  `tui::theme::install` → [auto-update gate for
  `Bare`/`Sync`/`ReverseSync`/`Pack`] → `dispatch`. `Parsed::Version` answers
  before any dispatch. The plugin no longer appears here at all – it is a
  different binary, which is a stronger separation than the sibling-arm
  arrangement it replaced.
- The plugin's `main.rs` composes: `parse` → `style::init_no_color()` for a
  `hook` invocation or `style::init()` otherwise → dispatch to a hook handler or
  a human verb. It installs no `inquire` theme and constructs no menu, and
  cannot: the crate links neither library.

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
- **`EXCLUDED_TREES` is enforced at every point of FINAL enumeration, never on
  the match list alone.** Core's `sync/mod.rs` owns the ONE rule: `EXCLUDED_TREES`
  = `.superset/backups` (pre-write backups, i.e. recovered secrets),
  `.superset/.magic` (the plugin's machine-local state), `.scratchpad`, `.git`;
  `under_excluded_tree(rel)` answers "is this rel one of them, or inside one".
  It must be applied in `apply::walk_source`, `apply::copy_dir_recursive(root,
  src, dst)`, reverse sync's candidate computation, AND
  `pack::append_dir_excluding_trees` – every walk that re-reads the live
  filesystem after the match set is decided. Filtering the flat match list is
  NOT sufficient: a directory match that is an ANCESTOR of an excluded tree (a
  bare `.superset` pattern, a broad `**`) re-admits the whole subtree through
  the walk, and ONE such match can sit above SEVERAL excluded trees at once,
  since `.superset` is the ancestor of both `backups` and `.magic`. Matching is
  per COMPONENT (`starts_with_components`), never a string prefix or a bare
  name, so `.superset/.magicked/` stays includable and – load-bearing –
  `.superset` ITSELF is never excluded; widening the rule would drop
  `config.json`, `magic.sh` and `magic.json` out of sync and pack. Flag a new
  enumeration path that omits the filter, a filter applied only to `rels`, a
  widened entry (`.superset` alone, or a bare-name match), or a comment
  asserting "X is never included" without a guard at the walk layer. Distinct
  from `DEFAULT_EXCLUDES` (`node_modules`/`.venv`), which drops a NAME at any
  depth.

Flag any second, divergent glob/exclude implementation — expansion must go
through `sync/apply.rs` (`run`/`match_paths`) and syntax checks through
`pattern::check_syntax`.

## Security: Secret Handling and Gitignore Safety

The files this tool moves are secrets. The main-checkout copy must never become
committable and must never leak.

- **Sync reconciles configured files between a worktree and the main checkout in
  BOTH directions.** The interactive worktree menu is ONE "Sync" entry that opens
  the full-screen cockpit (`sync/reverse_sync.rs::run` → `tui/cockpit.rs`), where
  the user sets each file's direction – push (worktree → main), pull (main →
  worktree), merge (both), or delete – with NOTHING pre-selected (every file
  starts `Undecided`, regardless of direction). Two direct non-interactive
  subcommands sit alongside it: `ss-magic sync` (main → worktree, `run_sync_flow`)
  and `ss-magic reverse-sync` (worktree → main, git-untracked candidates only,
  `run_bulk`); both take a pre-overwrite backup of the losing bytes unless
  `-n`/`--no-backup`.
- **The gitignore-in-main step fires ONLY for a git-UNTRACKED worktree source,
  determined POSITIVELY.** Only a PUSH or MERGE writes worktree bytes into main,
  and only an untracked (secret) source may add a `.gitignore` rule there – a
  TRACKED, already-committed file must NEVER gain one. The gate is
  `Baseline::source_untracked`, derived FAIL-CLOSED as `!tracked.contains(rel)`
  where `tracked` comes from `git::tracked_files` (`git ls-files --cached`): a
  path NOT positively known-tracked (a non-UTF-8 or oddly-normalized name, an
  unenumerable path) defaults to untracked = secret. `apply_decision`'s
  Push/Merge arms call `ensure_gitignored_in_main` iff `source_untracked`; it
  copies the covering `.gitignore` rule (verified via `git check-ignore -v`,
  negations excluded) or an anchored literal into main, then STRICTLY re-verifies
  with `git::is_ignored` and bails (writing NOTHING) if the path is still not
  ignored. Flag: a Push/Merge that appends a `.gitignore` rule for a tracked
  file; a Push/Merge that writes an untracked secret into main WITHOUT ensuring it
  is ignored there (dropping `ensure_gitignored_in_main` or its strict re-verify
  bail); OR deriving untracked-ness by ABSENCE from an untracked list (fail-OPEN –
  a name missing from a `git ls-files --others` set is not proof it is tracked)
  instead of positive tracked determination. Pull and Delete never touch main's
  `.gitignore`.
- **The reconcile set unions patterns across BOTH roots and classifies 4-way.**
  `compute_reconcile_set` expands the overlaid patterns against the worktree AND
  the main root (a main-only file is invisible to the worktree walk, and
  vice-versa), unions and de-dupes the matches, then classifies each rel 4-way via
  `classify`: `Differs` (both sides, different bytes), `WorktreeOnly`, `MainOnly`,
  or `Identical` (byte-equal, OR both absent – the walk↔classify race).
  `Identical` rels are dropped. DIRECTORY matches are dropped (reverse sync copies
  single files; a dir would `EISDIR` in `classify`/the cockpit), and any rel in an
  excluded tree (`sync::under_excluded_tree`) is dropped so neither a backed-up
  secret under `.superset/backups/` nor the plugin's `.superset/.magic/` state is
  ever re-offered. Flag a reconcile that scans only one root,
  surfaces a directory match, or re-offers a backup copy.
- **The review baseline pins the reviewed-absent side to None.** Before the
  cockpit opens, `review_baseline` captures each file's `(worktree, main)`
  metadata COHERENTLY with the status the user reviews: a `WorktreeOnly` file's
  main side is pinned `None`, and a `MainOnly` file's worktree side is pinned
  `None` (symmetric). So a copy that materializes on the pinned side during the
  review→apply window is seen as `Guard::Changed` and SKIPPED, never overwritten
  or deleted without having been shown in the confirm. Flag a baseline capture
  that stats the disk for a side the review classified as absent.
- **Baseline capture must never abort the whole reconcile for one unreadable
  file.** `review_baseline` is infallible: a read side that fails to `stat` (or,
  on a mtime-less filesystem, to hash) degrades to `None` via `baseline_side`
  instead of propagating the error, so one permission/I/O error on a single
  candidate does not tear down the entire interactive `run` (or `run_bulk`)
  session — matching the cockpit's `classify`/`load_entry`, which already degrade
  such reads to `FileDiff::Unreadable`. Folding to `None` is fail-closed: an
  unreadable-then-present side reads as `baseline None` vs a present target →
  `Guard::Changed` → SKIP (never a silent overwrite); only a genuinely-absent
  target is written. Flag any reintroduction of a `?`/propagating read in the
  baseline-capture loops that could abort the reconcile, or a degraded path that
  overwrites a side whose baseline could not be read.
- **A MainOnly pull is a non-destructive create; a MainOnly delete IS
  destructive.** For a main-only file a PULL creates the worktree copy (no
  worktree bytes are lost), so it MUST be excluded from the destructive
  batched-confirm list (`destructive_overwrites`: `Decision::Pull if f.status !=
  DiffStatus::MainOnly`). A DELETE removes main's copy and IS destructive – listed
  and badged `delete (main copy)` (`delete_target`), backed up first. Push and
  merge are no-ops for a MainOnly file (`set_push` gates `p` off MainOnly;
  `try_open_merge` only opens for a differing text file). Flag a MainOnly pull
  that appears in the destructive confirm, a MainOnly delete that is unlisted or
  not backed up, or a Push/Merge that becomes reachable for a main-only file.
- **Backups live under the root being OVERWRITTEN, gitignored via ONE helper.**
  Each direction writes its pre-overwrite backups under the `.superset/backups/`
  of the root it overwrites: the interactive cockpit → the worktree root, the
  direct `reverse-sync` → the main root, the forward `sync` → the worktree (cwd)
  root (`backups_root_for`). That dir is gitignored at the closest `.gitignore`
  via the single `ensure_backups_ignored` helper, which wraps
  `gitignore::ensure_path_ignored(root, root, ".superset/backups", PathKind::Dir)`
  (a `Dir` is queried/written with a trailing slash so a `.superset/backups/`
  rule matches before the dir exists on disk). The SAME helper is called eagerly
  by init/migrate (`ensure_bootstrap_gitignores`) so a fresh `ss-magic init`
  gitignores the backups tree up front, exactly like `magic.local.json`. Flag a
  backup written under the wrong root, a backups dir gitignored by a hand-rolled
  path instead of `ensure_backups_ignored`/`ensure_path_ignored`, or an init/
  migrate path that stops gitignoring the backups tree.
- **`--no-backup` skips ONLY the backup copy – never the secret gitignore or the
  TOCTOU guard.** `ApplyContext.backup == false` (from `-n`/`--no-backup` on a
  direct path) no-ops the pre-overwrite backup copy, but `apply_decision` still
  runs the `Guard::Changed` concurrent-edit skip AND still runs
  `ensure_gitignored_in_main` before any secret bytes land in main. Flag a
  `--no-backup` path that also skips the gitignore-in-main gate or the
  concurrent-edit guard.
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
- **Pack must never archive itself or the whole tree.** `pack_core` drops
  every root-level match shaped `ss-magic-*.tar.bz2` (covering the current
  derived name from `pack::archive_file_name`, the legacy fixed
  `ss-magic-files.tar.bz2`, and archives left under a previous derived name
  after an origin change) and any match that resolves to the repo root itself
  (a `.` pattern) before archiving. Deeper `ss-magic-*.tar.bz2` files are user
  data and stay packable. Flag removal or narrowing of any of these guards.
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
  expansion), `write_archive` must discard the temp file and leave any
  existing archive (the derived `ss-magic-<repo>.tar.bz2`) untouched —
  never rename an empty tarball over a
  prior good backup, and never report "Packed 0 files" as success (`main.rs`
  suppresses `PackEvent::Done` at zero and prints "No packable files remained"
  instead). `PackEvent::Done.count` is the size of `write_archive`'s `added`
  set: UNIQUE FILE PATHS actually written, not tar entries – archived
  directories are not counted and two overlapping patterns naming the same file
  count once. Flag a
  pack path that persists the temp archive when the added count is zero, or a
  change that makes `count` a raw entry tally while the message still says
  "entries".
- **Pack must never archive anything in an excluded tree.** A recovered secret
  copy under `.superset/backups/`, or the plugin's `.superset/.magic/` state,
  must never enter an archive. Two guards enforce this, and BOTH are needed:
  `pack_core` drops every LEAF match in an excluded tree from `rels`
  (`sync::under_excluded_tree` in the `rels.retain`), AND `write_archive` prunes
  those subtrees from any ANCESTOR-directory match (a bare `.superset` pattern,
  or a broad `**`/`.` that matches the `.superset` component) via
  `append_dir_excluding_trees`'s guarded `filter_entry` walk rather than a blind
  `append_dir_all`. `under_excluded_tree` matches a tree's full component path,
  so the flat retain filter CANNOT catch an ancestor dir – the guarded directory
  walk is required, and a single `.superset` match must prune BOTH `backups` and
  `.magic`. Flag removal of either guard, or a new archive path that reaches
  `append_dir_all`-style recursion for a directory match without pruning every
  excluded subtree.
- Overwrite safety: sync reconciles files through the full-screen
  merge cockpit (`tui/cockpit.rs`), never writing on any keypress. NOTHING is
  pre-selected (every file starts `Undecided`, in either direction), applying
  is gated by ONE batched confirm keyed **Enter = apply / Esc = back** (the old
  `y`/`n` bindings and the "default: No" idle path were removed – every bound key
  is now an explicit action; `render_confirm` prompt + the `Mode::Confirm` arm of
  `handle_key`), which lists every existing-target overwrite
  and delete, and every destructive write or unlink is preceded by a
  timestamped
  pre-write backup of the losing bytes under a gitignored `.superset/backups/`
  (`reverse_sync::apply_decision`), with a review-time baseline re-check —
  per-file `(worktree, main)` metadata captured (`review_baseline`) BEFORE the
  cockpit
  opens and re-compared at apply — that skips a file created, edited, or deleted
  since review (a non-`NotFound` stat error counts as changed, never as
  "missing"). The unchanged-check needs a REAL change signal: length + mtime
  when the filesystem reports mtimes, else a content hash captured at
  snapshot time — flag a guard that trusts a bare length (a same-length edit
  must never pass as unchanged). The baseline must be COHERENT with the
  reviewed status, not with the disk at capture time: a worktree-only
  candidate's main-side baseline is pinned absent, so a main copy that
  appears between classification and capture is skipped at apply — flag a
  baseline capture that stats the disk for a side the review classified as
  missing. The cockpit refuses to launch without an interactive
  TTY and writes nothing then, and `Esc` at the top-level file list cancels the
  whole cockpit (`CockpitOutcome::Cancel`), leaving both the worktree and main
  untouched. Flag a sync path that overwrites or deletes an
  existing file without a backup, applies an `Undecided` file, skips the batched
  confirm, reverts the confirm to a `y`/`n` or default-No prompt, or falls
  through to writing files when there is no TTY.
- **Backup layout + retention.** Backup batches are one UTC
  `YYYYmmdd-HHMMSS`-named directory per apply, with per-side `worktree/` and
  `main/` namespaces inside (`merge::backup_rel_path(ts, side, rel)`), so the
  same rel backed up from both sides never collides. After each apply the 10
  newest batch dirs are kept and older ones pruned (`prune_old_backups`) —
  pruning is best-effort (a failure warns, never fails the sync) and must only
  ever remove directories whose names match the batch shapes the tool itself
  wrote (`YYYYmmdd-HHMMSS` or legacy all-digit epoch), never foreign entries.
  An older pre-release merge layout wrote `local/<epoch>/` and `main/<epoch>/`
  at the TOP level of the backups root; those children are folded into their
  epoch's batch for the same keep budget, and a `local`/`main` side dir is
  removed only when this run pruned from it and it ended up empty — a foreign
  dir merely named `local`/`main` (or its non-batch children) is never
  touched.
  The batch written by the CURRENT run is protected by name and never pruned
  — a backward clock jump could otherwise name it "older" than the keep set
  and delete the backups whose recovery paths were just printed.
  Flag a retention change that deletes non-batch-named entries, prunes before
  the current batch's backups are written, drops the current-batch
  protection, or turns a pruning error into a sync failure.
- **Delete decisions remove every EXISTING side, backup-first.** `d` records
  `Decision::Delete`; apply unlinks the file from main and the worktree
  (whichever exist), each side backed up first and TOCTOU-guarded like an
  overwrite, main unlinked before the worktree so a failure leaves the
  worktree copy (and the next run's candidate) intact. The batched confirm and
  the file's badge name EXACTLY the same sides via one `delete_target`
  (`WorktreeOnly` → "delete (worktree copy)", `MainOnly` → "delete (main copy)",
  a two-sided file → "delete (worktree + main)"), so the confirm can never
  under-state what a delete removes. Deletes are always in the batched-confirm
  list. No gitignore step runs (nothing is written into main). Flag a delete
  path that unlinks without a backup, skips the baseline re-check, removes the
  worktree copy before main, or lets the badge and confirm name different sides.
- **Diff/merge inputs are EOL-normalized; raw copies are not.** Text
  candidates are normalized at load (`diffmodel::normalize_eol`: CRLF → LF,
  a trailing lone CR treated as an EOL — never given a synthesized `\n`
  after it — and a trailing newline ensured) so diff hunks and merge
  assembly reflect content
  only; sides equal after normalization render an explanatory "line endings
  only" notice instead of an empty diff. Push/pull must keep copying the RAW
  on-disk bytes, and byte-level classification (`classify`) stays byte-exact.
  Flag a change that diffs un-normalized text, normalizes the push/pull copy
  path, or hides an EOL-only-differing candidate entirely.
- **A change past the pane's right edge must never be silently invisible.**
  Diff lines wider than the visible content area are horizontally scrollable
  (`←`/`→`; the offset is clamped to the longest content line and reset when
  the focus moves to another file) with the line-number gutter held FIXED,
  and the pane title flags the state ("lines continue →" when clipped,
  "→ col N" while scrolled). The batched-confirm overlay is content-sized and
  truncates an over-long overwrite list with an explicit "… and N more"
  marker while keeping the count and the Enter/Esc prompt visible. Flag a
  diff-pane or overlay change that clips content with
  no indicator, scrolls the gutter away with the content, or leaves a stale
  horizontal offset when switching files.
- **The file-list pane WRAPS long paths, never clips them.** Each row renders
  badge + status tag (line 1), then the repo-relative path hard-wrapped across
  one or more lines (`wrap_hard` at `file_list_content_width(area)` = pane width −
  border − reserved `highlight_symbol`), then the mtime hint – because ratatui's
  `List` clips rather than wraps, a deeply-nested path would otherwise have its
  tail silently cut. Flag a revert to a single clipped path line, or a
  `file_list_item` that drops the `content_width` wrap.
- **Split and unified diff colors are MIRRORED (local green / main red in BOTH
  views).** The mental model is main = base, local = working copy: local-only or a
  change's local text is GREEN, main-only or a change's main text is RED, in the
  side-by-side view (`side_columns`: `RowTag::Delete|Replace` → green left,
  `RowTag::Insert|Replace` → red right) AND the unified view. The unified view
  achieves the conventional `-` red / `+` green by calling `diffmodel::unified(main,
  local, CONTEXT)` – the ONLY caller with that swapped `(old=main, new=local)`
  argument order – with `row.new_no`/`row.old_no` bound to `local_no`/`main_no` and
  printed local-first so the gutter's visible column order is unchanged; only the
  sign/color meaning flips. Flag recoloring ONE view without the other (they must
  stay mirrored), or changing `render_unified`'s `unified(main, local)` arg order
  WITHOUT keeping the `local_no`/`main_no` rename (which would silently reorder the
  gutter numbers). `diff_line_count` deliberately keeps `unified(local, main)` (row
  count is symmetric under the swap) – do not "fix" it to match `render_unified`.
- **The new-file / main-only "will be created" notice renders in a FIXED header
  row.** `render_created` draws its notice (green italic "new file – will be
  created in main" for `FileDiff::New`, cyan italic "main only – …" for
  `FileDiff::MainOnly`) in a fixed `Length(1)` header row, NEVER inside the
  scrolled `Paragraph` body – so it can never scroll away and the body's numbered
  `+` content (behind the fixed `NEW_GUTTER`) starts below it. The header is
  rendered on BOTH arms, including the content-absent (`None`, binary/oversized)
  arm. Flag moving the notice back into the scrollable body, dropping the header on
  the `None` arm, or scrolling the `NEW_GUTTER` line numbers with the content.
- **The cockpit's terminal is always restored, including on panic.**
  `run_cockpit` installs a panic hook and constructs a `TerminalGuard`
  (`Drop` disables raw mode / leaves the alternate screen) immediately after
  `enable_raw_mode()`, BEFORE entering the alternate screen — so a panic or
  an early `?` failure during setup can never strand the developer's terminal
  in raw mode. Flag a change that moves terminal setup/teardown outside the
  guard/panic-hook path, or that enters the alternate screen before the guard
  exists.
- **A diff or merge is never built from fabricated content, and one unreadable
  file never aborts the whole reconcile.** If EITHER side's copy of a candidate
  fails to read for a reason OTHER than "does not exist" (permissions, I/O), the
  cockpit surfaces `FileDiff::Unreadable { note, side }` with the real error and
  disables interactive merge for that file — it must NEVER substitute an empty
  buffer and diff/merge against that, and must NEVER propagate the error out of
  `classify`/`build_two_sided`/`build_new`/`build_main_only` (that would abort
  `compute_reconcile_set` or `App::new` for the whole session). `side`
  (`UnreadableSide::Worktree`/`Main`) is load-bearing: the direction gates must
  stay side-aware — `set_push` disabled only when the WORKTREE side is unreadable
  (or the file is main-only), `set_pull` disabled only when the MAIN side is
  unreadable (or the file is worktree-only). Flag a change that treats a
  non-missing read error as empty content, that propagates it instead of
  degrading to `Unreadable`, or that gates a direction on `Unreadable` without
  checking `side` (e.g. blocking pull for a worktree-unreadable file whose main
  copy is perfectly readable).
- Interactive merge: pressing `m` on a DIFFERING TEXT file opens a per-hunk
  overlay (`Mode::Merge`) that assembles bytes with `merge::merge_segments` +
  `merge::assemble` and, on `Enter`, records `Decision::Merge(assembled)`; `Esc`
  leaves the file's decision unchanged. `m` MUST be a no-op (never entering the
  overlay) for binary / oversized / worktree-only / main-only files — interactive
  merge is only available for a two-sided differing text file. A `Merge` decision
  overwrites BOTH the worktree and main,
  so the batched confirm must list it as a destructive write and `apply_decision`
  must back up whichever side exists before writing (distinct per-side
  `worktree/` + `main/` backup namespaces inside the batch dir) and run
  `ensure_gitignored_in_main` before the main-side write — gated on
  `source_untracked` exactly like Push (a tracked merge target must NOT gain a
  `.gitignore` rule; an untracked one must).
  Flag an `m` handler that opens the overlay for a non-text/new file, a merge
  apply that overwrites either side without a backup, a main-side merge write that
  skips the gitignore-safety step for an untracked source, or one that appends a
  rule for a tracked source.

## The Claude Code Plugin (`crates/ss-magic-plugin/`, `plugin/`, `scripts/`)

`ss-magic-plugin <VERB>` is a second binary beside the sync CLI. It runs inside
a developer's Claude Code session, writes into a state tree beside the secrets
the rest of the tool moves, and is reached by an automated caller that cannot
see its errors – so review it with the same suspicion as the sync engine.

### Two callers, two opposite postures

- **`ss-magic-plugin hook <event>` serves the harness.** The envelope arrives on
  stdin and the ONLY thing allowed on stdout is the JSON response, so a stray
  `println!`, a progress line, or an ANSI escape corrupts it. The crate root's
  `run` calls `style::init_no_color()` for a hook invocation precisely for this –
  flag a change that initializes color unconditionally, or that prints to stdout
  from a handler instead of returning a `Response`. Handler diagnostics go to
  stderr through `HookContext::diagnostic`, flushed after dispatch.
- **A named verb (`status`, `checklist`, …) serves a skill or the person's own
  request**, and reports on stderr with a non-zero exit like any CLI. Nobody
  types one at a shell prompt: the binary is installed under
  `${CLAUDE_PLUGIN_DATA}` and deliberately kept off the user's `PATH`.
- **No hook may reach anything that can set `plugin.enabled`.** That, not "no
  hook writes configuration", is the property. `enable`, `disable` and `config
  set` are the three verbs that can reach the key, and `HumanVerb::can_set_enabled`
  is the predicate that marks them; no hook invokes any of them. `HumanVerb::
  writes_config` is BROADER and no longer implies hook-unreachability, because
  `seed-config` writes configuration AND is invoked by the `SessionStart`
  bootstrap – see the seed's bounds below. Flag a hook handler that reaches a
  verb marked `can_set_enabled`, a new `enabled`-writing path added under
  `hook/`, or a review comment that re-derives "hook-reachable" from
  `writes_config`.
- **There is no `install` verb**, and `ss-magic sync` runs no plugin step: the
  marketplace is the only delivery path. Flag any code that writes a plugin tree
  onto the machine.

### A hook fails OPEN; a gate fails CLOSED

These pull in opposite directions and both are load-bearing. Do not "simplify"
either toward the other.

- **Fail-open, structurally.** `hook::run` has no code path that yields a
  non-zero exit, a handler panic is caught with `catch_unwind`, and an
  unroutable event name is a VALUE (`HookEvent::Unknown`) rather than a parse
  error – a manifest from a newer build can name an event this binary never
  heard of, and the contract is "exit 0, print nothing, record the name". An
  error, a panic, or a timeout must look to the harness exactly like a hook that
  decided to do nothing; a tool that is only advisory must never break a session
  in progress. Flag a `?`/`bail!` that can propagate out of `hook::run`, a
  removed `catch_unwind`, or an unroutable event turned into an error.
- **Fail-closed on anything that could leak.** The state-tree gate refuses on
  BOTH "git says not ignored" AND "git could not be asked"; the tracked-path
  check uses POSITIVE tracked determination (`git::tracked_files`), so an
  unenumerable name defaults to tracked-and-skipped; the temp-root ownership
  check refuses a base it cannot verify. Flag any of these rewritten so the
  unknown answer becomes the permissive one.
- **The gate can only DENY, never ALLOW.** `event::PermissionDecision` has a
  single `Deny` variant and there is no `updatedInput` rewrite channel anywhere
  in `Response`; `PreCompact` and `SessionEnd` have no `Response` variant at
  all, so their silence is enforced by the type system. These are structural
  guarantees – flag the addition of an `Allow`/`Ask` variant, a rewrite channel,
  or a response variant for a silent event.

### State: where it goes, and what guards it

- **`.superset/.magic/` is written only after git confirms it is ignored.**
  Core's `state_tree::ensure_state_ignored` is the ONE place that gitignore rule
  is written – the CLI's init/migrate calls it eagerly, the plugin's `enable` /
  `config set plugin.enabled true` call it lazily through the plugin's
  `scratchpad` re-export, and NO hook calls it, the `seed-config` bootstrap
  write included. The check uses
  `git::is_ignored_no_index_str` (rules-only, index-ignoring) so a tracked file
  inside the tree does not read as "the tree is unignored". Flag a write into
  the state tree that skips the gate, a hand-rolled gitignore append for it, or
  a hook that adds the rule.
- **Scaffold, never rewrite; never adopt a tracked path.** The six model-owned
  state files are created only when genuinely missing, via `create_new` (atomic
  against a race), and an existing one is left byte-for-byte alone; only the
  `current.json` pointer is rewritten each run, under an fd-lock plus
  temp-file-then-rename. A path git reports as tracked is skipped. Flag a
  truncating open, a blanket rewrite of a state file, or a tracked path adopted.
- **Containment is checked before creation.** Every directory and file the
  scratchpad writes is verified to canonicalize inside the worktree root, so an
  existing symlink cannot redirect a write outside it. Flag a new write path
  that skips the containment check.
- **The machine-level stores are deliberately outside any worktree.** The hook
  heartbeat log and the cost ledger live in the OS DATA dir (not the cache dir,
  which disk cleanup sweeps) because their rows must outlive worktree deletion.
  Flag a move of either into a repository or into the cache dir.
- **Mode bits: 0600 files / 0700 dirs for anything machine-local**, and 0644
  only for content that is committed (the generated CI workflow). Flag a
  world-readable state file or a 0600 committed artifact.

### Compaction guidance is advice only; `--set` is the single write

Four surfaces talk about the auto-compact window – `compact-window
--recommend`, `status`'s `Compaction` section and its `problems` line,
`enable`'s tip line, and the `SessionStart` operator notice – and every one of
them is read-only. The ONLY thing in the crate that writes a settings file is
`compact_window::run_core`, reached from an explicit `compact-window --set
<TOKENS>`, and it writes ONLY the gitignored `.claude/settings.local.json`,
never over a value already there. Nothing may edit the user's
`~/.claude/settings.json` (or `${CLAUDE_CONFIG_DIR}/settings.json`), the
git-tracked `.claude/settings.json`, or a platform managed-settings file, and
nothing may remove `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` from anywhere – the report
names the file and tells the person to delete the key by hand. Flag any new
write to a settings file outside `run_core`, any write reachable from
`--recommend` / `status` / `enable` / a hook, an "auto-apply the
recommendation" path, or a removal of the override; also flag a test for the
report that does not snapshot the tree before and after.

Two smaller rules ride along. The recommendation arithmetic is INTEGER
(`(max_peak * 5).div_ceil(4)`, then `div_ceil(10_000) * 10_000`, then a clamp
to 100000-1000000): `1.25 x 80000` must stay exactly 100,000, and a float
rewrite would round it to 110,000 – flag one. And the once-per-machine
`SessionStart` notice writes its `compact-advice-shown` marker only when the
notice actually went out (a silent path – wrong source, quiet mode, a window
already configured – must not spend the budget) and is withheld outright when
no cache directory can be resolved, because "once" is the promise; flag a
marker written on a silent path or a notice emitted with nowhere to record it.

`hook::quiet_mode(envelope, entrypoint)` is the one "is anybody watching"
verdict: quiet on `permission_mode` `bypassPermissions` or `dontAsk`, or on a
`CLAUDE_CODE_ENTRYPOINT` other than `cli`; ABSENT signals are NOT quiet, on
purpose (the notices are one operator line each and bounded, so a wrong
"quiet" hides them from every harness that omits a field). Flag an inverted
default, a new headless heuristic added on suspicion, or an operator notice
placed on `additionalContext` instead of `systemMessage`.

### Exactly-once claims must not be built on `unlink`

**Never treat a successful delete as having won a claim.** Measured on this
repo's own code: 8 threads racing to `unlink` one path produced up to 5
successes across 20 trials. Sequential testing shows exactly the `ENOENT` you
expect, which is what makes it dangerous – the one-shot bypass token ("exactly
the next gated Read") was built on it and would have admitted every concurrent
read that raced it.

The plugin crate's `claim.rs::take` is the single correct primitive: create a private
landing file in the SAME directory (so the rename never crosses a filesystem)
and `fs::rename` the claim onto it. `rename` requires its source to exist, so
exactly one caller wins. Flag any new one-shot/exactly-once store built on
`remove_file(...).is_ok()`, and flag an exclusivity test that only calls the
claim twice in a row – that proves the state machine, not the exclusion, and a
correct test must race N threads and assert exactly one winner.

### Parse-sensitive git output must bypass the trimming helper

`git()` and `git_optional()` in core's `git/mod.rs` `.trim()` the whole output.
That is right for a single value and **destructive** for a fixed-column format:
`git status --porcelain`'s index column is a literal SPACE when a file is
modified in the worktree only, so trimming eats the leading space of the FIRST
line and shifts every field on it – the status is misread and the path loses its
first character. `git::status_porcelain` is written against `git_raw` and splits
with `str::lines()` for exactly this reason. Flag a parse-sensitive git call
(porcelain, `check-ignore -v`, anything NUL-separated or column-indexed) routed
through `git`/`git_optional`, a per-line `.trim()` applied to such output, or a
"simplification" of `status_porcelain` back onto the shared helper.

### Config resolution is infallible and load-modify-write

- **Every malformed field degrades to a safe default; an out-of-range number is
  CLAMPED, not rejected.** A typo must never leave the gate more permissive than
  configured, and must never hard-fail a session. Flag a `?` that can propagate
  out of config resolution, or a bad value that widens a limit.
- **`plugin.enabled` is always read from the MAIN CHECKOUT's overlay**,
  regardless of the cwd, because a worktree's own `magic.local.json` is itself a
  forward-sync target. The `gate` block resolves against the cwd root. Flag a
  change that resolves `enabled` from the worktree.
- **Writes are load-modify-write on exactly ONE file and preserve unknown
  keys.** `MagicConfig` carries a flattened `extras` map and
  `write_magic_json(root, &MagicConfig)` / `write_magic_local_json` take the
  whole typed config, so a key a newer build or a hand edit put in the file
  survives. `config set` is scoped to keys rooted at `"plugin"`. Flag a write
  that rebuilds the file from known fields only, that touches both layers, or
  that reaches outside the `plugin` key.

### The `seed-config` bootstrap write has SIX bounds, each a test

`seed-config` exists because there is no terminal path to the configuration at
all: the CLI dropped its `plugin` subcommand, and the plugin's binary lives under
`${CLAUDE_PLUGIN_DATA}`, off the user's `PATH`. Rather than document a command
nobody can type, `hooks/bootstrap.sh` invokes the verb on every session that
reaches a usable pinned binary, so the gate's knobs are visible in a file the
repository already tracks. Note the frequency: the binary is installed once per
MACHINE but the block is seeded once per REPOSITORY, so a call sited only after
a fresh install seeds the first repository and silently skips every later one.
Flag a change that moves the call behind the already-installed fast path.
It is the one config write a hook can reach, so its bounds are the whole safety
argument. Each is asserted by a test, not left to convention:

1. **It never writes `enabled`.** `config::seed_block` is built field by field
   from `GateConfig::default()` as a literal map with three named inserts, so
   there is no code path to that key at all – a stronger guarantee than being
   kept away from it by convention. Writing `false` would buy nothing (an absent
   key already reads as off) and would make the seed look like a decision about
   enablement, which it must not be. Flag a rewrite that serializes a config
   struct instead (the guarantee would then follow whatever fields the struct
   grows next), and flag any `enabled` insertion here.
2. **It never stages.** No `git add` here or downstream: the block appears in
   `git status` as an ordinary edit, because it is being SURFACED, not slipped
   in. Flag a staging call anywhere on this path.
3. **It writes only when there is no `plugin` key at all.** Any existing block –
   seeded earlier, hand-edited, or committed by a teammate – means "already
   present" and no write, so the seed cannot repeat on later sessions or fight a
   deliberate edit. Flag a merge/deep-merge that touches an existing block.
4. **It never creates the file.** An absent `.superset/`, an absent
   `magic.json`, or one that does not parse all mean "not a workspace" and no
   write. Installing a plugin must not introduce a tracked file into a checkout
   that is not an ss-magic workspace, and an unparseable file is far more likely
   a merge conflict than an invitation to rebuild it. Flag a create-if-missing
   or a rebuild-from-defaults path.
5. **It never writes through a symlink that leaves the repository.**
   `seed_config_at` canonicalizes both the repository root and
   `.superset/magic.json` and requires the second to sit inside the first, so a
   link on the file OR on the `.superset` directory is refused
   (`SeedOutcome::OutsideRepository`). This is the bound the split created: the
   same write used to need a person to type a config verb, and now fires
   unattended from a `SessionStart` hook against a path the repository controls
   – `serde_json` reads through a symlink and `fs::write` writes through one, so
   without the check any JSON object file the user can write is in range. Flag a
   check that canonicalizes only the leaf, or only the root, or compares
   un-canonicalized paths.
6. **It preserves every other key.** The write goes through the same typed
   load-modify-write as the rest (`MagicConfig`'s flattened `extras`), never a
   hand-rolled JSON splice. Flag a text-level edit of `magic.json`. Note it
   re-serializes rather than patching, so unknown keys come back alphabetized;
   values are what survive, not byte order.

Every outcome is a normal result, never an error: the verb runs unattended from a
`SessionStart` hook in whatever repository the session happens to be in, and most
of those are not ss-magic workspaces at all. Flag a `?`/`bail!` that can turn one
of the four outcomes into a failed session start.

### The operator checklist is CLI-write-only

The `PreToolUse` handler denies a direct `Read`/`Edit`/`Write`/`NotebookEdit` of
a checklist file, so the plugin crate's `checklist/verbs.rs` is the ONLY write path – that
is what keeps every stored document canonically ordered and valid.

- Every mutating verb is read-modify-write over the WHOLE document (read →
  mutate one field → `canonicalize` → re-stamp `updated` → write back), so the
  flattened `extras` on every level survive; writes are temp-file-then-rename
  preserving the existing mode. An advisory lock spans the entire
  read-mutate-write, and spans exist-check plus write for `init`. Flag a partial
  write, a mutation that skips `canonicalize`, or a lock narrowed to the write.
- `canonicalize` must stay a pure function of content (items sort by `(done,
  priority rank, created)` with the id as final tie-break) so it is idempotent,
  and it must compare timestamps through the parsed instant, NEVER as strings –
  a `+02:00` stamp can sort lexically after a `Z` stamp that is actually
  earlier. Section order is author-declared and never re-sorted.
- The schema is permissive on purpose (every field defaulted) so a hand-edited
  file still parses; defects are the validator's job. `kind` defaults to the
  strictest variant so a missing kind never silently disables verification, and
  `expected` is `Option<Option<String>>` because an absent key and an explicit
  null differ. Flag a field made mandatory at the parse layer, or a defaulted
  `kind` that is not the strict one.
- Exit codes are distinct on purpose: 2 for "the command as typed cannot be
  carried out", 1 from `verify` for "the document is invalid", so CI can tell
  them apart. `Severity::Warning` describes shape defects the next write
  self-repairs and must NEVER fail CI. Flag a collapse of the two exit codes, or
  a warning promoted to a CI failure.
- The `.superset/.magic/checklist.json` pointer's contents are NOT trusted: the
  target is validated lexically against absolute paths and `..` segments. Flag a
  pointer target joined without that check.
- All rendering goes through one `render()`, so the CLI, the commit-time nudge
  and the CI comment are byte-identical; user-authored prose is escaped before
  insertion, timestamps render through a fixed UTC formatter (never a local
  clock), and the output is wrapped in the shared untrusted-data envelope with
  the framing text placed BEFORE the quoted body. Flag a second rendering path,
  unescaped prose, a locale/local-time date, or a bypassed envelope.

### A path gate classifies from the TARGET, in three fixed moves

The checklist deny above is only as good as its answer to "is this path that
file". Eight separate bypasses of it came from one habit: deciding from the
ACTOR (the hook's own process, the envelope's cwd) rather than from the TARGET,
and recognizing SPELLINGS rather than the property behind them. A symlinked
ancestor, a case difference, a relative target, a `..` component, a
`/proc/self/cwd` prefix, a leading `..`, a decoy symlink in an opaque path's
TAIL, and a leading `~` were each patched one at a time, and each patch produced
the next hole. Review any change to path classification against these three
moves, in this order.

- **Move 1 – expand, then reduce, before anything else looks at the path.**
  Perform every expansion the harness performs before it opens a file, or refuse
  to root the path. A leading `~` must be expanded against `HOME` on the RAW
  spelling, AHEAD of the lexical reduction: the normalizer treats `~` as an
  ordinary segment, so `~/../x` would otherwise have its `~` popped and come out
  working-directory-relative. `~name` (another account's home, known only to a
  user database) and an unset or non-absolute `HOME` must make the path
  unrootable, NEVER a guess at `/home/<name>`. Flag a reduction that runs before
  expansion, a guessed home directory, or a new expansion added on suspicion
  rather than measurement – the surface is bounded by probing (`~` diverges
  between harness and hook; `$HOME`-style syntax was probed and provably does
  not, so it is deliberately unhandled).
- **Move 2 – decide process-relativeness as a property, not a prefix list.** A
  path is process-relative when a `proc` component is followed by a process
  selector (`self`, `thread-self`, or all-digits) ANYWHERE in the component
  sequence – procfs is mountable anywhere, so a fixed `/proc/...` prefix match is
  wrong. Only `…/proc/<selector>/cwd/<rest>` is re-rootable; `root`, `fd/<n>`,
  `task/<tid>` and `ns/…` name what only the selected process sees. The FIRST
  selector must win, or `/proc/self/root/proc/self/cwd/…` re-roots on another
  process's mount namespace. Never trust a resolution whose result depends on
  which process performs it: `canonicalize` may be used to ADD a denial but
  NEVER to CLEAR one, because it answers about the hook's process and a wrong
  answer must cost a redirect rather than the deny itself. Flag a prefix match, a
  last-match scan, or a canonicalize that clears a path.
- **Move 3 – derive comparison roots from the target as well as the actor,** and
  compare fold-case where the naming convention does. Flag a comparison whose
  both sides come from one source: a fixture built that way cannot fail a basis
  bug, which is what hid three of the eight.

Lexical reduction itself must count leading `..` rather than push them – a
pushed one is poppable by the next `..`, so `../../x` collapses to `x`, turning a
path that escapes its tree into a valid-looking one inside it. And it must not be
reimplemented by walking `parent()`/`file_name()`: `file_name()` returns `None`
for a `..` component, so such a walk SKIPS the hop instead of cancelling it.

Test both levels. A unit test of the reduction helper is NOT a test of the gate –
during one of these fixes the helper's own `..` test passed throughout a revert of
the gate's call to it. Flag a new path-classification behavior covered only by a
helper unit test.

### The commit nudge is advisory and narrowly scoped

The `PreToolUse[Bash]` nudge matches only a command whose trailing words are
`git commit`, `git push`, or `gh pr create`. `gh pr view`/`list`/`diff` must NOT
trigger it – they open nothing. It fires only when `git::status_porcelain` shows
a candidate checklist untracked or edited-but-unstaged, sets
`additional_context` and NEVER a decision, and its text says the command was not
blocked. Flag a nudge that sets a decision, that widens the `gh pr` match beyond
`create`, or that fires with no checklist in the repository.

### The packaged plugin tree is content-pinned

- `.claude-plugin/marketplace.json` pins the `plugin/` zip by SHA-256; that pin
  is the ONLY integrity control on the plugin. The `sha256` key is optional in
  the schema and unknown keys inside a source object are silently ignored, so a
  typo such as `"sha"` validates cleanly and installs the plugin UNPINNED –
  which is why `python3 scripts/build-plugin-zip.py --check` asserts the key
  exists mechanically. Flag a renamed/removed `sha256`, or a source URL that is
  not https.
- The zip must stay **byte-reproducible**: sorted entries, fixed 1980-01-01
  timestamps, normalized modes (0644, 0755 under `bin/` and for `*.sh`),
  `create_system` forced to unix, STORED not deflated, `.DS_Store` excluded, and
  a LOUD refusal on a symlink or a non-ASCII filename (macOS normalizes to NFD
  and Linux to NFC, which hash differently). `.gitattributes` marks `plugin/**`
  as `-text` so checkout-time line-ending conversion cannot move the digest.
  Flag a builder change that reads a clock or an mtime, that deflates, that
  drops a refusal, or a removal of the `plugin/**` `-text` rule.
- **Any change under `plugin/` must re-pin AND bump the whole
  `ss-magic-plugin` version group** (see Version Bump Discipline below for the
  surfaces). Run `python3 scripts/build-plugin-zip.py --update-manifest` then
  `--check`. The resolved VERSION, not the digest, is the client's update
  signal – changing the zip and its `sha256` without a version bump leaves every
  installed user silently on the cached copy. Flag a `plugin/` change with a
  stale digest, an unbumped version, or a bump that touched the CLI's surfaces
  instead.
- **The pin file is `plugin/ss-magic-plugin.version`**, and the installed binary
  is `${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin`. Both were renamed when the
  plugin became its own binary; the marketplace plugin NAME and every marker
  file stayed `ss-magic`, deliberately, so per-machine state stays where it is.
  There is deliberately NO cleanup of a stale pre-split `bin/ss-magic` – nothing
  spawns it once the manifest and the wrapper both name the new one, so it is
  inert. Flag migration code added to remove it: a bootstrap that deletes files
  is a new failure mode on a path that must never fail a session.

### No hook command may name an artifact created at runtime

Every entry in `plugin/hooks/hooks.json` must spawn something that exists the
moment the plugin is installed: `"command": "bash"` with `args[0]` a path under
`${CLAUDE_PLUGIN_ROOT}`. **A command naming `${CLAUDE_PLUGIN_DATA}` – or any other
path the bootstrap creates at runtime – is a bug**, and it shipped once.

`${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin` does not exist until the
`SessionStart` bootstrap fetches it, and hooks on one event fire CONCURRENTLY, so
the bootstrap cannot be relied on to finish first. A manifest naming the binary
directly makes the harness `posix_spawn` a path that is not there, and the user's
first session dies with `ENOENT (posix_spawn)`. The specified behaviour is the
opposite: with no binary present every hook is INERT and the session behaves
normally. **The binary having its own name now changes nothing here** – it is
still fetched at runtime, so naming `ss-magic-plugin` in `hooks.json` reproduces
exactly the failure the shim exists to prevent.

The reason this is a manifest rule and not a code rule: the binary implements
fail-open itself – its hook entry point has no non-zero exit path and catches
handler panics – but **that code is unreachable when the binary is the missing
thing.** Fail-open has to live in a shim that always exists
(`plugin/hooks/run-hook.sh`), not inside the artifact that may be absent. Flag any
`command` that is not `bash`, any `args[0]` outside `${CLAUDE_PLUGIN_ROOT}`, and
any reasoning that treats "the binary handles it" as covering the case where the
binary is gone.

That shim is silent on BOTH stdout and stderr, unlike `bin/ss-magic-plugin`, which
prints one explanatory stderr line. The difference is the consumer: the wrapper
serves a person running a skill, while the shim runs on `PreToolUse`, which fires
on nearly every tool call. Flag a diagnostic added to the shim.

**Assert this over every entry, never just the one you are touching.** The
manifest check originally covered the bootstrap group alone, which is exactly how
the other five entries drifted into naming the binary. Assert the event token as
well as the script path: a manifest naming the right script with the wrong token
in `args[1]` routes the event to the wrong handler, and a check that reads only
`args[0]` passes it.

The invariant is asserted TWICE on purpose: `scripts/test-bootstrap.sh` walks
every `hooks.json` entry, and `python3 scripts/build-plugin-zip.py --check`
repeats it as its `hooks spawn through the shim` line so the one-command release
gate covers it without running the bash suite. **The two must be changed
together** – weakening one and leaving the other is how a manifest regression
reaches a release through whichever gate a given run happens to skip. Flag a
change to one without the other.

Note the one entry that must NOT go through the shim: the `SessionStart`
bootstrap. The shim does nothing when the binary is absent, and the bootstrap is
what installs it, so routing the bootstrap through the shim would leave the
plugin inert forever rather than for one session. Flag any change that does this,
and flag any prose claiming *every* hook is spawned through the shim.

### Deciding a file is runnable needs more than `[ -x ]`, and needs one definition

Before any script `exec`s the pinned binary it must go through
`ss_magic_is_loadable_executable` in `plugin/lib/execguard.sh`. **A bare
`[ -x "$bin" ]` is a bug**, and it shipped in two different scripts.

Two failures hide behind that test:

- `-x` is TRUE for a **directory** carrying the search bit. `exec` on a directory
  does not fail quietly – bash prints its own diagnostic and exits 126.
- `-f` and `-x` both ask about the FILE. Neither can see `execve` failing on the
  way into it, and **ENOEXEC is the dangerous one**: when the file is readable but
  not a loadable executable – a truncated download, a foreign architecture, disk
  corruption – bash does not report a failure at all. It falls back to POSIX
  behaviour and REINTERPRETS the bytes as a shell script, so `shopt -s execfail`
  never fires and the process exits with whatever those bytes parse to. Measured
  over 30 corrupted binaries on bash 3.2: exit 2 roughly half the time. **Exit 2
  from a `PreToolUse` hook means BLOCK the tool call**, so a binary damaged after
  install would silently block nearly every tool call in the session.

The guard is ONE shared file on purpose. The check was added to
`hooks/run-hook.sh` first while `bin/ss-magic-plugin` kept the weaker `[ -x ]`
for a whole release – the sibling drift a shared definition prevents. **Flag any
new inline copy of this check, and flag a fix applied to one of the two scripts
without the other.**

Its two failure directions are deliberately opposite, so do not "simplify" either
into the other: an unrecognised magic number fails CLOSED (refuse to exec), while
a missing `od` or `tr` fails OPEN (proceed), because refusing there would silently
disable the whole plugin on a machine merely lacking a utility. Callers still set
`shopt -s execfail` afterwards, for the exec failures no file test can see.

`hooks/bootstrap.sh` is exempt and should stay exempt: at install time it runs the
staged binary and refuses the install unless it reports the pinned version, which
is strictly stronger than a magic-number test.

Keep `exec` in both callers. It makes the harness's child pid BE the binary, so a
hook timeout kills the binary instead of killing the shell and orphaning it –
which matters most on `SessionEnd`, the hook the CLI blocks on while a session
exits. Flag a change that drops `exec` for a plain call. Equally, flag
`exec ... 2>/dev/null`: it applies to the SUCCESS path too and would swallow the
binary's own diagnostics.

### The bootstrap must never fail a session

`plugin/hooks/bootstrap.sh` runs on every fresh session on every machine.

- **No `set -e`; every path ends in `exit 0`.** Offline, DNS failure, proxy,
  404, checksum mismatch, unwritable data directory, unsupported platform: all
  are "do nothing, one line on stderr, exit 0".
- **Nothing on stdout on the success path.** A `SessionStart` hook's stdout
  enters the model's context every session, so silence is a token-budget rule,
  not a style preference.
- **An existing binary is never touched by a failing install.** The download is
  verified and staged first, and only a verified binary is moved into place; a
  failed install drops the success marker so the next session retries.
- **It fetches the platform release ARCHIVE and verifies it against that
  archive's published `.sha256`.** It must NOT pipe `ss-magic-installer.sh` into
  a shell, even as a fallback: the release publishes `.sha256` siblings for the
  archives but not for the installer script, so a piped installer is the one
  executed artifact no published digest covers. Flag any reintroduction of an
  installer-script hop.
- The install target is `${CLAUDE_PLUGIN_DATA}`, never `${CLAUDE_PLUGIN_ROOT}`
  (which is version-scoped and replaced wholesale on each plugin update), and
  the braced form is required for harness substitution. Flag either inversion.
- It is written for **bash 3.2** (macOS's version): no associative arrays, no
  `mapfile`, no `${var^^}`. Flag bash 4+ syntax here or in
  `scripts/test-bootstrap.sh`.
- `plugin/bin/ss-magic-plugin` is the wrapper skills invoke. It `exec`s the
  installed binary with argv passed through VERBATIM – there is no `plugin` verb
  to inject any more, because the binary's own argv starts at the verb
  (`ss-magic-plugin checklist list` is exactly what the binary sees). It must
  keep its distinct name: a wrapper called `ss-magic` would resolve
  non-deterministically against a user's own install, and would hand a skill the
  sync CLI's update gate and TUI. A missing binary is a normal state – it exits 0
  with one stderr line. No skill body may name `${CLAUDE_PLUGIN_DATA}` or a bare
  `ss-magic`; CI asserts both, plus that no document spells the retired
  `ss-magic` + `plugin` subcommand form.
- **`ss-magic-plugin` is required in ALL model-facing text, and there is no
  longer any exception.** Any command a hook response tells the model to run –
  every deny reason, nudge, and `additionalContext` – is executed through the
  Bash tool, where `${CLAUDE_PLUGIN_DATA}` is NOT exported, so only the wrapper
  resolves. A bare `ss-magic` there reaches nothing on a marketplace-only install
  (the only delivery path), so a conclusion can never be recorded and a bypass
  never consumed – the same oversized Read stays a miss forever. This already bit
  once: the checklist deny carried the wrapper from the start while the size-gate
  deny quietly did not. The human verbs' `Usage:` strings USED to be a deliberate
  exception, on the reasoning that a person runs those in a terminal; they no
  longer are, and every one of them now spells `ss-magic-plugin`, because nobody
  runs a verb in a terminal at all. Flag a bare `ss-magic` in any string this
  crate prints or returns.

### The plugin never self-updates and never opens a TUI, by construction

This used to be maintained by keeping the plugin out of the CLI's update gate.
It is now structural: `ss-magic-plugin` is its own binary linking neither
`self_update` nor `inquire`/`ratatui`, so there is no gate to stay out of and no
menu to construct. The binary is pinned alongside the skills, hooks and Markdown
the marketplace ships with it, so a silent mid-session swap would leave the two
describing different behavior; updating the plugin through the marketplace is
what replaces the binary. Flag any dependency or code path that would restore
either capability, and flag a "convenience" re-export that lets plugin code call
into the CLI crate.

### The shipped manifest declares FIVE hook events

`plugin/hooks/hooks.json` registers `SessionStart`, `PreToolUse`, `PreCompact`,
`SubagentStop`, and `SessionEnd` – and no `FileChanged` entry. `HookEvent`
parses a `file-changed` token and `hook/mod.rs::route()` still has an arm for it,
so the plugin crate's `hook/file_changed.rs` is reachable by argv and stays
covered by tests, but **nothing in a real session invokes it**. Do not describe it as a
shipped hook, and do not "fix" the manifest by adding a `FileChanged` entry
shaped like the others: that matcher is a watch-path list, not a name filter, so
an entry without one registers zero watch paths and can never fire. Flag
documentation or a status report that claims `file-changed` is active
(`status::DECLARED_EVENTS` deliberately lists only the five).

## Filesystem Writes: Atomic Staging

- `.superset/` materialisation stages the whole tree in a tempdir and copies it
  into place only after the user confirms the finishing action
  (`superset_files::copy_into_repo`, driven by `workspace/migrate.rs`). `*.sh` files are
  chmod `0755`; a `delete` set strips retired files (e.g. the old `setup.sh`).
  Flag partial in-place writes to `.superset/` that bypass this staging.
- `pack::write_archive` writes the archive to a `NamedTempFile` in the git root
  and renames it into place atomically only after the tar+bzip2 stream is fully
  finalised (`into_inner()` then `finish()`). Flag an archive path that writes
  the final archive (the derived `ss-magic-<repo>.tar.bz2`) directly, or that
  renames before both stream layers are flushed.

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

## Self-Update Safety (core's `release.rs`, the CLI's `update/`)

Only `ss-magic` self-updates. The per-line release CHECK lives in core
(`release.rs`); the apply path lives in the CLI crate (`update/`), and nothing in
the plugin crate reaches either.

- The daily-cached release check (core's `release.rs`) lists `/releases` (first
  page) and filters PER RELEASE LINE with an anchored, exact tag filter
  (`parse_line_tag`: `CLI_LINE` accepts only `v` + `MAJOR.MINOR.PATCH`,
  `PLUGIN_LINE` only `ss-magic-plugin-v` + `MAJOR.MINOR.PATCH` – nothing
  before, nothing after, case-sensitive, ASCII digits only), drops drafts and
  prereleases, and selects the GREATEST triple rather than the first entry.
  Both filters are load-bearing now that both tag shapes really exist in the
  repository: an unanchored `v` match would let `ss-magic-plugin-v1.0.0` read as
  a CLI release. Flag any change that reads `releases/latest`, matches a tag by
  substring or an unanchored regex, takes the first match, or lets a tag of the
  other line through. It uses `ureq` with an ETag and a short timeout, and must
  fall through SILENTLY on any offline / non-200 / timeout / non-JSON-array
  result – a failed update check must never block or slow a normal invocation.
  Flag an update-check change that surfaces a hard error or removes the timeout.
- Every `self_update` call is pinned: `apply_update`, `apply_update_unlocked`
  and `run_self_update` (`update/apply.rs`) take a mandatory `&str` tag and
  always set `target_version_tag`. Flag a signature that regrows
  `Option<&str>` or any path that lets the backend choose "latest" itself –
  with two release lines in one repository it WOULD install a plugin release
  over the CLI, and the two are no longer even the same program.
  `ss-magic update` resolves the tag first
  (`release::resolve_newest_uncached`, no cache) and maps a failed resolution to
  `UpdateReport::Unavailable` ("could not check"), never to `AlreadyLatest`;
  flag a change that conflates the two.
- The apply path (`update/apply.rs`) takes an advisory `fd-lock`
  (skip-on-contention), downloads over TLS, atomically swaps the binary, then
  re-execs and blocks on the child. The re-exec loop guard (`SS_MAGIC_UPDATED`
  / `SS_MAGIC_NO_UPDATE`) must prevent infinite re-exec — flag changes to
  `should_run_update_gate` / `guard_active` that could let a re-exec'd child
  re-enter the gate.
- The auto-update gate fires for `Bare`, `Sync`, `ReverseSync`, and `Pack`
  (`should_run_update_gate`); `Update` uses its own force path and bypasses the
  daily-cache gate. Keep this consistent when a new command is added.

## Style / Output

- All colored output goes through core's `style.rs` (gray info, bold green ok,
  bold orange warn, bold red err, bold cyan header) – reached as
  `crate::tui::style` from the CLI and as `ss_magic_core::style` from the plugin.
  The color decision (NO_COLOR + supports-color) is captured once in a
  `OnceLock<bool>`. The `inquire` half lives separately in the CLI's
  `tui/theme.rs`, which is what lets the palette sit in a crate that links no
  prompt library. Flag raw ANSI escape codes emitted outside core's `style.rs`,
  output that ignores the NO_COLOR decision, or an `inquire` type reintroduced
  into core.
- Interactive prompts must be inert on Esc / Ctrl-C (leave the tree untouched
  and exit success) — `tui/menu.rs` and the pickers follow this. Flag an
  interactive path where cancellation mutates the filesystem.
- A `ss-magic-plugin hook` invocation owns stdout for its JSON envelope: color is
  forced off there and nothing but the envelope may be printed. Flag a `println!`
  added to a hook handler, or a style init that ignores the hook case.

## Version Bump Discipline (REQUIRED)

**There are TWO release lines, with TWO version groups that never mix.** The
surfaces belong to exactly one group each:

| Group | Tag shape | Surfaces that must all agree |
|---|---|---|
| `ss-magic` | `vX.Y.Z` | `crates/ss-magic/Cargo.toml` `[package] version`; the `name = "ss-magic"` entry in `Cargo.lock`; `README.md`'s pinned installer tag – compared `<=`, never `==` |
| `ss-magic-plugin` | `ss-magic-plugin-vX.Y.Z` | `crates/ss-magic-plugin/Cargo.toml` `[package] version`; the `name = "ss-magic-plugin"` entry in `Cargo.lock`; `plugin/.claude-plugin/plugin.json`; `plugin/ss-magic-plugin.version`; both the tag AND the asset name in `.claude-plugin/marketplace.json`'s release URL; the literal zip filename in the plugin crate's `[[package.metadata.dist.extra-artifacts]]` (cargo-dist does not template it) |

`crates/ss-magic-core/Cargo.toml` is NOT a surface (`0.1.0`, `publish = false`,
`dist = false`). The root `Cargo.toml` is virtual and carries no version.
`Cargo.lock` holds one `[[package]]` per crate, so a bump must match on the
`name = "…"` line, never on a bare version string – and the lock is regenerated
by `cargo build`, never by a blind replace.

**The two groups' versions must NEVER be equal.** cargo-dist parses a release tag
as `[PACKAGE_NAME-]VERSION`, so a bare `vX.Y.Z` tag announces EVERY dist-able
package sitting at that version – pushing the CLI's tag at equal versions would
publish a `ss-magic-plugin` release nobody asked for, under a tag shape the
plugin line never uses. There is no way to say "this tag means the CLI only";
keeping the numbers apart IS the mechanism. `python3
scripts/build-plugin-zip.py --check` refuses a tree where they match (its
`distinct release lines` line). Flag a PR that lands equal versions, and flag a
change that weakens or removes that guard.

**The README installer pin is compared `<=`, not `==`, on purpose.** It names the
last PUBLISHED CLI release, and the procedure is bump → merge → tag, so an
equality rule would make `main`'s README name an unreleased tag – and 404 the
documented install command – for the whole window between a merged bump and a
published release. A lagging pin names an older release that still works. Flag a
"fix" that tightens this to equality, a pin that EXCEEDS the crate version, or
any `releases/latest/download/` URL in `README.md` (the mark is repository-wide
and can resolve to a plugin release, which publishes no installer script).

**When to bump.** For `ss-magic`: any change that alters CLI behavior – a fix, a
new/changed command or flag, or different output – because the installed binary
self-updates keyed on that version and a stale version means users never receive
the change. Bug fixes bump patch; new/changed user-visible behavior bumps minor
(pre-1.0). For `ss-magic-plugin`: any change under `plugin/` or in the plugin
crate, and the change must ALSO re-pin the digest (`--update-manifest`, then
`--check`). The resolved VERSION, not the digest, is the client's update signal,
so a content change without a version bump leaves every installed user silently
on the cached copy. Flag a behavior-changing PR that bumps neither group, one
that bumps a manifest without `Cargo.lock`, a `plugin/` change with any surface
out of step or a stale digest, or a bump applied to the wrong group's surfaces.

## Test Requirements

- **`cargo test --workspace` is not the whole suite.** Four checks cover ground
  it cannot reach, and CI runs all of them:
  `python3 scripts/build-plugin-zip.py --selftest` (the zip builder's
  reproducibility guarantees and its refusals);
  `python3 scripts/build-plugin-zip.py --check`, whose seven assertion lines are
  `R101 marketplace sha256 key`, `R95 version surfaces (ss-magic)`,
  `R95 version surfaces (ss-magic-plugin)`, `distinct release lines`,
  `hooks spawn through the shim`, `workspace shape` and
  `R96 committed digest pin`;
  `/bin/bash scripts/test-bootstrap.sh` (the bootstrap's failure paths –
  offline, corrupted download, hostile pin, unwritable data dir, unsupported
  platform, concurrent sessions – each asserting exit 0, empty stdout, and an
  untouched pre-existing binary, PLUS the shim's inertness contract, the
  wrapper's, and the `hooks.json` manifest invariant over every entry); and
  `cargo tree --locked -p ss-magic-plugin -i <crate>` for each of `self_update`,
  `inquire` and `ratatui`, which must report no match – a build can only show a
  dependency is present, so this is the only evidence of absence. Flag a change
  to `plugin/`, `scripts/`, either binary's manifest, or the release assertions
  that leaves any of these unrun or unmentioned, and flag a `--check` assertion
  quietly dropped from the list.
- Tests use `tempfile` for scratch trees and shell-invoked `git init` /
  `git worktree add` for git fixtures. Pure modules (`cli.rs`, `sync/pattern.rs`,
  `sync/apply.rs`, `sync/mod.rs`, `pack.rs`, `hashing.rs`,
  `workspace/superset_files.rs`, `git/mod.rs` probes, `tui/menu.rs`
  routing via `operations_for`, `sync/merge.rs`, `tui/diffmodel.rs`,
  `sync/reverse_sync.rs`'s `apply_decision`/backup/TOCTOU seam, and every module
  in the plugin crate – its parse, state modules, hook handlers and the whole
  `checklist/` family) have unit
  tests; the interactive
  menu/pickers and final-action git ops are validated by manual smoke, not
  unit tests. The reverse-sync merge cockpit (`tui/cockpit.rs`) is the same
  mix: its event loop and terminal lifecycle are manual-smoke, but its render
  path (`draw`) and pure key dispatch (`handle_key`) ARE unit-tested via
  `ratatui::backend::TestBackend` — do not treat a cockpit regression as
  automatically untested.
- New behavior in a pure module (a new command in `cli.rs`, a new
  `operations_for` entry, new glob/exclude/pack behavior) MUST come with tests
  covering the happy path and key edge cases (empty input, error/hard-fail
  paths, exclusions). Flag a behavior-adding PR to a pure module with no test
  changes.
- Bug fixes SHOULD include a test that reproduces the issue before the fix.
- **An exclusivity property must be tested by racing it, never sequentially.** A
  "consume exactly once" claim checked by calling it twice in a row proves the
  state machine, not the exclusion – a broken `unlink`-based claim passes that
  test and admits several concurrent winners. Require N threads contending and
  an assertion of exactly one winner, repeated enough times to catch a rare
  interleaving. Flag a new one-shot store whose only test is sequential.
- **A parser over line-oriented command output needs a SINGLE-LINE fixture.** A
  defect that corrupts only the first line (a whole-output `.trim()` eating a
  leading status column, for instance) is completely hidden by a multi-line
  fixture. For `git status --porcelain` specifically, also cover the
  worktree-only-modified case (` M`, leading space), not just `M ` and `??`.
- Test layout: every module declares `#[cfg(test)] mod tests;` with the body
  in a dedicated child file (`<module>/tests.rs`) – in all three crates,
  the plugin crate included; the CLI's crate-root tests live in `crates/ss-magic/src/tests/`
  (`sync.rs`, `reverse_sync_flow.rs`, `update_gate.rs`), and the shared
  helpers are `ss_magic_core::testutil` (`crates/ss-magic-core/src/testutil.rs`:
  `git_run`, `init_main_repo`, `make_worktree`, `write_magic`, `write_file`,
  `exit_code_to_u8`, `neutralize_global_excludes`, `run_ignored_test_in_child`,
  `run_ignored_test_in_child_from`, `test_path_in_binary`), compiled under
  `cfg(test)` or core's `testutil` feature, which a binary enables from its
  `[dev-dependencies]` ONLY – flag `features = ["testutil"]` on a normal
  `[dependencies]` entry, since that would compile test helpers into a release
  binary. A test whose subject is the process environment (`PATH`, a `GIT_*`
  variable, or the process's own working directory – `cargo test` starts a
  binary in `crates/<name>`, not the repository root) runs its assertion in a
  child process through those two helpers, never through `set_var` /
  `set_current_dir` in a parallel suite.
  Flag a PR that adds an inline `mod tests { ... }` block to a source file
  instead of a sibling test file.
- CI (`.github/workflows/ci.yml`) runs `cargo test --workspace --locked` on Ubuntu and
  macOS for every PR commit, plus a `plugin` job carrying the builder
  selftest, the release assertions, the `cargo tree` absence proof, a check that
  a content change under `plugin/` came with a version bump (its baseline
  considers BOTH tag shapes), the bootstrap failure-path suite, a build of the
  exact asset cargo-dist will publish, and three document greps: no skill body
  names `CLAUDE_PLUGIN_DATA`; no document (`plugin/skills/`, `README.md`,
  `CONTRIBUTING.md`, `CONCEPTS.md`, this file) spells the retired `ss-magic` +
  `plugin` subcommand form; and `README.md` names no `releases/latest/download/`
  URL. The plan phase additionally refuses a release tag matching neither
  `^v[0-9]+\.[0-9]+\.[0-9]+$` nor `^ss-magic-plugin-v[0-9]+\.[0-9]+\.[0-9]+$` – a
  prefixed CLI tag such as `ss-magic-v0.11.1` would publish a release the
  updater's anchored filter and every installed binary ignore, stranding the line
  silently. It gates cargo-dist releases via `plan-jobs` in
  `dist-workspace.toml`. Flag hand edits to the generated
  `.github/workflows/release.yml` (regenerate with the pinned `dist` version
  instead), flag `allow-dirty = ["ci"]` additions, and flag a workspace-level
  `extra-artifacts` entry for the plugin zip (it belongs on the plugin crate,
  with `working-dir = "../.."`, or it rides every release including the CLI's).
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

`README.md` (user-facing), `CONTRIBUTING.md` (contributor-facing: from-source
builds, tests, PR expectations, release/versioning), `CONCEPTS.md` (domain
vocabulary), and the repo's contributor-instructions file at the repo root
(architecture/conventions) must reflect the current state after every
implementation change — a new command, flag, module, or changed behavior. Flag
a behavior- or architecture-changing PR that leaves any of them describing the
old state (e.g. a new subcommand or plugin verb not listed in the README command
inventory or the
`main.rs`/`cli.rs` descriptions, a changed build/test/release workflow not
reflected in `CONTRIBUTING.md`, or a new module absent from that
contributor-instructions file's per-module Architecture section). The README's
command inventory must match `cli.rs`'s `parse` and the plugin crate's
`HumanVerb`/`HookEvent`, and the documented hook events must match what
`plugin/hooks/hooks.json` actually registers – flag a doc that claims an event
the manifest does not declare. README must also stay explicit that no plugin
verb is typed in a terminal: every one is `ss-magic-plugin <verb>`, reached
through Claude Code, and turning the plugin on for a repository is documented as
editing `"enabled": true` into the seeded `plugin` block, not as a command.
`docs/runbooks/forge-tag-and-release-protection.md` must cover both tag shapes.
This `.cursor/BUGBOT.md` must likewise be re-synchronised whenever the
conventions above change, and must stay self-contained – restate a convention
inline rather than pointing at another document.
