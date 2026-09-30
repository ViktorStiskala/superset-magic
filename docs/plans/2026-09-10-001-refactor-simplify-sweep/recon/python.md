# Recon: python (scripts/build-plugin-zip.py)

## Map

`scripts/build-plugin-zip.py` (1378 lines, stdlib only) – the deterministic builder for the plugin
release zip, plus every release-time assertion CI runs against the tree (`--check`, `--check-bump`)
and the builder's own reproducibility/refusal tests (`--selftest`). Structured as: tree-collection →
version-surface reading → CI assertions → bump check → manifest rewrite → selftest → argparse `main`.

Public / module-level items, in file order:

- `BuildError(Exception)` (line 89) – the one exception type; every refusal and check failure raises
  or is reported through it.
- `collect_entries(plugin_dir) -> list[tuple[str, Path]]` (112) – walks `plugin/`, sorts, refuses a
  symlink/non-ASCII name/irregular file, returns `(arcname, path)` pairs.
- `mode_for(arcname) -> int` (162) – `0755` under `bin/` or `*.sh`, else `0644`. Pure, small, generic
  (candidate for reuse anywhere else that needs the same policy, but nothing else in this partition
  needs it).
- `build_zip_bytes(plugin_dir) -> bytes` (174) – produces the archive bytes; pure in tree contents.
- `digest_of(data: bytes) -> str` (195) – `hashlib.sha256(data).hexdigest()`. Trivial one-liner,
  duplicated call-shape only (see Constants section – the underlying algorithm is intentionally
  reimplemented in Rust per the structure pins, not a finding).
- `version_surfaces(root) -> dict[str, dict[str, str]]` (311) – every version-bearing surface,
  grouped by release line (`{CLI_LINE: {...}, PLUGIN_LINE: {...}}`).
- `default_out_path(root) -> Path` (374) – derives the output zip filename from the plugin crate's
  version.
- `check_manifest_keys(root) -> list[str]` (392) – R101: the marketplace `source` object actually
  carries a well-formed `sha256`.
- `check_versions(root) -> list[tuple[str, list[str]]]` (491) – R95, as three separately-reported
  assertions (each line's surfaces agree; the two lines differ; README's pin is `<=` the CLI version).
- `check_hooks_shim(root) -> list[str]` (561) – every `hooks.json` entry spawns `bash` on a shipped
  script, never the binary directly. Explicitly documented (574-580) as a DELIBERATE duplicate of the
  same assertion in `scripts/test-bootstrap.sh` (pinned in structure-pins.md).
- `check_workspace_shape(root) -> list[str]` (687) – the plugin (and core, transitively) must not
  declare `self_update`/`inquire`/`ratatui`; core must carry `publish = false`.
- `check_pin(root, plugin_dir) -> list[str]` (730) – R96: committed sha256 == re-derived digest.
- `bump_verdict(base_digest, base_version, cur_digest, cur_version) -> str | None` (757) – pure
  decision behind `--check-bump` (R98): content changed but version did not, or version moved
  backwards.
- `check_bump(root, ref) -> list[str]` (790) – shells to `git archive`/`tar` to materialize the plugin
  tree at `ref`, then calls `bump_verdict`.
- `update_manifest(root, digest) -> bool` (843) – regex-rewrites only the `sha256` value in
  `marketplace.json`, re-parses to confirm the file is still valid JSON.
- `selftest() -> int` (1039) – ~215 lines of the builder's own reproducibility/refusal/assertion
  tests (AE79, AE80, AE81, R95, hook-shim, workspace-shape, extra-artifact fallback).
- `main(argv=None) -> int` (1264) – argparse + dispatch for `--out/--print-digest/--update-manifest/
  --check/--check-bump/--selftest`.

Private helpers (small/generic – candidates worth a glance for being reimplemented elsewhere):

- `_reject_name(rel, entry) -> None` (98) – ASCII-only path check; single caller (`collect_entries`).
- `_cargo_toml_version(path) -> str` (204) – hand-rolled `[package] version` reader (no TOML parser;
  `tomllib` is 3.11+ only). Small, generic text-scanning helper; called 4 times in this file
  (`version_surfaces` x2, `default_out_path`, `check_bump` reads `plugin.json` instead so not there).
