# Simplification sweep – validation report

Every one of the 251 findings in `../findings/*.json` was checked against the live source before a verdict was given: the quoted evidence had to be present (a few lines of drift allowed), the proposal had to preserve outputs, error text, side effects and ordering exactly, it had to leave every safety check and every settled decision in `../brief/structure-pins.md` alone, and it had to be worth a reader's time. Verdicts with reasons are in [verdicts.json](./verdicts.json); the plan that carries the accepted ones is [docs/plans/2026-09-10-001-refactor-simplify-sweep-plan.md](../../2026-09-10-001-refactor-simplify-sweep-plan.md).

Totals: **158 accepted**, **27 rejected**, **66 merged** into another finding (251 in all).

## Counts by category

| Category | Accepted | Merged | Rejected | Total |
|---|---|---|---|---|
| const-location | 13 | 18 | 2 | 33 |
| copy-paste-variant | 22 | 6 | 3 | 31 |
| dead-code | 10 | 1 | 1 | 12 |
| duplication | 39 | 33 | 8 | 80 |
| efficiency | 19 | 3 | 6 | 28 |
| hardcoded-value | 27 | 4 | 5 | 36 |
| naming | 3 | 0 | 1 | 4 |
| quality | 16 | 0 | 1 | 17 |
| reuse | 9 | 1 | 0 | 10 |

## Counts by partition

| Partition | Accepted | Merged | Rejected | Total |
|---|---|---|---|---|
| cli-commands | 6 | 4 | 3 | 13 |
| cli-sync | 14 | 4 | 1 | 19 |
| cli-tui | 16 | 1 | 2 | 19 |
| core-git | 14 | 0 | 1 | 15 |
| core-rest | 9 | 5 | 2 | 16 |
| cross | 19 | 1 | 5 | 25 |
| plugin-checklist | 6 | 6 | 1 | 13 |
| plugin-hook-core | 9 | 5 | 1 | 15 |
| plugin-hook-gates | 8 | 4 | 1 | 13 |
| plugin-state | 9 | 10 | 1 | 20 |
| plugin-stores | 9 | 3 | 4 | 16 |
| plugin-verbs-a | 10 | 5 | 2 | 17 |
| plugin-verbs-b | 6 | 7 | 0 | 13 |
| plugin-verbs-c | 10 | 6 | 0 | 16 |
| python | 6 | 1 | 0 | 7 |
| shell | 7 | 4 | 3 | 14 |

## Highest-value accepted items

- `cross-F2` – one fd-lock opener in core (`lockfile::open_lock_file`) replacing three copies across two crates
- `cross-F3` – one civil-date module in core replacing three hand-rolled Hinnant implementations
- `cross-F6` – core becomes the one owner of the `.superset/*` file names; six re-spellings across three crates go
- `cross-F7` – a plugin `modes` module replacing 17 private mode constants, including the `FILE_MODE = 0o644` trap
- `cross-F8` – one `usage_error`/`fail` pair replacing nine copies with two incompatible argument orders
- `cross-F9` – one `pathnorm::rel_display` replacing six byte-equivalent helpers under four names
- `cross-F10` – one harness-config-dir resolver replacing four copies (and the dead `transcript_root`)
- `plugin-state-F17` – an `AtomicWrite` options struct so a dozen seven-positional-argument calls become readable
- `cross-F14` – CLI test files stop re-implementing `testutil` helpers, one of them without the `gc.auto 0` race fix
- `plugin-stores-F15` – one `testutil::ignored_repo` fixture replacing eight copies in the plugin crate
- `cli-sync-F5` – the Merge and Delete arms of `apply_decision` share their two-sided guard and backup block on the secret-safety path
- `cli-sync-F11` – `FileState` and `file_state` are deleted; the cockpit stops computing a value `default_decision` ignores
- `plugin-state-F16` – `Refusal::code` and a false comment about the heartbeat writer are deleted
- `cross-F17` – one handoff-file reader in `lib/tmproot.sh` replacing the duplicated block in the shim and the wrapper
- `plugin-verbs-a-F17` – a test pins `hooks.json` to `HookEvent` and `DECLARED_EVENTS`, the drift nothing enforced

## Accepted

