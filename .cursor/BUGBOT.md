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

Standalone interactive Rust CLI (binary: `ss-magic`; repo:
`ViktorStiskala/superset-magic`). Edition 2021. Key dependencies: `anyhow`
(errors), `inquire` (interactive prompts), `ratatui` + `crossterm` (the
full-screen bidirectional sync merge cockpit; `crossterm` also backs `inquire`),
`similar` (line/word diffing for the diff model and merge assembly),
`globset` + `walkdir` (pattern
matching), `serde`/`serde_json` (config I/O), `tempfile` (atomic staging),
`tar` + `bzip2` (pack archives), `self_update` + `ureq` + `fd-lock`
(self-update and the plugin's advisory locks), `directories` (the plugin's
machine-level data dir), `supports-color` (palette). No `clap` (the arg parser is
hand-rolled) and no `git2` (all git/gh is shelled out). Hashing is in-crate
(`src/hashing.rs`: FNV-1a plus a hand-rolled SHA-256) – flag the addition of a
hashing crate for these uses. Release binaries are
built by cargo-dist (`dist-workspace.toml`) and self-update from GitHub
Releases. Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.

The binary also carries a **Claude Code plugin** (`src/plugin/`, exposed as
`ss-magic plugin <VERB>`), shipped as a packaged tree under `plugin/` and pinned
by SHA-256 in `.claude-plugin/marketplace.json`. Two non-Rust pieces are part of
the product, not tooling: `scripts/build-plugin-zip.py` (python3; the
reproducible zip builder and the release assertions) and
`plugin/hooks/bootstrap.sh` + `scripts/test-bootstrap.sh` (bash 3.2 – macOS's
version, so no associative arrays, no `mapfile`, no `${var^^}`).

## No External Process Libraries

- **All `git` and `gh` interaction shells out via `std::process::Command`** —
  there is NO `git2`/`libgit2`. Flag any addition of `git2`, `gix`, or another
  git-binding crate to `Cargo.toml`. The shared entry point is the `git_raw`
  helper in `src/git/mod.rs` (surfaces stderr verbatim); `git` and `git_optional`
  are thin one-liners on top. Flag new git/gh calls that spawn `Command`
  directly instead of routing through these helpers.
- **The CLI arg parser is hand-rolled in `src/cli.rs`** — there is NO `clap`.
  `parse(&[String]) -> Parsed` selects the command from the first non-flag
  token; `Command` is `{ Bare, Sync { no_backup }, ReverseSync { no_backup },
  Pack, Update }`, and `Parsed` additionally carries `Init(Vec<String>)`,
  `Plugin(Vec<String>)`, `Version`, `Help` and `Error(token)`.
  `sync`/`reverse-sync` read the `-n`/`--no-backup` flag via a
  full-argv scan (`has_no_backup`, position-independent – before OR after the
  subcommand), an intentional asymmetry with `-h`/`--help` (a terminal
  short-circuit recognized only BEFORE the subcommand). Flag any addition of
  `clap`/`structopt`/`argh`, command dispatch logic added outside `cli.rs`, or a
  "fix" that makes `has_no_backup` and the `--help` scan match each other.
- **Three deliberate parse asymmetries protect the plugin, and must not be
  "harmonised".** (1) `Parsed::Plugin` carries the remaining argv VERBATIM,
  flags included – unlike `Init`, which filters them – because the plugin verbs
  take their own `--json`/`--local`/`--set`. (2) `version_requested` scans PAST a
  subcommand token, so `ss-magic sync --version` prints the version; without
  that, an unrecognized `--version` is skipped as an unknown flag and falls
  through to `Command::Bare`, which IS update-gated and opens the interactive
  menu – exactly wrong when a hook shells out to identify the binary. (3) That
  same scan STOPS at the `plugin` token, because a `-V` after it may be a verb's
  own flag. Flag a change that filters `Parsed::Plugin`'s argv, that stops the
  `--version` scan at the subcommand, or that lets it run past `plugin`.

## Architecture: Layering (pure logic vs interactive layer)

The codebase is deliberately layered so the pure logic is unit-testable in
isolation from the interactive TUI. Preserve this boundary.

