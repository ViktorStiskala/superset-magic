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

`plugin/` (the packaged marketplace tree), `.claude-plugin/marketplace.json`
(the digest pin), `scripts/build-plugin-zip.py` (the reproducible builder and
the release assertions), `scripts/test-bootstrap.sh` (the bootstrap's
failure-path suite), `assets/workflow/checklist.yml` (embedded by the plugin crate's `setup_ci.rs`;
it installs `ss-magic-plugin` from an `ss-magic-plugin-v…` release and its env
var is `SS_MAGIC_PLUGIN_VERSION`), `.gitattributes` (line-ending pinning for
the digest), `scripts/mark-latest.sh` + `scripts/test-mark-latest.sh` +
`.github/workflows/mark-latest.yml` (the post-announce latest-mark step, see
Build), `scripts/lib/test-harness.sh` (the assertion helpers both shell suites
source), and
`docs/runbooks/forge-tag-and-release-protection.md` (the tag ruleset and
release immutability on the forge – applied 2026-08-31 and verified against the
live repository 2026-09-08; the ruleset also carries `required_signatures`,
which checks the tagged COMMIT's signature, not the tag object's, and two
disposable non-version tags from the proofs exist permanently).

Four shell pieces are worth knowing about, because all are load-bearing and
none is Rust. `plugin/hooks/bootstrap.sh` installs the pinned binary into
`${CLAUDE_PLUGIN_DATA}` – never `${CLAUDE_PLUGIN_ROOT}`, which is version-scoped
and replaced wholesale on each plugin update. It has no `set -e` and every path
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
install lock, and the end of a fresh install – which is R3a's pre-seeding of the
`plugin` block. Calling it from three sites rather than one is the correction of
a real defect: the binary is installed once per MACHINE while the block must be
seeded once per REPOSITORY, so a call sited only after a fresh install seeded
the first repository and silently skipped every later one, since
`already_installed && exit 0` returns above it on every subsequent session. The
once-ness lives in the binary (it writes only when there is no `plugin` key at
all), not in the caller, so repeat calls cost a read and an exit and need no
marker file. `scripts/test-bootstrap.sh` pins both halves: a second repository on
an already-provisioned machine IS seeded, and a failed install (or a failed
upgrade over a working install) seeds nothing. There is deliberately NO cleanup of a stale
pre-split `${CLAUDE_PLUGIN_DATA}/bin/ss-magic`: nothing spawns it once
`hooks.json` and the wrapper both name `bin/ss-magic-plugin`, so it is inert, and
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
dies with ENOENT on a first install. The binary having its own name changes
NOTHING here – it is still fetched at runtime, so naming `ss-magic-plugin` in
`hooks.json` reproduces exactly the failure the shim exists to prevent, and
`--check`'s `hooks spawn through the shim` line plus `test-bootstrap.sh` assert
that over every entry (a deliberate duplicate; change both or neither).
R77 specifies the opposite – every hook INERT for that session. The binary
implements that fail-open itself (`hook::run` has no non-zero exit path), but that
code is unreachable when the binary is the missing thing, so the guarantee has to
live in a script that ships inside `${CLAUDE_PLUGIN_ROOT}` and therefore always
exists. It is silent on BOTH streams – unlike the wrapper below, which explains
itself on stderr – because `PreToolUse` fires on nearly every tool call, and
because a `SessionStart` hook's stdout enters the model's context. It shares
`lib/tmproot.sh` with the bootstrap and the wrapper rather than reimplementing the
handoff lookup. `plugin/lib/execguard.sh` holds
`ss_magic_is_loadable_executable`, the single answer to "will the kernel actually
run this file", sourced by BOTH the shim and the wrapper. It exists as one file
because duplicating it is what went wrong: `[ -x ]` alone is true for a directory
carrying the search bit, and it cannot see execve failing on the way in - most
dangerously ENOEXEC, where bash does not report a failure at all but REINTERPRETS
a damaged binary as a shell script and exits with whatever those bytes parse to
(measured over 30 corrupted binaries on bash 3.2: exit 2 about half the time, and
exit 2 from `PreToolUse` means BLOCK the tool call). The check was added to the
shim first and the wrapper kept the weaker test, which is precisely the drift a
shared definition prevents. It matches on the magic number (ELF, Mach-O 32/64 and
universal, or `#!`), and the two failure directions are deliberately opposite: an
unrecognised FORMAT fails closed (refuse), while a missing `od` or `tr` fails OPEN
(proceed), because refusing there would silently disable the plugin on a machine
merely lacking a utility. Callers still set `shopt -s execfail` afterwards for the
exec failures no file test can see. `hooks/bootstrap.sh` deliberately does not use
it - at install time it runs the staged binary and refuses unless it reports the
pinned version, which is strictly stronger. `plugin/bin/ss-magic-plugin`
is the wrapper every skill invokes. It `exec`s the installed binary with argv
passed through VERBATIM – there is no `plugin` verb to inject any more, because
the binary's own argv starts at the verb, so `ss-magic-plugin checklist list` is
exactly what the binary sees. Its name is still deliberately `ss-magic-plugin`
rather than `ss-magic`: a wrapper called `ss-magic` would resolve
non-deterministically against a user's own install, handing a skill the sync
CLI's update gate and TUI. It finds the binary through a durable handoff file
under the R80 temp root, because `${CLAUDE_PLUGIN_DATA}` is exported to hook and
MCP processes but NOT to the Bash tool.
