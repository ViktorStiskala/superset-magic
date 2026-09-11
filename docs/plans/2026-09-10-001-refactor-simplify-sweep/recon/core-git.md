# Recon: core-git – git probes, gitignore, discover

Files: `crates/ss-magic-core/src/git/mod.rs` (421), `crates/ss-magic-core/src/git/gitignore.rs` (302),
`crates/ss-magic-core/src/git/discover.rs` (664).

## Map

### `git/mod.rs` – subprocess `git`/`gh` probes and mutating primitives, shared by both binaries

Public:
- `cwd_repo_root(cwd: &Path) -> Result<PathBuf>` (61) – `git rev-parse --show-toplevel`, resolved/canonicalized.
- `is_worktree(cwd_root: &Path) -> Result<bool>` (69) – true if `.git` is a file, or `git-dir` != `git-common-dir`.
- `main_checkout_root(cwd_root: &Path) -> Result<PathBuf>` (83) – parent of `git rev-parse --git-common-dir`.
- `origin_url(cwd_root: &Path) -> Result<Option<String>>` (96) – `git remote get-url origin`.
- `main_branch_name(main_root: &Path) -> Result<String>` (102) – `"main"` else `"master"` else error.
- `stage_paths(repo_root: &Path, paths: &[&str]) -> Result<()>` (131) – `git add --` per existing path.
- `nothing_to_commit(repo_root: &Path) -> Result<bool>` (144) – `git diff --cached --quiet` exit code.
- `commit(repo_root: &Path, msg: &str) -> Result<()>` (157).
- `push(repo_root: &Path, remote: &str, branch: &str) -> Result<String>` (163).
- `push_upstream(repo_root: &Path, remote: &str, branch: &str) -> Result<String>` (168).
- `create_branch(repo_root: &Path, name: &str) -> Result<()>` (173) – `git switch -c`.
- `gh_available() -> bool` (181).
- `pr_create(repo_root: &Path, base: &str) -> Result<String>` (194) – `gh pr create --fill --base`.
- `untracked_files(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<PathBuf>>` (238) – `git ls-files --others -z`, includes gitignored.
- `is_ignored(repo_root: &Path, rel: &Path) -> Result<bool>` (273) – UTF-8-checked wrapper over `is_ignored_str`.
- `is_ignored_str(repo_root: &Path, rel_str: &str) -> Result<bool>` (285).
- `is_ignored_no_index_str(repo_root: &Path, rel_str: &str) -> Result<bool>` (299) – rules-only, `--no-index`.
- `tracked_files(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<PathBuf>>` (331) – `git ls-files --cached -z`.
- `status_porcelain(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<(String, PathBuf)>>` (346) – built on `git_raw`, not `git`.
- `symbolic_ref_head(cwd_root: &Path) -> Result<Option<String>>` (390).
- `short_head_sha(cwd_root: &Path) -> Result<Option<String>>` (401).
- `timestamp_branch_suffix() -> Result<String>` (407) – shells `date +%Y%m%d-%H%M%S`.

Private (small/generic – candidates to compare against similar helpers elsewhere):
- `git_raw(args: &[&str], cwd: Option<&Path>) -> Result<Output>` (14) – base subprocess spawn.
- `git(args, cwd) -> Result<String>` (26) – trims stdout, errors on non-zero.
- `git_optional(args, cwd) -> Result<Option<String>>` (37) – non-zero → `Ok(None)`.
- `resolve(path: &str, base: &Path) -> Result<PathBuf>` (48) – join-if-relative + canonicalize. **Near-identical to `discover.rs`'s `resolve_against` (619)** – see Leads.
- `parse_ls_files_z(out: &str) -> Vec<PathBuf>` (251) – NUL-split, drops absolute/`..` entries; shared by `untracked_files` and `tracked_files`.
- `check_ignore(repo_root, rel_str, no_index: bool) -> Result<bool>` (306) – shared body of the two ignore probes.

### `git/gitignore.rs` – `.gitignore` read/append helpers at a git root

