# Recon: plugin-state (atomic, claim, tmproot, identity, scratchpad, heartbeat, pathnorm)

All line numbers verified against the files as read in this session.

## Map

### `crates/ss-magic-plugin/src/atomic.rs` (92 lines)
The one atomic-write primitive every plugin module that replaces a file uses: temp file in the target's own directory, write, flush, optional chmod, optional fsync, rename over the target.

- `write_atomically(path: &Path, body: &str, prefix: &str, suffix: &str, what: Option<&str>, mode: Option<u32>, sync: bool) -> Result<()>` (`pub(crate)`, line 51) – the sole write primitive: builds a `tempfile::Builder` in `path`'s parent dir, writes `body`, optionally chmods to `mode`, optionally fsyncs, then `persist`s over `path`.

### `crates/ss-magic-plugin/src/claim.rs` (81 lines)
The exactly-once file claim (rename-based, not unlink-based) both one-shot stores (`bypass.rs`, `expect_artifact.rs`) are built on.

- `struct Claimed { landing: NamedTempFile }` (pub, line 43) – a claim this caller won; `Drop` unlinks it.
- `Claimed::text(&self) -> Option<String>` (pub, line 54) – reads what the claim file held.
- `take(dir: &Path, path: &Path) -> Option<Claimed>` (pub, line 65) – creates a private landing file in `dir`, then `fs::rename(path, landing.path())`; `Some` iff this caller won the rename race.

### `crates/ss-magic-plugin/src/tmproot.rs` (271 lines)
The plugin's private per-machine temp root (`/tmp/ss-magic-plugin/<sha256(HOME)[..16]>/`, falling back to `$TMPDIR`) plus the fd-lock helpers built on it.

