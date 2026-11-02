# tui/ – interactive layer of the `ss-magic` CLI

Menu, `inquire` pickers, theme and the full-screen merge cockpit. Repository-wide rules live in the root [CLAUDE.md](../../../../CLAUDE.md); code rules for the TUI (the TUI section) live in [CODING_STANDARDS.md](../../../../CODING_STANDARDS.md).

None of this is linked by the plugin crate: the palette lives in core (`style.rs`, no `inquire`) precisely so the plugin links no prompt library.

## Modules

- `theme.rs` – the `inquire` half of styling. `install()` reads `style::enabled()` and installs the matching global `RenderConfig`; `render_config(false)` is `RenderConfig::empty()`, so with color off the theme adds nothing.\
  `main.rs` calls `style::init()` then `tui::theme::install()`; `tui/mod.rs` re-exports core's `style`.
- `ui.rs` – `inquire` wrappers.
  - `pick_with_actions` is the shared `Select`-loop driver behind `pick_patterns` and `pick_shared_directories`. Why `Select` loops and not a `MultiSelect`: [docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md](../../../../docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md).
  - The shared `Row` carries `suffix: Option<RowSuffix>` – `Warn` for the `(no matches)` flag, `Dim` for a shared-directory row's detail (the gitignored directories its pattern matches today: first three by name, the rest as a count).
  - `pick_shared_directories` is the Orca shared-directory picker; a pattern typed into it goes through the same `validate_pattern` as a sync pattern.
  - `pick_final_action`, `print_pattern_list` and `validate_pattern` (delegates to `pattern::check_syntax`) round out the module.
- `menu.rs` – bare-invocation operation menu, routed to handlers via the `Select` driver; Esc/Ctrl-C is inert.
  - Main checkout: init / migrate / edit config.
  - Worktree: a SINGLE "Sync" entry (`MenuOp::Sync`) opening the unified `reverse_sync::run` cockpit (push / pull / merge / delete per file, both directions).
  - `Pack` is offered wherever an initialized `magic.json` exists (any worktree, or main on a `Normal` branch), so it is in both lists.
  - `MenuOp::RefreshOrca` ("Refresh Orca files") is offered on the main checkout's `Normal` branch only and runs `crate::run_orca_flow`, the same operation as `ss-magic orca`. Init and migrate write the Orca files themselves; the worktree list is unchanged.
- `diffmodel.rs` – pure diff-to-rows model.
  - `normalize_eol` (CRLF → LF, a trailing lone CR treated as an EOL, trailing newline ensured) is applied to diff/merge inputs at cockpit load; push/pull still copy raw bytes.
  - `RowTag`/`UnifiedTag` Delete/Insert naming is relative to the diff call's `(old, new)` order (see the coloring note below).
  - `SPLIT_MIN_PANE_WIDTH` reserves one extra column (`+ 1`) for the split view's vertical divider.

## cockpit.rs – the unified-Sync merge cockpit

Full-screen `ratatui` UI on the `crossterm` backend (same `crossterm 0.29` as `inquire`). `run_cockpit` reads both versions of every offered candidate, shows a file-list pane beside a live side-by-side / unified diff, and lets the user set each file's `merge::Decision`.

- **Keys and decisions.** `p` push / `l` pull / `m` merge / `d` delete / `u` undecided. NOTHING is pre-selected: every file starts `Undecided`. Decisions are gated by a batched confirm.
- **Ownership.** The cockpit returns `CockpitOutcome::{Apply, Cancel}` and writes NOTHING itself; `reverse_sync::run` applies the decisions through `apply_decision` (see the `sync/` CLAUDE.md).
- **Launch and terminal.** `is_interactive` (stdin + stdout both TTY; the non-interactive refusal, R16, exits 2) guards launch. A `Drop` guard plus a panic hook always restore the terminal.

### Per-file load (`FileDiff`)

Each candidate is loaded once:

- `Text` – EOL-normalized on both sides via `diffmodel::normalize_eol`, so hunks are content-only; a pair equal after normalization renders a "line endings only" notice instead of an empty diff.
- `New` – worktree-only file (created in main by a push).
- `MainOnly` – main-only file (created locally by a pull; the mirror of `New`, sourced from main). `status_tag` labels it `(main only)` in cyan.
- `Binary`, `TooLarge`.
- `Unreadable { note, side }` – a side's copy failed to read (permissions / I/O, NOT missing). The error is surfaced verbatim, NEVER a fabricated empty buffer; a read error on either side degrades to this variant instead of aborting the whole reconcile / cockpit load, and `side` (`UnreadableSide::Worktree`/`Main`) records which copy failed.