Public:
- `enum PathKind { File, Dir }` (20).
- `enum Ignored { Already, Appended }` (29).
- `ensure_entry(git_root: &Path, line: &str) -> Result<()>` (46) – append-if-missing primitive.
- `ensure_path_ignored(target_root, rule_source_root, rel, kind) -> Result<Ignored>` (197) – the one entry point; prefers a covering glob from the source tree, else an anchored literal.

Private:
- `struct CoveringRule { pattern: String, source_dir: Option<PathBuf> }` (88).
- `find_covering_rule(worktree_root: &Path, rel: &Path) -> Result<Option<CoveringRule>>` (116) – `git check-ignore -v --no-index`.
- `parse_covering_line(line: &str) -> Option<CoveringRule>` (151) – parses `<source>:<line>:<pattern>\t<pathname>`.
- `is_ignored_opt(root, rel, kind) -> Option<bool>` (242) – `None` on git failure (tolerant probe).
- `closest_gitignore_dir(target_root, rel) -> PathBuf` (254).
- `anchored_literal(target_root, gi_dir, rel, kind) -> Result<String>` (277).

### `git/discover.rs` – subprocess-free discovery of worktree/main-checkout roots (hook fast path)

Public:
- `const GITFILE_MAX_BYTES: usize = 4096` (107).
- `const DECLINING_ENV: [(&str, &str); 6]` (119).
- `struct Roots { worktree_root, common_dir, main_checkout_root }` (148).
- `enum Discovery { Found(Roots), NotARepository, Undecided(&'static str) }` (165).
- `struct Resolved { repo_root, main_root, fallback }` (179).
- `discover(cwd: &Path) -> Discovery` (193).
- `discover_with_env(cwd, env: &dyn Fn(&str) -> Option<OsString>) -> Discovery` (200) – injectable-env variant for tests.
- `roots(cwd: &Path) -> Resolved` (238) – the pipeline's entry point; `Undecided` triggers the subprocess probes.
- `effective_uid() -> u32` (653) – raw `geteuid()`; also imported directly by the plugin's `tmproot.rs`.

Private:
- `const HEAD_READ_BYTES: usize = 255` (112).
- `const KNOWN_EXTENSIONS: [&str; 8]` (135).
- `enum Step { Found(Roots), Decline(&'static str), Absent }` (269).
- `looks_like_git_dir(dir: &Path) -> bool` (280).
- `inspect_dot_git(dir: &Path, euid: u32) -> Step` (289).
- `from_git_directory(dir, dot_git, euid) -> Step` (317).
- `from_gitfile(dir, dot_git, euid) -> Step` (336).
- `git_directory_shape(git_dir, common, euid) -> bool` (401).
- `head_content_is_valid(bytes: &[u8]) -> bool` (421).
- `is_c_space(b: u8) -> bool` (432).
- `searchable_dir(path: &Path, euid: u32) -> bool` (438).
- `struct FormatScan { unparseable, unsupported_format, core_override, worktree_config }` (449).
- `repository_format_check(git_dir, common, has_common: bool) -> Result<(), &'static str>` (474).
- `scan_format(text: &str) -> FormatScan` (522) – line-at-a-time INI-ish scanner.
- `read_capped(path: &Path, cap: usize) -> io::Result<Option<Vec<u8>>>` (596) – generic bounded-read helper. Candidate to be shared/duplicated elsewhere (used twice in this file for HEAD and gitfile/commondir reads).
- `trim_trailing_newlines(bytes: &[u8]) -> &[u8]` (608).
- `resolve_against(bytes: &[u8], base: &Path) -> io::Result<PathBuf>` (619) – join-if-relative + canonicalize, byte-slice version. See Leads (near-dup of `mod.rs::resolve`).
- `owned_by(path: &Path, euid: u32) -> bool` (632).
- `same_device(a: &Path, b: &Path) -> io::Result<bool>` (639).
- `extern "C" fn geteuid() -> u32` (660).

## Constants and literals

