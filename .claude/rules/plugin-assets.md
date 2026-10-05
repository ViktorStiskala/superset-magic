---
paths:
  - "plugin/**"
  - "scripts/**"
  - ".claude-plugin/**"
  - ".github/**"
  - "dist-workspace.toml"
  - "docs/runbooks/**"
---

### Non-Rust assets

`plugin/` (the packaged marketplace tree; its `skills/` holds the four
skills – `scratchpad`, `operator-checklist`, `setup-github-ci` and
`migrate-repository`, the last of which moves a repository's hand-written
Markdown operator checklist onto the plugin for the current branch only,
through the checklist verbs and five confirmation gates, with a
`reference.md` and a worked `example.md` beside its `SKILL.md`; every skill
spells a command `ss-magic-plugin <verb>` and names no
`${CLAUDE_PLUGIN_DATA}`), `.claude-plugin/marketplace.json` (the digest pin),
`scripts/build-plugin-zip.py` (the reproducible builder and the release
assertions), `scripts/test-bootstrap.sh` (the bootstrap's failure-path suite),
`assets/workflow/checklist.yml` (embedded by the plugin crate's
`setup_ci.rs`; it installs `ss-magic-plugin` from an `ss-magic-plugin-v…`
release, its env var is `SS_MAGIC_PLUGIN_VERSION`, and it verifies and
renders only the checklists the pull request adds or changes, passed to the
verbs as explicit arguments – the plugin map's `setup_ci.rs` bullet has the
selection step), `assets/workflow/legacy/` (byte-exact copies of every
earlier released template generation, the fixtures behind `setup_ci.rs`'s
`LEGACY_TEMPLATES`; never edited, only added to when a release changes the
template), `.gitattributes` (line-ending pinning for the digest),
`scripts/mark-latest.sh` + `scripts/test-mark-latest.sh` +
`.github/workflows/mark-latest.yml` (the post-announce latest-mark step; the
procedure is the `releases/latest` bullet in
[conventions.md](./conventions.md)), `scripts/check-docs.sh` (the seven
documentation guards – retired subcommand spelling, no
`CLAUDE_PLUGIN_DATA` in a skill, the README installer pin, a self-contained
BUGBOT, resolving relative links, rule-file frontmatter and the 50,000-byte
always-loaded budget – plus a `--selftest` over fixture trees; bash 3.2,
run by CI's `plugin` job and again under `/bin/bash` on the macOS leg; the
checks list in [conventions.md](./conventions.md) describes each),
`scripts/lib/test-harness.sh` (the assertion helpers sourced by
`test-bootstrap.sh`, `test-mark-latest.sh` and `check-docs.sh --selftest`),
and
`docs/runbooks/forge-tag-and-release-protection.md` (the tag ruleset and
release immutability on the forge – applied 2026-08-31 and verified against the
live repository 2026-09-08; the ruleset also carries `required_signatures`,
which checks the tagged COMMIT's signature, not the tag object's, and two
disposable non-version tags from the proofs exist permanently).

Four shell pieces are worth knowing about, because all are load-bearing and
none is Rust. `plugin/hooks/bootstrap.sh` installs the pinned binary into
`${CLAUDE_PLUGIN_DATA}` – never `${CLAUDE_PLUGIN_ROOT}`, which is version-scoped
and replaced wholesale on each plugin update. When `${CLAUDE_PLUGIN_DATA}` is
unset (a manual run), the bootstrap falls back to
`${CLAUDE_CONFIG_DIR:-$HOME/.claude}/plugins/data/ss-magic-ss-magic`, the
harness's documented layout for the `ss-magic@ss-magic` plugin. `status`
(`status::locate_data_dir`) checks `${CLAUDE_PLUGIN_DATA}` first, then the
bootstrap's `data-root` pointer file, and only then the same
`${CLAUDE_CONFIG_DIR:-$HOME/.claude}/plugins/data/ss-magic-ss-magic` layout
(reporting the location as missing when neither `CLAUDE_CONFIG_DIR` nor `HOME`
is set). The bootstrap has no `set -e` and every path
ends in `exit 0`, prints NOTHING on stdout on success (a SessionStart hook's
stdout enters the model's context every session, so silence is a token-budget
rule), never touches an existing binary on a failing install, and fetches the
platform release ARCHIVE directly – `ss-magic-plugin-<triple>.tar.gz` from
`.../download/ss-magic-plugin-v$pin/`, resolving
`ss-magic-plugin-<triple>/ss-magic-plugin` inside it – verifying it against that
archive's published `.sha256` before extracting, and refusing the install unless
the staged binary's `--version` reports exactly the pin. It deliberately does NOT
fall back to piping `ss-magic-installer.sh` into a shell: the release publishes
`.sha256` siblings for the archives but not for the installer script, so a piped
installer would be the one executed artifact no published digest covers. A
`seed_config` helper invokes `"$bin_path" seed-config`, discarding both streams
and the exit status, and is called from every point where a usable pinned binary
is known to exist – the already-installed fast path, the re-check under the
install lock, and the end of a fresh install. This is R3a, the pre-seeding of
the `plugin` block into `magic.json` so the gate defaults are visible in the
tracked file. It is called from three sites rather than one because the binary
is installed once per MACHINE while the block must be seeded once per
REPOSITORY: a call sited only after a fresh install would seed the first
repository and skip every later one, since `already_installed && exit 0`
returns above it on every subsequent session. The
once-ness lives in the binary (it writes only when there is no `plugin` key at
all), not in the caller, so repeat calls cost a read and an exit and need no
marker file. `scripts/test-bootstrap.sh` pins both halves: a second repository on
an already-provisioned machine IS seeded, and a failed install (or a failed
upgrade over a working install) seeds nothing. There is deliberately NO cleanup of a stale
`${CLAUDE_PLUGIN_DATA}/bin/ss-magic` (the binary's name before the plugin became
its own crate): nothing spawns it, since `hooks.json` and the wrapper both name
`bin/ss-magic-plugin`, so it is inert, and
a bootstrap that deletes files is a new failure mode on a path that must never
fail a session. `plugin/hooks/run-hook.sh` is the shim the five EVENT
hooks are spawned through – every entry in `hooks.json` except the `SessionStart`
bootstrap, which must keep naming `bootstrap.sh` directly, because the shim does
nothing when the binary is absent and the bootstrap is the thing that installs
it; routing it through the shim would leave the plugin inert forever rather than
for one session. The indirection is the whole point:
`${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin` does not exist until the bootstrap
fetches it, and hooks on one event fire CONCURRENTLY, so a manifest naming the
binary directly makes the harness `posix_spawn` a missing path and the session
dies with ENOENT on a first install. The binary is fetched at runtime, so
naming `ss-magic-plugin` in `hooks.json` reproduces exactly that failure, and
`--check`'s `hooks spawn through the shim` line plus `test-bootstrap.sh` assert
that over every entry (a deliberate duplicate; change both or neither).
R77, the requirement that every hook be INERT for a session whose binary is
missing, is what the shim implements. The binary
implements that fail-open itself (`hook::run` has no non-zero exit path), but that
code is unreachable when the binary is the missing thing, so the guarantee has to
live in a script that ships inside `${CLAUDE_PLUGIN_ROOT}` and therefore always
exists. It is silent on BOTH streams – unlike the `plugin/bin/ss-magic-plugin`
wrapper described at the end of this section, which explains
itself on stderr – because `PreToolUse` fires on nearly every tool call, and
because a `SessionStart` hook's stdout enters the model's context. It shares
`lib/tmproot.sh` with the bootstrap and the wrapper rather than reimplementing the
handoff lookup. `plugin/lib/execguard.sh` holds
`ss_magic_is_loadable_executable`, the single answer to "will the kernel actually
run this file", sourced by BOTH the shim and the wrapper, so the two callers
cannot hold different tests. It is not a bare `[ -x ]` because `-x` alone is true for a directory
carrying the search bit, and it cannot see execve failing on the way in, most
dangerously ENOEXEC, where bash does not report a failure at all but REINTERPRETS
a damaged binary as a shell script and exits with whatever those bytes parse to
(measured over 30 corrupted binaries on bash 3.2: exit 2 about half the time, and
exit 2 from `PreToolUse` means BLOCK the tool call). It matches on the magic number (ELF, Mach-O 32/64 and
universal, or `#!`), and the two failure directions are deliberately opposite: an
unrecognised FORMAT fails closed (refuse), while a missing `od` or `tr` fails OPEN
(proceed), because refusing there would silently disable the plugin on a machine
merely lacking a utility. Callers still set `shopt -s execfail` afterwards for the
exec failures no file test can see. `hooks/bootstrap.sh` deliberately does not use
it: at install time it runs the staged binary and refuses unless it reports the
pinned version, which is strictly stronger. `plugin/bin/ss-magic-plugin`
is the wrapper every skill invokes. It `exec`s the installed binary with argv
passed through VERBATIM – no `plugin` verb is injected, because
the binary's own argv starts at the verb, so `ss-magic-plugin checklist list` is
exactly what the binary sees. Its name is deliberately `ss-magic-plugin`
rather than `ss-magic`: a wrapper called `ss-magic` would resolve
non-deterministically against a user's own install, handing a skill the sync
CLI's update gate and TUI. It finds the binary through a durable handoff file
(`data-root`) under the R80 temp root – the private per-machine
`<base>/ss-magic-plugin/<identifier>/` directory that `lib/tmproot.sh` resolves,
trying `/tmp` as the base first and then `$TMPDIR` –
because `${CLAUDE_PLUGIN_DATA}` is exported to hook and
MCP processes but NOT to the Bash tool.
