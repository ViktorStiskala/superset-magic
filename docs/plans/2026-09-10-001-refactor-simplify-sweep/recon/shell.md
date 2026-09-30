# Recon: shell partition

Packaged plugin scripts, mark-latest, and the two bash test suites.

## Map

### `plugin/hooks/bootstrap.sh` (503 lines)
SessionStart hook: install the pinned `ss-magic-plugin` binary into
`${CLAUDE_PLUGIN_DATA}/bin/`, or leave the machine untouched; never a non-zero
exit, never stdout on success.

- `cleanup()` (private) – `()` – removes the staging dir; installed as the `EXIT` trap.
- `give_up(msg)` (private) – the one failure exit: drops the success marker, prints one stderr line, `exit 0`.
- `write_line(path, line)` (private) – writes one line to a file, both streams suppressed, braced so a failed redirect itself stays silent.
- `publish_data_root()` (private) – writes the resolved `$data` into the R80 handoff file (`data-root`) via temp-file + `mv`, on every run.
- `installed_version()` (private) – runs `"$bin_path" --version`, takes the last field of line 1.
- `already_installed()` (private) – true iff the marker file's content and `installed_version` both equal the pin.
- `seed_config()` (private) – runs `"$bin_path" seed-config`, both streams and exit status discarded; called from 3 sites (pinned as deliberate, see structure-pins).
- `fetch(url, dest, max_time)` (private) – downloads via curl or wget, silent, selected once into `$downloader`.
- `digest_of(path)` (private) – SHA-256 of a file via `shasum -a 256` or `sha256sum`; near-duplicate of `tmproot.sh`'s `ss_magic_sha256_hex` (see Leads L1).
- Top-level vars used as constants: `RELEASE_DOWNLOAD_BASE`, `RELEASE_PAGE_BASE` (42-43), `CONNECT_TIMEOUT`, `ARCHIVE_MAX_TIME`, `DIGEST_MAX_TIME`, `LOCK_WAIT_SECONDS` (49-52), `bin_path`, `state_file`, `disclosed_marker`, `unsupported_marker` (108-111).
- Sources `lib/tmproot.sh` (113-117) tolerantly (`-r` check, errors swallowed).

### `plugin/hooks/run-hook.sh` (118 lines)
The indirection every non-bootstrap hook is spawned through: exec the pinned
binary as `hook <event>`, or do nothing at all (both streams silent) if it
cannot.

