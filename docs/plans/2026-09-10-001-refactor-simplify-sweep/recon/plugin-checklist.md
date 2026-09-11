# Recon: plugin-checklist – the checklist document and verbs

Partition files: `crates/ss-magic-plugin/src/checklist/{mod,schema,order,validate,render,verbs}.rs`.

## Map

### `mod.rs` (72 lines) – module doc, re-exports, `#![allow(dead_code, unused_imports)]`

No functions. Re-exports the family's public surface from the five private submodules (`order`,
`render`, `schema`, `validate`, `verbs`), all declared `mod` (private) so only this file's `pub use`
list is externally reachable.

### `order.rs` (79 lines) – canonical sort order for a document

- `UNRANKED_RANK: u8 = 3` (pub const) – sort rank an item with no `priority` gets; one past the last real rank.
- `canonicalize(doc: &mut Document)` (pub fn) – sorts `doc.changelog` by `entry_key` and every section's `items` by `item_key`, id as tie-break; in place, idempotent.
- `item_key(item: &Item) -> (u8, u8, (u8, Instant))` (private) – sort key: `(done, priority rank, instant_key(created))`.
- `entry_key(entry: &ChangelogEntry) -> (u8, Instant)` (private) – sort key: `instant_key(created)`.
- `instant_key(ts: &Timestamp) -> (u8, Instant)` (private, small/generic) – `(0, instant)` when readable, `(1, Instant::default())` when not, so unreadable sorts last. **Candidate for reuse**: the "push a possibly-unreadable Timestamp to the end via a leading flag" idea is generic and could be reused anywhere else in the crate that sorts by `Timestamp` (grep found none today – see Cross-module references).

### `schema.rs` (683 lines) – typed document model + hand-rolled ISO-8601 reader

Types/consts (pub):
- `SCHEMA_ID: &str` (line 84) – the `$schema` value this build writes.
- `Timestamp(String)` (93) – raw ISO-8601 spelling, `Ord` deliberately absent.
  - `new`, `from_epoch_secs`, `as_str`, `is_empty`, `instant` (97-125).
- `Instant { secs: i64, nanos: u32 }` (139) – derived `Ord`.
- `TimeError` enum (`Shape`, `NoOffset`, `BadOffset`, `OutOfRange(&'static str)`) (152) + `Display` (165).
- `ItemKind` enum (`Check` default, `Record`, `Decision`) (183) – `as_str`, `allows_null_expectation`.
- `Priority` enum (`Blocking`, `DecisionBlocking`, `FollowUp`) (217) – `as_str`, `rank`.
- `Reference { label, url, extras }` (254).
- `Document { schema, title, slug, created, updated, changelog, sections, extras }` (270) – `new`, `with_sections`, `section(id)`, `items()` (448-486).
- `ChangelogEntry { id, created, summary, details, refs, extras }` (307).
- `Section { id, title, items, extras }` (331).
- `Item { id, title, kind, priority, created, done, completed, description, steps, expected, why, refs, extras }` (349) – `expected_text`, `expected_declared` (396-406).
- `default_sections() -> Vec<Section>` (431) – the four built-in sections (`verification`, `rollout`, `decisions`, `follow-ups`).
- `from_json(&str) -> Result<Document>` (491), `to_json(&Document) -> Result<String>` (498), `read_document(&Path) -> Result<Option<Document>>` (507).
- `parse_iso8601(&str) -> Result<Instant, TimeError>` (531) – the ISO-8601 reader.

Private helpers (small/generic – candidates for reuse elsewhere doing date math):
- `deserialize_some` (415) – serde helper distinguishing absent key from explicit null.
- `parse_offset(&str) -> Result<i64, TimeError>` (597) – `±HH[:]MM`/`Z` offset reader.
- `digits(bytes, start, count) -> Option<u32>` (629) – fixed-width ASCII-digit reader.
- `fraction_to_nanos(&str) -> u32` (644).
- `is_leap(i64) -> bool` (653).
- `days_in_month(i64, u32) -> u32` (657).
- `days_from_civil(year, month, day) -> i64` (670) – Howard Hinnant's algorithm; **module doc explicitly says this is "the exact inverse of the civil-from-days conversion the plugin's UTC formatter runs"** i.e. `scratchpad::format_rfc3339`/its epoch-to-civil counterpart. Two independent, hand-written implementations of the same calendar math in two files (see Leads L1).