| Finding | Unit | Title |
|---|---|---|
| `cli-commands-F1` | U3 | pack_core re-implements resolve_sync_roots' repo-root resolution, including the byte-identical error line |
| `cli-commands-F2` | U3 | The `files` is empty guard is duplicated between sync_core and pack_core with only the verb differing |
| `cli-commands-F4` | U3 | rename_setup_config re-implements copy_into_repo's delete-set, which run_migrate already invokes on the previous line |
| `cli-commands-F5` | U3 | run_init and run_init_noninteractive duplicate the whole stage-and-materialize sequence, tempdir prefix and status lines included |
| `cli-commands-F6` | U3 | The pack archive's name prefix and suffix are spelled once in archive_file_name and again in is_pack_archive_rel |
| `cli-commands-F7` | U3 | apply_update and apply_update_unlocked repeat the same run_self_update outcome mapping |
| `cli-sync-F3` | U5 | The cockpit help text re-spells both the backups path and the BACKUP_BATCHES_KEPT retention count as literals |
| `cli-sync-F5` | U4 | apply_decision's Merge and Delete arms carry ~25 byte-identical lines of two-sided guard plus two-sided backup |
| `cli-sync-F6` | U4 | backup_forward_targets re-implements finish_batch's print-backups-then-prune tail, including a verbatim warning string |
| `cli-sync-F7` | U4 | The "Nothing selected" message and its early return appear twice, six lines apart, in run() |
| `cli-sync-F8` | U4 | The legacy backup side-directory names "local" and "main" are spelled as bare literals in two places in prune_old_backups |
| `cli-sync-F10` | U4 | merge.rs's module doc states behavior default_decision no longer has ("only worktree-only files auto-push") |
| `cli-sync-F11` | U4 | default_decision ignores its only parameter, and cockpit::file_state exists solely to compute the value it discards |
| `cli-sync-F12` | U4 | Plan-id-and-task-number-only comments and superseded "picker" vocabulary in the two modules' docs |
| `cli-sync-F13` | U4 | Eleven items are declared pub with no caller outside their own module, overstating the engine's integration surface |
| `cli-sync-F14` | U1 | is_safe_rel is a second spelling of the absolute/`..` rejection core says lives in ONE place |
| `cli-sync-F15` | U4 | classify stats then fully reads both copies of every candidate, and the cockpit reads the same bytes again |
| `cli-sync-F16` | U4 | backup_if_exists pre-checks existence and then type, costing two stats where one metadata call decides both |
| `cli-sync-F17` | U4 | The review-baseline capture loop is written twice, in run and run_bulk |
| `cli-sync-F18` | U4 | ApplyOutcome names two unrelated types in the same crate |
| `cli-tui-F1` | U4 | The three sync-direction labels are hand-typed in two match statements over two enums |
| `cli-tui-F3` | U5 | The usize-count-to-u16-scroll-bound expression is written out three times |
| `cli-tui-F4` | U5 | The focused file's widest line and its diff row count are recomputed on every frame / keypress |
| `cli-tui-F5` | U5 | render_confirm clones every decision (including whole assembled merge texts) just to count them |
| `cli-tui-F6` | U5 | The merge overlay assembles its whole preview twice per frame and again per scroll key |
| `cli-tui-F7` | U5 | The 'Local file' / 'Main branch' side labels are hand-typed twice in cockpit.rs |
| `cli-tui-F8` | U5 | HIGHLIGHT_SYMBOL_WIDTH is a separate literal that must equal HIGHLIGHT_SYMBOL's column count, with nothing enforcing it |
| `cli-tui-F9` | U5 | The 4-column line-number field is spelled once in num() and again inside three gutter-width constants |
| `cli-tui-F10` | U5 | The merge overlay's key legend is an inline literal while the normal-view legend is a named const |
| `cli-tui-F11` | U5 | format_mtime's minute/hour/day boundaries are bare literals |
| `cli-tui-F12` | U5 | ui.rs and menu.rs carry doc comments describing a second picker and two flows that were removed |
| `cli-tui-F13` | U5 | cockpit::is_interactive restates the both-ends-are-a-tty rule that main::menu_blocked_reason owns |
| `cli-tui-F14` | U5 | The cursor glyph is a named const in one cockpit site and an inline literal in the other |
| `cli-tui-F15` | U5 | handle_key's Merge arm repeats `if let Some(m) = app.merge.as_mut()` six times |
| `cli-tui-F16` | U5 | render_created carries eight parameters and an allow(too_many_arguments) |
| `cli-tui-F17` | U5 | Seg is reached by full path inside cockpit.rs although diffmodel is already imported |
| `core-git-F1` | U2 | join-if-relative + canonicalize is written three times inside the git module |
| `core-git-F2` | U2 | `status_porcelain` re-implements `git()`'s success check and bail message verbatim |
| `core-git-F3` | U2 | `String::from_utf8_lossy(..).trim().to_string()` is spelled ten times across the module |
| `core-git-F4` | U2 | Four subprocess spawns bypass `git_raw` and hand-roll its shape, against the stated convention |
| `core-git-F5` | U2 | `repository_format_check` duplicates the read/NotFound/unparseable block and two error strings |
| `core-git-F6` | U2 | `GITFILE_MAX_BYTES` is `pub` with no user outside `discover.rs`, while its sibling cap is private |
| `core-git-F7` | U2 | Two decline reasons restate `GITFILE_MAX_BYTES`'s magnitude as prose |
| `core-git-F8` | U1 | The module that owns `.gitignore` handling does not name the file, so callers re-spell it |
| `core-git-F9` | U2 | `#[allow(dead_code)]` on a `pub` field of a `pub` struct in a library crate suppresses nothing |
| `core-git-F10` | U2 | `ensure_entry` pre-checks existence before reading (extra stat + TOCTOU window) |
| `core-git-F12` | U2 | Plan-id-only comments (`consumed by U11`, `wired into the menu by U10`) record task history, and one is stale |
| `core-git-F13` | U2 | Test helper `write` in git/tests.rs is byte-identical to `testutil::write_file` |
| `core-git-F14` | U2 | Test helper `git_init` in gitignore/tests.rs hand-rolls a git spawn instead of `testutil::git_run` |
| `core-git-F15` | U1 | Two test fixtures re-implement `testutil::init_main_repo`, one of them dropping its deliberate `gc.auto 0` guard |
| `core-rest-F1` | U2 | Four `#[allow(dead_code)]` + plan-id-only comments on `pub` items that a library crate can never lint as dead |
| `core-rest-F2` | U2 | Four `load_*` functions are the same three lines, each joining the same path twice |
| `core-rest-F3` | U2 | `write_magic_json` and `write_magic_local_json` are the same four lines with a different file-name constant |
| `core-rest-F4` | U2 | The `Glob::new` + "compiling glob" error context is written twice in the sync engine |
| `core-rest-F5` | U2 | Every glob pattern is compiled twice per `run`/`match_paths` – once to validate it, once to use it |
| `core-rest-F6` | U2 | Two test helpers in this partition re-implement `testutil::write_file` |
| `core-rest-F9` | U2 | `superset_files/tests.rs` re-declares `repo_scan::OPTIONS` byte for byte, under the same name |
| `core-rest-F12` | U2 | Two overlapping directory-exclusion lists in the `sync` tree with no cross-reference between them |
| `core-rest-F13` | U2 | `repo_scan`'s walk is the one enumeration in `sync/` that never asks `under_excluded_tree` |
| `cross-F2` | U1 | The fd-lock file opener is written three times across two crates |
| `cross-F3` | U1 | Hinnant civil-date arithmetic hand-rolled three times in two crates |
| `cross-F4` | U1 | The state-tree ignore query `.superset/.magic/` is spelled three times, in a module whose whole point is being the one spelling |
| `cross-F5` | U1 | `.superset/backups` exists as a component array in core and as a separate string const in the CLI |
| `cross-F6` | U1 | `.superset/magic.json` and `.superset/magic.local.json` are re-spelled in all three crates while core owns the parts privately |
| `cross-F7` | U6 | Owner-only mode constants declared 17 times across the plugin crate, one of them with the same name and a different value |
| `cross-F8` | U6 | Six copies of `print error, print usage, exit 2` with four signatures – two of them with swapped arguments |
| `cross-F9` | U6 | `path relative to root, lossy, fall back to absolute` written five times under four names |
| `cross-F10` | U6 | The harness config directory (`$CLAUDE_CONFIG_DIR` else `$HOME/.claude`) is resolved four times, twice byte-identically |
| `cross-F11` | U6 | `fn plural` is byte-identical in two plugin modules |
| `cross-F12` | U6 | `std::env::current_dir().context("reading the current directory")` repeated at 15 sites, with two divergent spellings |
| `cross-F13` | U9 | The `--json`-less report row renderer (label column width 22) is written twice |
| `cross-F14` | U2 | CLI test files re-implement testutil helpers they already partly import – and the copies are missing an upstream race fix |
| `cross-F16` | U10 | The shell mirror of tmproot names three constants but inlines the identifier length and the 0700 mode |
| `cross-F17` | U10 | The data-root handoff resolution block is duplicated between the hook shim and the Bash wrapper |
| `cross-F19` | U8 | The six state-file names are typed twice, in an order and spelling nothing enforces |
| `cross-F20` | U8 | session_start re-declares the checklist pointer name that checklist already exports, behind a stale doc comment |
| `cross-F21` | U7 | Two 30-day retention bounds the code says must match are declared independently |
| `cross-F22` | U1 | heartbeat re-implements core's now_secs with a fallback parameter |
| `plugin-checklist-F5` | U9 | DOCUMENT_ID and the validator's "document" location are two literals a prose comment asks to stay equal |
| `plugin-checklist-F6` | U9 | The wire tokens of ItemKind and Priority are re-spelled in verbs.rs instead of living beside as_str |
| `plugin-checklist-F7` | U9 | Priority::as_str, Document::section and Document::items are reachable from nothing, including tests |
| `plugin-checklist-F8` | U9 | Four copies of "blank field renders a placeholder, else prose_inline" in render.rs |
| `plugin-checklist-F9` | U9 | The reference-list block is written twice in render.rs |
| `plugin-checklist-F10` | U9 | The error-count expression is inlined twice in verbs.rs while validate.rs owns has_errors |
| `plugin-hook-core-F3` | U8 | The ignored-state-tree probe and its two refusal sentences are written twice, word for word apart from one suffix |
| `plugin-hook-core-F5` | U8 | SessionStart reads the plugin pin file twice, from two notices that document themselves as reading one pin |
| `plugin-hook-core-F6` | U8 | The cache-directory closure is invoked twice per SessionStart, and each call runs create_dir_all |
| `plugin-hook-core-F7` | U8 | pre_compact computes the same `trigger_display` fallback twice in one file |
| `plugin-hook-core-F8` | U8 | Three handlers repeat the same repo_root guard and the same hard-coded detail sentence |
| `plugin-hook-core-F11` | U8 | The `unroutable-event` heartbeat class has two independent owners |
| `plugin-hook-core-F13` | U8 | The harness's 10,000-character additionalContext cliff is spelled as a bare number in four comments and one test assertion |
| `plugin-hook-core-F14` | U8 | Four comments in hook/mod.rs still describe a pre-U12 state in which the handlers had not landed |
| `plugin-hook-core-F15` | U8 | `base` names two different row builders in one function, and a third identical closure is redeclared in the next |
| `plugin-hook-gates-F2` | U8 | The gate's worktree-root fallback is spelled twice, verbatim, in one file |
| `plugin-hook-gates-F3` | U8 | The bypass paragraph is repeated verbatim in the two deny reasons |
| `plugin-hook-gates-F7` | U8 | Two handlers each hand-roll the scan over `Refusal::TrackedPaths` |
| `plugin-hook-gates-F8` | U8 | Every Bash tool call copies the whole command string to strip heredocs that are almost never there |
| `plugin-hook-gates-F9` | U8 | `direnv_export` returns `Result<Option<Option<String>>>` where three named outcomes are meant |
| `plugin-hook-gates-F10` | U8 | `commit_nudge` builds its response inline instead of through the module's outcome helpers |
| `plugin-hook-gates-F11` | U8 | The case-folding comparison idiom is written twice in the checklist classifier |
| `plugin-hook-gates-F13` | U8 | A test fixture re-implements `testutil::write_file` |
| `plugin-state-F2` | U7 | identity::slugify re-implements core's pub `reponame::sanitize_segment` loop (a third copy sits in checklist/render.rs) |
| `plugin-state-F3` | U7 | identity::slugify's `trim_matches('-')` can never trim anything |
| `plugin-state-F5` | U7 | scratchpad::CLAIM_DIRS re-types the three directory names that cache/bypass/expect_artifact each own as pub DIR_NAME |
| `plugin-state-F10` | U7 | heartbeat::prune scans and filters the whole log twice |
| `plugin-state-F14` | U7 | tmproot::INSTALL_LOCK_NAME has no Rust caller and nothing checks it against the shell side it documents |
| `plugin-state-F15` | U7 | Comments that narrate unit history, one of them now factually wrong |
| `plugin-state-F16` | U7 | Refusal::code() is called only from tests, under a comment claiming the heartbeat writer consumes it |
| `plugin-state-F17` | U6 | write_atomically's seven positional parameters make every call site unreadable |
| `plugin-state-F20` | U7 | The 40-character slug bound is a bare literal in two modules that name every other bound |
| `plugin-stores-F1` | U7 | The state-tree bootstrap-and-refusal block is copied verbatim into all three verbs (and near-verbatim into checklist init) |
| `plugin-stores-F2` | U6 | The owner-only recursive directory create is hand-written in all three stores (and in five more modules) |
| `plugin-stores-F5` | U7 | bypass::claim_path and expect_artifact::record_path are the same function twice |
| `plugin-stores-F6` | U7 | The char-boundary backwalk is copied between trim_note and bound; std has floor_char_boundary |
| `plugin-stores-F7` | U7 | Three sibling stores resolve their own directory three different ways |
| `plugin-stores-F9` | U7 | Stale narration comment on Budget::Bytes: the Read gate does construct it now |
| `plugin-stores-F11` | U7 | cache::list calls age_stamp (which can stat the file) from inside the sort comparator |
| `plugin-stores-F13` | U7 | record_path and claim_path are pub with no caller outside their own module |
| `plugin-stores-F15` | U1 | ignored_repo() is re-implemented in all three sibling test modules (eight copies crate-wide) with a divergent root derivation |
| `plugin-verbs-a-F2` | U9 | `run_toggle_core` and `run_config_set_core` repeat the same root-resolution and ignore-rule block verbatim |
| `plugin-verbs-a-F3` | U9 | `resolve_with_roots` reads and parses the overlaid magic.json twice on the hook hot path |
| `plugin-verbs-a-F4` | U9 | `status::collect` resolves the plugin config twice for the same root, spawning a second `git rev-parse` |
| `plugin-verbs-a-F5` | U9 | `report` re-prefixes a pin onto the line tag to ask whether it is a plain triple; `release::parse_bare_triple` answers directly |
| `plugin-verbs-a-F6` | U9 | "Is an update available?" is derived twice, by two different code shapes, in release_check and status |
| `plugin-verbs-a-F11` | U9 | setup-github-ci's usage banner re-types the workflow path that `WORKFLOW_REL` owns |
| `plugin-verbs-a-F12` | U9 | `run_core` renders the whole embedded workflow up to three times per run |
| `plugin-verbs-a-F13` | U9 | `REFRESH_ARGV`'s doc claims a coupling with the verb's parser that does not exist |
| `plugin-verbs-a-F16` | U9 | `bootstrap.sh` re-spells the `seed-config` verb token and swallows the exit status that would report a rename |
| `plugin-verbs-a-F17` | U9 | The hook-event token vocabulary exists in four hand-maintained copies; the Rust-side pair is untied |
| `plugin-verbs-b-F5` | U9 | "unknown – <reason>" is spelled three times in status.rs because `unknown()` bakes in the style |
| `plugin-verbs-b-F6` | U9 | `"no reason recorded"` is a bare literal at nine sites in status.rs and two in release_check.rs |
| `plugin-verbs-b-F9` | U9 | `heartbeat_store_if_present` is a one-line pass-through with a single caller |
| `plugin-verbs-b-F10` | U9 | spill-index's empty `Index` is built field-by-field twice |
| `plugin-verbs-b-F11` | U9 | status/tests.rs's `repo()` re-implements `testutil::init_main_repo` |
| `plugin-verbs-b-F12` | U9 | `bare_inputs` retypes the epoch the file's `NOW` const already holds |
| `plugin-verbs-c-F6` | U9 | Two operator messages re-spell `WINDOW_KEY` and `SETTINGS_LOCAL_REL` as literals in the file that owns them |
| `plugin-verbs-c-F7` | U9 | The `USAGE` banner hand-types the window bounds and both settings paths that are consts three lines above it |
| `plugin-verbs-c-F8` | U9 | A bare `64 * 1024` line buffer sits beside the named `READ_BUF_BYTES` in the same scan pipeline |
| `plugin-verbs-c-F9` | U9 | `rows_for_repository` resolves every root in the whole ledger before truncating to 20 rows |
| `plugin-verbs-c-F10` | U9 | `--recommend` reads and parses each project settings file twice per run |
| `plugin-verbs-c-F11` | U9 | `read_settings_object` pre-checks existence instead of handling `NotFound`, unlike `ledger::read` next door |
| `plugin-verbs-c-F12` | U9 | The same `BTreeSet<String>` to comma-list chain is written twice in `report`, cloning every element |
| `plugin-verbs-c-F13` | U9 | Two `.json` writers pass a `.jsonl` temp suffix copied from the ledger writer |
| `plugin-verbs-c-F15` | U9 | Nine `pub` items in these two verbs have no consumer outside their own module |
| `plugin-verbs-c-F16` | U6 | The crate's timestamp helpers live in `scratchpad` and are imported by a dozen unrelated modules |
| `python-F2` | U10 | Repository data paths are named constants for four files but re-spelled inline at ten call sites, including inside the selftest fixture builder |
| `python-F3` | U10 | check_bump spells 'digest and declared version of a plugin tree' twice, and a third variant of the plugin.json read lives in version_surfaces |
| `python-F4` | U10 | _version_repo hand-rolls mkdir + write_text eight times while the file's own _write helper does exactly that |
| `python-F5` | U10 | The marketplace release-tag regex is inline in version_surfaces while every sibling pattern is a module constant |
| `python-F6` | U10 | _semver is defined under the R98 section header but its first caller is the R95 README pin check 270 lines earlier |
| `python-F7` | U10 | --plugin-dir is neither documented nor used by any caller |
| `shell-F1` | U10 | Every give_up message in bootstrap.sh hand-repeats the same "; installed nothing." suffix (16 sites) |
| `shell-F2` | U10 | The pin-shape rejection message is written out three times across two case statements |
| `shell-F3` | U10 | The `--version \| head -1 \| awk '{print $NF}'` probe is spelled twice in bootstrap.sh |
| `shell-F8` | U10 | The release URL prefix is spelled in full twice in bootstrap.sh |
| `shell-F10` | U10 | `give_up` names two different contracts in two sibling hook scripts |
| `shell-F11` | U10 | mark-latest.sh forks printf+grep once per listed release inside the selection loop |
| `shell-F12` | U10 | test-bootstrap.sh defines the SHA-256 tool-detection branch twice, once for files and once for stdin |

