---
paths:
  - "crates/ss-magic-core/**"
---

## Core modules (`crates/ss-magic-core/src/`)

- `lib.rs` – the crate root: declares the modules this map lists and nothing else
  (`testutil` only under `cfg(test)` or the `testutil` feature). Its module
  doc states the crate's boundary: nothing in core opens a terminal UI, spawns
  an updater or parses argv, which is what keeps the plugin binary unable to
  self-update or open a TUI by construction.
- `git/mod.rs` – the one place production code spawns a `git` or `gh`
  process, in any crate of the workspace, always through
  `std::process::Command` (test code is the exception: `testutil::git_run` and
  several test modules run `git` directly to build fixtures). Most git calls go
  through the private `git_raw`, which runs `git` with stdin closed and returns
  the raw `Output` without interpreting the exit. Two thin wrappers sit on it:
  `git` (trimmed stdout; a non-zero exit becomes an error carrying git's
  verbatim stderr) and `git_optional` (a non-zero exit becomes `None`). A
  caller that must read the exit code or the untrimmed output calls `git_raw`
  directly: `status_porcelain`, the shared `check_ignore` behind the ignore
  probes, and `gitignore.rs`'s `find_covering_rule`. Four functions spawn
  their own `Command` instead: `nothing_to_commit` (it needs only the exit
  status of `git diff --cached --quiet`, so every stream is discarded),
  `gh_available` and `pr_create` (they run `gh`), and `timestamp_branch_suffix`
  (it runs `date +%Y%m%d-%H%M%S`, neither git nor gh).
  Read-only probes: `cwd_repo_root`, `is_worktree`, `main_checkout_root`,
  `main_branch_name`, `origin_url` (the input to `reponame.rs`'s repo-name
  stem, and to the plugin checklist's `browsable_origin` repository link in
  the render's metadata block), plus `untracked_files`
  (`git ls-files --others` – untracked *including* gitignored, since reverse
  sync pushes gitignored secrets),
  `tracked_files` (`git ls-files --cached -z` – the mirror of `untracked_files`
  that does POSITIVE tracked determination for the secret push gate: a path
  NOT in this set is treated as an untracked secret, so an unenumerable name
  fails closed), `is_ignored` (reverse sync's strict re-check),
  `is_ignored_str` (the raw-pathname variant so a caller can force git's
  directory-only match with a trailing slash), `is_ignored_no_index_str` (the
  `--no-index` variant that asks whether the IGNORE RULES cover a path,
  ignoring the index – what the plugin's state-tree gate needs, since a
  tracked file inside the tree must not read as "the tree is unignored"; the
  ignore probes map exit 0 to ignored, exit 1 to not ignored, and anything
  else to an error carrying git's message), `status_porcelain` (parsed
  `(status, path)` pairs behind the plugin's checklist commit nudge – built on
  `git_raw`, NEVER the trimming `git` helper, because porcelain's leading
  column is a literal space for a worktree-only modification and a blanket
  `.trim()` shifts every field; an empty pathspec list returns nothing rather
  than the whole repository), and `symbolic_ref_head` / `short_head_sha` (the
  branch name and abbreviated SHA behind the plugin's `<repo>-<branch>`
  identity slug). `parse_ls_files_z` is the shared NUL-split behind BOTH
  `untracked_files` and `tracked_files`, defensively dropping any absolute or
  `..`-bearing entry in one place.
  Mutating primitives, used by the CLI's `workspace/migrate.rs` final-action
  step: `stage_paths` (skips a path missing on disk), `nothing_to_commit`,
  `commit`, `push`, `push_upstream`, `create_branch` (`git switch -c`),
  `pr_create` (`gh pr create --fill --base`), `gh_available` and
  `timestamp_branch_suffix`. `nothing_to_commit` is what lets migrate's
  final-action arms skip an empty commit instead of failing on one. The CLI's
  location routing is its bare menu, which asks `is_worktree` and
  `main_checkout_root`; there is no location-auto dispatch.
- `git/discover.rs` – the ONE filesystem-only reduction of git behavior in the
  workspace: the two roots the plugin's hook pipeline needs (`cwd_repo_root`
  and `main_checkout_root`) answered without spawning a process, and by
  convention never anything more – no ref, index, or write handling (R23, the
  plan requirement that fences this module to those two questions).
  `discover(cwd)` (or `discover_with_env`, with the environment lookup
  injected for tests) returns `Found(Roots { worktree_root, common_dir,
  main_checkout_root })`, `NotARepository`, or `Undecided(reason)`;
  `roots(cwd)` maps `Found` to its roots, `NotARepository` to two `None`s with
  no subprocess, and `Undecided` to the two subprocess probes – `cwd_repo_root`
  on `cwd`, then `main_checkout_root` on the worktree root it found (or on
  `cwd` when it found none), including the `.git/hooks` shape where
  `--show-toplevel` fails but `--git-common-dir` still answers. The invariant
  that makes it safe: **a fast answer is byte-equal to git's, or there is
  none.** The walk (KTD8, the plan's design for an ancestor walk over `.git`
  entries) declines on any of the six `GIT_*` variables in `DECLINING_ENV`
  (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_CEILING_DIRECTORIES`,
  `GIT_DISCOVERY_ACROSS_FILESYSTEM` and `GIT_OBJECT_DIRECTORY`, each of which
  changes how git resolves the repository), canonicalizes `cwd`
  (`canonicalize` owns `.`/`..`/symlinks – nothing lexical is reimplemented),
  and inspects each ancestor `D`: a directory that itself looks like a git dir
  declines; a symlinked `.git` declines; a `.git` directory yields
  `{D, D/.git, D}`; a gitfile (`gitdir: `, at most `GITFILE_MAX_BYTES` = 4 KiB,
  resolved against `D`, canonicalized) needs a `commondir` beside its target –
  absent is the submodule shape and declines – and yields
  `{D, C, parent(C)}`; an absent `.git` walks up unless the parent is on
  another filesystem. Measured against git 2.55, KTD8's text alone would
  answer differently from git in a handful of layouts, so each of those
  DECLINES instead (never a different `Found`): `git_directory_shape` applies
  git's own `is_git_directory` test (a `HEAD` whose content is a symref or
  object id – `head_content_is_valid` mirrors `validate_headref` – plus
  searchable `objects/` and `refs/`, because git WALKS UP past a `.git` that
  fails it); a `.git` directory carrying a `commondir` declines; a gitfile
  target that is not a git directory declines; `repository_format_check` scans
  `<common>/config` (and `config.worktree`) conservatively for
  `core.worktree`, a `core.bare` other than `false`, an unsupported
  `repositoryformatversion` or unknown extension – honoring the common
  config's `core.*` for a linked worktree only under
  `extensions.worktreeConfig`, as git does – and declines on any line it
  cannot follow; and directories not owned by this euid decline (git's
  "dubious ownership"). `effective_uid` (the raw `geteuid`, never a shelled
  `id -u`) is public because the plugin's `tmproot.rs` uses it for its own
  ownership checks. The equivalence matrix in `git/discover/tests.rs` runs
  every scenario against real `git` output and asserts `roots()` equals the
  subprocess probes' answer in ALL of them. Wired into the plugin crate only:
  the hook pipeline (`hook/mod.rs`, where spawning nothing is the point) and
  `compact-window --recommend` (its own root lookup, plus the ledger-row
  attribution in `ledger::rows_for_repository`, which `status`'s compaction
  section also reaches). The `cost` verb does not use it: `cost --here`
  resolves its root with the `git::cwd_repo_root` subprocess probe. Every
  CLI command keeps the subprocess probes (R22, the requirement that the walk
  stays off the CLI's paths).
- `git/gitignore.rs` – `.gitignore` helpers at a git root. `ensure_path_ignored`
  is the single entry point for adding an ignore rule, shared by reverse sync
  (the secret-safety boundary), the CLI's backups dir, the CLI's migrate/init
  bootstrap (`magic.local.json`), core's `state_tree.rs` (`.superset/.magic/`)
  and the plugin's `compact-window --set` (`.claude/settings.local.json`). It
  ensures a `rel` of `PathKind::{File, Dir}` is ignored under a target root,
  adding a rule only when git does not already ignore it, landing it in the
  closest EXISTING `.gitignore` among the path's ancestors (else the target
  root), and returning `Ignored::{Already, Appended}`. It prefers a covering
  glob resolved from a rule-source root over an anchored literal, but reuses
  that glob only when it would land in the same relative directory that owns
  it in the source tree, and only after verifying that it then ignores the path – a
  rule's meaning depends on where its `.gitignore` sits, so a bare `*` from a
  nested `.scratchpad/.gitignore` lifted to the root would ignore the whole
  repository. Otherwise it writes the anchored literal (a bare top-level
  filename such as `.env` stays unanchored, which only widens the ignore), and
  if a nested anchor still does not take, a root-anchored literal as a last
  resort. It is git-TOLERANT (a git failure – e.g. a non-git test tempdir –
  reads as "not ignored" and writes the literal), so a hard secret boundary
  re-checks strictly on top of it (see the CLI's
  `reverse_sync::ensure_gitignored_in_main`). `ensure_entry` (append a line iff
  no exact match exists, create the file if absent, never reorder) is the
  building block beneath it; `ensure_path_ignored` is its only caller, though
  it is `pub`. The private `find_covering_rule` resolves the rule covering a
  path via `git check-ignore -v --no-index` (negations excluded), returning a
  typed `CoveringRule { pattern, source_dir }` – `source_dir` is `None` for a
  rule from outside the tree (`.git/info/exclude`, a global excludesfile), so
  the caller falls back to the literal; `parse_covering_line` is its parser.
  The private `is_ignored_opt` (trailing-slash query for `Dir`),
  `closest_gitignore_dir`, and `anchored_literal` back `ensure_path_ignored`.
- `sync/mod.rs` – the sync engine's pure root, and the home of the ONE
  excluded-trees rule every enumeration layer applies. (The CLI's own
  `sync/mod.rs` declares `reverse_sync` and `merge` and re-exports `apply`,
  `pattern`, `repo_scan` and `under_excluded_tree` from core.)
  `EXCLUDED_TREES` lists four whole directory trees no walk may ever yield,
  each as its exact sequence of path components: `.superset/backups` (the
  tool's own copies of overwritten bytes – recovered secrets),
  `.superset/.magic` (the plugin's gitignored, machine-local state),
  `.scratchpad` (a tree ss-magic does not own but must never push into the
  shared main checkout), and `.git`. `under_excluded_tree(rel)` answers "is
  this rel one of them, or inside one", via the component-by-component
  `starts_with_components` – NEVER a string prefix or a bare name, so a sibling
  `.superset/.magicked/` stays includable, a root-level `.magic` file is
  untouched, and `.superset` ITSELF is never excluded (widening the rule would
  drop the contract files `config.json`, `magic.sh` and `magic.json` out of
  sync and pack entirely). Returning `true` for a tree root is what lets a
  `WalkDir::filter_entry` caller prune the whole subtree. It is applied at
  every point of FINAL enumeration – `apply::walk_source`,
  `apply::copy_dir_recursive(root, src, dst)` (which takes the tree root
  precisely so it can classify each entry's rel), the CLI's reverse-sync
  candidate computation, and the CLI's `pack::append_dir_excluding_trees` –
  never only on an upstream match list. Distinct from `apply::DEFAULT_EXCLUDES`,
  which drops a match containing one of a few directory NAMES (`node_modules`,
  `.venv`) at ANY depth.
- `sync/pattern.rs` – shared syntax checks for both the apply/sync expansion
  path and the CLI's picker validator: `has_glob_meta`, `has_parent_segment`,
  `SyntaxError`, `check_syntax`. One source of truth for "is this pattern
  structurally valid".
- `sync/repo_scan.rs` – `matches_for_patterns(root, &[&str])` walks the
  working tree once with a multi-pattern `GlobSet` and returns a bool vector
  aligned to the input; `pattern_matches_any` is the single-pattern shortcut
  used when the user adds a custom pattern in the bootstrap picker. `OPTIONS`
  holds the four default patterns the picker offers (`.env`, `**/.env`,
  `.env.local`, `**/.dev.vars`), in an order the picker indexes into. The walk
  skips the `SKIP_DIRS` names (`node_modules`, `.venv`, `.git`, `target`); it
  only decides which picker rows are preselected or flagged `(no matches)`,
  and copies nothing.
- `sync/apply.rs` – the glob/exclude/copy engine. `run(src, dest, patterns,
  on_event)` is the forward `ss-magic sync` copy; `match_paths` expands the
  same patterns with the same semantics but copies nothing, for the CLI's
  reverse sync and pack. Delegates syntax checks to `pattern::check_syntax`.
  Expansion rejects absolute and `..` patterns, requires a literal to exist,
  lets a glob match nothing, drops `DEFAULT_EXCLUDES` matches at any depth,
  de-duplicates by relative path and copies a directory match recursively
  through `copy_dir_recursive`. A literal pattern never passes through
  `walk_source`, so `is_excluded` re-applies `under_excluded_tree` to each
  match – that is where a literal naming a file inside an excluded tree is
  stopped. Emits an `Event` stream via a caller-supplied closure so tests can
  collect events while production prints them.
- `superset_files.rs` (the CLI reaches it as
  `crate::workspace::superset_files`) – `.superset/{config.json, magic.sh,
  magic.json, magic.local.json}` I/O (plus the legacy `setup_config.json`
  reader). `MAGIC_SH` embeds `assets/magic.sh` via `include_str!`.
  `load_config` reads Superset-owned `config.json`;
  `merge_setup_into_config` builds a new `Config` from a new `setup` array
  while preserving `teardown` and `run` from disk; `write_config_json` always
  rewrites pretty-printed. `load_overlaid` reads `magic.json` and overlays
  `magic.local.json` (union+dedupe `files`, base order first; for any other
  key the local value wins; a missing base is `None`, a malformed layer is a
  hard error naming the file); `load_magic_json` / `load_magic_local_json` read
  each layer on its own (what the plugin's config write path needs, since it
  load-modify-writes exactly one file). `write_magic_json(root, &MagicConfig)`
  and `write_magic_local_json` take the whole typed config – `MagicConfig`
  carries a `#[serde(flatten)] extras` map, so an unknown key a newer build or
  a hand edit put in the file survives a rewrite instead of being dropped –
  and both commit through a staged sibling plus `rename` (`write_atomically`,
  writing THROUGH a symlink onto its resolved target), never a truncating
  `fs::write`: `magic.json` is a tracked file with an unattended writer (the
  plugin's `seed-config` runs from every session start), so a write that dies
  half-way must leave the previous file, not a prefix;
  `merge_files_into_magic_config` folds a new `files` list into an existing
  config, carrying its `extras`, for the same reason. `write_magic_sh`,
  `bootstrap_magic_local_json`, `ensure_superset_dir` and `default_magic_files`
  (just `.superset/magic.local.json`, so forward sync copies the local overlay
  into each worktree) round out the init/migration writers.
  `load_setup_config` / `SetupConfig` are a READ-ONLY legacy path: migration
  reads the old `setup_config.json` `files` to carry them into `magic.json`.
  `existing_unknown_entries` preserves user-typed patterns across re-runs.
  `copy_into_repo` materializes the staged `.superset/` tree: it copies every
  staged file over its counterpart (files are always overwritten –
  preservation happens upstream of the write; `*.sh` are chmod 0755'd), writes
  `config.json` LAST so a mid-copy failure can never leave it pointing at a
  `magic.sh` that is not there yet, then removes the paths in its `delete` set
  (migration strips the retired `setup.sh`). It never prunes a destination
  entry it was not told to delete (KTD2, the plan's no-prune invariant), which
  is what keeps the plugin's `.superset/.magic/` state alive across an `init`
  or `migrate`.
- `state_tree.rs` – `STATE_REL` (`.superset/.magic`, the plugin's per-worktree
  state tree, spelled in one place) and `ensure_state_ignored(root)`, the ONE
  writer of its gitignore rule (a `Dir` rule through
  `gitignore::ensure_path_ignored`). The CLI calls it eagerly from `init` and
  `migrate` (`workspace/migrate.rs::ensure_bootstrap_gitignores`), so a
  repository is protected before any plugin state exists; the plugin's
  `enable`, and a `config set` that turns `plugin.enabled` on, call it lazily
  through `scratchpad.rs`'s re-export; no hook ever calls it. A test pins
  `STATE_REL` equal to the `.superset/.magic` entry of `sync::EXCLUDED_TREES`
  (that entry is the sync-exclusion control; this module is the gitignore
  side).
- `reponame.rs` – `repo_name_stem(root)`, the `<repo>` stem two consumers must
  agree on: the CLI's pack archive name (`ss-magic-<stem>.tar.bz2`; `pack.rs`
  re-exports it) and the plugin's `<repo>-<branch>` identity slug. It
  normalizes the `origin` remote through `stem_from_origin` (scheme, userinfo,
  host and port stripped, a trailing `.git` dropped, each segment lowercased
  and sanitized by `sanitize_segment`, segments joined with `_` – so the ssh,
  https and scp forms agree and nested GitLab groups keep every segment; a
  local-path or `file://` origin contributes only its final segment), falling
  back to the main checkout's basename. It returns `None` when neither yields
  usable characters and leaves the last resort to the caller: pack uses
  `files`, the plugin identity uses `repo`.
- `release.rs` – the daily-cached, PER-RELEASE-LINE GitHub release check (ureq
  over rustls, ETag, a 5 s global timeout, silent fall-through). The CLI's
  `update/mod.rs` and `update/apply.rs` import it as `ss_magic_core::release`;
  the plugin's `release_check.rs`, `status.rs` and `SessionStart` hook read
  it too. One repository (`REPO_SLUG`, also the CLI download backend's slug)
  hosts two release lines – the CLI on bare `vX.Y.Z` tags and the plugin on
  `ss-magic-plugin-vX.Y.Z` – so the repository-wide `releases/latest` mark
  names whichever line published last and cannot identify either line's
  newest release. The check therefore fetches the first page of
  `/releases?per_page=100`, drops drafts and prereleases, keeps only the tags
  that pass its `Line`'s anchored filter (`parse_line_tag`:
  `tag.strip_prefix(line.tag_prefix)` then exactly three ASCII-digit
  components and nothing else, via `parse_bare_triple`; `CLI_LINE` is `v`,
  cached in `version-check.json`, and `PLUGIN_LINE` is `ss-magic-plugin-v`,
  cached in `plugin-release-check.json` and consumed by the plugin crate
  alone, through its `release-check` verb and `SessionStart` suggestion), and
  `select_newest` takes the GREATEST triple, never the first entry, because
  the list is in creation order. A line with no release among the newest 100
  reads as "no update" – conservative by design. `is_newer(line, tag,
  current)` treats a tag that fails the line's filter as not newer, so a
  cached tag of the other line reads as `UpToDate`. The on-disk
  `Cache { checked_at, tag_name, etag, suggested }` is a superset of the
  shape older binaries wrote, so their cache files still parse; `tag_name`
  holds the tag the line's filter SELECTED, and a `checked_at` of 0 is never
  fresh. `suggested: Option<String>` is the plugin line's once-per-release
  marker, the tag the `SessionStart` hook has already announced; it is skipped
  when `None` and the CLI never sets it, so `version-check.json` never carries
  the key and an older CLI binary reads the file unchanged. `refresh_cache(prior,
  client, line, now) -> Refresh { cache, outcome }` (`RefreshOutcome::{Fetched,
  NotModified, Failed}`) is the ONE derivation of "the next record from the
  prior one plus a fetch": it always fetches with the prior ETag, moves
  `checked_at` on every outcome, keeps the prior tag on
  `NotModified`/`Failed`, and carries `suggested` forward whenever the
  selected tag equals the prior tag, clearing it only for a DIFFERENT tag – so
  a daily refresh cannot re-arm a notice already shown. `run_check` calls it
  once the cache is stale (a fresh cache answers with no network, a `Failed`
  refresh is always `UpToDate`, and the write is best-effort); `check(version)`
  wires `run_check` to the real cache dir, `UreqReleaseClient::new` and
  `CLI_LINE`, and is infallible. The plugin's `release-check --refresh` calls
  `refresh_cache` on every invocation (there is no freshness short-circuit in
  the verb). `read_cache` and `now_secs` are public for the hook, which reads
  the plugin cache directly and must never construct a client. The HTTP call
  sits behind the `ReleaseClient` trait (`FetchOutcome::{Ok, NotModified,
  Failed}`) so tests inject outcomes with no network.
  `UreqReleaseClient::for_product(product, version)` sets the user agent
  (`new(version)` is the CLI's `ss-magic/<version>` shorthand); the version is
  a required argument because `CARGO_PKG_VERSION` read inside core would name
  the library's `0.1.0`, not the calling binary's release.
  `resolve_newest_uncached` is the cache-free resolver behind `ss-magic
  update` (no `If-None-Match`; `None` means "could not check").
  `cache_dir()` resolves the OS cache dir (`~/Library/Caches/ss-magic`,
  `~/.cache/ss-magic`) and creates it, which suits the writers;
  `existing_cache_dir()` returns it only when it already exists, for the
  plugin's read-only `status`.
- `style.rs` (the CLI reaches it as `crate::tui::style`, the plugin crate as
  `ss_magic_core::style`) – palette (gray info, bold green ok, bold
  orange/xterm 208 warn, bold red err, bold cyan header). One `OnceLock<bool>`
  captures the color decision (NO_COLOR + supports-color); `init()` makes it
  from the terminal, `init_no_color()` forces it off (what a plugin hook
  calls, since an ANSI escape would break its JSON), and `enabled()` reads it;
  whichever of `init` and `init_no_color` runs first wins. `paint` takes the
  flag explicitly so tests bypass the global. It knows nothing about
  `inquire`.
- `hashing.rs` – the content-fingerprint primitives. `fnv1a_64` / `hash_file`
  are the non-cryptographic hashes behind the plugin's conclusion-cache keys
  and claim-file names (`bypass`, `expect-artifact`) and behind reverse sync's
  content-hash baseline on a filesystem that reports no mtime; FNV-1a rather
  than `DefaultHasher` because std explicitly does NOT promise its output is
  stable across releases or processes, and a long-lived cache keyed on an
  unstable hash silently rots. `sha256` / `sha256_hex` are a hand-rolled FIPS
  180-4 SHA-256 (pinned by literal test vectors), present for ONE reason: the
  plugin's per-machine temp-root identifier must be derived identically by
  this Rust code and by the shell bootstrap's `shasum -a 256`, so the
  algorithm has to be one every platform already implements the same way.
- `testutil.rs` – the shared test helpers (git fixtures such as `git_run`,
  `init_main_repo` and `make_worktree`, plus `run_ignored_test_in_child` /
  `run_ignored_test_in_child_from` and `test_path_in_binary` for a test whose
  subject is the process environment or working directory). Compiled under
  `cfg(test)` for core's own suite and behind core's `testutil` feature, which
  each binary enables from its `[dev-dependencies]` only, so no release build
  contains it; every helper is `pub` because the callers sit in other crates.
  The rules for using it are in the test-layout convention in
  `.claude/rules/conventions.md`.
