---
paths:
  - "crates/ss-magic/**"
---

## CLI modules (`crates/ss-magic/src/`)

- `tui/theme.rs` – installs the `inquire` prompt theme from core's color
  decision: `install()` reads `style::enabled()` and installs the matching
  global `RenderConfig` (`render_config(false)` is `RenderConfig::empty()`, so
  with color off the theme adds nothing). `main.rs` calls `style::init()` then
  `tui::theme::install()`. The palette (`style`) is core's and knows nothing
  about `inquire`; the theme lives in this crate because only this binary
  drives prompts, which is what lets the palette live in a crate that links no
  prompt library. The plugin binary never installs a theme.
- `tui/ui.rs` – `inquire` wrappers that keep the prompt strings in one place.
  `pick_with_actions` is the shared `Select`-loop driver behind
  `pick_patterns`; the shared `Row` shape carries
  `dim_suffix: Option<&'static str>` for the `(no matches)` flag.
  `pick_final_action` (the three finishing actions: commit and push to main,
  feature branch + PR, or done for now), `print_pattern_list`, and
  `validate_pattern` (delegating to core's `pattern::check_syntax`, plus a
  duplicate check) round out the module. Per-file sync decisions are not
  prompts here: they are made in the `tui/cockpit.rs` merge cockpit. See
  `docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`
  for why the pickers are `Select` loops rather than a `MultiSelect`.