- `_cargo_lock_version(path, crate) -> str` (219) – regex over `Cargo.lock` for one crate's version.
  Called twice (CLI line, plugin line).
- `_marketplace_entry(root) -> dict` (231) – loads and validates the single `plugins[0]` entry.
  Called 3 times (`version_surfaces`, `check_manifest_keys`, `check_pin`).
- `_read_text(root, rel, what) -> str` (243) – "read a required file, turn `FileNotFoundError` into a
  named `BuildError`". Generic and reused 8 times in this file alone (README, plugin manifest, hooks
  manifest, PLUGIN_PIN, both crate manifests, core manifest). Good candidate to note as *already*
  the shared idiom in this file – nothing here duplicates it.
- `_artifact_versions(text) -> list[str]` (274) – `ARTIFACT_RE.findall`. One-line wrapper, 2 callers.
- `_extra_artifact_versions(root) -> tuple[str, list[str]]` (278) – reads the extra-artifact zip
  filename from the plugin crate manifest, falling back to `dist-workspace.toml`. **This exact
  fallback order is re-implemented independently in bash** – see Leads L1.
- `_readme_installer_tag(root) -> str` (305) – regex-extracts the pinned installer tag from README.
- `_group_disagreement(line, surfaces) -> list[str]` (440) – "do these surfaces' values all agree",
  rendered as a diagnostic block. Generic, 2 callers (CLI group, plugin group).
- `_readme_pin_problems(pin, cli_version) -> list[str]` (450) – the `<=` comparison and its error
  text; 1 caller.
- `_declared_dependency_names(text) -> set[str]` (638) – hand-rolled TOML dependency-table scanner
  (comments skipped, renames resolved, hyphens folded). Fairly involved (47 lines) for something used
  by exactly one caller (`check_workspace_shape`), across 2 manifests.
- `_semver(v) -> tuple[int, int, int]` (753) – `"1.2.3" -> (1,2,3)`. Used by `_readme_pin_problems`
  and `bump_verdict`. Same *problem* (numeric compare of a `vX.Y.Z`/`X.Y.Z` tag) is solved a third,
  different way in bash – see Leads L2.
- Selftest-only private helpers, all single-purpose fixture builders: `_write` (868), `_sample_tree`
  (874), `_version_repo` (913, ~106 lines – see Leads L4), `_version_problems` (1022),
  `_expect_refusal` (1027). Plus the nested `git(*args)` closure inside `check_bump` (793) and inside
  `selftest`'s two file-mode branches.

## Constants and literals

Module-level consts (all in `scripts/build-plugin-zip.py`; none declared elsewhere in the workspace –
this is a standalone script, so nothing here appears in `index/consts.md`, which covers Rust only):

