# Plugin assets — every file the plugin ships, verbatim

Companion to [the plan](../2026-08-29-001-feat-ss-magic-claude-plugin-plan.md). These are the exact bytes `ss-magic plugin install` writes. All of it is **pure JSON and Markdown** — no scripts are vendored, because the hook command is the `ss-magic` binary itself.

## Installed layout

```plaintext
~/.claude/skills/ss-magic/
├── .claude-plugin/
│   └── plugin.json
├── hooks/
│   └── hooks.json
└── skills/
    ├── scratchpad/SKILL.md
    ├── operator-checklist/{SKILL.md,reference.md}
    └── setup/SKILL.md
```

Only `plugin.json` goes inside `.claude-plugin/`. The docs call the alternative out explicitly: *"Common mistake: Don't put `commands/`, `agents/`, `skills/`, or `hooks/` inside the `.claude-plugin/` directory."* `skills/<name>/SKILL.md` is always scanned; `hooks/hooks.json` is auto-discovered at the plugin root.

Skill names are deliberately **unprefixed** — the manifest `name` already becomes the invocation prefix, so they read `/ss-magic:scratchpad`, `/ss-magic:operator-checklist` and `/ss-magic:setup` rather than stuttering as `/ss-magic:ss-scratchpad`.

## `.claude-plugin/plugin.json`

Only `name` is required — confirmed against the real minimal manifest shipped by `security-guidance/2.0.7`. `name` is the invocation prefix and must stay `ss-magic`.

```json
{
  "name": "ss-magic",
  "version": "0.10.0",
  "description": "Session scratchpad, context page-fault gate, and cost ledger for the Superset workspace contract",
  "repository": "https://github.com/ViktorStiskala/superset-magic",
  "license": "MIT",
  "author": "Viktor Stiskala"
}
```

`version` tracks the crate version so `ss-magic plugin status` can report drift between the installed manifest and the running binary. It carries **no** install semantics of its own: *"Changing `version` in plugin.json doesn't flip existing user installations."*

CI gates on `claude plugin validate ./plugin --strict`.

## `hooks/hooks.json`

The hook command is the **bare binary name on PATH** — measured working with a real Mach-O binary, no absolute path, no vendoring, no wrapper script:

```plaintext
argv[0]=ss-magic  argv[1]=plugin  argv[2]=hook  argv[3]=session-start
CLAUDE_PLUGIN_ROOT=/…/ss-magic   CLAUDE_PLUGIN_DATA=/Users/…/.claude/plugins/data/ss-magic
```

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup|resume|clear|compact|fork",
        "hooks": [
          { "type": "command", "command": "ss-magic", "args": ["plugin", "hook", "session-start"], "timeout": 10 }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Read|Grep|Glob",
        "hooks": [
          { "type": "command", "command": "ss-magic", "args": ["plugin", "hook", "pre-tool-use"], "timeout": 5 }
        ]
      }
    ],
    "PreCompact": [
      {
        "matcher": "manual|auto",
        "hooks": [
          { "type": "command", "command": "ss-magic", "args": ["plugin", "hook", "pre-compact"], "timeout": 10 }
        ]
      }
    ],
    "SubagentStop": [
      {
        "hooks": [
          { "type": "command", "command": "ss-magic", "args": ["plugin", "hook", "subagent-stop"], "timeout": 10 }
        ]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [
          { "type": "command", "command": "ss-magic", "args": ["plugin", "hook", "session-end"] }
        ]
      }
    ]
  }
}
```

### Why each detail is what it is

- **`"matcher": "Read|Grep|Glob"` is a LIST, not a regex.** Matcher syntax stays list-shaped only while it contains just alphanumerics, `_`, `-`, spaces, `,` and `|`. Any other character makes it an **unanchored regex**, where `Edit.*` would also match `NotebookEdit`. Keep it clean.
- **`SessionStart` matches all five sources**, including `fork`. `fork` carries the cost fields (`context_tokens`, `estimated_cache_write_usd`) alongside `resume`, so matching only four misses forks. `compact` is the reliable "a compaction actually happened" signal. [hook-contract.md](./hook-contract.md) is the contract authority for these five sources and agrees on the count – do not "fix" this matcher back to four.
- **`SubagentStop` and `SessionEnd` take no matcher** — those events support none.
- **`SessionEnd` sets no `timeout`.** The 1500 ms default is per-hook and parallel; raising it is paid directly in user-visible exit latency (a `"timeout": 30` hook turned a ~3 s run into 8.39 s), and the measured worst-case scan is 0.87 s.
- **`PreToolUse` gets the shortest timeout (5 s)** because a timed-out PreToolUse hook silently does not block. Returning fast is the difference between a gate and a no-op.
- **`CLAUDE_PLUGIN_ROOT` and `CLAUDE_PLUGIN_DATA` arrive as env vars**, so read them with `std::env::var` rather than threading them through argv. `CLAUDE_PLUGIN_ROOT` is explicitly ephemeral; `CLAUDE_PLUGIN_DATA` (`~/.claude/plugins/data/<id>/`) is the only update-surviving write location the harness offers — though ss-magic keeps its state in the repo's `.scratchpad/` instead, so this is informational.

## Requiring the binary on PATH

There is **no declarative way** to express it. `dependencies` covers other plugins only; the undocumented `binaries` map fetches by digest from Anthropic's own asset API and is gated off by default; and the only automatic install is Node packages from a lockfile.

Follow the documented LSP precedent: README install instructions, plus a **SessionStart self-check** that emits a `systemMessage` when the running binary's version differs from the installed manifest's. A missing binary cannot be reported from inside a hook at all – the hook simply never starts – so that case is covered only by the README and by `ss-magic plugin status`.
## Install verification

```bash
claude plugin list --json    # match id == "ss-magic@skills-dir" && enabled == true
```

**Ignore the exit code** — it was 0 in every run including total failure. Surface `errors[]` and `notes[]` verbatim; that is where the trust-gate suppression notice appears:

```json
{ "id": "…@skills-dir", "scope": "project", "enabled": false,
  "notes": ["1 project-scope directory under ./.claude/skills/ that may load as a plugin was
             skipped because this workspace was not trusted when plugins were scanned."] }
```

## Idempotent writes

`ss-magic plugin install` renders the manifest bytes, compares them to disk, and **writes nothing when identical**. When bytes do change it prints a one-line notice naming `/reload-plugins` (and "restart for monitors").

This is required, not cosmetic: **nothing hot-reloads.** Measured in a live session where both `hooks/hooks.json` and `SKILL.md` were rewritten mid-session — the *old* hook kept firing for the remaining three tool calls and the model was served the *old* skill body while the new one sat on disk. This refutes the docs' claim that "SKILL.md edits apply immediately"; in 2.1.251 the whole plugin is snapshotted at session start.
