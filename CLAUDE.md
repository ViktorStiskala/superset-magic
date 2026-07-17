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

## Architecture

Layered to keep the pure logic unit-testable in isolation from the
interactive layer. Source is grouped by purpose: `git/` (git plumbing),
`sync/` (the sync engine), `tui/` (interactive layer), `workspace/`
(`.superset` contract I/O + lifecycle), `update/` (self-update), with
`main.rs` and `cli.rs` at the root:

- `git/mod.rs` — read-only probes (`is_worktree`, `main_checkout_root`,
  `cwd_repo_root`, `main_branch_name`, `origin_url` (backs pack's
  repo-derived archive naming), plus the reverse-sync probes
  `untracked_files` (`git ls-files --others` – untracked *including*
  gitignored, since reverse sync pushes gitignored secrets), `tracked_files`
  (`git ls-files --cached -z` – the mirror of `untracked_files` that does
  POSITIVE tracked determination for the secret push gate: a path NOT in this
  set is treated as an untracked secret, so an unenumerable name fails closed),
  `is_ignored`, `is_ignored_str` (the raw-pathname variant so a caller can force
  git's directory-only match with a trailing slash), `check_ignore_pattern`;
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
  `files`, base order first); `write_magic_json`, `write_magic_sh`,
  `bootstrap_magic_local_json`, and `default_magic_files` are the
  init/migration writers. `load_setup_config` / `SetupConfig` survive as
  a READ-ONLY legacy path: migration reads the old `setup_config.json`
  `files` to carry them into `magic.json`. `existing_unknown_entries`
  preserves user-typed patterns across re-runs. `copy_into_repo`
  materializes the staged `.superset/` tree atomically (files always
  overwritten — preservation happens upstream of the write; `*.sh` are
  chmod 0755'd; a `delete` set strips the retired `setup.sh`).
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
  from main), `Binary`, `TooLarge`, or `Unreadable` when main's copy fails to
  read (surfaced verbatim, NEVER a fabricated empty buffer, so interactive merge
  is unavailable for it). `set_push` gates `p` off a `MainOnly` file (no
  worktree source to read – a transient footer notice instead), mirroring
  `set_pull`'s gate off a worktree-only or `Unreadable` file; `status_tag`
  labels a `MainOnly` file `(main only)` in cyan. `m` on a DIFFERING TEXT file
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
  `Command` enum). Pure and unit-testable without spawning the process.
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
  `run_init_noninteractive`) call `ensure_magic_local_ignored`, a thin wrapper
  over `gitignore::ensure_path_ignored` that gitignores `magic.local.json` at
  the closest existing `.gitignore` (or the git-root file) – git-tolerant, so it
  degrades to a literal append in the non-git test tempdirs.
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
  failed); `backups_root_for(root, ensure_ignore)` is the ONE place the
  `.superset/backups` path + its ignore rule (via
  `gitignore::ensure_path_ignored`) are wired, so backups always live under the
  root being OVERWRITTEN: cockpit `run` → worktree, `run_bulk` → main, forward
  `backup_forward_targets` → cwd. `apply_decision` is the backup-first apply
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
  driving the cockpit's merge overlay; `under_backups_dir` is shared with pack
  so an archive never captures a recovered secret. `tui/diffmodel.rs` owns the
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
  A successful pack emits `PackEvent::Done { out_path, count }`; the
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
  resolves to the repo root); it excludes the tool's own `.superset/backups/`
  tree so a recovered secret is never packed – a LEAF match via the flat
  `under_backups_dir` retain filter, and a directory match that is an ANCESTOR
  of `.superset/backups` (a bare `.superset` pattern, or a broad `**`) via
  `append_dir_excluding_backups`, whose recursive `WalkDir` prunes the backups
  subtree that the flat filter cannot catch; it classifies each match with
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
  known; `find_covering_rule` resolves the rule covering a path via
  `git check-ignore -v` (negations excluded); the private `is_ignored_opt`
  (trailing-slash query for `Dir`), `closest_gitignore_dir`, and
  `anchored_literal` back `ensure_path_ignored`.
- `update/` — every-invocation self-update: `check.rs` does the
  daily-cached GitHub latest check (ureq, ETag, 5 s timeout, silent
  fall-through); `update/apply.rs` does the fd-lock / download / atomic swap /
  spawn-and-wait re-exec via the `self_update` crate. Integrity rests on
  TLS + cargo-dist checksums (no SHA-256-vs-asset-digest check — see the
  KTD5 conformance notes in `update/apply.rs`); `bin_path_in_archive`
  matches cargo-dist's `<bin>-<target>/` tarball layout.
- `main.rs` — composes everything: `tui::style::init` → `cli::parse` →
  [auto-update gate for `Bare`/`Sync`/`ReverseSync`/`Pack`, per
  `should_run_update_gate`] → `dispatch`. `Bare` routes to `tui::menu::run`;
  `Sync { no_backup }` runs the non-interactive forward copy (`sync_core`),
  which now runs a pre-copy backup pass (`reverse_sync::backup_forward_targets`)
  before `sync::apply::run` unless `--no-backup`; `ReverseSync { no_backup }`
  runs `run_reverse_sync_flow`, which hard-errors from the main checkout
  (nothing to push) and otherwise bulk-pushes via `reverse_sync::run_bulk`;
  `Pack` runs `pack::pack_core` (`run_pack_flow` + `print_pack_event`); `Update`
  forces a self-update. `resolve_sync_roots` resolves the cwd + main-checkout
  roots shared by the forward and reverse flows. `print_event` renders the
  `sync::apply::Event` stream.

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
  access. Crate-root tests and shared helpers live in `src/tests/`
  (`sync.rs`, `reverse_sync_flow.rs`, `update_gate.rs`, `support.rs`). CI
  (`.github/workflows/
  ci.yml`) runs the suite on every PR commit and gates cargo-dist releases
  via `plan-jobs` (see dist-workspace.toml).
- Always bump the crate version (`version` in `Cargo.toml`, and the
  matching `ss-magic` entry in `Cargo.lock`) on any change that alters
  CLI behavior — a fix, a new/changed command or flag, or different
  output. The binary self-updates from GitHub Releases keyed on version
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

## Documented Solutions

`docs/solutions/` — documented solutions to past problems (bugs, best
practices, design patterns, workflow learnings), organized by category
with YAML frontmatter (`module`, `tags`, `problem_type`, `component`).
Relevant when implementing or debugging in documented areas.

`CONCEPTS.md` (repo root) — shared domain vocabulary (the sync model:
main checkout, forward/reverse sync, sync patterns, candidates).
Relevant when orienting to the codebase or discussing domain concepts.