| name | line | value | note |
|---|---|---|---|
| `FIXED_DATE_TIME` | 70 | `(1980, 1, 1, 0, 0, 0)` | the ZIP epoch; unique to this file |
| `UNIX_CREATE_SYSTEM` | 71 | `3` | unique to this file |
| `MODE_FILE` | 72 | `0o644` | see below – same literal value as 3 Rust consts |
| `MODE_EXEC` | 73 | `0o755` | unique numeric value in this file |
| `EXCLUDED_NAMES` | 74 | `frozenset({".DS_Store"})` | unique |
| `VERSION_RE` | 75 | `r"^\d+\.\d+\.\d+$"` | see Leads L2 |
| `ARTIFACT_RE` | 76 | `r"ss-magic-plugin-v(\d+\.\d+\.\d+)\.zip"` | matched in bash by a near-identical pattern, L1 |
| `SHA256_RE` | 77 | `r"^[0-9a-fA-F]{64}$"` | unique |
| `TAG_RE` | 78 | `r"^v(\d+\.\d+\.\d+)$"` | see Leads L2 |
| `REPO_ROOT` | 80 | `Path(__file__).resolve().parent.parent` | unique |
| `CLI_LINE` | 85 | `"ss-magic"` | also a crate/binary name everywhere in the workspace (expected, not a finding) |
| `PLUGIN_LINE` | 86 | `"ss-magic-plugin"` | same |
| `CLI_MANIFEST` | 255 | `Path("crates")/"ss-magic"/"Cargo.toml"` | path literal |
| `PLUGIN_MANIFEST` | 256 | `.../ss-magic-plugin/Cargo.toml` | path literal |
| `CORE_MANIFEST` | 257 | `.../ss-magic-core/Cargo.toml` | path literal |
| `PLUGIN_PIN` | 261 | `Path("plugin")/"ss-magic-plugin.version"` | filename string also literal at 879, 970, 1157 in this file, and owned/read by Rust's `status.rs::PIN_FILE` (see Cross-module refs) |
| `README_SURFACE` | 266 | `"README.md installer tag"` | unique |
| `README_INSTALLER_RE` | 271 | `r"releases/download/([^/\s)]+)/ss-magic-installer\.sh"` | unique |
| `HOOK_ENTRYPOINTS` | 555-558 | `("${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.sh", ".../bootstrap.sh")` | these exact strings recur across `hooks.json`, `run-hook.sh`, `bootstrap.sh`, `test-bootstrap.sh` per `index/literals.md`; **deliberate duplicate** per structure-pins.md item 1 (`hooks spawn through the shim` assertion in both this script and `test-bootstrap.sh`) – not a new finding |
| `FORBIDDEN_PLUGIN_DEPS` | 634 | `("self_update", "inquire", "ratatui")` | mirrors the pinned rule (structure-pins.md); the same 3 names are asserted by `cargo tree -i` in CI – intentional, not a finding |
| `_DEP_TABLES` | 635 | `frozenset({"dependencies", "dev-dependencies", "build-dependencies"})` | unique |
| `_HOOK_GOOD` | 897-902 | dict literal, a well-formed hook entry | selftest fixture |
| `_HOOK_BAD` | 905-910 | dict literal, the ENOENT-risk shape | selftest fixture |

Other hardcoded values worth flagging (name/path/key/limit shape, file:line, what it denotes):

- `0o644` / `0o755` (72-73) – file-permission literals. The *same values* (`0o644`) are declared as
  `FILE_MODE` (`crates/ss-magic-plugin/src/setup_ci.rs:84`) and twice more as `NEW_FILE_MODE`
  (`crates/ss-magic-plugin/src/checklist/verbs.rs:111`, `crates/ss-magic-plugin/src/compact_window.rs:132`)
  per `index/consts.md`'s `dup-name` flags. Different languages, same policy value repeated 4 times
  total across the workspace; not fixable by sharing a constant across languages, but worth a lead
  for the reviewer to note as a documented-once-nowhere value (see Leads L3).
- `".DS_Store"` (74, and again at 1083-1084 in selftest) – matches `index/literals.md`'s
  `.DS_Store` row (3 occurrences, all inside this file only).
  `marketplace.json` (232, 845, 999) – same file, 3 internal call sites, all in this partition.
