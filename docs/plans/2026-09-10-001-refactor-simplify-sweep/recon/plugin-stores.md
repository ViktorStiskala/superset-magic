# Recon: plugin-stores (bypass, expect_artifact, conclusion cache)

Partition: `crates/ss-magic-plugin/src/{bypass.rs, expect_artifact.rs, cache.rs}` plus their
sibling `tests.rs` files (context only).

## Map

### `crates/ss-magic-plugin/src/bypass.rs` (265 lines)
One-shot Read-gate bypass claim store at `.superset/.magic/bypass/<hash>.json`.

- `DIR_NAME` (pub, const `&str`) – bypass.rs:56 – claim subdirectory name (`"bypass"`).
- `MAX_AGE_SECS` (pub, const `u64`) – bypass.rs:64 – claim lifetime, 24h.
- `dir_for_root` (pub) – `fn(root: &Path) -> PathBuf` – bypass.rs:93 – `root.join(STATE_REL).join(DIR_NAME)`.
- `claim_path` (pub) – `fn(dir: &Path, realpath: &Path) -> PathBuf` – bypass.rs:106 – hashes `realpath` with FNV-1a into `<hash>.json`.
- `record` (pub) – `fn(dir: &Path, realpath: &Path, now: u64) -> Result<PathBuf>` – bypass.rs:116 – writes a `Claim{path, recorded_epoch}` atomically.
- `consume` (pub) – `fn(dir: &Path, realpath: &Path, now: u64) -> bool` – bypass.rs:148 – takes the claim via `claim::take`, then age-checks it.
- `run` (pub) – `fn(args: &[String]) -> Result<ExitCode>` – bypass.rs:176 – the `bypass` verb's arg parse (hand-rolled, no helper).
- `run_core` (private) – `fn(cwd: &Path, target: &str, now: u64) -> Result<ExitCode>` – bypass.rs:219 – resolve target, bootstrap scratchpad, record claim, print.
- `Claim` (private struct) – bypass.rs:84 – `{path: String, recorded_epoch: u64}`.
- `CLAIM_EXT` (private const) – bypass.rs:59 – `"json"`.
- `DIR_MODE` / `FILE_MODE` (private const `u32`) – bypass.rs:67-68 – `0o700`/`0o600`. **Candidate for shared const** (identical pair repeated in 8+ files; see Constants section).
- `BYPASS_USAGE` (private const `&str`) – bypass.rs:71 – usage text, inlined 3x in `run` rather than through a helper (unlike the other two files in this partition).

### `crates/ss-magic-plugin/src/expect_artifact.rs` (524 lines)
Pending subagent-output declarations at `.superset/.magic/expect-artifact/<hash>.json`; consumed once by `SubagentStop`.

