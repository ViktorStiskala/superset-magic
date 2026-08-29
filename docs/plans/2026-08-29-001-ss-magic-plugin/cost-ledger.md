# Cost ledger — measured feasibility and the attribution rules

Companion to [the plan](../2026-08-29-001-feat-ss-magic-claude-plugin-plan.md).
Every figure was measured against this session's own transcripts (113 files, 23.82 MB) on
Claude Code 2.1.251.

## Why this workstream exists

It is the falsifiability harness for everything else. The Read gate's threshold, the
compaction cap, and any quality-for-tokens trade are unverifiable without a measure of cost per
unit of delivered work. Build it first or the rest is guesswork.

## Feasibility — settled by measurement

| question | answer | evidence |
|---|---|---|
| Does `SessionEnd` carry usage data? | **No** — six keys only | payload captured verbatim |
| Does it carry `transcript_path`? | **Yes**, always, absolute | payload captured |
| Is the transcript complete when it fires? | **Yes** | snapshot inside the hook was byte-identical to the post-exit file, twice |
| Full scan time, 113 files / 23.82 MB | **0.072–0.079 s** in-process; ~0.09 s incl. interpreter start | 5 trials |
| Fits the 1500 ms budget? | **Yes, ~15–20x headroom** | measured against ~1.15 s usable |
| Is the JSONL append-only? | **Yes, empirically** | same SHA-256 over the first 500 KB while the file grew 1,535,811 → 1,628,651 bytes |

→ **Incremental tailing from a stored `(file_path, byte_offset)` is viable**, so the steady-state
cost is far below the 0.09 s upper bound.

## Attribution rules — what the data actually supports

- **Group by `gitBranch`, not `cwd`.** `gitBranch` is present on every line, main and subagent
  alike, and was stable throughout. `cwd` is *also* per-line but **not worktree-stable**: this one
  session already shows two distinct values (the worktree root and a nested
  `.scratchpad/…/prototype/proj`). Raw `cwd` would fragment one worktree's cost across paths.
  → Normalize each distinct `cwd` to a worktree root **once**, by caching
  `git rev-parse --show-toplevel` per unique value.
- **`gitBranch` alone is not globally unique** — two worktrees can share a branch name. The
  ledger keys on the resolved worktree root, with branch as a label.
- **Read the nested cache-TTL fields, not the flat total.** `cache_creation_input_tokens`
  collapses 1-hour and 5-minute writes, which price differently (2x vs 1.25x base input). The
  correct fields are `usage.cache_creation.ephemeral_1h_input_tokens` /
  `ephemeral_5m_input_tokens`. Reading only the flat total **understated this session's
  main-thread cost by ~13%** — a mistake the measuring agent made on its first pass and caught on
  review, which is exactly why the plan calls it out.
- **Subagents are found by walking the tree, not by an index.** No manifest ties a subagent
  transcript to its parent beyond directory placement (`<session>/subagents/`,
  `<session>/subagents/workflows/<wf-id>/`).

## Pricing — read the harness's own figures first

An earlier draft made a private price table the primary mechanism. Measurement demoted it to a
fallback.

**Claude Code writes its own priced records into main-session transcripts**:
`{"type":"cost-state", totalCostUSD, modelUsage{<model>:{costUSD}}}` — 28 found across 15 files,
all dated ≥ 2026-08-26. `claude -p --output-format json` likewise exposes `total_cost_usd` and
`modelUsage[].costUSD` with a `costBasis: list|managed|unknown`.

