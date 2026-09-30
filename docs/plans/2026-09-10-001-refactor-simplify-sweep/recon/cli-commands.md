# Recon: cli-commands (ss-magic CLI: main, cli parser, pack, update, workspace migrate)

## Map

### `crates/ss-magic/src/main.rs` (478 lines)
Composition root: parses argv, initializes style/theme, gates auto-update, dispatches to sync/pack/update/menu, renders sync + pack events.

- `should_run_update_gate` (pub) – `(cmd: Command, guard_active: bool) -> bool` – inclusion-list gate deciding whether the auto-update check runs before a command; `Update` and any guard-active run never gate.
- `version_line` (pub) – `() -> String` – `"ss-magic {CARGO_PKG_VERSION}"`, compile-time sourced.
- `menu_blocked_reason` (pub) – `(stdin_tty: bool, stdout_tty: bool) -> Option<&'static str>` – pure decision of why the interactive menu can't open.
- `load_magic_or_exit` (pub) – `(root: &Path) -> Result<MagicConfig, ExitCode>` – probes `.superset/magic.json`, loads overlaid config, prints styled error + exit code on absence/malformation. Shared by `sync_core` and `pack::pack_core` (cross-crate-internal call from pack.rs via `crate::load_magic_or_exit`).
- `run_pack_flow` (pub) – `(cwd: &Path) -> Result<ExitCode>` – thin wrapper: `pack::pack_core(cwd, print_pack_event)`. Used by `tui/menu.rs`.
- `run_sync_flow` (pub) – `(cwd: &Path, no_backup: bool) -> Result<ExitCode>` – thin wrapper over `sync_core`.
- `run_reverse_sync_flow` (pub) – `(cwd: &Path, no_backup: bool) -> Result<ExitCode>` – hard-errors from main checkout, else bulk-pushes via `sync::reverse_sync::run_bulk`.
- `run` (private) – `() -> Result<ExitCode>` – top-level composition: parse → style init → theme install → match `Parsed`.
- `dispatch` (private) – `(cmd: Command) -> Result<ExitCode>` – routes `Command` variants to their handler (small, but name collides with plugin's own `dispatch` in `crates/ss-magic-plugin/src/main.rs:501` – different signatures, not a real duplicate, just a naming echo).
- `resolve_sync_roots` (private) – `(cwd: &Path) -> Result<(PathBuf, PathBuf), ExitCode>` – resolves cwd-repo-root + main-checkout-root with a styled error on either failure; shared by `sync_core` and `run_reverse_sync_flow`. **Small, generic, single-file-private – candidate to compare against `pack.rs`'s inline root-resolution (pack.rs:120-132), which duplicates one half of this instead of reusing it.**
- `sync_core` (private, generic `F: FnMut(&Event)`) – `(cwd: &Path, no_backup: bool, on_event: F) -> Result<ExitCode>` – full forward-sync flow: resolve roots → load config → empty guard → pre-copy backup pass → `sync::apply::run`.
- `init_noninteractive` (private) – `(patterns: &[String]) -> Result<ExitCode>` – resolves repo root, delegates to `workspace::migrate::run_init_noninteractive`.
- `update_flow` (private) – `() -> Result<ExitCode>` – calls `update::update_command()`, renders the four `UpdateReport` variants.
- `print_pack_event` (private) – `(ev: &pack::PackEvent)` – renders `Add`/`Done` (includes the tar extraction hint + clipboard copy).
- `print_event` (private) – `(ev: &Event)` – renders forward-sync `Copy`/`Skip` events.
- `main` (private) – `() -> ExitCode` – entry point; name collides with plugin's `main` (`crates/ss-magic-plugin/src/main.rs:544`), not a duplicate (each crate needs one).

### `crates/ss-magic/src/cli.rs` (162 lines)
Hand-rolled argv parser (no `clap`) for `ss-magic`'s five subcommands + flags.

- `Command` (pub enum) – `Bare | Sync{no_backup} | ReverseSync{no_backup} | Pack | Update` – `Copy`/`Eq`.
- `Parsed` (pub enum) – `Command(Command) | Init(Vec<String>) | Version | Help | Error(String)`.
- `USAGE` (pub const `&str`) – the usage banner (dup-name with 6 other `USAGE` consts across the workspace – see Constants section).
- `usage` (pub) – `() -> &'static str` – returns `USAGE`. Name collides with plugin's own `usage` (`crates/ss-magic-plugin/src/main.rs:379`), same shape/purpose, different crate – copy-paste-variant, not reuse-worthy (core cannot depend on `clap`-free per-binary usage strings meaningfully, but see Leads).
- `parse` (pub) – `(args: &[String]) -> Parsed` – the whole-argv parse; name collides with 3 other `parse` functions workspace-wide (different signatures/purposes, not real duplicates).
- `has_no_backup` (private) – `(args: &[String]) -> bool` – whole-slice scan for `--no-backup`/`-n`, deliberately position-independent (unlike `-h`/`--help`).
- `version_requested` (private) – `(args: &[String]) -> bool` – whole-slice scan for `--version`/`-V`, deliberately scans past the subcommand token (unlike `-h`).

### `crates/ss-magic/src/pack.rs` (396 lines)
The pack engine: expand overlaid `magic.json` patterns, tar+bzip2 the matches into `ss-magic-<repo>.tar.bz2` at the repo root, excluding the archive itself and every `sync::EXCLUDED_TREES` entry.

- `archive_file_name` (pub) – `(root: &Path) -> String` – `ss-magic-<stem>.tar.bz2`, stem via `repo_name_stem` with a `"files"` fallback.
- `repo_name_stem` (pub(crate), re-export) – `use ss_magic_core::reponame::repo_name_stem;` – re-exported so the plugin's `identity.rs` and this archive name can never disagree.
- `copy_to_clipboard` (pub) – `(text: &str) -> bool` – pipes to the first available clipboard tool; deliberately not called from `pack_core` (kept a pure engine).
  - `TOOLS` (private const, fn-local) – `&[(&str, &[&str])]` – `pbcopy`, `wl-copy`, `xclip -selection clipboard`, `xsel --clipboard --input`.
- `PackEvent` (pub enum) – `Add{rel} | Done{out_path, count}`.
- `pack_core` (pub, generic `F: FnMut(&PackEvent)`) – `(cwd: &Path, on_event: F) -> Result<ExitCode>` – the full flow: resolve root → `crate::load_magic_or_exit` → empty guard → `apply::match_paths` → filter self/root/excluded-tree matches → `write_archive` → zero-count guard → `Done` event.
- `is_pack_archive_rel` (private) – `(rel: &Path) -> bool` – true for a root-level `ss-magic-*.tar.bz2` match (current/legacy/previous-origin names all covered by the glob shape, not by an exact-name list).
- `is_repo_root_rel` (private) – `(rel: &Path) -> bool` – true when every component is `Component::CurDir` or the path is empty (a `.` pattern match).
- `write_archive` (private, generic `F: FnMut(&PackEvent)`) – `(root, rels, out_path, on_event) -> Result<usize>` – builds the tar+bz2 in a `NamedTempFile` in `root`, classifies each match via `symlink_metadata` (symlink / dir / file / special), persists atomically, discards on zero-add.
- `append_dir_excluding_trees` (private, generic `W: Write`) – `(builder, root, rel, abs, added) -> Result<()>` – `WalkDir` with `filter_entry` pruning `sync::under_excluded_tree` matches; the walk that makes the flat retain-filter (pack_core step 6) sufficient for ancestor-directory matches.

### `crates/ss-magic/src/update/mod.rs` (171 lines)
The cheap self-update gate/report layer: the force path (`ss-magic update`) and the auto-update gate main.rs wires in.

- `lock_path` (private) – `() -> Option<PathBuf>` – `release::cache_dir().map(|d| d.join(apply::LOCK_FILE_NAME))`.
- `UpdateReport` (pub enum) – `Updated{version} | AlreadyLatest | Skipped | Unavailable`. Name collides with `crates/ss-magic-plugin/src/release_check.rs`'s own `UpdateReport` struct – different shape (struct vs enum) and different purpose (this crate's force-update outcome vs the plugin's release-notice payload); naming echo, not a real duplicate.
- `update_command` (pub) – `() -> UpdateReport` – wires `update_command_with` to the production resolver (`release::resolve_newest_uncached` on `CLI_LINE`) and swap (`apply::apply_update`/`apply_update_unlocked`).
- `update_command_with` (private, generic `R: FnOnce() -> Option<String>, F: FnOnce(&str) -> ApplyOutcome`) – `(current_version, resolve, run_swap) -> UpdateReport` – the testable decision order (unresolved → filter-fail → not-newer → swap).
- `map_report` (private) – `(outcome: ApplyOutcome) -> UpdateReport` – pure outcome mapping.
- `display_version` (private) – `(raw: String) -> String` – normalizes a raw CLI-line tag to bare `MAJOR.MINOR.PATCH` via `release::parse_line_tag`.
- `auto_update` (pub) – `()` – the gate main.rs calls: `release::check` → on `Newer`, lock + swap + (on success) `reexec_and_exit` (never returns on that path).

### `crates/ss-magic/src/update/apply.rs` (431 lines)
The heavy self-update apply path: fd-lock, `self_update` GitHub backend, re-exec.

- `BIN_NAME` (private const `&str`) – `"ss-magic"` – must equal the crate's package name (pinned by a compile-time test) and the cargo-dist `<bin>` half.
- `LOCK_FILE_NAME` (pub const `&str`) – `"update.lock"` – dup-name with `heartbeat.rs`'s and `ledger.rs`'s own `LOCK_FILE_NAME` (different values, different files; see Constants).
- `UPDATED_ENV` (pub const `&str`) – `"SS_MAGIC_UPDATED"`.
- `NO_UPDATE_ENV` (pub const `&str`) – `"SS_MAGIC_NO_UPDATE"`.
- `STALE_TTL` (private const `Duration`) – `Duration::from_secs(60)` – the lock-file staleness TTL.
- `guard_active` (pub) – `() -> bool` – `env_flag_set(UPDATED_ENV) || env_flag_set(NO_UPDATE_ENV)`.
- `env_flag_set` (private) – `(name: &str) -> bool` – present and not empty/`"0"`.
- `LockState` (pub enum, `#[allow(dead_code)]`) – `Acquired | Contended`.
- `propagate_code` (pub) – `(code: Option<i32>) -> i32` – `code.unwrap_or(1)`.
- `Spawner` (pub trait) – `fn spawn_and_wait(&self, exe: &Path, args: &[OsString]) -> io::Result<Option<i32>>`.
- `ProcessSpawner` (pub struct) – production `Spawner` impl: `Command::new(exe).args(args).env(UPDATED_ENV, "1").status()`.
- `reexec_and_exit` (pub, generic `S: Spawner`) – `(spawner: &S) -> !` – resolves target, spawns, exits with propagated code; never returns.
- `reexec_target` (private) – `() -> io::Result<(PathBuf, Vec<OsString>)>` – `current_exe()` + `args_os().skip(1)`.
- `lock_is_stale` (private) – `(path: &Path, ttl: Duration, now: SystemTime) -> bool`.
- `open_lock_file` (private) – `(path: &Path) -> io::Result<File>` – create-if-absent, non-truncating. Name collides with `crates/ss-magic-plugin/src/tmproot.rs:221`'s own `open_lock_file` – near-identical shape (`OpenOptions::new().read(true).write(true).create(true)...open(path)`), flagged in functions.md as `dup-name`. **Candidate for a shared core helper – see Leads.**
- `touch` (private) – `(path: &Path)` – best-effort mtime bump.
- `try_lock_state` (pub, `#[allow(dead_code)]`) – `(lock_path: &Path) -> LockState`.
- `try_lock_state_at` (private) – `(lock_path, ttl, now) -> LockState` – testable core with injectable TTL/clock.
- `ApplyOutcome` (pub enum) – `Skipped | NoUpdate | Updated{version}`.
- `apply_update` (pub) – `(lock_path: &Path, target_tag: &str) -> ApplyOutcome` – lock (with stale-reclaim) → `run_self_update` → outcome.
- `apply_update_unlocked` (pub) – `(target_tag: &str) -> ApplyOutcome` – same swap, no lock (fallback when no cache dir resolves).
- `run_self_update` (private) – `(target_tag: &str) -> Result<Option<String>, Box<dyn Error>>` – configures and drives the `self_update` GitHub backend, mandatory tag pin.
- `split_slug` (private) – `(slug: &str) -> (&str, &str)` – splits `"owner/repo"` on the first `/`.

### `crates/ss-magic/src/workspace/mod.rs` (8 lines)
Module glue only: `pub(crate) mod migrate;` plus a re-export `pub(crate) use ss_magic_core::superset_files;` so `crate::workspace::superset_files::…` paths keep resolving post-split. No logic.

### `crates/ss-magic/src/workspace/migrate.rs` (671 lines)
Migration + init branching off `config.json`'s `setup` array, and the three materialize flows (interactive migrate, interactive init, non-interactive init).

- `MAGIC_WRAPPER_ENTRY` (pub const `&str`) – `"./.superset/magic.sh sync"` – the `setup` entry written by migration/init. Coupled to `magic.sh` being a pure passthrough (`exec ss-magic "$@"`); if that ever changed to inject `sync` itself, this constant would need to drop it in lockstep.
- `Branch` (pub enum) – `Migrate | Normal | Init`. Used by `tui/menu.rs`.
- `detect_branch` (pub) – `(config: Option<&Config>) -> Branch` – pure 3-way branch decision (Migrate wins over Normal on any `setup.sh` reference).
- `build_pattern_options` (pub) – `(existing_magic_files: &[String], fs_match: &[bool]) -> (Vec<String>, Vec<usize>)` – builds the picker's options + preselected indices for `run_init`.
- `run_migrate` (pub) – `(repo_root: &Path, existing: &Config) -> Result<ExitCode>` – interactive migrate: idempotency guard → summary print → stale-worktree advisory → prompt → stage → materialize → 7 status lines → `execute_final_action`.
- `run_init` (pub) – `(repo_root: &Path, existing: Option<&Config>) -> Result<ExitCode>` – interactive first-time/edit-config init: read existing → picker → stage → materialize → status lines → `execute_final_action`. Name collides with `crates/ss-magic-plugin/src/checklist/verbs.rs:503`'s private `run_init` – unrelated (checklist document init), not a real duplicate.
- `run_init_noninteractive` (pub) – `(repo_root: &Path, patterns: &[String]) -> Result<ExitCode>` – the TUI-free `ss-magic init` core: same stage/materialize sequence as `run_init` minus the picker/prompt, no git ops.
- `ensure_bootstrap_gitignores` (private) – `(repo_root: &Path) -> Result<()>` – the 3 idempotent gitignore rules (magic.local.json, backups/, .magic/) shared verbatim by all three flows above.
- `remove_legacy_skills_dir` (private) – `(home: &Path) -> Result<bool>` – removes `~/.claude/skills/ss-magic` if present (symlink/file vs dir).
- `clear_legacy_skills_install` (private, `#[cfg(not(test))]` **and** `#[cfg(test)]` – two bodies, both listed in functions.md as same-file dup) – production calls `remove_legacy_skills_dir` against the real home; test version is a no-op so tests never touch `~`. This is a documented, deliberate split (not a finding).
- `entry_is_setup_sh` (private) – `(entry: &str) -> bool` – `entry.contains(SETUP_SH_MARKER)`.
- `entry_is_magic_marker` (private) – `(entry: &str) -> bool` – `entry.contains("magic.sh") || entry.contains("ss-magic sync")`.
- `migrated_setup` (private) – `(old_setup: &[String]) -> Vec<String>` – replace-first/drop-rest `setup.sh` entries with `MAGIC_WRAPPER_ENTRY`.
- `migration_summary` (private) – `(existing: &Config) -> Vec<String>` – the 5-line on-screen "this will:" summary for migrate.
- `stage_migration` (private) – `(repo_root, stage_root, existing: &Config) -> Result<()>` – stages the migrated tree into a tempdir.
- `run_init_magic_files` – actually named `init_magic_files` (private) – `(chosen: &[String]) -> Vec<String>` – `default_magic_files()` + chosen, deduped.
- `rename_setup_config` (private) – `(repo_root: &Path) -> Result<()>` – deletes the old `setup_config.json` (not-found is not an error).
- `execute_final_action` (private) – `(repo_root, action: FinalAction, commit_message: &str, branch_prefix: &str) -> Result<ExitCode>` – the shared Done/CommitPushMain/FeatureBranchPR tail for both migrate and init.
- `SETUP_SH_MARKER`, `SETUP_SH_REL`, `MAGIC_LOCAL_REL`, `LEGACY_SKILLS_REL`, `MIGRATE_COMMIT_MESSAGE`, `INIT_COMMIT_MESSAGE` – private consts, see below.

## Constants and literals

| Name/literal | file:line | Value | Elsewhere in workspace? | Owning def? |
|---|---|---|---|---|
| `BIN_NAME` | `update/apply.rs:77` | `"ss-magic"` | `literals.md` "ss-magic" occurs 23× across 9 files incl. `status.rs:94` (`MANIFEST_NAME`), `heartbeat.rs:192`, `build-plugin-zip.py`. This one is pinned by a compile-time test to equal the crate's package name – legitimately file-local, not a stray literal. |
| `LOCK_FILE_NAME` | `update/apply.rs:80` | `"update.lock"` | `dup-name` in consts.md with `heartbeat.rs:67` (`"hooks.lock"`) and `ledger.rs:96` (`"cost.lock"`) – same *name*, three different values/files. No shared owner; each is a per-store lock file name. Not a bug, but worth noting the naming collision if a future reader greps for `LOCK_FILE_NAME`. |
| `UPDATED_ENV` | `update/apply.rs:84` | `"SS_MAGIC_UPDATED"` | Unique to this file. Read by `main.rs` indirectly via `apply::guard_active()`. |
| `NO_UPDATE_ENV` | `update/apply.rs:87` | `"SS_MAGIC_NO_UPDATE"` | Unique. |
| `STALE_TTL` | `update/apply.rs:93` | `Duration::from_secs(60)` | The literal `60` recurs at `apply/tests.rs:133,148` (re-deriving the same TTL for tests rather than importing the const – see Leads) and unrelated `60`s in `sync/reverse_sync.rs:710`, `tui/cockpit.rs:715/718`, `checklist/schema.rs:589/624`, `release_check.rs:566`, `scratchpad.rs:658` (all time-unit arithmetic, not TTLs – no dup with this one). |
| `USAGE` | `cli.rs:61` | banner text | `dup-name` with 6 other `USAGE` consts (`checklist/verbs.rs:220`, `compact_window.rs:134`, `expect_artifact.rs:96`, `main.rs:341` [plugin], `release_check.rs:71`, `setup_ci.rs:92`). All per-command usage strings, no shared owner – a naming pattern, not a value duplicate. |
| `MAGIC_WRAPPER_ENTRY` | `workspace/migrate.rs:57` | `"./.superset/magic.sh sync"` | `literals.md`: `./.superset/magic.sh sync` appears 9× total, 2 in non-test files (`migrate.rs:57` here, and the test-only `magic.sh`-embedding elsewhere). This IS the owning definition – everywhere else consumes it or duplicates it in test fixtures. |
| `SETUP_SH_MARKER` | `workspace/migrate.rs:47` | `"setup.sh"` | `literals.md`: `setup.sh` appears 10× (2 non-test), `migrate.rs:47` (this) and `migrate.rs` line ~60 (`SETUP_SH_REL`, which embeds it as `.superset/setup.sh`). Owning definition here. |
| `SETUP_SH_REL` | `workspace/migrate.rs:60` | `".superset/setup.sh"` | Same family – `literals.md` `.superset/setup.sh` 5 occurrences (1 non-test: this line; rest are tests). |
| `MAGIC_LOCAL_REL` | `workspace/migrate.rs:65` | `".superset/magic.local.json"` | `dup-name`-style value collision with core's `MAGIC_LOCAL_PATTERN` (`superset_files.rs:35`, same string `".superset/magic.local.json"`) and `superset_files.rs:31`'s `MAGIC_LOCAL_JSON` (`"magic.local.json"`, the bare filename). Three related constants for the same path fragment, in two crates. See Leads – const-location. |
| `LEGACY_SKILLS_REL` | `workspace/migrate.rs:103` | `".claude/skills/ss-magic"` | Unique to this file. |
| `MIGRATE_COMMIT_MESSAGE` | `workspace/migrate.rs:667` | `"chore(superset): migrate to ss-magic layout"` | Unique. |
| `INIT_COMMIT_MESSAGE` | `workspace/migrate.rs:668` | `"chore(superset): initialize ss-magic layout"` | Unique. |
| `"error: cannot resolve git repo root from {}: {err:#}"` | `main.rs:301` and `pack.rs:126` | identical format string | **Duplicate literal + duplicate control flow** – see Leads L1. |
| `"magic.json \`files\` is empty — nothing to sync."` / `"...to pack."` | `main.rs:343` / `pack.rs:145` | near-identical (word varies) | Same guard shape in `sync_core` and `pack_core`; both operate on the same `cfg.files` from the same `load_magic_or_exit`. See Leads L2. |
| `"creating {} staging tempdir"` context strings | `migrate.rs:355,509,558` | `"creating migration staging tempdir"` / `"creating init staging tempdir"` (×2, byte-identical the second and third time) | Local to this file; `run_init` and `run_init_noninteractive` share the exact same context string and `tempfile::Builder` shape. See Leads L3. |
| `"Wrote .superset/magic.json"` / `"Wrote .superset/magic.sh"` / `"Gitignored .superset/backups/"` / `"Gitignored .superset/.magic/"` | `migrate.rs:365-371, 531-540, 579-586` | byte-identical across all three flows | See Leads L4 – the whole post-materialize status block is copy-pasted three times. |
| `"ss-magic-migrate-"` / `"ss-magic-init-"` (tempdir prefixes) | `migrate.rs:353,507,556` | `run_init` and `run_init_noninteractive` use the identical `"ss-magic-init-"` prefix | Minor; not a bug (both are inits), but the prefix can't distinguish the two callers in a leftover-tempdir audit. |
| numeric `60` (`STALE_TTL`'s literal seconds) | `update/apply/tests.rs:133,148` | `Duration::from_secs(60)` | Tests re-write the literal instead of importing `STALE_TTL` – the module IS `#[cfg(test)]`-visible (`super::*` typically), so this is reasonably a reuse opportunity, not a real bug (see Leads L11). |

No `const`/`static` items are declared in `main.rs`, `cli.rs` (aside from `USAGE`), `pack.rs` (aside from the fn-local `TOOLS`), `update/mod.rs`, or `workspace/mod.rs` beyond what's listed above and in consts.md.

## Cross-module references

**Imports into this partition:**
- `main.rs` imports `ss_magic_core::{git, hashing}` (re-exported as `crate::git`/`crate::hashing`), plus `crate::{cli, pack, sync, tui, update, workspace}`.
- `cli.rs` imports nothing beyond `std`.
- `pack.rs` imports `crate::sync::apply`, `crate::git`, `crate::tui::style`, `ss_magic_core::reponame::repo_name_stem` (re-export), `crate::sync::under_excluded_tree` (called via `crate::sync::…`), plus `bzip2`, `tar`, `walkdir`, `tempfile`.
- `update/mod.rs` imports `ss_magic_core::release::{self, UpdateCheck, UreqReleaseClient, CLI_LINE}` and its own `apply` submodule.
- `update/apply.rs` imports `ss_magic_core::release::REPO_SLUG`, `self_update`, `fd_lock`.
- `workspace/migrate.rs` imports `crate::git`, `crate::git::gitignore`, `crate::sync::reverse_sync`, `crate::workspace::superset_files::{self, Config}` (core re-export), `crate::tui::{style, ui::{self, FinalAction}}`, `ss_magic_core::state_tree`.

**Public symbols and their external callers** (grepped workspace-wide, excluding this partition's own files and its `tests.rs`):

- `main::should_run_update_gate` – unused outside this file (only called from `run()` in the same file).
- `main::version_line` – unused outside this file (called only from `run()`; plugin crate has its own separate `version_line`).
- `main::menu_blocked_reason` – unused outside this file.
- `main::load_magic_or_exit` – called from `pack.rs:136` (`crate::load_magic_or_exit`) and `main.rs:334` itself.
- `main::run_pack_flow` – called from `tui/menu.rs:149,174`.
- `main::run_sync_flow`, `run_reverse_sync_flow`, `resolve_sync_roots`, `print_event` – unused outside `main.rs` (no external callers found).
- `cli::usage`, `cli::parse`, `Command`, `Parsed` – consumed only by `main.rs` (`cli::parse`, `cli::usage`, `Command`, `Parsed` all used at `main.rs:22,79,92,100`).
- `pack::archive_file_name`, `copy_to_clipboard`, `pack_core`, `PackEvent` – `archive_file_name` and `pack_core`'s behavior are referenced only in comments elsewhere (`reponame.rs:20`, `identity.rs:121`, `tui/menu.rs:52`) – no runtime caller outside `main.rs`'s `run_pack_flow`/`print_pack_event`.
- `pack::repo_name_stem` (re-export) – re-exported again, ultimately consumed by the plugin's `identity.rs` through the *original* `ss_magic_core::reponame::repo_name_stem`, not through this crate's re-export (the plugin crate cannot depend on `ss-magic`). So this pub(crate) re-export is CLI-internal only; the plugin gets the same function directly from core.
- `update::update_command`, `auto_update`, `UpdateReport` – called only from `main.rs` (`update_flow`, `run()`).
- `update::apply::{guard_active, propagate_code, reexec_and_exit, try_lock_state, ApplyOutcome, LockState, Spawner, ProcessSpawner, apply_update, apply_update_unlocked}` – all consumed only within `update/mod.rs` and this file's own tests; no external callers.
- `workspace::migrate::{Branch, detect_branch, run_migrate, run_init, run_init_noninteractive, build_pattern_options, MAGIC_WRAPPER_ENTRY}` – all called from `tui/menu.rs` (`detect_branch` at :166, `run_migrate` at :171, `run_init` at :172,212) except `run_init_noninteractive` (called only from `main.rs::init_noninteractive`) and `build_pattern_options`/`MAGIC_WRAPPER_ENTRY` (used only inside `migrate.rs` itself – "unused outside this file").

## Leads

| # | category | file:line(s) | Hypothesis | Compare against |
|---|---|---|---|---|
| L1 | duplication | `crates/ss-magic/src/main.rs:294-319` (`resolve_sync_roots`) vs `crates/ss-magic/src/pack.rs:120-132` | Both resolve `git::cwd_repo_root` and print the byte-identical error `"error: cannot resolve git repo root from {}: {err:#}"` on failure, but `pack.rs` re-implements it inline instead of calling `resolve_sync_roots` (or a shared "resolve repo root, print styled error" helper `pack.rs` could import). `pack_core` doesn't need `main_root` so it can't reuse the whole tuple-returning function, but the single-root half is a clean extraction candidate. | `crates/ss-magic/src/pack.rs:120-132` |
| L2 | duplication | `crates/ss-magic/src/main.rs:339-346` vs `crates/ss-magic/src/pack.rs:141-148` | `sync_core` and `pack_core` both guard `cfg.files.is_empty()` with a near-identical info line ("...nothing to sync."/"...nothing to pack.") right after calling the shared `load_magic_or_exit`. A single helper `fn empty_files_guard(cfg: &MagicConfig, verb: &str) -> Option<ExitCode>` (or similar) would collapse both. Low risk since behavior is identical. | `crates/ss-magic/src/main.rs:340-345` |
| L3 | copy-paste-variant | `crates/ss-magic/src/workspace/migrate.rs:352-355` (`run_migrate`), `:506-509` (`run_init`), `:555-558` (`run_init_noninteractive`) | All three build a `tempfile::Builder::new().prefix(...).tempdir().context("creating ... staging tempdir")?` staging dir with only the prefix string differing (`run_init` and `run_init_noninteractive` even share the exact same prefix `"ss-magic-init-"`). A shared `fn staging_tempdir(prefix: &str) -> Result<TempDir>` would remove the triplication. | Same file, all three call sites |
| L4 | duplication | `crates/ss-magic/src/workspace/migrate.rs:365-371` (`run_migrate`), `:531-540` (`run_init`), `:579-586` (`run_init_noninteractive`) | The post-materialize status-line block (`println!("{}", style::ok("Wrote .superset/magic.json"))`, `"Wrote .superset/magic.sh"`, `"Gitignored .superset/backups/"`, `"Gitignored .superset/.magic/"`, plus a varying middle line) is byte-identical in 4 of ~6-7 lines across all three functions. A shared `fn print_bootstrap_status(had_local: Option<bool>, extra: &[&str])`-shaped helper could de-duplicate the constant lines while leaving the varying ones (config.json vs setup array wording) explicit. | Same file, all three call sites |
| L5 | reuse / const-location | `crates/ss-magic/src/workspace/migrate.rs:65` (`MAGIC_LOCAL_REL = ".superset/magic.local.json"`) vs `crates/ss-magic-core/src/superset_files.rs:31` (`MAGIC_LOCAL_JSON = "magic.local.json"`) and `:35` (`MAGIC_LOCAL_PATTERN = ".superset/magic.local.json"`) | `migrate.rs`'s private `MAGIC_LOCAL_REL` duplicates core's `MAGIC_LOCAL_PATTERN` value verbatim in a different crate. Since core already owns the canonical path fragment and the CLI already depends on core (`ss_magic_core::state_tree`, `superset_files`), `migrate.rs` could import `superset_files::MAGIC_LOCAL_PATTERN` (if made `pub`) instead of re-declaring the literal. Cross-partition: core's `superset_files.rs` is outside this partition. | `crates/ss-magic-core/src/superset_files.rs:31,35` |
| L6 | dead-code / naming | `crates/ss-magic/src/main.rs:65` (`fn run()`) vs `crates/ss-magic-plugin/src/main.rs` `fn run` in `hook/mod.rs:358` and this crate's own `run()` | `run` is declared 3× workspace-wide with unrelated signatures (functions.md). Not a bug, but worth flagging that a reader grepping `fn run` gets noisy cross-crate hits – no action needed beyond noting it is NOT duplication. | n/a – informational, no fix needed |
| L7 | naming | `crates/ss-magic/src/cli.rs:85` (`pub fn usage() -> &'static str`) vs `crates/ss-magic-plugin/src/main.rs:379` (`pub fn usage() -> &'static str`) | Same signature, same purpose (render this binary's usage banner), in two crates that share no code path per the structure pins (binaries must not depend on each other). Not fixable by extraction without inventing a generic "usage renderer" abstraction that would be over-engineering for a single `&'static str` return – flagged for awareness only, likely a non-finding. | `crates/ss-magic-plugin/src/main.rs:379` |
| L8 | efficiency | `crates/ss-magic/src/pack.rs:222-224` (`is_pack_archive_rel`'s `root_level` check via `rel.parent().is_none_or(...)`) | Minor: computing `root_level` via `.parent().is_none_or(...)` allocates no memory but is a slightly indirect way to ask "does this path have zero components before the file name" – compare with `is_repo_root_rel` right below it (`:235-237`), which iterates `.components()` directly. Two different idioms for adjacent "how deep is this path" questions in the same file; not a bug, just an inconsistency worth a look during cleanup. | `crates/ss-magic/src/pack.rs:235-237` |
| L9 | const-location | `crates/ss-magic/src/update/apply.rs:80` (`LOCK_FILE_NAME = "update.lock"`) | `dup-name` with `crates/ss-magic-plugin/src/heartbeat.rs:67` (`"hooks.lock"`) and `crates/ss-magic-plugin/src/ledger.rs:96` (`"cost.lock"`). Different values so not a value-duplication bug, but three unrelated stores each invent their own `LOCK_FILE_NAME` constant name; a shared naming convention (e.g. suffix the store name in the const identifier) would make greps unambiguous. Low priority, naming only. | `crates/ss-magic-plugin/src/heartbeat.rs:67`, `crates/ss-magic-plugin/src/ledger.rs:96` |
| L10 | reuse | `crates/ss-magic/src/update/apply.rs:234-244` (`open_lock_file`) vs `crates/ss-magic-plugin/src/tmproot.rs:221` (`open_lock_file`) | Both privately implement "create the parent dir, then `OpenOptions::new().read(true).write(true).create(true)...open(path)`" for an `fd_lock`-style advisory lock file. Per structure-pins, a helper shared by both binaries belongs in core – this is a legitimate duplication to report (flagged explicitly in structure-pins.md's "Deliberate duplications" list as NOT pinned: "core cannot depend on the plugin, so the shared copy would live in core"). Verify the two bodies are actually byte-identical logic before proposing the extraction. | `crates/ss-magic-plugin/src/tmproot.rs:221` (partition: plugin) |
| L11 | hardcoded-value | `crates/ss-magic/src/update/apply/tests.rs:133,148` | Tests re-derive the literal `Duration::from_secs(60)` instead of referencing `super::STALE_TTL` (the module-private const at `apply.rs:93`), even though the test file is a sibling `#[cfg(test)] mod tests` with private-item access per the repo's test-layout convention. Using `STALE_TTL` directly would keep the test locked to the real constant instead of a copy that could silently drift. | `crates/ss-magic/src/update/apply.rs:93` |
| L12 | copy-paste-variant | `crates/ss-magic/src/main.rs:123-125` (`version_line`) vs `crates/ss-magic-plugin/src/main.rs:394-396` (`version_line`) | Both are `format!("<bin-name> {}", env!("CARGO_PKG_VERSION"))` with only the literal binary name differing (`"ss-magic"` vs `"ss-magic-plugin"`). Structurally identical one-liners in two crates that must not depend on each other (structure pin) – not extractable without a shared macro/helper in core, and CLAUDE.md's Architecture section documents each `--version`/`-V` contract as binary-specific (the plugin's line shape is load-bearing for `bootstrap.sh`). Likely a non-finding, listed for completeness. | `crates/ss-magic-plugin/src/main.rs:394-396` |
| L13 | naming | `crates/ss-magic/src/workspace/migrate.rs:472` doc comment names the function `run_init` while an unrelated private `run_init` exists in `crates/ss-magic-plugin/src/checklist/verbs.rs:503` (a checklist-document initializer, not a workspace initializer) | Purely a naming collision across crates (functions.md flags `run_init` as declared in 2 files). No shared logic; not a real duplication. Listed so the reviewer doesn't chase it further. | n/a – informational |
| L14 | efficiency | `crates/ss-magic/src/pack.rs:259-260` (`NamedTempFile::new_in(root)`) vs `crates/ss-magic/src/workspace/migrate.rs:352-355,506-509,555-558` (`tempfile::Builder::new().prefix(...).tempdir()`) | Both use the `tempfile` crate but through different APIs (`NamedTempFile::new_in` vs `Builder::tempdir`) for a similar "stage into a scratch location before atomic materialize" pattern. Not a bug – one needs a file, the others need a directory tree – but worth noting during a broader `tempfile`-usage pass if the sweep touches call-site consistency. | n/a – cross-check only |
