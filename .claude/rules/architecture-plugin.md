---
paths:
  - "crates/ss-magic-plugin/**"
  - "assets/workflow/**"
---

## The Claude Code plugin (`crates/ss-magic-plugin/src/`)

`ss-magic-plugin <VERB>` is a second, largely independent program built on
`ss-magic-core`: it shares core's git plumbing (the probes, `gitignore` and
`discover`), `hashing`, `style`, the per-line `release` check, `reponame`,
`state_tree` and `superset_files`, and never depends on the CLI crate. Module
paths below are relative to `crates/ss-magic-plugin/src/`. Three facts shape
every module in it:

- **Two callers, two postures.** The harness invokes `ss-magic-plugin hook
  <event>`: the envelope arrives on stdin, the answer is JSON on stdout (so
  nothing else may be printed there), and a hook that cannot do its job exits 0
  anyway. A skill – or the person, by asking the model – invokes a named verb
  (`status`, `checklist`, ...): problems go to stderr with a non-zero exit, the
  ordinary CLI contract. **Nobody types a verb in a terminal**: the binary is
  installed under `${CLAUDE_PLUGIN_DATA}` and deliberately kept off `PATH`, and
  the Bash tool inside a session carries the `bin/ss-magic-plugin` wrapper
  instead. Keeping the two postures apart is a safety boundary, not tidiness –
  no hook reaches anything that can set `plugin.enabled` (`enable`, `disable`,
  `config set`), so a repository cannot arrange its own enablement by getting a
  hook to fire. Note the exact shape of that claim: the bootstrap DOES invoke
  one config-writing verb, `seed-config`, and it is safe because it has no code
  path to the `enabled` key at all; and the `SessionStart` handler spawns
  `release-check --refresh --quiet`, which writes only the plugin release cache
  in the OS cache directory and reads no configuration.
- **No update gate, no TUI, no install verb.** The marketplace is the only
  delivery path, and the binary is pinned alongside the skills, hooks and
  Markdown shipped with it; a mid-session self-update would leave the two
  describing different behavior. This is structural rather than a routing
  rule: the crate links neither `self_update` nor `inquire`/`ratatui`, and
  `--check` plus `cargo tree -i` assert that mechanically. What the plugin DOES
  do about releases is advise (R29–R33): `release-check` reports the newest
  known plugin release against the pin and, with `--refresh`, rewrites its
  cache from one bounded fetch; `SessionStart` reads that cache and tells the
  operator once per release, on `systemMessage`, that `/plugin` has a newer
  one. Nothing in the crate downloads a binary.
- **Fail-open, but fail-CLOSED on anything that could leak.** A hook that
  errors, panics or times out must look exactly like a hook that decided to do
  nothing. The gates that protect secrets invert that: an unknown answer is the
  refusing answer.

### Entry point

- `main.rs` (the crate root) – the argv parse and the dispatch table, nothing
  else.
  `HookEvent::from_token` and `HumanVerb::from_token` are the two closed
  vocabularies; `HookEvent::{Unknown, Missing}` are VALUES rather than parse
  errors, because a manifest from a newer plugin build can name an event this
  binary never heard of and the contract for that is "exit 0, print nothing,
  record the unroutable name" – which the wrapper can only do if the name
  reaches it. `parse` returns `Parsed::{Invocation, Version, Help, MissingVerb,
  UnknownVerb}`; `run` calls `style::init_no_color()` for a hook invocation (an
  ANSI escape would make the JSON unparseable) and the ordinary `style::init()`
  otherwise, then dispatches. Note `HookEvent::FileChanged` parses and routes,
  but the shipped manifest declares no `FileChanged` entry – see the
  `hook/file_changed.rs` entry under Hooks.
  `-V`/`--version` prints `version_line()` – `ss-magic-plugin <version>` on ONE
  line, exit 0 – ahead of verb parsing, and both flags are recognized only as the
  FIRST token (unlike the CLI's whole-argv scan, because every verb here parses
  its own flags). That line's shape is a contract, not cosmetics:
  `hooks/bootstrap.sh` gates every install on
  `"$staged_bin" --version | head -1 | awk '{print $NF}'` equalling the pin, and
  `status.rs` probes the same flag for drift, so the version must stay LAST on
  line one and the flag must never answer with usage text.
  The two predicates over `HumanVerb` say different things: `writes_config` is
  `Enable | Disable | Config | SeedConfig`, while `can_set_enabled` is
  `Enable | Disable | Config`. Only the second carries the safety property, and
  `writes_config` cannot stand in for "nothing reachable from a hook may be one
  of these": `SeedConfig` (`seed-config`) is invoked by the `SessionStart`
  bootstrap, and `ReleaseCheck` (`release-check`) is spawned by the
  `SessionStart` handler itself (`release-check --refresh --quiet`, detached).
  The test `no_hook_invoked_verb_can_set_enabled` lists exactly those two as
  hook-invoked and asserts neither can set `enabled`. Do NOT re-derive
  "hook-reachable" from `writes_config`.

### State: where the plugin keeps things, and why there

- `atomic.rs` – the one atomic-write primitive the plugin's whole-file writers
  share: create a temp file in the target's own directory, write and flush it,
  chmod it when a mode is given, fsync it when asked, then rename it over the
  target – so a reader never sees a half-written file, and a crash mid-write
  leaves the previous file untouched rather than truncated.
  `write_atomically(path, body, prefix, suffix, what, mode, sync)` takes the
  temp-file suffix, the mode and the fsync choice as arguments because its
  callers differ on all three; no module hand-rolls its own
  `tempfile::Builder` -> `write_all` -> `flush` -> chmod -> `persist` sequence.
  Its callers are `heartbeat.rs` (the prune rewrite only – an ordinary append
  is not a rewrite), `ledger.rs` (replacing a session's existing row, the
  offsets file and the price snapshot – a new row is a plain append),
  `cache.rs`, `bypass.rs`, `expect_artifact.rs`, `scratchpad.rs`
  (the session pointer), `compact_window.rs`, `setup_ci.rs` (the CI workflow
  writer), `checklist/verbs.rs` (the document and pointer writers) and
  `release_check.rs` (the release cache).