- `give_up()` (private) – `exit 0`, no message (silent by design – contrast with the wrapper's `explain`).
- Inline logic (48-78): resolve `$data` from `$CLAUDE_PLUGIN_DATA` or, failing that, source `tmproot.sh` and read the `data-root` handoff file with a braced `read -r` – byte-identical to `bin/ss-magic-plugin` lines 72-87 (see Leads L2).
- Sources `lib/execguard.sh` unconditionally (90-93) and calls `ss_magic_is_loadable_executable`.
- Ends with `shopt -s execfail; exec "$bin" hook "$event"` then a dead `exit 0` (118) that only runs if `exec` itself fails to even attempt (execfail covers the rest).

### `plugin/bin/ss-magic-plugin` (109 lines)
The wrapper every skill's Bash-tool command actually runs; resolves the
pinned binary via the same R80 handoff and execs argv verbatim.

- `explain(msg)` (private) – prints one stderr line, `exit 0` (contrast with `run-hook.sh`'s silent `give_up`, documented as deliberate: this is the person-facing path).
- Inline logic (54-99): same `$data` resolution as `run-hook.sh` (source `tmproot.sh`, read `data-root` handoff) – see Leads L2.
- Sources `lib/execguard.sh` (96-99), calls `ss_magic_is_loadable_executable`.
- `shopt -s execfail; exec "$bin" "$@"` then a reachable `explain` at EOF (109) – unlike `run-hook.sh`, this fallback line is live because `execfail` can return control here.

### `plugin/lib/tmproot.sh` (137 lines)
Sourced by both `bootstrap.sh` and `bin/ss-magic-plugin` (and, via
`run-hook.sh`'s fallback, there too): the R80 per-machine temp root, byte-for-byte
mirroring `crates/ss-magic-plugin/src/tmproot.rs`.

- `SS_MAGIC_NAMESPACE_DIR` = `"ss-magic-plugin"` (38) – mirrors Rust `NAMESPACE_DIR`.
- `SS_MAGIC_INSTALL_LOCK_NAME` = `"install.lock"` (42) – mirrors Rust `INSTALL_LOCK_NAME`.
- `SS_MAGIC_DATA_ROOT_FILE` = `"data-root"` (48) – mirrors Rust `status.rs::DATA_ROOT_FILE`.
- `ss_magic_sha256_hex()` (public, no namespace prefix conflict) – `()` reading stdin – SHA-256 of stdin, lowercase hex, via `shasum`/`sha256sum`.
- `ss_magic_identifier()` (public) – `()` – `sha256($HOME)`'s first 16 hex chars, validated as hex.
- `ss_magic_valid_component(path, euid)` (public) – true iff `path` is a real dir (no symlink), mode exactly 0700, owned by `euid`.
- `ss_magic_resolve_root([mode])` (public) – `mode` = `"create"` or read-only – tries `/tmp` then `$TMPDIR`, validates/creates `<base>/ss-magic-plugin/<id>/`, prints the winning path.

### `plugin/lib/execguard.sh` (66 lines)
Sourced by `run-hook.sh` and `bin/ss-magic-plugin`: the one "will exec actually
run this file" test, replacing a bare `[ -x ]`.

- `ss_magic_is_loadable_executable(path)` (public) – true iff `path` is a regular executable file whose first 4 bytes match a known magic number (ELF, Mach-O 32/64/universal, or `#!`); fails open when `od`/`tr` are missing.

### `scripts/mark-latest.sh` (135 lines)
Post-announce job: give the repository's `releases/latest` mark back to the
newest CLI (`vX.Y.Z`) release after a plugin release (`ss-magic-plugin-vX.Y.Z`)
took it.

- `note(msg)` (private) – one stderr line, prefixed `mark-latest: `.
- `is_bare_cli_tag(tag)` (private) – true iff `tag` matches `^v[0-9]+\.[0-9]+\.[0-9]+$`.
- `newest_cli_tag_in()` (private, reads stdin) – filters a flattened `gh release list --json` stream to non-draft/non-prerelease bare CLI tags, sorts numerically on the 3 components, prints the greatest with its `v` prefix restored.
- Top-level vars: `tag`, `repo` (default `ViktorStiskala/superset-magic`, 50), `dry_run` (49-51).

### `scripts/lib/test-harness.sh` (46 lines)
Sourced (never executed) by both bash test suites; the shared assertion
helpers, written for bash 3.2.

- `pass(label)` / `fail(label)` (private) – increment `$passed`/`$failed`, print on failure (or on `-v`).
- `assert_eq`, `assert_file_absent`, `assert_file_present`, `assert_empty_file`, `assert_contains`, `assert_contains_fixed`, `assert_lacks_fixed` (all private, all small, all generic – exactly the kind of helper a duplicate would re-implement inside a test suite instead of sourcing).

## Constants and literals

| Value | Location | Denotes | Elsewhere in workspace? |
|---|---|---|---|
| `RELEASE_DOWNLOAD_BASE="https://github.com/.../releases/download"` | `bootstrap.sh:42` | GitHub release download base URL | Not literal elsewhere; `release.rs` builds GitHub API URLs from `REPO_SLUG`, a different constant, same repo. |
| `RELEASE_PAGE_BASE="https://github.com/.../releases/tag"` | `bootstrap.sh:43` | GitHub release page base URL | Same repo slug as above, no shared constant across languages. |
| `CONNECT_TIMEOUT=8` | `bootstrap.sh:49` | curl/wget connect timeout (s) | Not found elsewhere in literals.md/numbers.md; comment ties it to the hook's 90s budget (`hooks.json:11`) without deriving from it. |
| `ARCHIVE_MAX_TIME=40` | `bootstrap.sh:50` | curl/wget max time for the archive fetch (s) | Same 90s-budget comment; not derived programmatically. |
| `DIGEST_MAX_TIME=15` | `bootstrap.sh:51` | curl/wget max time for the `.sha256` fetch (s) | Same. |
| `LOCK_WAIT_SECONDS=20` | `bootstrap.sh:52` | `flock -w` / perl `alarm` wait (s) | Same. Sum of all four (8+40+15+20=83, plus lock-wait can overlap fetches) is close to the hooks.json `90` timeout at `plugin/hooks/hooks.json:11` – nothing asserts the margin. |
| `90` | `plugin/hooks/hooks.json:11` | SessionStart hook timeout (s) | The four timers above are sized against this number by comment only (bootstrap.sh:45-48); no code or test cross-checks them (see Leads L3). |
| `bin_path="$data/bin/ss-magic-plugin"` | `bootstrap.sh:108` | installed binary path, relative to data dir | Same literal spelled independently at `run-hook.sh:80` and `ss-magic-plugin:89`; the Rust side has this as `BINARY_REL = "bin/ss-magic-plugin"` at `crates/ss-magic-plugin/src/status.rs:129`. Owning definition is arguably the Rust const; the 3 shell files each spell it inline (see Leads L4). |
| `state_file="$data/.ss-magic-installed"` | `bootstrap.sh:109` | success marker (per-machine) | Not spelled elsewhere; the `.ss-magic-` prefix is intentionally kept even though the binary is `ss-magic-plugin` now (comment 103-107). |
| `disclosed_marker="$data/.ss-magic-disclosed"` | `bootstrap.sh:110` | one-time-disclosure marker | Not spelled elsewhere. |
| `unsupported_marker="$data/.ss-magic-unsupported"` | `bootstrap.sh:111` | unsupported-platform marker | Not spelled elsewhere. |
| `SS_MAGIC_NAMESPACE_DIR="ss-magic-plugin"` | `tmproot.sh:38` | R80 namespace dir name | Mirrors Rust `NAMESPACE_DIR` (`crates/ss-magic-plugin/src/tmproot.rs:89`, per literals.md). Intentional cross-language mirror (file header states this), not a stray duplicate. |
| `SS_MAGIC_INSTALL_LOCK_NAME="install.lock"` | `tmproot.sh:42` | install lock filename | Mirrors Rust `INSTALL_LOCK_NAME` (`tmproot.rs:97`). Same as above. |
| `SS_MAGIC_DATA_ROOT_FILE="data-root"` | `tmproot.sh:48` | handoff filename | Mirrors Rust `status.rs::DATA_ROOT_FILE` (`status.rs:144`). Same as above. |
| `0700` / mode check | `tmproot.sh:100` (`[ "$mode" = "700" ]`), `tmproot.sh:126,130` (`mkdir -m 0700`) | required/created directory mode for R80 components | Mirrors Rust `DIR_MODE = 0o700` (`tmproot.rs:101`), which is itself one of 8 identically-named `DIR_MODE` consts across the plugin crate (bypass.rs, cache.rs, expect_artifact.rs, heartbeat.rs, hook/subagent_stop.rs, ledger.rs, scratchpad.rs, tmproot.rs – all `0o700`, flagged `dup-name` in consts.md). Cross-partition lead, not actionable in shell alone. |
| `16` (hex chars) | `tmproot.sh` via `IDENTIFIER_HEX_LEN` semantics, `cut -c1-16` at line 70 | length of the per-machine identifier | Mirrors Rust `IDENTIFIER_HEX_LEN = 16` (`tmproot.rs:105`). Shell has no named constant for it – it is the literal `1-16` inline (see Leads L5). |
| `aarch64`/`x86_64` × `apple-darwin`/`unknown-linux-gnu` | `bootstrap.sh:275-282` (case arms) | supported install targets | Same 4-triple set is the `targets = [...]` list in `dist-workspace.toml:13`. No shared source of truth between the cargo-dist manifest and this case statement (see Leads L6). |
| `ss-magic-plugin-v$pin` / `ss-magic-plugin-$triple.tar.gz` | `bootstrap.sh:414-415` | release tag / asset name shape | Matches the plugin release-line naming documented in CLAUDE.md and asserted by `build-plugin-zip.py --check`; not duplicated in shell beyond this one construction site. |
| checksum hex-char case pattern (64 `[0-9a-f]` alternations) | `bootstrap.sh:425` | validates a SHA-256 hex digest shape | Same shape-validation idea (though different mechanism: char-class glob here vs `case`) is not repeated elsewhere in shell; `tmproot.sh:72` validates a 16-char hex identifier with the same glob-repetition style (see Leads L7). |
| `--limit 200` | `mark-latest.sh:76` | page size for `gh release list` | Not found duplicated; comment explains why 200 (vs `gh`'s default 30). No relation to `release.rs`'s `RELEASES_PER_PAGE` (100, per `release.rs:530` template) – different APIs (`gh` CLI vs GitHub REST), same shape of "bounded fetch" decision, not a shared constant. |
| `ViktorStiskala/superset-magic` | `mark-latest.sh:50` | default repo slug | Matches `bootstrap.sh`'s URL bases (42-43) textually (same owner/repo) but as a full URL there, not a shared literal; also appears in `release.rs`'s `REPO_SLUG` template per functions/consts context. Three independent spellings of the same repo identity across languages. |
| `^v[0-9]+\.[0-9]+\.[0-9]+$` | `mark-latest.sh:59` | bare-CLI-tag shape | Same shape (MAJOR.MINOR.PATCH) is validated in `bootstrap.sh:166-174` via two `case` glob checks instead of a regex – two different mechanisms for the same shape rule, in sibling scripts of the same repo (see Leads L8). |

## Cross-module references

**What these files import:**
- `bootstrap.sh` sources `lib/tmproot.sh` (tolerant `-r` check; failures swallowed, `113-117`).
- `run-hook.sh` sources `lib/tmproot.sh` (fallback path only, `59-62`) and `lib/execguard.sh` (unconditional, `90-93`).
- `bin/ss-magic-plugin` sources `lib/tmproot.sh` (unconditional, `64-67`) and `lib/execguard.sh` (`96-99`).
- `scripts/test-bootstrap.sh` and `scripts/test-mark-latest.sh` each source `scripts/lib/test-harness.sh` (per literals.md: both cite `$REPO_ROOT/scripts/lib/test-harness.sh`).
- `mark-latest.sh` sources nothing; self-contained apart from `gh`.
- None of these shell files import anything from the Rust crates directly – the relationship is behavioral mirroring (documented in each file's header), not code sharing.

**Where public symbols are used elsewhere:**

- `ss_magic_resolve_root` (`tmproot.sh:116`) – called at `bootstrap.sh:136,319`, `run-hook.sh:66`, `bin/ss-magic-plugin:77`. All 4 call sites guarded by `command -v` or preceded by sourcing; no other file.
- `ss_magic_identifier` (`tmproot.sh:67`) – used only internally, inside `ss_magic_resolve_root` (`tmproot.sh:118`). Unused outside this file.
- `ss_magic_valid_component` (`tmproot.sh:91`) – used only internally, inside `ss_magic_resolve_root` (`tmproot.sh:127,131`). Unused outside this file.
- `ss_magic_sha256_hex` (`tmproot.sh:54`) – used only internally, inside `ss_magic_identifier` (`tmproot.sh:69`). Unused outside this file – notably NOT called from `bootstrap.sh`'s own `digest_of`, which re-implements the same shasum/sha256sum branching (Leads L1).
- `ss_magic_is_loadable_executable` (`execguard.sh:46`) – called at `run-hook.sh:95` and `bin/ss-magic-plugin:101`. Not called from `bootstrap.sh` (deliberate – see structure-pins: bootstrap has a stronger check, running `--version`).
- `SS_MAGIC_NAMESPACE_DIR` / `SS_MAGIC_INSTALL_LOCK_NAME` / `SS_MAGIC_DATA_ROOT_FILE` – all three referenced only inside `tmproot.sh` itself plus the three caller scripts (`bootstrap.sh:137,320`; `run-hook.sh:67`; `ss-magic-plugin:79`); no other shell file reads them directly (test-bootstrap.sh may assert on the resulting paths but not the variable names – not verified line-by-line here).
- `mark-latest.sh`'s `is_bare_cli_tag` and `newest_cli_tag_in` – private, used only within the same file; `scripts/test-mark-latest.sh` exercises them indirectly by running the whole script, not by sourcing the functions.
- `scripts/lib/test-harness.sh`'s assert helpers – used throughout `scripts/test-bootstrap.sh` and `scripts/test-mark-latest.sh` (not individually enumerated; every assertion in both 949/225-line suites goes through one of these 7 functions).

## Leads

| # | category | file:line(s) | hypothesis | compare against |
|---|---|---|---|---|
| L1 | duplication | `plugin/hooks/bootstrap.sh:390-398` vs `plugin/lib/tmproot.sh:54-62` | `digest_of()` re-implements the shasum/sha256sum tool-detection branch that `ss_magic_sha256_hex()` already provides (which `bootstrap.sh` already sources); `digest_of "$f"` could be `ss_magic_sha256_hex <"$f"` (or a one-line wrapper) instead of duplicating the `command -v` chain. | `plugin/lib/tmproot.sh:54-62` (`ss_magic_sha256_hex`) |
| L2 | duplication | `plugin/hooks/run-hook.sh:57-78` vs `plugin/bin/ss-magic-plugin:72-87` | The "`$data` from env, else source tmproot.sh, resolve root, read `data-root` handoff with a braced `read -r`" block is near byte-identical (differs only in the error-message text at each failure point). A shared function in `lib/tmproot.sh` (e.g. `ss_magic_resolve_data_root`) could return the resolved `$data` or fail, leaving only the message-text choice to each caller. | same two files, side by side |
| L3 | hardcoded-value | `plugin/hooks/bootstrap.sh:45-52` vs `plugin/hooks/hooks.json:11` | The four timing constants (`CONNECT_TIMEOUT`, `ARCHIVE_MAX_TIME`, `DIGEST_MAX_TIME`, `LOCK_WAIT_SECONDS`) are sized "to fit inside the 90s hook timeout" by comment only; nothing asserts `8+40+15+20 <= 90` (with margin for lock contention). A drift in either number silently breaks the stated invariant. | `plugin/hooks/hooks.json:11` (`"timeout": 90`) |
| L4 | const-location | `plugin/hooks/bootstrap.sh:108`, `plugin/hooks/run-hook.sh:80`, `plugin/bin/ss-magic-plugin:89` | The literal `bin/ss-magic-plugin` path segment is spelled independently in 3 shell files; the Rust side already names it `BINARY_REL` (`crates/ss-magic-plugin/src/status.rs:129`). Shell cannot import a Rust const, but the 3 shell spellings could at least share one (e.g. via `tmproot.sh`, which all 3 already source). | `crates/ss-magic-plugin/src/status.rs:129` |
| L5 | hardcoded-value | `plugin/lib/tmproot.sh:70` (`cut -c1-16`) | The identifier length `16` is a bare literal in the `cut` invocation, with no named constant in this file, while the Rust mirror names it `IDENTIFIER_HEX_LEN` (`tmproot.rs:105`). Extracting a shell variable (`SS_MAGIC_IDENTIFIER_HEX_LEN=16`) alongside the other 3 named constants in this file would make the mirror complete and the number self-documenting. | `crates/ss-magic-plugin/src/tmproot.rs:105` |
| L6 | hardcoded-value | `plugin/hooks/bootstrap.sh:274-282` vs `dist-workspace.toml:13` | The 2x2 platform matrix (`Darwin`/`Linux` × `arm64,aarch64`/`x86_64,amd64`) is a `case` statement here and a `targets = [...]` list in `dist-workspace.toml`. Adding or removing a cargo-dist target requires a human to remember to update this unrelated shell file too; nothing links them. | `dist-workspace.toml:13` |
| L7 | copy-paste-variant | `plugin/hooks/bootstrap.sh:425` vs `plugin/lib/tmproot.sh:72` | Both validate a hex string via a long repeated `[0-9a-f]` `case` glob (64 repetitions vs 16). Same idiom, different length, independently maintained; a shared helper (e.g. `ss_magic_is_hex(str, len)`) would remove one of the two hand-counted glob patterns. Low priority (both are already correct and rarely touched). | `plugin/lib/tmproot.sh:72` |
| L8 | copy-paste-variant | `scripts/mark-latest.sh:59` vs `plugin/hooks/bootstrap.sh:166-174` | Both validate "is this string exactly `v`-optional + MAJOR.MINOR.PATCH", one via `grep -Eq` regex, the other via two `case` glob passes (justified in-file: "glob patterns cannot count"). `mark-latest.sh` already depends on `gh`, so it could use the same glob-based approach for consistency, or `bootstrap.sh`'s comment about avoiding `grep -E` could be revisited if `grep` is already a hard dependency elsewhere in the install path (it is not, currently). Informational; likely not worth changing given the differing tool-availability constraints. | `plugin/hooks/bootstrap.sh:159-174` |
| L9 | dead-code / efficiency | `plugin/hooks/run-hook.sh:117-118` | `exec "$bin" hook "$event"` is followed by `exit 0` (118) that is unreachable unless `exec` fails to even be attempted (with `execfail` set, a genuine exec failure returns control here rather than crashing, so the line is reachable only in that narrow case – confirm this is intended documentation-as-code rather than truly dead). Compare with `bin/ss-magic-plugin:109`, which puts a real `explain(...)` call in the equivalent slot instead of a bare `exit 0` – the two scripts handle the "exec returned control" case with different levels of diagnostic even though both share the identical `execfail` justification comment. | `plugin/bin/ss-magic-plugin:108-109` |
| L10 | naming | `plugin/hooks/bootstrap.sh:69` (`give_up`) vs `plugin/hooks/run-hook.sh:44` (`give_up`) vs `plugin/bin/ss-magic-plugin:49` (`explain`) | Three sibling scripts, three different names/behaviors for "the one early-exit helper": `give_up` in bootstrap prints + clears the state file, `give_up` in run-hook.sh is silent, `explain` in the wrapper prints. The two `give_up`s share a name but not a body; the wrapper's near-twin has a different name. Not proposing unification (documented as deliberate posture difference in CLAUDE.md), but the naming is worth a second look: `run-hook.sh`'s silent `give_up` and `bootstrap.sh`'s message-printing `give_up` are easy to conflate when reading both files, given the identical name for different contracts. | `plugin/hooks/bootstrap.sh:69`, `plugin/bin/ss-magic-plugin:49` |
| L11 | reuse | `plugin/lib/tmproot.sh:91-103` (`ss_magic_valid_component`) | This is a small, generic, already-shared helper (exactly the kind called out as a good candidate) – flagged here only to confirm it has no shell sibling anywhere else that could instead be collapsed into it; none found in this partition. No action needed; recorded so the reviewer does not re-flag it as an unshared duplicate. | n/a (confirms no finding) |
| L12 | duplication (cross-partition, informational) | `plugin/lib/tmproot.sh:38,42,48,100` | These 4 shell constants mirror 4 Rust constants byte-for-byte by design (file header states the contract explicitly). Listed so the reviewer does not flag the mirror itself as a duplication; the only actionable angle is whether a test asserts the mirror stays in sync – `scripts/test-bootstrap.sh` was not read function-by-function in this recon; worth checking there for such an assertion before proposing one. | `crates/ss-magic-plugin/src/tmproot.rs:89,97,101` and `crates/ss-magic-plugin/src/status.rs:144` |
| L13 | duplication (cross-partition, informational) | `crates/ss-magic-plugin/src/*.rs` (8 files, per `index/consts.md` `DIR_MODE` rows) | Eight separate `DIR_MODE = 0o700` consts across the Rust plugin crate (bypass.rs, cache.rs, expect_artifact.rs, heartbeat.rs, hook/subagent_stop.rs, ledger.rs, scratchpad.rs, tmproot.rs) are flagged `dup-name` in the consts index. Out of scope for this shell partition to fix, but the shell mirror (`tmproot.sh:100,126,130`, all `700`/`0700`) means a 9th and 10th copy of the same magic number exist once shell is counted. Flag for whichever partition owns the Rust consts to consider a single shared `const DIR_MODE` in core. | `docs/plans/2026-09-10-001-refactor-simplify-sweep/index/consts.md` rows for `DIR_MODE` |