- `DIR_NAME` (pub, const `&str`) – expect_artifact.rs:72 – `"expect-artifact"`.
- `MAX_AGE_SECS` (pub, const `u64`) – expect_artifact.rs:84 – declaration lifetime, 6h.
- `Expectation` (pub struct) – expect_artifact.rs:116 – `{path, relative, note: Option<String>, declared_epoch}`.
- `Unmet` (pub enum) – expect_artifact.rs:136 – `Missing | Empty | NotAFile`.
- `Unmet::describe` (pub) – `fn(self) -> &'static str` – expect_artifact.rs:150 – block-reason clause.
- `Unmet::code` (pub) – `fn(self) -> &'static str` – expect_artifact.rs:159 – heartbeat detail word.
- `check` (pub) – `fn(path: &Path) -> Option<Unmet>` – expect_artifact.rs:173 – stat-based contract check; stat failure reads as `Missing`.
- `dir_in` (pub) – `fn(state_root: &Path) -> PathBuf` – expect_artifact.rs:187 – `state_root.join(DIR_NAME)`. **Note**: takes an already-resolved `state_root`, unlike `bypass::dir_for_root`/`cache::dir_for_root`, which take the worktree `root` and do the `STATE_REL` join themselves – same shape of helper, inconsistent parameter (see Leads).
- `record_path` (pub) – `fn(dir: &Path, resolved: &Path) -> PathBuf` – expect_artifact.rs:199 – FNV-1a hash of `resolved` → `<hash>.json`. Identical shape/body to `bypass::claim_path` and near-identical to `cache::entry_path`'s key derivation (see Leads).
- `record` (pub) – `fn(dir, resolved, relative, note: Option<&str>, now) -> Result<PathBuf>` – expect_artifact.rs:212 – writes `Expectation` atomically.
- `trim_note` (private) – `fn(note: &str) -> String` – expect_artifact.rs:248 – bounds note to `MAX_NOTE_LEN`, char-boundary safe, ellipsis. Same "cut at char boundary" idiom as `cache::bound` (line 568) – smaller, single-purpose sibling.
- `take_oldest` (pub) – `fn(dir: &Path, now: u64) -> Option<Expectation>` – expect_artifact.rs:275 – claims oldest live record, sweeping expired/unusable ones.
- `candidates` (private) – `fn(dir: &Path) -> Vec<PathBuf>` – expect_artifact.rs:312 – lists `*.json` records oldest-first, unreadable ones sort last (`u64::MAX`).
- `resolve_declared` (private) – `fn(cwd, root_canon, target: &str) -> Result<PathBuf, String>` – expect_artifact.rs:358 – lexical-normalize + walk-to-existing-ancestor + canonicalize + re-append, then containment + not-a-dir checks.
- `run` (pub) – expect_artifact.rs:412 – arg parse for `expect-artifact <FILE> [--note TEXT]`.
- `refuse` (private) – `fn(message: impl Display) -> ExitCode` – expect_artifact.rs:449 – "print usage_error, exit 2" helper. Same purpose as `bypass.rs`'s inlined blocks and `cache::usage_error`, but a **different signature/shape** (see Leads).
- `run_core` (private) – expect_artifact.rs:457 – resolve root, resolve target, bootstrap scratchpad, record, print.
- `RECORD_EXT` (private const) – expect_artifact.rs:75 – `"json"` (same value as `bypass::CLAIM_EXT`).
- `MAX_NOTE_LEN` (private const `usize`) – expect_artifact.rs:89 – `500`.
- `DIR_MODE`/`FILE_MODE` (private const) – expect_artifact.rs:92-93 – `0o700`/`0o600`, third copy in this partition.
- `USAGE` (private const `&str`) – expect_artifact.rs:96.

### `crates/ss-magic-plugin/src/cache.rs` (1013 lines)
The conclusion cache: keyed on file identity `(realpath, size, mtime|content-hash)`, rendered through an untrusted-data envelope; backs `conclude`/`conclusions`/`gc`.

