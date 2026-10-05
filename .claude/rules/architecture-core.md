---
paths:
  - "crates/ss-magic-core/**"
---

## Core modules (`crates/ss-magic-core/src/`)

- `git/mod.rs` — read-only probes (`is_worktree`, `main_checkout_root`,
  `cwd_repo_root`, `main_branch_name`, `origin_url` (backs pack's
  repo-derived archive naming), plus the reverse-sync probes
  `untracked_files` (`git ls-files --others` – untracked *including*
  gitignored, since reverse sync pushes gitignored secrets), `tracked_files`
  (`git ls-files --cached -z` – the mirror of `untracked_files` that does
  POSITIVE tracked determination for the secret push gate: a path NOT in this
  set is treated as an untracked secret, so an unenumerable name fails closed),
  `is_ignored`, `is_ignored_str` (the raw-pathname variant so a caller can force
  git's directory-only match with a trailing slash), `is_ignored_no_index_str`
  (the `--no-index` variant that asks whether the IGNORE RULES cover a path,
  ignoring the index – what the plugin's state-tree gate needs, since a tracked
  file inside the tree must not read as "the tree is unignored"),
  `status_porcelain` (parsed `(status, path)` pairs behind the checklist commit
  nudge – built on `git_raw`, NEVER the trimming `git` helper, because porcelain's
  leading column is a literal space for a worktree-only modification and a blanket
  `.trim()` shifts every field), `symbolic_ref_head` / `short_head_sha` (the branch
  name and abbreviated SHA behind the plugin's `<repo>-<branch>` identity slug);
  `parse_ls_files_z` is the shared NUL-split behind BOTH `untracked_files` and
  `tracked_files`, defensively dropping any absolute / `..`-bearing entry in one
  place) and mutating primitives (`stage_paths`, `nothing_to_commit`, `commit`,
  `push`, `push_upstream`, `create_branch`, `pr_create`,
  `timestamp_branch_suffix`, `gh_available` – `nothing_to_commit` runs
  `git diff --cached --quiet` so `workspace/migrate.rs`'s final-action arms can
  skip an empty commit instead of failing on one). All `git`/`gh` invocations shell out via a shared `git_raw`
  helper that surfaces stderr verbatim; `git` and `git_optional` are thin
  one-liners on top. (The bare location-auto `probe`/`Mode` dispatch was removed
  in U13 – routing is now the menu via `is_worktree` + `main_checkout_root`.)
- `git/discover.rs` – the ONE filesystem-only reduction of git behavior in the
  crate: the two roots the plugin's hook pipeline needs (`cwd_repo_root` and
  `main_checkout_root`) answered without spawning a process, and by convention
  never anything more – no ref, index, or write handling (R23). `discover(cwd)`
  returns `Found(Roots { worktree_root, common_dir, main_checkout_root })`,
  `NotARepository`, or `Undecided(reason)`; `roots(cwd)` maps `Found` to its
  roots, `NotARepository` to two `None`s with no subprocess, and `Undecided` to
  exactly the two probe calls the pipeline made before – in the same order and
  against the same directories, including the `.git/hooks` shape where
  `--show-toplevel` fails but `--git-common-dir` still answers. The invariant
  that makes it safe: **a fast answer is byte-equal to git's, or there is
  none.** The walk (KTD8) declines on any of six `GIT_*` variables
  (`DECLINING_ENV`: R21's five plus `GIT_OBJECT_DIRECTORY`), canonicalizes
  `cwd` (`canonicalize` owns `.`/`..`/symlinks – nothing lexical is
  reimplemented), and inspects each ancestor: a directory that itself looks
  like a git dir declines; a symlinked `.git` declines; a `.git` directory
  yields `{D, D/.git, D}`; a gitfile (`gitdir: `, at most 4 KiB, resolved
  against `D`, canonicalized) needs a `commondir` beside its target – absent
  is the submodule shape and declines – and yields `{D, C, parent(C)}`; absent
  walks up unless the parent is on another filesystem. Measured against git
  2.55, KTD8's text alone would answer differently from git in a handful of
  layouts, so each of those DECLINES instead (never a different `Found`):
  `git_directory_shape` applies git's own `is_git_directory` test (a `HEAD`
  whose content is a symref or object id – `head_content_is_valid` mirrors
  `validate_headref` – plus searchable `objects/` and `refs/`, because git
  WALKS UP past a `.git` that fails it); a `.git` directory carrying a
  `commondir` declines; a gitfile target that is not a git directory declines;
  `repository_format_check` scans `<common>/config` (and `config.worktree`)
  conservatively for `core.worktree`, a `core.bare` other than `false`, an
  unsupported `repositoryformatversion` or unknown extension – honoring the
  common config's `core.*` for a linked worktree only under
  `extensions.worktreeConfig`, as git does – and declines on any line it cannot
  follow; and directories not owned by this euid decline (git's "dubious
  ownership"). The equivalence matrix in `git/discover/tests.rs` runs every
  scenario against real `git` output and asserts `roots()` equals today's
  answer in ALL of them. Wired into the plugin crate only: the hook pipeline
  (`hook/mod.rs`, where spawning nothing is the point) and the two read-only
  verbs that attribute ledger rows to a repository, `compact-window
  --recommend` and the `cost` ledger's `rows_for_repository`. Every CLI
  command keeps the subprocess probes (R22).
- `superset_files.rs` (core; the CLI reaches it as
  `crate::workspace::superset_files`) – `.superset/{config.json, magic.sh,
  magic.json, magic.local.json}` I/O (plus the legacy `setup_config.json`
  reader).
  `load_config` reads Superset-owned `config.json`;
  `merge_setup_into_config` builds a new `Config` from a new `setup`
  array while preserving `teardown` and `run` from disk;
  `write_config_json` always rewrites pretty-printed. `load_overlaid`
  reads `magic.json` and overlays `magic.local.json` (union+dedupe
  `files`, base order first); `load_magic_json` / `load_magic_local_json` read
  each layer on its own (what the plugin's config write path needs, since it
  load-modify-writes exactly one file). `write_magic_json(root, &MagicConfig)`
  and `write_magic_local_json` take the whole typed config – `MagicConfig`
  carries a `#[serde(flatten)] extras` map, so an unknown key a newer build or a
  hand edit put in the file survives a rewrite instead of being dropped – and
  both commit through a staged sibling plus `rename` (`write_atomically`,
  writing THROUGH a symlink onto its resolved target), never a truncating
  `fs::write`: `magic.json` is a tracked file with an unattended writer now
  (the plugin's `seed-config` runs from every session start), so a write that
  dies half-way must leave the previous file, not a prefix;
  `merge_files_into_magic_config` folds a new `files` list into an existing
  config for the same reason. `write_magic_sh`,
  `bootstrap_magic_local_json`, and `default_magic_files` round out the
  init/migration writers. `load_setup_config` / `SetupConfig` survive as
  a READ-ONLY legacy path: migration reads the old `setup_config.json`
  `files` to carry them into `magic.json`. `existing_unknown_entries`
  preserves user-typed patterns across re-runs. `copy_into_repo`
  materializes the staged `.superset/` tree atomically (files always
  overwritten — preservation happens upstream of the write; `*.sh` are
  chmod 0755'd; a `delete` set strips the retired `setup.sh`).
- `sync/mod.rs` (core) – the sync engine's pure root, and the home of the ONE
  excluded-trees rule every enumeration layer applies. (The CLI's own
  `sync/mod.rs` declares `reverse_sync` and `merge` and re-exports the rest
  from here.) `EXCLUDED_TREES` lists
  four whole directory trees no walk may ever yield, each as its exact sequence
  of path components: `.superset/backups` (the tool's own copies of overwritten
  bytes – recovered secrets), `.superset/.magic` (the plugin's gitignored,
  machine-local state), `.scratchpad` (a tree ss-magic does not own but must
  never push into the shared main checkout), and `.git`.
  `under_excluded_tree(rel)` answers "is this rel one of them, or inside one",
  via the component-by-component `starts_with_components` – NEVER a string
  prefix or a bare name, so a sibling `.superset/.magicked/` stays includable, a
  root-level `.magic` file is untouched, and `.superset` ITSELF is never
  excluded (widening the rule would drop the contract files `config.json`,
  `magic.sh` and `magic.json` out of sync and pack entirely). Returning `true`
  for a tree root is what lets a `WalkDir::filter_entry` caller prune the whole
  subtree. It is applied at every point of FINAL enumeration –
  `apply::walk_source`, `apply::copy_dir_recursive(root, src, dst)` (which takes
  the tree root precisely so it can classify each entry's rel), reverse sync's
  candidate computation, and pack's `append_dir_excluding_trees` – never only on
  an upstream match list. Distinct from `apply::DEFAULT_EXCLUDES`, which drops a
  match containing one of a few directory NAMES (`node_modules`, `.venv`) at ANY
  depth. (This generalizes the retired `reverse_sync::under_backups_dir`.)
- `sync/repo_scan.rs` — `matches_for_patterns(root, &[&str])` walks the
  working tree once with a multi-pattern `GlobSet` and returns a bool
  vector aligned to the input. `pattern_matches_any` is the single-
  pattern shortcut used when the user adds a custom pattern in the
  bootstrap picker.
- `sync/pattern.rs` — shared syntax checks for both the apply/sync
  expansion path and the picker UI validator: `has_glob_meta`,
  `has_parent_segment`, `SyntaxError`, `check_syntax`. One source of
  truth for "is this pattern structurally valid".
- `sync/apply.rs` — the glob/exclude/copy engine reused by forward `sync`
  (and, via `match_paths`, by reverse sync). Delegates syntax checks to
  `pattern::check_syntax`. Emits an `Event` stream via a caller-supplied
  closure so tests can collect events while production prints them.
  (`load_main_config`, the old interactive apply path, was removed in
  U13.)
- `style.rs` (core; the CLI reaches it as `crate::tui::style`, the plugin crate
  as `ss_magic_core::style`) – palette (gray
  info, bold green ok, bold orange/xterm 208 warn, bold red err, bold cyan
  header). One `OnceLock<bool>` captures the color decision (NO_COLOR +
  supports-color); `init()` makes it from the terminal, `init_no_color()`
  forces it off, `enabled()` reads it. It knows nothing about `inquire`.
- `git/gitignore.rs` — `.gitignore` helpers at a git root. `ensure_path_ignored`
  is the single entry point shared by reverse sync (the secret-safety boundary),
  the backups dir, and the migrate/init bootstrap: it ensures a `rel` of
  `PathKind::{File, Dir}` is ignored under a target root, adding a rule only when
  git does not already ignore it, landing it in the closest EXISTING `.gitignore`
  among the path's ancestors (else the target root), preferring a covering glob
  resolved from a rule-source root (verified) over an anchored literal, and
  returning `Ignored::{Already, Appended}`. It is git-TOLERANT (a git failure –
  e.g. a non-git test tempdir – reads as "not ignored" and writes the literal),
  so a hard secret boundary re-checks strictly on top of it (see
  `reverse_sync::ensure_gitignored_in_main`). `ensure_entry` (append a line iff
  no exact match exists, create the file if absent, never reorder) is now the
  building block beneath it, still called directly where the exact rule text is
  known; the private `find_covering_rule` resolves the rule covering a path via
  `git check-ignore -v` (negations excluded), returning a typed
  `CoveringRule { pattern, source_dir }` – the source dir matters because a
  pattern is only meaningful relative to the `.gitignore` it came from, so the
  caller can verify the rule actually covers the path before copying it into the
  target root; `parse_covering_line` is its parser. The private `is_ignored_opt`
  (trailing-slash query for `Dir`), `closest_gitignore_dir`, and
  `anchored_literal` back `ensure_path_ignored`.
- `hashing.rs` (core) – the content-fingerprint primitives. `fnv1a_64` /
  `hash_file` are the non-cryptographic hashes behind cache keys and claim-file
  names; FNV-1a rather than `DefaultHasher` because std explicitly does NOT
  promise its output is stable across releases or processes, and a long-lived
  cache keyed on an unstable hash silently rots. `sha256` / `sha256_hex` are a
  hand-rolled FIPS 180-4 SHA-256 (pinned by literal test vectors), present for
  ONE reason: the plugin's per-machine temp-root identifier must be derived
  identically by this Rust code and by the shell bootstrap's `shasum -a 256`, so
  the algorithm has to be one every platform already implements the same way.
  (Replaces the removed `reverse_sync::hash_file`.)