### `validate.rs` (437 lines) – pure findings over a `Document`

- `Severity` enum (`Error`, `Warning`) (25) + `Display` (34).
- `Finding { severity, location, message }` (49) + `Display` (58).
- `has_errors(&[Finding]) -> bool` (65).
- `validate(&Document) -> Vec<Finding>` (71) – the one entry point; walks header, changelog, sections/items, sharing one `HashMap<&str,String>` id namespace.
- Private per-record checks (all take `findings: &mut Vec<Finding>, at: &str`, small/generic pattern repeated 8 times): `check_document_header` (91), `check_changelog_entry` (139), `check_section` (151), `check_item` (165), `check_steps` (181), `check_completion` (200), `check_expected` (232), `check_refs` (280), `check_id` (314), `require_text` (347), `check_timestamp` (353).
- `is_well_formed_id(&str) -> bool` (pub, 374) – kebab-case id predicate.
- `is_absolute_url(&str) -> bool` (pub, 393) – shallow `scheme://authority` predicate.
- `error`/`warning` (410/418, private) – `Finding` constructors, near-identical bodies (only the `severity` field differs) – **copy-paste variant candidate**, see L2.
- `display_id(&str) -> &str` (428, private, small/generic) – `"<no id>"` fallback for an empty id; same shape as several "or a placeholder" helpers in `render.rs` (`"(untitled item)"`, `"(untitled section)"`, `"Untitled checklist"`, `"(no summary)"`, `"(no date)"`) – none of those five share a helper either (L3).

### `render.rs` (512 lines) – pure Markdown renderer

- `render(doc, path, repo_url, budget) -> String` (pub, 81) – the one entry point; wraps `render_body` + `render_head` in `crate::cache::envelope`.
- `render_head(&Path) -> String` (91, private).
- `SectionPlan<'a> { section, anchor, items }`, `ItemPlan<'a> { item, anchor }` (109/116, private).
- `render_body`, `render_title`, `render_metadata`, `plan_sections`, `render_toc`, `render_changelog`, `render_changelog_entry`, `render_section`, `render_item` (121-346, all private) – the body-building pipeline.
- `format_ts(&Timestamp) -> Option<String>` (357, private) – renders through `crate::scratchpad::format_rfc3339` over the instant `schema::parse_iso8601` parsed.
- `escape_inline_char(char, &mut String)` (373, private, small/generic) – CommonMark punctuation escaper.
- `prose_inline(&str) -> String` (407, private) – line-wise escape + `<br>` join.
- `md_link(label, url) -> String` (429, private) – safe `[label](<url>)`.
- `slugify(&str) -> String` (455, private, small/generic) – alnum/hyphen reduction for anchor ids.
- `anchor_for(seen, prefix, index, id, title) -> String` (480, private).
- `dedupe(seen: &mut HashMap<String,u32>, base: String) -> String` (501, private, small/generic) – GitHub-style `-1`/`-2` disambiguation. **Candidate for reuse**: this exact "count occurrences, suffix `-N-1` from the second on" idea is a common anchor/slug-dedup pattern; grep found no sibling implementation elsewhere in the workspace today (see Cross-module references) but it is generic enough to flag if one turns up.

### `verbs.rs` (1462 lines) – the `checklist` command surface

Pub consts:
- `ACTIONS_REL: &str = "docs/actions"` (94).
- `CHECKLIST_SUFFIX: &str = ".checklist.json"` (97).
- `POINTER_NAME: &str = "checklist.json"` (101).

Private consts:
- `LOCK_NAME: &str = "checklist.lock"` (105).
- `NEW_FILE_MODE: u32 = 0o644` (111).
- `LIST_BYTE_BUDGET: usize = 24_000` (121).
- `USAGE: &str` (220) – the verb's help text.
- `DOCUMENT_ID: &str = "document"` (866) – the id addressing the document header itself.