| Item | file:line | value | denotes | elsewhere in workspace? |
|---|---|---|---|---|
| `GITFILE_MAX_BYTES` | discover.rs:107 | `4096` | cap on gitfile/commondir read size (KTD8's 4 KiB) | consts.md lists it once; no other 4096-byte size constant in workspace shares this meaning (`BYTES_PER_LINE`=40, `MAX_DIFF_BYTES`=2 MiB, `MAX_TRANSCRIPT_BYTES`=16 MiB, `SALVAGE_BYTE_BUDGET`=64 KiB are unrelated magnitudes/purposes) – no collision, but note `read_capped`'s two call sites pass `GITFILE_MAX_BYTES` and `HEAD_READ_BYTES` as the same parameter type, so a future caller could accidentally swap them (see L9). |
| `HEAD_READ_BYTES` | discover.rs:112 | `255` | how much of `HEAD` to read, matches git's `validate_headref` 256-byte NUL-terminated buffer | private, single use (415); not duplicated elsewhere. |
| `DECLINING_ENV` | discover.rs:119 | 6-entry `[(&str,&str);6]` table of `GIT_*` var names + reasons | env vars that force fallback to subprocess probes | Each name string (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_CEILING_DIRECTORIES`, `GIT_DISCOVERY_ACROSS_FILESYSTEM`, `GIT_OBJECT_DIRECTORY`) also appears in `discover/tests.rs` (854-890) as literal env-var names in tests – expected, not a stray duplicate. |
| `KNOWN_EXTENSIONS` | discover.rs:135 | `[&str; 8]` (`noop, preciousobjects, partialclone, worktreeconfig, objectformat, compatobjectformat, refstorage, relativeworktrees`) | git repo-format extensions this walk tolerates | Only definition; no other extension list in workspace. |
| `.git` (string literal) | discover.rs:290, mod.rs:70 (via `.join(".git")`) | `".git"` | the git dir/gitfile entry name | Appears 33× across the workspace per literals.md (tests, reponame.rs:64, repo_scan.rs:16, verbs.rs:1350, etc.) – ordinary, not a hidden constant that should move. |
| `HEAD` | discover.rs:281, 402 | `"HEAD"` | the ref file name | Appears in mod.rs:392,402 (`symbolic-ref …HEAD`, `rev-parse --short HEAD`) and tests – same well-known git name, not a candidate for a shared const given how differently each site uses it (filename vs. rev arg). |
| `commondir` | discover.rs:285, 318, 358 | `"commondir"` | the linked-worktree pointer file name | Only in discover.rs + its tests; not duplicated in gitignore.rs or mod.rs. |
| `"not-ignored"` | (not in these 3 files; owned by plugin's `hook/mod.rs:113` as `REASON_NOT_IGNORED`) | – | – | listed in literals.md; **not present in this partition** – flag only if a core-git file is later found emitting the bare string without the plugin's const (checked: none here). |
| `gitdir: ` | discover.rs:342 | `b"gitdir: "` | gitfile content prefix | Only here + its test (191); git's own fixed format, not reusable elsewhere. |
| `4 KiB` reason strings | discover.rs:339, 361 (`"gitfile is larger than 4 KiB"`, `"commondir is larger than 4 KiB"`) | literal text repeating the `GITFILE_MAX_BYTES` magnitude in prose | Both derived from the same const value but as free-standing strings, not `format!("...{GITFILE_MAX_BYTES}...")` – a docs-drift risk if the const ever changes (see L8). |
| `146_097`, `719_468`, `1_460`, `36_524`, `146_096`, `365`, `153`, `400` etc. | **not in this partition** | civil-date math constants live in `reverse_sync.rs:707-720` and `checklist/schema.rs:670-679`, NOT in these three files | cross-partition note only; core-git has no date-civil arithmetic. |
| `env` var names (`GIT_DIR` etc.) | discover.rs:120-128 | see `DECLINING_ENV` | – | same as above. |
| `255`/`4096` byte caps | discover.rs:107, 112 | – | – | see rows above. |

No `.gitignore`-specific magic strings (`.gitignore`, `.superset/backups`, `.superset/.magic`) are hardcoded literally in `gitignore.rs` itself – callers (`state_tree.rs`, `reverse_sync.rs`, `compact_window.rs`) pass the `rel` path in; `gitignore.rs` only hardcodes the filename `".gitignore"` (mod.rs? – actually gitignore.rs:47: `path.join(".gitignore")`) and nothing else. That literal is the sole hardcode in that file and is exactly the standard name – not a lead.

## Cross-module references

**Imports into these files:**
- `git/mod.rs` imports only `std` + `anyhow`; declares `pub mod discover; pub mod gitignore;`.
- `git/gitignore.rs` imports `crate::git` (for `super::git_raw` at 120, and `git::is_ignored_str` at 247).
- `git/discover.rs` imports only `std` (no dependency on `git/mod.rs` or `gitignore.rs`, except `super::cwd_repo_root` / `super::main_checkout_root` inside `roots()` at 251/255 – the one place `discover.rs` calls back into `mod.rs`'s subprocess probes).

**Usage of public symbols elsewhere in the workspace** (file:line, capped at 5):
- `git::cwd_repo_root` – used in ~20+ call sites across both binaries: `crates/ss-magic-plugin/src/identity.rs:56`, `crates/ss-magic-plugin/src/config.rs:526`, `crates/ss-magic-plugin/src/scratchpad.rs:379`, `crates/ss-magic/src/pack.rs:120`, `crates/ss-magic/src/main.rs:149`. (Heavily used – core plumbing.)
- `git::is_worktree` – `crates/ss-magic/src/tui/menu.rs:131`, `crates/ss-magic-plugin/src/status.rs:896`.
- `git::main_checkout_root` – `crates/ss-magic-core/src/reponame.rs:29`, `crates/ss-magic-core/src/git/discover.rs:255` (internal), `crates/ss-magic-plugin/src/config.rs` (via `resolve`), `crates/ss-magic/src/main.rs`, `crates/ss-magic/src/tui/menu.rs`.
- `git::origin_url` – `crates/ss-magic-core/src/reponame.rs:24`, `crates/ss-magic-plugin/src/checklist/verbs.rs:1348`.
- `git::main_branch_name` – only `crates/ss-magic/src/workspace/migrate.rs:622,652` (CLI-only, matches its doc comment about the final-action step).
- `git::stage_paths` / `nothing_to_commit` / `create_branch` / `push_upstream` / `gh_available` / `pr_create` – all used ONLY in `crates/ss-magic/src/workspace/migrate.rs` (613-653), nowhere else. These six mutating primitives are effectively single-caller; consider whether they need to be `pub` on the crate boundary at all vs. `pub(crate)` scoped tighter, though core's own visibility rules likely require `pub` for cross-crate reuse by the CLI binary.
- `git::push` (bare, not `push_upstream`) – **no call site found** outside `git/mod.rs` itself; `push_upstream` is used instead. Possible dead code (see Leads).
- `git::untracked_files` – `crates/ss-magic/src/sync/reverse_sync.rs:130` (only call site).
- `git::tracked_files` – `crates/ss-magic-plugin/src/scratchpad.rs:461`, `crates/ss-magic/src/sync/reverse_sync.rs:197`.
- `git::is_ignored` (the `Path`-typed, non-`_str` variant) – **no call site found** anywhere outside its own definition; all callers use `is_ignored_str` or `is_ignored_no_index_str` directly. Likely dead code (see Leads).
- `git::is_ignored_str` – `crates/ss-magic-core/src/git/gitignore.rs:247`, `crates/ss-magic-plugin/src/scratchpad.rs:65` (doc-comment only reference; check for a real call) – actual call sites are mostly in tests; production call is `gitignore.rs:247`.
- `git::is_ignored_no_index_str` – `crates/ss-magic-plugin/src/scratchpad.rs:439`, `crates/ss-magic-plugin/src/status.rs:1271`, `crates/ss-magic-plugin/src/hook/mod.rs:620`.
- `git::status_porcelain` – only `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:1535`.
- `git::symbolic_ref_head` – only `crates/ss-magic-plugin/src/identity.rs:92`.
- `git::short_head_sha` – only `crates/ss-magic-plugin/src/identity.rs:106`.
- `git::timestamp_branch_suffix` – only `crates/ss-magic/src/workspace/migrate.rs:638`.
- `gitignore::ensure_path_ignored` – `crates/ss-magic-core/src/state_tree.rs:46`, `crates/ss-magic-plugin/src/compact_window.rs:259`, `crates/ss-magic/src/workspace/migrate.rs:90`, `crates/ss-magic/src/sync/reverse_sync.rs:273,671`.
- `gitignore::ensure_entry` – `crates/ss-magic/src/workspace/migrate/tests.rs:264` (only non-test-of-itself call site found).
- `gitignore::PathKind` / `Ignored` – used at each `ensure_path_ignored` call site above.
- `discover::roots` – `crates/ss-magic-plugin/src/compact_window.rs:731`, `crates/ss-magic-plugin/src/ledger.rs:440`, `crates/ss-magic-plugin/src/hook/mod.rs:475`.
- `discover::effective_uid` – `crates/ss-magic-plugin/src/tmproot.rs:79,151` (imported directly, cross-crate, explicitly documented as shared).
- `discover::GITFILE_MAX_BYTES`, `DECLINING_ENV` – no external (non-test) call sites found; used only within `discover.rs` itself. `#[allow(dead_code)]` is already present on `Roots::common_dir` (155), acknowledging it's unread outside tests.

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `crates/ss-magic-core/src/git/mod.rs:48` vs `crates/ss-magic-core/src/git/discover.rs:619` | `resolve()` (str path, join-if-relative + canonicalize) and `resolve_against()` (byte-slice path, join-if-relative + canonicalize) do the same join/canonicalize dance with different input types; both exist in the same partition. | Compare signatures directly: `resolve(path: &str, base: &Path) -> Result<PathBuf>` (mod.rs:48-57) vs `resolve_against(bytes: &[u8], base: &Path) -> io::Result<PathBuf>` (discover.rs:619-628). One could delegate to the other via a `&str`/`&[u8]` conversion, or share one private helper in `mod.rs` that both call (discover.rs already calls back into `mod.rs` at line 251/255). |
| L2 | dead-code | `crates/ss-magic-core/src/git/mod.rs:163` (`push`) | `push(repo_root, remote, branch)` has no call site anywhere in the workspace outside its own definition; every caller uses `push_upstream` instead (`migrate.rs:642`). | `crates/ss-magic/src/workspace/migrate.rs:642` (the only push-family call, and it's `push_upstream`). Confirm via full-workspace grep before removing. |
| L3 | dead-code | `crates/ss-magic-core/src/git/mod.rs:273` (`is_ignored`) | The `Path`-typed `is_ignored()` wrapper (UTF-8-checks then calls `is_ignored_str`) has no non-test caller; all production call sites (`gitignore.rs:247`, `scratchpad.rs:439`, `status.rs:1271`, `hook/mod.rs:620`) go directly to `is_ignored_str` or `is_ignored_no_index_str`. | `crates/ss-magic-core/src/git/gitignore.rs:247` and the plugin call sites listed above – none takes a `&Path`, they all already have a `&str`. |
| L4 | efficiency / reuse | `crates/ss-magic-core/src/git/mod.rs:69` (`is_worktree`) | `is_worktree` always does at least one filesystem check (`dot_git.is_file()`) and, on the common "main checkout" path, two extra `git rev-parse` subprocess spawns to compare git-dir vs common-dir – while `discover::roots()` (discover.rs:238) already derives the identical worktree/main distinction from the filesystem alone in the fast path. `is_worktree` is deliberately kept subprocess-based per R22 (CLI commands keep the probes) – flag as a possible reuse opportunity ONLY for a future caller that already has `discover::Roots`, not as a mandate to change `is_worktree` itself (R22 pin). | `crates/ss-magic-core/src/git/discover.rs:238-263` (`roots`) and its `Roots.worktree_root == Roots.main_checkout_root` equivalence for a main checkout. Low confidence given the R22 pin – flag but do not propose removing the CLI's subprocess path. |
| L5 | naming / efficiency | `crates/ss-magic-core/src/git/mod.rs:337-376` (`status_porcelain`) | Correctly built on `git_raw` (not the trimming `git()`) per the documented incident, but re-implements its own success/stderr-bail logic (359-363) that duplicates the body of `git()` (26-33) minus the final trim. Not a bug (the pin explicitly requires avoiding `git()`), but the error-handling boilerplate (`if !out.status.success() { …bail!… }`) is copy-pasted rather than factored into a shared "run and get raw output, but check success" helper. | `crates/ss-magic-core/src/git/mod.rs:26-33` (`git`) – same bail! pattern, same stderr formatting `` `git {}` failed: {stderr} ``. A small `git_raw_checked(args, cwd) -> Result<Output>` used by both would remove the duplicate error text/logic while keeping the untrimmed stdout `status_porcelain` needs. |
| L6 | duplication | `crates/ss-magic-core/src/git/discover.rs:596` (`read_capped`) | Generic bounded-read helper (open, read up to `cap+1` bytes, `None` if over cap) used twice in this file (HEAD at 409, gitfile body at 337, commondir at 359) – a reasonable one-off, but check whether an equivalent bounded-read pattern exists elsewhere in the workspace (e.g. transcript salvage's `SALVAGE_BYTE_BUDGET`, checklist gitfile-like reads) that could share it, or whether it's fine as a local-only helper. | `crates/ss-magic-plugin/src/hook/subagent_stop.rs` (`MAX_TRANSCRIPT_BYTES`/`SALVAGE_BYTE_BUDGET` truncation logic) – different partition; note only, do not merge without checking that partition's recon. |
| L7 | hardcoded-value | `crates/ss-magic-core/src/git/discover.rs:339,361` | The strings `"gitfile is larger than 4 KiB"` and `"commondir is larger than 4 KiB"` restate the `GITFILE_MAX_BYTES` (4096) magnitude in prose rather than deriving it, so a future change to the const would silently desync the message from the actual limit. | `crates/ss-magic-core/src/git/discover.rs:107` (`GITFILE_MAX_BYTES`). Low-severity (static `&'static str` reasons can't easily embed a `format!`), but worth a comment or a `const` for "4 KiB" text if the cap ever changes. |
| L8 | const-location | `crates/ss-magic-core/src/git/discover.rs:107` (`GITFILE_MAX_BYTES`, pub) vs `:112` (`HEAD_READ_BYTES`, private) | Both are byte-size caps for `read_capped`, one `pub` (used by `from_gitfile`/`from_git_directory` via commondir/gitfile reads, and exported for... check: is `GITFILE_MAX_BYTES` actually read outside this file? Cross-module-references above found NO external call site.) – if nothing outside `discover.rs` reads `GITFILE_MAX_BYTES`, its `pub` visibility may be broader than needed (contrast with `HEAD_READ_BYTES`, correctly private for the same kind of value). | Compare visibility rationale: is there a doc comment or test (`discover/tests.rs`) importing `GITFILE_MAX_BYTES` to justify `pub`? If only doc-linked (`[`GITFILE_MAX_BYTES`]` in the module doc at lines 42-43), consider `pub(crate)` or private. |
| L9 | copy-paste-variant | `crates/ss-magic-core/src/git/discover.rs:401-416` (`git_directory_shape`) vs `:474-509` (`repository_format_check`) | Both are "read a small file, decline on anything unexpected" functions with similar shapes (`fs::symlink_metadata`/`fs::read`, match on `io::ErrorKind::NotFound`, cascading `Decline`/`Err` returns) but are NOT literal copies – flagging only because the pattern of "read this file, treat NotFound as one thing, treat other errors as decline" recurs 4+ times in this file (`from_gitfile`'s two `read_capped` calls at 337/359, plus `repository_format_check`'s two `fs::read` calls at 479/495) with near-identical `match` arms. A shared helper `fn read_optional(path) -> Result<Option<Vec<u8>>, &'static str>` folding "NotFound → None, other error → Decline reason" could reduce 4 near-identical match blocks to 1. | `crates/ss-magic-core/src/git/discover.rs:337-341`, `:359-366`, `:479-483`, `:495-507` – four sites with the same `Ok(bytes) => …, Err(NotFound) => …, Err(_) => Decline` shape. Medium confidence; the exact `Decline` reason text differs per site so a shared helper needs a parameter for the message. |
| L10 | reuse (cross-partition) | `crates/ss-magic-core/src/git/discover.rs:653` (`effective_uid`) | This is the ONE place `geteuid()` is declared; `crates/ss-magic-plugin/src/tmproot.rs:79` imports it directly across the crate boundary rather than redeclaring. This is correct reuse (documented in CLAUDE.md), not a finding – listed here only so the reviewer doesn't flag `tmproot.rs`'s import as suspicious when auditing the plugin-side partition. | `crates/ss-magic-plugin/src/tmproot.rs:77-79` (its own comment explains the sharing). No action needed. |
| L11 | naming | `crates/ss-magic-core/src/git/gitignore.rs:116` (`find_covering_rule`) and `:151` (`parse_covering_line`) | Both are private and named for a very specific one-off (`git check-ignore -v --no-index` line parsing); fine as-is, but note their shared struct `CoveringRule` (88) has a `source_dir: Option<PathBuf>` documented in unusually deep detail (three paragraphs) for a 2-field private struct – a naming/doc-density observation, not a bug. Low priority; mentioned for completeness. | N/A – doc-density only, no code comparison needed. |
| L12 | efficiency | `crates/ss-magic-core/src/git/gitignore.rs:197-236` (`ensure_path_ignored`) | Calls `is_ignored_opt` (itself a `git check-ignore` subprocess) up to 3 times in the worst path (line 203, 222, 232) plus `find_covering_rule` (another subprocess, line 218) – 4 git subprocess spawns for one "ensure ignored" call in the worst case. This is likely necessary given the verify-after-write correctness requirement (documented behavior, not a bug) – flagged for the reviewer's awareness only, comparable to L4's fast-path-vs-subprocess tension. | `crates/ss-magic-core/src/git/discover.rs` – no direct comparison; the gitignore module has no filesystem-only fast path the way `discover.rs` does for root resolution. Could a future gitignore fast-path mirror `discover.rs`'s pattern? Speculative – low confidence, not actionable without more context on call frequency. |
| L13 | hardcoded-value | `crates/ss-magic-core/src/git/mod.rs:405-418` (`timestamp_branch_suffix`) | Shells out to `date +%Y%m%d-%H%M%S` (local timezone) for a branch-name suffix, while `crates/ss-magic/src/sync/reverse_sync.rs:707-720` (`format_timestamp`, UTC) and `crates/ss-magic-plugin/src/checklist/schema.rs:670-679` (`days_from_civil`, the inverse direction) independently hand-roll the SAME Hinnant civil-date algorithm in pure Rust, deliberately avoiding a date crate each time. `timestamp_branch_suffix` avoids the algorithm entirely by shelling to `date`, so it's not a literal duplicate of the arithmetic – but it is a THIRD, different strategy (subprocess vs. two independent from-scratch civil-date implementations) for "get a timestamp string" in the same workspace. | `crates/ss-magic/src/sync/reverse_sync.rs:707-720` and `crates/ss-magic-plugin/src/checklist/schema.rs:670-679` – cross-partition (not in this partition's file set) but worth flagging to whichever partition owns those two, since three different timestamp strategies (shell `date`, hand-rolled UTC civil-from-days, hand-rolled civil-to-days) coexist for related purposes (branch suffix, backup batch naming, checklist timestamp parsing) without a shared time module in `ss-magic-core`. A shared `ss-magic-core` civil-date helper (used by mod.rs, reverse_sync.rs, and checklist/schema.rs) could remove two of the three independent implementations – but `timestamp_branch_suffix`'s local-timezone requirement (deliberately matching what a developer would type by hand, per its doc comment) means it CANNOT simply reuse the two UTC-only implementations without also solving local-timezone lookup, which the other two explicitly avoid. Medium confidence lead; likely belongs to whichever partition covers `reverse_sync.rs` and `checklist/schema.rs`. |
| L14 | dead-code (low confidence) | `crates/ss-magic-core/src/git/discover.rs:119` (`DECLINING_ENV`, pub) and `:107` (`GITFILE_MAX_BYTES`, pub) | Both are `pub` but no non-test call site outside `discover.rs` was found via grep. They may be `pub` purely for the module's doc-comments (`[`DECLINING_ENV`]` link targets) or for a future external consumer, not actual current reuse. | Grep results in Cross-module references above ("no external (non-test) call sites found"). Confidence 40 – a doc-link reference could justify `pub` without a code caller; worth a second grep pass restricted to `///[...]` links before downgrading visibility. |
