# ss-magic-core

The shared library (`ss_magic_core`, `publish = false`, `[package.metadata.dist] dist = false`, never tagged). Repository-wide rules live in the root [CLAUDE.md](../../CLAUDE.md).

Code rules: the root [CODING_STANDARDS.md](../../CODING_STANDARDS.md) – read it before writing or reviewing code here, and update it when a change establishes or retires a rule.

## Scope

Core owns what both binaries need, so the plugin can share the plumbing without linking the updater or a prompt library. It has no `inquire`, `ratatui`, `self_update` or `saphyr` (the YAML parser is CLI-only; Orca-specific code lives in the CLI's `orca/`, core holds only the generic plumbing it calls).

- The CLI re-exports `git` and `hashing` at its crate root, so `crate::git::…` resolves there. The plugin does the same, and reaches the rest by real paths (`ss_magic_core::style`, `ss_magic_core::reponame::repo_name_stem`).
- The CLI reaches `style` as `crate::tui::style` and `superset_files` as `crate::workspace::superset_files`; the plugin has no `tui/`.

## Modules

### git/mod.rs

Read-only probes: `is_worktree`, `main_checkout_root`, `cwd_repo_root`, `main_branch_name`, `origin_url` (backs pack's archive naming), `symbolic_ref_head` / `short_head_sha` (the plugin's `<repo>-<branch>` identity slug).

- `untracked_files` – `git ls-files --others`, untracked INCLUDING gitignored (reverse sync pushes gitignored secrets).
- `tracked_files` – `git ls-files --cached -z`, the mirror: POSITIVE tracked determination for the secret push gate. A path not in the set is treated as an untracked secret, so an unenumerable name fails closed. `parse_ls_files_z` is the shared NUL-split behind both and drops any absolute or `..`-bearing entry in one place.
- `is_ignored`; `is_ignored_str` takes the raw pathname so a caller can force git's directory-only match with a trailing slash; `is_ignored_no_index_str` is the `--no-index` variant that asks whether the IGNORE RULES cover a path regardless of the index (the plugin's state-tree gate needs it: a tracked file inside the tree must not read as "the tree is unignored").
- `ignored_untracked` – the BULK probe behind the Orca snapshot and shared-directory list. ONE `git check-ignore --stdin -z` process answers "ignored AND untracked" for a whole candidate list (`check-ignore` consults the index by default, so it never reports a tracked path). Details:
  - A directory is queried with a trailing slash.
  - Exit 1 is an empty answer; any other failure is an `Err` the caller must read as "nothing is positively ignored".
  - It builds its own `Command` because `git_raw` nulls stdin, and writes the list from a helper thread while draining stdout, so a list larger than a pipe buffer cannot deadlock.
  - Git aborts the whole batch on a pathspec inside a submodule or beneath a symlinked directory, so callers drop those candidates first.
  - Git reads every candidate as a PATHSPEC, so the probe drops any candidate that is empty or starts with `:` (`:(top)a.env` would be answered as `a.env` and falsely reported ignored; `:(icase)z` or `:!x` would abort the batch). Such a name is never reported ignored, and an emptied list returns without spawning git.
- `status_porcelain` – parsed `(status, path)` pairs behind the checklist commit nudge; built on `git_raw`, not the trimming `git` helper (rule: [CODING_STANDARDS.md](../../CODING_STANDARDS.md)).