Pub types/fns:
- `Pointer { path, slug, recorded_at }` (127) – **naming collision** with `scratchpad::Pointer` (see Cross-module references / L5), different shape and different write path (fd_lock there, `tmproot::with_lock` here).
- `pointer_path(root) -> PathBuf` (139).
- `pointer_target(root) -> Option<PathBuf>` (150) – reads+parses the pointer, then `contained_join`s it.
- `matches_convention(root, path) -> bool` (191) – case-insensitive `docs/actions/*.checklist.json` naming-convention test.
- `run(args: &[String]) -> Result<ExitCode>` (427) – the entry point.

Private types/fns (selection; small/generic ones flagged):
- `contained_join(root, rel) -> Option<PathBuf>` (163, small/generic) – rejects absolute/`..`-bearing relative paths before joining. **Compare against** `pathnorm::normalize`/`ParentDir` handling in `hook/pre_tool_use.rs` and `scratchpad::verify_contained` – three different "is this relative path safe to join under root" implementations in the same crate (L4).
- `Sub` enum + `wants_stdin`/`stdin_is_required`/`with_body` (258-332).
- `ParsedSub` enum (338), `parse(&[String]) -> ParsedSub` (346), `arity(&str) -> ParsedSub` (419).
- `run_core`, `run_init`, `checklist_stem`, `split_date_prefix`, `year_month`, `title_from_slug` (470-634).
- `run_render`, `run_verify`, `print_findings` (641-715).
- `run_mutation`, `mutate_locked`, `add_item`, `add_entry`, `mark_done` (721-850).
- `Located` enum, `locate`, `apply_set`, `set_document_field`, `set_entry_field`, `set_item_field`, `set_steps`, `set_refs`, `parse_index`, `optional_text`, `required_text`, `timestamp` (856-1165).
- `load_active`, `resolve_active` (1181-1255).
- `write_document`, `write_pointer`, `with_lock` (1266-1335) – `with_lock` (1329, small/generic): "if `state_root.is_dir()` run under `tmproot::with_lock`, else just run `f()` unlocked" – **near-identical guard pattern** appears in `config.rs::write_locked` (tries `tmproot::resolve...` and falls through) and `release_check.rs` (`tmproot::try_with_lock`, non-blocking variant) (L6).
- `browsable_origin(root) -> Option<String>` (1347) – parses `ssh://`, `https?://`, and scp-like (`user@host:path`) git remote forms into a browsable `https://` URL. **Overlaps** `ss_magic_core::reponame::stem_from_origin` (core, `crates/ss-magic-core/src/reponame.rs:47`), which parses the same three remote-URL shapes for a different purpose (a filename stem) (L1 sibling, see L7).
- `display_rel(root, path) -> String` (1374, small/generic) – `strip_prefix(root)` + `to_string_lossy`. Same idea used ad hoc via `.to_string_lossy().into_owned()` in `bypass.rs`, `expect_artifact.rs`, `ledger.rs` (multiple sites), `scratchpad.rs`; none of those five call a shared helper (L8).
- `check_new_id`, `unknown_id`, `unknown_key`, `list_ids`, `usage_error`, `refused`, `fail` (1384-1459).

## Constants and literals

