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
(self-update), `supports-color` (palette). No `clap` (the arg parser is
hand-rolled) and no `git2` (all git/gh is shelled out). Release binaries are
built by cargo-dist (`dist-workspace.toml`) and self-update from GitHub
Releases. Tests use `tempfile` + shell-invoked `git init` / `git worktree add`.

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
  Pack, Update }`. `sync`/`reverse-sync` read the `-n`/`--no-backup` flag via a
  full-argv scan (`has_no_backup`, position-independent – before OR after the
  subcommand), an intentional asymmetry with `-h`/`--help` (a terminal
  short-circuit recognized only BEFORE the subcommand). Flag any addition of
  `clap`/`structopt`/`argh`, command dispatch logic added outside `cli.rs`, or a
  "fix" that makes `has_no_backup` and the `--help` scan match each other.

## Architecture: Layering (pure logic vs interactive layer)

The codebase is deliberately layered so the pure logic is unit-testable in
isolation from the interactive TUI. Preserve this boundary.

- Pure/testable modules: `git/mod.rs` (probes + mutating primitives), `cli.rs`
  (arg parsing), `sync/pattern.rs` (glob syntax checks), `sync/apply.rs` (glob/copy
  engine), `workspace/superset_files.rs` (`.superset/` I/O), `sync/repo_scan.rs` (working-tree
  scan), `git/gitignore.rs` (`.gitignore` helpers), `sync/merge.rs` (the
  push/pull/merge decision model and per-hunk merge assembly), `tui/diffmodel.rs`
  (the diff-to-rows model powering the cockpit's diff pane).
- Interactive/side-effecting: `tui/menu.rs`, `tui/ui.rs` (`inquire` wrappers),
  `tui/style.rs` (palette), the finishing-action prompts in `workspace/migrate.rs` /
  `sync/reverse_sync.rs`, `tui/cockpit.rs` (the full-screen reverse-sync merge
  cockpit — its event loop and terminal lifecycle are manual-smoke like the
  rest of this list, but its render path (`draw`) and key dispatch
  (`handle_key`) are unit-tested via `ratatui::backend::TestBackend`, so a
  regression there IS expected to be caught by `cargo test`).
- `main.rs` composes: `tui::style::init` → `cli::parse` → [auto-update gate for
  `Bare`/`Sync`/`Pack`] → `dispatch`.

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
  single files; a dir would `EISDIR` in `classify`/the cockpit), and any rel under
  the tool's own `.superset/backups/` tree (`under_backups_dir`) is excluded so a
  backed-up secret is never re-offered. Flag a reconcile that scans only one root,
  surfaces a directory match, or re-offers a backup copy.
- **The review baseline pins the reviewed-absent side to None.** Before the
  cockpit opens, `review_baseline` captures each file's `(worktree, main)`
  metadata COHERENTLY with the status the user reviews: a `WorktreeOnly` file's
  main side is pinned `None`, and a `MainOnly` file's worktree side is pinned
  `None` (symmetric). So a copy that materializes on the pinned side during the
  review→apply window is seen as `Guard::Changed` and SKIPPED, never overwritten
  or deleted without having been shown in the confirm. Flag a baseline capture
  that stats the disk for a side the review classified as absent.
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
  via the single `gitignore::ensure_path_ignored(root, root, ".superset/backups",
  PathKind::Dir)` helper (a `Dir` is queried/written with a trailing slash so a
  `.superset/backups/` rule matches before the dir exists on disk). Flag a backup
  written under the wrong root, or a backups dir gitignored by a hand-rolled path
  instead of `ensure_path_ignored`.
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
  prior good backup, and never report "Packed 0 entries" as success. Flag a
  pack path that persists the temp archive when the added count is zero.
- **Pack must never archive anything under `.superset/backups/`.** A recovered
  secret copy under the tool's own backups tree must never re-enter an archive.
  Two guards enforce this, and BOTH are needed: `pack_core` drops every LEAF match
  under `.superset/backups/` from `rels` (`under_backups_dir` in the
  `rels.retain`), AND `write_archive` prunes the backups subtree from any
  ANCESTOR-directory match (a bare `.superset` pattern, or a broad `**`/`.` that
  matches the `.superset` component) via `append_dir_excluding_backups`'s guarded
  `filter_entry` walk rather than a blind `append_dir_all`. `under_backups_dir`
  needs BOTH the `.superset` and `backups` path components, so the flat retain
  filter CANNOT catch an ancestor dir – the guarded directory walk is required.
  Flag removal of either guard, or a new archive path that reaches
  `append_dir_all`/`append_dir_all`-style recursion for a directory match without
  pruning the `.superset/backups/` subtree.
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

## Version Bump Discipline (REQUIRED)

The binary self-updates from GitHub Releases keyed on the crate version, so a
stale version means users never receive the change. **Any change that alters
CLI behavior — a fix, a new/changed command or flag, or different output —
MUST bump `version` in `Cargo.toml` AND the matching `ss-magic` entry in
`Cargo.lock`.** Bug fixes bump patch; new/changed user-visible behavior bumps
minor (pre-1.0). Flag a behavior-changing PR that does not bump both
`Cargo.toml` and `Cargo.lock`, or that bumps only one of the two.

## Test Requirements

- Tests use `tempfile` for scratch trees and shell-invoked `git init` /
  `git worktree add` for git fixtures. Pure modules (`cli.rs`, `sync/pattern.rs`,
  `sync/apply.rs`, `pack.rs`, `workspace/superset_files.rs`, `git/mod.rs` probes, `tui/menu.rs`
  routing via `operations_for`, `sync/merge.rs`, `tui/diffmodel.rs`, and
  `sync/reverse_sync.rs`'s `apply_decision`/backup/TOCTOU seam) have unit
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
- Test layout: every module declares `#[cfg(test)] mod tests;` with the body
  in a dedicated child file (`<module>/tests.rs`); crate-root tests and shared
  helpers live in `src/tests/` (`sync.rs`, `update_gate.rs`, `support.rs`).
  Flag a PR that adds an inline `mod tests { ... }` block to a source file
  instead of a sibling test file.
- CI (`.github/workflows/ci.yml`) runs `cargo test --locked` on every PR
  commit and gates cargo-dist releases via `plan-jobs` in
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
builds, tests, PR expectations, release/versioning), and `CLAUDE.md`
(architecture/conventions) must reflect the current state after every
implementation change — a new command, flag, module, or changed behavior. Flag
a behavior- or architecture-changing PR that leaves any of them describing the
old state (e.g. a new subcommand not listed in the README command list or the
`main.rs`/`cli.rs` descriptions, a changed build/test/release workflow not
reflected in `CONTRIBUTING.md`, or a new module absent from the `CLAUDE.md`
architecture list).
This `.cursor/BUGBOT.md` must likewise be re-synchronised whenever the
conventions above change.