- `tui/cockpit.rs` – the full-screen `ratatui` unified Sync merge cockpit
  (`crossterm` backend, the same `crossterm 0.29` as `inquire`). `run_cockpit`
  reads both versions of every offered candidate, presents a left file-list
  pane beside a live side-by-side / unified diff (via `tui/diffmodel.rs`), and
  lets the user set each file's `merge::Decision` with explicit keys (`p` push
  / `l` pull / `m` merge / `d` delete / `u` undecided) – NOTHING is
  pre-selected (every file starts `Undecided`). `Enter` opens a batched
  confirm (content-sized popup; an overwrite list too long for the frame
  truncates with an explicit "… and N more" marker, never silently) in which
  `Enter` applies and `Esc` goes back to the file list; there is no `y`/`n`
  binding, so every bound key is an explicit action. `Esc` in the file list
  cancels with nothing written. Each candidate is loaded once into a
  `FileDiff`: `Text` (EOL-normalized on both sides via
  `diffmodel::normalize_eol` at load, so hunks are content-only and a pair
  equal after normalization renders a notice that the sides differ only by
  line endings instead of an empty diff), `New` for a worktree-only file
  (created in main by a push), `MainOnly` for a main-only file (created
  locally by a pull – the mirror of `New`, sourced from main), `Binary`,
  `TooLarge` (either side over `diffmodel::MAX_DIFF_BYTES`, 2 MiB, decided
  from metadata before any full read), or `Unreadable { note, side }` when a
  side's copy fails to read (permissions / I/O, NOT missing – surfaced
  verbatim, NEVER a fabricated empty buffer; a read error on EITHER side
  degrades to `Unreadable` rather than aborting the whole cockpit load, and
  `side` (`UnreadableSide::Worktree`/`Main`) records WHICH copy failed). The
  direction gates are side-aware: `set_push` (`p`) is a no-op with a footer
  notice for a `MainOnly` file (no worktree source) or a WORKTREE-unreadable
  file (source can't be read) – but a MAIN-unreadable file can still be
  pushed; `set_pull` (`l`) is a silent no-op for a worktree-only file or a
  MAIN-unreadable file – but a WORKTREE-unreadable file CAN be pulled (main is
  readable and overwrites the local copy, the natural recovery). Merge needs
  both sides and is unavailable for any `Unreadable`. `status_tag` labels a
  `MainOnly` file `(main only)` in cyan. `m` on a DIFFERING TEXT file opens
  the per-hunk merge overlay (`Mode::Merge`, state in `App::merge`): it
  computes hunks with `merge::merge_segments`, holds one `MergeChoice` per
  `Diff` segment (default `Local`), walks them with the arrows, cycles
  keep-local / keep-main / keep-both with `←`/`→` (`h`/`l`), previews the live
  `merge::assemble` result (scrollable with `PgUp`/`PgDn`/`Space`/`b`, clamped
  to the preview and re-clamped when a choice cycle shrinks it), and on
  `Enter` sets `Decision::Merge(assembled)` (badge `⇄ merge (assembled)`);
  `Esc` cancels unchanged. Because the overlay works on the normalized text,
  accepting a merge converges both sides to LF line endings with a trailing
  newline, which is also how an EOL-only difference is reconciled. For binary
  / oversized / one-sided / unreadable files `m` is a no-op that shows a
  transient footer notice (R9: interactive merge is offered only where both
  sides are diffable text). The batched confirm lists a merge as an overwrite
  of BOTH sides, a MainOnly pull as a non-destructive CREATE (EXCLUDED from the
  overwrite list), and a delete with the sides it removes; the delete badge
  names the same sides via `delete_target` (`✗ delete (worktree copy)`
  worktree-only, `✗ delete (main copy)` main-only, else
  `✗ delete (worktree + main)`). `apply_decision` (in `sync/reverse_sync.rs`)
  writes the bytes; the cockpit returns `CockpitOutcome::{Apply, Cancel}` and
  writes NOTHING itself; `reverse_sync::run` applies the decisions.
  `is_interactive` (stdin and stdout both a TTY – R16, the requirement that
  the full-screen cockpit only ever launches on a real terminal) guards
  launch. A `Drop` guard (constructed the moment raw mode is on) + panic hook
  always restore the terminal. File-list rows WRAP the repo-relative path
  instead of clipping it: `file_list_item` renders badge + status on line 1,
  then the path hard-wrapped (`wrap_hard`) at `file_list_content_width` (pane
  border + reserved `HIGHLIGHT_SYMBOL`), then the mtime hint (labelled
  unreliable) on its own lines. The split view draws a faint DarkGray vertical
  divider (`render_split_divider`) between the Local/Main columns on both the
  title and content rows. Both split (`side_columns`) and unified
  (`render_unified`) diffs color by main = base / local = working copy – a
  local-only line or a change's local text GREEN, a main-only line or its main
  text RED; `render_unified` gets the conventional `-` red / `+` green by
  calling `diffmodel::unified(main, local, …)` (the only swapped caller,
  binding `old_no`/`new_no` to `main_no`/`local_no` at the print site so the
  visible gutter order stays local-first). The one-sided "will be created"
  view (`render_created`, shared by `New` and `MainOnly`) shows its notice –
  green "new file — will be created in main" / cyan "main only — will be
  created in this worktree" – in a fixed `Length(1)` header row (NOT the
  scrollable body), with content numbered 1-based behind the fixed
  `NEW_GUTTER` gutter, like the text-diff views' fixed gutters. The help
  overlay is sized to its content (`centered_rect_abs`, 22 lines) so the full
  help – safety facts included – fits an 80×24 terminal. Long diff lines are
  horizontally scrollable with `←`/`→` (`diff_hscroll`, reset per file,
  clamped via `max_content_width`): the content shifts under FIXED
  line-number gutters (`render_gutter_and_content`;
  `SPLIT_GUTTER`/`UNIFIED_GUTTER`/`NEW_GUTTER`), and the pane title flags
  clipped lines ("lines continue →" / "→ col N") so a change past the pane
  edge is never silently invisible. The pure `draw(frame, app)`, the pure key
  dispatch `handle_key` and the pure `merge_preview` are exercised with
  `ratatui`'s `TestBackend` and synthetic key codes, without the event loop.