- `"ss-magic-plugin.version"` (261, and the literal string again at 879, 970, 1157 in test fixtures) –
  same filename is the Rust `status.rs:106` `PIN_FILE` const, whose own doc comment (status.rs,
  above line 106) explicitly says this Python script is "asserted by `scripts/build-plugin-zip.py
  --check`", i.e. the duplication is acknowledged and intentional (structure pin), not new.
- `"pre-tool-use"` (900, 908) – hook-name literal shared with 4+ other files per `index/literals.md`
  row 36; this file's copies are inside `_HOOK_GOOD`/`_HOOK_BAD` selftest fixtures, cross-referenced
  against the real `main.rs`/`status.rs` spellings only by convention, not by a shared constant –
  low-risk since selftest would fail loudly on a real spelling drift.
- `9.9.9` (878, 916 default arg, 917 default arg) – sentinel "obviously fake" version, also used at
  `crates/ss-magic-plugin/src/*/tests.rs` and `scripts/test-bootstrap.sh:161` per `index/literals.md`
  row 154. Cosmetic naming consistency only.
- `ss-magic-plugin-v[0-9]+\.[0-9]+\.[0-9]+\.zip` pattern, expressed as `ARTIFACT_RE` here (76) and
  re-typed almost verbatim as a bash variable in `.github/workflows/ci.yml:296`
  (`pattern='ss-magic-plugin-v[0-9]+\.[0-9]+\.[0-9]+\.zip'`) – see Leads L1.
- `^v[0-9]+\.[0-9]+\.[0-9]+$` in `scripts/mark-latest.sh:59` vs `TAG_RE`/`VERSION_RE` here (75, 78) –
  see Leads L2.

## Cross-module references

Imports (all stdlib; file docstring line 41 states "Standard library only" deliberately, so the tree
that builds it and the tree that runs `--check` never disagree over a third-party version):
`argparse, hashlib, io, json, os, re, stat, subprocess, sys, tempfile, zipfile, pathlib.Path`
(lines 56-67). No imports from any workspace crate or another script – this file is invoked as a
subprocess (`python3 scripts/build-plugin-zip.py ...`), never imported as a module (confirmed: no
`import build_plugin_zip` / `from build_plugin_zip` anywhere in the tree).

Since nothing imports it as a module, "callers of a public symbol" means invocations of the CLI
surface (flags), not Python-level calls. Grepped call sites for each entry point:

- bare invocation (build the zip) – `crates/ss-magic-plugin/Cargo.toml:65`
  (`build = ["python3", "scripts/build-plugin-zip.py"]`, the cargo-dist extra-artifact build command);
  `.github/workflows/ci.yml:307` (CI's own asset-build step).
- `--selftest` – `.github/workflows/ci.yml:129`; `CONTRIBUTING.md:198`; `CLAUDE.md:1519`.
- `--check` – `.github/workflows/ci.yml:137`; `CONTRIBUTING.md:74,199,340`; `CLAUDE.md:46,111`;
  `.cursor/BUGBOT.md:39,986,1053,1333,1364`.
- `--check-bump REF` – `.github/workflows/ci.yml:204`.
- `--update-manifest` – `CLAUDE.md:1551`; `CONTRIBUTING.md:361`; own refusal text at line 743.
- No caller outside this file uses `--print-digest`, `--out`, or `--plugin-dir` (unused outside this
  file, beyond the `main()` argparse definitions themselves).

Every function/const defined in this file is private to it in the sense that nothing else in the
repository can reference a Python symbol from it (no module import path exists); the "cross-module"
surface that matters here is the **value agreement** the script enforces between itself and:
`crates/ss-magic-plugin/src/status.rs` (`PIN_FILE`, `BINARY_REL` – same filenames, independently
declared, documented as intentional), `.claude-plugin/marketplace.json` (data it reads and rewrites),
`plugin/hooks/hooks.json` (data it validates), `plugin/.claude-plugin/plugin.json` (data it reads),
`README.md` (data it reads), `Cargo.lock` / the three crate `Cargo.toml`s (data it reads),
`dist-workspace.toml` (data it reads as a fallback), and `.gitattributes` (documents *why* this file
and `marketplace.json` are pinned `text eol=lf` while `plugin/**` is `-text` – this script is the
reason that pin exists, per the comment at `.gitattributes:1-11`).

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `scripts/build-plugin-zip.py:278-302` (`_extra_artifact_versions`) vs `.github/workflows/ci.yml:293-301` | The "read the extra-artifact zip filename from the plugin crate manifest, fall back to `dist-workspace.toml`" logic is implemented twice: once in Python (regex + fallback), once in bash (`grep -oE` + fallback) with the same two-source order and the same regex shape (`ss-magic-plugin-v[0-9]+\.[0-9]+\.[0-9]+\.zip` vs `ARTIFACT_RE`). A change to the fallback order or the artifact naming rule has to be made in both places or CI's own asset-build step and `--check`'s surface enumeration silently disagree. | `.github/workflows/ci.yml:296-301` (own partition: shell/yml, cross-partition lead) |
| L2 | duplication | `scripts/build-plugin-zip.py:75,78,753-754` (`VERSION_RE`, `TAG_RE`, `_semver`) vs `scripts/mark-latest.sh:59,80-104` | Three independent implementations of "parse a `vX.Y.Z` (or bare `X.Y.Z`) string and compare/select numerically" exist in the workspace: this file's regex+int-tuple compare, `mark-latest.sh`'s regex+`sort -t . -k1,1n -k2,2n -k3,3n`, and (per CLAUDE.md's Architecture section) Rust's `release.rs::parse_line_tag`/`select_newest`. None share code (different languages), but a reviewer changing the tag shape (e.g. adding a pre-release suffix) needs to touch all three, and only `--selftest`/`test-mark-latest.sh` would catch a drift. | `scripts/mark-latest.sh:59,80-104`; also `crates/ss-magic-core/src/release.rs` (Rust partition) |
| L3 | hardcoded-value | `scripts/build-plugin-zip.py:72-73` (`MODE_FILE`, `MODE_EXEC`) | `0o644`/`0o755` are policy literals with no single owning definition; the same `0o644` value is separately named `FILE_MODE` (`crates/ss-magic-plugin/src/setup_ci.rs:84`) and `NEW_FILE_MODE` twice more (`checklist/verbs.rs:111`, `compact_window.rs:132`), each a distinct Rust const with the same value and a different name. Not fixable across the language boundary, but the 3-way Rust duplication (all `0o644`, two different names) is worth flagging to whoever reviews the Rust partition. | `crates/ss-magic-plugin/src/setup_ci.rs:84`; `crates/ss-magic-plugin/src/checklist/verbs.rs:111`; `crates/ss-magic-plugin/src/compact_window.rs:132` (Rust partition) |
| L4 | efficiency / copy-paste-variant | `scripts/build-plugin-zip.py:913-1019` (`_version_repo`) | A single ~106-line fixture builder writes 8 files by hand (two `Cargo.toml`s, `Cargo.lock`, `dist-workspace.toml`, `README.md`, `plugin.json`, `ss-magic-plugin.version`, `hooks.json`, `marketplace.json`) with ad hoc string formatting for each, to exercise `check_versions`/`check_hooks_shim`/`check_workspace_shape`. It is the only caller-built fixture of this shape in the file (used by ~7 of the 13 selftest sections). Worth checking whether smaller, section-scoped fixtures would be easier to read/maintain than one monolithic one with 6 keyword-only knobs (`cli`, `plugin`, `readme_pin`, `bad_hook`, `plugin_deps`, `artifact_in_dist`). | itself only – no external comparison; flagged for maintainability, not correctness |
| L5 | naming | `scripts/build-plugin-zip.py:195-196` (`digest_of`) | One-line wrapper around `hashlib.sha256(data).hexdigest()` with no other logic; kept as a named function only so `check_pin`/`check_bump`/`main`/selftest share the exact same call shape (5 call sites). Not a bug, but note for the reviewer: this is the *only* place in the file hashing happens, so there is no risk of a second, drifted hash routine appearing in this file – mentioned here only because `index/consts.md` and the sha256 hand-roll in Rust's `hashing.rs` are pinned as intentionally separate implementations (structure-pins.md); do not propose merging them. | `crates/ss-magic-core/src/hashing.rs` (Rust partition; pinned, not a finding) |
| L6 | dead-code (verify) | `scripts/build-plugin-zip.py:1272-1273` (`--plugin-dir` argparse option) and `1276-1277` (`--out`/`--print-digest` combo) | `--plugin-dir` and `--out` are defined and used inside `main` (1303-1304, 1354) but never invoked anywhere else in the repository (CI, docs, Cargo.toml) – see Cross-module references. They may be intentionally kept only for local/manual debugging (the docstring's own Usage block at 44-51 does show `--out FILE`), so this is worth a reviewer glance rather than a confident dead-code claim. | `crates/ss-magic-plugin/Cargo.toml:65` and `.github/workflows/ci.yml:307` (the only two real invocations, both bare) |
| L7 | copy-paste-variant | `scripts/build-plugin-zip.py:793-798` (nested `git()` closure inside `check_bump`) | A tiny local `git(*args) -> subprocess.run(["git","-C",str(root),*args], capture_output=True, text=False)` closure exists only inside `check_bump`; it is not reused by `--selftest`'s AE79/AE80 sections (which never shell out) but is structurally the same "run git with capture, no exception on failure" shape used throughout the Rust `git/mod.rs::git_raw` helper described in CLAUDE.md. Cross-language, so not mergeable, but worth confirming no second Python helper of this shape exists (none found by grep) before assuming it is fine as a one-off local closure. | `crates/ss-magic-core/src/git/mod.rs` (`git_raw`, Rust partition; different language, informational only) |
