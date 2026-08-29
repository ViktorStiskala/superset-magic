# Plugin assets – every file the plugin ships, verbatim

Companion to [the plan](../2026-08-29-001-feat-ss-magic-claude-plugin-plan.md). These are **committed
bytes**, not bytes any code renders. The plugin is a subdirectory of this repository that the
marketplace serves; `ss-magic` writes none of it, reads none of it at runtime, and has no `install`
verb (R66). The embed-and-render pipeline that once produced this tree is retired with KTD15.

The tree is JSON, Markdown and two small shell scripts. The behavior is the `ss-magic` binary, which
a `SessionStart` bootstrap installs into `${CLAUDE_PLUGIN_DATA}` at a pinned version (R70-R73).

## The committed layout

```plaintext
<repo root>/
├── .claude-plugin/
│   └── marketplace.json          # the distribution manifest; one git-subdir entry
└── plugin/                       # everything the harness installs, verbatim
    ├── .claude-plugin/
    │   └── plugin.json
    ├── ss-magic.version          # the binary pin – one version literal, no newline noise
    ├── hooks/
    │   ├── hooks.json
    │   └── bootstrap.sh
    ├── bin/
    │   └── ss-magic-plugin       # Bash-tool-reachable wrapper (R75)
    └── skills/
        ├── scratchpad/SKILL.md
        ├── operator-checklist/{SKILL.md,reference.md}
        └── setup-github-ci/SKILL.md
```

Only `plugin.json` goes inside `plugin/.claude-plugin/`. The docs call the alternative out
explicitly: *"Common mistake: Don't put `commands/`, `agents/`, `skills/`, or `hooks/` inside the
`.claude-plugin/` directory."* `skills/<name>/SKILL.md` is always scanned; `hooks/hooks.json` and
`bin/` are auto-discovered at the plugin root.

Two roots are easy to confuse and mean different things:

- `.claude-plugin/marketplace.json` sits at the **repository** root. It is the catalogue, never
  installed.
- `plugin/` is what the marketplace entry's `path` names, and is what lands on a user's machine as
  `${CLAUDE_PLUGIN_ROOT}`.

Skill names are deliberately **unprefixed** – the manifest `name` already becomes the invocation
prefix, so they read `/ss-magic:scratchpad`, `/ss-magic:operator-checklist` and
`/ss-magic:setup-github-ci` rather than stuttering as `/ss-magic:ss-scratchpad`.

## `plugin/.claude-plugin/plugin.json`

Only `name` is required – confirmed against the real minimal manifest shipped by
`security-guidance/2.0.7`. `name` is the invocation prefix and must stay `ss-magic`.

```json
{
  "name": "ss-magic",
  "version": "0.10.0",
  "description": "Session scratchpad, context page-fault gate, operator checklist, and cost ledger for the Superset workspace contract",
  "repository": "https://github.com/ViktorStiskala/superset-magic",
  "license": "MIT",
  "author": { "name": "Viktor Stiskala" }
}
```

`version` tracks the crate version, and R95 makes that agreement a CI gate rather than a convention:
one release moves `Cargo.toml`, this manifest, `plugin/ss-magic.version`, the marketplace entry's
`ref`/`sha`, and the workflow pin together. The field carries **no** install semantics of its own:
*"Changing `version` in plugin.json doesn't flip existing user installations."* What actually decides
which binary runs is the pin file below, not this key.

`author` is an **object**, not a string – every plugin installed on this machine uses the object form
(`compound-engineering`, `cloudflare`, and the rest all carry `{"name": …}`), and the structure
reference documents `name`/`email`/`url` fields inside it.

## `plugin/ss-magic.version`

```plaintext
0.10.0
```

One bare `MAJOR.MINOR.PATCH` literal. This file, not `plugin.json`, is what the bootstrap compares
against the installed binary and what it interpolates into a release URL – so it is a supply-chain
boundary: whoever can write it decides what every installed machine downloads and executes at session
start. R71 therefore makes the bootstrap validate it against a strict version pattern **before** any
URL is composed, and R95 forbids advancing it in a commit that lands before the named release's
assets are published (a pin naming an unpublished release 404s, installs nothing, and fails silently
open).

It lives beside `plugin.json` inside the version-scoped plugin root, so a plugin update is what
triggers a binary update – the two can never drift.

## `plugin/hooks/hooks.json`