- `cli.rs` – hand-rolled arg parser (no `clap`). `parse(&[String]) -> Parsed`
  selects `Command::{Bare, Sync { no_backup }, ReverseSync { no_backup }, Pack,
  Update}` from the first non-flag arg (absent → `Bare`; `sync` → forward
  copy, `reverse-sync` → reverse copy), short-circuits a `--help`/`-h` seen
  before that arg to `Parsed::Help`, skips any other flag while scanning, and
  returns `Parsed::Error(token)` for an unknown subcommand (`main.rs` prints
  usage on stderr and exits 2). `Sync` / `ReverseSync` are struct variants
  carrying `no_backup`, set by `has_no_backup` – a whole-slice scan for
  `--no-backup`/`-n` anywhere in argv (before OR after the subcommand token,
  deliberately asymmetric with the terminal `-h`/`--help` short-circuit).
  `Command` stays `Copy`/`Eq` (`bool` is both). `init [PATTERN...]` parses to
  `Parsed::Init(patterns)` (every non-flag arg after `init`, carried apart
  from the `Command` enum). There is NO `plugin` token, no `Parsed::Plugin`
  variant, and no alias or redirect: the plugin is its own binary, so
  `ss-magic plugin` is an ordinary unknown subcommand (`Parsed::Error`, exit
  2). `--version`/`-V` short-circuits to `Parsed::Version` and wins over
  everything, with ONE deliberate asymmetry against `-h`/`--help`
  (`version_requested`): the scan covers the whole argv, PAST a subcommand
  token, so `ss-magic sync --version` still prints the version rather than
  falling through to the update-gated `Bare` menu when a script shells out to
  identify the binary. Do not add a `plugin` token or a scan stop back: the
  binary has no sub-argv to protect. Pure and unit-testable without spawning
  the process.
- `tui/menu.rs` – the bare-invocation operation menu. Location-gated by the
  pure, unit-tested `operations_for`: in the main checkout,
  `migrate::detect_branch` on `config.json` picks `[Migrate]`, `[Init]` or
  `[EditConfig, Pack]` (edit config reuses `migrate::run_init`; a malformed
  `config.json` exits 1 naming the path); a worktree offers `[Sync, Pack]`,
  whose SINGLE "Sync" entry (`MenuOp::Sync`) opens the unified
  `reverse_sync::run` cockpit (push / pull / merge / delete per file, both
  directions) – there are no separate forward/reverse entries. `Pack` is
  offered wherever an initialized `magic.json` exists (any worktree, or main
  on a `Normal` branch), so it appears in both location lists. Routes
  selections to their handlers via the `Select` driver; Esc/Ctrl-C is inert
  (it prints a cancel line and exits 0).
- `workspace/migrate.rs` – detect + migrate/init branching off
  `config.json`'s `setup` (old `setup.sh` reference → migrate, which wins
  when both markers are present; `magic.sh` marker → normal; neither, or no
  `config.json` → init). The interactive `run_migrate` / `run_init` print a
  change summary, ask the finishing-action prompt, and only then stage
  renames/writes/deletes into a tempdir and materialize via `copy_into_repo`
  (Esc at the prompt leaves the old layout untouched; "Done" materializes
  without committing). `run_init_noninteractive` is the TUI-free init behind
  `ss-magic init` (writes the layout from CLI patterns merged into
  `magic.json`'s `files`, keeps an existing `magic.local.json`, no prompt, no
  git operations, not gated by auto-update). All three write paths
  (`run_migrate`, `run_init`, `run_init_noninteractive`) call
  `ensure_bootstrap_gitignores`, which applies three idempotent rules (each a
  no-op when git already ignores the path) at the closest existing
  `.gitignore` (or the git-root file): `magic.local.json` (a
  `gitignore::ensure_path_ignored` `File` rule), the tool's
  `.superset/backups/` tree (via `reverse_sync::ensure_backups_ignored`, the
  same `Dir` rule the first sync would otherwise add lazily), and the
  plugin's `.superset/.magic/` state tree (core's
  `state_tree::ensure_state_ignored`, the rule's single owner). Each is
  git-tolerant, so it degrades to a literal append in the non-git test
  tempdirs. Ignoring backups up front means a fresh `ss-magic init` protects
  the backup tree before any secret bytes are ever backed up; ignoring the
  state tree up front is what lets the plugin's hooks write state at all,
  since they refuse to write while git does not report that tree ignored.
  The same three paths then call `clear_legacy_skills_install`, the only
  write these commands make outside the repository: it removes a
  pre-marketplace `~/.claude/skills/ss-magic/` (a symlink or file at that
  path is unlinked, never followed; a directory is removed with
  `remove_dir_all`; nothing outside that exact path is touched), because the
  marketplace install shadows that copy and the harness's plugin-errors view
  keeps reporting it as a conflict. It is best-effort (a failure warns and
  init/migrate still finish), prints `Removed legacy ~/.claude/skills/ss-magic/`
  when it removed something, and is a no-op under `cfg(test)` so the unit
  suite can never delete under a developer's home; the removal logic
  (`remove_legacy_skills_dir`) is tested against a temporary home instead.