| Const/literal | Value | Location | Elsewhere in workspace? | Owning definition |
|---|---|---|---|---|
| `SCHEMA_ID` | `"https://github.com/ViktorStiskala/superset-magic/schema/checklist/v1"` | schema.rs:84 | Not found elsewhere (consts.md single hit) | Here – correct owner. |
| `UNRANKED_RANK` | `3` | order.rs:40 | referenced by `Priority::rank` comment in schema.rs:237 only | Here – correct owner, `schema.rs` just documents it. |
| `ACTIONS_REL` | `"docs/actions"` | verbs.rs:94 | Used in `setup_ci.rs:55,275,389` and `hook/pre_tool_use.rs:1492` via `checklist::ACTIONS_REL` – reuse, not duplication. | Here – correct owner. |
| `CHECKLIST_SUFFIX` | `".checklist.json"` | verbs.rs:97 | Used in `setup_ci.rs:55,275,396` via re-export – reuse, not duplication. | Here – correct owner. |
| `POINTER_NAME` | `"checklist.json"` | verbs.rs:101 | **`hook/session_start.rs:62` independently defines `const CHECKLIST_POINTER_NAME: &str = "checklist.json"`** – same literal, not imported from `checklist::POINTER_NAME` (literals.md:119 confirms two owners of the string `"checklist.json"`). | Should be `checklist::POINTER_NAME` (already `pub`). See L9. |
| `LOCK_NAME` | `"checklist.lock"` | verbs.rs:105 | Name collides (not value) with `release_check.rs:53 LOCK_NAME = "release-check.lock"` and `hook/file_changed.rs:106 LOCK_NAME = "file-changed.lock"` – three modules each declare a private `LOCK_NAME` constant; values differ so this is a naming pattern, not a duplicate value. Flagged `dup-name` in consts.md. | Naming convention note only – see L10. |
| `NEW_FILE_MODE` | `0o644` | verbs.rs:111 | consts.md flags `dup-name`; no other `0o644` const found under this exact name in the plugin crate (grep of numbers.md shows no `0o644`/`0o777` bare literal >= 60 threshold list for this file besides schema.rs's unrelated numbers). | Here – correct owner (committed-content mode, deliberately distinct from state-tree's `0o600`/`0o700` in `scratchpad.rs`). |
| `LIST_BYTE_BUDGET` | `24_000` | verbs.rs:121 | Not found duplicated elsewhere. | Here – correct owner. |
| `DOCUMENT_ID` | `"document"` | verbs.rs:866 | Not found duplicated; deliberately spelled the same as validator's `"document"` location string (validate.rs:92 `let at = "document";`) – same string, two independent literals meant to stay in sync (documented coupling, module comment at verbs.rs:864-865 calls this out explicitly). | Two owners by design/comment – still worth a lead if either drifts (L11). |
| `USAGE` | multi-line usage text | verbs.rs:220 | consts.md flags `dup-name` (other `USAGE` consts exist per-verb elsewhere in the crate, e.g. `config.rs`, `release_check.rs`, `scratchpad.rs`'s `SCRATCHPAD_USAGE`) – naming convention, not value duplication. | N/A. |
| `86_400`, `3600`, `60` (seconds-per-day/hour/minute) | schema.rs:587-590, 624 | Bare numeric literals in `parse_iso8601`/`parse_offset` | Not found elsewhere as named consts; ordinary calendar constants. | Fine as-is; flagged only because numbers.md lists them. |
| `146_097`, `719_468`, `400`, `100` (Hinnant civil-from-days magic numbers) | schema.rs:674-679 | Hinnant's algorithm constants | **Same algorithm/constants exist in `scratchpad.rs`'s "civil-from-days" UTC formatter** per schema.rs's own doc comment at line 668-669 ("the exact inverse of the civil-from-days conversion the plugin's UTC formatter runs"). | Two independent hand-rolled implementations – see L1. |
| `19` (fixed ISO-8601 date+time byte length) | schema.rs:536 | bare literal, not a named const | Not found elsewhere. | Fine as local magic number with a comment. |

## Cross-module references

**Imports into this partition:**
- `crate::scratchpad::{format_rfc3339, now_secs, STATE_REL}` – schema.rs (format_rfc3339 only), render.rs (format_rfc3339), verbs.rs (all three). `STATE_REL`/`format_rfc3339`/`now_secs` are themselves re-exports of `ss_magic_core::state_tree`/`ss_magic_core::release::now_secs` inside `scratchpad.rs`.
- `crate::cache::{self, Budget}` – render.rs (`cache::envelope`), verbs.rs (`Budget` only).
- `crate::git` – verbs.rs (`git::cwd_repo_root`, `git::origin_url`).
- `crate::atomic` – verbs.rs (`atomic::write_atomically`).
- `crate::tmproot` – verbs.rs (`tmproot::with_lock`).
- `ss_magic_core::style` – verbs.rs (`style::ok/err/warn/info`).

**Public symbols this partition defines, and where used elsewhere (grep, ≤5 callers each):**

- `checklist::canonicalize`, `UNRANKED_RANK` – re-exported from `mod.rs` but **no caller outside `checklist/` itself found** (order.rs and its own tests use it internally; `write_document` in verbs.rs calls `canonicalize` – that's inside the partition). Unused outside this file family.
- `checklist::render` (the function) – called at `verbs.rs:649` (`render_markdown`) only. Unused outside this file family.
- `checklist::{from_json, to_json, read_document, default_sections, ChangelogEntry, Document, Instant, Item, ItemKind, Priority, Reference, Section, TimeError, Timestamp, SCHEMA_ID, parse_iso8601}` – all consumed inside `verbs.rs`/`render.rs`/`order.rs`/`validate.rs` (intra-partition). No external caller found via grep of `checklist::` outside `checklist/`.
- `checklist::{has_errors, is_absolute_url, is_well_formed_id, validate, Finding, Severity}` – `has_errors`/`validate`/`Finding`/`Severity` used only inside `verbs.rs`. `is_well_formed_id`/`is_absolute_url` used only inside `verbs.rs`/`validate.rs`. No external caller.
- `checklist::{matches_convention, pointer_path, pointer_target, run, Pointer, ACTIONS_REL, CHECKLIST_SUFFIX, POINTER_NAME}` – these ARE used externally:
  - `matches_convention`: `hook/pre_tool_use.rs:457`, `:537`, `:1497` (3 call sites).
  - `pointer_target`: `hook/pre_tool_use.rs:461`, `:547`, `:1489` (3 call sites).
  - `pointer_path`: `status.rs:1312` (1 call site).
  - `run`: `main.rs:532` (`HumanVerb::Checklist => checklist::run(args)`).
  - `ACTIONS_REL`: `setup_ci.rs:55,275,389`; `hook/pre_tool_use.rs:1492` (4 sites).
  - `CHECKLIST_SUFFIX`: `setup_ci.rs:55,275,396` (3 sites).
  - `POINTER_NAME`: **no external caller found** – `hook/session_start.rs:62` re-declares the same string as its own private const instead of importing this one (see L9).
  - `Pointer` (the struct): no external caller found (readers go through `pointer_target`, not the struct directly).

Given the above, most of the module's public surface (documented in `mod.rs`'s comment as "the format's vocabulary rather than leftovers") is genuinely unused outside the `checklist/` family today except the six symbols listed as externally-used.

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `crates/ss-magic-plugin/src/checklist/schema.rs:670-680` (`days_from_civil`, `is_leap`, `days_in_month`) | Hand-rolled Hinnant civil-from-days calendar math duplicates the inverse (civil-from-days → epoch) formatter already in `scratchpad.rs`; schema.rs's own doc comment (line 668) admits this is "the exact inverse" of that other formatter. Two independent constant sets (`146_097`, `719_468`, `400`) for what is mathematically one round-trip. | `crates/ss-magic-plugin/src/scratchpad.rs` – locate its epoch-to-civil UTC formatter (likely near `format_rfc3339`) and compare constants/structure; consider a single shared `ss-magic-core` module owning both directions since both binaries/the plugin need calendar math (core is allowed per structure-pins: "helper shared by both binaries belongs in core" – though here it's plugin-internal reuse, so core is optional, a plugin-local shared module suffices). |
| L2 | copy-paste-variant | `crates/ss-magic-plugin/src/checklist/validate.rs:410-424` (`error`, `warning`) | Two 6-line functions differing only in the `Severity` variant pushed. Could collapse to one `push_finding(findings, severity, at, message)` or a closure factory. | Same file, both functions; no external analog needed. |
| L3 | copy-paste-variant | `crates/ss-magic-plugin/src/checklist/render.rs:152` (`"Untitled checklist"`), `:258` (`"(no summary)"`), `:256` (`"(no date)"`), `:284` (`"(untitled section)"`), `:300` (`"(untitled item)"`) vs `validate.rs:428-434` (`display_id`, `"<no id>"`) | Five near-identical "is this field blank? render a bracketed placeholder instead" call sites in render.rs, plus a sixth analogous helper in validate.rs, none sharing a common `or_placeholder(text, fallback)` helper. | Compare the five render.rs sites against each other first; `display_id` in validate.rs is the same idea one file over. |
| L4 | duplication | `crates/ss-magic-plugin/src/checklist/verbs.rs:163-175` (`contained_join`) | A third "reject absolute path / `..` component, then join under root" implementation in the plugin crate. | `crates/ss-magic-plugin/src/pathnorm.rs` (the crate's designated textual `..`-canceller – pinned by structure-pins as the one way to do this) and `scratchpad.rs:341` (`verify_contained`, canonicalize-based). Note structure-pins says `pathnorm::normalize` is the canonical textual reducer; `contained_join` doesn't call it and instead hand-rolls its own component scan – worth checking whether it should route through `pathnorm` instead of a third bespoke check. |
| L5 | naming / copy-paste-variant | `crates/ss-magic-plugin/src/checklist/verbs.rs:127-136` (`Pointer { path, slug, recorded_at }`) | Same type name `Pointer` as `crates/ss-magic-plugin/src/scratchpad.rs:96-145` (`Pointer { slug, dir, repo, branch, resolved_at }`), same "which X is active" role, same "plain JSON, not a symlink" rationale copied nearly verbatim in both doc comments, but two different structs, two different lock mechanisms (`tmproot::with_lock` here vs raw `fd_lock::RwLock` in scratchpad.rs), and two different atomic-write call shapes. Not necessarily wrong (different data), but the pointer-write plumbing (lock + temp-file + rename) is written twice. | `crates/ss-magic-plugin/src/scratchpad.rs:580-620` (`write_pointer`) vs `crates/ss-magic-plugin/src/checklist/verbs.rs:1303-1321` (`write_pointer`) – same function name, same shape, different lock primitive. Possibly belongs in the plugin-scratchpad partition too; flag as cross-partition. |
| L6 | duplication | `crates/ss-magic-plugin/src/checklist/verbs.rs:1329-1335` (`with_lock`) | "If the state/lock root exists as a directory, take the advisory lock; otherwise just run the closure unlocked" – same conditional-lock idea appears with different concrete lock calls in `config.rs::write_locked` (line 58-61, resolves a lock root first) and `release_check.rs` (`try_with_lock`, non-blocking, lines 258/334). Three call sites hand-rolling "best-effort lock, else proceed" rather than one shared helper in `tmproot.rs`. | `crates/ss-magic-plugin/src/config.rs:53-65`, `crates/ss-magic-plugin/src/release_check.rs:250-275` – likely a different partition (plugin-config / plugin-release-check); flag cross-partition. |
| L7 | duplication | `crates/ss-magic-plugin/src/checklist/verbs.rs:1347-1370` (`browsable_origin`) | Re-implements scheme/scp-style git-remote-URL parsing (`ssh://`, `https?://`, `user@host:path`) that `ss_magic_core::reponame::stem_from_origin` already does for a related purpose (turning a remote into a normalized stem rather than a clickable URL). Different output shape justifies a second function, but the URL-shape-recognition logic (three branches: `ssh://`, `http(s)://`, scp-like) is duplicated reasoning, not duplicated bytes. | `crates/ss-magic-core/src/reponame.rs:47-70` (`stem_from_origin`) – core, so this may cross into a core-partition; flag cross-partition. Low-risk to unify since both need "recognize the remote-URL shape" first. |
| L8 | reuse | `crates/ss-magic-plugin/src/checklist/verbs.rs:1374-1379` (`display_rel`) | `path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned()` is a tiny, generic, repository-relative-display helper defined once here but not reused; several other files independently call `.to_string_lossy().into_owned()` inline for the same "show a path relative to some root, or absolute as fallback" need. | `crates/ss-magic-plugin/src/bypass.rs:124`, `expect_artifact.rs:226`, `ledger.rs:619,1024-1050`, `scratchpad.rs:465` – none of these strip a prefix first, so they're not exact duplicates of `display_rel`, but a shared `display_rel`-style helper in a common module (e.g. `atomic.rs` or a new small `pathdisplay` module) would remove the repeated `.to_string_lossy().into_owned()` idiom. Medium confidence; mostly a reuse opportunity, not a bug. |
| L9 | const-location | `crates/ss-magic-plugin/src/hook/session_start.rs:62` (`const CHECKLIST_POINTER_NAME: &str = "checklist.json";`) | Independently declares the same string value as `checklist::POINTER_NAME` (verbs.rs:101, already `pub`), instead of importing it. A rename of one constant's value would silently desync the other – `session_start.rs`'s guidance text would then name a pointer path the checklist verbs no longer use. | `crates/ss-magic-plugin/src/checklist/verbs.rs:101` (`pub const POINTER_NAME`) – straightforward "import instead of re-literal" fix; verify `session_start.rs` already imports `crate::checklist` for anything else before deciding the fix shape. |
| L10 | naming | `crates/ss-magic-plugin/src/checklist/verbs.rs:105`, `release_check.rs:53`, `hook/file_changed.rs:106` | Three modules each define a private `const LOCK_NAME: &str = "...";` with a different value. Not a bug (module-scoped, no collision at compile time) but the repeated identical *name* for three different concrete strings is worth normalizing (e.g. `CHECKLIST_LOCK_NAME`, `RELEASE_CHECK_LOCK_NAME`) for grep-ability, per the `dup-name` flags already in consts.md. | consts.md rows for `LOCK_NAME` (three entries) – cross-partition; low priority, cosmetic. |
| L11 | naming | `crates/ss-magic-plugin/src/checklist/verbs.rs:866` (`DOCUMENT_ID: &str = "document"`) vs `crates/ss-magic-plugin/src/checklist/validate.rs:92` (`let at = "document";`) | Two independent string literals that must stay byte-identical for a validator finding's `location` to match the id a `set document ...` command accepts; verbs.rs:864-865 documents the coupling in prose but nothing enforces it at compile time (e.g. a shared const). | Both sites are in this partition; a shared `pub(super) const DOCUMENT_ID` visible to both `validate.rs` and `verbs.rs` (currently private to verbs.rs) would remove the "spelled the same" comment in favor of a compiler-checked invariant. |
| L12 | efficiency | `crates/ss-magic-plugin/src/checklist/render.rs:121-148` (`render_body`) | Builds `seen_anchors: HashMap<String, u32>` once and threads it through `plan_sections`/`anchor_for`/`dedupe` correctly (not a bug) – noted only because `dedupe`'s `seen.entry(base.clone()).or_insert(0)` clones `base` on every call even on the always-taken first-insert path; a `HashMap::entry` without the eager clone (e.g. check-then-insert) would save one allocation per anchor. Very low impact (small documents), included for completeness. | `crates/ss-magic-plugin/src/checklist/render.rs:501-509` (`dedupe`) itself; no external comparison needed. |
| L13 | naming | `crates/ss-magic-plugin/src/checklist/validate.rs:428` (`display_id`) vs `crates/ss-magic-plugin/src/checklist/render.rs` placeholder strings (see L3) | Both files solve "missing text → bracketed placeholder" but with different bracket styles (`<no id>` vs `(untitled item)`, `(no summary)`, `(no date)`) – not necessarily wrong, but worth a reviewer glance at whether the two files should share one convention. | Cross-reference only; same partition. |
| L14 | dead-code | `crates/ss-magic-plugin/src/checklist/mod.rs:40-47` (`#![allow(dead_code, unused_imports)]`) plus the "Unused outside this file family" list in Cross-module references | The module-level blanket allow suppresses warnings for a large fraction of the family's public surface (`canonicalize`, `UNRANKED_RANK`, `render`, `from_json`, `to_json`, `read_document`, `default_sections`, `has_errors`, `is_absolute_url`, `validate`, `Finding`, `Severity`, `Pointer`, and most schema types) that grep shows is not called from outside `checklist/` today. The mod.rs doc comment defends this deliberately ("the format's vocabulary rather than leftovers"), so this is very likely a **settled** decision, not a bug – flagged only so the reviewer can weigh the doc's justification against how much of the list is genuinely never touched by any caller, human-typed command, or test outside the family. | `crates/ss-magic-plugin/src/checklist/mod.rs:40-47` and its own doc comment; low confidence this should change given the explicit rationale. |

