# Structure pins and review rules for the simplify sweep

Read this before reviewing. Every item here is a deliberate decision recorded in `CLAUDE.md` or the
workspace-split plan. A finding that undoes one of these is a FALSE POSITIVE; do not report it, or
report it under `category: "settled"` with the pin named, so the validator can drop it.

## Repository shape (do not propose to change)

- Three crates: `crates/ss-magic-core` (library, shared plumbing), `crates/ss-magic` (sync CLI, binary
  `ss-magic`), `crates/ss-magic-plugin` (Claude Code plugin, binary `ss-magic-plugin`). The two
  binaries depend on core and NEVER on each other. The plugin crate must never link `self_update`,
  `inquire` or `ratatui`; core's `style.rs` knows nothing about `inquire` (the CLI's `tui/theme.rs`
  is the inquire half). A helper shared by both binaries belongs in core; a helper used by one binary
  belongs in that binary.
- Both binaries re-export core's `git` and `hashing` under `crate::` (`crate::git::…` paths), on
  purpose, so old paths read unchanged. Not a duplication.
- Test layout: every module declares `#[cfg(test)] mod tests;` with the body in the sibling file
  `<module>/tests.rs` (private-item access). Shared test helpers live in
  `crates/ss-magic-core/src/testutil.rs` (feature `testutil`, dev-dependency only). A test helper
  re-implemented in a module's `tests.rs` when `testutil` already provides it IS a finding; test
  bodies that look alike are NOT.
- A test whose subject is the process environment (`PATH`, `GIT_*`, cwd) runs in a CHILD process via
  `testutil::run_ignored_test_in_child` / `_from`. Do not propose `set_var` instead.
- Version surfaces are grouped by release line and enumerated by `scripts/build-plugin-zip.py --check`
  (seven assertion lines). Do not propose collapsing surfaces. The literal zip filename in the plugin
  crate's `[[package.metadata.dist.extra-artifacts]]` is deliberately literal (cargo-dist does not
  template it).

## Deliberate duplications (each one is documented as "change both or neither")

- The `hooks spawn through the shim` invariant is asserted in BOTH `scripts/build-plugin-zip.py
  --check` and `scripts/test-bootstrap.sh`.
- `plugin/hooks/bootstrap.sh` calls its `seed_config` helper from THREE sites (already-installed fast
  path, re-check under the install lock, end of a fresh install). The once-ness lives in the binary.
- `plugin/lib/execguard.sh` is sourced by both the shim and the wrapper – that is the fix for a prior
  drift, not a duplication.
- The excluded-trees filter (`sync::under_excluded_tree` over `sync::EXCLUDED_TREES`) is applied at
  EVERY point of final enumeration (`apply::walk_source`, `apply::copy_dir_recursive`, reverse sync's
  candidate computation, pack's `append_dir_excluding_trees`), never only on an upstream list.
  Applying it in several walks is the rule, not a repeat.
- `seed_block()` in the plugin's `config.rs` is built field by field as a literal map rather than
  serialized from `GateConfig` – so the writer is structurally unable to emit `enabled`.
- `seed_config_at` calls `landing()` before `write_plugin_key` calls it again: the seed needs an
  OUTCOME (`SeedOutcome::OutsideRepository`), the writer needs an error.
- The hashing module hand-rolls SHA-256 (must match the shell bootstrap's `shasum -a 256` byte for
  byte) and uses FNV-1a rather than `DefaultHasher` (std does not promise a stable hash). Do not
  propose a crate or `DefaultHasher`.
- `git::status_porcelain` is built on the untrimmed `git_raw`, never the trimming `git()` helper.
  The two helpers coexist on purpose.
- Fail-open hooks (`hook::run` has no non-zero exit path) versus fail-closed gates (scratchpad ignore
  gate, tracked-path check, tmproot ownership) are opposite postures by design. Never "unify" them.
- `pathnorm::normalize` is a textual `..` canceller; do not propose `parent()`/`file_name()` walks or
  `canonicalize` as a replacement. Canonicalization may ADD a denial, never CLEAR one.
- `claim::take` is rename-based; never propose an `unlink`-based claim.
- `HookEvent::{Unknown, Missing}` are VALUES, not parse errors; `PermissionDecision` has only `Deny`;
  `PreCompact`/`SessionEnd` have no `Response` variant. Structural guarantees, not gaps.
- Testability seams that inject the environment (`session_start::Surroundings`,
  `compact_window::Sources`, `status::Inputs`, `Baseline`/`ApplyContext` in reverse sync) are not
  parameter sprawl.
- `[profile.release] opt-level = "z"` is a measured decision (KTD9).
- `git::discover` DECLINES (falls back to git) in every layout where the fast answer might differ
  from git's; a proposal to "just answer" in one of those layouts is out.
- The plugin's `atomic::write_atomically` and core's private `superset_files::write_atomically` are
  two implementations today. Reporting THAT as duplication is legitimate (core cannot depend on the
  plugin, so the shared copy would live in core). It is listed here only so you know it is not pinned.

## Review rules

- Preserve exact behavior: outputs, errors, side effects, ordering. If you cannot argue preservation,
  mark the finding `confidence <= 50` and say why.
- Never simplify away a safety check: trust-boundary validation, containment checks, fail-closed
  gates, data-loss protection. If a fix would thin one, do not propose it.
- Readable, explicit code over compact code. Fewer lines is not the goal.
- Comments explain rules and reasons; a comment citing a plan id alone is a finding (category
  `quality`), a comment stating a non-obvious invariant is not.
- Markdown you write uses en dashes (–), never em dashes.