- `sync/reverse_sync.rs` – the sync engine: reconcile the configured files
  between a worktree and main, safely, in BOTH directions. Three entry points.
  `run` is the interactive unified Sync cockpit (the worktree menu's single
  "Sync" entry): it computes `compute_reconcile_set` – every overlaid-pattern
  match on EITHER root (patterns expanded against both, so a main-only file is
  seen) whose worktree and main copies are not byte-identical, with directory
  matches, unsafe paths and every `sync::EXCLUDED_TREES` tree dropped –
  classifies each via the 4-way `classify` (`WorktreeOnly` / `MainOnly` /
  `Differs` / `Identical`; `(false,false)`, vanished on both sides, hides as
  `Identical`, and a read error on a two-sided file shows as `Differs`, never
  hidden), prints an info line and exits 0 when nothing differs, refuses
  without a terminal (R16: the cockpit needs stdin and stdout to be TTYs; it
  names the two non-interactive commands and exits 2, writing nothing), hands
  the offered set to the `tui/cockpit.rs` cockpit, then applies the returned
  per-file push / pull / merge / delete decisions via
  `apply_decision(&ApplyContext, rel, &Decision, Baseline)`. `run_bulk` is the
  non-interactive `ss-magic reverse-sync` (worktree → main): bulk-push every
  git-untracked `compute_candidates` match that differs from main, no TUI,
  `source_untracked` hard-coded `true`. `backup_forward_targets` is the
  pre-copy backup pass for the forward `ss-magic sync` (main → worktree),
  backing up under `cwd`'s `.superset/backups/` every worktree file the copy
  will overwrite. Each `Candidate` carries `wt_untracked`, derived by POSITIVE
  tracked determination (`git::tracked_files`) and fail-closed (`true` for
  anything not positively-tracked) – the gate for the
  `ensure_gitignored_in_main` secret-safety step described with
  `apply_decision` in this bullet. `finish_batch(label)` is the shared batch
  tail (print recorded backups, best-effort `prune_old_backups`, print the
  applied/skipped/failed summary prefixed with the direction `label` –
  bidirectional "Sync" for `run`, one-way "Reverse sync" for `run_bulk` – and
  pick the exit code, non-zero iff a file failed);
  `backups_root_for(root, ensure_ignore)` joins the `.superset/backups` path
  under the root each flow designates – the cockpit `run` always uses the
  WORKTREE, for both sides' losing bytes (main's land under `<ts>/main/…`),
  so recovered secret bytes stay in the worktree's gitignored tree and are
  never committed; `run_bulk` uses main and the forward
  `backup_forward_targets` uses cwd, the tree each one overwrites – and, when
  `ensure_ignore` (false only under `--no-backup`), gitignores it via
  `ensure_backups_ignored` – the ONE place the `.superset/backups` ignore rule
  (a `gitignore::ensure_path_ignored` `Dir` rule) is wired, shared with the
  eager init/migrate bootstrap (`ensure_bootstrap_gitignores`) so a fresh
  `ss-magic init` adds the same rule up front rather than waiting for the
  first sync. `apply_decision` is the backup-first apply seam: a path-safety
  guard; a review-time baseline re-check via `check_target` – per-file
  `(worktree, main)` `FileMeta` is captured via `review_baseline` BEFORE the
  cockpit opens (the `Baseline` passed into `apply_decision`; `run_bulk`, with
  no review window, captures it immediately before applying) and re-compared
  at apply (`metas_match`: length + mtime, with a content-hash fallback
  captured when the filesystem reports no mtime, so a bare length never passes
  as unchanged), so a file edited/created/deleted during review is skipped,
  not clobbered. Both the side a decision READS and the side it OVERWRITES are
  guarded, since a changed source is content the user never reviewed. The
  baseline is COHERENT with the reviewed status, pinning the reviewed-ABSENT
  side to `None` SYMMETRICALLY: a `WorktreeOnly` candidate's main side and a
  `MainOnly` candidate's worktree side are both `None`, so a copy that
  materializes on that side between classify and apply is skipped instead of
  clobbered without having been listed in the confirm. `review_baseline` NEVER
  aborts the reconcile for one bad file: a side that fails to `stat` (or, on a
  mtime-less filesystem, to hash) degrades to `None` via `baseline_side`
  rather than propagating the error – one permission/I/O error on a single
  candidate must not tear down the whole session (mirroring
  `classify`/`load_entry`, which degrade the same failures to `Differs` and a
  `FileDiff::Unreadable`). Folding to `None` is fail-closed: an
  unreadable-then-present side reads as `baseline None` vs a present target →
  `Guard::Changed` → SKIP, so nothing the review could not see is overwritten;
  only a genuinely-absent target is written (a create, no prior bytes to
  lose). Both the interactive `run` capture loop and `run_bulk`'s are covered,
  since the degradation lives in `review_baseline` itself.
  `backup_if_unchanged` takes a timestamped pre-write backup of the losing
  bytes under a gitignored `.superset/backups/<YYYYmmdd-HHMMSS>/{worktree,main}/…`
  (`apply_timestamp` → the pure `format_timestamp`, UTC civil-from-days, no
  date crate), skipped when `ApplyContext.backup` is false (`--no-backup`)
  though the TOCTOU `Guard::Changed` skip is unaffected; and
  `ensure_gitignored_in_main` runs before any secret bytes land in main, but
  ONLY for an untracked source (`Baseline.source_untracked`) – a tracked file
  is already committed and must NOT gain a `.gitignore` rule. Unlike the
  git-tolerant `ensure_path_ignored` beneath it, it re-checks strictly and
  fails the write if git still does not report the path ignored. Writes are
  atomic (a temp file in the target's directory, renamed over it, keeping an
  existing target's mode). `Push` and `Merge` each carry a one-sided guard (a
  `Push` with no worktree baseline, or a `Merge` missing either side, skips
  rather than reading an absent side – defense-in-depth against an
  out-of-contract MainOnly). `Decision::Delete` unlinks BOTH sides (whichever
  exist), each backed up first and baseline-guarded like an overwrite, main
  removed before the worktree so a failure leaves the worktree candidate
  intact – no gitignore step, nothing lands in main. After each apply,
  `prune_old_backups` keeps the `BACKUP_BATCHES_KEPT` (10) newest batch dirs
  and removes older ones – best-effort (a failure warns, never fails the sync)
  and only for names matching `is_backup_batch_name` (current or legacy epoch
  shape), never foreign entries; the unreleased-0.4.0 merge layout's top-level
  `local/<epoch>`+`main/<epoch>` dirs are folded into their epoch's batch
  under the same budget, an emptied side dir is removed only when the current
  prune removed something from it, and the batch written by the current run
  is protected by name (never pruned, even under a backward clock jump).
  `ApplyContext` carries the two tree roots, the batch's shared backups
  root/timestamp, and the `backup: bool` toggle. Backup paths are printed so a
  mistaken overwrite is recoverable. `sync/merge.rs` owns the pure
  `Decision`/`FileState` (`ExistsBoth` / `WorktreeOnly` /
  `MainOnly`)/`default_decision` – which returns `Decision::Undecided` for
  EVERY state (nothing is pre-selected; the unified set includes tracked
  worktree-only files that must not push on a bare keystroke) – plus
  backup-naming (`backup_rel_path(ts, BackupSide, rel)` → `<ts>/<side>/<rel>`)
  and the per-hunk merge model (`merge_segments`, `assemble`, `diff_count`,
  `MergeSegment`, `MergeChoice`, `Decision::Merge`) driving the cockpit's
  merge overlay. The excluded-trees predicate the reconcile set and pack share
  is `sync::under_excluded_tree` (core's `sync/mod.rs`, described in the core
  map, `.claude/rules/architecture-core.md`), so neither a recovered secret
  under `.superset/backups/` nor the plugin's `.superset/.magic/` state is
  ever re-offered or archived. `tui/diffmodel.rs` owns the pure diff-to-rows
  model plus `normalize_eol` (CRLF → LF, a trailing lone CR treated as an
  EOL, and a trailing newline ensured; applied to diff/merge inputs at cockpit load –
  push/pull still copy raw bytes); its `RowTag`/`UnifiedTag` Delete/Insert
  naming is relative to the diff call's `(old, new)` order and carries a
  cross-reference to `tui::cockpit`'s coloring (local-only renders green,
  main-only red), and `SPLIT_MIN_PANE_WIDTH` reserves one extra column
  (`+ 1`) for the split view's vertical divider.
