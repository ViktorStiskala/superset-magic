# ss-magic

Interactive Rust CLI for the Superset workspace contract (standalone repo:
`ViktorStiskala/superset-magic`; binary: `ss-magic`). See README.md for
user-facing docs.

## Build

```
make build     # cargo build --release
make install   # cargo install --path .
make clean     # cargo clean
```

Rust toolchain is provided by `rustup` (cargo on `~/.cargo/bin`).

Release binaries are published to GitHub Releases via cargo-dist
(`dist-workspace.toml`); the binary self-updates from there. The per-target
release archives are attested (cargo-dist `github-attestations` →
`actions/attest` in `build-local-artifacts`, Sigstore/Rekor provenance;
user-facing verification via `gh attestation verify` — see README). The
self-update path is unchanged and still trusts TLS + cargo-dist checksums,
not attestations. Note the attesting build job necessarily runs third-party
build scripts with `id-token: write` live — inherent to the feature; the
default (build-local) phase is deliberate because it signs same-job build
output before artifacts transit Actions storage, and changing the phase is a
security decision. End-user install
instructions (the installer script and prebuilt-binary download) live in
README.md; from-source builds and the rest of the contributor docs (tests,
PR expectations, release/versioning) live in CONTRIBUTING.md.

The Claude Code plugin ships on the SAME release. `plugin/` is the packaged
marketplace tree (`.claude-plugin/plugin.json`, `hooks/hooks.json`,
`hooks/bootstrap.sh`, `bin/ss-magic-plugin`, `lib/tmproot.sh`, `skills/`,
`ss-magic.version`); `scripts/build-plugin-zip.py` packs it byte-reproducibly
(sorted entries, fixed 1980-01-01 timestamps, normalized modes, STORED not
deflated, `create_system` forced to unix, `.DS_Store` excluded, symlinks and
non-ASCII names refused loudly), and `.claude-plugin/marketplace.json` pins the
resulting zip by SHA-256. Four version surfaces must agree – `Cargo.toml`,
`plugin/.claude-plugin/plugin.json`, `plugin/ss-magic.version`, and the release
URL in `marketplace.json`. Verify with `python3 scripts/build-plugin-zip.py
--check`; after any change under `plugin/`, re-pin with `--update-manifest`
then re-run `--check`. `.gitattributes` marks `plugin/**` as `-text` so a
checkout's line-ending conversion can never move the digest.

## Architecture

Layered to keep the pure logic unit-testable in isolation from the
interactive layer. Source is grouped by purpose: `git/` (git plumbing),
`sync/` (the sync engine), `tui/` (interactive layer), `workspace/`
(`.superset` contract I/O + lifecycle), `update/` (self-update), `plugin/`
(the Claude Code plugin verb tree – its own section below), with `main.rs`,
`cli.rs`, `pack.rs` and `hashing.rs` at the root:

