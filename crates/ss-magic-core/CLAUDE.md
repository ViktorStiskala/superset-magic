# ss-magic-core

The shared library (`ss_magic_core`, `publish = false`, `[package.metadata.dist] dist = false`, never tagged). Repository-wide rules live in the root [CLAUDE.md](../../CLAUDE.md).

Code rules: the root [CODING_STANDARDS.md](../../CODING_STANDARDS.md) – read it before writing or reviewing code here, and update it when a change establishes or retires a rule.

## Scope

Core owns what both binaries need, so the plugin shares the plumbing without linking the updater or a prompt library. No `inquire`, `ratatui`, `self_update` or `saphyr` (the YAML parser is CLI-only; Orca code lives in the CLI's `orca/`, core holds only the generic plumbing it calls).

- Both binaries re-export `git` and `hashing` at their crate root, so `crate::git::…` resolves in each. The plugin reaches the rest by real paths (`ss_magic_core::style`, `ss_magic_core::reponame::repo_name_stem`); it has no `tui/`.
- The CLI reaches `style` as `crate::tui::style` and `superset_files` as `crate::workspace::superset_files`.

## Modules

### git/mod.rs

Every `git`/`gh` invocation, through `git_raw` (stderr verbatim) and its thin `git` / `git_optional` wrappers – rules in [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (Git plumbing).

Read-only probes: `is_worktree`, `main_checkout_root`, `cwd_repo_root`, `main_branch_name`, `origin_url` (pack's archive naming), `symbolic_ref_head` / `short_head_sha` (the plugin's `<repo>-<branch>` slug).

- `untracked_files` – `git ls-files --others`, INCLUDING gitignored (reverse sync pushes gitignored secrets).
- `tracked_files` – `git ls-files --cached -z`, the POSITIVE tracked set behind the secret push gate. `parse_ls_files_z` is the NUL-split behind both, dropping any absolute or `..`-bearing entry.
- `is_ignored`; `is_ignored_str` (raw pathname, so a trailing slash forces a directory-only match); `is_ignored_no_index_str` (`--no-index`: do the IGNORE RULES cover the path, regardless of the index – the plugin's state-tree gate, where a tracked file inside the tree must not read as "unignored").
- `ignored_untracked` – the BULK probe behind the Orca snapshot and shared-directory list: ONE `git check-ignore --stdin -z` answers "ignored AND untracked" for a whole list. Directories are queried with a trailing slash; exit 1 is an empty answer, any other failure an `Err` read as "nothing ignored". Builds its own `Command` (`git_raw` nulls stdin). Drops empty or `:`-leading candidates itself (an emptied list spawns no git); callers drop submodule and beneath-symlinked-directory candidates.
- `status_porcelain` – `(status, path)` pairs behind the checklist commit nudge, on `git_raw`.

Mutating: `stage_paths`, `nothing_to_commit` (`git diff --cached --quiet`, so `workspace/migrate.rs`'s final-action arms skip an empty commit), `commit`, `push`, `push_upstream`, `create_branch`, `pr_create`, `timestamp_branch_suffix`, `gh_available`.

### git/discover.rs

The filesystem-only reduction of the two roots the plugin's hook pipeline needs (`cwd_repo_root`, `main_checkout_root`), spawning nothing; R23 bounds it to those two roots. Invariant and scope rule: [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (Git plumbing).

- `discover(cwd)` → `Found(Roots { worktree_root, common_dir, main_checkout_root })`, `NotARepository` or `Undecided(reason)`.
- `roots(cwd)`: `Found` → its roots; `NotARepository` → two `None`s, no subprocess; `Undecided` → exactly the two probes the pipeline would otherwise run, same order and directories, including the `.git/hooks` shape where `--show-toplevel` fails but `--git-common-dir` answers.
- The walk (KTD8, the plan's walk design) canonicalizes `cwd` (nothing lexical reimplemented) and inspects each ancestor `D`:
  - a `.git` directory yields `{D, D/.git, D}`;
  - a gitfile (`gitdir: `, at most 4 KiB, resolved against `D`, canonicalized) needs a `commondir` beside its target (absent is the submodule shape: decline) and yields `{D, C, parent(C)}`;
  - no `.git`: walk up, unless the parent is on another filesystem.
- Declines, each found by measuring git 2.55 against the plain walk:
  - any `DECLINING_ENV` variable (R21's five `GIT_*` plus `GIT_OBJECT_DIRECTORY`);
  - a directory that itself looks like a git dir; a symlinked `.git`;
  - a `.git` failing `git_directory_shape` (git's `is_git_directory`: `HEAD` a symref or object id via `head_content_is_valid`, mirroring `validate_headref`, plus searchable `objects/` and `refs/`) – git WALKS UP past one;
  - a `.git` directory with a `commondir`; a gitfile target that is not a git directory;
  - `repository_format_check` on `<common>/config` and `config.worktree`: `core.worktree`, `core.bare` other than `false`, unsupported `repositoryformatversion` or unknown extension, any unparsable line (common `core.*` counts for a linked worktree only under `extensions.worktreeConfig`, as in git);
  - directories not owned by this euid (git's "dubious ownership").
- `git/discover/tests.rs`'s equivalence matrix asserts `roots()` equals real `git` in EVERY scenario.
- Callers: the hook pipeline and the two ledger-attribution verbs (`compact-window --recommend`, the `cost` ledger's `rows_for_repository`). The CLI keeps the subprocess probes (R22).

### git/gitignore.rs

- `ensure_path_ignored` – the one entry point for reverse sync (the secret-safety boundary), the backups dir and the init/migrate bootstrap. Ensures a `rel` of `PathKind::{File, Dir}` is ignored under a target root, adding a rule only when git does not already ignore it, in the closest EXISTING ancestor `.gitignore` (else the target root); a verified covering glob is preferred over an anchored literal. Returns `Ignored::{Already, Appended}`.
- Git-TOLERANT: a git failure (e.g. a non-git test tempdir) reads as "not ignored" and writes the literal, so a hard secret boundary re-checks strictly on top (`reverse_sync::ensure_gitignored_in_main`).
- `ensure_entry` – append a line iff no exact match, create the file if absent, never reorder; used directly where the rule text is known.
- Private: `find_covering_rule` (`git check-ignore -v`, negations excluded) → `CoveringRule { pattern, source_dir }`, the source dir letting the caller verify coverage before copying the rule; `parse_covering_line`, `is_ignored_opt` (trailing-slash `Dir` query), `closest_gitignore_dir`, `anchored_literal`.

### superset_files.rs

`.superset/{config.json, magic.sh, magic.json, magic.local.json}` I/O, plus the READ-ONLY legacy `setup_config.json` reader `load_setup_config` / `SetupConfig` (migration carries its `files` into `magic.json`). `MAGIC_SH` embeds the canonical `assets/magic.sh` (`include_str!`; edit it and re-run init/migrate); `write_magic_sh` writes it.

- `load_config` reads Superset-owned `config.json`; `merge_setup_into_config` builds a `Config` from a new `setup` array, keeping `teardown` and `run`; `write_config_json` always pretty-prints.
- `load_overlaid` overlays `magic.local.json` on `magic.json`: `files` union+deduped, base order first; `sharedDirectories` from the BASE only (it lands in the committed `orca.yaml`). `load_magic_json` / `load_magic_local_json` read one layer (the plugin's writer modifies exactly one file).
- `MagicConfig::shared_directories` (`Option<Vec>`): `None` stays absent, `Some(vec![])` writes `[]`, a file without the key round-trips byte-identically.
- `write_magic_json(root, &MagicConfig)` / `write_magic_local_json` take the whole config (`#[serde(flatten)] extras` keep unknown keys). `merge_files_into_magic_config(existing, files, shared_directories)` REPLACES both lists.
- Atomic writers (`magic.json` has an unattended writer, the plugin's `seed-config` at every session start); staging via `create_staging` – rule in [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (Filesystem writes):
  - `write_atomically` writes THROUGH a symlink; public for the CLI's Orca writers (`orca.yaml`, `.worktreeinclude`).
  - `write_atomically_no_follow` (shared private `replace_entry`) renames onto the path AS GIVEN, replacing a symlink instead of opening its target – for the automatic Orca refresh, which may change only the one name git answered for.
- `bootstrap_magic_local_json`, `default_magic_files` (init/migration); `existing_unknown_entries` keeps user-typed patterns.
- `copy_into_repo` materializes the staged `.superset/` tree: files always overwritten (preservation is upstream), `*.sh` 0755, a `delete` set strips the retired `setup.sh`, `config.json` LAST (Superset reads it to locate the wrapper).

### sync/mod.rs

The sync engine's pure root (the CLI's `sync/mod.rs` adds `reverse_sync` and `merge` and re-exports the rest). Exclusion rule: [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (Secret safety).

- `EXCLUDED_TREES`, as exact component sequences: `.superset/backups` (recovered secrets), `.superset/.magic` (plugin state), `.scratchpad` (not ours, never pushed to main), `.git`.
- `under_excluded_tree(rel)` via component-wise `starts_with_components`: `.superset/.magicked/` stays includable, a root-level `.magic` file is untouched; `true` for a tree root lets `WalkDir::filter_entry` prune the subtree.
- Applied in `apply::walk_source`, `apply::copy_dir_recursive(root, src, dst)` (the root classifies each rel), reverse sync's candidates, pack's `append_dir_excluding_trees`, and the CLI's Orca snapshot and shared-directory walks (the skip set has no walk).
- Distinct from `apply::DEFAULT_EXCLUDES` (`node_modules`, `.venv` as a NAME at any depth).
- A core test pins `state_tree::STATE_REL` to the `.superset/.magic` entry.

### sync/repo_scan.rs

`matches_for_patterns(root, &[&str])` walks once with a multi-pattern `GlobSet`, returning a bool vector aligned to the input; `pattern_matches_any` is the single-pattern shortcut for the bootstrap picker.

### sync/pattern.rs

The one structural-validity check, for expansion and the picker: `has_glob_meta`, `has_parent_segment`, `SyntaxError`, `check_syntax`.

### sync/apply.rs

The glob/exclude/copy engine behind forward `sync` (and reverse sync via `match_paths`), emitting an `Event` stream through a closure; syntax via `pattern::check_syntax`. Glob semantics: [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (one glob and exclude engine).

- `run_filtered` takes a `PathFilter`: an entry covers itself and everything beneath by whole component; an empty path is dropped (it would cover the tree). `run` and `copy_dir_recursive` use the empty filter.
- Applied to each match and inside `copy_dir_recursive_filtered`, so a directory match above a filtered path still skips it.
- `SkipReason::Filtered` is tallied in `Summary::filtered`, apart from `skipped`. A directory match with every file filtered is not reported copied (the copy returns its file count), and leaves no empty backup directory.
- Match semantics: a literal pattern must exist, while a glob matching nothing is non-fatal. Absolute and `..` patterns are rejected.
- `DEFAULT_EXCLUDES` is public so the Orca snapshot prunes the same names.

### style.rs

Palette: gray info, bold green ok, bold orange/xterm 208 warn, bold red err, bold cyan header. A `OnceLock<bool>` holds the color decision (NO_COLOR + supports-color): `init()`, `init_no_color()`, `enabled()`. No `inquire`; the CLI's `tui/theme.rs` installs the prompt theme from `enabled()`.

### hashing.rs

- `fnv1a_64` / `hash_file` – cache keys and claim-file names. Not `DefaultHasher`: std does not promise stable output across releases or processes, so a long-lived cache keyed on it silently rots.
- `sha256` / `sha256_hex` – hand-rolled FIPS 180-4 (pinned by test vectors), only because the plugin's temp-root identifier must match the shell bootstrap's `shasum -a 256` byte for byte.

### reponame.rs

`repo_name_stem` (with `stem_from_origin`, `sanitize_segment`), from the normalized `origin`: shared by the CLI's `pack.rs` (re-exported) and the plugin's identity slug so the two cannot disagree.

### state_tree.rs

`STATE_REL` (`.superset/.magic`) and `ensure_state_ignored`, the one writer of that gitignore rule: eagerly from init/migrate, lazily from the plugin's `enable` / `config set`, never from a hook.

### release.rs

The daily-cached, PER-RELEASE-LINE GitHub release check (ureq, ETag, 5 s timeout, silent fall-through), used as `ss_magic_core::release` by the CLI's `update/` and the plugin's `release-check` verb and `SessionStart` suggestion. Tag-filter rule: [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (self-updater).

- Two lines share the repository (CLI on bare `vX.Y.Z`, plugin on `ss-magic-plugin-vX.Y.Z`), so `releases/latest` cannot name the newest CLI release. The check fetches `/releases?per_page=100` (first page), drops drafts and prereleases, and keeps tags passing the `Line`'s `parse_line_tag` (`tag.strip_prefix(line.tag_prefix)`, then exactly three ASCII-digit components). `CLI_LINE` is `v`; `PLUGIN_LINE` is `ss-magic-plugin-v` (plugin only).
- `select_newest` takes the GREATEST triple (the list is in creation order); no release among the newest 100 reads as "no update".
- `Cache { checked_at, tag_name, etag, suggested }` is additive, so an older binary's file parses. `tag_name` is the SELECTED tag; one of the other line reads as `UpToDate`. `suggested: Option<String>` (skipped when `None`, so the CLI's `version-check.json` is unchanged) is the plugin's once-per-release marker, the tag `SessionStart` announced.
- `refresh_cache(prior, client, line, now) -> Refresh { cache, outcome }` – the ONE next-record derivation: fetches with the prior ETag, bumps `checked_at` always, keeps the prior tag on `NotModified`/`Failed`, and keeps `suggested` unless the selected tag changed, so a refresh cannot re-arm a shown notice. `run_check` calls it once stale; `release-check --refresh` every time.
- `read_cache`, `now_secs` – public for the hook, which never constructs a client. `existing_cache_dir` is the non-creating lookup (`cache_dir` scaffolds).
- `UreqReleaseClient::for_product(product, version)` sets the user agent (`new` = `ss-magic/<version>`). `resolve_newest_uncached` backs `ss-magic update`.

### testutil.rs

Shared test helpers, under `cfg(test)` or the `testutil` feature (see Tests).

## Tests

Layout, child-process and injected-lookup rules: [CODING_STANDARDS.md](../../CODING_STANDARDS.md) (Tests). Core-specific:

- `ss_magic_core::testutil` (`src/testutil.rs`), `testutil` feature from a binary's `[dev-dependencies]` only. All `pub`: `git_run`, `neutralize_global_excludes`, `exit_code_to_u8`, `init_main_repo`, `write_magic`, `write_file`, `make_worktree`, `run_ignored_test_in_child`, `run_ignored_test_in_child_from`, `test_path_in_binary`.
- Fixtures: `tempfile` plus shell-invoked `git init` / `git worktree add`.
- A child test is an `#[ignore]`d test named via `test_path_in_binary`, spawned from `current_exe()` with the variable set in the child only; the `_from(cwd, …)` variant sets a cwd.
- `cargo test` runs a test binary in its PACKAGE root (`crates/<name>`), so a fixture needing a repository-level `docs/` builds a scratch tree and uses `_from`.
- `ENV_LOCK` is a private static per test file (plugin `hook/pre_tool_use/tests.rs`, CLI `update/apply/tests.rs`).
- Injected environment lookups: `git::discover::discover_with_env`; in the CLI, `sync_core` and `orca::detect`.