- `DIR_NAME` (pub, const `&str`) – cache.rs:84 – `"conclusions"`.
- `ENTRIES_KEPT` (pub, const `usize`) – cache.rs:94 – `200`.
- `MAX_AGE_SECS` (pub, const `u64`) – cache.rs:100 – 30 days. Same literal value (`30 * 24 * 60 * 60`) as `heartbeat::MAX_AGE_SECS` (heartbeat.rs:84) – different subsystem, same number, no shared const (see Leads/cross-partition).
- `Stamp` (pub enum) – cache.rs:132 – `Mtime(u128) | Content(u64) | Unknown`.
- `FileIdentity` (pub struct) – cache.rs:147 – `{realpath, size, stamp}`.
- `FileIdentity::key` (pub) – `fn(&self) -> String` – cache.rs:159 – FNV-1a hex of `key_material()`.
- `FileIdentity::key_material` (private) – `fn(&self) -> String` – cache.rs:166.
- `identify` (pub) – `fn(path: &Path) -> Result<FileIdentity>` – cache.rs:180 – canonicalize + stat; mtime, else `hash_file` fallback, else `Unknown`. **Mirrors** `ss-magic`'s `sync::reverse_sync::FileMeta`/`baseline_side`/`metas_match` (crates/ss-magic/src/sync/reverse_sync.rs:861-970): both independently implement "identity by mtime, falling back to a content hash when the filesystem has no mtime" – different crates, same idea, no shared helper (cross-partition lead).
- `dir_for_root` (pub) – `fn(root: &Path) -> PathBuf` – cache.rs:203 – byte-identical body to `bypass::dir_for_root` (bypass.rs:93): `root.join(STATE_REL).join(DIR_NAME)`.
- `entry_path` (pub) – `fn(dir: &Path, key: &str) -> PathBuf` – cache.rs:208 – `dir.join("{key}.md")`. Unlike `bypass::claim_path`/`expect_artifact::record_path`, takes an already-computed `key` rather than hashing a path itself – consistent with cache's key living on `FileIdentity`, but note the asymmetry (see Leads).
- `Entry` (pub struct) – cache.rs:221 – parsed on-disk entry (`path, key, header, body, source, realpath, size, recorded, recorded_epoch`).
- `Entry::parse` (private) – cache.rs:248 – splits stamped header from body, extracts `- field: value` lines.
- `Entry::age_stamp` (private) – `fn(&self) -> u64` – cache.rs:283 – `recorded_epoch` else file mtime else 0.
- `load` (pub) – `fn(dir: &Path, key: &str) -> Option<Entry>` – cache.rs:301 – miss on absent/empty body.
- `list` (pub) – `fn(dir: &Path) -> Result<Vec<Entry>>` – cache.rs:313 – newest-first, key tie-break.
- `write` (pub) – `fn(dir, id, source, body, now) -> Result<PathBuf>` – cache.rs:365 – stamps header + body, atomic write.
- `stamp_header` (private) – `fn(id, source, key, now) -> String` – cache.rs:404.
- `Budget` (pub enum) – cache.rs:428 – `Unbounded | Bytes(usize)`.
- `render` (pub) – `fn(entry: &Entry, budget: Budget) -> String` – cache.rs:481 – synthesizes a header for unstamped entries, then calls `envelope`.
- `envelope` (pub) – `fn(head, body, whole: &Path, budget) -> String` – cache.rs:514 – the untrusted-data wrapper shared with `hook/subagent_stop.rs` (salvage) and `checklist/render.rs`.
- `render_cached` (pub) – `fn(dir, key, budget) -> Option<String>` – cache.rs:561 – `load` + `render` composed.
- `bound` (private) – `fn(text: &str, limit: usize) -> (&str, bool)` – cache.rs:568 – char/line-boundary-safe truncation. Same "cut at char boundary" idiom as `expect_artifact::trim_note` (smaller/single-purpose sibling; see Leads).
- `nonce_for` (private) – `fn(inner: &str) -> String` – cache.rs:599 – deterministic-then-clock-salted nonce search, avoiding collision with body text.
- `prune` (pub) – `fn(dir, keep, max_age, now, protect: Option<&str>) -> Result<Vec<PathBuf>>` – cache.rs:629 – count+age eviction, protects one key. Same shape as `heartbeat::prune` and `reverse_sync::prune_old_backups` (per module docs, deliberately parallel – not flagged as duplication per CLAUDE.md KTD14).
- `prune_best_effort` (private) – `fn(dir, protect: Option<&str>)` – cache.rs:665 – calls `prune` with the module's own bounds, warns on failure.
- `GcReport` (pub struct, `Default`) – cache.rs:686 – `{orphaned, pruned, unverifiable, live}`.
- `gc` (pub) – `fn(dir: &Path) -> Result<GcReport>` – cache.rs:709 – drops orphaned entries then calls `prune`.
- `run_conclude` / `conclude_core` (pub/private) – cache.rs:760/809 – `conclude` verb.
- `run_conclusions` / `conclusions_core` (pub/private) – cache.rs:865/893 – `conclusions` verb.
- `run_gc` / `gc_core` (pub/private) – cache.rs:958/972 – `gc` verb.
- `is_key` (private) – `fn(value: &str) -> bool` – cache.rs:1000 – 16 hex chars.
- `usage_error` (private) – `fn(message: &str, usage: &str) -> Result<ExitCode>` – cache.rs:1006 – third distinct "usage failure" helper shape in this partition (see Leads; also collides in name, not shape, with `config.rs`'s `usage_error(usage: &str, message: &str) -> ExitCode` – **reversed argument order and return type**).
- `ENTRY_EXT` / `KEY_HEX_LEN` / `SEPARATOR` / `HEADER_TITLE_PREFIX` / `ENVELOPE_OPEN` / `ENVELOPE_CLOSE` / `NONCE_HEX_LEN` (private consts) – cache.rs:88,103,112,118,444-445,448.
- `DIR_MODE`/`FILE_MODE` (private const) – cache.rs:106-107 – fourth copy in this partition/workspace-wide ninth+ copy.
- `CONCLUDE_USAGE` / `CONCLUSIONS_USAGE` / `GC_USAGE` (private const) – cache.rs:733,746,753.

## Constants and literals

| Item | Location | Value | Denotes | Elsewhere in workspace? |
|---|---|---|---|---|
| `DIR_MODE` | bypass.rs:67, expect_artifact.rs:92, cache.rs:106 | `0o700` | state-tree dir mode | Also `heartbeat.rs:71`, `ledger.rs:100`, `scratchpad.rs:119`, `tmproot.rs:101`, `hook/subagent_stop.rs:85` – **9 private copies of the identical value**, all commented "matching the rest of the state tree (R58)". No shared owner; `scratchpad.rs` (owner of `STATE_REL`) is the natural home. |
| `FILE_MODE` | bypass.rs:68, expect_artifact.rs:93, cache.rs:107 | `0o600` | state-tree file mode | Same 9+ files as above, plus a bare literal `0o600` at `release_check.rs:114` and `hook/session_start.rs:301` (not even a named const there). |
| `MAX_AGE_SECS` | bypass.rs:64 | `24 * 60 * 60` (1 day) | bypass claim lifetime | Distinct meaning per file; `index/consts.md` flags all four `MAX_AGE_SECS` names as `dup-name` (bypass 1d, expect_artifact 6h, cache 30d, heartbeat 30d). Cache's and heartbeat's share the exact 30-day value independently. |
| `MAX_AGE_SECS` | expect_artifact.rs:84 | `6 * 60 * 60` (6h) | declaration lifetime | see above |
| `MAX_AGE_SECS` | cache.rs:100 | `30 * 24 * 60 * 60` (30d) | conclusion prune age | same numeric value as `heartbeat.rs:84`'s `MAX_AGE_SECS` (30d) – no shared const, coincidence per differing module docs, but worth a reviewer's glance since both cite "matching the heartbeat log" (cache.rs:97-99) as their reasoning, i.e. cache's own comment claims the two are meant to track each other. |
| `CLAIM_EXT` | bypass.rs:59 | `"json"` | claim file extension | same value as `expect_artifact::RECORD_EXT` (expect_artifact.rs:75) – two identically-valued, identically-purposed private consts, never unified (they're in different one-shot stores, both built on `claim::take`, so the redundancy is structural not accidental – see `crate::claim`'s pinned design). |
| `RECORD_EXT` | expect_artifact.rs:75 | `"json"` | record file extension | see above |
| `ENTRY_EXT` | cache.rs:88 | `"md"` | conclusion entry extension | unique to cache.rs (deliberately markdown, per comment) |
| `KEY_HEX_LEN` | cache.rs:103 | `16` | cache key hex length | same value as `NONCE_HEX_LEN` (cache.rs:448) – both "16 hex chars from an FNV-1a u64", coincidental (a u64 is always 16 hex chars) rather than a shared derivation; a single `const HEX64_LEN: usize = 16` used by both (and by `bypass::claim_path`/`expect_artifact::record_path`, which hardcode `{hash:016x}` inline without a named length const at all) would remove 3-4 magic `16`s. |
| `SEPARATOR` | cache.rs:112 | `"\n---\n"` | header/body split marker | unique |
| `HEADER_TITLE_PREFIX` | cache.rs:118 | `"# ss-magic conclusion"` | stamped-entry marker | unique |
| `ENTRIES_KEPT` | cache.rs:94 | `200` | conclusion retention count | unique (`index/numbers.md` – not checked further, value < 60 threshold anyway for some related consts) |
| `MAX_NOTE_LEN` | expect_artifact.rs:89 | `500` | note truncation bound | unique |
| `"bypass"` | bypass.rs:56 (`DIR_NAME`) | dir name | Referenced from `hook/pre_tool_use.rs:817`, `status.rs:1304`, `main.rs` verb table (`literals.md` shows 4 occurrences total, all through the const, not re-hardcoded) – clean. |
| `"expect-artifact"` | expect_artifact.rs:72 | dir name | Same pattern – `main.rs:227,257`, `status.rs:2113`, all via the const. Clean. |
| `"conclusions"` | cache.rs:84 | dir name | `main.rs:224,254`, `status.rs:2111` via the const. Clean. |
| `{hash:016x}` / `{key:016x}` format | bypass.rs:108, expect_artifact.rs:201, cache.rs:160 | format string | Same shape repeated 3x for "u64 hash as 16 lowercase hex chars" – a shared `fn hex16(u64) -> String` in `hashing.rs` (core) would collapse it, though core is shared with the CLI so this would need to be judged against whether the CLI has any use for it (grep did not find one). |
| `.bypass-` / `.expect-` / `.conclusion-` | bypass.rs:133, expect_artifact.rs:237, cache.rs:390 | temp-file prefix passed to `atomic::write_atomically` | Per-module, deliberately distinct (so a stray temp file's origin is identifiable) – not a duplication, just noting the pattern is otherwise identical across all three calls (same `".tmp"` suffix, same `Some(FILE_MODE)`, same `true` for fsync). |

## Cross-module references

**Imports into this partition:**
- All three import `crate::hashing` (FNV-1a), `crate::atomic::write_atomically`, `crate::claim`, `crate::scratchpad` (`ensure`, `now_secs`; bypass/cache also `STATE_REL`), `ss_magic_core::style`.
- `expect_artifact.rs` additionally imports `crate::git` (for `cwd_repo_root`) and `crate::pathnorm::normalize`.
- `cache.rs` additionally imports `crate::git` (for `cwd_repo_root`).

**Where this partition's public symbols are used elsewhere (grep, ≤5 callers each):**
- `bypass::DIR_NAME` – `status.rs:1304`.
- `bypass::dir_for_root` – `hook/pre_tool_use.rs:817`, `hook/pre_tool_use/tests.rs:58`.
- `bypass::consume` – `hook/pre_tool_use.rs:817` (only caller).
- `bypass::record` – `hook/pre_tool_use/tests.rs:357,435,686,701,1153` (test-only outside `bypass.rs` itself; no production caller besides the verb).
- `bypass::run` – `main.rs:525` (only caller, the verb dispatch table).
- `bypass::claim_path`, `bypass::MAX_AGE_SECS` – unused outside `bypass.rs`/`bypass/tests.rs`.
- `expect_artifact::DIR_NAME` – `status.rs:1307`.
- `expect_artifact::dir_in` – `hook/subagent_stop.rs:131`, `hook/subagent_stop/tests.rs:48`.
- `expect_artifact::take_oldest` – `hook/subagent_stop.rs:132`, `hook/subagent_stop/tests.rs:264,311`.
- `expect_artifact::check` – `hook/subagent_stop.rs:139` (only production caller).
- `expect_artifact::{Expectation, Unmet}` – imported by `hook/subagent_stop.rs:56`.
- `expect_artifact::record` – `hook/subagent_stop/tests.rs:55` (test-only outside its own module; no production caller besides the verb).
- `expect_artifact::MAX_AGE_SECS` – `hook/subagent_stop/tests.rs:276`.
- `expect_artifact::run` – `main.rs:526` (only caller).
- `expect_artifact::record_path` – unused outside `expect_artifact.rs`.
- `cache::DIR_NAME` – `status.rs:1303`.
- `cache::identify` – `hook/pre_tool_use.rs:812`, `hook/pre_tool_use/tests.rs:64,987`.
- `cache::dir_for_root` – `hook/pre_tool_use.rs:824`, `hook/pre_tool_use/tests.rs:54`.
- `cache::entry_path` – `hook/pre_tool_use.rs:825`, `hook/pre_tool_use/tests.rs:988`.
- `cache::render_cached` / `cache::Budget` – `hook/pre_tool_use.rs:838`, `checklist/verbs.rs:84` (`Budget` only).
- `cache::envelope` – `hook/subagent_stop.rs:55,268` (subagent transcript salvage), `checklist/render.rs:65,84` (checklist rendering) – the one function in this partition genuinely reused by two other subsystems, exactly as its own doc comment (cache.rs:502-509) says it should be.
- `cache::load` – `hook/pre_tool_use/tests.rs:879` (test-only outside cache.rs).
- `cache::write` – `hook/pre_tool_use/tests.rs:66` (test-only outside cache.rs).
- `cache::{run_conclude, run_conclusions, run_gc}` – `main.rs:527-529` (verb dispatch, one caller each).
- `cache::{Stamp, FileIdentity, Entry, GcReport, bound, nonce_for, prune, prune_best_effort, is_key, usage_error, gc, list}` – unused outside `cache.rs`/`cache/tests.rs`.

## Leads

| # | Category | Location(s) | Hypothesis | Compare against |
|---|---|---|---|---|
| L1 | const-location | bypass.rs:67-68, expect_artifact.rs:92-93, cache.rs:106-107 | `DIR_MODE`/`FILE_MODE` = `0o700`/`0o600` are redeclared as private consts in every state-tree writer (9+ occurrences workspace-wide) instead of living once, e.g. in `scratchpad.rs` (which already owns `STATE_REL` and the R58 comment these all cite) and being re-exported. | `heartbeat.rs:71,73`, `ledger.rs:100-101`, `scratchpad.rs:119,122`, `tmproot.rs:101`, `hook/subagent_stop.rs:85-86` – cross-partition, all citing the same "R58" rule. |
| L2 | duplication | bypass.rs:93 `fn dir_for_root(root: &Path) -> PathBuf { root.join(STATE_REL).join(DIR_NAME) }` vs cache.rs:203, identical body | Two byte-identical one-line functions differing only in the `DIR_NAME` constant closed over; a single generic `fn store_dir(root: &Path, name: &str) -> PathBuf` in `scratchpad.rs` would serve both (and `expect_artifact::dir_in` could adopt the same shape if it took `root` instead of `state_root`). | `expect_artifact.rs:187` (`dir_in`, takes `state_root` not `root` – inconsistent parameter across the three "get my subdirectory" functions). |
| L3 | copy-paste-variant | bypass.rs (inlined thrice), expect_artifact.rs:449 `refuse`, cache.rs:1006 `usage_error` | Three different shapes of the same "print usage_error, exit 2" tail exist inside this one partition alone: bypass.rs repeats `style::err(...)` + `eprintln!("{BYPASS_USAGE}")` + `ExitCode::from(2)` three separate times inline (lines 184-201) instead of factoring a helper; `expect_artifact::refuse` takes `impl Display` and returns bare `ExitCode`; `cache::usage_error` takes `(message: &str, usage: &str)` and returns `Result<ExitCode>`. | `config.rs:938` `usage_error(usage: &str, message: &str) -> ExitCode` (**argument order reversed** relative to `cache::usage_error`'s `(message, usage)` – a real trap for anyone copying a call site between the two), `checklist/verbs.rs:1438`, `compact_window.rs:837`, `ledger.rs:1250` – five-plus independent reimplementations of one pattern workspace-wide. |
| L4 | duplication | bypass.rs:106-109 `claim_path`, expect_artifact.rs:199-202 `record_path`, cache.rs:160 (`FileIdentity::key`) | All three compute `format!("{hash:016x}.{EXT}")` from an FNV-1a hash of a path (or path-derived string) with only the extension constant differing (`"json"` vs `"json"` vs – for cache the format string is used for the key itself, `entry_path` just appends `.md` to an already-computed key). `claim_path` and `record_path` in particular are structurally identical functions with different doc comments explicitly cross-referencing each other (expect_artifact.rs:194 literally says "the reason `crate::bypass::claim_path` gives"). | Nothing beyond these two – a shared `fn hashed_path(dir: &Path, seed: &OsStr, ext: &str) -> PathBuf` in `hashing.rs` would remove one of the two. |
| L5 | naming | expect_artifact.rs:187 `dir_in(state_root: &Path)` vs bypass.rs:93 `dir_for_root(root: &Path)` and cache.rs:203 `dir_for_root(root: &Path)` | Same conceptual helper ("where does my subdirectory live"), two different names (`dir_in` vs `dir_for_root`) and two different parameter conventions (already-resolved `state_root` vs raw worktree `root`) across three sibling modules built to the same pattern. | Same three functions listed in L2. |
| L6 | duplication | expect_artifact.rs:248-258 `trim_note` vs cache.rs:568-584 `bound` | Both implement "truncate a string to a byte/char budget, cut at a safe boundary" – `trim_note` cuts at a char boundary only (with a trailing `…`), `bound` cuts at a char boundary and then prefers the last full line, returning whether it truncated. Different enough in behavior (line-preference, ellipsis vs a separate notice) that unifying them outright would need care, but the character-boundary-walk loop (`while end > 0 && !text.is_char_boundary(end) { end -= 1; }`) is copied verbatim between them. | expect_artifact.rs:253-256 vs cache.rs:572-575 – literally the same 3-line loop in both files. |
| L7 | hardcoded-value / dup-name | cache.rs:100 `MAX_AGE_SECS = 30 * 24 * 60 * 60` vs heartbeat.rs:84 `MAX_AGE_SECS = 30 * 24 * 60 * 60` | Cache's own doc comment (cache.rs:97-99) says its 30-day bound is "matching the heartbeat log", i.e. the author intends the two to move together, yet they are two independent private consts with no shared definition – a change to one silently desyncs from the other. | `heartbeat.rs:84` (cross-partition; heartbeat is not in this partition's file list but is named by cache's own comment as the thing it matches). |
| L8 | reuse (positive; not a defect, flagged for completeness) | cache.rs:514 `envelope` | Correctly the single shared implementation used by `hook/subagent_stop.rs` (transcript salvage) and `checklist/render.rs` (checklist rendering) as well as this module's own `render`/`render_cached`. No duplication found here – recorded so the reviewer does not re-flag it. | `hook/subagent_stop.rs:268`, `checklist/render.rs:84`. |
| L9 | duplication (cross-partition, cross-crate) | cache.rs:180-200 `identify`/`Stamp` (mtime, else content-hash, else unknown) | Independently reimplements the same "identity by mtime, fallback to content hash when the filesystem has no mtime" idea that `ss-magic`'s reverse-sync baseline already has (`FileMeta`/`baseline_side`/`metas_match`). The two crates cannot share code directly today (the plugin depends on core only, not on `ss-magic`), so this is a lead for whether the pattern belongs in `ss-magic-core::hashing` as a shared "stat, else hash" primitive both binaries call, rather than for direct removal. | `crates/ss-magic/src/sync/reverse_sync.rs:861` (`FileMeta`), `:948` (`baseline_side`), `:959` (`metas_match`). |
| L10 | hardcoded-value | cache.rs:103 `KEY_HEX_LEN = 16`, cache.rs:448 `NONCE_HEX_LEN = 16`, plus the bare `16` implied by `{hash:016x}` in bypass.rs:108 and expect_artifact.rs:201 | Four places encode "an FNV-1a `u64` formatted as hex is 16 characters" – two as named consts (used for different purposes: entry filename stem length, and nonce length) and two as a bare `016` in a format literal. All four are the same fact about `u64`/hex, expressed four different ways. | bypass.rs:108, expect_artifact.rs:201, cache.rs:103, cache.rs:448. |
| L11 | efficiency (minor) | cache.rs:717 `gc()`'s inner loop | `identify(realpath).map(|id| id.key() == entry.key)` recomputes a full `identify` (stat + possible whole-file hash on mtime-less filesystems) for every entry on every `gc` run, with no short-circuit for the common case where the file is simply gone (`identify` already returns `Err` for that via `canonicalize`/`metadata`, so this is likely fine) – flagged only because the fallback path re-reads the whole file's bytes per orphan-check when mtime is unavailable, same cost noted in the module's own doc comment (cache.rs:127-130) about `identify`. Not a bug, worth a reviewer glance for whether `gc` should skip the content-hash fallback and treat "no mtime" entries as always-orphaned-on-gc. | cache.rs:180-200 (`identify`), cache.rs:709-729 (`gc`). |
| L12 | naming (minor) | bypass.rs's `run` (lines 176-215) has no `refuse`/`usage_error` helper at all, unlike its two siblings in this partition | Given L3 already flags the three-way variant, this specifically notes that `bypass.rs` is the odd one out with zero factoring – three verbatim-repeated error blocks (lines 184-191, 193-200, 204-211) each doing `style::err(...)`, `eprintln!("{BYPASS_USAGE}")`, `ExitCode::from(2)`. | expect_artifact.rs:449 (`refuse`), which is the shape bypass.rs could adopt directly (same crate, same partition). |