Every entry is in **exec form** and every path is a braced variable (KTD18, R74).

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup",
        "hooks": [
          {
            "type": "command",
            "command": "bash",
            "args": ["${CLAUDE_PLUGIN_ROOT}/hooks/bootstrap.sh"],
            "timeout": 90
          }
        ]
      },
      {
        "matcher": "startup|resume|clear|compact|fork",
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic",
            "args": ["plugin", "hook", "session-start"],
            "timeout": 10
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Read|Edit|Write|NotebookEdit|Grep|Glob|Bash",
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic",
            "args": ["plugin", "hook", "pre-tool-use"],
            "timeout": 5
          }
        ]
      }
    ],
    "PreCompact": [
      {
        "matcher": "manual|auto",
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic",
            "args": ["plugin", "hook", "pre-compact"],
            "timeout": 10
          }
        ]
      }
    ],
    "SubagentStop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic",
            "args": ["plugin", "hook", "subagent-stop"],
            "timeout": 10
          }
        ]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic",
            "args": ["plugin", "hook", "session-end"],
            "timeout": 10
          }
        ]
      }
    ],
    "FileChanged": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_DATA}/bin/ss-magic",
            "args": ["plugin", "hook", "file-changed"],
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

### Why each detail is what it is

- **`SessionStart` carries two matcher groups, and they are not interchangeable.** The bootstrap is
  restricted to `"matcher": "startup"` (R76) so it runs once per fresh session instead of again on
  every resume, clear, compaction and fork – there is nothing to reinstall on those. ss-magic's own
  `session-start` handler keeps all five sources, because `compact` is the signal it exists for: it
  is what re-injects orientation after the window was cleared (R19, F2). Collapsing these into one
  group breaks whichever half it is collapsed toward.
- **Exec form, not a shell string (KTD18).** Each argument is passed as a plain string, so a plugin
  path containing a quote, `$` or backtick never reaches a shell parser. It also removes the
  dependency on `bootstrap.sh`'s executable bit surviving distribution – which it does today, but
  fails silently when it does not, which is the worst failure shape available.
- **The braced form is mandatory.** Both `${CLAUDE_PLUGIN_ROOT}` and `${CLAUDE_PLUGIN_DATA}` are
  substituted by the harness inside an entry's `command` string and per-element inside `args`
  (measured on 2.1.251). The bare `$NAME` form is substituted **only** in shell form, so writing
  `$CLAUDE_PLUGIN_DATA/bin/ss-magic` here yields a literal path that does not exist.
- **Every entry declares an explicit `timeout`, in seconds.** The harness default is **600 seconds**.
  An undeclared timeout is therefore not "fast by default" – it is a ten-minute ceiling on a hook
  that hangs, paid in user-visible latency at session start and session exit. The measured worst-case
  ledger scan is 0.87 s against a 354.7 MiB transcript tree, so 10 s is generous for `session-end`;
  the bootstrap gets 90 s because it may fetch over a slow link, with the fetch itself separately
  time-bounded inside the script.
- **`PreToolUse` gets the shortest timeout (5 s)** because a timed-out `PreToolUse` hook silently
  does not block. Returning fast is the difference between a gate and a no-op.
- **The `PreToolUse` matcher is a LIST, not a regex – keep it that way.** Matcher syntax stays
  list-shaped only while it contains just alphanumerics, `_`, `-`, spaces, `,` and `|`. Any other
  character makes it an **unanchored regex**, where `Edit.*` would also match `NotebookEdit`. The
  list widened from `Read|Grep|Glob` to include `Edit`, `Write` and `NotebookEdit` for the checklist
  deny (R88, which covers writes as well as reads) and `Bash` for the advisory commit nudge (R91).
  `Grep`/`Glob` remain inert forward-compatibility entries. Every added name is alphanumeric, so the
  matcher is still a list.
- **`Bash` in that matcher does not reopen the Bash page-fault half.** R20 as narrowed forbids two
  things: a tool-input rewrite on any event, and any handler that reads or page-faults Bash *output*.
  An advisory handler that only emits `additionalContext` is permitted, and is all R91 uses.
- **`FileChanged` ships without a matcher, and filters inside the handler.** The `.env`/`.envrc`
  classification R92 needs is a path decision, not a tool-name decision, and the matcher semantics
  for this event are not measured on 2.1.251. Filtering in `file_changed.rs` keeps the decision in
  tested Rust and costs one string comparison on every other changed file.
- **`SubagentStop` and `SessionEnd` take no matcher** – those events support none.
- **The success path prints nothing.** A hook's stdout enters the model's context on every session
  start, so silence is a token-budget rule, not a style preference (R72). Diagnostics go to stderr,
  at most one line.

### `${CLAUDE_PLUGIN_DATA}` and `${CLAUDE_PLUGIN_ROOT}` – measured lifecycle

