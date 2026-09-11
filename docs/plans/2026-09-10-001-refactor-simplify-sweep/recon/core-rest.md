# Recon: core-rest (release, superset_files, sync engine, hashing, reponame, state_tree, style, testutil)

## Map

### `crates/ss-magic-core/src/lib.rs` (33 lines)
Crate root: re-exports the library's modules, nothing else.
- `mod git`, `mod hashing`, `mod release`, `mod reponame`, `mod state_tree`, `mod style`, `mod superset_files`, `mod sync` (all `pub`).
- `mod testutil` (pub, `#[cfg(any(test, feature = "testutil"))]`).

### `crates/ss-magic-core/src/release.rs` (609 lines)
Daily-cached, per-release-line GitHub "is a newer release available?" check.
- `REPO_SLUG: &str` (pub const) – `"ViktorStiskala/superset-magic"`, line 50.
- `FRESH_FOR: Duration` (private const) – `24 * 60 * 60` secs, line 53.
- `HTTP_TIMEOUT: Duration` (private const) – `5` secs, line 57.
- `RELEASES_PER_PAGE: u32` (private const) – `100`, line 64.
- `Line { tag_prefix, cache_file }` (pub struct) – a release line's tag shape and cache-file name.
- `CLI_LINE: Line` (pub const) – `v` / `"version-check.json"`, line 81.
- `PLUGIN_LINE: Line` (pub const) – `ss-magic-plugin-v` / `"plugin-release-check.json"`, line 91.
- `UpdateCheck { UpToDate, Newer { tag } }` (pub enum) – infallible verdict.
- `ReleaseEntry { tag_name, draft, prerelease }` (pub struct) – minimal `/releases` entry projection.
- `FetchOutcome { Ok, NotModified, Failed }` (pub enum) – normalized HTTP outcome.
- `trait ReleaseClient { fn fetch_releases(&self, etag) -> FetchOutcome }` (pub) – HTTP seam.
- `Cache { checked_at, tag_name, etag, suggested }` (pub struct) – on-disk record.
  - `Cache::is_fresh(&self, now: u64) -> bool` (pub method) – within `FRESH_FOR`.
- `now_secs() -> u64` (pub fn) – current unix time, saturating to 0.
- `read_cache(path: &Path) -> Option<Cache>` (pub fn) – any error → `None`.
- `write_cache(path: &Path, cache: &Cache)` (private fn) – best-effort write; swallows errors. *Small, generic – candidate to unify with the other "best-effort JSON write" helpers elsewhere (compare to `superset_files::write_atomically`'s stronger atomic version, and the plugin's `atomic::write_atomically`).*
- `is_fresh(checked_at: u64, now: u64) -> bool` (private fn) – small, generic; only one caller (`Cache::is_fresh`).
- `parse_line_tag(line: &Line, tag: &str) -> Option<(u64,u64,u64)>` (pub fn) – anchored tag filter.
- `parse_bare_triple(s: &str) -> Option<(u64,u64,u64)>` (pub fn) – parses `MAJOR.MINOR.PATCH`.
- `ascii_digits(s: &str) -> Option<u64>` (private fn) – small, generic digit-only parse helper.
- `select_newest(line: &Line, releases: &[ReleaseEntry]) -> Option<String>` (pub fn) – greatest triple wins.
- `resolve_newest_uncached<C: ReleaseClient>(client: &C, line: &Line) -> Option<String>` (pub fn) – no-cache resolve, used by forced update.
- `is_newer(line: &Line, tag: &str, current: &str) -> bool` (pub fn) – numeric triple compare.
- `RefreshOutcome { Fetched, NotModified, Failed }` (pub enum).
- `Refresh { cache: Cache, outcome: RefreshOutcome }` (pub struct).
- `refresh_cache<C: ReleaseClient>(prior, client, line, now) -> Refresh` (pub fn) – the one "next cache from prior + fetch" derivation.
- `run_check<C: ReleaseClient>(cache_file, client, line, current_version) -> UpdateCheck` (pub fn) – testable core of the daily check.
- `verdict_from_tag(line: &Line, tag: &str, current: &str) -> UpdateCheck` (private fn) – small, generic tag→verdict mapper.
- `cache_dir() -> Option<PathBuf>` (pub fn) – app-scoped OS cache dir, creates it.
- `existing_cache_dir() -> Option<PathBuf>` (pub fn) – non-creating variant, for read-only diagnostics.
- `cache_dir_path() -> Option<PathBuf>` (private fn) – shared resolver behind both of the above.
- `UreqReleaseClient { url, user_agent }` (pub struct) – real ureq/rustls client.
  - `UreqReleaseClient::new(user_agent_version: &str) -> Self` (pub) – CLI shorthand → `for_product("ss-magic", …)`.
  - `UreqReleaseClient::for_product(product: &str, version: &str) -> Self` (pub).
  - `impl ReleaseClient for UreqReleaseClient::fetch_releases` (private impl method).
- `check(current_version: &str) -> UpdateCheck` (pub fn) – wires real client + real cache path + `CLI_LINE`.