→ **Read those first.** Ship a private price table **only** as the fallback for: subagent
transcripts (which never carry cost-state), pre-2026-08-26 transcripts, per-turn attribution
(cost-state is session-cumulative and does not decompose — 696,545 opus output tokens in cost-state
versus 323,008 summed from the same transcript's assistant messages), and `hasUnknownModelCost:true`.

When the fallback table is used it must be **snapshotted and versioned at ingest**, so a historical
entry does not silently change when prices update later — the table bundled in this environment was
already ~2 months stale.

**Present it as a relative signal, never as a bill.** Transcript-derived cost cannot know about
negotiated rates, org discounts, or billing reconciliation. It is sound for "branch A cost 3x branch
B"; the Admin Usage & Cost API is the authority for an invoice. The output says so on its face.

## Shape — ruled

**Ship the `SessionEnd` hook, and let it write the whole row. Do not detach, and do not make this
an on-demand-only subcommand.** An on-demand tool can never observe a session that has already
ended, which is the entire point of a ledger; a detached background write is unobservable and
un-debuggable; and the budget objection is dead:

| scope | time | vs the 1500 ms budget |
|---|---|---|
| this session's tree (113 files) | 0.078–0.099 s | **5%** |
| worst tree in the whole 2.61 GiB corpus (1,256 files, 354.7 MiB) | 0.87 s cache-bypassed | **58%** |

The all-time worst case fits with ~40% headroom, and it is CPU-bound, so a cold cache does not
change it.

Constraints, each forced by a measurement:

- **Scope is the ending session's own tree only** — `transcript_path` plus
  `find <dirname>/<session_id>/ -name '*.jsonl'`. Main-jsonl-only misses **~82%** of usage records
  (450 opus messages across the tree, 83 in the main file).
- **Do not set an explicit `timeout`.** 1500 ms is the default *per hook*, and hooks run in parallel
  each with their own timer — adding hooks does not shrink the window. But the CLI genuinely blocks
  on exit: a `"timeout": 30` hook turned a ~3 s run into 8.39 s. Raising it is paid in user-visible
  exit latency.
- **Keep the byte-offset store** `{path → (offset, inode, size)}` — now as a **rotation guard**
  rather than a performance necessity. Offsets always land on `\n` (append-only re-confirmed by
  re-hashing a 983,819-byte prefix), so there is no partial-line handling. Reset a file to 0 when
  `size < stored_offset` or the inode changed.
- **Append-only and idempotent, keyed on `session_id`.** `/clear` mints a new session id and emits
  `SessionEnd(reason:"clear")` → `SessionStart(source:"clear")` → `SessionEnd(reason:"other")`, so
  one CLI process yields more than one session id. Transcript-completeness is measured only for
  normal `-p` exit; SIGKILL / crash / logout are untested, so a row must be safe to write twice.
- **Do not use `PostToolUse[Agent]` as the primary subagent path.** For *backgrounded* subagents —
  the default here — it fires at launch with `{"isAsync":true,"status":"async_launched"}`, carries
  no usage, and no later PostToolUse fires. Sum `message.usage` from the agent transcript instead
  (verified 13,310 computed versus 13,307 reported). Use it only as a fast path when
  `tool_response.status == "completed"`, and never treat `async_launched` as a completed zero-cost
  agent.
- **Fail open and silent.** On any error, exit 0 with no output. The audit channel is the
  transcript's `hook_non_blocking_error` attachment.
- **`ss-magic plugin cost` must be able to backfill.** SIGKILL is uncatchable, so a killed CLI runs
  no SessionEnd hook at all and leaves no row — a missing-invocation problem, not a lossy-transcript
  one. The `cost` verb therefore takes an optional session id or path and writes the row on demand,
  which is what makes the idempotency requirement pay for itself.

## What this session measured about itself

Recorded because it **contradicts** the source ideation doc, and the plan should not overfit:

| | this session | the ideation doc's session |
|---|---|---|
| main thread share (cost-weighted) | **13.1%** | 93% |
| subagent share | **86.9%** | 7% |
| main-thread requests | 200 | 2,629 |
| oversized tool-results carrying text | 8.5% carry **52.4%** | 10.5% carry 69.5% |
| dominant main-thread sink | **Bash (81.5%)**; zero direct `Read` | `Read` (46%) |

The Pareto shape reproduces; the main/subagent split **inverts**. This session delegated almost
everything to workflows, so its cost lives in subagents and its main thread never called `Read`
directly at all.

**Consequence for the plan, stated plainly:** the Read gate's value is *workload-dependent*, not
universal. It pays in a session that reads files on the main thread; it does nothing in a
workflow-heavy session like this one. That is an argument for the ledger existing — so the gate's
worth can be measured per-workload rather than assumed — and an argument against hard-coding a
threshold. Both land in the plan as requirements.