| | `${CLAUDE_PLUGIN_ROOT}` | `${CLAUDE_PLUGIN_DATA}` |
|---|---|---|
| what it holds | the installed `plugin/` tree, verbatim | the bootstrapped binary |
| across a plugin update | **replaced** – version-scoped path | survives |
| on uninstall | removed | **deleted**, unless `--keep-data` |
| created by | the plugin installer | the harness, automatically, mode 0755 |
| visible to the Bash tool | yes (its `bin/` is on `PATH`) | **no** |

`${CLAUDE_PLUGIN_DATA}` resolves to `~/.claude/plugins/data/<plugin>-<marketplace>/`. The separator
is a **hyphen, not `@`** – every character outside `[a-zA-Z0-9_-]` is replaced – so `ss-magic@ss-magic`
becomes:

```plaintext
~/.claude/plugins/data/ss-magic-ss-magic/
```

That is why `${CLAUDE_PLUGIN_ROOT}` is never the install target (R70): a plugin update would discard
the binary on every bump.

## `plugin/hooks/bootstrap.sh`

Full behavior is U23's; what the shipped file must contain, and why, is here.

1. **Compare and exit.** Read `${CLAUDE_PLUGIN_ROOT}/ss-magic.version`; compare against
   `"${CLAUDE_PLUGIN_DATA}/bin/ss-magic" --version` (R68 makes that flag short-circuit before the
   update gate and before the TUI, so it is safe to call with no TTY). On a match: exit 0, print
   nothing anywhere.
2. **Validate before composing a URL.** The pin must match `^[0-9]+\.[0-9]+\.[0-9]+$` (R71). A pin of
   `v1.2.3; rm -rf /` never reaches a URL.
3. **Lock.** Take the R80 lock under `/tmp/ss-magic-plugin/<frozen-identifier>/`, falling back to
   `$TMPDIR`, so two sessions starting at once produce one install and neither fails (AE61).
4. **Install into a staging directory, then move** (R73). Never install straight into the final path.
5. **Marker discipline.** Write the success marker only after the move succeeds, and remove it on any
   failure, so the next session retries rather than trusting a half-written tree.
6. **Never `set -e`.** Every path exits 0 (R72) – no network, DNS failure, proxy, 404, checksum
   mismatch, unwritable data directory, unsupported platform. At most one stderr line, never on
   stdout.
7. **No published target is a no-op, once.** Windows has no release target, so the script reports the
   reason once and does not repeat it on every session start (R78, AE66).
8. **First run on a machine discloses** the hooks being registered and the release the binary comes
   from; later runs are silent (R79, AE67).

### The install invocation, verified against the real v0.9.0 assets

The published `ss-magic-installer.sh` is cargo-dist 0.32.0 and resolves its install directory in a
fixed order. Reading the shipped script, the branch matters more than the directory:

| variable | layout | consequences |
|---|---|---|
| `SS_MAGIC_INSTALL_DIR` | `cargo-home` | binary at `<dir>/bin/`, PATH edit, receipt written to `${XDG_CONFIG_HOME:-~/.config}/ss-magic/`, bundled self-updater installed |
| `CARGO_DIST_FORCE_INSTALL_DIR` | `cargo-home` | identical to the above |
| `SS_MAGIC_UNMANAGED_INSTALL` | `flat` | binary directly at `<dir>/`, and it sets `NO_MODIFY_PATH=1` **and** `INSTALL_UPDATER=0` in one step – so no PATH edit and, because the receipt is written only when the updater is installed, no receipt at all |

Only the third is acceptable. A PATH edit from a session-start hook is unacceptable on its face, and
a second self-updater living inside a directory the plugin manager owns is exactly the drift R69's
pinning exists to prevent. So:

```sh
SS_MAGIC_UNMANAGED_INSTALL="$stage" sh "$installer"
# flat layout -> "$stage/ss-magic"; then move it to "${CLAUDE_PLUGIN_DATA}/bin/ss-magic"
```

The `--no-modify-path` **flag is deprecated** in favour of `SS_MAGIC_NO_MODIFY_PATH=1` (the script
says so on stderr when the flag is used); the unmanaged path sets it anyway, so neither is needed.

Published assets per release, confirmed on `v0.9.0`:

```plaintext
ss-magic-{aarch64,x86_64}-apple-darwin.tar.gz         + .sha256
ss-magic-{aarch64,x86_64}-unknown-linux-gnu.tar.gz    + .sha256
ss-magic-installer.sh
sha256.sum        # aggregate; covers the archives and source.tar.gz
dist-manifest.json, source.tar.gz + .sha256
```