### `crates/ss-magic-core/src/superset_files.rs` (501 lines)
`.superset/{config.json, magic.sh, setup_config.json, magic.json, magic.local.json}` I/O.
- `MAGIC_SH: &str` (pub const) – `include_str!("../../../assets/magic.sh")`, line 24.
- `SUPERSET_DIR` (private const) – `".superset"`, line 26.
- `CONFIG_JSON` (private const) – `"config.json"`, line 27.
- `MAGIC_SH_NAME` (private const) – `"magic.sh"`, line 28.
- `SETUP_CONFIG_JSON` (private const) – `"setup_config.json"`, line 29.
- `MAGIC_JSON` (private const) – `"magic.json"`, line 30.
- `MAGIC_LOCAL_JSON` (private const) – `"magic.local.json"`, line 31.
- `MAGIC_LOCAL_PATTERN` (private const) – `".superset/magic.local.json"`, line 35.
- `Config { setup, teardown, run }` (pub struct) – shape of `config.json`.
- `SetupConfig { files }` (pub struct) – legacy `setup_config.json`, read-only.
- `MagicConfig { files, extras }` (pub struct) – shape of `magic.json`/`magic.local.json`; `extras` is `#[serde(flatten)]`.
- `load_overlaid(root: &Path) -> Result<Option<MagicConfig>>` (pub fn) – union+dedupe overlay.
- `write_magic_json(root: &Path, cfg: &MagicConfig) -> Result<()>` (pub fn) – atomic write.
- `write_magic_local_json(root: &Path, cfg: &MagicConfig) -> Result<()>` (pub fn) – atomic write.
- `write_atomically(path: &Path, body: &str) -> Result<()>` (private fn) – staged-sibling + rename, canonicalizes through symlinks first; has its own `SEQ: AtomicU64` static inside the function body (line 198). *Duplicate-in-spirit of the plugin crate's `atomic::write_atomically` (see Cross-module references) – noted in the brief as legitimate to report since core cannot depend on the plugin.*
- `merge_files_into_magic_config(existing: Option<&MagicConfig>, new_files: Vec<String>) -> MagicConfig` (pub fn).
- `default_magic_files() -> Vec<String>` (pub fn, `#[allow(dead_code)]`, "consumed by U9").
- `bootstrap_magic_local_json(root: &Path) -> Result<()>` (pub fn, `#[allow(dead_code)]`, "consumed by U9") – writes RAW JSON with a hand-written `_comment` body string (line 265), bypassing `MagicConfig`/serde and `write_atomically` both.
- `superset_dir(root: &Path) -> PathBuf` (private fn) – `root.join(SUPERSET_DIR)`; small, generic, called from nearly every fn in this file.
- `load_config(root: &Path) -> Result<Option<Config>>` (pub fn).
- `load_magic_json(root: &Path) -> Result<Option<MagicConfig>>` (pub fn).
- `load_magic_local_json(root: &Path) -> Result<Option<MagicConfig>>` (pub fn).
- `load_setup_config(root: &Path) -> Result<Option<SetupConfig>>` (pub fn).
- `read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>>` (private fn) – shared loader behind the four `load_*` functions above.
- `ensure_superset_dir(root: &Path) -> Result<()>` (pub fn).
- `write_magic_sh(root: &Path) -> Result<()>` (pub fn, `#[allow(dead_code)]`) – plain `fs::write` (NOT `write_atomically`), then `chmod_executable`.
- `chmod_executable(path: &Path) -> Result<()>` (private fn, `#[cfg(unix)]` + `#[cfg(not(unix))]` twin, lines 366/377).
- `write_config_json(root: &Path, cfg: &Config) -> Result<()>` (pub fn) – plain `fs::write` (NOT `write_atomically`), unlike the two `magic*.json` writers above.
- `merge_setup_into_config(existing: Option<&Config>, new_setup: Vec<String>) -> Config` (pub fn) – same preservation shape as `merge_files_into_magic_config`.
- `copy_into_repo(stage_root: &Path, repo_root: &Path, delete: &[&str]) -> Result<()>` (pub fn) – materializes staged `.superset/`, `config.json` written last.
- `existing_unknown_entries(existing: &[String], options: &[&str]) -> Vec<String>` (pub fn).

### `crates/ss-magic-core/src/sync/mod.rs` (87 lines)
The excluded-trees rule shared by every enumeration layer, plus the sync module's pure root.
- `mod apply`, `mod pattern`, `mod repo_scan` (pub).
- `EXCLUDED_TREES: [&[&str]; 4]` (pub const) – `.superset/backups`, `.superset/.magic`, `.scratchpad`, `.git`, line 40.
- `under_excluded_tree(rel: &Path) -> bool` (pub fn) – the shared predicate.
- `starts_with_components(rel: &Path, tree: &[&str]) -> bool` (private fn) – small, generic component-prefix matcher; single caller today.