- `NAMESPACE_DIR: &str = "ss-magic-plugin"` (pub, line 89) – the namespace dir name; cross-language contract with `plugin/hooks/bootstrap.sh`.
- `INSTALL_LOCK_NAME: &str = "install.lock"` (pub, `#[allow(dead_code)]`, line 97) – lock name shared with `bootstrap.sh`'s shell-side lock; not referenced anywhere in the Rust source (only the shell side uses this exact relative path).
- `struct NoValidRoot` (pub, line 114) + `Display`/`Error` impls (lines 116, 126) – the "no usable root" error value.
- `identifier(home: &str) -> String` (pub, line 132) – first 16 hex chars of `sha256_hex(home)`.
- `resolve_root() -> Result<PathBuf, NoValidRoot>` (pub, line 145) – reads real `$HOME`/`$TMPDIR`/euid, delegates to `resolve_root_at`.
- `resolve_root_at(home, primary_base, fallback_base, euid) -> Result<PathBuf, NoValidRoot>` (private, line 163) – injectable core of `resolve_root`.
- `ensure_and_validate(base, id, euid) -> Option<PathBuf>` (private, line 181) – creates+validates `base/ss-magic-plugin/<id>` (parent first).
- `ensure_dir(path: &Path)` (private, line 202) – best-effort single-level `mkdir` at `DIR_MODE`. **Small, generic helper** – candidate for reuse (compare `scratchpad.rs`'s own `DirBuilder::new().recursive(true).mode(DIR_MODE).create(...)` calls at lines 488, 510).
- `validate_component(path, euid) -> bool` (private, line 210) – lstat-based: real dir, owned by `euid`, mode exactly `0700`.
- `open_lock_file(path: &Path) -> io::Result<File>` (private, line 221) – `OpenOptions` read+write+create+`truncate(false)`. **Small, generic helper** – identical shape appears inline in `scratchpad.rs::write_pointer` (lines 595-602) and in `crates/ss-magic/src/update/apply.rs:242` (`open_lock_file`, per functions.md).
- `with_lock<T>(root, name, f) -> io::Result<T>` (pub, line 242) – blocking exclusive `fd_lock::RwLock`.
- `try_with_lock<T>(root, name, f) -> io::Result<Option<T>>` (pub, line 256) – non-blocking sibling.

### `crates/ss-magic-plugin/src/identity.rs` (182 lines)
Deterministic `<repo>-<branch>` session-identity slug, derived from git alone.

- `struct Identity { repo: String, branch: String, slug: String }` (pub, line 30).
- `resolve(cwd: &Path) -> Option<Identity>` (pub, line 55) – `git::cwd_repo_root` → `repo_component` + `branch_component` → `Identity`.
- `repo_component(root: &Path) -> String` (private, line 75) – `reponame::repo_name_stem`, falls back to `"repo"`.
- `branch_component(root: &Path) -> Option<String>` (private, line 91) – `symbolic_ref_head` → `slugify`, else `detached_component`.
- `detached_component(root: &Path) -> Option<String>` (private, line 105) – `detached-<short-sha>`.
- `slugify(input: &str) -> String` (private, line 138) – hand-rolled ASCII-only slugify with diacritic stripping and combining-mark skipping, truncated to 40 chars.
- `strip_diacritic(ch: char) -> Option<char>` (private, line 167) – table of precomposed Latin-1/Latin-Extended-A letters → ASCII base.

### `crates/ss-magic-plugin/src/scratchpad.rs` (805 lines)
The per-worktree state tree at `.superset/.magic/`: bootstrap (`ensure`), the `current.json` pointer, and the `scratchpad` human verb.

- `STATE_REL` (pub, re-export of `ss_magic_core::state_tree::STATE_REL`, line 60).
- `STATE_QUERY: &str = ".superset/.magic/"` (private, line 66) – dup-name with `hook/mod.rs:93`.
- `SUPERSET_REL: &str = ".superset"` (private, line 70).
- `STATE_FILES: [&str; 6]` (pub, line 80) – the six model-owned scaffolded files.
- `README_NAME: &str = "README.md"` (private, line 90).
- `POINTER_NAME: &str = "current.json"` (private, line 96) – dup-name with `checklist/verbs.rs:101` (`"checklist.json"`, different value, same const name).
- `POINTER_LOCK_NAME: &str = "current.lock"` (private, line 104).
- `SESSIONS_DIR: &str = "sessions"` (private, line 107).
- `CLAIM_DIRS: [&str; 3] = ["conclusions", "bypass", "expect-artifact"]` (private, line 113) – see Constants section: mirrors `cache::DIR_NAME`/`expect_artifact::DIR_NAME`/`bypass::DIR_NAME` literals.
- `DIR_MODE: u32 = 0o700` (private, line 119), `FILE_MODE: u32 = 0o600` (private, line 122) – dup-name with 7 other files (see Constants section).
- `SCRATCHPAD_USAGE: &str` (private, line 125).
- `struct Pointer { slug, dir, repo, branch, resolved_at }` (pub, line 137).
- `enum Refusal { NotIgnored, Escapes, NotADirectory, TrackedProbeFailed, TrackedPaths }` (pub, line 156) + `code()` (pub, `#[allow(dead_code)]`, line 201) + `Display` (line 212).
- `struct Report { state_root, slug, session_dir, created, refusals, wrote_state }` (pub, line 239) + `heartbeat_note(&self) -> String` (pub, line 278).
- `pub use ss_magic_core::state_tree::ensure_state_ignored;` (line 309).
- `struct Ctx { root, root_canon, tracked }` (private, line 316) – `rel_of` (line 326), `verify_contained` (line 341), `is_tracked` (line 363). **`rel_of`'s `strip_prefix`+`to_string_lossy` shape is small and generic** – similar ad hoc conversions appear scattered in `ledger.rs` (lines 619, 1024-1050) and `spill_index.rs` (219, 273), though none share a named helper.
- `ensure(cwd: &Path) -> Result<Report>` (pub, line 378) – the whole bootstrap: containment → not-a-directory → R63 ignore gate → R17 tracked-files gate → create dirs → scaffold files → write pointer.
- `scaffold(ctx, path, body, report) -> Result<()>` (private, line 549) – `create_new` idempotent file creation.
- `write_pointer(state_root, path, identity, session_rel) -> Result<()>` (private, line 588) – fd-lock + `atomic::write_atomically`. **The lock-open block (lines 594-606) duplicates `tmproot::open_lock_file` + `fd_lock::RwLock::new` verbatim** rather than calling `tmproot::with_lock`.
- `pub(crate) use ss_magic_core::release::now_secs;` (line 640) – re-export, replacing six independent local copies (comment at 634-639 documents this consolidation already happened).
- `now_rfc3339() -> String` (private, line 643).
- `format_rfc3339(secs: u64) -> String` (`pub(crate)`, line 655) – hand-rolled Hinnant civil-from-days formatter. **Same core algorithm (lines 661-669) as `crates/ss-magic/src/sync/reverse_sync.rs`'s backup-timestamp formatter** (different output shape, documented as deliberately not shared – see Leads).
- `README_BODY`, `state_file_body(name) -> &'static str` (private, lines 678, 697).
- `run(args: &[String]) -> Result<ExitCode>` (pub, line 746) – `scratchpad <SUBVERB>` dispatch.
- `run_ensure(cwd: &Path) -> Result<ExitCode>` (private, line 778).

### `crates/ss-magic-plugin/src/heartbeat.rs` (377 lines)
The append-only machine-level `hooks.jsonl`, one row per hook invocation.

- `STORE_SUBDIR: &str = "plugin"` (pub, line 58) – dup-name with `ledger.rs:90`'s `PRICES_DIR_NAME = "prices"` (different value, no clash; noted because both are subdir-name consts in sibling modules).
- `LOG_FILE_NAME: &str = "hooks.jsonl"` (pub, line 61).
- `LOCK_FILE_NAME: &str = "hooks.lock"` (private, line 67) – dup-name with `ledger.rs:96` (`"cost.lock"`) and `crates/ss-magic/src/update/apply.rs:80` (`"update.lock"`, that one `pub`).
- `DIR_MODE: u32 = 0o700` / `FILE_MODE: u32 = 0o600` (private, lines 71, 73) – dup-name group, see Constants.
- `ROWS_KEPT: usize = 2_000` (pub, line 80).
- `MAX_AGE_SECS: u64 = 30 * 24 * 60 * 60` (pub, line 84) – dup-name (different values) with `bypass.rs` (`24*60*60`), `expect_artifact.rs` (`6*60*60`); dup-VALUE with `cache.rs:100` (`30*24*60*60`).
- `PRUNE_TRIGGER_BYTES: u64 = 256 * 1024` (pub, line 91) – dup-value with `ledger.rs:555` `READ_BUF_BYTES = 256 * 1024` (different meaning, same magic number).
- `enum Outcome { Ok, NoOp, Error }` (pub, line 100).
- `struct Row { event, at, ts, cwd, outcome, reason, detail }` (pub, line 118) + `Row::new` (144), `with_outcome` (158), `with_cwd` (164), `with_reason` (170), `with_detail` (176) – builder-style, all pub.
- `store_path() -> Option<PathBuf>` (private, line 191) – `directories::ProjectDirs::from("", "", "ss-magic")`.
- `existing_store_dir() -> Option<PathBuf>` (pub, line 201) – non-creating variant for read-only callers.
- `store_dir() -> Option<PathBuf>` (pub, line 212) – creating variant.
- `ensure_store(dir: &Path) -> Result<()>` (private, line 221).
- `log_path(store: &Path) -> PathBuf` (pub, line 233).
- `append(store: &Path, row: &Row) -> Result<()>` (pub, line 250) – locks via `tmproot::with_lock`, writes, best-effort prunes.
- `write_row(path, row) -> Result<()>` (private, line 272).
- `now_secs_or(fallback: u64) -> u64` (private, line 286) – **near-duplicate of `ss_magic_core::release::now_secs`** (the shared one `scratchpad.rs:640` re-exports); this one adds a caller-supplied fallback instead of defaulting to 0.
- `maybe_prune(path, now) -> Result<usize>` (private, line 297).
- `prune(path, keep, max_age, now) -> Result<usize>` (private, line 315) – rewrite-all pruning via `atomic::write_atomically`.
- `read(store: &Path) -> Result<Vec<Row>>` (pub, line 363).

### `crates/ss-magic-plugin/src/pathnorm.rs` (240 lines)
Lexical path normalization shared by every path-deciding gate (checklist deny, Read gate).

- `normalize(path: &Path) -> PathBuf` (`pub(crate)`, line 41) – textual `.`/`..` reduction; leading unresolvable `..` counted, not pushed.
- `enum HomeRelative { No, Own(PathBuf), Other }` (`pub(crate)`, line 105).
- `home_relative(path: &Path) -> HomeRelative` (`pub(crate)`, line 127) – classifies a leading `~`.
- `enum ProcessView { Independent, Cwd(PathBuf), Opaque }` (`pub(crate)`, line 168).
- `process_view(path: &Path) -> ProcessView` (`pub(crate)`, line 198) – scans for a `proc/<selector>` anywhere in the path.
- `names_a_process(component: &OsStr) -> bool` (private, line 230) – `"self"`, `"thread-self"`, or all-ASCII-digit.

## Constants and literals

| Item | file:line | Value | Elsewhere in workspace? | Owning definition |
|---|---|---|---|---|
| `DIR_MODE` (tmproot) | `tmproot.rs:101` | `0o700` | Same value/name in `bypass.rs:67`, `cache.rs:106`, `expect_artifact.rs:92`, `heartbeat.rs:71`, `hook/subagent_stop.rs:85`, `ledger.rs:100`, `scratchpad.rs:119` (7 more, per consts.md) | No shared owner – 8 independent module-private consts, all `0o700` |
| `FILE_MODE` (heartbeat) | `heartbeat.rs:73` | `0o600` | Same value/name in `bypass.rs:68`, `cache.rs:107`, `expect_artifact.rs:93`, `hook/file_changed.rs:95`, `hook/pre_compact.rs:67`, `hook/subagent_stop.rs:86`, `ledger.rs:101`, `scratchpad.rs:122` (8 more) | No shared owner – 9 independent module-private consts, 8 of them `0o600`; `setup_ci.rs:84` is `0o644` (different, deliberately committed content) |
| `MAX_AGE_SECS` (heartbeat) | `heartbeat.rs:84` | `30 * 24 * 60 * 60` | Same VALUE at `cache.rs:100`; same NAME, different values at `bypass.rs:64` (`24*60*60`), `expect_artifact.rs:84` (`6*60*60`) | No shared owner; each module's TTL is semantically distinct (log retention vs. cache vs. one-shot claims), so same-name-different-value is not obviously wrong, but the *heartbeat/cache* pair sharing the exact 30-day value with no shared const is worth a look |
| `PRUNE_TRIGGER_BYTES` (heartbeat) | `heartbeat.rs:91` | `256 * 1024` | Same numeric value (different name/purpose) at `ledger.rs:555` `READ_BUF_BYTES` | Coincidental – one is a size trigger, the other a read-buffer size; likely not worth unifying but flagged since numbers.md/consts.md would show both |
| `IDENTIFIER_HEX_LEN` (tmproot) | `tmproot.rs:105` | `16` | Same value at `cache.rs:103` `KEY_HEX_LEN`, `cache.rs:448` `NONCE_HEX_LEN` | No shared owner; all three truncate a hex digest to 16 chars for different purposes |
| `NAMESPACE_DIR` | `tmproot.rs:89` | `"ss-magic-plugin"` | `bootstrap.sh` and other shell scripts spell it literally (`tmproot.sh` per literals.md – this is the packaged asset counterpart, not in this partition) | Documented cross-language contract; not a Rust-side dup |
| `INSTALL_LOCK_NAME` | `tmproot.rs:97` | `"install.lock"` | `tmproot.sh:42` (shell side) – 2 occurrences per literals.md | Cross-language contract (documented); Rust side is `#[allow(dead_code)]` – unused from Rust, only the shell side spells this path |
| `STATE_QUERY` | `scratchpad.rs:66` | `".superset/.magic/"` | Same name+value at `hook/mod.rs:93` | Two independent private consts holding the identical string – candidate to share (e.g. re-export from one) |
| `STATE_REL` | via re-export, `ss-magic-core/src/state_tree.rs:35` | `".superset/.magic"` | Owning definition already in core; this file just re-exports (per structure-pins, this is intentional, not a dup) | `ss-magic-core::state_tree::STATE_REL` |
| `LOCK_FILE_NAME` (heartbeat) | `heartbeat.rs:67` | `"hooks.lock"` | dup-NAME (different value) at `ledger.rs:96` (`"cost.lock"`), `crates/ss-magic/src/update/apply.rs:80` (`"update.lock"`, pub) | Independent – each module's own lock file name; naming pattern is consistent but not literally shared |
| `POINTER_NAME` (scratchpad) | `scratchpad.rs:96` | `"current.json"` | dup-NAME (different value) at `checklist/verbs.rs:101` (`"checklist.json"`) | Independent – coincidental same const identifier for a different pointer file |
| `MAGIC_LOCAL_JSON`-style pattern | n/a | n/a | n/a | n/a |
| `CLAIM_DIRS` | `scratchpad.rs:113` | `["conclusions", "bypass", "expect-artifact"]` | Same three strings individually own-defined as `DIR_NAME` pub consts: `cache.rs:84` (`"conclusions"`), `bypass.rs:56` (`"bypass"`), `expect_artifact.rs:72` (`"expect-artifact"`) | **`scratchpad::CLAIM_DIRS` is the one place all three literals are spelled together; the three per-module `DIR_NAME` consts are the presumable owners and are NOT imported here – scratchpad hardcodes its own copies of the same three strings.** See Leads L1. |
| `STATE_FILES` | `scratchpad.rs:80` | `["CONTEXT.md","DECISIONS.md","LEARNINGS.md","OPERATOR-CHECKLIST.md","STATUS.md","TASKS.md"]` | Each filename also spelled individually in `hook/session_start.rs` (`STATE_FILE_NOTES` at line 68, and `CONTEXT.md`/`DECISIONS.md`/etc. at lines 70,74,78,82,86,89 per consts.md) | `scratchpad::STATE_FILES` is the canonical array; `session_start.rs::STATE_FILE_NOTES` independently re-spells the same six file names alongside prose notes – see Leads L2 |
| `README_NAME` | `scratchpad.rs:90` | `"README.md"` | Generic literal, appears 17 times across the workspace per literals.md (mostly tests/docs) – not a meaningful dup |
| `now_secs_or` fallback pattern | `heartbeat.rs:286` | n/a | `ss_magic_core::release::now_secs()` (owning "now in seconds" primitive, re-exported at `scratchpad.rs:640`) | `now_secs_or` is a **near-duplicate**: same `SystemTime::now().duration_since(UNIX_EPOCH)` body as `release::now_secs`, just with a fallback parameter instead of defaulting to 0 |
| civil-from-days constants (`719_468`, `146_097`, `1460`, `36_524`, `146_096`, `365`, `153`) | `scratchpad.rs:661-669` (`format_rfc3339`) | Hinnant algorithm literals | Identical constant set at `crates/ss-magic/src/sync/reverse_sync.rs:713-` (backup timestamp formatter) | Both are documented as intentionally-separate implementations (different output audiences) in the module doc comments – see Leads L3 |

## Cross-module references

**Imports into this partition:**
- `atomic.rs`: only `std`/`anyhow`/`tempfile`. No intra-crate imports.
- `claim.rs`: only `std::fs`, `std::path::Path`, `tempfile::NamedTempFile`.
- `tmproot.rs`: `crate::hashing` (re-exported core hashing under `crate::` path), `ss_magic_core::git::discover::effective_uid`.
- `identity.rs`: `crate::git`, `ss_magic_core::reponame`.
- `scratchpad.rs`: `crate::git`, `crate::atomic`, `crate::identity::{self, Identity}`, `ss_magic_core::style`, `ss_magic_core::state_tree::{STATE_REL, ensure_state_ignored}` (re-exported), `ss_magic_core::release::now_secs` (re-exported).
- `heartbeat.rs`: `crate::atomic`, `crate::scratchpad::format_rfc3339`, `crate::tmproot`.
- `pathnorm.rs`: only `std::path`.

**Callers of this partition's public symbols, elsewhere in the crate (≤5 each):**
- `atomic::write_atomically` – used in 13 other files: `release_check.rs:108`, `cache.rs:387`, `bypass.rs:130`, `compact_window.rs:327`, `ledger.rs:354,1123,1154`, `expect_artifact.rs:234`, `setup_ci.rs:444`, `checklist/verbs.rs:1292,1312` (and its own callers in `heartbeat.rs:352`, `scratchpad.rs:620`).
- `claim::take` – `bypass.rs:153`, `expect_artifact.rs:277`.
- `tmproot::with_lock` – `ledger.rs:874`, `config.rs:60`, `heartbeat.rs:257`, `hook/file_changed.rs:217`, `checklist/verbs.rs:1333`, plus test files.
- `tmproot::try_with_lock` – `release_check.rs:258,334`, `config.rs:545`.
- `tmproot::resolve_root` – `release_check.rs:735`, `config.rs:59,538`, `hook/file_changed.rs:216`, `hook/session_start.rs:140`.
- `tmproot::identifier` / `NAMESPACE_DIR` – only `status.rs:832,835` (diagnostic re-derivation of the root path, read-only).
- `tmproot::INSTALL_LOCK_NAME` – **unused outside this file** (grep found no reference anywhere in the crate besides its own declaration and `#[allow(dead_code)]` marker); only the shell-side `tmproot.sh` spells the literal independently.
- `identity::resolve` – `scratchpad.rs:381`, `status.rs:1071`, plus 3 test call sites.
- `identity::Identity` (struct) – `scratchpad.rs:54,591`, `status.rs`.
- `scratchpad::ensure` – `cache.rs:838`, `bypass.rs:238`, `expect_artifact.rs:492`, `hook/pre_compact.rs:98`, `hook/subagent_stop.rs:113`, `hook/session_start.rs:171`, `checklist/verbs.rs:514` (7 call sites, the widest fan-out symbol in this partition).
- `scratchpad::STATE_REL` – `cache.rs`, `bypass.rs`, `hook/pre_tool_use.rs:64`, `hook/pre_compact/tests.rs:234`, `status.rs:1265,1270`, `checklist/verbs.rs:85`.
- `scratchpad::STATE_FILES` – `hook/session_start.rs` (doc comment), `hook/session_start/tests.rs:475`, `hook/pre_compact.rs` (doc comment).
- `scratchpad::format_rfc3339` – `ledger.rs:75`, `cache.rs:420`, `release_check.rs:44`, `spill_index.rs:43`, `heartbeat.rs:51`, `hook/pre_compact.rs:197`, `hook/subagent_stop.rs:419,499`, `hook/session_start.rs:303`, `checklist/schema.rs:76`, `checklist/render.rs:66`, `checklist/verbs.rs:85`, `status.rs:1669`, `compact_window/tests.rs:15`.
- `scratchpad::now_secs` – `ledger.rs:75`, `cache.rs:77`, `bypass.rs:50`, `expect_artifact.rs:66`, `checklist/verbs.rs:85`.
- `scratchpad::ensure_state_ignored` – `config.rs:636,748`.
- `scratchpad::Report`/`Refusal` – `hook/pre_compact.rs:47`, `hook/subagent_stop.rs:59`, `hook/session_start.rs:33`.
- `heartbeat::store_dir` – `ledger.rs:1232`, `hook/mod.rs:366`, `hook/session_end.rs:54`.
- `heartbeat::existing_store_dir` – `compact_window.rs:737`, `status.rs:1937`.
- `heartbeat::append`, `Row`, `Outcome`, `log_path`, `read` – `hook/mod.rs`, `status.rs` (diagnostic report), plus extensive test call sites.
- `pathnorm::normalize` – `expect_artifact.rs:63` (import), `hook/pre_tool_use.rs:287,290,294,413`.
- `pathnorm::home_relative`/`HomeRelative`/`process_view`/`ProcessView` – all consumed exclusively by `hook/pre_tool_use.rs` (lines 280-314); **unused outside `hook/pre_tool_use.rs`** among production code (only referenced again in its own test file).

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `scratchpad.rs:113` (`CLAIM_DIRS`) vs `cache.rs:84`, `bypass.rs:56`, `expect_artifact.rs:72` (`DIR_NAME`) | `scratchpad::CLAIM_DIRS = ["conclusions", "bypass", "expect-artifact"]` hardcodes the same three directory names that `cache::DIR_NAME`, `bypass::DIR_NAME`, and `expect_artifact::DIR_NAME` each separately own as `pub` consts. A rename of one of those three `DIR_NAME`s would silently desync from `scratchpad.rs`'s copy since nothing imports them here. | `cache.rs:84`, `bypass.rs:56`, `expect_artifact.rs:72` – each is `pub`, so `scratchpad.rs` could build `CLAIM_DIRS` from `[cache::DIR_NAME, bypass::DIR_NAME, expect_artifact::DIR_NAME]` instead of re-typing the strings |
| L2 | duplication | `scratchpad.rs:80` (`STATE_FILES`) vs `hook/session_start.rs:68` (`STATE_FILE_NOTES`) | The six state-file names (`CONTEXT.md`, `DECISIONS.md`, …) are spelled once in `STATE_FILES` and again individually inside `session_start.rs`'s `STATE_FILE_NOTES` table – two arrays that must stay in the same order and same six names by convention alone, with no shared constant or test tying them together beyond what the docs promise. | `hook/session_start.rs:65-90` (module doc + `STATE_FILE_NOTES` declaration) |
| L3 | duplication | `scratchpad.rs:655-672` (`format_rfc3339`) vs `crates/ss-magic/src/sync/reverse_sync.rs` backup-timestamp formatter (~line 713 onward) | Both hand-roll the identical Hinnant civil-from-days algorithm (same magic constants `719_468`, `146_097`, `1460`, `36_524`, `146_096`, `365`, `153`) to format a Unix timestamp, differing only in the final string shape. The module doc at `scratchpad.rs:647-654` already argues this is deliberate (different audiences/output shapes), so this is a documented-but-still-real algorithmic duplication; a shared `days_to_ymd`/`seconds_to_hms` decomposition could serve both format strings. Lower confidence given the explicit rationale already in comments. | `crates/ss-magic/src/sync/reverse_sync.rs` (`apply_timestamp` / the pure civil-from-days helper it calls) |
| L4 | reuse | `scratchpad.rs:594-606` (`write_pointer`'s lock block) vs `tmproot.rs:221-247` (`open_lock_file` + `with_lock`) | `write_pointer` hand-opens its lock file with the exact same `OpenOptions().read(true).write(true).create(true).truncate(false)` shape as `tmproot::open_lock_file`, then wraps it in `fd_lock::RwLock::new(...).write()` directly, instead of calling `tmproot::with_lock(state_root, POINTER_LOCK_NAME, ...)`. The only functional difference is the explicit `.mode(FILE_MODE)` on the open call, which `tmproot::open_lock_file` lacks. | `tmproot.rs:221-247` (`open_lock_file`, `with_lock`) – also compare `crates/ss-magic/src/update/apply.rs:242` (`open_lock_file`), a third near-identical copy per functions.md, cross-partition |
| L5 | const-location | `tmproot.rs:101` `DIR_MODE = 0o700`; `heartbeat.rs:71,73` `DIR_MODE`/`FILE_MODE`; `scratchpad.rs:119,122` `DIR_MODE`/`FILE_MODE` | Eight independent `DIR_MODE = 0o700` consts and nine independent `FILE_MODE = 0o600` (mostly) consts exist across the plugin crate (per consts.md's `dup-name` flags), three of them inside this partition. None is imported from a shared location; each module re-declares the same octal literal. A single `crate`-level `OWNER_ONLY_DIR`/`OWNER_ONLY_FILE` pair (or constants module) would remove the duplication without weakening any check. | `bypass.rs:67-68`, `cache.rs:106-107`, `expect_artifact.rs:92-93`, `hook/subagent_stop.rs:85-86`, `ledger.rs:100-101`, `hook/file_changed.rs:95`, `hook/pre_compact.rs:67` – cross-partition, same fix would touch modules outside this recon's file set |
| L6 | duplication | `scratchpad.rs:66` `STATE_QUERY = ".superset/.magic/"` vs `hook/mod.rs:93` `STATE_QUERY` (same name, same value) | Two private consts named `STATE_QUERY` holding the identical directory-ignore-query string, declared independently in two files. A change to the trailing-slash convention (documented as load-bearing for `git::is_ignored_str`) has to be made in both places with nothing enforcing that. | `hook/mod.rs:93` (cross-partition – hook module, not in this file set but same crate) |
| L7 | efficiency / reuse | `heartbeat.rs:286` (`now_secs_or`) vs `ss_magic_core::release::now_secs` (re-exported at `scratchpad.rs:640`) | `now_secs_or` re-implements the identical `SystemTime::now().duration_since(UNIX_EPOCH).map(...).unwrap_or(...)` body that `release::now_secs` already provides, just swapping the `unwrap_or(0)` default for a caller-supplied fallback. Could be `release::now_secs_or_similar` or simply `now_secs().max(...)`-style composition using the existing primitive, though the semantics (fallback vs. floor) would need checking before merging. | `crates/ss-magic-core/src/release.rs:199` (`now_secs`) |
| L8 | reuse | `scratchpad.rs:326-331` (`Ctx::rel_of`) | `rel_of` (`path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().into_owned()`) is a small, generic "make a path repo-relative for display" helper, private to `scratchpad::Ctx`. Similar ad hoc `to_string_lossy().into_owned()` conversions recur in `ledger.rs` (lines 619, 1024, 1025, 1035, 1050) and `spill_index.rs` (219, 273) without a shared name – not necessarily worth unifying given differing "unwrap_or" fallback semantics, but worth a look since the pattern repeats at least 8 times crate-wide. | `ledger.rs:619,1024-1050`, `spill_index.rs:219,273` – cross-partition |
| L9 | dead-code | `tmproot.rs:97` `INSTALL_LOCK_NAME` (`#[allow(dead_code)]`) | Marked dead by the compiler and left with an explicit `#[allow]`; genuinely unused from Rust (grep across the crate found no caller besides the declaration), existing purely so the constant's *text* is visible next to its shell-side twin in `tmproot.sh`. This is explained in the doc comment at lines 84-88, so it is a deliberate documentation device rather than an oversight – flagged for completeness, likely not actionable. | `plugin/lib/tmproot.sh` (shell counterpart, outside Rust source) |
| L10 | naming | `scratchpad.rs:96` `POINTER_NAME = "current.json"` vs `checklist/verbs.rs:101` `POINTER_NAME = "checklist.json"` | Two unrelated pointer files share the const identifier `POINTER_NAME` in different modules with different values. Not a bug (both are module-private), but a reader grepping for `POINTER_NAME` gets two unrelated hits – a naming clarity nit, not a behavior issue. | `checklist/verbs.rs:101` – cross-partition |
| L11 | efficiency | `heartbeat.rs:315-354` (`prune`) reads the whole file into a `String`, splits into `Vec<&str>` lines, filters, and rewrites via `atomic::write_atomically` – only triggered past `PRUNE_TRIGGER_BYTES` (256 KiB) so cost is bounded, but the `text.lines()` scan runs twice (once at line 323 for `total`, again at line 325 for `kept`) over the same string. | Two passes over `text.lines()` could be one; low-value given the 256 KiB trigger bounds the cost already, and the doc explicitly says "best-effort, never the reason a hook fails" – flag only, not a hot path. | n/a (self-contained within `heartbeat.rs`) |
| L12 | hardcoded-value | `scratchpad.rs:125-129` `SCRATCHPAD_USAGE` names only the `ensure` subverb; `run` (line 746) matches `Some("ensure")` and falls through to the same usage text for `Some(other)` and `None`. No test coverage gap noted here – just flagging that adding a second subverb means touching both the match arm and the usage string by hand, a common but easy-to-miss two-spot edit. | n/a – pattern note, not a specific duplicate elsewhere | 
| L13 | reuse | `atomic.rs:51-89` (`write_atomically`) vs `ss-magic-core/src/superset_files.rs:180` (private `write_atomically`) | Two independently-implemented atomic-write helpers exist in the workspace under the same function name: the plugin's general-purpose one in this partition, and core's private one (used for `magic.json`/`magic.local.json`, writing THROUGH a symlink via `canonicalize` first, with its own per-writer `AtomicU64` sequence counter for temp-file uniqueness). `structure-pins.md` explicitly flags this as legitimate to report (core cannot depend on the plugin crate, so a shared copy would have to live in core and be re-exported by the plugin) – listed here as the cross-partition instance. | `ss-magic-core/src/superset_files.rs:180-211` |

Files mapped: 7 (atomic.rs, claim.rs, tmproot.rs, identity.rs, scratchpad.rs, heartbeat.rs, pathnorm.rs).