Mutating primitives: `stage_paths`, `nothing_to_commit` (`git diff --cached --quiet`, so `workspace/migrate.rs`'s final-action arms skip an empty commit instead of failing on one), `commit`, `push`, `push_upstream`, `create_branch`, `pr_create`, `timestamp_branch_suffix`, `gh_available`.

Every `git`/`gh` invocation shells out through `git_raw`, which surfaces stderr verbatim; `git` and `git_optional` are thin one-liners on top. No git-binding crate (`git2`, `gix`).

### git/discover.rs

The ONE filesystem-only reduction of git behavior: the two roots the plugin's hook pipeline needs (`cwd_repo_root` and `main_checkout_root`), answered without spawning a process, and never anything more – no ref, index or write handling (R23, the requirement that bounds this module to the two roots).

- `discover(cwd)` returns `Found(Roots { worktree_root, common_dir, main_checkout_root })`, `NotARepository` or `Undecided(reason)`.
- `roots(cwd)` maps `Found` to its roots, `NotARepository` to two `None`s with no subprocess, and `Undecided` to exactly the two probe calls the pipeline would otherwise make – same order, same directories, including the `.git/hooks` shape where `--show-toplevel` fails but `--git-common-dir` still answers.
- **Invariant: a fast answer is byte-equal to git's, or there is none.** Anything the walk cannot prove DECLINES (`Undecided`), never a different `Found`.
- The walk (KTD8, the plan's filesystem-walk design) canonicalizes `cwd` (`canonicalize` owns `.`/`..`/symlinks; nothing lexical is reimplemented) and inspects each ancestor `D`:
  - a `.git` directory yields `{D, D/.git, D}`;
  - a gitfile (`gitdir: `, at most 4 KiB, resolved against `D`, canonicalized) needs a `commondir` beside its target (absent is the submodule shape and declines) and yields `{D, C, parent(C)}`;
  - no `.git` walks up, unless the parent is on another filesystem.
- Decline classes, each found by measuring git 2.55 against the plain walk:
  - any of six `GIT_*` variables (`DECLINING_ENV`: R21's five plus `GIT_OBJECT_DIRECTORY`);
  - a directory that itself looks like a git dir, or a symlinked `.git`;
  - `git_directory_shape` applies git's own `is_git_directory` test (`HEAD` holding a symref or object id – `head_content_is_valid` mirrors `validate_headref` – plus searchable `objects/` and `refs/`), because git WALKS UP past a `.git` that fails it;
  - a `.git` directory carrying a `commondir`, or a gitfile target that is not a git directory;
  - `repository_format_check` scans `<common>/config` (and `config.worktree`) conservatively for `core.worktree`, a `core.bare` other than `false`, an unsupported `repositoryformatversion` or unknown extension – honoring the common config's `core.*` for a linked worktree only under `extensions.worktreeConfig`, as git does – and declines on any line it cannot follow;
  - directories not owned by this euid (git's "dubious ownership").
- The equivalence matrix in `git/discover/tests.rs` runs every scenario against real `git` output and asserts `roots()` equals git's answer in ALL of them.
- Wired into the plugin crate only: the hook pipeline (where spawning nothing is the point) and the two read-only verbs that attribute ledger rows to a repository (`compact-window --recommend`, the `cost` ledger's `rows_for_repository`). Every CLI command keeps the subprocess probes (R22, the requirement that the CLI is not changed).

### git/gitignore.rs

`.gitignore` helpers at a git root.

- `ensure_path_ignored` is the single entry point shared by reverse sync (the secret-safety boundary), the backups dir and the migrate/init bootstrap. It ensures a `rel` of `PathKind::{File, Dir}` is ignored under a target root, adding a rule only when git does not already ignore it. The rule lands in the closest EXISTING `.gitignore` among the path's ancestors (else the target root), preferring a covering glob resolved from a rule-source root (verified) over an anchored literal. Returns `Ignored::{Already, Appended}`.
- It is git-TOLERANT: a git failure (e.g. a non-git test tempdir) reads as "not ignored" and writes the literal. A hard secret boundary therefore re-checks strictly on top of it (see `reverse_sync::ensure_gitignored_in_main`).
- `ensure_entry` (append a line iff no exact match exists, create the file if absent, never reorder) is the building block beneath it, called directly where the exact rule text is known.
- Private helpers: `find_covering_rule` resolves the covering rule via `git check-ignore -v` (negations excluded) into a typed `CoveringRule { pattern, source_dir }` – the source dir matters because a pattern only means something relative to the `.gitignore` it came from, so the caller can verify it covers the path before copying it into the target root; `parse_covering_line` parses it; `is_ignored_opt` (trailing-slash query for `Dir`), `closest_gitignore_dir`, `anchored_literal`.

### superset_files.rs

`.superset/{config.json, magic.sh, magic.json, magic.local.json}` I/O, plus the READ-ONLY legacy `setup_config.json` reader (`load_setup_config` / `SetupConfig`: migration carries its `files` into `magic.json`). `assets/magic.sh` is embedded here (`MAGIC_SH`, via `include_str!`) and is the canonical wrapper script: edit the asset and re-run init/migrate to propagate; `write_magic_sh` writes it.

- `load_config` reads Superset-owned `config.json`; `merge_setup_into_config` builds a new `Config` from a new `setup` array while preserving `teardown` and `run` from disk; `write_config_json` always rewrites pretty-printed.
- `load_overlaid` reads `magic.json` and overlays `magic.local.json`: `files` are union+deduped, base order first; `sharedDirectories` comes from the BASE file only (it is expanded into the committed `orca.yaml`, so the per-machine overlay must not reach it). `load_magic_json` / `load_magic_local_json` read each layer alone (the plugin's config write path load-modify-writes exactly one file).
- `MagicConfig::shared_directories` is `Option<Vec>`: `None` ("never chosen") stays absent on write, `Some(vec![])` ("chose none") is written as `[]`, and a file without the key round-trips byte-identically.
- `write_magic_json(root, &MagicConfig)` and `write_magic_local_json` take the whole typed config. `MagicConfig` carries a `#[serde(flatten)] extras` map, so an unknown key from a newer build or a hand edit survives a rewrite. `merge_files_into_magic_config(existing, files, shared_directories)` folds new `files` and `sharedDirectories` lists into an existing config; both REPLACE what was stored, never merge.
- Atomic writes: both writers commit through a staged sibling plus `rename`, never a truncating `fs::write`. `magic.json` is tracked and has an unattended writer (the plugin's `seed-config` runs every session start), so a write that dies half-way must leave the previous file, not a prefix.
  - `write_atomically` writes THROUGH a symlink onto its resolved target. It is public so the CLI's Orca writers replace `orca.yaml` and `.worktreeinclude` the same way.
  - `write_atomically_no_follow` is the mirror (same staged write, shared private `replace_entry`): it renames onto the path AS GIVEN, so a symlink at that name is replaced by a regular file and its target is never opened. It exists for the CLI's automatic Orca refresh, an unattended writer that may only change the one name git answered for.
  - Both create the staging file through `create_staging`, which opens with `create_new` (`O_EXCL`). A plain `fs::write` would follow a symlink planted at the staging name and write into its target, so an existing entry there is unlinked – the link itself, never its target – and created afresh.
- `bootstrap_magic_local_json` and `default_magic_files` round out the init/migration writers; `existing_unknown_entries` preserves user-typed patterns across re-runs.
- `copy_into_repo` materializes the staged `.superset/` tree atomically: files always overwritten (preservation happens upstream of the write), `*.sh` chmod 0755, a `delete` set strips the retired `setup.sh`, and `config.json` is written LAST (it is the file Superset reads to locate the wrapper).

### sync/mod.rs

The pure root of the sync engine and the home of the ONE excluded-trees rule. The CLI's own `sync/mod.rs` declares `reverse_sync` and `merge` and re-exports the rest from here.

- `EXCLUDED_TREES` lists four whole directory trees no walk may ever yield, each as its exact sequence of path components:
  - `.superset/backups` – the tool's own copies of overwritten bytes (recovered secrets);
  - `.superset/.magic` – the plugin's gitignored, machine-local state;
  - `.scratchpad` – a tree ss-magic does not own but must never push into the shared main checkout;
  - `.git`.
- `under_excluded_tree(rel)` answers "is this rel one of them, or inside one" via the component-by-component `starts_with_components` – NEVER a string prefix or a bare name. A sibling `.superset/.magicked/` stays includable, a root-level `.magic` file is untouched, and `.superset` ITSELF is never excluded (widening the rule would drop the contract files `config.json`, `magic.sh` and `magic.json` out of sync and pack entirely). It returns `true` for a tree root, which is what lets a `WalkDir::filter_entry` caller prune the whole subtree.
- It is applied at every point of FINAL enumeration, never only on an upstream match list: `apply::walk_source`; `apply::copy_dir_recursive(root, src, dst)` (takes the tree root so it can classify each entry's rel); reverse sync's candidate computation; pack's `append_dir_excluding_trees`; the CLI's two Orca walks (snapshot directory expansion, shared-directory walk – the skip set has no walk, it only checks that each listed path exists). Rule and the ancestor-match trap: [CODING_STANDARDS.md](../../CODING_STANDARDS.md).
- Distinct from `apply::DEFAULT_EXCLUDES`, which drops a match containing a directory NAME (`node_modules`, `.venv`) at ANY depth.
- A core test pins `state_tree::STATE_REL` equal to the `.superset/.magic` entry.

### sync/repo_scan.rs

`matches_for_patterns(root, &[&str])` walks the working tree once with a multi-pattern `GlobSet` and returns a bool vector aligned to the input. `pattern_matches_any` is the single-pattern shortcut used when the user adds a custom pattern in the bootstrap picker.

### sync/pattern.rs

Shared syntax checks for the apply/sync expansion path and the picker validator: `has_glob_meta`, `has_parent_segment`, `SyntaxError`, `check_syntax`. The one source of truth for "is this pattern structurally valid".

### sync/apply.rs

The glob/exclude/copy engine behind forward `sync` (and, via `match_paths`, reverse sync). Delegates syntax checks to `pattern::check_syntax`. Emits an `Event` stream through a caller-supplied closure, so tests collect events while production prints them.

- `run_filtered` is the same engine with a `PathFilter`: repo-relative paths the copy must leave alone. An entry covers itself and everything beneath it by whole path component; an empty path is dropped at construction (it would cover the whole tree). `run` and `copy_dir_recursive` delegate with the empty filter.
- The filter is applied at BOTH enumeration points – each expanded match and inside the recursive directory copy (`copy_dir_recursive_filtered`) – so a directory match above a filtered path still skips it.
- A filtered path emits `SkipReason::Filtered`, which never counts and is tallied in `Summary::filtered` apart from `skipped`, so it cannot turn the summary into a warning. A directory match whose every file was filtered is not reported as copied (the recursive copy returns how many files it copied), and the backup pass likewise leaves no empty backup directory for it.
- `DEFAULT_EXCLUDES` is public so the Orca snapshot prunes the same names in its own walk.

### style.rs

Palette (gray info, bold green ok, bold orange/xterm 208 warn, bold red err, bold cyan header). One `OnceLock<bool>` captures the color decision (NO_COLOR + supports-color): `init()` decides from the terminal, `init_no_color()` forces it off, `enabled()` reads it. Knows nothing about `inquire`; the CLI's `tui/theme.rs` installs the prompt theme from `enabled()`.

### hashing.rs

Content-fingerprint primitives.

- `fnv1a_64` / `hash_file` – non-cryptographic hashes behind cache keys and claim-file names. FNV-1a rather than `DefaultHasher` because std does NOT promise its output is stable across releases or processes, and a long-lived cache keyed on an unstable hash silently rots.
- `sha256` / `sha256_hex` – hand-rolled FIPS 180-4 SHA-256 (pinned by literal test vectors), present for ONE reason: the plugin's per-machine temp-root identifier must be derived identically by this Rust code and by the shell bootstrap's `shasum -a 256`, so the algorithm must be one every platform already implements the same way.

### reponame.rs

`repo_name_stem` (with `stem_from_origin` and `sanitize_segment`): the repo name derived from the normalized `origin`, shared by the CLI's `pack.rs` (re-exported) and the plugin's identity slug so the two cannot disagree about what the repo is called.

### state_tree.rs

`STATE_REL` (`.superset/.magic`) and `ensure_state_ignored`: the ONE owner of the state tree's gitignore rule, called eagerly by init/migrate and lazily by the plugin's `enable` / `config set`; no hook calls it.

### release.rs

The daily-cached, PER-RELEASE-LINE GitHub release check (ureq, ETag, 5 s timeout, silent fall-through), imported by the CLI's `update/` as `ss_magic_core::release` and used by the plugin's `release-check` verb and `SessionStart` suggestion.

- One repository hosts two lines – the CLI on bare `vX.Y.Z` tags, the plugin on `ss-magic-plugin-vX.Y.Z` – so the repository-wide `releases/latest` mark does not identify the newest CLI release. The check fetches the first page of `/releases?per_page=100`, drops drafts and prereleases, and keeps only tags passing its `Line`'s anchored filter. `parse_line_tag` is `tag.strip_prefix(line.tag_prefix)` then exactly three ASCII-digit components and nothing else; `CLI_LINE` is `v`, `PLUGIN_LINE` is `ss-magic-plugin-v` (consumed by the plugin crate alone).
- `select_newest` takes the GREATEST triple, never the first entry (the list is in creation order). A line with no release among the newest 100 reads as "no update" – conservative by design.
- Cache: `Cache { checked_at, tag_name, etag, suggested }`. Fields are additive so an older binary's cache file still parses; `suggested: Option<String>` is skipped when `None`, keeping the CLI's `version-check.json` byte-identical. `tag_name` holds the SELECTED tag, and a cached tag of the other line reads as `UpToDate`. `suggested` is the plugin line's once-per-release marker: the tag `SessionStart` already announced.
- `refresh_cache(prior, client, line, now) -> Refresh { cache, outcome }` is the ONE derivation of the next record from the prior one plus a fetch. It always fetches with the prior ETag, bumps `checked_at` on every outcome, keeps the prior tag on `NotModified`/`Failed`, and carries `suggested` forward whenever the selected tag equals the prior tag (cleared only for a DIFFERENT tag), so a daily refresh cannot re-arm a notice already shown.
- `run_check` calls it once the cache is stale; the plugin's `release-check --refresh` calls it on every invocation (no freshness short-circuit in the verb).
- `read_cache` and `now_secs` are public for the hook, which reads the plugin cache directly and must never construct a client. `existing_cache_dir` is the non-creating lookup (status promises to create nothing; `cache_dir` scaffolds the directory).
- `UreqReleaseClient::for_product(product, version)` sets the user agent (`new` is the CLI's `ss-magic/<version>` shorthand). `resolve_newest_uncached` is the cache-free resolver behind `ss-magic update`.

### testutil.rs

Shared test helpers, compiled only under `cfg(test)` or the `testutil` feature (see Tests).

## Glob semantics

Owned by `sync/apply.rs` and `sync/pattern.rs`:

- Absolute paths and `..` segments are rejected.
- Literals must exist.
- A glob matching nothing is non-fatal.
- `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at any depth.

## Tests

- Each module declares `#[cfg(test)] mod tests;` with the body in a sibling child file (`<module>/tests.rs`), keeping private-item access. Same layout in all three crates.
- `ss_magic_core::testutil` (`src/testutil.rs`) is compiled under `cfg(test)` for core's own suite and behind core's `testutil` feature for a binary's; a binary enables the feature from its `[dev-dependencies]` ONLY, so no release build contains it. Every helper is `pub`: `git_run`, `neutralize_global_excludes`, `exit_code_to_u8`, `init_main_repo`, `write_magic`, `write_file`, `make_worktree`, `run_ignored_test_in_child`, `run_ignored_test_in_child_from`, `test_path_in_binary`.
- Tests use `tempfile` plus shell-invoked `git init` / `git worktree add`.
- **Process-environment tests run in a child process.** A test whose subject is `PATH`, a `GIT_*` variable or the process's own working directory runs its assertion via `testutil::run_ignored_test_in_child` (`…_from(cwd, …)` for a cwd): an `#[ignore]`d child test, named through `test_path_in_binary`, spawned from `current_exe()` with the variable set in the child only. WHY: the suite runs multithreaded, and a `set_var` is inherited by every `git` any other thread spawns during the window; a mutex around the setter does not prevent that.
- `cargo test` starts a test binary in its PACKAGE root (`crates/<name>`), not the repository root. A fixture needing a repository-level directory such as `docs/` in the cwd builds a scratch tree and uses the `_from` variant.
- `set_var` under an `ENV_LOCK` is acceptable only for a variable no concurrent test's child could misread (`HOME` in the checklist-deny tests). `ENV_LOCK` is a private static per test file (plugin `hook/pre_tool_use/tests.rs`, CLI `update/apply/tests.rs`), not a shared helper.
- Injected lookups are the cheaper half of the same rule: where code reads the environment through an injected closure (`git::discover::discover_with_env`, and in the CLI `sync_core` and `orca::detect`), tests pass a closure and never touch the process environment.