### `crates/ss-magic-core/src/sync/apply.rs` (365 lines)
The glob/exclude/copy engine (`ss-magic sync`, reverse sync's `match_paths`).
- `DEFAULT_EXCLUDES: [&str; 2]` (private const) – `["node_modules", ".venv"]`, line 28. *See `sync/repo_scan.rs::SKIP_DIRS` and Leads – three different exclusion lists live in this one module tree.*
- `Summary { copied, skipped }` (pub struct).
- `SkipReason { AbsolutePathRejected, ParentSegmentRejected, BadGlob, MissingLiteral, NoMatches, Excluded, NotAFileOrDir, CopyFailed }` (pub enum).
  - `SkipReason::counts(&self) -> bool` (pub method).
  - `SkipReason::label(&self) -> &str` (pub method).
- `Event { Copy { rel }, Skip { reason, label } }` (pub enum).
- `run<F: FnMut(&Event)>(src, dest, patterns, on_event) -> Result<Summary>` (pub fn) – the copy driver.
- `match_paths(root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>>` (pub fn, `#[allow(dead_code)]`, "consumed by U11") – same semantics as `run`'s match phase, no copying.
- `expand_patterns<F>(src, patterns, on_event) -> Result<Vec<PathBuf>>` (private fn) – shared by `run` and `match_paths`.
- `walk_source(src: &Path) -> Vec<PathBuf>` (private fn) – one walk, prunes `EXCLUDED_TREES` via `filter_entry`.
- `build_matcher(pattern: &str) -> Result<GlobMatcher>` (private fn) – small, generic; wraps `Glob::new(...).compile_matcher()`, mirrors what `pattern::check_syntax` and `repo_scan::matches_for_patterns` each also do with `Glob::new`.
- `is_excluded(rel: &Path) -> bool` (private fn) – ORs `under_excluded_tree` with a `DEFAULT_EXCLUDES` name scan.
- `copy_dir_recursive(root: &Path, src: &Path, dst: &Path) -> Result<()>` (pub(crate) fn) – recursive copy, re-applies `under_excluded_tree` via `filter_entry` (its own enforcement point, by design – see structure-pins).

### `crates/ss-magic-core/src/sync/pattern.rs` (63 lines)
Shared pattern-syntax checks used by both apply-mode and the CLI's bootstrap UI validator.
- `has_glob_meta(s: &str) -> bool` (pub fn).
- `has_parent_segment(s: &str) -> bool` (pub fn).
- `SyntaxError { Empty, AbsolutePath, ParentSegment, BadGlob }` (pub enum).
  - `SyntaxError::label(&self) -> String` (pub method).
- `check_syntax(pattern: &str) -> Result<(), SyntaxError>` (pub fn) – calls `Glob::new(pattern)` itself when `has_glob_meta`, line 55 – a second, independent glob-compile call from the one `apply::build_matcher` does later for the same pattern.

### `crates/ss-magic-core/src/sync/repo_scan.rs` (79 lines)
Scans the working tree for the bootstrap MultiSelect's preselect/no-match state.
- `OPTIONS: [&str; 4]` (pub const) – `[".env", "**/.env", ".env.local", "**/.dev.vars"]`, line 14. *Flagged `dup-name` in the workspace index – a private test-local `const OPTIONS` with the identical value is redeclared in `superset_files/tests.rs:63` instead of importing this one.*
- `SKIP_DIRS: [&str; 4]` (private const) – `["node_modules", ".venv", ".git", "target"]`, line 16. *Overlaps `apply::DEFAULT_EXCLUDES` (`node_modules`, `.venv`) but is a distinct, wider list that is NEVER checked against `sync::EXCLUDED_TREES` – this walk does not prune `.superset/backups` or `.superset/.magic`. Low stakes (UI preselection only, not a copy path) but worth a reviewer's eye – see Leads.*
- `matches_for_patterns(root: &Path, patterns: &[&str]) -> Result<Vec<bool>>` (pub fn).
- `pattern_matches_any(root: &Path, pattern: &str) -> Result<bool>` (pub fn) – one-pattern wrapper over the above.
- `skip_excluded(entry: &walkdir::DirEntry) -> bool` (private fn) – `filter_entry` predicate using `SKIP_DIRS`, not `under_excluded_tree`.

### `crates/ss-magic-core/src/hashing.rs` (170 lines)
FNV-1a (cache keys) + hand-rolled SHA-256 (must byte-match the shell bootstrap's `shasum -a 256`).
- `FNV_OFFSET_BASIS: u64` (private const) – `0xcbf29ce484222325`, line 36.
- `FNV_PRIME: u64` (private const) – `0x100000001b3`, line 38.
- `fnv1a_64(bytes: &[u8]) -> u64` (pub fn).
- `hash_file(path: &Path) -> Result<u64>` (pub fn) – whole-file FNV-1a.
- `SHA256_H0: [u32; 8]` (private const) – the eight IVs, line 64.
- `SHA256_K: [u32; 64]` (private const) – the 64 round constants, line 71.
- `sha256(data: &[u8]) -> [u8; 32]` (pub fn) – from-scratch FIPS 180-4.
- `sha256_hex(data: &[u8]) -> String` (pub fn) – hex-encodes `sha256`'s output.

### `crates/ss-magic-core/src/reponame.rs` (105 lines)
The `<repo>` stem shared by the pack archive name and the plugin's session identity.
- `repo_name_stem(root: &Path) -> Option<String>` (pub fn) – origin URL, else main-checkout basename.
- `stem_from_origin(url: &str) -> Option<String>` (pub fn) – normalizes any remote URL form to a stem.
- `final_segment(s: &str) -> &str` (private fn) – small, generic "last `/`-segment" helper; only one caller today, but the same operation ss-magic's own `pack.rs`/URL-handling code may need – worth a grep before assuming it is unique.
- `sanitize_segment(seg: &str) -> String` (pub fn) – lowercases + collapses non-alphanumerics to `-`.

### `crates/ss-magic-core/src/state_tree.rs` (51 lines)
The `.superset/.magic` path constant and its one gitignore writer.
- `STATE_REL: &str` (pub const) – `".superset/.magic"`, line 35.
- `ensure_state_ignored(root: &Path) -> Result<()>` (pub fn) – wraps `git::gitignore::ensure_path_ignored`.

### `crates/ss-magic-core/src/style.rs` (127 lines)
Color palette + the process-wide color decision (no `inquire`).
- `Kind { Info, Ok, Warn, Err, Header }` (pub enum).
  - `Kind::ansi(self) -> &'static str` (private method) – the five literal ANSI escapes.
- `ANSI_RESET: &str` (private const) – `"\x1b[0m"`, line 49.
- `COLOR_ENABLED: OnceLock<bool>` (private static), line 51.
- `detect() -> bool` (private fn) – `NO_COLOR` then `supports_color::on`.
- `init()` (pub fn) – `get_or_init(detect)`.
- `init_no_color()` (pub fn) – `get_or_init(|| false)`, used by hook invocations.
- `enabled() -> bool` (pub fn).
- `paint(text: &str, kind: Kind, enabled: bool) -> String` (pub fn).
- `role<S: Display>(s: S, kind: Kind) -> String` (private fn) – small, generic; `paint(&s.to_string(), kind, enabled())`, backs the five role helpers below.
- `info`, `ok`, `warn`, `err`, `header` (all pub fn, `<S: Display>(s: S) -> String`) – five near-identical one-liners, all delegating to `role`.
- `print_section(title: &str)` (pub fn) – prints a cyan `── title ──` banner.

### `crates/ss-magic-core/src/testutil.rs` (232 lines)
Shared test-only fixtures for every crate (`cfg(test)` here, `testutil` feature for binaries).
- `neutralize_global_excludes(repo_root: &Path)` (pub fn).
- `git_run(args: &[&str], cwd: &Path)` (pub fn) – isolated identity + config, asserts success.
- `exit_code_to_u8(code: ExitCode) -> u8` (pub fn).
- `init_main_repo(branch: &str) -> TempDir` (pub fn) – git init + `gc.auto 0` + neutralize + one commit.
- `write_magic(root: &Path, patterns: &[&str])` (pub fn) – writes `.superset/magic.json`.
- `write_file(root: &Path, rel: &str, body: &str)` (pub fn).
- `make_worktree(main_dir: &Path) -> (TempDir, PathBuf)` (pub fn) – `git worktree add -b feature/sync-flow-test`.
- `run_ignored_test_in_child(test_path, env, remove) -> String` (pub fn).
- `run_ignored_test_in_child_from(cwd, test_path, env, remove) -> String` (pub fn).
- `run_child(cwd: Option<&Path>, test_path, env, remove) -> String` (private fn) – shared by both `run_ignored_test_in_child*` entry points.
- `test_path_in_binary(module_path: &str, child: &str) -> String` (pub fn).

## Constants and literals

| Const/literal | file:line | Value | What it denotes | Elsewhere in workspace? |
|---|---|---|---|---|
| `REPO_SLUG` | `release.rs:50` | `"ViktorStiskala/superset-magic"` | GitHub repo slug for both release lines | Same literal in `release/tests.rs` (2 places, per `literals.md`); this is the one production owner. |
| `FRESH_FOR` | `release.rs:53` | `Duration::from_secs(24*60*60)` (86400s) | cache freshness window | The bare number `86_400` appears elsewhere in the workspace (`reverse_sync.rs:708-709`, `tui/cockpit.rs:719/722`, `checklist/schema.rs:587`, `release_check.rs:564`, `scratchpad.rs:656-657`) but those are all a *day-of-seconds arithmetic* idiom (Hinnant's civil-from-days), unrelated to this cache TTL – coincidence, not duplication. |
| `HTTP_TIMEOUT` | `release.rs:57` | `Duration::from_secs(5)` | per-request network budget | `status.rs:152` (`VERSION_TIMEOUT`, plugin crate) is also `Duration::from_secs(5)` for an unrelated purpose (bounding a `--version` subprocess probe) – same value, unrelated owners, not a real duplication but worth a glance. |
| `RELEASES_PER_PAGE` | `release.rs:64` | `100` (`u32`) | `/releases?per_page=100` | Not seen elsewhere. |
| `CLI_LINE` / `PLUGIN_LINE` | `release.rs:81`, `release.rs:91` | `Line{tag_prefix, cache_file}` values | the two release lines' tag shapes and cache filenames | `CLI_LINE` used in 3 files, `PLUGIN_LINE` in 7 (mostly plugin crate) – this is the sole owner, correct location per the pinned "grouped by release line" doc. |
| `MAGIC_SH` | `superset_files.rs:24` | `include_str!("../../../assets/magic.sh")` | embedded wrapper script body | `assets/magic.sh` is documented as canonical (CLAUDE.md, "Source of truth for magic.sh") – correctly the one owner. |
| `SUPERSET_DIR` | `superset_files.rs:26` | `".superset"` | the workspace-contract directory name | **Same literal value** owned independently by `SUPERSET_REL` in `crates/ss-magic-plugin/src/scratchpad.rs:70` (private const, same value, different name). Two crates each define their own `".superset"` constant. See Leads L9. |
| `CONFIG_JSON` | `superset_files.rs:27` | `"config.json"` | Superset-owned setup/teardown/run file | literal `"config.json"` appears standalone at `literals.md:159` (3 files, 16 occurrences) – all in this file and its own tests; no other production owner. |
| `MAGIC_SH_NAME` | `superset_files.rs:28` | `"magic.sh"` | on-disk wrapper filename | literal `"magic.sh"` appears in `literals.md:71` (2 files, 4 sites / 11 total incl. tests) – `superset_files.rs` and its tests only. |
| `SETUP_CONFIG_JSON` | `superset_files.rs:29` | `"setup_config.json"` | legacy read-only file | `literals.md:170` – 3 files, all this module + tests. |
| `MAGIC_JSON` | `superset_files.rs:30` | `"magic.json"` | committed sync-pattern file | `literals.md:62` – 6 files but all in this module + its own and `sync`/plugin test files that build the literal path themselves rather than importing the const (see Leads). |
| `MAGIC_LOCAL_JSON` | `superset_files.rs:31` | `"magic.local.json"` | gitignored overlay file | `literals.md:35` (`.superset/magic.local.json`, 10 file-groups) – mostly test fixtures spelling the full relative path as a literal string instead of joining `SUPERSET_DIR`+`MAGIC_LOCAL_JSON` (expected in test code, flagged only for awareness). |
| `MAGIC_LOCAL_PATTERN` | `superset_files.rs:35` | `".superset/magic.local.json"` | the pattern default-seeded into `magic.json` | This is the ONE production place `SUPERSET_DIR`/`MAGIC_LOCAL_JSON` get concatenated into the full relative form as a literal rather than a `format!`/`Path::join` of the two – see Leads L10. |
| `write_atomically`'s temp suffix | `superset_files.rs:200` | `".{name}.{pid}.{seq}.tmp"` | staged-sibling filename shape | Compare to the plugin's own `atomic::write_atomically` staging scheme (different crate, different file) – see Leads L1. |
| `DEFAULT_EXCLUDES` | `sync/apply.rs:28` | `["node_modules", ".venv"]` | dirs dropped from sync matches at any depth | `node_modules`/`.venv` literals appear again in `sync/repo_scan.rs:16` (`SKIP_DIRS`, superset of this list) and in test fixtures (`literals.md:90,92`) – see Leads L2. |
| `EXCLUDED_TREES` | `sync/mod.rs:40` | `[[".superset","backups"], [".superset",".magic"], [".scratchpad"], [".git"]]` | whole trees no walk may yield | The correct single owner (per structure-pins); mirrored by name in plugin's `scratchpad.rs` comments (not re-declared, just referenced) and by `state_tree::STATE_REL` (pinned-equal by a test). |
| `OPTIONS` | `sync/repo_scan.rs:14` | `[".env","**/.env",".env.local","**/.dev.vars"]` | bootstrap MultiSelect default patterns | Re-declared byte-for-byte as a private `const OPTIONS` in `superset_files/tests.rs:63` instead of importing `repo_scan::OPTIONS` – see Leads L11. Also the literal name `OPTIONS` collides (unrelated meaning) with CLI usage-string placeholders in several plugin files (`ledger.rs`, `spill_index.rs`, `status.rs`, `cli.rs`) – those are just the word "OPTIONS" in `Usage:` text, not a value collision. |
| `SKIP_DIRS` | `sync/repo_scan.rs:16` | `["node_modules",".venv",".git","target"]` | dirs pruned from the preselect scan | Only declared here (`functions.md`/`consts.md` show 1 file) – see Leads L2/L3 for its relationship to `DEFAULT_EXCLUDES` and `EXCLUDED_TREES`. |
| `FNV_OFFSET_BASIS` / `FNV_PRIME` | `hashing.rs:36,38` | `0xcbf29ce484222325` / `0x100000001b3` | FNV-1a constants | Standard algorithm constants, only owner; pinned by structure-pins (do not propose `DefaultHasher`). |
| `SHA256_H0` / `SHA256_K` | `hashing.rs:64,71` | standard IVs / round constants | FIPS 180-4 constants | Only owner; pinned (must match shell `shasum`). |
| `ANSI_RESET` | `style.rs:49` | `"\x1b[0m"` | reset escape | The five `Kind::ansi()` escapes (lines 40-44) and this reset are the ONE place ANSI codes are spelled – good, no duplication found elsewhere in the workspace for these exact byte sequences. |
| `STATE_REL` | `state_tree.rs:35` | `".superset/.magic"` | plugin state tree path | Pinned-equal by test to the `[".superset",".magic"]` entry of `EXCLUDED_TREES` (structure-pins says this is deliberate, not a finding). 13 files reference `STATE_REL` by name (mostly plugin crate) – correctly the one owner. |
| `write_atomically`'s AtomicU64 `SEQ` | `superset_files.rs:198` | `AtomicU64::new(0)` | per-process write-sequence counter, declared as a `static` INSIDE the function body | Function-local statics are unusual placement; only one in this file. Not a value duplication, but a style oddity worth a reviewer glance (see Leads L12). |

## Cross-module references

**What these files import from elsewhere:**
- `release.rs` – `serde::{Deserialize, Serialize}`, `ureq`, `directories::ProjectDirs`, `serde_json`. No intra-crate imports beyond `std`.
- `superset_files.rs` – `anyhow::{bail, Context, Result}`, `serde`, `serde_json`, `assets/magic.sh` via `include_str!`. No other core module.
- `sync/mod.rs` / `apply.rs` – `globset`, `walkdir`; `apply.rs` imports `crate::sync::pattern` and calls `crate::sync::under_excluded_tree`.
- `sync/pattern.rs` – `globset::Glob` only.
- `sync/repo_scan.rs` – `globset`, `walkdir`, `anyhow`.
- `hashing.rs` – `anyhow::{Context, Result}`, `std::path::Path`. Fully standalone otherwise.
- `reponame.rs` – `crate::git` (for `git::origin_url`, `git::main_checkout_root`).
- `state_tree.rs` – `crate::git::gitignore::{self, PathKind}`, `anyhow::Result`.
- `style.rs` – `supports_color::Stream`.
- `testutil.rs` – `tempfile::TempDir`, `std::process::Command`; calls into `crate::superset_files::MagicConfig` (from `write_magic`).

**Where these files' public symbols are used elsewhere (grep, ≤5 callers each):**

- `reponame::repo_name_stem` – `ss-magic/src/pack.rs:43` (used, then re-exported at `pack.rs:50` as `pub(crate) use`); `ss-magic-plugin/src/identity.rs:76`; test call sites in `ss-magic-plugin/src/identity/tests.rs:176,201`.
- `reponame::stem_from_origin` – no external caller found outside `reponame.rs` itself and its own tests; used indirectly through `repo_name_stem`. *Effectively unused outside this file as a direct call, though it is `pub` and covered by `reponame/tests.rs`.*
- `reponame::sanitize_segment` – `ss-magic/src/pack/tests.rs:3,524` (imported directly in a test).
- `sync::EXCLUDED_TREES` – referenced (not re-declared) in `ss-magic/src/pack.rs:169,344` (comments) and `ss-magic-plugin/src/scratchpad.rs:58,117` (comments); the actual predicate consumers are `sync::apply::{walk_source,copy_dir_recursive}` (same crate) and `ss-magic`'s `sync/reverse_sync.rs` + `pack.rs::append_dir_excluding_trees` (other partitions, per structure-pins "applied at every point of final enumeration").
- `sync::under_excluded_tree` – same call sites as above; 8 files reference it workspace-wide.
- `sync::apply::run` / `match_paths` / `copy_dir_recursive` – called from `ss-magic/src/sync/mod.rs` (re-export), `ss-magic/src/sync/reverse_sync.rs`, `ss-magic/src/main.rs` (forward sync flow), `ss-magic/src/pack.rs` (`append_dir_excluding_trees`, via `copy_dir_recursive`).
- `sync::pattern::check_syntax` – `ss-magic/src/tui/ui.rs` (bootstrap validator), `sync/apply.rs` (own crate).
- `sync::repo_scan::{OPTIONS, matches_for_patterns, pattern_matches_any}` – `ss-magic/src/workspace/migrate.rs:408-483` and `migrate/tests.rs` (many sites, one representative block shown above).
- `superset_files::load_overlaid` – `ss-magic/src/main.rs:221`, `ss-magic/src/sync/reverse_sync.rs:107,179`, `ss-magic-plugin/src/config.rs:218,697`.
- `superset_files::{write_magic_json, write_magic_local_json, load_magic_json, load_magic_local_json, merge_files_into_magic_config}` – `ss-magic/src/workspace/migrate.rs` (multiple call sites, e.g. lines 285, 512, 562), `ss-magic-plugin/src/config.rs:848` (the `seed-config`/`enable`/`config set` write path).
- `superset_files::{load_config, write_config_json, merge_setup_into_config}` – `ss-magic/src/workspace/migrate.rs` only (init/migrate flows).
- `superset_files::copy_into_repo` – `ss-magic/src/workspace/migrate.rs:359,526,575`.
- `superset_files::existing_unknown_entries` – `ss-magic/src/workspace/migrate.rs:431` and `ss-magic-plugin` (via `crate::sync::repo_scan`-adjacent flows, same function reused for command-preservation in migrate).
- `superset_files::{default_magic_files, bootstrap_magic_local_json, write_magic_sh}` – each `#[allow(dead_code)]`-marked "consumed by U9"; grep shows real call sites in `ss-magic/src/workspace/migrate.rs` init/migrate paths (4-5 files touch the names, mostly comments/tests) – genuinely wired in, the `#[allow(dead_code)]` markers look stale (see Leads L13).
- `hashing::{fnv1a_64, hash_file}` – used by `ss-magic/src/sync/reverse_sync.rs` (mtime-less TOCTOU fallback) and by several plugin modules (`cache.rs`, `ledger.rs` per module docs) – 5 and 4 files respectively.
- `hashing::sha256_hex` – `ss-magic-plugin/src/tmproot.rs` (per module docs: the per-machine temp-root identifier) – 4 files reference the name.
- `state_tree::{STATE_REL, ensure_state_ignored}` – `ss-magic/src/workspace/migrate.rs::ensure_bootstrap_gitignores` (eager path), `ss-magic-plugin/src/config.rs` (`enable`/`config set`, lazy path), `ss-magic-plugin/src/scratchpad.rs` (re-exports both under old names).
- `style::{init, init_no_color, enabled, paint, info, ok, warn, err, header, print_section}` – `ss-magic/src/main.rs` (~20 call sites via `tui::style::…`), `ss-magic/src/tui/theme.rs` (`style::enabled()`), `ss-magic-plugin/src/main.rs:452` (`style::init()`), `ss-magic-plugin/src/cache.rs:934` (`style::print_section`), `ss-magic/src/workspace/migrate.rs:315,464` (`style::print_section`).
- `testutil::{git_run, neutralize_global_excludes, init_main_repo, write_magic, write_file, make_worktree, run_ignored_test_in_child*, test_path_in_binary}` – imported across many `tests.rs` files in both binaries; see Leads L4-L8 for cases where a caller re-implements one of these instead of importing it.
- `release::{check, run_check, refresh_cache, read_cache, now_secs, cache_dir, existing_cache_dir, UreqReleaseClient::for_product, PLUGIN_LINE}` – `ss-magic/src/update/mod.rs` and `update/apply.rs` (the CLI's forced-update + gate), `ss-magic-plugin/src/release_check.rs` and `hook/session_start.rs` (the plugin's `release-check` verb and `SessionStart` notice, per module docs – `read_cache`/`now_secs` reached directly, never through `run_check`, by design).

## Leads

| # | category | file:line(s) | Hypothesis | Compare against |
|---|---|---|---|---|
| L1 | duplication | `crates/ss-magic-core/src/superset_files.rs:180-213` vs `crates/ss-magic-plugin/src/atomic.rs:51` | Two independent "temp-file-then-rename" atomic writers exist (core's private `write_atomically`, the plugin's `pub(crate) write_atomically`). Per structure-pins this is a known, legitimate-to-report duplication: core cannot depend on the plugin, so a shared copy would need to live in core and be reused by both, or the plugin's copy is intentionally separate. Worth a design note either way. | `crates/ss-magic-plugin/src/atomic.rs:51` (generalized helper with mode + fsync + suffix params) |
| L2 | hardcoded-value / const-location | `crates/ss-magic-core/src/sync/apply.rs:28` (`DEFAULT_EXCLUDES = ["node_modules", ".venv"]`) vs `crates/ss-magic-core/src/sync/repo_scan.rs:16` (`SKIP_DIRS = ["node_modules", ".venv", ".git", "target"]`) | Two directory-name exclusion lists in the same `sync` module tree overlap on two entries but diverge on two more (`.git`, `target`), with no shared definition or comment cross-referencing the other. A reader has no way to tell if the divergence is deliberate (repo_scan's UI-preselect walk vs apply's copy-time exclude) or drift. | `sync/apply.rs:28` vs `sync/repo_scan.rs:16`; also compare both to `sync/mod.rs:40` (`EXCLUDED_TREES`, a third, structurally different exclusion mechanism) |
| L3 | hardcoded-value | `crates/ss-magic-core/src/sync/repo_scan.rs:69-76` (`skip_excluded`) | `repo_scan`'s working-tree walk (used only to preselect bootstrap UI options) prunes `SKIP_DIRS` but never checks `sync::under_excluded_tree` – so scanning for `.env` matches could in principle walk into `.superset/backups/` (recovered secrets) while deciding UI preselection. Low severity since nothing is copied or displayed from inside, but it is the one walk in `sync/` that does not apply the shared exclusion predicate other walks in this same file's sibling module apply everywhere else. | `sync/apply.rs:273-293` (`walk_source`, which DOES call `crate::sync::under_excluded_tree`) |
| L4 | duplication | `crates/ss-magic/src/pack/tests.rs:30-45` (`write_magic`, `write_file`) | Byte-for-byte duplicates of `ss_magic_core::testutil::write_magic` / `testutil::write_file` (identical signatures and bodies), re-implemented locally instead of imported, in a file that already imports `testutil::git_run` from the same module. | `crates/ss-magic-core/src/testutil.rs:100-116` |
| L5 | duplication | `crates/ss-magic/src/sync/reverse_sync/tests.rs:32-45` (`write` at line 32, `write_magic` at line 38) | Same pattern as L4: `write_magic` is a byte-for-byte duplicate of `testutil::write_magic`; `write` (renamed from `write_file`) is a byte-for-byte duplicate of `testutil::write_file`, in a file that already imports `testutil::git_run`. | `crates/ss-magic-core/src/testutil.rs:100-116` |
| L6 | copy-paste-variant | `crates/ss-magic/src/sync/reverse_sync/tests.rs:6-13` (`init_main_repo`) | Near-duplicate of `testutil::init_main_repo`, but hardcodes branch `"main"` (testutil's takes `branch: &str`) and omits the `git config gc.auto 0` call testutil's version added specifically to avoid a background-gc race – so this local copy is missing a fix already made upstream. | `crates/ss-magic-core/src/testutil.rs:81-97` |
| L7 | duplication | `crates/ss-magic/src/sync/reverse_sync/tests.rs:16-30` (`make_worktree`) | Byte-for-byte duplicate of `testutil::make_worktree` (identical body, same hardcoded branch name `"feature/sync-flow-test"` vs this copy's `"feature/rs-test"` – only the branch literal differs). | `crates/ss-magic-core/src/testutil.rs:121-137` |
| L8 | copy-paste-variant | `crates/ss-magic-core/src/sync/repo_scan/tests.rs:5-9` (`write_file(root: &TempDir, rel: &str)`) | A third variant of the same "write a file, creating parent dirs" helper, with a narrower signature (`&TempDir` not `&Path`, no body param, hardcoded `"x"` content) than `testutil::write_file`. Lower priority than L4/L5 since the signature genuinely differs, but still one more copy of the same three-line idiom. | `crates/ss-magic-core/src/testutil.rs:112-116` |
| L9 | const-location | `crates/ss-magic-core/src/superset_files.rs:26` (`SUPERSET_DIR = ".superset"`) vs `crates/ss-magic-plugin/src/scratchpad.rs:70` (`SUPERSET_REL = ".superset"`) | Same literal value, two independently-declared private consts in two different crates, under two different names. `state_tree::STATE_REL` shows the pattern this workspace otherwise follows (one core const, re-exported/compared-equal by a pinned test) – `.superset` itself has no such shared owner. | `crates/ss-magic-core/src/superset_files.rs:26` vs `crates/ss-magic-plugin/src/scratchpad.rs:70` |
| L10 | const-location | `crates/ss-magic-core/src/superset_files.rs:35` (`MAGIC_LOCAL_PATTERN = ".superset/magic.local.json"`) | This is the one production spot where `SUPERSET_DIR` + `MAGIC_LOCAL_JSON` are concatenated as a fresh literal instead of via `format!("{SUPERSET_DIR}/{MAGIC_LOCAL_JSON}")` or a `Path::join` – a future rename of either constituent constant would not touch this one automatically. | `crates/ss-magic-core/src/superset_files.rs:26,31,35` (all three consts, same file) |
| L11 | duplication | `crates/ss-magic-core/src/superset_files/tests.rs:63` (local `const OPTIONS`) | Redeclares `sync::repo_scan::OPTIONS`'s exact array value under the same name instead of `use`-ing it, in a test file that already sits in the same crate as the real const. | `crates/ss-magic-core/src/sync/repo_scan.rs:14` |
| L12 | naming / quality | `crates/ss-magic-core/src/superset_files.rs:198` (`static SEQ: AtomicU64` declared inside `write_atomically`'s function body) | A function-local `static` is an unusual spot for a process-wide sequence counter; moving it to module scope (as `FNV_OFFSET_BASIS` etc. are) would read more consistently with every other const/static in this file and module, though behavior is identical either way (function-locals of this shape are still one instance per program). Low-confidence style note, not a behavior finding. | `crates/ss-magic-core/src/style.rs:51` (`static COLOR_ENABLED`, correctly module-scoped) |
| L13 | dead-code (maybe stale annotation) | `crates/ss-magic-core/src/superset_files.rs:242-245,255-256,356-357` (`default_magic_files`, `bootstrap_magic_local_json`, `write_magic_sh`, all `#[allow(dead_code)]` "consumed by U9") | Grep shows these ARE reached from `ss-magic/src/workspace/migrate.rs`'s init/migrate flows (cross-crate, so `#[allow(dead_code)]` is arguably still needed for the lint to pass when core is compiled/tested standalone) – but a reviewer should confirm the annotation is still necessary rather than a leftover from before the callers existed. | `crates/ss-magic/src/workspace/migrate.rs` (init/migrate call sites) |
| L14 | efficiency / duplication | `crates/ss-magic-core/src/sync/apply.rs:210-214` (`build_matcher`, called after `pattern::check_syntax` already ran `Glob::new` once) | `check_syntax` (line 55 of `pattern.rs`) compiles the glob once just to validate it, then `expand_patterns` immediately calls `build_matcher` which compiles the SAME pattern string again via `Glob::new(pattern).compile_matcher()` to get a usable matcher. Two `Glob::new` calls per glob pattern per `run`/`match_paths` invocation. The comment at `apply.rs:212-213` even says "check_syntax above already verified the glob compiles, so build_matcher can't fail here" – acknowledging the redundancy without avoiding it. | `crates/ss-magic-core/src/sync/pattern.rs:44-60` (`check_syntax`) vs `crates/ss-magic-core/src/sync/apply.rs:295-299` (`build_matcher`) |
| L15 | copy-paste-variant | `crates/ss-magic-core/src/superset_files.rs:357-363` (`write_magic_sh`, plain `fs::write`) vs `:384-390` (`write_config_json`, plain `fs::write`) vs `:256-268` (`bootstrap_magic_local_json`, plain `fs::write` of a hand-written JSON string) vs `:142-147`/`:158-163` (`write_magic_json`/`write_magic_local_json`, both routed through `write_atomically`) | Five writer functions in one file, three different durability postures: two go through the atomic staged-rename helper, three use a plain truncating `fs::write`. The module doc for `write_atomically` (lines 165-179) explains WHY the atomic path exists (an unattended writer, `seed-config`, can die mid-write) but that reasoning is never applied to `config.json` (also load-modify-written by CLI flows) or `magic.sh`/`magic.local.json`'s bootstrap writer. Worth confirming with the author whether the atomicity requirement really is `magic.json`-specific or was just not carried to the siblings yet. | `crates/ss-magic-core/src/superset_files.rs:142-163` (the two atomic writers) |
| L16 | naming | `crates/ss-magic-core/src/style.rs:105-119` (`info`, `ok`, `warn`, `err`, `header`) | Five one-line functions, each `role(s, Kind::X)` – this is a deliberate, minimal pattern (not a finding on its own), but flagged for the reviewer because it is the kind of five-way near-identical repetition a simplify pass might be tempted to touch; changing it would only be cosmetic (e.g. a macro) with no behavior gain, so likely NOT worth doing – noted to save the reviewer from re-deriving that conclusion. | n/a (informational, not a defect) |
| L17 | hardcoded-value | `crates/ss-magic-core/src/release.rs:57` (`HTTP_TIMEOUT = Duration::from_secs(5)`) vs `crates/ss-magic-plugin/src/status.rs:152` (`VERSION_TIMEOUT = Duration::from_secs(5)`) | Coincidentally identical 5-second timeout values for unrelated purposes (a GitHub HTTP fetch vs bounding a `--version` subprocess probe) in different crates. Almost certainly NOT worth unifying (different failure domains), but noted so the reviewer doesn't have to re-check whether it's the same knob. | `crates/ss-magic-plugin/src/status.rs:152` |
| L18 | reuse | `crates/ss-magic-core/src/reponame.rs:79-81` (`final_segment`) | Small, generic, single-caller "last `/`-segment of a string" helper. Given the sync/pack code elsewhere in the workspace does other URL/path-segment manipulation (e.g. `pack.rs`'s archive naming, which itself calls into this same `reponame` module), it's worth a quick grep in the ss-magic/ss-magic-plugin partitions for an equivalent hand-rolled `rfind('/')` before assuming this one is unique – not verified duplicated, flagged for the other partitions' reviewers. | other partitions' `pack.rs`, `identity.rs` |
| L19 | dead-code | `crates/ss-magic-core/src/reponame.rs:48-75` (`stem_from_origin`, `pub`) | No direct external call site was found outside `reponame.rs` and `reponame/tests.rs` itself – every other consumer reaches it indirectly through `repo_name_stem`. It is legitimately `pub` (crate-external testability / API surface), so this is informational rather than a defect: confirm before treating as unused. | `crates/ss-magic-core/src/reponame.rs:23-35` (`repo_name_stem`, the sole indirect caller) |

Files mapped: 12 (all listed source files in this partition).