**No Windows target**, which is what R78 is for.

**One measured gap, stated rather than papered over.** The installer embeds each target's SHA-256
inline and verifies the archive it downloads, so the *archive* is checksum-verified before extraction
– R71's substance. But `ss-magic-installer.sh` is the one release asset with **no** published
`.sha256` sibling and no entry in `sha256.sum`, so the bootstrap cannot verify the script before
executing it; that hop rests on TLS alone. Two ways to close it, both open to U23: publish a checksum
for the installer, or skip the installer and have the bootstrap fetch
`ss-magic-<target>.tar.gz` plus its `.sha256` directly, verify, extract and move – which satisfies
R71 literally ("verifies it against the release's published SHA-256 before anything is executed") and
drops the dependency on installer internals altogether.

## `plugin/bin/ss-magic-plugin` – the wrapper

`${CLAUDE_PLUGIN_DATA}` is exported to **hook and MCP/LSP processes only**. It is *not* in the Bash
tool's environment, so a shipped skill body cannot name that path – but the plugin's own `bin/` **is**
on the Bash tool's `PATH` while the plugin is enabled. R75's wrapper is the bridge, and it is what
every skill invokes.

```sh
#!/usr/bin/env bash
# Shipped at ${CLAUDE_PLUGIN_ROOT}/bin/, which is on the Bash tool's PATH while the plugin
# is enabled. ${CLAUDE_PLUGIN_DATA} is NOT exported to the Bash tool, so re-derive the data
# directory from the documented layout: <config dir>/plugins/data/<plugin>-<marketplace>/,
# where every character outside [a-zA-Z0-9_-] is replaced (so "ss-magic@ss-magic" is
# "ss-magic-ss-magic"). Prefer the real variable when one is set, for hook/MCP callers.
set -u
data="${CLAUDE_PLUGIN_DATA:-${CLAUDE_CONFIG_DIR:-$HOME/.claude}/plugins/data/ss-magic-ss-magic}"
bin="$data/bin/ss-magic"
if [ ! -x "$bin" ]; then
  echo "ss-magic: pinned plugin binary not installed at $bin" >&2
  echo "It installs on the next fresh session (SessionStart bootstrap); start a new session." >&2
  exit 127
fi
exec "$bin" plugin "$@"
```

Three decisions in that file are load-bearing:

- **The name is `ss-magic-plugin`, not `ss-magic`.** A wrapper named `ss-magic` would collide on
  `PATH` with a user's own installed `ss-magic`, and the resolution order between the plugin's `bin/`
  and the user's install is not something this plan controls. Non-deterministic resolution is exactly
  what the pin exists to prevent, so the wrapper takes a name nothing else claims.
- **It injects the `plugin` verb**, so skills read `ss-magic-plugin checklist list` and
  `ss-magic-plugin status --json` rather than stuttering. Every surface a skill needs is a `plugin`
  verb (R8), and the wrapper consequently cannot reach bare `ss-magic` – no update gate, no TUI,
  which is R69's posture enforced by shape rather than by care.
- **A missing binary is a clear message, not a silent 127.** The first session after an install or a
  pin bump runs with every ss-magic hook inert (R77), because sibling hooks on one event fire
  concurrently (R81) and the bootstrap cannot be relied on to finish first. A skill invoked in that
  window must say so rather than look broken.

## `.claude-plugin/marketplace.json` – at the repository root

```json
{
  "name": "ss-magic",
  "owner": { "name": "Viktor Stiskala" },
  "plugins": [
    {
      "name": "ss-magic",
      "source": {
        "source": "git-subdir",
        "url": "https://github.com/ViktorStiskala/superset-magic.git",
        "path": "plugin",
        "ref": "v0.10.0",
        "sha": "0000000000000000000000000000000000000000"
      },
      "description": "Session scratchpad, context page-fault gate, operator checklist, and cost ledger for the Superset workspace contract"
    }
  ]
}
```

- **`path` resolves from the repository root**, not from `.claude-plugin/`. `"plugin"` is correct;
  `"../plugin"` is not.
- **`sha` must be lowercase hex**, 40 characters. Uppercase is rejected at validation (AE55), which is
  what keeps a malformed pin out of a release.
- **`sha` is the effective pin when both are set**, and the CLI refuses to install when the pinned
  commit does not match the ref. `ref` stays for readability and for the case where the sha is being
  advanced.
- **The `sha` is a placeholder in this document only.** R95 makes the release commit fill it, and
  fills it *after* the named release's assets are published.
- No ss-magic code path reads this file at runtime. It is repository content, served to the harness.

### The cost of `git-subdir`: an object source is "external"