- `pathnorm.rs` – the lexical path reduction every gate that decides from
  a path shares, and the reason the R88 checklist deny stopped growing new
  bypasses. `normalize` removes `.` and cancels `..` TEXTUALLY (the caller
  canonicalizes afterwards, which is what handles symlinks – and the order is not
  interchangeable, because the filesystem cancels a `..` AFTER resolving a
  symlink); a leading `..` that cannot be cancelled is COUNTED rather than
  pushed, since a pushed one is poppable by the next `..` and `../../x` would
  collapse to `x` – a path escaping its tree quietly becoming one inside it, the
  exact failure a containment check exists to catch. Do NOT reimplement it by
  walking `parent()`/`file_name()`: `file_name()` is `None` for a `..`
  component, so such a walk SKIPS the hop instead of cancelling it.
  `process_view` answers whether resolving a path here would even mean the same
  thing as resolving it there, as a property of the COMPONENT SEQUENCE rather
  than a list of recognized prefixes – `…/proc/<selector>/cwd/<rest>` is the one
  re-rootable form (`Cwd(rest)`), everything else below a process selector
  (`root`, `fd/<n>`, `task/<tid>`, `ns/…`) is `Opaque`, and the FIRST selector
  wins so `/proc/self/root/proc/self/cwd/…` cannot be re-rooted on another
  process's mount namespace. The scan runs over the whole sequence because
  procfs is mountable anywhere. `home_relative` classifies a LEADING `~`:
  `Own(rest)` for the current user's home – safe to expand here because the hook
  inherits the harness's `HOME`, so unlike `/proc/self` the expansion is
  process-INDEPENDENT – and `Other` for `~name`, whose location only a user
  database knows, so it is reported unexpandable rather than guessed at
  `/home/<name>`. It tests the leading byte through `to_string_lossy`, so a
  non-UTF-8 `~name` is still caught (missing one would leave it unexpanded,
  which is the unsafe direction).