## Rejected

| Finding | Title | Reason |
|---|---|---|
| `cli-commands-F8` | apply_update inlines the stale-check/open/RwLock prelude that try_lock_state_at already owns, and the tested function is not the production one | try_lock_state_at is a test-only seam that returns a verdict and drops the lock guard, while apply_update must keep the RwLock alive across the swap; sharing the three-line prelude means reshaping the seam. Low value. |
| `cli-commands-F11` | load_magic_or_exit stats magic.json before load_overlaid stats it again, for an answer load_overlaid already gives | Dropping the is_file() pre-check changes stderr for a .superset/magic.json that is a directory (today a one-line "not initialized" notice, afterwards load_overlaid's read error). Not behavior-preserving. |
| `cli-commands-F13` | The lock tests re-type STALE_TTL's 60 seconds instead of naming the constant (lead, weakly held) | A test retyping STALE_TTL's 60 seconds; confidence-50 lead with no reader benefit. Low value. |
| `cli-sync-F19` | Lead (unverified): the legacy 0.4.0 backup-layout fold may be scaffolding for a layout that was never released | Unverified lead (confidence 50) about removing the legacy 0.4.0 backup-layout fold; removing it changes prune behavior for existing backup trees. |
| `cli-tui-F18` | Two rect-centering helpers coexist, one percentage-based and one absolute | The two rect-centering helpers compute different things (percentage-sized vs absolute-sized rects); unifying them is a layout decision, not a duplication removal. |
| `cli-tui-F19` | Three renderers each inline their own gutter-line builder around num() | Confidence-50 lead; the three gutter builders differ in sign column and numbering, so a shared builder would need parameters that read worse than the three inline forms. Low value. |
| `core-git-F11` | The ancestor walk stats every intermediate directory twice on the hook fast path | The ancestor walk's per-directory stat order is part of the discover module's pinned "byte-equal to git or decline" behavior (structure-pins: git::discover declines); collapsing the two stats changes which race windows the equivalence matrix observes. Efficiency on a path that is already subprocess-free; not worth the risk. |
| `core-rest-F15` | The release-line tag prefix and repo slug that `release.rs` owns are re-spelled in shell, YAML and JSON with nothing asserting they agree | The proposed cross-language pin adds a new assertion line to build-plugin-zip.py --check, whose seven-line output is enumerated by the verification contract and CLAUDE.md; a refactor sweep must not change that surface. |
| `core-rest-F16` | Five writers in `superset_files` use three different durability postures with no stated rule | A durability-policy decision (which writers fsync, which stage), not a simplification; confidence 50. |
| `cross-F1` | Three independent atomic-write implementations, one per crate | Not behavior-preserving as proposed: the three writers differ in staged-file naming, error-context strings, mode preservation (reverse_sync::write_bytes copies the existing mode), fsync policy and symlink handling (core canonicalizes the target first). One helper would change at least two of them; core-vs-plugin write_atomically is legitimately reportable per structure-pins.md but the honest answer is "different contracts". |
| `cross-F15` | `bin/ss-magic-plugin` spelled independently in three shell files that already share a constants file, plus once in Rust | The three shell files do not all source lib/tmproot.sh unconditionally (bootstrap.sh sources it only when readable; run-hook.sh only on its fallback path), so a shared shell constant would either change the sourcing contract or leave at least one literal in place; the Rust BINARY_REL cannot be read from shell at all. Low value. |
| `cross-F18` | CI re-implements the builder's `which zip does dist publish` lookup in bash | CI re-implements the artifact-name lookup on purpose: the workflow step verifies that the builder produced the filename cargo-dist declares, and reading that name through the builder under test would make the check circular. Not a simplification. |
| `cross-F24` | Per-binary `version_line` and `usage` – settled by the crate-independence pin | Settled by structure-pins.md: per-binary version_line and usage are the crate-independence pin. |
| `cross-F25` | Hand-rolled SHA-256 in core mirroring the shell's shasum – settled | Settled by structure-pins.md: the hand-rolled SHA-256 mirroring the shell shasum is a deliberate cross-language contract. |
| `plugin-checklist-F11` | validate.rs's error and warning constructors share one body | Two four-line constructors differing in one enum value; a shared helper used twice buys nothing. Low value. |
| `plugin-hook-core-F4` | Every state-writing hook runs the same `git check-ignore --no-index` probe twice per invocation | The second probe is scratchpad::ensure's own fail-closed gate; passing the pipeline's verdict across that boundary thins a safety check that structure-pins.md keeps fail-closed. |
| `plugin-hook-gates-F12` | The checklist pointer is read from disk and parsed on every gated tool call, up to twice | Memoizing the checklist pointer for the process lifetime changes a safety gate's reads-per-decision, and the pre_tool_use tests rewrite the pointer mid-process. |
| `plugin-state-F12` | The state-root not-a-directory check stats the same path twice | The two calls are exists() (which swallows errors) and metadata(); replacing them with one metadata() call would propagate an error exists() hid. Not behavior-preserving on the error path. |
| `plugin-stores-F10` | Claim and declaration lifetimes are re-spelled in prose that no test ties to MAX_AGE_SECS | Prose in usage banners and success lines restating MAX_AGE_SECS; turning them into format! strings or tests gives the reader nothing. Low value. |
| `plugin-stores-F12` | gc reads and parses every entry twice: once itself, once through prune | gc lists, removes orphans, then prune re-lists; a single listing would prune against a set that still counts the entries gc just removed, changing which entries survive. Not behavior-preserving. |
| `plugin-stores-F14` | "a u64 in hex is 16 characters" is expressed four different ways | "16 hex characters" expressed as KEY_HEX_LEN, NONCE_HEX_LEN, {:016x} and a glob are four different local facts; a shared const would couple unrelated formats. Low value. |
| `plugin-stores-F16` | Lead, unverified: 'mtime, else content hash' file identity exists independently in cache::identify and the CLI's reverse-sync baseline | Unverified lead (confidence 50): cache::identify keys on nanosecond mtime plus size with a content-hash fallback, reverse_sync::FileMeta on length plus mtime with a hash fallback captured only when mtime is absent; the two do not behave the same. |
| `plugin-verbs-a-F14` | Three different flag-parsing idioms for the same job inside one crate | Three argv-parsing idioms across verbs is a style decision (confidence 50); cross-F8 already unifies the tail they share. |
| `plugin-verbs-a-F15` | Bare `LOCK_NAME` and `SCHEMA_VERSION` identifiers hold different values in different modules, beside a prefixed convention for the same role | Renaming module-private LOCK_NAME / SCHEMA_VERSION identifiers that never collide in scope; no reader benefit. Low value. |
| `shell-F4` | bootstrap.sh's `digest_of` re-implements the shasum/sha256sum detection chain that the tmproot.sh it already sources provides | bootstrap.sh sources lib/tmproot.sh only when readable, so depending on its sha256 helper changes behavior when the lib is absent or unreadable. |
| `shell-F9` | The supported-platform matrix is maintained independently in bootstrap.sh and dist-workspace.toml with nothing linking them | Pinning the platform matrix against dist-workspace.toml is a new gate, not a simplification. |
| `shell-F13` | The bootstrap's four timeout constants are sized against hooks.json's 90 s budget by comment only | Asserting the timeout sum against hooks.json's 90 s is a new assertion (confidence 50), not a simplification. |

## Merged

Each of these names the same locations or the same proposal as its target; the target's verdict applies.

| Finding | Merged into | Target verdict |
|---|---|---|
| `cli-commands-F3` | `cross-F6` | accepted |
| `cli-commands-F9` | `cross-F2` | accepted |
| `cli-commands-F10` | `cross-F14` | accepted |
| `cli-commands-F12` | `cross-F6` | accepted |
| `cli-sync-F1` | `cross-F3` | accepted |
| `cli-sync-F2` | `cross-F5` | accepted |
| `cli-sync-F4` | `cross-F1` | rejected |
| `cli-sync-F9` | `cross-F14` | accepted |
| `cli-tui-F2` | `cli-commands-F1` | accepted |
| `core-rest-F7` | `cross-F14` | accepted |
| `core-rest-F8` | `cross-F14` | accepted |
| `core-rest-F10` | `cross-F6` | accepted |
| `core-rest-F11` | `cross-F6` | accepted |
| `core-rest-F14` | `cross-F1` | rejected |
| `cross-F23` | `cross-F8` | accepted |
| `plugin-checklist-F1` | `plugin-state-F2` | accepted |
| `plugin-checklist-F2` | `cross-F9` | accepted |
| `plugin-checklist-F3` | `cross-F8` | accepted |
| `plugin-checklist-F4` | `cross-F20` | accepted |
| `plugin-checklist-F12` | `cross-F3` | accepted |
| `plugin-checklist-F13` | `plugin-stores-F15` | accepted |
| `plugin-hook-core-F1` | `cross-F9` | accepted |
| `plugin-hook-core-F2` | `cross-F4` | accepted |
| `plugin-hook-core-F9` | `plugin-hook-gates-F7` | accepted |
| `plugin-hook-core-F10` | `cross-F7` | accepted |
| `plugin-hook-core-F12` | `cross-F19` | accepted |
| `plugin-hook-gates-F1` | `cross-F9` | accepted |
| `plugin-hook-gates-F4` | `cross-F6` | accepted |
| `plugin-hook-gates-F5` | `cross-F7` | accepted |
| `plugin-hook-gates-F6` | `plugin-state-F20` | accepted |
| `plugin-state-F1` | `cross-F9` | accepted |
| `plugin-state-F4` | `cross-F4` | accepted |
| `plugin-state-F6` | `cross-F7` | accepted |
| `plugin-state-F7` | `cross-F22` | accepted |
| `plugin-state-F8` | `cross-F3` | accepted |
| `plugin-state-F9` | `cross-F2` | accepted |
| `plugin-state-F11` | `plugin-stores-F2` | accepted |
| `plugin-state-F13` | `cross-F19` | accepted |
| `plugin-state-F18` | `cross-F1` | rejected |
| `plugin-state-F19` | `cross-F21` | accepted |
| `plugin-stores-F3` | `cross-F7` | accepted |
| `plugin-stores-F4` | `plugin-state-F5` | accepted |
| `plugin-stores-F8` | `cross-F8` | accepted |
| `plugin-verbs-a-F1` | `cross-F8` | accepted |
| `plugin-verbs-a-F7` | `cross-F13` | accepted |
| `plugin-verbs-a-F8` | `cross-F7` | accepted |
| `plugin-verbs-a-F9` | `cross-F7` | accepted |
| `plugin-verbs-a-F10` | `cross-F6` | accepted |
| `plugin-verbs-b-F1` | `cross-F10` | accepted |
| `plugin-verbs-b-F2` | `cross-F11` | accepted |
| `plugin-verbs-b-F3` | `cross-F13` | accepted |
| `plugin-verbs-b-F4` | `cross-F8` | accepted |
| `plugin-verbs-b-F7` | `plugin-verbs-a-F4` | accepted |
| `plugin-verbs-b-F8` | `plugin-verbs-a-F17` | accepted |
| `plugin-verbs-b-F13` | `cross-F10` | accepted |
| `plugin-verbs-c-F1` | `cross-F11` | accepted |
| `plugin-verbs-c-F2` | `plugin-stores-F2` | accepted |
| `plugin-verbs-c-F3` | `cross-F10` | accepted |
| `plugin-verbs-c-F4` | `cross-F10` | accepted |
| `plugin-verbs-c-F5` | `cross-F8` | accepted |
| `plugin-verbs-c-F14` | `cross-F7` | accepted |
| `python-F1` | `cross-F18` | rejected |
| `shell-F5` | `cross-F17` | accepted |
| `shell-F6` | `cross-F17` | accepted |
| `shell-F7` | `cross-F16` | accepted |
| `shell-F14` | `cross-F15` | rejected |
