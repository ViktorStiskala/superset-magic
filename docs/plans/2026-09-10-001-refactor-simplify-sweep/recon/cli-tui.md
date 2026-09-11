# Recon: cli-tui (cockpit, diffmodel, menu, theme, ui)

## Map

### `crates/ss-magic/src/tui/mod.rs` (11 lines)
Module wiring for the CLI's interactive layer; re-exports core's `style`.

- `pub(crate) mod cockpit / diffmodel / menu / theme / ui` – declares the five sibling modules.
- `pub(crate) use ss_magic_core::style;` – re-export so `crate::tui::style::…` keeps resolving after the workspace split (pinned in `structure-pins.md`, not a duplication).

### `crates/ss-magic/src/tui/theme.rs` (55 lines)
Installs the global `inquire` `RenderConfig` matching the palette's color decision.

- `install()` (pub) – `fn() -> ()` – calls `inquire::set_global_render_config(render_config(style::enabled()))`.
- `render_config(enabled: bool)` (pub) – `-> RenderConfig<'static>` – builds the themed config (cyan/green/gray/red) or `RenderConfig::empty()`.
- Local color bindings `cyan`/`green`/`dim_gray`/`red` (lines 30-33) are `let` bindings, not consts – small, inline, single-use.

### `crates/ss-magic/src/tui/menu.rs` (218 lines)
Bare-invocation operation menu: location-aware routing to Migrate/Init/EditConfig/Sync/Pack.

- `enum MenuOp` (pub) – `{Migrate, Init, EditConfig, Sync, Pack}` – the offerable operations.
- `impl fmt::Display for MenuOp` (private impl, lines 58-69) – label text.
- `enum Location` (pub) – `{Main, Worktree}` – context for `operations_for`.
- `operations_for(location: Location, branch: Branch) -> Vec<MenuOp>` (pub) – pure routing table (see truth table in doc comment).
- `run(cwd: &Path) -> Result<ExitCode>` (pub) – resolves root/location, builds the op list, dispatches.
- `dispatch_menu<F>(ops: Vec<MenuOp>, handler: F) -> Result<ExitCode>` (private) – shared `Select` + cancel-is-inert driver. Small, generic-shaped (`FnMut(MenuOp) -> Result<ExitCode>`) – candidate to compare against `ui.rs`'s `pick_with_actions` (also a "render a Select loop, dispatch on the answer" driver, different shape).
- `edit_config(repo_root: &Path, existing: Option<&Config>) -> Result<ExitCode>` (private) – thin delegate to `migrate::run_init`.

### `crates/ss-magic/src/tui/ui.rs` (286 lines)
Thin `inquire` wrappers: the bootstrap pattern picker, the final-action picker, and the shared action-loop driver behind both in-tree pickers.