- `tmproot.rs` – the private, per-machine, cross-session temporary root
  for coordination that predates any repository or session context (the "R80
  temp root" other entries name). `resolve_root()` is
  `/tmp/ss-magic-plugin/<identifier>/`, falling back to `$TMPDIR`;
  `identifier(home)` is the first 16 hex chars of SHA-256 of `$HOME` exactly as
  read, matching the shell bootstrap's `shasum -a 256` byte for byte.
  A predictable path is NOT evidence of ownership, so each managed component is
  `lstat`ed (never followed) and must be a real directory owned by this
  process's euid (raw `geteuid()`, not a shelled `id -u`) at mode exactly 0700;
  any failure makes that base entirely unusable rather than writing into a root
  someone else could control. `with_lock(root, name, f)` BLOCKS (unlike the
  self-updater's skip-on-contention lock) because concurrent hook handlers must
  actually coordinate, not silently skip; `try_with_lock` is the non-blocking
  variant. `flock` releases on process death, so there is no stale-lock
  reclaim. Both take the directory to lock in, so not every lock lives here:
  under this root are `install.lock` (the bootstrap's install), `magic-json.lock`
  (`config.rs`), `release-check.lock` (`release_check.rs`) and
  `file-changed.lock` (`hook/file_changed.rs`); `hooks.lock` and `cost.lock`
  sit in the machine-level store beside the heartbeat log and the ledger, and
  `current.lock` and `checklist.lock` in the `.superset/.magic/` state tree.
- `identity.rs` – the deterministic `<repo>-<branch>` slug, derived from
  git alone and never from the Superset workspace name (which can be silently
  renamed). `resolve(cwd)` returns `None` outside a git repo – there is no
  fallback identity, and the plugin simply does nothing. The repo half reuses
  `ss_magic_core::reponame::repo_name_stem` – the same derivation the CLI's
  `pack.rs` re-exports, so the two can never disagree about what this repo is
  called; the branch half slugifies HEAD, falling back to
  `detached-<short-sha>`, and strips diacritics so a precomposed and an
  NFD-decomposed accented branch name resolve to the SAME directory.
- `scratchpad.rs` – the per-worktree state tree at `.superset/.magic/`
  (`STATE_REL`), holding a `README.md` that explains the tree to anyone who
  finds it in a working copy; `sessions/<slug>/` with the six model-owned
  `STATE_FILES` (`CONTEXT.md`, `DECISIONS.md`, `LEARNINGS.md`,
  `OPERATOR-CHECKLIST.md`, `STATUS.md`, `TASKS.md`), beside which the hooks add
  the tool-owned `PRE-COMPACT.md` (`hook/pre_compact.rs`) and the
  `research-salvage/` directory (`hook/subagent_stop.rs`); the `current.json`
  pointer and its `current.lock`; the `conclusions/`, `bypass/` and
  `expect-artifact/` stores; and, written by the checklist verbs rather than
  here, the `checklist.json` pointer and its `checklist.lock`. Three hard
  rules: (1) **scaffold, never rewrite** – an existing state file is left
  byte-for-byte alone and only a genuinely missing one is created, via
  `create_new` so a race cannot clobber; only `current.json` is rewritten each
  run, under an fd-lock plus temp-file-then-rename so a lock-free reader never
  sees a half file. (2) **never adopt a tracked path** – POSITIVE tracked
  determination via `git::tracked_files`, so an unenumerable name fails closed
  as tracked-and-skipped. (3) **write nothing until git says the tree is
  ignored** – `ensure` refuses on both "git says no" AND "git could not be
  asked", using `git::is_ignored_no_index_str` so a tracked file inside the tree
  does not trigger a blanket refusal. Every path is containment-checked
  (`verify_contained`: an existing symlink must canonicalize inside the worktree
  root or the write is refused, checked for the `.superset` and
  `.superset/.magic` ancestors before creation). Dirs are 0700, files 0600 –
  defense in depth, NOT the sync-exclusion control, which is
  `sync::EXCLUDED_TREES`. `Refusal` and `Report` carry the outcome outward;
  `STATE_REL` and `ensure_state_ignored` are re-exports of core's
  `state_tree`, the ONE place the `.superset/.magic/` gitignore rule is
  written: the CLI's `workspace/migrate.rs::ensure_bootstrap_gitignores` calls
  the core function eagerly from init/migrate, so no CLI code reaches into the
  plugin crate; `ss-magic-plugin enable` / `config set` call it lazily through
  the re-export; and no hook ever calls it. A core test pins `STATE_REL` equal
  to the `.superset/.magic` entry of `sync::EXCLUDED_TREES`.
- `claim.rs` – the exactly-once file claim both one-shot stores are built
  on. `take(dir, path)` creates a private landing file in the SAME directory and
  `fs::rename`s the claim onto it; since `rename` requires its source to exist,
  exactly one racing caller wins. It is deliberately NOT built on `unlink`'s
  `ENOENT` – see
  [docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md](../../docs/solutions/logic-errors/unlink-is-not-an-exclusive-claim.md),
  the write-up behind the first plugin constraint in
  [hard-rules.md](./hard-rules.md).
- `heartbeat.rs` – the append-only machine-level `hooks.jsonl` every hook
  invocation leaves a `Row` in (including no-ops and failures), which is what
  `ss-magic-plugin status` reports last-fired-at and outcome counts from. It
  lives under `directories`' DATA dir, not the cache dir (a history swept by
  disk cleanup would be worse than none) and outside any worktree, so rows
  outlive worktree deletion. `append` holds an exclusive `tmproot::with_lock`
  on `hooks.lock` in that store, covering append AND prune together, since a
  prune rewrites the file wholesale. An append is a plain `O_APPEND` write of
  one line, creating the file owner-only (`FILE_MODE`, 0600) when absent and
  leaving an existing file's mode alone; the prune rewrite goes through
  `atomic::write_atomically`, so a prune that dies half-way leaves the previous
  file intact. `prune` keeps the newest `ROWS_KEPT` (2000) rows and drops
  anything older than 30 days, but a row stamped in the FUTURE (a backward
  clock jump) is kept, not dropped; it fires only once the file passes
  `PRUNE_TRIGGER_BYTES` (256 KiB), so the common case costs one `stat`. A prune
  failure never fails an otherwise-good append.

### Hooks (`hook/`)

The shipped `plugin/hooks/hooks.json` registers these entries; the
`PreToolUse` matcher is the tool set `GateTool::from_name` must cover:

| Event | Runs | Matcher | Timeout |
|---|---|---|---|
| `SessionStart` | `hooks/bootstrap.sh` | `startup` | 90 s |
| `SessionStart` | `session-start` | `startup\|resume\|clear\|compact\|fork` | 10 s |
| `PreToolUse` | `pre-tool-use` | `Read\|Edit\|MultiEdit\|Write\|NotebookEdit\|Grep\|Glob\|Bash` | 5 s |
| `PreCompact` | `pre-compact` | `manual\|auto` | 10 s |
| `SubagentStop` | `subagent-stop` | none | 10 s |
| `SessionEnd` | `session-end` | none | none (the harness default) |

Every entry except the bootstrap runs through `hooks/run-hook.sh` with the
event token as its argument.

- `hook/mod.rs` – the ONE pipeline: decode stdin, gate, dispatch, encode stdout,
  append a heartbeat row, always exit 0. `run` has no code path that produces a
  non-zero exit; fail-open is structural, not incidental, and a handler panic is
  caught with `catch_unwind` so it cannot take the session down. Handlers never
  touch stdout or stderr themselves – only `HookContext::diagnostic`, flushed to
  stderr after dispatch. Two gates sit HERE, before dispatch, not in the
  handlers: `plugin.enabled` re-resolved from disk on every invocation, and –
  for any route whose `Route.writes_state` is true – a fail-closed check that
  git reports `.superset/.magic/` ignored. `route()` is the whole routing table.
  The two roots the enablement gate needs come from `git::discover::roots`
  (R20): a filesystem walk that spawns nothing on the ordinary layouts, so a
  hook that stops at that gate – nearly every `PreToolUse` – runs no `git` at
  all (a PATH-shim test proves it); when the walk declines, the `rev-parse`
  probes run and every row from that invocation ends its `detail` with
  `discovery: fallback (<reason>)`, so `grep -c 'discovery: fallback'` over
  `hooks.jsonl` gives the fallback rate. `status` does not compute that rate;
  it shows only each event's latest row detail. `HookContext` carries the
  discovered `main_root` beside `repo_root`. The ignored-tree gate still asks
  git – it is fail-closed and runs only past the enablement gate.
  `quiet_mode(envelope, entrypoint)` (KTD12, R31) is the shared "is anybody
  watching" verdict every operator notice consults: quiet when the envelope's
  `permission_mode` is `bypassPermissions` or `dontAsk`, or when
  `CLAUDE_CODE_ENTRYPOINT` (`ENTRYPOINT_ENV`) names an embedding other than
  `cli`; ABSENT signals mean NOT quiet, deliberately – the notices are one
  operator line each and bounded once-per-machine or once-per-release, so a
  wrong "not quiet" costs a line while a wrong "quiet" hides the notice from
  every harness that omits a field. No further heuristic is layered on; the
  entrypoint is injected so the table is testable without touching the process
  environment.
- `hook/event.rs` – the pure wire format. Decoding is permissive (unknown keys
  ignored, only `cwd` required) but routing is not: the argv token picks the
  `Payload` variant, never the envelope's own `hook_event_name`. Two structural
  guarantees live in the types rather than in discipline: `PermissionDecision`
  has only a `Deny` variant (a hook can never GRANT a capability), and there is
  no `updatedInput` rewrite channel anywhere in `Response`. `PreCompact` and
  `SessionEnd` have no `Response` variant at all, so their silence is enforced
  by the compiler. `Common.permission_mode` is typed `Option<String>`: the
  2.1.251 bundle the contract was measured on and the 2.1.259 bundle installed
  when it was written both put a `permission_mode` key in the common envelope,
  but its value is whatever the harness had and `JSON.stringify` drops an
  undefined one, so absence is never read as any particular mode. `encode`
  emits the harness's field names – `hookSpecificOutput`, `hookEventName`,
  `additionalContext`, `permissionDecision`, `permissionDecisionReason`,
  `systemMessage`, and a top-level `{decision: "block", reason}` for a
  `SubagentStop` block.
- `hook/session_start.rs` – scaffolds the scratchpad and returns the operating
  guidance as `additionalContext`. A hard scratchpad refusal still returns a
  response, just a short "not set up yet" explanation – it never claims a state
  file exists that does not. `version_drift_notice` compares the running binary
  against the plugin root's pin and reports drift on `systemMessage`, the
  operator channel, never the model-facing one; it is best-effort and silent on
  every failure. `compaction_advice` (R27) shares that channel: on a
  `startup` source only, when `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` is in the
  hook's environment (the harness copies every settings file's `env` block
  into its own), the session is not `quiet_mode`, and neither project settings
  file configures an `autoCompactWindow`, it emits one notice per machine,
  recorded by a `compact-advice-shown` marker in the `ss-magic` cache dir –
  CLAIMED with `create_new` (owner-only), so two sessions starting at the same
  moment on a fresh machine race for one creation and exactly one announces,
  `AlreadyExists` being the "already shown" answer; written only by a notice
  that actually went out, and resolved LAST so a plain `resume` never touches
  the cache dir; with no directory to record it in, the notice is withheld
  rather than repeated. `release_suggestion` (R29–R31, KTD12) is the third
  notice on the same channel and reads a FILE, never the network: on `startup`
  only, it reads the plugin line's release cache (`PLUGIN_LINE.cache_file`,
  `plugin-release-check.json`, under the injected `Surroundings.cache_dir`),
  decides through the pure `release_check::suggestion(pinned, cache, source,
  quiet)` – not startup, no pin, no cache, no plugin tag, not newer, already
  suggested, then quiet mode LAST so a headless session with something to say
  records `release suggestion suppressed (quiet mode: …)` while one with
  nothing to say records "not newer" – and, when a newer release is due,
  RECORDS it before announcing it: `release_check::record_suggested` takes the
  shared non-blocking `release-check.lock` under the R80 root, re-reads the
  cache, and writes `suggested = tag`; contention (`Busy`), a tag the cache no
  longer names (`Superseded`), no lock root, or a write failure all WITHHOLD
  the notice for this session rather than announce without a record, because
  the record is what makes it once-per-tag. Quiet mode is decided before the
  write, so a headless session never spends the budget. Then (R30), still
  only on `startup`, only when someone is watching, and only when a pin
  exists: if the cache is missing or older than 24 h it spawns
  `current_exe() release-check --refresh --quiet` detached
  (`release_check::spawn_detached`: own process group, every stream on
  `/dev/null`, child dropped) and returns without waiting – AFTER the
  `release-check.lock` is released, so the refresh never contends with this
  invocation's own marker write. A source-scan test in `hook/tests.rs` asserts
  no `hook/` module names `UreqReleaseClient`, `ureq`, `fetch_releases`,
  `refresh_cache`, `refresh_with`, `for_product` or `resolve_newest_uncached`.
  The three notices join into one `systemMessage` a blank line apart
  (`join_system_messages`). Everything the handler reads from outside the
  envelope – plugin root, override, entrypoint, the cache dir (compaction
  marker AND release cache), the lock root, and the spawner – arrives in one
  `Surroundings` value (`handle` = `handle_with(ctx,
  &Surroundings::from_process())`), so no test can write a once-per-machine
  marker into the developer's own cache directory, which running the handler
  against the real environment would do on a machine where the override is
  set, and no test ever spawns a real refresh.