### Direction gates (side-aware)

- `set_push` (`p`): no-op for a `MainOnly` file (no worktree source) or a WORKTREE-unreadable file; a MAIN-unreadable file can still be pushed.
- `set_pull` (`l`): no-op for a worktree-only file or a MAIN-unreadable file; a WORKTREE-unreadable file CAN be pulled (main is readable and overwrites the local copy – the natural recovery).
- Merge needs both sides and is unavailable for any `Unreadable` file.

### Merge overlay (`Mode::Merge`, state in `App::merge`)

- `m` on a DIFFERING TEXT file opens it. It computes hunks with `merge::merge_segments` and holds one `MergeChoice` per `Diff` segment (default `Local`).
- Arrows walk the hunks; `←`/`→` (`h`/`l`) cycle keep-local / keep-main / keep-both.
- The live `merge::assemble` result is previewed (`merge_preview`), scrollable with `PgUp`/`PgDn`/`Space`/`b`, clamped to the preview and re-clamped when a choice cycle shrinks it.
- `Enter` sets `Decision::Merge(assembled)` (badge `⇄ merge (assembled)`); `Esc` cancels unchanged.
- For binary / oversized / one-sided files `m` is a no-op that shows a transient footer notice (R9: merge is not offered where no two-sided text diff exists).

### Batched confirm

- Enter = apply, Esc = back.
- The popup is content-sized; an overwrite list too long for the frame truncates with an explicit "… and N more" marker, never silently.
- A merge is listed as an overwrite of BOTH sides. A `MainOnly` pull is a non-destructive CREATE, EXCLUDED from the overwrite list. A delete lists the sides it removes.
- The delete badge names the same sides via `delete_target`: `✗ delete (worktree copy)` worktree-only, `✗ delete (main copy)` main-only, else `✗ delete (worktree + main)`.

### Rendering

- **File-list rows wrap the repo-relative path instead of clipping it.** `file_list_item` renders badge + status on line 1, then the path hard-wrapped (`wrap_hard`) at `file_list_content_width` (pane border + reserved `HIGHLIGHT_SYMBOL`), then the mtime hint on its own lines.
- **Coloring is main = base, local = working copy.** A local-only line or a change's local text is GREEN; a main-only line or its main text is RED. The split view (`side_columns`) and unified view (`render_unified`) both follow it. `render_unified` gets the conventional `-` red / `+` green by calling `diffmodel::unified(main, local, …)` – the only caller with swapped `(old, new)` – and renames `old_no`/`new_no` to `main_no`/`local_no` at the print site so the visible gutter order stays local-first.
- **Split divider.** A faint DarkGray vertical divider (`render_split_divider`) separates the Local/Main columns on both the title and content rows.
- **One-sided "will be created" view** (`render_created`, shared by `New` and `MainOnly`): the notice (green "new file" plus "will be created in main" for `New`, cyan "main only" plus "will be created in this worktree" for `MainOnly`; the code joins the two halves with a dash) sits in a fixed `Length(1)` header row, NOT the scrollable body; content is numbered 1-based behind the same fixed `NEW_GUTTER` gutter the text-diff views use.
- **Horizontal scroll.** Long diff lines scroll with `←`/`→` (`diff_hscroll`, reset per file, clamped via `max_content_width`). Content shifts under FIXED line-number gutters (`render_gutter_and_content`; `SPLIT_GUTTER`/`UNIFIED_GUTTER`/`NEW_GUTTER`), and the pane title flags clipped lines ("lines continue →" / "→ col N") so a change past the pane edge is never silently invisible.
- **Help overlay** is sized to its content (`centered_rect_abs`, 22 lines) so the full help – safety facts included – fits an 80×24 terminal.

### Testing

The event loop and terminal lifecycle are manual-smoke. The render path (pure `draw(frame, app)`), the pure `merge_preview` and the pure key dispatch (`handle_key`) ARE unit-tested by driving `ratatui::backend::TestBackend` with synthetic key events (`cockpit/tests.rs`). The menu and pickers have no unit tests – manual smoke.