- `enum FinalAction` (pub) – `{CommitPushMain, FeatureBranchPR, Done}`.
- `impl fmt::Display for FinalAction` (private, lines 28-37).
- `struct Row` (private) – `{raw: String, checked: bool, dim_suffix: Option<&'static str>}` – one picker row.
- `enum Action` (private) – `{Toggle{idx,label}, AddNew{label}, Done}` – one action-loop menu entry.
- `impl fmt::Display for Action` (private, lines 64-72).
- `render_row(row: &Row) -> String` (private) – `"[x] raw  warn(suffix)"` formatting. Small, generic-shaped helper – single call site (`pick_with_actions`).
- `struct PickerStrings` (private) – static text bundle for one action-loop picker (prompt/help/add-row/add-prompt/cancel-context strings). Only one instance exists in-tree (`pick_patterns`'s `STRINGS` const); doc comment says "both in-tree pickers construct one of these", but `pick_final_action` does NOT use `PickerStrings`/`pick_with_actions` at all (plain `Select`) – see Leads.
- `pick_with_actions<V, D>(strings, rows, validator, dim_for_new_row) -> Result<Vec<String>>` (private) – the shared toggle/add/done loop driver. Generic over validator + dim closures; only call site is `pick_patterns`.
- `prompt_one_custom_entry<V>(prompt_label, prompt_help, cancel_context, taken, validator) -> Result<Option<String>>` (private) – one-off `Text` sub-prompt. Small, generic-shaped helper wrapping `inquire::Text` + validator adaptation.
- `pick_patterns(options: &[String], preselected: &[usize], repo_root: &Path) -> Result<Vec<String>>` (pub) – builds rows + calls `pick_with_actions`.
- `validate_pattern(pattern_str: &str, taken: &[String]) -> Result<(), String>` (private) – wraps `pattern::check_syntax` + duplicate check.
- `pick_final_action() -> Result<FinalAction>` (pub) – plain `Select::new(...).prompt()`, no action-loop.
- `print_pattern_list(patterns: &[String])` (pub) – prints a dim bulleted list via `style::info`.

### `crates/ss-magic/src/tui/diffmodel.rs` (390 lines)
Pure diff model (no `ratatui`/TUI dependency): text classification, side-by-side and unified row builders. Consumed only by `cockpit.rs` (verified below – every `pub` item here has its one caller in `cockpit.rs`; nothing else in the workspace touches it).

- `const MAX_DIFF_BYTES: u64 = 2 * 1024 * 1024` (pub, line 24) – 2 MiB diff-size cap.
- `normalize_eol(s: &str) -> String` (pub) – CRLF→LF, trailing-CR, and trailing-newline normalization for diffing/merging.
- `enum ContentKind` (pub) – `{Text(String), Binary, TooLarge(u64)}`.
- `classify_content(bytes: &[u8]) -> ContentKind` (pub) – size cap, then NUL/UTF-8 checks.
- `struct Seg` (pub) – `{text: String, emphasized: bool}` one intra-line span.
- `enum RowTag` (pub) – `{Equal, Delete, Insert, Replace, Fold(usize)}` (side-by-side row role).
- `struct DiffRow` (pub) – one side-by-side row.
- `enum UnifiedTag` (pub) – `{Context, Delete, Insert, Fold(usize)}`.
- `struct UnifiedRow` (pub) – one unified row.
- `const MIN_SPLIT_COL: u16 = 40` (private, line 174) – minimum legible content columns per split column.
- `const SPLIT_GUTTER: u16 = 5` (pub, line 179) – per-side line-number gutter width; re-used by `cockpit.rs` for layout (`render_split`, `visible_content_width`).
- `const SPLIT_MIN_PANE_WIDTH: u16 = 2 * (MIN_SPLIT_COL + SPLIT_GUTTER) + 1` (private, line 190) – derived threshold, "+1" reserved for the divider column.
- `should_split(diff_pane_inner_width: u16) -> bool` (pub) – threshold check, wrapped again in `cockpit.rs::use_split`.
- `line_no(idx: Option<usize>) -> Option<usize>` (private) – 0-based → 1-based.
- `segs_from_strings<'s>(strings: impl Iterator<...>) -> Vec<Seg>` (private) – builds a line's `Seg`s, stripping the trailing EOL (`\n`, then a leftover `\r`) and dropping an emptied-out trailing segment.
- `fold_gap(group: &[similar::DiffOp], prev_end: usize) -> (Option<usize>, usize)` (private) – shared context-fold bookkeeping used by both `side_by_side` and `unified`.
- `side_by_side(local: &str, main: &str, context: usize) -> Vec<DiffRow>` (pub) – two-column model.
- `fold_row(n: usize) -> DiffRow` (private) – side-by-side fold-row constructor. Structurally identical in shape to `cockpit.rs::fold_line` (both build a "N unchanged lines" placeholder, one as a `DiffRow`, one as a rendered `Line`) – see Leads.
- `unified(local: &str, main: &str, context: usize) -> Vec<UnifiedRow>` (pub) – single-column model.

### `crates/ss-magic/src/tui/cockpit.rs` (1812 lines)
Full-screen `ratatui` merge cockpit: file list + live diff + per-hunk merge overlay + batched confirm. The largest file in the partition; below is grouped by section, private items called out where they look like reusable/duplicated shape.

**Constants** (lines 53-75, see also Constants section):
- `CONTEXT: usize = 3` (private, line 54)
- `H_SCROLL_STEP: u16 = 8` (private, line 57)
- `UNIFIED_GUTTER: u16 = 12` (private, line 62)
- `NEW_GUTTER: u16 = 7` (private, line 69)
- `EOL_ONLY_NOTE: &str` (private, line 73) – multi-line notice text.
- `HIGHLIGHT_SYMBOL: &str = "› "` (private, line 749)
- `HIGHLIGHT_SYMBOL_WIDTH: u16 = 2` (private, line 754)
- `LIST_BORDER_WIDTH: u16 = 2` (private, line 758)
- `FOOTER_LEGEND: &str` (private, line 1337) – the persistent key legend.

**Public surface**:
- `enum CockpitOutcome` (pub, line 80) – `{Apply(Vec<(PathBuf,Decision)>), Cancel}`.
- `is_interactive() -> bool` (pub, line 90) – stdin+stdout TTY check.
- `run_cockpit(worktree_root, main_root, offered) -> Result<CockpitOutcome>` (pub, line 1632) – terminal lifecycle + `event_loop` entry point.

**State types** (private):
- `enum FileDiff` (line 99) – `{Text{local,main}, New{content}, MainOnly{content}, Binary{note}, TooLarge{note}, Unreadable{note,side}}`.
- `enum UnreadableSide` (line 133) – `{Worktree, Main}`.
- `struct FileEntry` (line 142) – one candidate's rel/status/decision/diff/mtimes.
- `enum Mode` (line 159) – `{Normal, Help, Confirm, Merge}`.
- `struct MergeOverlay` (line 174) + `impl` (lines 187-278): `build`, `hunk_count`, `next_hunk`, `prev_hunk`, `cycle_choice`, `max_preview_scroll`, `scroll_preview_down`, `scroll_preview_up`, `diff_sides`, `focused_local`, `focused_main`, `preview`.
- `struct App` (line 282) + `impl` (lines 297-498): `new`, `focus_next`, `focus_prev`, `scroll_down`, `scroll_up`, `scroll_right`, `scroll_left`, `max_hscroll`, `max_scroll`, `set_decision`, `set_push`, `set_pull`, `try_open_merge`, `accept_merge`, `decisions`, `destructive_overwrites`.
  - Note the near-mirror-image pair `scroll_down`/`scroll_up` and `scroll_right`/`scroll_left` (lines 332-349) versus `MergeOverlay::scroll_preview_down`/`scroll_preview_up` (lines 241-249) – same saturating-add/sub-and-clamp shape duplicated 3 times across two types.

**Loading/classification helpers** (private, lines 500-743):
- `load_entry(worktree_root, main_root, rel, status) -> Result<FileEntry>` (line 501).
- `read_created_content(path: &Path, label: &str) -> Result<Option<String>>` (line 543) – shared by `build_new`/`build_main_only`.
- `build_new(wt_path: &Path) -> FileDiff` (line 562).
- `build_main_only(main_path: &Path) -> FileDiff` (line 578) – documented mirror of `build_new`.
- `build_two_sided(wt_path, main_path) -> Result<FileDiff>` (line 600).
- `main_unreadable(err: &io::Error) -> FileDiff` (line 629) / `wt_unreadable(err: &io::Error) -> FileDiff` (line 640) – near-identical constructors, mirror pair by design (per-side notice text).
- `file_state(status: DiffStatus) -> FileState` (line 647).
- `build_text_diff(wt_bytes, main_bytes) -> FileDiff` (line 657).
- `binary_note(wt_bytes, main_bytes) -> String` (line 683).
- `too_large_note(n: u64) -> String` (line 692).
- `cheap_hash(bytes: &[u8]) -> u64` (line 698) – `DefaultHasher`-based display-only fingerprint (NOT a cache key – see Leads re: `hashing.rs`'s FNV-1a rule).
- `format_mtime(path: &Path) -> String` (line 707) – hardcoded 60/3600/86400 thresholds (unnamed).
- `diff_line_count(f: &FileEntry) -> usize` (line 730).

**Pure render-support helpers** (private, lines 745-858):
- `file_list_content_width(area: Rect) -> u16` (line 764).
- `wrap_hard(s: &str, width: u16) -> Vec<String>` (line 779) – char-boundary hard-wrap; small, generic-shaped, single call site (`file_list_item`).
- `use_split(diff_pane_inner_width: u16) -> bool` (line 793) – thin wrapper over `diffmodel::should_split`.
- `badge_text(decision: &Decision, status: DiffStatus) -> (String, Color)` (line 801).
- `delete_target(status: DiffStatus) -> &'static str` (line 818) – shared by `badge_text` and `App::destructive_overwrites` (deliberately, per doc comment – not a duplication).
- `merge_preview(segments, choices) -> String` (line 829) – thin delegate to `sync::merge::assemble`.
- `next_choice(c: MergeChoice) -> MergeChoice` (line 834) / `prev_choice(c: MergeChoice) -> MergeChoice` (line 843) – forward/backward cycle, mirror pair.
- `choice_label(c: MergeChoice) -> &'static str` (line 852).

**Rendering** (private, lines 864-1626, pure – no I/O):
- `draw(frame, app)` (line 864) – top-level layout + dispatch by `Mode`.
- `render_file_list` (891), `file_list_item` (914), `status_tag` (939).
- `render_diff` (948), `max_content_width` (1019), `visible_content_width` (1032).
- `render_created` (1057, `#[allow(clippy::too_many_arguments)]`) – shared by `New`/`MainOnly`.
- `render_notice` (1106).
- `render_unified` (1124) – calls `diffmodel::unified(main, local, CONTEXT)` (note the swapped arg order vs. every other caller, documented).
- `render_split_divider` (1160), `split_thirds` (1172), `render_split` (1181).
- `render_gutter_and_content` (1225) – the fixed-gutter/scrollable-content primitive shared by `render_created` (via caller), `render_unified`, and `render_split` (via `side_columns`'s per-column call).
- `struct SideColumns` (1241) + `side_columns(rows: &[DiffRow]) -> (SideColumns, SideColumns)` (1247).
- `push_side_cell` (1284), `push_segments` (1300), `style_of` (1310).
- `fold_line(n: usize) -> Line<'static>` (1321) – see `diffmodel::fold_row` note above.
- `num(n: Option<usize>) -> String` (1329) – `"{n:>4}"` right-aligned 4-wide gutter number (or 4 spaces).
- `render_footer` (1341), `render_help` (1350), `centered_rect_abs(width, height, area) -> Rect` (1414, percent-independent centering).
- `render_confirm` (1422) – local `const CHROME_LINES: u16 = 7` (line 1432).
- `render_merge` (1484), `render_merge_header` (1536), `render_merge_side` (1555), `render_merge_hunk_list` (1573), `render_merge_preview` (1595).
- `centered_rect(percent_x, percent_y, area) -> Rect` (1613) – percentage-based centering; compare against `centered_rect_abs` (1414) – two different centering primitives, only one call site each (`render_help`/`render_confirm` use `_abs`; `render_merge` uses the percent version). See Leads.

**Event loop / lifecycle** (private except `run_cockpit`, lines 1628-1809):
- `run_cockpit` (pub, 1632).
- `event_loop<B>(terminal, app) -> Result<CockpitOutcome>` (1657).
- `handle_key(app, code, page) -> Option<CockpitOutcome>` (1691) – the whole key dispatch table, unit-tested directly.
- `restore_terminal()` (1785), `struct TerminalGuard` + `Drop` (1792-1798), `install_panic_hook()` (1803).

## Constants and literals

| Item | Value | Location | Elsewhere in workspace? | Owning definition |
|---|---|---|---|---|
| `MAX_DIFF_BYTES` | `2 * 1024 * 1024` (2 MiB) | `diffmodel.rs:24` | Not found elsewhere in `consts.md`/`numbers.md`; no other diff-size cap in the workspace. | Here – this IS the owning definition; used only by `cockpit.rs` (5 sites). |
| `MIN_SPLIT_COL` | `40` | `diffmodel.rs:174` | Not found elsewhere. | Here. |
| `SPLIT_GUTTER` | `5` | `diffmodel.rs:179` | Not found elsewhere; re-used by `cockpit.rs` (2 sites: `visible_content_width`, `render_split`'s per-column call). | Here (pub for exactly this cross-module reuse). |
| `SPLIT_MIN_PANE_WIDTH` | `2 * (MIN_SPLIT_COL + SPLIT_GUTTER) + 1` = `91` | `diffmodel.rs:190` | Not found elsewhere. | Here – derived, not literal. |
| `CONTEXT` | `3` | `cockpit.rs:54` | Not found elsewhere (`consts.md` line 37 confirms single occurrence). | Here. |
| `H_SCROLL_STEP` | `8` | `cockpit.rs:57` | Not found elsewhere. | Here. |
| `UNIFIED_GUTTER` | `12` | `cockpit.rs:62` | Not found elsewhere. Note: `12` is documented as derived from `"%4d %4d "` (2×4-digit + 2 separators = 10) `+ 2` for the `"± "` sign, but is a bare literal, not an expression like `SPLIT_MIN_PANE_WIDTH`'s. | Here. |
| `NEW_GUTTER` | `7` | `cockpit.rs:69` | Not found elsewhere. Documented as mirroring `UNIFIED_GUTTER`'s derivation (`"%4d " ` = 5, `+2` for `"+ "`) but again a bare literal. | Here. |
| `HIGHLIGHT_SYMBOL` | `"› "` | `cockpit.rs:749` | Not found elsewhere. | Here; `HIGHLIGHT_SYMBOL_WIDTH` (`2`) is a manually-kept-in-sync twin of this string's column width – a classic "two facts that must agree" pair with no compile-time link between them (see Leads). |
| `HIGHLIGHT_SYMBOL_WIDTH` | `2` | `cockpit.rs:754` | – | Here. |
| `LIST_BORDER_WIDTH` | `2` | `cockpit.rs:758` | – | Here. |
| `EOL_ONLY_NOTE` | long string, literals.md confirms single occurrence | `cockpit.rs:73` | Not found elsewhere. | Here. |
| `FOOTER_LEGEND` | `"↑↓/jk move · PgUp/PgDn/Space/b scroll · p push · l pull · m merge · d delete · u undecided · Enter apply · ? help · Esc cancel"` | `cockpit.rs:1337` | Not found elsewhere (single occurrence per `consts.md` line 90). | Here – but see Leads: the merge-overlay footer at `cockpit.rs:1529` and the help overlay body (`cockpit.rs:1352-1397`) restate several of the SAME key bindings (`p`/`l`/`m`/`d`/`u`, `PgUp`/`PgDn`/`Space`/`b`) as separate hand-written strings, not built from a shared source. |
| `CHROME_LINES` | `7` | `cockpit.rs:1432` (local `const` inside `render_confirm`) | Not found elsewhere. | Here – function-local, correctly scoped. |
| `"worktree → main"` / `"main → worktree"` / `"merged → both"` | string literals | `cockpit.rs:482,488,490` (`App::destructive_overwrites`) | **Yes** – the identical three strings appear in `crates/ss-magic/src/sync/reverse_sync.rs:493,494,495` (`run_bulk`'s summary print, matched on `WriteDirection`). Confirmed via `literals.md` lines 127,129,133. | No single owner – two independent match statements over two different enums (`Decision` here, `WriteDirection` there) that happen to describe the same three directions with the same words. See Leads (cross-partition). |
| `"delete (worktree copy)"` / `"delete (main copy)"` / `"delete (worktree + main)"` | string literals | `cockpit.rs:820,821,822` (`delete_target`) | Only in this file's own tests (`literals.md` lines 243,244,283 show `cockpit.rs` + its own `tests.rs`) – not duplicated elsewhere. | Here; already deliberately shared between `badge_text` and `App::destructive_overwrites` per the doc comment (pinned, not a finding). |
| `"Local file"` / `"Main branch"` | string literals | `cockpit.rs:1193,1200` (`render_split`) and `cockpit.rs:1514,1521` (`render_merge`, as `"Local file (keep-local)"` / `"Main branch (keep-main)"`) | Only within this file + its own tests (`literals.md` lines 224,225). | Here – the SAME two labels are hand-typed twice in one file (`render_split` and `render_merge_side`'s call site), once plain and once with a parenthetical suffix. See Leads. |
| secs thresholds `60`, `3600`, `86_400` | `format_mtime` | `cockpit.rs:715,717,719` | Confirmed single-file via `numbers.md` lines 9,10,137,138,159,160. | Here – unnamed magic numbers for a minute/hour/day boundary; display-only ("unreliable hint" per doc comment) so low severity. |
| `centered_rect(88, 90, ...)` | percentages | `cockpit.rs:1494` | Single occurrence (`numbers.md` lines 35,39 both point at the same line: the two args). | Here. |
| Layout percentages `38`/`62` | `cockpit.rs:871` | file-list vs diff-pane split | Single occurrence. | Here. |
| `render_merge`'s vertical `Constraint::Percentage` budget `24, 24, 18` | `cockpit.rs:1502-1504` | – | Single occurrence, no name. | Here. |

## Cross-module references

**What these files import from elsewhere:**
- `cockpit.rs` imports `crate::sync::merge::{assemble, default_decision, diff_count, merge_segments, Decision, FileState, MergeChoice, MergeSegment}` (defined `crates/ss-magic/src/sync/merge.rs`), `crate::sync::reverse_sync::DiffStatus` (defined `crates/ss-magic/src/sync/reverse_sync.rs:65`), and `crate::tui::diffmodel::{self, ContentKind, DiffRow, RowTag, UnifiedTag}` (sibling module, this partition).
- `menu.rs` imports `crate::git`, `crate::workspace::migrate::{self, Branch}`, `crate::sync::reverse_sync`, `crate::tui::style` (= core's `style`), `crate::workspace::superset_files`.
- `ui.rs` imports `crate::sync::pattern`, `crate::sync::repo_scan`, `crate::tui::style`.
- `theme.rs` imports `super::style` (core's `style` via `tui/mod.rs`'s re-export).
- `diffmodel.rs` imports only `std::borrow::Cow` and the `similar` crate – no crate-internal imports at all (confirms it is a pure leaf module).
- External crates: `inquire` (menu.rs, ui.rs, theme.rs), `ratatui` + `ratatui::crossterm` (cockpit.rs only), `similar` (diffmodel.rs only), `anyhow` (all but diffmodel.rs and mod.rs).

**Where public symbols from these files are used elsewhere (file:line, ≤5 each):**
- `diffmodel::{MAX_DIFF_BYTES, normalize_eol, ContentKind, classify_content, Seg, RowTag, DiffRow, UnifiedTag, UnifiedRow, SPLIT_GUTTER, should_split, side_by_side, unified}` – **unused outside `cockpit.rs`**. Every one of these is `pub` (crate-visible or wider) but the only caller in the whole workspace is `tui/cockpit.rs`. (Confirmed by grepping each symbol against `crates/ss-magic/src`, `crates/ss-magic-core/src`, `crates/ss-magic-plugin/src`.)
- `cockpit::{CockpitOutcome, is_interactive, run_cockpit}` – used by `crates/ss-magic/src/sync/reverse_sync.rs:58` (import), `:318` (`is_interactive`), `:362` (`run_cockpit`), `:363,367` (`CockpitOutcome` match arms). This is the cockpit's one caller.
- `menu::run` – used by `crates/ss-magic/src/main.rs:188` (`tui::menu::run(&cwd)`), the `Command::Bare` dispatch arm. `MenuOp`/`Location`/`operations_for` are **unused outside `menu.rs`** (only referenced from `menu.rs` itself and `menu/tests.rs`).
- `theme::install` – used by `crates/ss-magic/src/main.rs:81`. `render_config` is called only from `install` and from `theme/tests.rs` – unused elsewhere.
- `ui::{FinalAction, pick_patterns, pick_final_action, print_pattern_list}` – all four used by `crates/ss-magic/src/workspace/migrate.rs` (`pick_patterns` at line 488; `pick_final_action` at lines 349 and 503; `print_pattern_list` at lines 334 and 495; `FinalAction` imported at line 42 and matched at lines 605/612/627). `migrate.rs` is this module's one caller.

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication (cross-partition) | `crates/ss-magic/src/tui/cockpit.rs:482,488,490` vs `crates/ss-magic/src/sync/reverse_sync.rs:493,494,495` | The three direction labels `"worktree → main"` / `"main → worktree"` / `"merged → both"` are hand-typed identically in two `match` statements over two different enums (`Decision` in cockpit, `WriteDirection` in reverse_sync) describing the same three sync directions. A rename of one wording will silently desync the other. | `reverse_sync.rs:491-496` (the `match result.direction` block) against `cockpit.rs:479-497` (`destructive_overwrites`). Consider a single shared `fn direction_label(...) -> &'static str` in `sync::merge` or `sync::reverse_sync` that both consult (note `Decision` and `WriteDirection` are different types, so this needs a shared intermediate or a trait). |
| L2 | copy-paste-variant | `crates/ss-magic/src/tui/cockpit.rs:1613-1626` (`centered_rect`) vs `crates/ss-magic/src/tui/cockpit.rs:1414-1420` (`centered_rect_abs`) | Two independent "center a rect inside `area`" helpers exist side by side – one percentage-based (used once, by `render_merge`), one absolute-size-based (used twice, by `render_help`/`render_confirm`). Both are small and single/double-purpose; `centered_rect_abs` could likely absorb `centered_rect`'s one call site by converting `88%`/`90%` of the popup's target frame to an absolute width/height, or vice versa. | Nothing else in the workspace defines a rect-centering helper (grep for `centered_rect` found no hits outside `cockpit.rs`/its tests) – this is self-contained to the file. |
| L3 | duplication | `crates/ss-magic/src/tui/cockpit.rs:1124-1153` (`render_unified`) vs `crates/ss-magic/src/tui/cockpit.rs:1181-1220` (`render_split`) vs `crates/ss-magic/src/tui/cockpit.rs:1057-1104` (`render_created`) | Three renderers each build a parallel `(nums: Vec<Line>, content: Vec<Line>)` pair row-by-row and hand it to `render_gutter_and_content`/`push_side_cell` – but each re-derives its own per-row `(sign, color)` / gutter-formatting logic inline rather than sharing one "build a gutter+content pair from a generic row" helper. `push_side_cell` (1284) already factors part of this for the split view only; `render_unified` and `render_created` each inline an equivalent loop instead of reusing/generalizing it. | `push_side_cell` (`cockpit.rs:1284-1297`) as the existing partial abstraction; compare its shape against `render_unified`'s loop at `1128-1151` and `render_created`'s loop at `1079-1091` to see if one generic "gutter cell builder" parametrized by sign/color could serve all three. |
| L4 | duplication | `crates/ss-magic/src/tui/diffmodel.rs:336-344` (`fold_row`) vs `crates/ss-magic/src/tui/cockpit.rs:1321-1326` (`fold_line`) | Both build the identical "`n` unchanged lines" placeholder text (`"  ⋯ {n} unchanged lines ⋯"` vs a `DiffRow` fold variant) independently – one returns a `DiffRow`, the other a rendered `ratatui::Line` directly from an integer. Not exactly the same signature, but the "fold gap → placeholder" concept is expressed twice with slightly different wording responsibility split between the two modules. | `diffmodel::RowTag::Fold(usize)`/`UnifiedTag::Fold(usize)` (carries the count) vs `cockpit.rs::fold_line(n)` (renders the count) – check whether `fold_line` could take a `RowTag`/`UnifiedTag::Fold` directly instead of a bare `usize`, tightening the coupling that already exists implicitly. |
| L5 | efficiency / naming | `crates/ss-magic/src/tui/cockpit.rs:332-349` (`App::scroll_down/up/right/left`) vs `crates/ss-magic/src/tui/cockpit.rs:241-249` (`MergeOverlay::scroll_preview_down/up`) | The same `saturating_add(step).min(max)` / `saturating_sub(step)` clamp pattern is written out 3 separate times across 2 different structs (vertical, horizontal, and merge-preview scroll) with no shared helper – e.g. a generic `clamp_scroll(current, step, max, forward) -> u16` used by all three pairs. | Compare `App::scroll_down` (`cockpit.rs:332`), `App::scroll_right` (`cockpit.rs:340`), `MergeOverlay::scroll_preview_down` (`cockpit.rs:241`) side by side – identical shape modulo the field name and the `max_*` bound function. |
| L6 | hardcoded-value / const-location | `crates/ss-magic/src/tui/cockpit.rs:749-758` (`HIGHLIGHT_SYMBOL = "› "`, `HIGHLIGHT_SYMBOL_WIDTH = 2`) | `HIGHLIGHT_SYMBOL_WIDTH` must equal the display width of `HIGHLIGHT_SYMBOL` but is a separately-declared literal with no compile-time or runtime link between them – a future edit to the symbol string (e.g. widening it) will silently desync the width used by `file_list_content_width`. | Consider deriving the width from the string (e.g. `HIGHLIGHT_SYMBOL.chars().count() as u16`) as a `const fn`, or a single comment co-locating both – currently they are two facts 5 lines apart with only a doc comment (line 751-754) asserting they agree. |
| L7 | naming / const-location | `crates/ss-magic/src/tui/cockpit.rs:62,69` (`UNIFIED_GUTTER = 12`, `NEW_GUTTER = 7`) | Both are documented as derived expressions (`"%4d %4d " + "± "` = 10+2, `"%4d " + "+ "` = 5+2) but are written as bare integer literals rather than the const-expression style already used two lines away for `SPLIT_MIN_PANE_WIDTH` (`diffmodel.rs:190`, `2 * (MIN_SPLIT_COL + SPLIT_GUTTER) + 1`). Inconsistent style within the same partition: one file expresses a derived gutter width as an expression, the other as a bare number with a comment claiming the derivation. | `diffmodel.rs:190` (`SPLIT_MIN_PANE_WIDTH`, the expression style) vs `cockpit.rs:62,69` (the bare-literal style) – not a functional bug, a consistency/readability lead. |
| L8 | hardcoded-value | `crates/ss-magic/src/tui/cockpit.rs:715,717,719` (`format_mtime`: `60`, `3600`, `86_400`) | Unnamed minute/hour/day-in-seconds boundaries in a function explicitly marked "unreliable hint" – low severity since it's display-only, but the same three constants (`SECS_PER_MINUTE`/`SECS_PER_HOUR`/`SECS_PER_DAY`) are exactly the kind of value the workspace elsewhere names (see `numbers.md` for other `86_400`/`3600` occurrences in the workspace to check if a shared constant already exists there). | Check `numbers.md`/`consts.md` workspace-wide for an existing `SECS_PER_DAY`-style constant (e.g. `state_tree.rs`/heartbeat pruning uses "30 days") before adding a local one here. |
| L9 | duplication | `crates/ss-magic/src/tui/cockpit.rs:1193,1200` (`render_split`'s `"Local file"`/`"Main branch"`) vs `crates/ss-magic/src/tui/cockpit.rs:1514,1521` (`render_merge`'s `"Local file (keep-local)"`/`"Main branch (keep-main)"`) | The same two side labels are hand-typed twice in the same file, once bare and once with a parenthetical describing the merge choice. A shared base label (e.g. `const LOCAL_LABEL: &str = "Local file"`) composed with an optional suffix would remove the duplicate literal. | `cockpit.rs:1193` / `cockpit.rs:1200` against `cockpit.rs:1514` / `cockpit.rs:1521`. |
| L10 | dead-code / reuse (candidate, low confidence) | `crates/ss-magic/src/tui/ui.rs:82-92` (`struct PickerStrings`) and `107-178` (`pick_with_actions`) | The doc comment at `ui.rs:82-83` says "Both in-tree pickers construct one of these", but only `pick_patterns` (`ui.rs:217-253`) actually builds a `PickerStrings`/calls `pick_with_actions`; `pick_final_action` (`ui.rs:266-275`) uses a plain `Select` directly and never touches this machinery. The doc comment is stale, OR the abstraction is over-general for its single real caller – worth a second look at whether `pick_with_actions`'s generic `V`/`D` type parameters earn their complexity for one call site. Confidence 50 – the machinery might exist to serve a picker mentioned elsewhere in the repo (e.g. the plugin's checklist) via a similar-but-separate implementation; that would make it not literally unused, just not shared. | `ui.rs:217-253` (`pick_patterns`, the one real caller) vs `ui.rs:266-275` (`pick_final_action`, which the comment implies also uses it but doesn't) – also check `workspace/migrate.rs`'s other pickers (if any) for a parallel hand-rolled loop that duplicates `pick_with_actions`' shape instead of calling it. |
| L11 | naming | `crates/ss-magic/src/tui/cockpit.rs:300` (`App::new`) and `crates/ss-magic/src/tui/menu.rs:115` (`run`) | Flagged by `functions.md` as name collisions with unrelated functions elsewhere in the workspace (`ss_magic_core::release::UreqReleaseClient::new`, `checklist::schema`'s `new`s, `heartbeat::Row::new`, `hook::mod::Response::new`; and `run` collides with `main.rs`'s private `run`, `reverse_sync::run`, `sync::apply::run`, several plugin verb `run`s). All are unrelated types/contexts – this is a false-positive-shaped naming collision, not real duplication, but confirming it here saves the reviewer re-checking it. | No action needed; listed for completeness per the sweep's "reviewer judges" instruction. |
| L12 | reuse | `crates/ss-magic/src/tui/menu.rs:182-199` (`dispatch_menu`) vs `crates/ss-magic/src/tui/ui.rs:107-178` (`pick_with_actions`) | Both are "render a `Select`, dispatch on the answer, treat cancel specially" drivers used exactly once each in this partition (`menu::run` calls `dispatch_menu`; `ui::pick_patterns` calls `pick_with_actions`). Different enough in shape (menu's is a one-shot dispatch table; ui's is a stateful toggle loop) that merging them is likely NOT warranted, but worth the reviewer's eyes since both are "the generic Select-loop wrapper" pattern this codebase already has a design-pattern doc for (`docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md`, referenced from `CLAUDE.md`'s cockpit section). | `menu.rs:182-199` vs `ui.rs:107-178`; also read `docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md` (outside this partition) before proposing any merge – it documents WHY the pickers are `Select` loops rather than `MultiSelect`, which may also explain why they are not further unified. |
| L13 | efficiency (low confidence) | `crates/ss-magic/src/tui/cockpit.rs:698-703` (`cheap_hash`) | Uses `std::collections::hash_map::DefaultHasher`, which `CLAUDE.md`'s hashing conventions elsewhere in the workspace explicitly avoid for anything that must be stable ("std does not promise its output is stable across releases or processes"). Here it is explicitly a display-only fingerprint in a binary-file notice, never persisted or compared across runs, so this is very likely fine as-is – flagged only because the pattern superficially resembles the thing the workspace's own hashing rule warns against. Confidence 40; likely a non-finding once the "display hint only" framing is accepted. | `crates/ss-magic-core/src/hashing.rs` (`fnv1a_64`/`hash_file`, chosen specifically for stability) – compare doc comments to confirm `cheap_hash`'s one-shot-display use case is exempt from that rule (it is, per its own doc comment at `cockpit.rs:696-697`). |
| L14 | const-location (low confidence) | `crates/ss-magic/src/tui/cockpit.rs:1432` (`const CHROME_LINES: u16 = 7`, function-local inside `render_confirm`) | Correctly scoped as a function-local const (not a finding by itself), but listed because it is the only function-local `const` in the whole partition while every other magic-layout number in `cockpit.rs` is a module-level `const` – worth confirming this one couldn't/shouldn't also be hoisted for consistency, or that the module-level ones couldn't be pushed down. Confidence 30 – likely a non-finding; style-consistency observation only. | Module-level consts at `cockpit.rs:53-75` for contrast. |