- `hook/pre_tool_use.rs` – three jobs on one event, in a fixed decision order:
  the checklist deny first, then the commit nudge (a `Bash` call returns
  there), then the Read gate, so the nudge and the Read gate never apply to
  the same tool call.
  (1) The **checklist deny**: a Read / Edit / Write / NotebookEdit of a checklist
  file (matched by the `docs/actions/<stem>.checklist.json` convention or by the
  pointer's recorded target) is denied with instructions to use the checklist
  verbs. It deliberately does NOT suggest an Explore agent, unlike the size
  gate – dispatching an agent to read the checklist would leak it into that
  agent's context just the same. `resolve_target` is the load-bearing part and
  runs in a FIXED order: expand a leading `~` on the RAW spelling (ahead of the
  reduction – the normalizer treats `~` as an ordinary segment, so `~/../x`
  would otherwise have its `~` popped and come out cwd-relative), reduce
  lexically, then ask `pathnorm::process_view`. A `Cwd` view re-roots on the
  ENVELOPE's cwd (the agent's, not this process's); an `Opaque` view is judged
  WITHOUT a root by the shape test, which then canonicalizes and re-asks only to
  ADD a denial, never to remove one – so a wrong answer costs a redirect to the
  checklist verbs rather than the deny itself. (2) The **commit nudge**: a
  `Bash` command whose trailing words are `git commit`, `git push`, or
  `gh pr create` – and NOT `gh pr view` / `list` / `diff`, which open nothing –
  gets `additionalContext` reminding the model to update the checklist, but
  ONLY when a candidate checklist looks absent from the commit: named by the
  pointer but never written (answered with no git call), or shown by
  `git::status_porcelain` as untracked or edited-but-unstaged. It never sets a
  decision; the command is never blocked, and the text says so. (3) The **Read
  gate**: a `Read` past the configured byte threshold
  (`threshold_lines * BYTES_PER_LINE`, with `BYTES_PER_LINE` = 40, so 120,000
  bytes at the default 3000 lines) is denied and routed to an Explore agent, or
  answered with the cached conclusion when one exists. It never emits an
  allow – only `Silent` or `Deny` – so it can never grant a capability, and
  every uncertain stat, path resolution or cache lookup falls through to allow.
  Escape hatches: a bounded `offset`/`limit` window, a subagent's own read, the
  `.superset/.magic/` state tree, non-text extensions, configured exemption
  globs, and a one-shot `bypass` claim. `GateTool::from_name` maps `Read`, then
  `Edit|MultiEdit|Write|NotebookEdit` (mutating), `Grep|Glob` (inert), and
  `Bash`.
  Every command these three jobs put in front of the model is spelled
  `ss-magic-plugin`, the wrapper on the Bash tool's PATH, and NEVER a bare
  `ss-magic`: the model runs them through Bash, where `${CLAUDE_PLUGIN_DATA}` is
  not exported and the bootstrapped binary cannot be named directly.
  `no_model_facing_deny_text_names_the_sync_cli` asserts the rule over every
  deny reason at once rather than string by string, so a new or reworded deny
  cannot slip the bare spelling past it. It checks each LINE for a leading
  `ss-magic ` (with the space that `ss-magic-plugin` does not have there), so it
  catches a command line without flagging prose or the conclusion cache's own
  `# ss-magic conclusion` heading. The human verbs' `Usage:` strings follow the
  same rule: nobody runs a verb in a terminal, so every `Usage:` string in the
  crate spells `ss-magic-plugin`.
- `hook/pre_compact.rs` – appends one timestamped entry to a tool-owned
  `PRE-COMPACT.md` in the session dir and returns silence. That file is
  deliberately NOT one of `STATE_FILES`: those are model-owned and never
  rewritten by the tool, so this is a seventh file the model is never told to
  edit. It re-checks the tracked-path refusal for its own file, since
  `scratchpad::ensure` only guards the paths IT writes. Compaction is never
  blocked or slowed.
- `hook/subagent_stop.rs` – two independent jobs, both skipped when the
  handler returns early: on a `stop_hook_active` re-entry, outside a
  repository, and when the scratchpad refuses. The **artifact contract**:
  `expect_artifact::take_oldest` removes a pending declaration and, if the named
  file is missing / empty / not a file, blocks the stop once. With nothing
  declared, nothing is EVER blocked; "at most once" is guaranteed twice over, by
  the `stop_hook_active` short-circuit and by the fact that taking the record IS
  the one-shot flag. The **salvage** acts only for a resultless agent – one
  whose `last_assistant_message` is absent or blank: its assistant-message text
  is pulled from its transcript, tail-kept to `SALVAGE_BYTE_BUDGET` (64 KiB) and
  written to `research-salvage/<ts>-<slug>.md` in the session dir with
  `create_new`, so an earlier salvage is never overwritten. When it runs, it
  runs BEFORE the block decision and independently of it, because data loss is
  irreversible while a block is retriable, and it can never fail the stop.
- `hook/session_end.rs` – the only moment the ledger row can be written, since
  the payload carries no usage data: it scans the session's transcript tree and
  appends one row. Heavily budgeted against the hook timeout (measured ~0.85 s
  cold, ~35 ms warm on a 382 MiB / 1257-file worst case, against ~1.15 s of real
  budget) using the ledger's byte-offset store. Raising the timeout is
  explicitly NOT the remedy – the CLI blocks on session exit waiting for this
  hook. It is `writes_state: false`, exempt from the ignored-tree gate, because
  the ledger is machine-level by design.
- `hook/file_changed.rs` – **present, tested, and INERT.** The shipped
  `plugin/hooks/hooks.json` declares five events – `SessionStart`, `PreToolUse`,
  `PreCompact`, `SubagentStop`, `SessionEnd` – and NO `FileChanged` entry, so
  nothing in a real session ever invokes this handler. It stays wired into
  `route()` and reachable by argv so the code stays exercised and landing the
  feature later is a manifest change rather than a rewrite. Do not describe it as
  a shipped hook. What it WOULD do: on a watched `.env`/`.envrc` write, ask
  `direnv status --json` (read-only – it never runs `direnv allow`) whether the
  user already trusts that file, and only then append the exported environment to
  the harness-supplied `$CLAUDE_ENV_FILE`, refusing if that target resolves
  inside the repo and writing nothing at all when the variable is unset.

### Human verbs

- `config.rs` – the typed `plugin` key in the overlaid `magic.json`, and
  the write path behind `enable` / `disable` / `config get` / `config set
  [--local]`. The gate's three knobs live under `plugin.gate`:
  `threshold_lines` (default 3000, clamped to 500..=20000) is the size above
  which a `Read` is gated; `inline_byte_budget` (default 10000, clamped to
  1000..=100000) bounds only the inline cached conclusion on
  `additionalContext`; `exemptions` (globs the gate never applies its
  threshold to) is empty by default. `resolve` is infallible by design: every
  malformed field degrades to a safe default and an out-of-range number CLAMPS
  rather than rejecting, so a typo can never turn the gate into something more
  permissive than configured.
  `enabled` is always read from the MAIN CHECKOUT's overlay regardless of cwd,
  because a worktree's own `magic.local.json` is itself a forward-sync target;
  `gate` resolves against the cwd root. `resolve(cwd_root)` finds the main
  checkout with the `git rev-parse --git-common-dir` probe and is what the
  human verbs call; `resolve_with_roots(cwd_root, main_root)` takes the main
  root already discovered and is what the hook pipeline calls, so the
  enablement gate costs no subprocess on the fast path – `None` falls back to
  `cwd_root`'s own overlay exactly as `resolve` does outside a repository.
  Writes are load-modify-write on exactly one file, preserving every unknown
  key, and every one of them goes through `write_plugin_key`, which REFUSES a
  target that resolves outside the repository through a symlink – on the file
  or on the `.superset` directory – as an error, before it reads anything.
  The decision is `landing(target_root, local)`: canonicalize the root and the
  deepest EXISTING component of `.superset/<file>` (the file, else `.superset`,
  else nothing – a fresh create under the root cannot be redirected) and
  require the second inside the first, so a repository behind a link such as
  macOS's `/tmp` still passes and an in-repo link is written through onto its
  target. It sits inside the one writer rather than in each verb so that
  `enable`, `disable`, `config set`, the seed and any verb added later cannot
  reach the file without it; the seed calls `landing` a step earlier as well,
  only so it can answer `SeedOutcome::OutsideRepository` instead of surfacing
  a failed write. The hazard is a repository committing `.superset/magic.json`
  as a link to a JSON file the person owns (a harness settings file, say): a
  verb run in that checkout would load THAT file, fold a `plugin` key in and
  write it back through the link. Every writer in the crate also takes the one
  `magic-json.lock` under the R80 temp root around its load-modify-write
  (`write_locked`): the human verbs BLOCK on it (a person asked for the write),
  while `seed-config` uses the non-blocking `try_with_lock` and defers to the
  next session on contention, because it runs from a hook. Without the lock a
  seed that loaded the file an instant before `enable` wrote `plugin.enabled`
  would write its loaded copy back and silently drop the key. `enable` prints
  `compact_window::enable_tip` after its success line (R27, the compaction
  advice) – one line naming `compact-window --recommend`, only when neither
  project settings file configures a window, and never a write.
  It also owns `seed-config` (R3a), the bootstrap's one-time gate-defaults
  write. There is no terminal path to the configuration verbs – the binary is
  off `PATH` and the CLI has no `plugin` subcommand – so rather than document a
  command nobody can type, the install makes the settings visible in the file
  the repository already tracks. `seed_block()` is the ONE place the block's
  shape is decided – `{"gate": {"threshold_lines", "inline_byte_budget",
  "exemptions"}}`, built field by field out of `GateConfig::default()` as a
  literal map rather than serialized from a struct, precisely so the writer is
  structurally incapable of emitting `enabled` (a `Serialize` derive would move
  that guarantee into whatever fields the struct grows next). `seed_config_at`
  is its pure half, and its bounds are tests, not conventions: it never writes
  `enabled` (an absent key already reads as off, so writing `false` would buy
  nothing and would make the seed look like a decision about enablement, which
  it must not be – this verb runs from a hook); it never stages (the block shows
  up in `git status` as an ordinary edit, because it is being SURFACED, not
  slipped in); it writes only when `load_magic_json` returns a config with no
  `plugin` key at all, so it is strictly once and never fights a hand edit; it
  never creates the file (absent OR unparseable both read as
  `SeedOutcome::NotAWorkspace` – an unparseable `magic.json` is far more likely
  a merge conflict than an invitation to rebuild it); it never writes through a
  symlink that leaves the repository (`SeedOutcome::OutsideRepository`, the
  shared `landing` decision described above under the write path – `Existing`
  seeds, `Outside` is this outcome, `Fresh` is `NotAWorkspace` because the seed
  never creates the file); and it writes through the same typed
  load-modify-write, so `MagicConfig`'s flattened `extras` preserve every other
  key – values, not byte order, since it re-serializes rather than patching.
  All four `SeedOutcome` variants are normal results, never errors – the verb
  runs unattended in whatever repository the session happens to be in, and
  most of those are not ss-magic workspaces. It seeds `root`, the CURRENT
  checkout's root, not the main checkout's, because `gate` (unlike `enabled`)
  resolves against the cwd root's own overlay.
- `cache.rs` – the conclusion cache behind `conclude <FILE> [--from BODY_FILE]`
  (the body is read from stdin without `--from`), `conclusions [KEY|FILE]` and
  `gc`. `identify` keys an entry on `(realpath, size, stamp)` – NEVER the read's
  offset or limit, so a conclusion about a file answers every later read of it.
  `envelope` wraps rendered content in nonce-keyed untrusted-data markers with
  the framing text placed BEFORE the quoted body; it is shared with the
  checklist renderer and the transcript salvage, because all three inject
  repository-authored text into a model's context. `conclude` prunes after each
  write through the private `prune_best_effort` (the newest `ENTRIES_KEPT`, 200,
  entries within 30 days, never the one just written), which warns and never
  fails the verb; `prune` and `gc` themselves return errors, so a sweep that
  cannot remove an entry makes `gc` exit non-zero.
- `bypass.rs` / `expect_artifact.rs` – the two one-shot stores
  built on `claim::take`. `bypass <FILE>` lets exactly the next gated Read of a
  resolved path through (`MAX_AGE_SECS` 24 h; an expired claim is still consumed
  but does NOT open the gate, so it cannot bypass indefinitely).
  `expect-artifact <FILE> [--note TEXT]` declares an output a later subagent must
  produce (6 h, shorter because it waits only for a machine-paced stop; an
  expired record is dropped rather than enforced, since blocking an unrelated
  agent hours later is worse than not enforcing). Both resolve and
  containment-check the path at DECLARE time, write records atomically at 0600,
  and inherit the scratchpad's ignore-gate refusal so a record can never appear
  as an untracked file in the working copy.
- `ledger.rs` – the machine-level `cost.jsonl` and the `cost [--here]
  [--backfill REF] [--json]` verb. One row per session id, enforced under an
  fd-lock (`cost.lock`, beside the ledger) held for the commit only (the scan
  runs outside it). The scan is incremental via a byte-offset store
  (`transcript-offsets.json`, beside the ledger) keyed on inode plus size, so a
  rotated transcript forces a full rescan instead of reading garbage. Two
  pricing rules matter: the harness's own `cost-state` figure is a cumulative
  FLOOR (take the max, and add table pricing for the main thread only when no
  harness figure exists, or the cost double-counts); and cache-write tokens are
  split 5 m (1.25x) versus 1 h (2x), because reading only the flat total
  undercounts. `Basis` records what priced a row: `Harness` (the harness figure
  alone), `Table` (the bundled price table alone, when there is no harness
  figure) or `Mixed` (the harness priced the main thread and the table priced
  the subagents, which no `cost-state` record covers). A row priced from the
  table names the table's version, and that version's rates are snapshotted
  once under `prices/<version>.json` in the store. Each row also carries
  `peak_context_tokens` (R26): the largest `input + cache_read +
  cache_creation` of any ONE assistant message on the MAIN transcript –
  subagents run in their own windows – kept as a running maximum across
  incremental scans (`build_row` folds `max(prior, tail)`, a full rescan
  starts over with its totals); `None` on a row written before the field
  existed or for a session with no assistant message, and a `None` is ignored
  rather than read as zero. `rows_for_repository(store, main_root, limit)` is
  the recommendation's population: rows whose `root` (or any `also_roots`)
  `git::discover`s to the same main checkout, so every worktree of one
  repository pools together and a deleted worktree simply drops out (a root
  that is no longer a directory is answered before discovery would spawn a
  fallback probe in it), newest first.
- `status.rs` – `status [--all] [--json]`, the one place that answers "why is
  the plugin not doing anything", across every silent-failure path: config
  disabled, harness registration missing or disabled, state tree not
  gitignored, binary not installed, manifest-versus-binary drift. Read-only –
  it never calls `scratchpad::ensure`, creates a store, or adds a gitignore
  rule – and it exits 0 whenever a report was produced, so a script parsing
  `--json` never has to special-case an exit code. Every null JSON value
  carries a non-null `note`; `acting` is `None` rather than a guess when the
  harness layer is unknown. The hooks section reads this worktree's heartbeat
  rows by default and every row on the machine under `--all` or outside a
  repository (`hooks.scope` is `this-worktree` or `machine`).
  `DECLARED_EVENTS` lists the five events the manifest actually registers and
  deliberately excludes `file-changed`. `PIN_FILE` is `ss-magic-plugin.version`
  and `BINARY_REL` is `bin/ss-magic-plugin`; the drift probe runs `--version`
  and reads the LAST field of the FIRST line, which is why that output shape is
  a contract. The harness and binary probes are time-bounded and degrade to a
  note. The `compaction` section (R27) is `compact_window::recommend_report`
  rendered as four `Field`s – the override and where it was found, the two
  windows, the recommendation with its basis – and adds ONE `problems` line,
  only when the override is set AND no window is configured;
  `Inputs.compaction` carries the `Sources` so the tests point it at a fake
  home. The `versions` section carries `newest_release` (a `Field`: the plugin
  release cache's tag with "checked <when>, fresh/stale" as the source, or a
  note when the cache is absent, holds no plugin tag, or no cache dir
  resolves) and `update_available: Option<bool>` against the pin – read from
  `Inputs.release_cache` at `Inputs.now`, never refreshed by `status`, and
  never a `problems` line (an available update is information, not a fault);
  a pin that is not a plain triple makes it `None` (unknown), never `false`,
  the same answer `release-check` gives. The cache path comes from the
  NON-creating `release_check::existing_cache_file` (core's
  `release::existing_cache_dir`), because `status` promises to create nothing
  and the writers' `cache_dir()` scaffolds the OS cache directory as a side
  effect. `SCHEMA_VERSION` is `2`, bumped whenever a key changes meaning or
  goes away. Shape `2`'s top-level keys are `schema`, `tool_version`, `cwd`,
  `repo`, `identity`, `enablement`, `state_tree`, `gate`, `compaction`,
  `bootstrap`, `versions`, `hooks` and `problems`; `versions` reports the
  running plugin's own version as `versions.running` (a reader of shape `1`
  expects `versions.cli`, which shape `2` does not have) beside `manifest`,
  `pin`, `binary`, `drift`, `detail` (the human-readable explanation of
  `drift`), `newest_release` and `update_available`.
  The heartbeat store it reads is the non-creating
  `heartbeat::existing_store_dir` (shared with `--recommend`), so a diagnostic
  never scaffolds the store it reports on.