- `pack.rs` – `ss-magic pack`: expand the overlaid `magic.json` patterns
  against the current git repo root (via core's `sync/apply.rs` `match_paths`)
  and write the matches – repo-relative – into `ss-magic-<repo>.tar.bz2` at
  that root. `archive_file_name` derives `<repo>` from the normalized `origin`
  remote (scheme/userinfo/host stripped, segments sanitized and joined with
  `_` – identical for ssh/https/scp forms; nested GitLab groups keep all
  segments), falling back to the primary worktree basename, then `files`.
  `repo_name_stem` is the stem derivation behind it – owned by core's
  `reponame.rs` (with `stem_from_origin` and `sanitize_segment`) and
  re-exported from `pack.rs` – reused verbatim by the plugin's identity slug
  so the two can never disagree about what this repo is called. A successful
  pack emits `PackEvent::Done { out_path, count }` – `count` is UNIQUE FILE
  PATHS (the `added: HashSet<PathBuf>` of files and symlinks actually
  written), not tar entries: archived directories are not counted and two
  overlapping patterns naming the same file count once; the rendering layer
  (`main.rs::print_pack_event`) owns the summary line, the `tar -xjvf`
  extraction hint, and `copy_to_clipboard` (pbcopy/wl-copy/xclip/xsel) of the
  archive's canonical path – clipboard is deliberately outside `pack_core` so
  tests never touch the user's clipboard. Everything (config source, match
  target, archive destination) is the one `cwd_repo_root`.
  `pack_core(cwd, on_event)` mirrors `main::sync_core`'s control flow
  (resolve root → probe + load the overlaid `magic.json` through the shared
  `load_magic_or_exit` → empty guard → work) and emits a `PackEvent` stream.
  `write_archive` tars into a bzip2 stream (`bzip2` crate, pure-Rust
  `libbz2-rs-sys` backend – no C toolchain) via a `NamedTempFile` in the root,
  then persists atomically. Safety: it never packs a pack archive into itself
  – every root-level `ss-magic-*.tar.bz2` match is excluded (current derived
  name, legacy fixed name, and archives from a previous origin's name; nor a
  `.` match that resolves to the repo root); it excludes every
  `sync::EXCLUDED_TREES` tree so neither a recovered secret under
  `.superset/backups/` nor the plugin's `.superset/.magic/` state is ever
  packed – a LEAF match via the flat `sync::under_excluded_tree` retain
  filter, and a directory match that is an ANCESTOR of one (a bare `.superset`
  pattern, or a broad `**`) via `append_dir_excluding_trees`, whose recursive
  `WalkDir` `filter_entry` prunes each excluded subtree that the flat filter
  cannot catch – one directory match can sit above several at once, since
  `.superset` is the ancestor of BOTH `backups` and `.magic`; it classifies
  each match with `symlink_metadata` (no-follow) so a matched symlink –
  including one to a directory – is stored as a single symlink entry rather
  than followed (`Path::is_dir()` would follow it and archive the target
  tree), and skips special files and entries that vanished after expansion;
  and it discards the temp file without touching an existing archive when
  nothing was actually added, so a prior good backup is never replaced by an
  empty tarball. In that zero-file case `pack_core` itself prints "No packable
  files remained" and emits no `Done` event at all.