Measured in the 2.1.251 loader, and it is the reason this choice needed a ruling rather than a
default. **The loader branches on whether a source is a string or an object.** A relative-path string
source (`"./plugin"`) is resolved inside the already-cloned marketplace and is *not* treated as
external. **Every object source, `git-subdir` included, takes the external branch.**

The consequence: a plugin that only a project's `.claude/settings.json` enables is reported as **not
installed** until each user runs `claude plugin install` themselves. `git-subdir` therefore forgoes
collaborator auto-install. That is an acceptable price here – nothing in this plan depends on
zero-touch install for collaborators, and R66 already makes the install an explicit user action – but
it is a real capability the string form would have kept.

It also requires **`git` >= 2.25** on the client, for sparse-checkout cone mode.

What it buys, and why the trade was taken: explicit release pinning that survives being added by
direct URL to `marketplace.json`, with a commit-level `sha` the CLI enforces.

## Installing it

```bash
claude plugin marketplace add ViktorStiskala/superset-magic
claude plugin install ss-magic@ss-magic
```

- The first command accepts the `owner/repo` shorthand, a full git URL, or a direct URL to
  `marketplace.json`.
- `install` defaults to **`--scope user`** – machine-global, which is precisely why `plugin.enabled`
  survives as the per-repository gate (R5-R7, R65). An install made for one repository must not act
  in every other one on the machine.
- **Nothing hot-reloads.** Run `/reload-plugins` or restart the session; see below.
- The plugin loads from its local cache and needs no network at session start. Marketplace refresh
  happens in the background afterwards, and a failed refresh keeps the cached version.

Migration removes any pre-existing `~/.claude/skills/ss-magic/` (R66). It is not a fallback: the
loader resolves by plugin **name**, a marketplace install outranks a `@skills-dir` copy, and the
shadowed copy is reported in the `/plugin` Errors tab rather than loaded. Deleting it is cleaner than
relying on that precedence.

### Reading `claude plugin list --json`

`status` (R65) reports the harness-side view – scope, registration id, enabled flag – alongside
ss-magic's own resolved `plugin.enabled`, so "why is the plugin not acting" has exactly one place
that answers it. The shape it reads, measured on 2.1.251:

- The output is a **bare top-level JSON array**, not the `{"plugins": [...]}` object some docs show.
- Each entry carries `id`, `version`, `scope`, `enabled`, `installPath`, `installedAt`,
  `lastUpdated`, `projectPath`.
- `errors[]` and `notes[]` appear only when there is something to report – a trust-suppressed
  project-scope directory produces a `notes[]` entry, which is how suppression surfaces:

```json
{ "id": "…@skills-dir", "scope": "project", "enabled": false,
  "notes": ["1 project-scope directory under ./.claude/skills/ that may load as a plugin was
             skipped because this workspace was not trusted when plugins were scanned."] }
```

- **Ignore the exit code** – it was 0 in every run, including total failure. Surface `errors[]` and
  `notes[]` verbatim.
- Duplicate ids can appear once per `projectPath`, so dedupe when scripting against it.
- Match on **manifest name** `ss-magic`, not on id. A marketplace-sourced registration carries
  `ss-magic@ss-magic`; an id match written against `ss-magic@skills-dir` would miss it entirely, and
  matching on name is also what lets `status` see a second, shadowed registration at all.

Separately, `claude plugin validate` has no `--json` flag, so CI relies on its exit code
(`--strict` treats warnings as errors):

```bash
claude plugin validate .          # the marketplace root
claude plugin validate ./plugin   # the plugin subdirectory
```

**Validation is non-strict about source objects.** An unknown key inside a source object is silently
ignored rather than flagged – so a typo like `"subdir"` for `"path"` passes validation and fails at
install. CI must assert the source object's keys itself; `claude plugin validate` will not.

## Nothing hot-reloads

Measured in a live session where both `hooks/hooks.json` and `SKILL.md` were rewritten mid-session:
the *old* hook kept firing for the remaining three tool calls, and the model was served the *old*
skill body while the new one sat on disk. This refutes the docs' claim that "SKILL.md edits apply
immediately" – in 2.1.251 the whole plugin is snapshotted at session start.

Two consequences the plan depends on:

- A plugin update needs `/reload-plugins` or a restart before its skills or hooks take effect
  (monitors need a restart specifically).
- The bootstrap's pin comparison is not racing a live reload. The pin the running session sees is the
  pin that was on disk when the session started, which is why "the session that upgrades runs with
  ss-magic's hooks inert" (R77) is a bounded, one-session condition rather than an indefinite one.