- `spill_index.rs` – a strictly read-only listing of the harness's own
  oversized-tool-output files for this worktree, which otherwise have
  unguessable names and no index. An empty result always carries a note
  distinguishing "nothing found" from "could not locate the directory".
- `release_check.rs` – the plugin line's release cache and the
  `release-check [--refresh] [--json] [--quiet]` verb (R32, R33), plus the
  pure pieces the `SessionStart` suggestion is built from. The cache is core's
  `PLUGIN_LINE.cache_file` (`plugin-release-check.json`) in the shared
  `ss-magic` cache dir, written ONLY through `write_cache` →
  `atomic::write_atomically` at 0600, because it has two writers (the refresh
  and the hook's `suggested` marker) and a lock-free reader (the hook).
  `LOCK_NAME` (`release-check.lock`, under the R80 tmproot) is the ONE lock
  both writers take with `tmproot::try_with_lock` – never the blocking
  variant, since a hook must not wait on a 5 s fetch and a refresh skipped
  this session runs next session. `refresh_with(client, lock_root,
  cache_file, now)` reads the prior record INSIDE the lock (so a marker the
  hook just wrote is what gets carried forward), runs core's `refresh_cache`,
  writes, and reports `Ran(outcome)` or `Busy`; `record_suggested(lock_root,
  cache_file, tag)` re-reads under the same lock and answers `Written`,
  `AlreadyRecorded`, `Superseded` (the cache's tag moved) or `Busy`.
  `spawn_detached(exe, args)` is the R30 spawn (own process group, stdio
  null, child dropped, pid returned) and `spawn_refresh` points it at
  `current_exe()` with `REFRESH_ARGV` (`release-check --refresh --quiet`),
  the one argv both the hook and the verb's parser agree on. The verb is the
  ONLY place in the crate an HTTP client is constructed
  (`UreqReleaseClient::for_product("ss-magic-plugin", …)`), and only under
  `--refresh`; `--quiet` prints nothing (what the hook spawns), and the exit
  is 0 on every path that produced a report, a failed fetch included (R33) –
  only an unknown argument exits 2. The report names the newest known tag,
  the cache's age and freshness, the pin (`${CLAUDE_PLUGIN_ROOT}` first, then
  the harness registration's `installPath`, the same way `status` finds it –
  the harness probe runs only when the variable is absent), the running
  version, whether an update is available, whether the notice was already
  shown, and what `--refresh` did; every null carries a note. `REMEDY` is the
  operator's remedy text shared by the notice and the report, and it ends in
  "start a new session" rather than `/reload-plugins` alone, because a reload
  re-registers the plugin but keeps the old binary until a fresh session's
  bootstrap swaps it. Nothing here installs anything.
- `setup_ci.rs` – `setup-github-ci [--check|-n] [--force|-f]` writes
  `.github/workflows/ss-magic-checklist.yml` from the embedded
  `assets/workflow/checklist.yml` (`TEMPLATE`), pinning the running binary's
  version. Its first output line is `state: <token>` (`absent`, `identical`,
  `pin-stale`, `differs`), a stable token the `setup-github-ci` skill branches
  on. `classify` returns `State::{Absent, Identical, PinStale { found,
  generation }, Differs}`, and only `Differs` (a local edit, or not this
  workflow at all) needs `--force`. `PinStale` is proved, never guessed: a
  template re-rendered at the version the file itself names must reproduce the
  file byte for byte. `classify` tries the current template first
  (`Generation::Current`, only for a version other than the running one), then
  every `LEGACY_TEMPLATES` entry (`Generation::Legacy(label)`), each read
  through its own pin key and placeholder and with NO version comparison: a
  legacy workflow is stale even at today's version, because the template
  around the pin is what changed. An edited legacy file matches no render and
  is `Differs`.
  `LEGACY_TEMPLATES` holds one `LegacyTemplate { label, body, pin_key,
  placeholder }` per template generation an earlier release shipped, oldest
  first, each `body` an `include_str!` of a byte-exact fixture under
  `assets/workflow/legacy/` (extracted with `git show
  <tag>:assets/workflow/checklist.yml`; a fixture carries no comment, since one
  would break the byte comparison, so provenance lives in the table's doc
  comment). `0.11.0` (`checklist-0.11.0.yml`) is what `v0.10.0` and `v0.11.0`
  shipped while the plugin was still the CLI's `plugin` subcommand, pinning
  the CLI under `SS_MAGIC_VERSION:` / `@SS_MAGIC_VERSION@`; `1.0.1`
  (`checklist-1.0.1.yml`) is what `v0.11.1` through `ss-magic-plugin-v1.0.1`
  shipped, with today's key and placeholder. Both generations ran `verify` /
  `render-md` with no arguments, through the active-checklist route – they
  rendered whichever checklist was already merged and failed once
  `docs/actions/` held two. Whenever a release changes
  `assets/workflow/checklist.yml`, copy the released file into
  `assets/workflow/legacy/` and append it to the table BEFORE editing the
  template: a generation missing from the table makes every workflow that
  release wrote read as `differs`, a "local edit" needing `--force`.
  `pinned_version` reads today's `PIN_KEY` and then each legacy key.
  The report line (`report_state`) names the generation found, and a bounded
  unified diff (`print_diff`, `DIFF_LINE_BUDGET` 120 lines, then a count of the
  rest) prints for `Differs` AND for every `PinStale` – the current
  generation's is the one pin line, a legacy one's is the whole rewrite the
  write makes. `--check`/`-n` writes nothing and ends with the `would` line
  for its state; for a legacy `PinStale` that line says a run would replace
  the file with the current workflow rather than advance the pin. A workflow
  that exists but cannot be read is refused (exit 2) rather than treated as
  absent, a refused `Differs` exits 1, and a repository with no
  `docs/actions/*.checklist.json` yet gets an advisory warning, never a
  refusal. `run_core` hands the process's streams to `run_core_with(cwd,
  version, check, force, out, err)`, so the tests assert on the state line,
  the report and the diff – what the skill reads before it asks anything.
  Written 0644 through `atomic::write_atomically` – committed content, unlike
  the 0600 state tree.
  The template is on the PLUGIN's release line throughout:
  `VERSION_PLACEHOLDER` is `@SS_MAGIC_PLUGIN_VERSION@` and `PIN_KEY` is
  `SS_MAGIC_PLUGIN_VERSION:`, and the workflow downloads
  `ss-magic-plugin-<target>.tar.gz` from an `ss-magic-plugin-v$VERSION`
  release (verifying its published `.sha256` sibling). It must use the plugin
  tag because the plugin's version is never equal to the CLI's, so a
  `v$VERSION` tag would name a different release entirely – or none at all.
  Its `render` job selects the documents itself and never relies on the
  gitignored pointer, which CI never has: it checks out GitHub's merge commit
  with `fetch-depth: 2`, fails unless `HEAD^2` exists (anything but a merge
  commit would diff against the wrong parent), and writes `git diff
  --name-only -z --no-renames --diff-filter=AMT HEAD^1 HEAD --
  ':(glob)docs/actions/*.checklist.json'` to a file under `$RUNNER_TEMP` – the
  top-level checklists the pull request adds, modifies or type-changes against
  the target branch tip, so a deleted one is never selected, a renamed one is
  selected at its new path, and a nested one never is; a failed diff fails the
  job rather than reading as "none". `present` is true when at least one name
  was written. The verify and render steps each read the NUL-separated names
  back into a bash array, fail when it comes back empty (the no-argument
  route would guess a document), and pass the names as separate arguments,
  never interpolated, to `ss-magic-plugin checklist verify` and `checklist
  render-md --max-bytes 60000` (under GitHub's 65,536-character comment
  limit), so several changed checklists become one comment and none means a
  green job with no comment.
- `compact_window.rs` – two halves, and the split IS the safety
  story (R28). `compact-window --set <TOKENS>` writes an absolute
  `autoCompactWindow` into the per-machine, gitignored
  `.claude/settings.local.json`, never the tracked `.claude/settings.json`. It
  is strictly opt-in (no flag at all prints usage and does nothing), never
  clobbers an existing value (an explicit `null` is NOT a value – it reads as
  unset here exactly as `read_window` reads it, so the verb every other
  surface points a `null`-window user at actually writes), load-modify-writes
  so unrelated harness keys survive, and refuses rather than rebuilding a
  malformed file – and it is the ONLY settings write in the module (it also
  appends the `.claude/settings.local.json` ignore rule to the repository's
  `.gitignore`, the same rule every per-machine file gets).
  `compact-window --recommend [--json]` (R24) is read-only: it reports whether
  `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` is set and WHERE (the process environment,
  then the `env` block of the user's `${CLAUDE_CONFIG_DIR:-~/.claude}/settings.json`,
  the project's two files, and the platform's managed-settings file – `Sources`
  carries those paths so tests run against tempdirs), the window each project
  file configures, and a recommendation with the exact `--set` command; it
  writes nothing and exits 0, which a test proves by snapshotting the
  repository tree, a fake home and the store before and after. The
  recommendation (R25, KTD11) is `recommend(peaks)`: over the newest
  `RECOMMEND_ROWS` (20) ledger rows attributable to this repository that carry
  `peak_context_tokens`, `clamp(ceil_to_10000(1.25 × max_peak), 100000,
  1000000)` in INTEGER arithmetic (`(max*5).div_ceil(4)`, so `1.25 × 80000`
  stays exactly 100,000 instead of a float rounding it up to 110,000); three or
  more rows are `high` confidence, one or two `low`, none gives no number and
  the generic range guidance instead – never a made-up figure.
  `recommend_report` is the shared read-only report `status`'s compaction
  section is built from; `window_configured` / `enable_tip` are the two small
  helpers the other advisory surfaces key on. Every advisory surface says the
  override is the person's to remove by hand; nothing in the crate ever edits
  the user's settings, a managed settings file, or the tracked project file.

### The operator checklist (`checklist/`)

The typed document at `docs/actions/<YYYY-MM-slug>.checklist.json` and its verbs.
Layered like `cache.rs` – a pure model with the hook as one caller – and the
submodules are PRIVATE behind `checklist/mod.rs`, so no caller can bypass
canonical ordering or validation. The `PreToolUse` deny above makes these verbs
the ONLY write path.

- `checklist/schema.rs` – the `Document` / `Section` / `Item` model. Every field
  is `#[serde(default)]` so a hand-edited or partial file still parses (defects
  are the validator's job, not the parser's), and every level carries a
  `#[serde(flatten)] extras` map so a key from a newer build survives a rewrite.
  `kind` defaults to the strictest `Check`, so a missing kind never silently
  disables verification; `expected` is `Option<Option<String>>` because an
  absent key and an explicit null mean different things. `parse_iso8601`
  (Hinnant's `days_from_civil`, no date crate) requires an explicit offset and
  rejects `24:00` and leap seconds. `Timestamp` deliberately has no `Ord` –
  compare through `.instant()`, since `+02:00` can sort lexically after a `Z`
  stamp that is actually earlier.
- `checklist/order.rs` – `canonicalize` re-establishes the one arrangement a
  checklist is ever stored in on every write, so a diff shows real changes.
  Items sort by `(done, priority rank, created)` with the id as final tie-break,
  making order a pure function of content rather than of prior position; an
  unreadable timestamp sorts to the end instead of aborting the sort. Section
  order is never touched – author-declared order is render order.
- `checklist/validate.rs` – pure findings, no printing and no I/O.
  `Severity::Error` makes `verify` exit 1 (the CI gate) and makes a mutating
  verb print a "still to fill in" note after its write; it does not stop
  rendering: `list` renders and adds a stderr note counting the errors, and
  `render-md` does not validate at all. `Warning` describes shape defects the
  next CLI write self-repairs and must NEVER fail CI.
- `checklist/render.rs` – the single `render()` behind `list` (bounded to
  `LIST_BYTE_BUDGET`, 24,000 bytes) and `render-md` (unbounded unless
  `--max-bytes` says otherwise; the exact body CI posts as the PR comment), so
  for one document the two outputs differ only in the byte budget; joining
  several documents and enforcing `--max-bytes` happen in `verbs.rs`, one
  `render()` per document. `verify` does not render, and the commit nudge is a
  fixed advisory string with no checklist in it. Every field of user-authored
  prose goes through `prose_inline` / `md_link` escaping (`prose_inline` is
  `pub(super)` so `verbs.rs` escapes the omitted-document marker's paths the
  same way), and each document's output is wrapped in its own
  `cache::envelope` – checklist prose is repository-authored text that
  reaches a model's context. Timestamps render through the shared UTC
  formatter, never a local clock, so output is identical across machines and
  timezones.
- `checklist/verbs.rs` – `init`, `add-item`, `add-entry`, `set`, `done`, `list`,
  `verify [FILE...]`, `render-md [--max-bytes N] [FILE...]`. Every mutating
  verb is read-modify-write over the WHOLE document (read, mutate one field,
  `canonicalize`, re-stamp `updated`, write back), so `extras` survive;
  writes are temp-file-then-rename preserving the existing mode. An advisory
  `tmproot::with_lock` on `checklist.lock` in the state tree spans the whole
  read-mutate-write, and spans exist-check plus write for `init`, so
  concurrent verbs cannot lose an update or duplicate a slug.
  Exit codes are distinct on purpose: 2 for "the command as typed cannot be
  carried out", but 1 from `verify` for "the document is invalid", so CI can
  tell them apart. The `.superset/.magic/checklist.json` pointer's contents are
  NOT trusted blindly – the target is validated lexically against absolute
  paths and `..` segments – and `resolve_active` falls back to the naming
  convention (unambiguous single match only) when no pointer exists.
  With no FILE, `verify` and `render-md` take exactly that active-checklist
  route, unchanged; `render-md --max-bytes N` with no FILE applies the budget
  to the active checklist alone. Explicit FILEs are what a caller without a
  pointer (the CI workflow) uses. `load_explicit` validates and reads EVERY
  FILE before anything is printed, so one refused path stops the run (exit 2)
  before any verdict, and it never reads or writes the pointer – one that
  happens to exist cannot change which documents are read. `validate_explicit`
  refuses a FILE, naming the rule it broke, when it is absolute or has a `..`
  component; when it fails `matches_convention` (the case-folding predicate
  the checklist deny uses, so a nested `docs/actions/sub/…` or a differently
  named file is refused); when `lstat` shows a symlink (never followed,
  wherever it points); or when its canonical path falls outside the
  CANONICALIZED repository root (a symlinked directory higher up), while a
  root that itself sits behind a link, such as macOS's `/tmp`, still accepts
  its own files. The document is then read through that canonical path. Any
  other `-`-prefixed token is an error (exit 2), never a path, and only
  `render-md` accepts `--max-bytes`, once, anywhere in the tail.
  `verify FILE...` reports each document in turn, prefixing every finding
  with its path, and exits 1 when any is invalid. `render-md FILE...`
  (`render_documents`) emits each document's own `render()` output, in its
  own envelope, in the order given, joined by the fixed `DOCUMENT_SEPARATOR`
  (a Markdown thematic break; the verb adds no repository text of its own).
  `--max-bytes N` bounds the WHOLE body, the marker included. When everything
  fits nothing is cut; otherwise the body is the longest run of leading
  documents that fits beside a marker naming the rest (`omitted_marker`), so
  a document is never split mid-text. A FIRST document too big on its own is
  rendered with `Budget::Bytes` set to the room the marker leaves, and
  `cache::envelope` truncates it inside its own envelope; only when even that
  does not fit is it left out too. The marker is fixed text plus the omitted
  paths, each through `prose_inline` with every CR made a line feed first (a
  lone CR ends a CommonMark line and could open a code fence), listing names
  while they fit and counting the rest as "… and N more"; it is capped at
  `MARKER_RESERVE` (2,048 bytes, separator included), which is also the
  smallest `--max-bytes` the parser accepts. `run_core` hands the process's
  streams to `run_core_with(cwd, sub, now, out, err)`, which passes them to the
  reading verbs (`list`, `verify`, `render-md`) so tests assert on what they
  print; the writing verbs still print to the process's own streams, since
  their result is the file they wrote.