- Pure/testable modules: `git/mod.rs` (probes + mutating primitives), `cli.rs`
  (arg parsing), `sync/pattern.rs` (glob syntax checks), `sync/apply.rs` (glob/copy
  engine), `workspace/superset_files.rs` (`.superset/` I/O), `sync/repo_scan.rs` (working-tree
  scan), `git/gitignore.rs` (`.gitignore` helpers), `sync/merge.rs` (the
  push/pull/merge decision model and per-hunk merge assembly), `tui/diffmodel.rs`
  (the diff-to-rows model powering the cockpit's diff pane), `hashing.rs`
  (FNV-1a + SHA-256), and effectively all of `src/plugin/` – the plugin's parse
  (`plugin/mod.rs`), its state modules, its hook handlers, and the whole
  `plugin/checklist/` family (schema, canonical ordering, validator, renderer,
  verbs) are unit-tested with no terminal and no harness involved.
- Interactive/side-effecting: `tui/menu.rs`, `tui/ui.rs` (`inquire` wrappers),
  `tui/style.rs` (palette), the finishing-action prompts in `workspace/migrate.rs` /
  `sync/reverse_sync.rs`, `tui/cockpit.rs` (the full-screen reverse-sync merge
  cockpit — its event loop and terminal lifecycle are manual-smoke like the
  rest of this list, but its render path (`draw`) and key dispatch
  (`handle_key`) are unit-tested via `ratatui::backend::TestBackend`, so a
  regression there IS expected to be caught by `cargo test`).
- `main.rs` composes: `cli::parse` → `tui::style::init` (SKIPPED for
  `Parsed::Plugin`, which makes the color decision itself so a hook can force
  color off) → [auto-update gate for `Bare`/`Sync`/`ReverseSync`/`Pack`] →
  `dispatch`. `Parsed::Version` answers before any dispatch; `Parsed::Plugin`
  is handled in a SIBLING arm of the update gate, never inside it.

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
  the match list alone.** `src/sync/mod.rs` owns the ONE rule: `EXCLUDED_TREES`
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

## The Claude Code Plugin (`src/plugin/`, `plugin/`, `scripts/`)

`ss-magic plugin <VERB>` is a second program inside the same binary. It runs
inside a developer's Claude Code session, writes into a state tree beside the
secrets the rest of the tool moves, and is reached by an automated caller that
cannot see its errors – so review it with the same suspicion as the sync engine.

### Two callers, two opposite postures

- **`plugin hook <event>` serves the harness.** The envelope arrives on stdin
  and the ONLY thing allowed on stdout is the JSON response, so a stray
  `println!`, a progress line, or an ANSI escape corrupts it. `plugin::run`
  calls `style::init_no_color()` for a hook invocation precisely for this – flag
  a change that initializes color unconditionally, or that prints to stdout from
  a handler instead of returning a `Response`. Handler diagnostics go to stderr
  through `HookContext::diagnostic`, flushed after dispatch.
- **A named verb (`status`, `checklist`, …) serves a person or a skill**, and
  reports on stderr with a non-zero exit like any CLI.
- **Only human verbs may write configuration.** `enable`, `disable` and `config
  set` are the write path, and nothing reachable from a hook may call one –
  otherwise a repository could arrange its own enablement by getting a hook to
  fire. `HumanVerb::writes_config` marks them. Flag any hook handler that
  reaches a config write, or a new config-writing path added to `hook/`.
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
  `scratchpad::ensure_state_ignored` is the ONE place that gitignore rule is
  written, called eagerly from init/migrate and lazily from `plugin
  enable`/`config set plugin.enabled true`, NEVER from a hook. The check uses
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

### Exactly-once claims must not be built on `unlink`

**Never treat a successful delete as having won a claim.** Measured on this
repo's own code: 8 threads racing to `unlink` one path produced up to 5
successes across 20 trials. Sequential testing shows exactly the `ENOENT` you
expect, which is what makes it dangerous – the one-shot bypass token ("exactly
the next gated Read") was built on it and would have admitted every concurrent
read that raced it.

`plugin/claim.rs::take` is the single correct primitive: create a private
landing file in the SAME directory (so the rename never crosses a filesystem)
and `fs::rename` the claim onto it. `rename` requires its source to exist, so
exactly one caller wins. Flag any new one-shot/exactly-once store built on
`remove_file(...).is_ok()`, and flag an exclusivity test that only calls the
claim twice in a row – that proves the state machine, not the exclusion, and a
correct test must race N threads and assert exactly one winner.

### Parse-sensitive git output must bypass the trimming helper

`git()` and `git_optional()` in `src/git/mod.rs` `.trim()` the whole output.
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

### The operator checklist is CLI-write-only

The `PreToolUse` handler denies a direct `Read`/`Edit`/`Write`/`NotebookEdit` of
a checklist file, so `plugin/checklist/verbs.rs` is the ONLY write path – that
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
- **Any change under `plugin/` must re-pin AND bump the version.** Run `python3
  scripts/build-plugin-zip.py --update-manifest` then `--check`. The resolved
  VERSION, not the digest, is the client's update signal – changing the zip and
  its `sha256` without a version bump leaves every installed user silently on
  the cached copy. Flag a `plugin/` change with a stale digest or an unbumped
  version.

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
- `plugin/bin/ss-magic-plugin` is the wrapper skills invoke. It must keep
  injecting the `plugin` verb (so a skill can never reach bare `ss-magic`, its
  update gate, or its TUI) and must keep its distinct name (a wrapper called
  `ss-magic` would resolve non-deterministically against a user's own install).
  A missing binary is a normal state – it exits 0 with one stderr line. No skill
  body may name `${CLAUDE_PLUGIN_DATA}` or a bare `ss-magic`; CI asserts this.

### `ss-magic plugin` never self-updates and never opens the TUI

`main.rs` handles `Parsed::Plugin` in a sibling arm of the auto-update gate, and
`should_run_update_gate` is an INCLUSION list over `Command` – `plugin` is not a
`Command` at all. The binary is pinned alongside the skills, hooks and Markdown
the marketplace ships with it, so a silent mid-session swap would leave the two
describing different behavior. Flag a `plugin` arm added to
`should_run_update_gate`, an inversion of it to an exclusion list, or any
interactive-menu construction reachable from a plugin verb.

### The shipped manifest declares FIVE hook events

`plugin/hooks/hooks.json` registers `SessionStart`, `PreToolUse`, `PreCompact`,
`SubagentStop`, and `SessionEnd` – and no `FileChanged` entry. `HookEvent`
parses a `file-changed` token and `hook/mod.rs::route()` still has an arm for it,
so `src/plugin/hook/file_changed.rs` is reachable by argv and stays covered by
tests, but **nothing in a real session invokes it**. Do not describe it as a
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

## Self-Update Safety (`update/`)

- The daily-cached "latest release" check (`update/check.rs`) uses `ureq` with
  an ETag and a short timeout, and must fall through SILENTLY on any offline /
  non-200 / timeout result — a failed update check must never block or slow a
  normal invocation. Flag an update-check change that surfaces a hard error or
  removes the timeout.
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

- All colored output goes through `tui/style.rs` (gray info, bold green ok, bold
  orange warn, bold red err, bold cyan header). The color decision (NO_COLOR +
  supports-color) is captured once in a `OnceLock<bool>`. Flag raw ANSI escape
  codes emitted outside `tui/style.rs`, or output that ignores the NO_COLOR
  decision.
- Interactive prompts must be inert on Esc / Ctrl-C (leave the tree untouched
  and exit success) — `tui/menu.rs` and the pickers follow this. Flag an
  interactive path where cancellation mutates the filesystem.
- A `plugin hook` invocation owns stdout for its JSON envelope: color is forced
  off there and nothing but the envelope may be printed. Flag a `println!` added
  to a hook handler, or a style init that ignores the hook case.

## Version Bump Discipline (REQUIRED)

The binary self-updates from GitHub Releases keyed on the crate version, so a
stale version means users never receive the change. **Any change that alters
CLI behavior — a fix, a new/changed command or flag, or different output —
MUST bump `version` in `Cargo.toml` AND the matching `ss-magic` entry in
`Cargo.lock`.** Bug fixes bump patch; new/changed user-visible behavior bumps
minor (pre-1.0). Flag a behavior-changing PR that does not bump both
`Cargo.toml` and `Cargo.lock`, or that bumps only one of the two.

**A change under `plugin/` bumps FOUR version surfaces, not one**, and re-pins
the digest: `Cargo.toml`, `plugin/.claude-plugin/plugin.json`,
`plugin/ss-magic.version`, and the release-asset URL in
`.claude-plugin/marketplace.json` must all agree, and the `sha256` there must
match the rebuilt zip. `python3 scripts/build-plugin-zip.py --check` asserts all
of it; `--update-manifest` re-pins. The resolved VERSION, not the digest, is the
client's update signal, so a content change without a version bump leaves every
installed user silently on the cached copy. Flag a `plugin/` change with any
surface out of step, or with a stale digest.

## Test Requirements

- **`cargo test` is not the whole suite.** Three non-Rust suites cover code it
  cannot reach, and CI runs all of them:
  `python3 scripts/build-plugin-zip.py --selftest` (the zip builder's
  reproducibility guarantees and its refusals),
  `python3 scripts/build-plugin-zip.py --check` (the release assertions: the
  marketplace `sha256` key exists, the four version surfaces agree, the
  committed digest matches the tree), and
  `/bin/bash scripts/test-bootstrap.sh` (the bootstrap's failure paths –
  offline, corrupted download, hostile pin, unwritable data dir, unsupported
  platform, concurrent sessions – each asserting exit 0, empty stdout, and an
  untouched pre-existing binary). Flag a change to `plugin/`, `scripts/`, or the
  release assertions that leaves these unrun or unmentioned.
- Tests use `tempfile` for scratch trees and shell-invoked `git init` /
  `git worktree add` for git fixtures. Pure modules (`cli.rs`, `sync/pattern.rs`,
  `sync/apply.rs`, `sync/mod.rs`, `pack.rs`, `hashing.rs`,
  `workspace/superset_files.rs`, `git/mod.rs` probes, `tui/menu.rs`
  routing via `operations_for`, `sync/merge.rs`, `tui/diffmodel.rs`,
  `sync/reverse_sync.rs`'s `apply_decision`/backup/TOCTOU seam, and every module
  under `src/plugin/` – its parse, state modules, hook handlers and the whole
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
  in a dedicated child file (`<module>/tests.rs`) – including every module under
  `src/plugin/`; crate-root tests and shared
  helpers live in `src/tests/` (`sync.rs`, `reverse_sync_flow.rs`,
  `update_gate.rs`, `support.rs`).
  Flag a PR that adds an inline `mod tests { ... }` block to a source file
  instead of a sibling test file.
- CI (`.github/workflows/ci.yml`) runs `cargo test --locked` on Ubuntu and
  macOS for every PR commit, plus a `plugin package` job carrying the builder
  selftest, the release assertions, a check that a content change under
  `plugin/` came with a version bump, a check that no skill body names
  `CLAUDE_PLUGIN_DATA`, and the bootstrap failure-path suite; it gates
  cargo-dist releases via `plan-jobs` in
  `dist-workspace.toml`. Flag hand edits to the generated
  `.github/workflows/release.yml` (regenerate with the pinned `dist` version
  instead) and flag `allow-dirty = ["ci"]` additions.
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
vocabulary), and `CLAUDE.md`
(architecture/conventions) must reflect the current state after every
implementation change — a new command, flag, module, or changed behavior. Flag
a behavior- or architecture-changing PR that leaves any of them describing the
old state (e.g. a new subcommand or plugin verb not listed in the README command
inventory or the
`main.rs`/`cli.rs` descriptions, a changed build/test/release workflow not
reflected in `CONTRIBUTING.md`, or a new module absent from the `CLAUDE.md`
architecture list). The README's command inventory must match `cli.rs`'s `parse`
and `plugin/mod.rs`'s `HumanVerb`/`HookEvent`, and the documented hook events
must match what `plugin/hooks/hooks.json` actually registers – flag a doc that
claims an event the manifest does not declare.
This `.cursor/BUGBOT.md` must likewise be re-synchronised whenever the
conventions above change.