- `git/mod.rs` — read-only probes (`is_worktree`, `main_checkout_root`,
  `cwd_repo_root`, `main_branch_name`, `origin_url` (backs pack's
  repo-derived archive naming), plus the reverse-sync probes
  `untracked_files` (`git ls-files --others` – untracked *including*
  gitignored, since reverse sync pushes gitignored secrets), `tracked_files`
  (`git ls-files --cached -z` – the mirror of `untracked_files` that does
  POSITIVE tracked determination for the secret push gate: a path NOT in this
  set is treated as an untracked secret, so an unenumerable name fails closed),
  `is_ignored`, `is_ignored_str` (the raw-pathname variant so a caller can force
  git's directory-only match with a trailing slash), `is_ignored_no_index_str`
  (the `--no-index` variant that asks whether the IGNORE RULES cover a path,
  ignoring the index – what the plugin's state-tree gate needs, since a tracked
  file inside the tree must not read as "the tree is unignored"),
  `status_porcelain` (parsed `(status, path)` pairs behind the checklist commit
  nudge – built on `git_raw`, NEVER the trimming `git` helper, because porcelain's
  leading column is a literal space for a worktree-only modification and a blanket
  `.trim()` shifts every field), `symbolic_ref_head` / `short_head_sha` (the branch
  name and abbreviated SHA behind the plugin's `<repo>-<branch>` identity slug);
  `parse_ls_files_z` is the shared NUL-split behind BOTH `untracked_files` and
  `tracked_files`, defensively dropping any absolute / `..`-bearing entry in one
  place) and mutating primitives (`stage_paths`, `commit`, `push`,
  `push_upstream`, `create_branch`, `pr_create`, `timestamp_branch_suffix`,
  `gh_available`). All `git`/`gh` invocations shell out via a shared `git_raw`
  helper that surfaces stderr verbatim; `git` and `git_optional` are thin
  one-liners on top. (The bare location-auto `probe`/`Mode` dispatch was removed
  in U13 – routing is now the menu via `is_worktree` + `main_checkout_root`.)
- `workspace/superset_files.rs` — `.superset/{config.json, magic.sh, magic.json,
  magic.local.json}` I/O (plus the legacy `setup_config.json` reader).
  `load_config` reads Superset-owned `config.json`;
  `merge_setup_into_config` builds a new `Config` from a new `setup`
  array while preserving `teardown` and `run` from disk;
  `write_config_json` always rewrites pretty-printed. `load_overlaid`
  reads `magic.json` and overlays `magic.local.json` (union+dedupe
  `files`, base order first); `load_magic_json` / `load_magic_local_json` read
  each layer on its own (what the plugin's config write path needs, since it
  load-modify-writes exactly one file). `write_magic_json(root, &MagicConfig)`
  and `write_magic_local_json` take the whole typed config – `MagicConfig`
  carries a `#[serde(flatten)] extras` map, so an unknown key a newer build or a
  hand edit put in the file survives a rewrite instead of being dropped;
  `merge_files_into_magic_config` folds a new `files` list into an existing
  config for the same reason. `write_magic_sh`,
  `bootstrap_magic_local_json`, and `default_magic_files` round out the
  init/migration writers. `load_setup_config` / `SetupConfig` survive as
  a READ-ONLY legacy path: migration reads the old `setup_config.json`
  `files` to carry them into `magic.json`. `existing_unknown_entries`
  preserves user-typed patterns across re-runs. `copy_into_repo`
  materializes the staged `.superset/` tree atomically (files always
  overwritten — preservation happens upstream of the write; `*.sh` are
  chmod 0755'd; a `delete` set strips the retired `setup.sh`).
- `sync/mod.rs` – the sync engine's root, and the home of the ONE
  excluded-trees rule every enumeration layer applies. `EXCLUDED_TREES` lists
  four whole directory trees no walk may ever yield, each as its exact sequence
  of path components: `.superset/backups` (the tool's own copies of overwritten
  bytes – recovered secrets), `.superset/.magic` (the plugin's gitignored,
  machine-local state), `.scratchpad` (a tree ss-magic does not own but must
  never push into the shared main checkout), and `.git`.
  `under_excluded_tree(rel)` answers "is this rel one of them, or inside one",
  via the component-by-component `starts_with_components` – NEVER a string
  prefix or a bare name, so a sibling `.superset/.magicked/` stays includable, a
  root-level `.magic` file is untouched, and `.superset` ITSELF is never
  excluded (widening the rule would drop the contract files `config.json`,
  `magic.sh` and `magic.json` out of sync and pack entirely). Returning `true`
  for a tree root is what lets a `WalkDir::filter_entry` caller prune the whole
  subtree. It is applied at every point of FINAL enumeration –
  `apply::walk_source`, `apply::copy_dir_recursive(root, src, dst)` (which takes
  the tree root precisely so it can classify each entry's rel), reverse sync's
  candidate computation, and pack's `append_dir_excluding_trees` – never only on
  an upstream match list. Distinct from `apply::DEFAULT_EXCLUDES`, which drops a
  match containing one of a few directory NAMES (`node_modules`, `.venv`) at ANY
  depth. (This generalizes the retired `reverse_sync::under_backups_dir`.)
- `sync/repo_scan.rs` — `matches_for_patterns(root, &[&str])` walks the
  working tree once with a multi-pattern `GlobSet` and returns a bool
  vector aligned to the input. `pattern_matches_any` is the single-
  pattern shortcut used when the user adds a custom pattern in the
  bootstrap picker.
- `sync/pattern.rs` — shared syntax checks for both the apply/sync
  expansion path and the picker UI validator: `has_glob_meta`,
  `has_parent_segment`, `SyntaxError`, `check_syntax`. One source of
  truth for "is this pattern structurally valid".
- `sync/apply.rs` — the glob/exclude/copy engine reused by forward `sync`
  (and, via `match_paths`, by reverse sync). Delegates syntax checks to
  `pattern::check_syntax`. Emits an `Event` stream via a caller-supplied
  closure so tests can collect events while production prints them.
  (`load_main_config`, the old interactive apply path, was removed in
  U13.)
- `tui/style.rs` — palette (gray info, bold green ok, bold orange/xterm 208
  warn, bold red err, bold cyan header). One `OnceLock<bool>` captures
  the color decision (NO_COLOR + supports-color). `inquire`'s global
  `RenderConfig` is installed from the same palette.
- `tui/ui.rs` — `inquire` wrappers. `pick_with_actions` is the shared
  `Select`-loop driver behind `pick_patterns`; the shared `Row` shape
  carries `dim_suffix: Option<&'static str>` for the `(no matches)`
  flag. `pick_final_action`, `print_pattern_list`, and `validate_pattern`
  (delegating to `pattern::check_syntax`) round out the module. (The
  setup-command picker/validator and the `.envrc`/apply confirms were
  removed in U13; the reverse-sync picker + overwrite-confirm were
  replaced by the `tui/cockpit.rs` merge cockpit.) See
  `docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`
  for why the pickers are `Select` loops rather than a `MultiSelect`.
- `tui/cockpit.rs` — the full-screen `ratatui` unified-Sync merge cockpit
  (`crossterm` backend, same `crossterm 0.29` as `inquire`). `run_cockpit` reads
  both versions of every offered candidate, presents a left file-list pane
  beside a live side-by-side / unified diff (via `tui/diffmodel.rs`), and lets
  the user set each file's `merge::Decision` with explicit keys (`p` push / `l`
  pull / `m` merge / `d` delete / `u` undecided) – NOTHING is pre-selected
  (every file starts `Undecided`) – gated by a batched confirm (content-sized
  popup; an overwrite list too long for the frame truncates with an explicit
  "… and N more" marker, never silently). Each candidate is loaded once into a
  `FileDiff`: `Text` (EOL-normalized on both sides via
  `diffmodel::normalize_eol` at load, so hunks are content-only and a pair equal
  after normalization renders a "line endings only" notice instead of an empty
  diff), `New` for a worktree-only file (created in main by a push), `MainOnly`
  for a main-only file (created locally by a pull – the mirror of `New`, sourced
  from main), `Binary`, `TooLarge`, or `Unreadable { note, side }` when a side's
  copy fails to read (permissions / I/O, NOT missing — surfaced verbatim, NEVER a
  fabricated empty buffer; a read error on EITHER side degrades to `Unreadable`
  rather than aborting the whole reconcile / cockpit load, and `side`
  (`UnreadableSide::Worktree`/`Main`) records WHICH copy failed). The direction
  gates are side-aware: `set_push` (`p`) is a no-op for a `MainOnly` file (no
  worktree source) or a WORKTREE-unreadable file (source can't be read) — but a
  MAIN-unreadable file can still be pushed; `set_pull` (`l`) is a no-op for a
  worktree-only file or a MAIN-unreadable file — but a WORKTREE-unreadable file
  CAN be pulled (main is readable and overwrites the local copy, the natural
  recovery). Merge needs both sides and is unavailable for any `Unreadable`.
  `status_tag` labels a `MainOnly` file `(main only)` in cyan. `m` on a DIFFERING TEXT file
  opens the per-hunk merge overlay (`Mode::Merge`, state in `App::merge`): it
  computes hunks with `merge::merge_segments`, holds one `MergeChoice` per `Diff`
  segment (default `Local`), walks them with the arrows, cycles keep-local /
  keep-main / keep-both with `←`/`→` (`h`/`l`), previews the live
  `merge::assemble` result (scrollable with `PgUp`/`PgDn`/`Space`/`b`, clamped to
  the preview and re-clamped when a choice cycle shrinks it), and on `Enter` sets
  `Decision::Merge(assembled)` (badge `⇄ merge (assembled)`); `Esc` cancels
  unchanged. For binary / oversized / one-sided files `m` is a no-op that shows a
  transient footer notice (R9). The batched confirm lists a merge as an overwrite
  of BOTH sides, a MainOnly pull as a non-destructive CREATE (EXCLUDED from the
  overwrite list), and a delete with the sides it removes; the delete badge names
  the same sides via `delete_target` (`✗ delete (worktree copy)` worktree-only,
  `✗ delete (main copy)` main-only, else `✗ delete (worktree + main)`).
  `apply_decision` (in `sync/reverse_sync.rs`) writes the bytes; the cockpit
  returns `CockpitOutcome::{Apply, Cancel}` and writes NOTHING itself;
  `reverse_sync::run` applies the decisions. `is_interactive` (stdin+stdout TTY,
  R16) guards launch. A `Drop` guard + panic hook always restore the terminal.
  This run made four TUI changes: (1) file-list rows WRAP the repo-relative path
  instead of clipping it – `file_list_item` renders badge + status on line 1,
  then the path hard-wrapped (`wrap_hard`) at `file_list_content_width` (pane
  border + reserved `HIGHLIGHT_SYMBOL`), then the mtime hint on its own lines;
  (2) the split view draws a faint DarkGray vertical divider
  (`render_split_divider`) between the Local/Main columns on both the title and
  content rows, and both split (`side_columns`) and unified (`render_unified`)
  diffs color by main = base / local = working copy – a local-only line or a
  change's local text GREEN, a main-only line or its main text RED –
  `render_unified` achieving the conventional `-` red / `+` green by calling
  `diffmodel::unified(main, local, …)` (the only swapped caller, renaming
  `old_no`/`new_no` to `main_no`/`local_no` at the print site so the visible
  gutter order stays local-first); (3) the batched confirm now uses
  Enter = apply / Esc = back (the `y`/`n` bindings were dropped in
  `render_confirm` / `handle_key`); (4) the one-sided "will be created" view
  (`render_created`, shared by `New` and `MainOnly`) shows its notice – green
  "new file — will be created in main" / cyan "main only — will be created in
  this worktree" – in a fixed `Length(1)` header row (NOT the scrollable body),
  with content numbered 1-based behind the same fixed `NEW_GUTTER` gutter the
  text-diff views use. The help overlay is sized to its content
  (`centered_rect_abs`, 22 lines) so the full help – safety facts included –
  fits an 80×24 terminal. Long diff lines are horizontally scrollable with
  `←`/`→` (`diff_hscroll`, reset per file, clamped via `max_content_width`): the
  content shifts under FIXED line-number gutters (`render_gutter_and_content`;
  `SPLIT_GUTTER`/`UNIFIED_GUTTER`/`NEW_GUTTER`), and the pane title flags clipped
  lines ("lines continue →" / "→ col N") so a change past the pane edge is never
  silently invisible. The pure `draw(frame, app)` and the pure `merge_preview`
  are exercised with `ratatui`'s `TestBackend` without the event loop.
- `cli.rs` — hand-rolled arg parser (no `clap`). `parse(&[String]) -> Parsed`
  selects `Command::{Bare, Sync { no_backup }, ReverseSync { no_backup }, Pack,
  Update}` from the first non-flag arg (absent → `Bare`; `sync` → forward copy,
  `reverse-sync` → reverse copy), short-circuits `--help`/`-h` to `Parsed::Help`,
  and returns `Parsed::Error(token)` for an unknown subcommand. `Sync` /
  `ReverseSync` are struct variants carrying `no_backup`, set by `has_no_backup`
  – a whole-slice scan for `--no-backup`/`-n` anywhere in argv (before OR after
  the subcommand token, deliberately asymmetric with the terminal `-h`/`--help`
  short-circuit). `Command` stays `Copy`/`Eq` (`bool` is both). `init
  [PATTERN...]` parses to `Parsed::Init(patterns)` (carried apart from the
  `Command` enum). `plugin [ARGS...]` parses to `Parsed::Plugin(args)` – the
  remaining argv is carried VERBATIM, flags included (unlike `Init`, which
  filters them), because the plugin verbs take their own `--json` / `--local` /
  `--set`. `--version`/`-V` short-circuits to `Parsed::Version` and wins over
  everything, with two deliberate asymmetries against `-h`/`--help`
  (`version_requested`): the scan runs PAST a subcommand token, so
  `ss-magic sync --version` still prints the version rather than falling through
  to the update-gated `Bare` menu when a hook shells out to identify the binary;
  and it STOPS at the `PLUGIN_TOKEN`, because a `-V` after `plugin` may be a
  verb's own flag. Pure and unit-testable without spawning the process.
- `tui/menu.rs` — bare-invocation operation menu. Location-gated: main
  checkout offers init / migrate / edit config; a worktree offers a SINGLE
  "Sync" entry (`MenuOp::Sync`) that opens the unified `reverse_sync::run`
  cockpit (push / pull / merge / delete per file, both directions) – the
  separate forward/reverse menu entries and the old `forward_sync_in_worktree`
  handler are gone. `Pack` is offered wherever an initialized `magic.json`
  exists (any worktree, or main on a `Normal` branch), so it appears in both
  location lists. Routes selections to their handlers via the `Select` driver;
  Esc/Ctrl-C is inert.
- `workspace/migrate.rs` — detect + migrate/init branching off `config.json`'s
  `setup` (old `setup.sh` reference → migrate; `magic.sh` marker →
  normal; neither → init). Stages renames/writes/deletes into a tempdir
  and materializes via `copy_into_repo` only after the finishing-action
  prompt. `run_init_noninteractive` is the TUI-free init behind
  `ss-magic init` (writes the layout from CLI patterns, no prompt, not
  gated by auto-update). All three write paths (`run_migrate`, `run_init`,
  `run_init_noninteractive`) call `ensure_bootstrap_gitignores`, which gitignores
  BOTH `magic.local.json` (a `gitignore::ensure_path_ignored` `File` rule) AND
  the tool's `.superset/backups/` tree (via `reverse_sync::ensure_backups_ignored`,
  the same `Dir` rule the first sync would otherwise add lazily) at the closest
  existing `.gitignore` (or the git-root file) – git-tolerant, so each degrades to
  a literal append in the non-git test tempdirs. Ignoring backups up front means
  a fresh `ss-magic init` protects the backup tree before any secret bytes are
  ever backed up.
- `sync/reverse_sync.rs` — the sync engine: reconcile the configured files
  between a worktree and main, safely, in BOTH directions. Three entry points.
  `run` is the interactive unified Sync cockpit (the worktree menu's single
  "Sync" entry): it computes `compute_reconcile_set` – every overlaid-pattern
  match on EITHER root (patterns expanded against both, so a main-only file is
  seen) whose worktree and main copies are not byte-identical, with directory
  matches and the tool's own `.superset/backups/` tree dropped – classifies each
  via the 4-way `classify` (`WorktreeOnly` / `MainOnly` / `Differs` /
  `Identical`; `(false,false)`, vanished on both sides, hides as `Identical`),
  refuses non-interactively (R16, exit 2), hands the offered set to the
  `tui/cockpit.rs` cockpit, then applies the returned per-file push / pull /
  merge / delete decisions via `apply_decision(&ApplyContext, rel, &Decision,
  Baseline)`. `run_bulk` is the non-interactive `ss-magic reverse-sync`
  (worktree → main): bulk-push every git-untracked `compute_candidates` match
  that differs from main, no TUI, `source_untracked` hard-coded `true`.
  `backup_forward_targets` is the pre-copy backup pass for the forward
  `ss-magic sync` (main → worktree), backing up under `cwd`'s
  `.superset/backups/` every worktree file the copy will overwrite. Each
  `Candidate` carries `wt_untracked`, derived by POSITIVE tracked determination
  (`git::tracked_files`) and fail-closed (`true` for anything not
  positively-tracked) – the gate for the secret-safety step below.
  `finish_batch(label)` is the shared batch tail (print recorded backups,
  best-effort `prune_old_backups`, print the applied/skipped/failed summary
  prefixed with the direction `label` – bidirectional "Sync" for `run`, one-way
  "Reverse sync" for `run_bulk` – and pick the exit code, non-zero iff a file
  failed); `backups_root_for(root, ensure_ignore)` joins the `.superset/backups`
  path under the root being OVERWRITTEN (cockpit `run` → worktree, `run_bulk` →
  main, forward `backup_forward_targets` → cwd) and, when `ensure_ignore`,
  gitignores it via `ensure_backups_ignored` – the ONE place the
  `.superset/backups` ignore rule (a `gitignore::ensure_path_ignored` `Dir` rule)
  is wired, shared with the eager init/migrate bootstrap
  (`ensure_bootstrap_gitignores`) so a fresh `ss-magic init` adds the same rule up
  front rather than waiting for the first sync. `apply_decision` is the backup-first apply
  seam: a path-safety guard; a review-time baseline re-check via `check_target`
  – per-file `(worktree, main)` `FileMeta` is captured via `review_baseline`
  BEFORE the cockpit opens (the `Baseline` passed into `apply_decision`) and
  re-compared at apply (`metas_match`: length + mtime, with a content-hash
  fallback captured when the filesystem reports no mtime, so a bare length never
  passes as unchanged), so a file edited/created/deleted during review is
  skipped, not clobbered. The baseline is COHERENT with the reviewed status,
  pinning the reviewed-ABSENT side to `None` SYMMETRICALLY: a `WorktreeOnly`
  candidate's main side and a `MainOnly` candidate's worktree side are both
  `None`, so a copy that materializes on that side between classify and apply is
  skipped instead of clobbered without having been listed in the confirm.
  `review_baseline` NEVER aborts the reconcile for one bad file: a side that
  fails to `stat` (or, on a mtime-less filesystem, to hash) degrades to `None`
  via `baseline_side` rather than propagating the error — one permission/I/O
  error on a single candidate must not tear down the whole session (mirroring
  `classify`/`load_entry`, which already degrade the same failures to a
  `FileDiff::Unreadable`). Folding to `None` is fail-closed: an
  unreadable-then-present side reads as `baseline None` vs a present target →
  `Guard::Changed` → SKIP, so nothing the review could not see is overwritten;
  only a genuinely-absent target is written (a create, no prior bytes to lose).
  Both the interactive `run` capture loop and `run_bulk`'s are covered, since
  the degradation lives in `review_baseline` itself.
  `backup_if_unchanged` takes a timestamped pre-write backup of the losing bytes
  under a gitignored `.superset/backups/<YYYYmmdd-HHMMSS>/{worktree,main}/…`
  (`apply_timestamp` → the pure `format_timestamp`, UTC civil-from-days, no date
  crate), skipped when `ApplyContext.backup` is false (`--no-backup`) though the
  TOCTOU `Guard::Changed` skip is unaffected; and `ensure_gitignored_in_main`
  runs before any secret bytes land in main, but ONLY for an untracked source
  (`Baseline.source_untracked`) – a tracked file is already committed and must
  NOT gain a `.gitignore` rule. `Push` and `Merge` each carry a one-sided guard
  (a `Push` with no worktree baseline, or a `Merge` missing either side, skips
  rather than reading an absent side – defense-in-depth against an
  out-of-contract MainOnly). `Decision::Delete` unlinks BOTH sides (whichever
  exist), each backed up first and baseline-guarded like an overwrite, main
  removed before the worktree so a failure leaves the worktree candidate intact
  – no gitignore step, nothing lands in main. After each apply,
  `prune_old_backups` keeps the `BACKUP_BATCHES_KEPT` (10) newest batch dirs and
  removes older ones – best-effort (a failure warns, never fails the sync) and
  only for names matching `is_backup_batch_name` (current or legacy epoch
  shape), never foreign entries; the unreleased-0.4.0 merge layout's top-level
  `local/<epoch>`+`main/<epoch>` dirs are folded into their epoch's batch under
  the same budget, an emptied side dir is removed only when this run pruned from
  it, and the batch written by the current run is protected by name (never
  pruned, even under a backward clock jump). `ApplyContext` carries the two tree
  roots, the batch's shared backups root/timestamp, and the `backup: bool`
  toggle. Backup paths are printed so a mistaken overwrite is recoverable.
  `sync/merge.rs` owns the pure `Decision`/`FileState` (`ExistsBoth` /
  `WorktreeOnly` / `MainOnly`)/`default_decision` – which now returns
  `Decision::Undecided` for EVERY state (nothing is pre-selected; the unified
  set includes tracked worktree-only files that must not push on a bare
  keystroke) – plus backup-naming (`backup_rel_path(ts, BackupSide, rel)` →
  `<ts>/<side>/<rel>`) and the per-hunk merge model (`merge_segments`,
  `assemble`, `diff_count`, `MergeSegment`, `MergeChoice`, `Decision::Merge`)
  driving the cockpit's merge overlay. The excluded-trees predicate the
  reconcile set and pack share is `sync::under_excluded_tree` (see
  `sync/mod.rs`), so neither a recovered secret under `.superset/backups/` nor
  the plugin's `.superset/.magic/` state is ever re-offered or archived. `tui/diffmodel.rs` owns the
  pure diff-to-rows model plus `normalize_eol` (CRLF → LF, a trailing lone CR
  treated as an EOL, + trailing newline ensured; applied to diff/merge inputs at
  cockpit load – push/pull still copy raw bytes); its `RowTag`/`UnifiedTag`
  Delete/Insert naming is relative to the diff call's `(old, new)` order and
  carries a cross-reference to `tui::cockpit`'s coloring (local-only renders
  green, main-only red), and `SPLIT_MIN_PANE_WIDTH` reserves one extra column
  (`+ 1`) for the split view's vertical divider.
- `pack.rs` — `ss-magic pack`: expand the overlaid `magic.json` patterns
  against the current git repo root (via `sync/apply.rs`'s `match_paths`) and
  write the matches — repo-relative — into `ss-magic-<repo>.tar.bz2` at that
  root. `archive_file_name` derives `<repo>` from the normalized `origin`
  remote (scheme/userinfo/host stripped, segments sanitized and joined with
  `_` — identical for ssh/https/scp forms; nested GitLab groups keep all
  segments), falling back to the primary worktree basename, then `files`.
  `repo_name_stem` is the extracted stem derivation behind it, reused verbatim
  by the plugin's identity slug so the two can never disagree about what this
  repo is called. A successful pack emits `PackEvent::Done { out_path, count }`
  – `count` is UNIQUE FILE PATHS (the `added: HashSet<PathBuf>` of files and
  symlinks actually written), not tar entries: archived directories are not
  counted and two overlapping patterns naming the same file count once; the
  rendering layer (`main.rs::print_pack_event`) owns the summary line, the
  `tar -xjvf` extraction hint, and `copy_to_clipboard` (pbcopy/wl-copy/
  xclip/xsel) of the archive's canonical path — clipboard is deliberately
  outside `pack_core` so tests never touch the user's clipboard.
  Everything (config source, match target, archive destination) is the
  one `cwd_repo_root`. `pack_core(cwd, on_event)` mirrors `main::sync_core`'s
  control flow (resolve root → probe magic.json → load overlaid → empty
  guard → work) and emits a `PackEvent` stream. `write_archive` tars into a
  bzip2 stream (`bzip2` crate, pure-Rust `libbz2-rs-sys` backend — no C
  toolchain) via a `NamedTempFile` in the root, then persists atomically.
  Safety: it never packs a pack archive into itself — every root-level
  `ss-magic-*.tar.bz2` match is excluded (current derived name, legacy fixed
  name, and archives from a previous origin's name; nor a `.` match that
  resolves to the repo root); it excludes every `sync::EXCLUDED_TREES` tree so
  neither a recovered secret under `.superset/backups/` nor the plugin's
  `.superset/.magic/` state is ever packed – a LEAF match via the flat
  `sync::under_excluded_tree` retain filter, and a directory match that is an
  ANCESTOR of one (a bare `.superset` pattern, or a broad `**`) via
  `append_dir_excluding_trees`, whose recursive `WalkDir` `filter_entry` prunes
  each excluded subtree that the flat filter cannot catch – one directory match
  can sit above several at once, since `.superset` is the ancestor of BOTH
  `backups` and `.magic`; it classifies each match with
  `symlink_metadata` (no-follow) so a matched symlink — including one to a
  directory — is stored as a single symlink entry rather than followed
  (`Path::is_dir()` would follow it and archive the target tree); and it
  discards the temp file without touching an existing archive when nothing was
  actually added, so a prior good backup is never replaced by an empty tarball.
- `git/gitignore.rs` — `.gitignore` helpers at a git root. `ensure_path_ignored`
  is the single entry point shared by reverse sync (the secret-safety boundary),
  the backups dir, and the migrate/init bootstrap: it ensures a `rel` of
  `PathKind::{File, Dir}` is ignored under a target root, adding a rule only when
  git does not already ignore it, landing it in the closest EXISTING `.gitignore`
  among the path's ancestors (else the target root), preferring a covering glob
  resolved from a rule-source root (verified) over an anchored literal, and
  returning `Ignored::{Already, Appended}`. It is git-TOLERANT (a git failure –
  e.g. a non-git test tempdir – reads as "not ignored" and writes the literal),
  so a hard secret boundary re-checks strictly on top of it (see
  `reverse_sync::ensure_gitignored_in_main`). `ensure_entry` (append a line iff
  no exact match exists, create the file if absent, never reorder) is now the
  building block beneath it, still called directly where the exact rule text is
  known; the private `find_covering_rule` resolves the rule covering a path via
  `git check-ignore -v` (negations excluded), returning a typed
  `CoveringRule { pattern, source_dir }` – the source dir matters because a
  pattern is only meaningful relative to the `.gitignore` it came from, so the
  caller can verify the rule actually covers the path before copying it into the
  target root; `parse_covering_line` is its parser. The private `is_ignored_opt`
  (trailing-slash query for `Dir`), `closest_gitignore_dir`, and
  `anchored_literal` back `ensure_path_ignored`.
- `update/` — every-invocation self-update: `check.rs` does the
  daily-cached GitHub latest check (ureq, ETag, 5 s timeout, silent
  fall-through); `update/apply.rs` does the fd-lock / download / atomic swap /
  spawn-and-wait re-exec via the `self_update` crate. Integrity rests on
  TLS + cargo-dist checksums (no SHA-256-vs-asset-digest check — see the
  KTD5 conformance notes in `update/apply.rs`); `bin_path_in_archive`
  matches cargo-dist's `<bin>-<target>/` tarball layout.
- `hashing.rs` – the crate's content-fingerprint primitives. `fnv1a_64` /
  `hash_file` are the non-cryptographic hashes behind cache keys and claim-file
  names; FNV-1a rather than `DefaultHasher` because std explicitly does NOT
  promise its output is stable across releases or processes, and a long-lived
  cache keyed on an unstable hash silently rots. `sha256` / `sha256_hex` are a
  hand-rolled FIPS 180-4 SHA-256 (pinned by literal test vectors), present for
  ONE reason: the plugin's per-machine temp-root identifier must be derived
  identically by this Rust code and by the shell bootstrap's `shasum -a 256`, so
  the algorithm has to be one every platform already implements the same way.
  (Replaces the removed `reverse_sync::hash_file`.)
- `main.rs` – composes everything: `cli::parse` → `tui::style::init` (skipped
  for `Parsed::Plugin`, which makes the color decision itself) →
  [auto-update gate for `Bare`/`Sync`/`ReverseSync`/`Pack`, per
  `should_run_update_gate`] → `dispatch`. `Parsed::Version` prints
  `version_line()` and stops before any dispatch; `Parsed::Plugin` routes to
  `plugin::run` in a SIBLING arm of the update gate, never inside it, so no
  plugin invocation can self-update or open the TUI – `should_run_update_gate`
  is deliberately an INCLUSION list over `Command`, and `plugin` is not a
  `Command` at all. `Bare` routes to `tui::menu::run`;
  `Sync { no_backup }` runs the non-interactive forward copy (`sync_core`),
  which now runs a pre-copy backup pass (`reverse_sync::backup_forward_targets`)
  before `sync::apply::run` unless `--no-backup`; `ReverseSync { no_backup }`
  runs `run_reverse_sync_flow`, which hard-errors from the main checkout
  (nothing to push) and otherwise bulk-pushes via `reverse_sync::run_bulk`;
  `Pack` runs `pack::pack_core` (`run_pack_flow` + `print_pack_event`); `Update`
  forces a self-update. `resolve_sync_roots` resolves the cwd + main-checkout
  roots shared by the forward and reverse flows. `print_event` renders the
  `sync::apply::Event` stream.

## The Claude Code plugin (`src/plugin/`)

`ss-magic plugin ...` is a second, largely independent program sharing this
crate's git, hashing and gitignore plumbing. Three facts shape every module in
the tree:

- **Two callers, two postures.** The harness invokes `plugin hook <event>`: the
  envelope arrives on stdin, the answer is JSON on stdout (so nothing else may
  be printed there), and a hook that cannot do its job exits 0 anyway. A person
  or a skill invokes a named verb (`status`, `checklist`, ...): problems go to
  stderr with a non-zero exit, the ordinary CLI contract. Keeping them apart is
  a safety boundary, not tidiness – only human verbs reach anything that writes
  configuration (`enable`, `disable`, `config set`), so a repository cannot
  arrange its own enablement by getting a hook to fire.
- **No update gate, no TUI, no install verb.** The marketplace is the only
  delivery path, and the binary is pinned alongside the skills, hooks and
  Markdown shipped with it; a mid-session self-update would leave the two
  describing different behavior.
- **Fail-open, but fail-CLOSED on anything that could leak.** A hook that
  errors, panics or times out must look exactly like a hook that decided to do
  nothing. The gates that protect secrets invert that: an unknown answer is the
  refusing answer.

### Entry point

- `plugin/mod.rs` – the second-level parse and the dispatch table, nothing else.
  `HookEvent::from_token` and `HumanVerb::from_token` are the two closed
  vocabularies; `HookEvent::{Unknown, Missing}` are VALUES rather than parse
  errors, because a manifest from a newer plugin build can name an event this
  binary never heard of and the contract for that is "exit 0, print nothing,
  record the unroutable name" – which the wrapper can only do if the name
  reaches it. `HumanVerb::writes_config` keeps the hook/human split honest.
  `parse` returns `Parsed::{Invocation, Help, MissingVerb, UnknownVerb}`; `run`
  calls `style::init_no_color()` for a hook invocation (an ANSI escape would make
  the JSON unparseable) and the ordinary `style::init()` otherwise, then
  dispatches. Note `HookEvent::FileChanged` parses and routes, but the shipped
  manifest declares no `FileChanged` entry – see the hook section.

### State: where the plugin keeps things, and why there

- `plugin/tmproot.rs` – the private, per-machine, cross-session temporary root
  for coordination that predates any repository or session context.
  `resolve_root()` is `/tmp/ss-magic-plugin/<identifier>/`, falling back to
  `$TMPDIR`; `identifier(home)` is the first 16 hex chars of SHA-256 of `$HOME`
  exactly as read, matching the shell bootstrap's `shasum -a 256` byte for byte.
  A predictable path is NOT evidence of ownership, so each managed component is
  `lstat`ed (never followed) and must be a real directory owned by this
  process's euid (raw `geteuid()`, not a shelled `id -u`) at mode exactly 0700;
  any failure makes that base entirely unusable rather than writing into a root
  someone else could control. `with_lock` BLOCKS (unlike the self-updater's
  skip-on-contention lock) because concurrent hook handlers must actually
  coordinate, not silently skip; `try_with_lock` is the non-blocking variant.
  `flock` releases on process death, so there is no stale-lock reclaim.
- `plugin/identity.rs` – the deterministic `<repo>-<branch>` slug, derived from
  git alone and never from the Superset workspace name (which can be silently
  renamed). `resolve(cwd)` returns `None` outside a git repo – there is no
  fallback identity, and the plugin simply does nothing. The repo half reuses
  `pack::repo_name_stem`; the branch half slugifies HEAD, falling back to
  `detached-<short-sha>`, and strips diacritics so a precomposed and an
  NFD-decomposed accented branch name resolve to the SAME directory.
- `plugin/scratchpad.rs` – the per-worktree state tree at `.superset/.magic/`
  (`STATE_REL`), holding `sessions/<slug>/` with the six model-owned
  `STATE_FILES` (`CONTEXT.md`, `DECISIONS.md`, `LEARNINGS.md`,
  `OPERATOR-CHECKLIST.md`, `STATUS.md`, `TASKS.md`), the `current.json` pointer,
  and the `conclusions/`, `bypass/` and `expect-artifact/` stores. Three hard
  rules: (1) **scaffold, never rewrite** – an existing state file is left
  byte-for-byte alone and only a genuinely missing one is created, via
  `create_new` so a race cannot clobber; only `current.json` is rewritten each
  run, under an fd-lock plus temp-file-then-rename so a lock-free reader never
  sees a half file. (2) **never adopt a tracked path** – POSITIVE tracked
  determination via `git::tracked_files`, so an unenumerable name fails closed
  as tracked-and-skipped. (3) **write nothing until git says the tree is
  ignored** – `ensure` refuses on both "git says no" AND "git could not be
  asked", using `git::is_ignored_no_index_str` so a tracked file inside the tree
  does not trigger a blanket refusal. Every path is containment-checked
  (`verify_contained`: an existing symlink must canonicalize inside the worktree
  root or the write is refused, checked for the `.superset` and
  `.superset/.magic` ancestors before creation). Dirs are 0700, files 0600 –
  defense in depth, NOT the sync-exclusion control, which is
  `sync::EXCLUDED_TREES`. `Refusal` and `Report` carry the outcome outward;
  `ensure_state_ignored` is the ONE place the `.superset/.magic/` gitignore rule
  is written, called eagerly from init/migrate and lazily from `plugin enable`,
  never from a hook.
- `plugin/claim.rs` – the exactly-once file claim both one-shot stores are built
  on. `take(dir, path)` creates a private landing file in the SAME directory and
  `fs::rename`s the claim onto it; since `rename` requires its source to exist,
  exactly one racing caller wins. It is deliberately NOT built on `unlink`'s
  `ENOENT` – see the write-up linked under the plugin hard rules below.
- `plugin/heartbeat.rs` – the append-only machine-level `hooks.jsonl` every hook
  invocation leaves a `Row` in (including no-ops and failures), which is what
  `plugin status` reports last-fired-at and outcome counts from. It lives under
  `directories`' DATA dir, not the cache dir (a history swept by disk cleanup
  would be worse than none) and outside any worktree, so rows outlive worktree
  deletion. `append` holds an exclusive `tmproot::with_lock` covering append AND
  prune together, since a prune rewrites the file wholesale. `prune` keeps the
  newest `ROWS_KEPT` (2000) rows and drops anything older than 30 days, but a
  row stamped in the FUTURE (a backward clock jump) is kept, not dropped; it
  fires only once the file passes `PRUNE_TRIGGER_BYTES` (256 KiB), so the common
  case costs one `stat`. A prune failure never fails an otherwise-good append.
  `write_atomically` is shared with `ledger.rs`.

### Hooks (`plugin/hook/`)

- `hook/mod.rs` – the ONE pipeline: decode stdin, gate, dispatch, encode stdout,
  append a heartbeat row, always exit 0. `run` has no code path that produces a
  non-zero exit; fail-open is structural, not incidental, and a handler panic is
  caught with `catch_unwind` so it cannot take the session down. Handlers never
  touch stdout or stderr themselves – only `HookContext::diagnostic`, flushed to
  stderr after dispatch. Two gates sit HERE, before dispatch, not in the
  handlers: `plugin.enabled` re-resolved from disk on every invocation, and –
  for any route whose `Route.writes_state` is true – a fail-closed check that
  git reports `.superset/.magic/` ignored. `route()` is the whole routing table.
- `hook/event.rs` – the pure wire format. Decoding is permissive (unknown keys
  ignored, only `cwd` required) but routing is not: the argv token picks the
  `Payload` variant, never the envelope's own `hook_event_name`. Two structural
  guarantees live in the types rather than in discipline: `PermissionDecision`
  has only a `Deny` variant (a hook can never GRANT a capability), and there is
  no `updatedInput` rewrite channel anywhere in `Response`. `PreCompact` and
  `SessionEnd` have no `Response` variant at all, so their silence is enforced
  by the compiler. `encode` emits the harness's field names –
  `hookSpecificOutput`, `hookEventName`, `additionalContext`,
  `permissionDecision`, `permissionDecisionReason`, `systemMessage`, and a
  top-level `{decision: "block", reason}` for a `SubagentStop` block.
- `hook/session_start.rs` – scaffolds the scratchpad and returns the operating
  guidance as `additionalContext`. A hard scratchpad refusal still returns a
  response, just a short "not set up yet" explanation – it never claims a state
  file exists that does not. `version_drift_notice` compares the running binary
  against the plugin root's pin and reports drift on `systemMessage`, the
  operator channel, never the model-facing one; it is best-effort and silent on
  every failure.
- `hook/pre_tool_use.rs` – three jobs on one event, in a fixed decision order.
  (1) The **checklist deny**: a Read / Edit / Write / NotebookEdit of a checklist
  file (matched by the `docs/actions/<stem>.checklist.json` convention or by the
  pointer's recorded target) is denied with instructions to use the checklist
  verbs. It deliberately does NOT suggest an Explore agent, unlike the size
  gate – dispatching an agent to read the checklist would leak it into that
  agent's context just the same. (2) The **Read gate**: a `Read` past the
  configured byte threshold (`threshold_lines * BYTES_PER_LINE`) is denied and
  routed to an Explore agent, or answered with the cached conclusion when one
  exists. It never emits an allow – only `Silent` or `Deny` – so it can never
  grant a capability, and every uncertain stat, path resolution or cache lookup
  falls through to allow. Escape hatches: a bounded `offset`/`limit` window, a
  subagent's own read, the `.superset/.magic/` state tree, non-text extensions,
  configured exemption globs, and a one-shot `bypass` claim. `GateTool::from_name`
  maps `Read`, then `Edit|MultiEdit|Write|NotebookEdit` (mutating), `Grep|Glob`
  (inert), and `Bash`. (3) The **commit nudge**: a `Bash` command whose trailing
  words are `git commit`, `git push`, or `gh pr create` – and NOT `gh pr view` /
  `list` / `diff`, which open nothing – gets `additionalContext` reminding the
  model to update the checklist, but ONLY when `git::status_porcelain` shows a
  candidate checklist untracked or edited-but-unstaged. It never sets a decision;
  the command is never blocked, and the text says so.
- `hook/pre_compact.rs` – appends one timestamped entry to a tool-owned
  `PRE-COMPACT.md` in the session dir and returns silence. That file is
  deliberately NOT one of `STATE_FILES`: those are model-owned and never
  rewritten by the tool, so this is a seventh file the model is never told to
  edit. It re-checks the tracked-path refusal for its own file, since
  `scratchpad::ensure` only guards the paths IT writes. Compaction is never
  blocked or slowed.
- `hook/subagent_stop.rs` – two independent jobs. The **artifact contract**:
  `expect_artifact::take_oldest` removes a pending declaration and, if the named
  file is missing / empty / not a file, blocks the stop once. With nothing
  declared, nothing is EVER blocked; "at most once" is guaranteed twice over, by
  the `stop_hook_active` short-circuit and by the fact that taking the record IS
  the one-shot flag. The **salvage**: an agent's assistant-message text is pulled
  from its transcript, tail-kept to `SALVAGE_BYTE_BUDGET` and written to
  `research-salvage/<ts>-<slug>.md` with `create_new`, so an earlier salvage is
  never overwritten. Salvage runs unconditionally and independently of the block
  decision, because data loss is irreversible while a block is retriable, and it
  can never fail the stop.
- `hook/session_end.rs` – the only moment the ledger row can be written, since
  the payload carries no usage data: it scans the session's transcript tree and
  appends one row. Heavily budgeted against the hook timeout (measured ~0.85 s
  cold, ~35 ms warm on a 382 MiB / 1257-file worst case, against ~1.15 s of real
  budget) using the ledger's byte-offset store. Raising the timeout is
  explicitly NOT the remedy – the CLI blocks on session exit waiting for this
  hook. It is `writes_state: false`, exempt from the ignored-tree gate, because
  the ledger is machine-level by design.
- `hook/file_changed.rs` – **present, tested, and INERT.** The shipped
  `plugin/hooks/hooks.json` declares five events – `SessionStart`, `PreToolUse`,
  `PreCompact`, `SubagentStop`, `SessionEnd` – and NO `FileChanged` entry, so
  nothing in a real session ever invokes this handler. It stays wired into
  `route()` and reachable by argv so the code stays exercised and landing the
  feature later is a manifest change rather than a rewrite. Do not describe it as
  a shipped hook. What it WOULD do: on a watched `.env`/`.envrc` write, ask
  `direnv status --json` (read-only – it never runs `direnv allow`) whether the
  user already trusts that file, and only then append the exported environment to
  the harness-supplied `$CLAUDE_ENV_FILE`, refusing if that target resolves
  inside the repo and writing nothing at all when the variable is unset.

### Human verbs

- `plugin/config.rs` – the typed `plugin` key in the overlaid `magic.json`, and
  the write path behind `enable` / `disable` / `config get` / `config set
  [--local]`. `resolve` is infallible by design: every malformed field degrades
  to a safe default and an out-of-range number CLAMPS rather than rejecting, so
  a typo can never turn the gate into something more permissive than configured.
  `enabled` is always read from the MAIN CHECKOUT's overlay regardless of cwd,
  because a worktree's own `magic.local.json` is itself a forward-sync target;
  `gate` resolves against the cwd root. Writes are load-modify-write on exactly
  one file, preserving every unknown key.
- `plugin/cache.rs` – the conclusion cache behind `conclude` / `conclusions` /
  `gc`. `identify` keys an entry on `(realpath, size, stamp)` – NEVER the read's
  offset or limit, so a conclusion about a file answers every later read of it.
  `envelope` wraps rendered content in nonce-keyed untrusted-data markers with
  the framing text placed BEFORE the quoted body; it is shared with the
  checklist renderer and the transcript salvage, because all three inject
  repository-authored text into a model's context. `prune`/`gc` are best-effort
  and never fail the caller.
- `plugin/bypass.rs` / `plugin/expect_artifact.rs` – the two one-shot stores
  built on `claim::take`. `bypass <FILE>` lets exactly the next gated Read of a
  resolved path through (`MAX_AGE_SECS` 24 h; an expired claim is still consumed
  but does NOT open the gate, so it cannot bypass indefinitely).
  `expect-artifact <FILE> [--note TEXT]` declares an output a later subagent must
  produce (6 h, shorter because it waits only for a machine-paced stop; an
  expired record is dropped rather than enforced, since blocking an unrelated
  agent hours later is worse than not enforcing). Both resolve and
  containment-check the path at DECLARE time, write records atomically at 0600,
  and inherit the scratchpad's ignore-gate refusal so a record can never appear
  as an untracked file in the working copy.
- `plugin/ledger.rs` – the machine-level `cost.jsonl` and the `cost [--here]
  [--backfill REF] [--json]` verb. One row per session id, enforced under an
  fd-lock held for the commit only (the scan runs outside it). The scan is
  incremental via a byte-offset store keyed on inode plus size, so a rotated
  transcript forces a full rescan instead of reading garbage. Two pricing rules
  matter: the harness's own `cost-state` figure is a cumulative FLOOR (take the
  max, and add table pricing for the main thread only when no harness figure
  exists, or the cost double-counts); and cache-write tokens are split 5 m
  (1.25x) versus 1 h (2x), because reading only the flat total undercounts.
  `Basis` records which of the two priced a row.
- `plugin/status.rs` – the one place that answers "why is the plugin not doing
  anything", across every silent-failure path: config disabled, harness
  registration missing or disabled, state tree not gitignored, binary not
  installed, manifest-versus-binary drift. Read-only – it never calls
  `scratchpad::ensure`, creates a store, or adds a gitignore rule – and it exits
  0 whenever a report was produced, so a script parsing `--json` never has to
  special-case an exit code. Every null JSON value carries a non-null `note`;
  `acting` is `None` rather than a guess when the harness layer is unknown.
  `DECLARED_EVENTS` lists the five events the manifest actually registers and
  deliberately excludes `file-changed`. The harness and binary probes are
  time-bounded and degrade to a note.
- `plugin/spill_index.rs` – a strictly read-only listing of the harness's own
  oversized-tool-output files for this worktree, which otherwise have
  unguessable names and no index. An empty result always carries a note
  distinguishing "nothing found" from "could not locate the directory".
- `plugin/setup_ci.rs` – writes `.github/workflows/ss-magic-checklist.yml` from
  the embedded `assets/workflow/checklist.yml`, pinning the running binary's
  version. `classify` returns `State::{Absent, Identical, PinStale, Differs}`
  and only `Differs` (a local edit) needs `--force`; `--check`/`-n` reports
  without writing. `PinStale` is proved by re-rendering the template at the
  version found in the file and requiring an exact byte match. Written 0644 –
  committed content, unlike the 0600 state tree.
- `plugin/compact_window.rs` – `compact-window --set <TOKENS>` writes an
  absolute `autoCompactWindow` into the per-machine, gitignored
  `.claude/settings.local.json`, never the tracked `.claude/settings.json`. It
  is strictly opt-in (no `--set` prints usage and does nothing), never clobbers
  an existing value, load-modify-writes so unrelated harness keys survive, and
  refuses rather than rebuilding a malformed file.

### The operator checklist (`plugin/checklist/`)

The typed document at `docs/actions/<YYYY-MM-slug>.checklist.json` and its verbs.
Layered like `cache.rs` – a pure model with the hook as one caller – and the
submodules are PRIVATE behind `checklist/mod.rs`, so no caller can bypass
canonical ordering or validation. The `PreToolUse` deny above makes these verbs
the ONLY write path.

- `checklist/schema.rs` – the `Document` / `Section` / `Item` model. Every field
  is `#[serde(default)]` so a hand-edited or partial file still parses (defects
  are the validator's job, not the parser's), and every level carries a
  `#[serde(flatten)] extras` map so a key from a newer build survives a rewrite.
  `kind` defaults to the strictest `Check`, so a missing kind never silently
  disables verification; `expected` is `Option<Option<String>>` because an
  absent key and an explicit null mean different things. `parse_iso8601`
  (Hinnant's `days_from_civil`, no date crate) requires an explicit offset and
  rejects `24:00` and leap seconds. `Timestamp` deliberately has no `Ord` –
  compare through `.instant()`, since `+02:00` can sort lexically after a `Z`
  stamp that is actually earlier.
- `checklist/order.rs` – `canonicalize` re-establishes the one arrangement a
  checklist is ever stored in on every write, so a diff shows real changes.
  Items sort by `(done, priority rank, created)` with the id as final tie-break,
  making order a pure function of content rather than of prior position; an
  unreadable timestamp sorts to the end instead of aborting the sort. Section
  order is never touched – author-declared order is render order.
- `checklist/validate.rs` – pure findings, no printing and no I/O. `Severity::Error`
  blocks `verify` and the renderer; `Warning` describes shape defects the next
  CLI write self-repairs and must NEVER fail CI.
- `checklist/render.rs` – the single `render()` behind `list`, `verify`,
  `render-md`, the commit nudge and the CI PR comment, so all five are
  byte-identical. Every field of user-authored prose goes through
  `prose_inline` / `md_link` escaping, and the whole output is wrapped in
  `cache::envelope` – checklist prose is repository-authored text that reaches a
  model's context. Timestamps render through the shared UTC formatter, never a
  local clock, so output is identical across machines and timezones.
- `checklist/verbs.rs` – `init`, `add-item`, `add-entry`, `set`, `done`, `list`,
  `verify`, `render-md`. Every mutating verb is read-modify-write over the WHOLE
  document (read, mutate one field, `canonicalize`, re-stamp `updated`, write
  back), so `extras` survive; writes are temp-file-then-rename preserving the
  existing mode. An advisory `tmproot::with_lock` spans the whole
  read-mutate-write, and spans exist-check plus write for `init`, so concurrent
  verbs cannot lose an update or duplicate a slug. Exit codes are distinct on
  purpose: 2 for "the command as typed cannot be carried out", but 1 from
  `verify` for "the document is invalid", so CI can tell them apart. The
  `.superset/.magic/checklist.json` pointer's contents are NOT trusted blindly –
  the target is validated lexically against absolute paths and `..` segments –
  and `resolve_active` falls back to the naming convention (unambiguous single
  match only) when no pointer exists.

### Non-Rust assets

`plugin/` (the packaged marketplace tree), `.claude-plugin/marketplace.json`
(the digest pin), `scripts/build-plugin-zip.py` (the reproducible builder and
the release assertions), `scripts/test-bootstrap.sh` (the bootstrap's
failure-path suite), `assets/workflow/checklist.yml` (embedded by `setup_ci.rs`),
`.gitattributes` (line-ending pinning for the digest), and
`docs/runbooks/forge-tag-and-release-protection.md` (tag/release immutability
settings a human must apply by hand – currently NOT applied).

Two shell pieces are worth knowing about, because both are load-bearing and
neither is Rust. `plugin/hooks/bootstrap.sh` installs the pinned binary into
`${CLAUDE_PLUGIN_DATA}` – never `${CLAUDE_PLUGIN_ROOT}`, which is version-scoped
and replaced wholesale on each plugin update. It has no `set -e` and every path
ends in `exit 0`, prints NOTHING on stdout on success (a SessionStart hook's
stdout enters the model's context every session, so silence is a token-budget
rule), never touches an existing binary on a failing install, and fetches the
platform release ARCHIVE directly, verifying it against that archive's published
`.sha256` before extracting. It deliberately does NOT fall back to piping
`ss-magic-installer.sh` into a shell: the release publishes `.sha256` siblings
for the archives but not for the installer script, so a piped installer would be
the one executed artifact no published digest covers. `plugin/bin/ss-magic-plugin`
is the wrapper every skill invokes; it injects the `plugin` verb (so a skill can
never reach bare `ss-magic`, its update gate or its TUI) and is named
`ss-magic-plugin` rather than `ss-magic` so it cannot resolve
non-deterministically against a user's own install. It finds the binary through
a durable handoff file under the R80 temp root, because `${CLAUDE_PLUGIN_DATA}`
is exported to hook and MCP processes but NOT to the Bash tool.

## Source of truth for magic.sh

`assets/magic.sh` is the canonical wrapper script, embedded into the
binary via `include_str!`. Migration and init write that body to
`.superset/magic.sh`. Edit `assets/magic.sh` and re-run migration/init
to propagate. (The legacy `assets/setup.sh` was deleted in U13 — the
binary is the sole file-copy implementation.)

## Conventions

- No `git2` — all git/gh interactions shell out via `std::process::Command`.
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
  access – including every module under `src/plugin/`. Crate-root tests and
  shared helpers live in `src/tests/`
  (`sync.rs`, `reverse_sync_flow.rs`, `update_gate.rs`, `support.rs`). CI
  (`.github/workflows/
  ci.yml`) runs the suite on every PR commit and gates cargo-dist releases
  via `plan-jobs` (see dist-workspace.toml).
- **`cargo test` is no longer the whole suite.** Three non-Rust suites cover
  code `cargo test` cannot reach, and CI runs all three:
  `python3 scripts/build-plugin-zip.py --selftest` (the builder's own
  reproducibility and refusal tests), `python3 scripts/build-plugin-zip.py
  --check` (the release assertions: the marketplace `sha256` key exists, the
  four version surfaces agree, the committed digest matches the tree), and
  `/bin/bash scripts/test-bootstrap.sh` (the bootstrap's failure paths –
  offline, corrupted download, hostile pin, unwritable data dir, unsupported
  platform, concurrent sessions – each asserting exit 0, empty stdout, and an
  untouched pre-existing binary; written for bash 3.2, so no associative
  arrays, no `mapfile`, no `${var^^}`).
- The plugin's packaged tree is **content-pinned**. Any change under `plugin/`
  moves the zip's digest, so it must be followed by `python3
  scripts/build-plugin-zip.py --update-manifest` and then `--check`, and by a
  version bump on all four surfaces – `Cargo.toml`,
  `plugin/.claude-plugin/plugin.json`, `plugin/ss-magic.version`, and the
  release URL in `.claude-plugin/marketplace.json`. The resolved VERSION, not
  the digest, is the harness's update signal: changing the zip and its `sha256`
  without bumping the version leaves every installed user silently on the
  cached copy.
- Always bump the crate version (`version` in `Cargo.toml`, and the
  matching `ss-magic` entry in `Cargo.lock`) on any change that alters
  CLI behavior — a fix, a new/changed command or flag, or different
  output. A change under `plugin/` bumps the other three version surfaces with
  it (see the plugin content-pin convention below). The binary self-updates from GitHub Releases keyed on version
  (see Build), so a stale version means users never receive the change.
  Bug fixes bump patch; new/changed user-visible behavior bumps minor
  (pre-1.0).
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

## Secret-safety constraints (hard rules)

The unified sync engine is the ONE path that writes untracked (secret) files into
the shared main checkout, and `pack` archives the configured files, so both are
secret-leak surfaces. Two constraints are load-bearing here: violating either is a
secret leak, not a cosmetic bug. Each is backed by a `docs/solutions/` write-up of
the real incident this run fixed.

- **Determine "is this a secret?" POSITIVELY, and fail closed.** The
  gitignore-in-main gate must fire for a git-UNTRACKED worktree source, decided by
  POSITIVE tracked determination (`!git::tracked_files(...).contains(rel)`) so that
  anything NOT positively known-tracked (a non-UTF-8 / NFD-vs-NFC / otherwise
  unenumerable name) defaults to secret and runs the gate. NEVER derive
  untracked-ness by ABSENCE from an untracked set (`untracked.contains(rel)`) — a
  lookup miss then lands on the permissive side and leaks. Rule for any security
  gate: phrase the question so the UNKNOWN answer is the SAFE one. See
  [docs/solutions/logic-errors/secret-gate-positive-tracked-determination-fail-closed.md](./docs/solutions/logic-errors/secret-gate-positive-tracked-determination-fail-closed.md).
- **Enforce a secret-excluding path filter at the point of final enumeration, not
  on an upstream list.** The excluded-trees filter (`sync::under_excluded_tree`
  over `sync::EXCLUDED_TREES` – `.superset/backups`, `.superset/.magic`,
  `.scratchpad`, `.git`) must be applied where the file set is actually
  materialized – every directory walk (`pack`'s `append_dir_excluding_trees`,
  `apply::walk_source`, `apply::copy_dir_recursive`, reverse sync's candidate
  computation) – NOT only on the
  flat match list, because a later step that re-walks the live filesystem
  (`append_dir_all`, `copy_dir_recursive`, `WalkDir`) bypasses an upstream filter.
  The trap is a directory match that is an ANCESTOR of the excluded subtree (a bare
  `.superset` pattern, a broad `**`) – and one such match can sit above SEVERAL
  excluded trees at once, since `.superset` is the ancestor of both `backups` and
  `.magic`. A comment asserting "X is never included" is a
  red flag unless the guard sits on the enumeration layer; test the directory-match
  shape, not just the leaf. (The write-up below predates the rename: it describes
  `under_backups_dir` / `append_dir_excluding_backups`, now generalized into
  `sync::under_excluded_tree` / `pack::append_dir_excluding_trees`.) See
  [docs/solutions/logic-errors/pack-backups-exclusion-must-guard-the-directory-walk.md](./docs/solutions/logic-errors/pack-backups-exclusion-must-guard-the-directory-walk.md).

## Plugin constraints (hard rules)

The plugin adds three surfaces with their own failure modes. Each rule below is
backed by a `docs/solutions/` write-up of the real incident behind it.

- **Never build "consume exactly once" on `unlink`'s error, and never validate
  an exclusivity property sequentially.** Measured here: 8 threads racing to
  `unlink` one path produced up to 5 successes across 20 trials, while
  sequential testing shows exactly the `ENOENT` you expect – which is what makes
  it dangerous. The one-shot bypass token (exactly the next gated Read) was
  built on it and would have leaked to several concurrent reads. The fix is
  `rename` onto a private landing file in the same directory
  (`plugin/claim.rs::take`), which gave exactly one winner in every trial. See
  [docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md](./docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md).
- **Parse-sensitive git output must not go through a trimming convenience
  wrapper.** The shared `git()` helper trims the whole output, which eats the
  leading space of the first `git status --porcelain` line – that column is a
  literal space when a file is modified in the worktree only – shifting every
  field and silently misreading the status. `git::status_porcelain` is written
  against `git_raw` for exactly this reason. See
  [docs/solutions/logic-errors/trimming-wrapper-corrupts-porcelain-status.md](./docs/solutions/logic-errors/trimming-wrapper-corrupts-porcelain-status.md).
- **Phrase every plugin gate so the UNKNOWN answer is the SAFE one, and never
  let a hook fail loudly.** The two postures pull in opposite directions and
  both are load-bearing: a hook that errors, panics or times out must look
  exactly like one that decided to do nothing (`hook::run` has no non-zero exit
  path, and a handler panic is caught), while the scratchpad's ignore gate, the
  tracked-path check and the tmproot ownership check all refuse on "could not
  ask" as well as on "no". Do not "simplify" either half toward the other.

## Documented Solutions

`docs/solutions/` — documented solutions to past problems (bugs, best
practices, design patterns, workflow learnings), organized by category
with YAML frontmatter (`module`, `tags`, `problem_type`, `component`).
Relevant when implementing or debugging in documented areas.

`CONCEPTS.md` (repo root) — shared domain vocabulary (the sync model:
main checkout, forward/reverse sync, sync patterns, candidates).
Relevant when orienting to the codebase or discussing domain concepts.