- `update/` – every-invocation self-update of the CLI line. The per-release-line
  GitHub check it builds on is core's `release.rs` (imported as
  `ss_magic_core::release`), described in the core map,
  `.claude/rules/architecture-core.md`. `update/mod.rs` has two entry points.
  `auto_update`, the gate `main.rs` runs, consults the daily-cached check and
  acts only on a `Newer` verdict: it swaps under the lock, then re-execs the
  swapped binary (resolved via `current_exe`, never `argv[0]`) with the
  original args and `SS_MAGIC_UPDATED=1`, waits for it and exits with its
  code; lock contention or a failed swap proceeds on the current binary.
  `update_command` (`ss-magic update`) skips the cache and resolves the
  newest CLI tag straight from GitHub; its testable core
  `update_command_with` decides in a fixed order before any download: no tag
  resolved → `UpdateReport::Unavailable` ("could not check", deliberately
  distinct from "already latest", and the backend is never constructed); a
  tag failing the CLI filter → `Unavailable`; not newer → `AlreadyLatest`;
  else the swap (`Updated`, or `Skipped` when another updater holds the
  lock). The force path does not re-exec: the update is the requested work.
  `update/apply.rs` does the fd-lock (non-blocking – contention skips rather
  than waits; a lock file at least 60 s old is reclaimed) / download / atomic
  swap / spawn-and-wait re-exec via the `self_update` crate, and EVERY entry
  point (`apply_update`, `apply_update_unlocked`, `run_self_update`) takes a
  mandatory `&str` tag – `target_version_tag` is always pinned, so the
  backend can never pick "latest" itself and install a plugin release over
  the CLI; a compile-time test pins the signatures. `guard_active` is the
  loop guard: `SS_MAGIC_UPDATED` (set on the re-exec'd child) or the
  documented opt-out `SS_MAGIC_NO_UPDATE`, set to anything but empty or `0`,
  skips the check. Integrity rests on TLS + cargo-dist checksums (no
  SHA-256-vs-asset-digest check – the "KTD5 conformance" notes in
  `update/apply.rs` list which swap, temp-file, timeout and digest controls
  `self_update` does and does not expose); `bin_path_in_archive` matches
  cargo-dist's `<bin>-<target>/` tarball layout, and a test pins
  `BIN_NAME == CARGO_PKG_NAME` because that name is the `<bin>` half.
- `main.rs` – composes everything: `cli::parse` → `tui::style::init` then
  `tui::theme::install` → [auto-update gate for `Bare`/`Sync`/`ReverseSync`/
  `Pack`, per `should_run_update_gate`] → `dispatch`. `Parsed::Version` prints
  `version_line()` and `Parsed::Help` prints usage, both exit 0 before any
  dispatch; `Parsed::Error` prints usage on stderr and exits 2.
  `Parsed::Init(patterns)` is handled in `run()` BEFORE the update gate, so
  one-time setup never waits on a network round-trip: it runs
  `run_init_noninteractive` at the cwd repo root, exiting 1 outside a git
  repository. `should_run_update_gate` is an INCLUSION list over `Command`
  and is false whenever `update::apply::guard_active()`; keep it an inclusion
  list so a future command does not self-update unless it is named. The
  plugin is a separate binary that links neither `self_update` nor
  `inquire`, so nothing here routes or gates it. `Bare` refuses with exit 2
  unless both stdin and stdout are TTYs (`menu_blocked_reason` names which
  end is not, and the error points at `--help`), then routes to
  `tui::menu::run`; `Sync { no_backup }` runs the non-interactive forward copy
  (`sync_core`), which runs a pre-copy backup pass
  (`reverse_sync::backup_forward_targets`) before `sync::apply::run` unless
  `--no-backup`; `ReverseSync { no_backup }` runs `run_reverse_sync_flow`,
  which hard-errors (exit 1) from the main checkout (nothing to push) and
  otherwise bulk-pushes via `reverse_sync::run_bulk`; `Pack` runs
  `pack::pack_core` (`run_pack_flow` + `print_pack_event`); `Update` forces a
  self-update (`update_flow`, which exits 0 on every report).
  `resolve_sync_roots` resolves the cwd + main-checkout roots shared by the
  forward and reverse flows, and `load_magic_or_exit` is the one
  `.superset/magic.json` probe-and-load (absent or malformed → styled error,
  exit 1) shared by `sync_core` (against the main checkout) and `pack_core`
  (against the cwd repo root). `print_event` renders the `sync::apply::Event`
  stream.
