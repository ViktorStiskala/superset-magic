# Documentation drift findings (consolidated)

Repository: superset-magic (branch `ViktorStiskala/docs-update`, base `7f6c62d`).\
Inputs: 103 adversarially verified per-chunk findings and 24 raw omission findings from two coverage agents.\
Method: every raw omission was spot-checked against the code before it was kept. All 24 held up, some after small corrections (noted inline), and 6 of them duplicated confirmed findings. Findings that state the same wrong fact in several docs are merged into one entry. The entry sits under its primary doc, and the other docs' sections point to it.

Severity:

- **high** – user-facing claim that is false in a way a reader could act on (privacy, safety).
- **medium** – wrong fact, wrong count, or missing behavior that misleads a maintainer, reviewer or user about what the code does.
- **low** – incomplete, imprecise, or a minor omission.

Historical narration and code-suspect items are listed separately at the end and are **not** in the counts below.

## Summary

| Doc | High | Medium | Low | Cross-refs into it | Historical passages |
|---|---|---|---|---|---|
| `CLAUDE.md` | 0 | 11 | 15 | – | 44 |
| `README.md` | 1 | 6 | 4 | 4 | 1 |
| `CONTRIBUTING.md` | 0 | 2 | 4 | 2 | 7 |
| `CONCEPTS.md` | 0 | 0 | 5 | 3 | 0 |
| `.cursor/BUGBOT.md` | 0 | 6 | 7 | 7 | 15 |
| `docs/runbooks/forge-tag-and-release-protection.md` | 0 | 1 | 0 | – | 3 |
| **Total** | **1** | **26** | **35** | | **70** |

There are also 14 code-suspect items (code comments or code behavior to raise with the user, not doc fixes). See [Code-suspect items](#code-suspect-items).

## Top fixes by importance

1. README:766: "the only network access the plugin makes is the bootstrap" is false. `release-check --refresh` also fetches GitHub's release list, and README's own section on keeping the plugin current describes it (R1).
2. CLAUDE.md:208 and :1463, plus BUGBOT:88-93: overstate how git/gh calls route through `git_raw`. `gh` cannot go through it at all (C1).
3. CLAUDE.md:450, README:248, BUGBOT:323-335: `ensure_bootstrap_gitignores` applies three rules, not two. The `.superset/.magic/` rule is the one that unblocks the plugin (C2).
4. README:255: "picking done leaves the old layout intact" is wrong. Done materializes the new layout (R2).
5. CLAUDE.md:444, README:236/425: `clear_legacy_skills_install`, a write to the user's home directory, is undocumented everywhere (C3).
6. CONTRIBUTING:213-230: the sample `--check` output shows 7 lines, but the script prints 8, and the prose skips the release-gate assertion (N1).
7. CLAUDE.md:1332 and :1329, BUGBOT:911, CONCEPTS:273: the checklist `render()` caller list and the "Error blocks the renderer" claim are both wrong (C9, C10).
8. CLAUDE.md:1044 and CONCEPTS:327: salvage is conditional (only for a resultless agent), not unconditional (C7).
9. BUGBOT:158, :1211, :1389: name a nonexistent `workspace/superset_files.rs` (B1). BUGBOT:369 and :379 misdescribe pack's directory walk and its zero-count handling (B2, B3).
10. CLAUDE.md:1518 and CONTRIBUTING:193: CI runs more gates than the "FIVE" checks, and `--check-bump` (R98) is never named as a runnable command (C11).

---

## CLAUDE.md

### Medium

**C1. `git_raw` routing is overstated**

- **Location(s):** CLAUDE.md:208 (git/mod.rs bullet), CLAUDE.md:1463-1464 (Conventions). The same claim is in .cursor/BUGBOT.md:88-93.
- **Claim:** "All `git`/`gh` invocations shell out via a shared `git_raw` helper that surfaces stderr verbatim." BUGBOT adds: "Flag new git/gh calls that spawn `Command` directly."
- **Reality:** `git_raw` hardcodes `Command::new("git")` and returns the raw `Output`. It does not surface stderr; the `git` wrapper does that on a non-zero exit. Four functions spawn their own `Command`:
  - `nothing_to_commit` needs only the exit status of `git diff --cached --quiet`.
  - `gh_available` and `pr_create` run `gh`.
  - `timestamp_branch_suffix` runs `date`, which is neither git nor gh.
  All of these are in core's `git/mod.rs`, and nothing outside it spawns git or gh. Read literally, the BUGBOT rule flags existing code.
- **Evidence:** crates/ss-magic-core/src/git/mod.rs:14-22, :26-33, :145, :182, :195, :408
- **Fix:**
  - Both CLAUDE.md locations: "Every git/gh call shells out via `std::process::Command` from core's `git/mod.rs`. Most git probes go through the private `git_raw` (raw `Output`). `git` (trimmed stdout; a non-zero exit becomes an error carrying the verbatim stderr) and `git_optional` (a non-zero exit becomes `None`) are thin wrappers on it. The exceptions spawn their own `Command`: `nothing_to_commit` (exit status only) and the `gh` calls `gh_available` / `pr_create`."
  - BUGBOT: "Flag git/gh spawns added outside `git/mod.rs`, and new git calls there that bypass `git_raw` without needing an exit status."

**C2. `ensure_bootstrap_gitignores` applies three rules, not two**

- **Location(s):** CLAUDE.md:450-458 (migrate.rs bullet). Also README.md:248 and .cursor/BUGBOT.md:323-335 (the "exactly like `magic.local.json`" sentence and its flag list).
- **Claim:** It gitignores "BOTH `magic.local.json` AND the `.superset/backups/` tree". README says "two things".
- **Reality:** It applies three rules:
  - `magic.local.json` (a `File` rule)
  - `.superset/backups/` (`reverse_sync::ensure_backups_ignored`)
  - `.superset/.magic/` (core's `state_tree::ensure_state_ignored`)
  Its own doc comment says "Three rules", and each write path prints "Gitignored .superset/.magic/". CLAUDE.md:863 already says migrate calls `ensure_state_ignored`, so the two bullets contradict each other.
- **Evidence:** crates/ss-magic/src/workspace/migrate.rs:63-98 (line 97), :361, :540, :586
- **Fix:**
  - CLAUDE.md: "applies three idempotent rules: `magic.local.json` (...), the `.superset/backups/` tree (...), and the plugin's `.superset/.magic/` state tree (core's `state_tree::ensure_state_ignored`, the rule's single owner)". Extend the closing sentence: ignoring the state tree up front is what lets the plugin's hooks write state at all.
  - README: "three things up front ... and `.superset/.magic/`, the Claude plugin's per-worktree state tree, which the plugin will not write to until git reports it ignored."
  - BUGBOT: add the state tree to the sentence and to the flag list.

**C3. `clear_legacy_skills_install` is undocumented**

- **Location(s):** CLAUDE.md:444 (migrate.rs bullet). Also README.md:236 (Init and migration) and README.md:425-431 (scripted `ss-magic init`). BUGBOT has no matching rule.
- **Claim:** The write paths are listed without any write outside the repository.
- **Reality:** `run_migrate`, `run_init` and `run_init_noninteractive` all call `clear_legacy_skills_install()`, which removes a pre-marketplace `~/.claude/skills/ss-magic/`:
  - A symlink there has the link removed, not its target.
  - A directory there is removed with `remove_dir_all`.
  - It prints "Removed legacy ~/.claude/skills/ss-magic/".
  - It is best-effort (a failure warns) and is a no-op under `cfg(test)`.
  This is the only write these commands make outside the repo. No doc mentions it.
- **Evidence:** crates/ss-magic/src/workspace/migrate.rs:95-160, :362, :528, :577
- **Fix:** Add a sentence to the CLAUDE.md migrate.rs bullet covering:
  - the exact path, and that a symlink is unlinked, not followed
  - why it exists (the copy is shadowed by the marketplace install and shows up as a plugin-errors conflict)
  - that it is best-effort
  - that it is a no-op under `cfg(test)`
  Add one user-facing sentence to both README sections and a short BUGBOT rule (flag widening the removed path, or following the symlink).

**C4. The plugin uses more of core than the doc says**

- **Location(s):** CLAUDE.md:700
- **Claim:** `ss-magic-plugin` shares "core's git, hashing and gitignore plumbing – and nothing else".
- **Reality:** The plugin also imports `style`, `release`, `reponame`, `state_tree` and `superset_files` (config.rs:41). The same section names several of these as shared.
- **Evidence:** `grep -rhoE 'ss_magic_core::[a-z_]+' crates/ss-magic-plugin/src`; config.rs:41; identity.rs:24; release_check.rs; scratchpad.rs
- **Fix:** "... built on `ss-magic-core`: it shares core's git plumbing (probes, `gitignore`, `discover`), `hashing`, `style`, the per-line `release` check, `reponame`, `state_tree` and `superset_files`, and never depends on the CLI crate."

**C5. Heartbeat appends do not use `atomic::write_atomically`, and the caller list is incomplete**

- **Location(s):** CLAUDE.md:887 (heartbeat.rs), CLAUDE.md:781-787 (atomic.rs)
- **Claim:** "Appends go through the shared `atomic::write_atomically` helper." The atomic.rs entry lists its callers and says they "all call this one copy now".
- **Reality:**
  - `write_row` appends with `OpenOptions` and `append(true)`. Only `prune` uses `write_atomically`.
  - `release_check.rs::write_cache` also calls `write_atomically` and is missing from the list.
- **Evidence:** crates/ss-magic-plugin/src/heartbeat.rs:283-292, :363; release_check.rs:108
- **Fix:**
  - heartbeat: "An append is a plain `O_APPEND` write of one line at `FILE_MODE`. The prune rewrite goes through `atomic::write_atomically`, so a prune that dies half-way leaves the previous file intact."
  - atomic.rs: list the callers as heartbeat.rs (prune only), ledger, cache, bypass, expect_artifact, scratchpad (session pointer), compact_window, setup_ci, checklist/verbs and release_check. The refactor history goes to H-group 6 below.

**C6. `status` does not report a discovery fallback rate**

- **Location(s):** CLAUDE.md:907. Also the code comment in crates/ss-magic-plugin/src/hook/mod.rs:49-51 (code-suspect S4).
- **Claim:** The `discovery: fallback (<reason>)` suffix means "the fallback rate is readable from `status`".
- **Reality:** `status` keeps per-event outcome counts and only the LAST row's detail, so it never computes a rate. The rate comes from grepping `hooks.jsonl`, which is what `with_discovery_note`'s own doc comment says.
- **Evidence:** crates/ss-magic-plugin/src/status.rs:502-526, :1790-1814; hook/mod.rs:597-601
- **Fix:** "... so `grep -c 'discovery: fallback'` over `hooks.jsonl` gives the fallback rate; `status` shows only the latest row's detail per event."

**C7. Salvage is conditional, not unconditional**

- **Location(s):** CLAUDE.md:1044. Also CONCEPTS.md:327-329.
- **Claim:** "Salvage runs unconditionally and independently of the block decision." CONCEPTS implies salvage happens on every stop.
- **Reality:**
  - `salvage()` returns `None` when `last_assistant_message` is non-blank, so it acts only for a resultless agent.
  - Before salvage, the handler returns early on `stop_hook_active`, outside a repository, and when the scratchpad refuses.
  - When salvage does run, it runs before the block decision and independently of it.
- **Evidence:** crates/ss-magic-plugin/src/hook/subagent_stop.rs:31-37, :100-127, :220-237
- **Fix:**
  - CLAUDE.md: "The salvage acts only for a resultless agent (absent or blank `last_assistant_message`) and is skipped on a `stop_hook_active` re-entry, outside a repository, and when the scratchpad refuses; when it runs, it runs before and independently of the block decision ..."
  - CONCEPTS: "a stopping agent that ended without a final message has the text it did produce recovered from its transcript ..."

**C8. `prune`/`gc` are not best-effort**

- **Location(s):** CLAUDE.md:1147
- **Claim:** "`prune`/`gc` are best-effort and never fail the caller."
- **Reality:** Only the private `prune_best_effort` (called by `conclude`) swallows a failure. `prune` and `gc` return `Result`, and `gc_core` uses `?`, so a failed sweep makes `ss-magic-plugin gc` exit non-zero.
- **Evidence:** crates/ss-magic-plugin/src/cache.rs:629-681, :709-729, :860, :976
- **Fix:** "`conclude` prunes after each write through `prune_best_effort`, which warns and never fails the verb; `prune` and `gc` themselves return errors, so a sweep that cannot remove an entry makes `gc` exit non-zero."

**C9. `render()` has two callers, not five**

- **Location(s):** CLAUDE.md:1332. Also .cursor/BUGBOT.md:911-912, CONCEPTS.md:273-274, and crates/ss-magic-plugin/src/checklist/render.rs:3-6 (code-suspect S7).
- **Claim:** The single `render()` is behind `list`, `verify`, `render-md`, the commit nudge and the CI PR comment, "so all five are byte-identical".
- **Reality:**
  - Only `run_render` calls `render()`. It serves `list` (bounded) and `render-md` (unbounded). The CI comment is the `render-md` output.
  - `verify` deliberately does not render.
  - The commit nudge is a static advisory string with no checklist in it.
- **Evidence:** crates/ss-magic-plugin/src/checklist/verbs.rs:77, :482-484, :641-650, :674-680; hook/pre_tool_use.rs:1556-1567; assets/workflow/checklist.yml:149,156
- **Fix:** "the single `render()` behind `list` (byte-bounded) and `render-md` (unbounded, the exact body CI posts as the PR comment); `verify` does not render and the commit nudge is a fixed advisory string." Apply the same correction in BUGBOT and CONCEPTS (drop "a commit-time nudge").

**C10. `Severity::Error` does not block the renderer**

- **Location(s):** CLAUDE.md:1329. Also crates/ss-magic-plugin/src/checklist/validate.rs:29-31 (code-suspect S6).
- **Claim:** "`Severity::Error` blocks `verify` and the renderer."
- **Reality:**
  - `verify` exits 1 on an Error.
  - `list` still renders and adds a stderr note counting the errors.
  - `render-md` renders without validating at all.
- **Evidence:** crates/ss-magic-plugin/src/checklist/verbs.rs:482-483, :641-670
- **Fix:** "`Severity::Error` makes `verify` exit 1 (the CI gate). It does not stop rendering: `list` adds a stderr note counting the errors, and `render-md` does not validate."

**C11. CI runs more gates than the "FIVE checks", and `--check-bump` is never named**

- **Location(s):** CLAUDE.md:1518-1530. Also CONTRIBUTING.md:193 (Tests list) and CONTRIBUTING.md:306 (CI prose names the check but not its command). BUGBOT has no `--check-bump` mention.
- **Claim:** "FIVE checks cover ground `cargo test` cannot reach, and CI runs all five."
- **Reality:** CI's `plugin` job also runs:
  - `build-plugin-zip.py --check-bump <base>` (R98: a change under `plugin/` without a version bump fails; the base is the newest reachable tag of either shape)
  - a grep that no skill body names `CLAUDE_PLUGIN_DATA`
  - a grep that no document spells `ss-magic plugin `
  - a grep that README names no `releases/latest/download/` URL
  - a build of the exact asset cargo-dist will publish
  `--check-bump` appears in no doc. The builder's other flags (`--out`, `--print-digest`, `--root`, `--plugin-dir`) are also undocumented.
- **Evidence:** .github/workflows/ci.yml:188-219, :226, :245, :268, :298-330; scripts/build-plugin-zip.py:44-50, :1448-1479
- **Fix:**
  - CLAUDE.md: keep the five local checks and add a sentence listing the extra CI-only gates.
  - CONTRIBUTING Tests section: "Before opening a PR that touches `plugin/`, run `python3 scripts/build-plugin-zip.py --check-bump <newest release tag reachable from HEAD>`."
  - BUGBOT: mention R98 `--check-bump` next to the version-bump rule.

### Low

**C12. `ensure_entry` is not called directly anywhere**

- **Location(s):** CLAUDE.md:607
- **Claim:** `ensure_entry` is "still called directly where the exact rule text is known".
- **Reality:** Its only callers are the three calls inside `ensure_path_ignored`.
- **Evidence:** crates/ss-magic-core/src/git/gitignore.rs:46, :221, :229, :233
- **Fix:** "is the building block beneath it; `ensure_path_ignored` is its only caller." See also code-suspect S10.

**C13. The main.rs bullet leaves out three routing facts**

- **Location(s):** CLAUDE.md:678-692 (main.rs bullet). Also README (the bare-menu TTY refusal is undocumented; only the cockpit's refusal at README:383 is).
- **Claim:** `Bare` routes to `tui::menu::run`.
- **Reality:**
  - `Parsed::Init(patterns)` is handled in `run()` before the update gate.
  - `Bare` is refused with exit 2 unless both stdin and stdout are TTYs (`menu_blocked_reason`).
  - `load_magic_or_exit` is the shared magic.json probe for the sync and pack flows.
- **Evidence:** crates/ss-magic/src/main.rs:83-112, :124-141, :160-180, :191-222
- **Fix:** Add the three facts to CLAUDE.md. Add one README line: the bare menu needs a terminal on both stdin and stdout, and otherwise exits 2 pointing at `--help`.

**C14. The state-tree inventory is incomplete**

- **Location(s):** CLAUDE.md:841
- **Claim:** The inventory lists `sessions/<slug>/`, `current.json`, `conclusions/`, `bypass/` and `expect-artifact/`.
- **Reality:** It omits:
  - the root `README.md` (`README_NAME`)
  - the `current.lock` pointer lock
  - the session dir's `PRE-COMPACT.md` and `research-salvage/`, which are documented elsewhere in CLAUDE.md
- **Evidence:** crates/ss-magic-plugin/src/scratchpad.rs:90, :104, :518, :594
- **Fix:** Extend the inventory with these entries.

**C15. The commit nudge has a second trigger**

- **Location(s):** CLAUDE.md:1012. Also .cursor/BUGBOT.md:974-978 and CONCEPTS.md:331-336.
- **Claim:** The nudge fires "ONLY when `git::status_porcelain` shows a candidate checklist untracked or edited-but-unstaged".
- **Reality:** `staleness()` also fires, with no git call, when a pointer-named candidate does not exist on disk.
- **Evidence:** crates/ss-magic-plugin/src/hook/pre_tool_use.rs:1198-1203, :1509-1517
- **Fix:** "... ONLY when a candidate checklist is absent from the commit: named by the pointer but never written, or shown by `git::status_porcelain` as untracked or edited-but-unstaged." In BUGBOT, change the flag to "fires when neither the pointer nor the naming convention yields a candidate".

**C16. The `pre_tool_use.rs` numbering does not match the code order**

- **Location(s):** CLAUDE.md:986
- **Claim:** The jobs are numbered (1) checklist deny, (2) Read gate, (3) commit nudge.
- **Reality:** The code runs deny, then nudge (a `Bash` call returns there), then the Read gate.
- **Evidence:** crates/ss-magic-plugin/src/hook/pre_tool_use.rs:679-697, :716-737
- **Fix:** Renumber, or note that the nudge runs first and the two never apply to the same tool.

**C17. `release_suggestion` does not call `release_check::cache_file`**

- **Location(s):** CLAUDE.md:955
- **Claim:** It reads "`release_check::cache_file`".
- **Reality:** It builds `dir.join(PLUGIN_LINE.cache_file)` from the injected `Surroundings.cache_dir`.
- **Evidence:** crates/ss-magic-plugin/src/hook/session_start.rs:394-402
- **Fix:** "(`PLUGIN_LINE.cache_file`, `plugin-release-check.json`, under the injected `Surroundings.cache_dir`)".

**C18. `Basis` has three variants**

- **Location(s):** CLAUDE.md:1169
- **Claim:** "`Basis` records which of the two priced a row."
- **Reality:** The variants are `Harness`, `Table` and `Mixed`.
- **Evidence:** crates/ss-magic-plugin/src/ledger.rs:214-224
- **Fix:** List the three variants.

**C19. "see Build" points to the wrong section**

- **Location(s):** CLAUDE.md:1362
- **Claim:** mark-latest.yml is "the post-announce latest-mark step, see Build".
- **Reality:** Build never mentions it. The procedure is in a Conventions bullet (about :1565).
- **Evidence:** CLAUDE.md:7-115, :1565-1590
- **Fix:** "see Conventions, the post-plugin-release `releases/latest` bullet".

**C20. The test-harness counter names are wrong**

- **Location(s):** CLAUDE.md:1542
- **Claim:** "its `pass`/`fail` counters".
- **Reality:** The counters are `passed`/`failed`. `pass`/`fail` are the recorder functions.
- **Evidence:** scripts/lib/test-harness.sh:13-18
- **Fix:** "its `passed`/`failed` counters, the `pass`/`fail` recorders and every `assert_*` helper".

**C21. The test-bootstrap.sh coverage list is incomplete**

- **Location(s):** CLAUDE.md:1534
- **Claim:** The listed coverage is bootstrap failure paths, the shim, and the manifest invariant.
- **Reality:** The suite also covers:
  - the `bin/ss-magic-plugin` wrapper (AE65 handoff and removed handoff, a directory or non-loadable file in place of the binary, AE9 no binary)
  - R3a seed-config
  - the AE67 disclosure
- **Evidence:** scripts/test-bootstrap.sh:547-593, :696-756, :790, :813
- **Fix:** Add a "PLUS the wrapper ..." clause.

**C22. The plugin-constraints intro counts its exceptions wrong**

- **Location(s):** CLAUDE.md:1691
- **Claim:** "Each rule below is backed by a write-up ... except the last."
- **Reality:** Only rules 1 and 2 link a write-up. Rule 3 has none either. A stray blank line also splits the list before rule 4.
- **Evidence:** CLAUDE.md:1694-1717; docs/solutions/ listing
- **Fix:** "The first two ... the third records a design posture with no write-up; the last is backed by the eight-bypass sequence." Remove the blank line.

**C23. The CONCEPTS.md description covers only half the file**

- **Location(s):** CLAUDE.md:1750
- **Claim:** CONCEPTS.md covers "the sync model".
- **Reality:** It also has a "## Claude Code plugin" section.
- **Evidence:** CONCEPTS.md:8, :150
- **Fix:** Name both halves.

**C24. The `CLAUDE_PLUGIN_DATA` fallback location is documented nowhere**

- **Location(s):** CLAUDE.md:1371 (bootstrap/wrapper paragraph). The same gap is in README, CONTRIBUTING and CONCEPTS.
- **Claim:** The binary lives in `${CLAUDE_PLUGIN_DATA}/bin/ss-magic-plugin`.
- **Reality:** When the variable is unset, both the bootstrap and `status::locate_data_dir` fall back to `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/plugins/data/ss-magic-ss-magic`.
- **Evidence:** plugin/hooks/bootstrap.sh:95-100; crates/ss-magic-plugin/src/status.rs:793-807
- **Fix:** Add one sentence to CLAUDE.md. Optionally note it in README's stores section (R4).

**C25. The hook manifest's matchers and timeouts are undocumented**

- **Location(s):** CLAUDE.md:891 (Hooks section)
- **Claim:** The section documents each handler, but not the manifest's per-event matchers and timeouts.
- **Reality:** From hooks.json:

  | Event | Matcher | Timeout |
  |---|---|---|
  | SessionStart (bootstrap) | `startup` | 90 s |
  | SessionStart (session-start) | `startup\|resume\|clear\|compact\|fork` | 10 s |
  | PreToolUse | `Read\|Edit\|MultiEdit\|Write\|NotebookEdit\|Grep\|Glob\|Bash` | 5 s |
  | PreCompact | `manual\|auto` | 10 s |
  | SubagentStop | none | 10 s |
  | SessionEnd | none | none |

  The PreToolUse matcher is the set `GateTool::from_name` must cover.
- **Evidence:** plugin/hooks/hooks.json:3-71
- **Fix:** Add a short table, or one sentence per event, to `hook/mod.rs` or the hooks section.

**C26. Several on-disk files are never named**

- **Location(s):** CLAUDE.md:1160 (ledger.rs) and the heartbeat, scratchpad and checklist entries
- **Claim:** The entries name `cost.jsonl`, `hooks.jsonl` and `current.json`.
- **Reality:** These are never named: `transcript-offsets.json` (`OFFSETS_FILE_NAME`) and the lock files `cost.lock`, `hooks.lock`, `current.lock`, `checklist.lock` and `file-changed.lock`. `install.lock`, `magic-json.lock` and `release-check.lock` are named.
- **Evidence:** crates/ss-magic-plugin/src/ledger.rs:86, :96; heartbeat.rs:67; scratchpad.rs:104; checklist/verbs.rs:105; hook/file_changed.rs:106
- **Fix:** Name the offsets file in the ledger entry. Optionally add one "lock files under the temp root" sentence to tmproot.rs.

Cross-refs into CLAUDE.md from other entries:

- the `config.rs` gate-knob semantics (R6)
- `status --all` / `conclude --from` (R5)
- the `MARK_LATEST_TAG`/`MARK_LATEST_REPO` interface (N5)
- the backups-location wording at CLAUDE.md:485 (K1)

---

## README.md

### High

**R1. The plugin makes a second network access**

- **Location(s):** README.md:766
- **Claim:** "Nothing is sent anywhere ... the only network access the plugin makes is the bootstrap's download of the pinned binary."
- **Reality:** `release-check --refresh` constructs a ureq client and GETs GitHub's `/releases` list with a 5 s timeout. SessionStart spawns it detached when the cache is missing or older than 24 h, only when the session is not quiet and a pin exists. README:668-698 describes this, so the two sections contradict each other.
- **Evidence:** crates/ss-magic-plugin/src/release_check.rs:57, :739; hook/session_start.rs:439-443; crates/ss-magic-core/src/release.rs:53, :57
- **Fix:** "Nothing about you or your repository is sent anywhere: every file above is local. The plugin touches the network in two places only: the bootstrap's download of the pinned binary, and a background `release-check --refresh` that reads the public release list from the GitHub API at most once a day, never in a headless session, with a 5 s budget."
- **Note:** The confirmed pass rated this medium and the coverage agent rated it high. It is raised to high because it is a user-facing egress claim.

### Medium

**R2. "Done" does not leave the old layout intact**

- **Location(s):** README.md:255
- **Claim:** Changes are materialized only after a non-cancel choice, "so picking 'done' or aborting leaves the old layout intact".
- **Reality:** "Done for now" is a non-cancel choice. It stages and materializes the new layout uncommitted. Only Esc/Ctrl-C at the prompt leaves the old layout intact.
- **Evidence:** crates/ss-magic/src/workspace/migrate.rs:17-24, :347-361, :503-527, :605-611
- **Fix:** "Nothing is written until the finishing-action prompt returns. Esc / Ctrl-C there leaves the old layout intact; any choice, 'Done for now' included, then stages the changes and materializes them in one step. 'Done' leaves them on disk, uncommitted."

**R3. "What it stores, and where" leaves out several stores**

- **Location(s):** README.md:757-766. The confirmed finding and the raw finding are merged here.
- **Claim:** The section lists only `.superset/.magic/` (scratchpad, conclusions, bypass and expect-artifact records) and a per-machine data dir (heartbeat, cost ledger), then says "every file above is local".
- **Reality:** The plugin also writes:
  - In the state tree: `PRE-COMPACT.md`, `research-salvage/`, and the `current.json` and `checklist.json` pointers.
  - In the OS cache dir (`~/Library/Caches/ss-magic`, `~/.cache/ss-magic`): `plugin-release-check.json` (with the `suggested` marker) and `compact-advice-shown`.
  - In the private temp root `/tmp/ss-magic-plugin/<id>/` (or `$TMPDIR`): the locks and the binary handoff file.
  - In the data dir: `transcript-offsets.json`.
  - On request: `.claude/settings.local.json` (`compact-window --set`), `docs/actions/*.checklist.json`, and `.github/workflows/ss-magic-checklist.yml`.
- **Evidence:** crates/ss-magic-plugin/src/hook/pre_compact.rs:52; hook/subagent_stop.rs:64; release_check.rs:90-92; hook/session_start.rs:94, :293-302; tmproot.rs:97-146; ledger.rs:86; compact_window.rs:89; setup_ci.rs:75
- **Fix:**
  - Add a bullet for the OS cache dir.
  - Add a bullet for the temp root.
  - Name `PRE-COMPACT.md` and `research-salvage/` under the state tree.
  - Add a short "files it writes only when you ask" line covering the settings file, the checklist and the workflow.

**R4. The Environment variables table is missing variables the plugin reads**

- **Location(s):** README.md:893-904. The confirmed findings and both raw findings are merged here.
- **Claim:** The table lists NO_COLOR, SS_MAGIC_NO_UPDATE, SS_MAGIC_UPDATED, CLAUDE_CONFIG_DIR, CLAUDE_PLUGIN_ROOT and CLAUDE_PLUGIN_DATA.
- **Reality:** The plugin also reads:
  - `CLAUDE_CODE_ENTRYPOINT`: a non-empty value other than `cli` puts the session in quiet mode, which suppresses the compaction advice, the update notice and the background refresh.
  - `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`: from the environment and from the settings files' `env` blocks. It is mentioned in prose at :634 only.
  - The six `GIT_*` variables in `DECLINING_ENV`: they make hook root discovery fall back to `git rev-parse`. `GIT_DIR` alone is mentioned at :721.
  - `HOME` (temp-root identifier) and `TMPDIR` (temp-root fallback).
  `CLAUDE_ENV_FILE` is read only by the inert FileChanged handler, so its absence from the table is correct.
- **Evidence:** crates/ss-magic-plugin/src/hook/mod.rs:128, :158-170; compact_window.rs:100, :368; hook/session_start.rs:137-138; crates/ss-magic-core/src/git/discover.rs:119-130; tmproot.rs:146; plugin/lib/tmproot.sh:16-25
- **Fix:** Add rows for `CLAUDE_CODE_ENTRYPOINT` and `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`, worded as in the confirmed findings. Add one combined row for the `GIT_*` set ("makes hook discovery fall back to `git`; visible as `discovery: fallback` in `status`"). Optionally add `HOME`/`TMPDIR`.

**R5. Verb flags are undocumented**

- **Location(s):** README.md:589-612 (Verbs block) and :622 (the `--json` sentence). The same gap is in CLAUDE.md:1141 (cache.rs) and :1182 (status.rs).
- **Claim:** The block shows bare `status`, bare `cost`, `conclude <FILE>` and `release-check [--refresh] [--json]`.
- **Reality:** These flags exist but are not documented:
  - `status --all`: every heartbeat row, not just this worktree's. It appears in no doc.
  - `cost --here` and `cost --backfill <REF>`: REF is a session id or a transcript path.
  - `conclude <FILE> --from BODY_FILE`: otherwise the body is read from stdin. It appears in no doc.
  - `release-check --quiet`.
  - `setup-github-ci -n` / `-f`.
- **Evidence:** crates/ss-magic-plugin/src/status.rs:1860-1887; ledger.rs:1190-1227; cache.rs:733-775; release_check.rs:660-663; setup_ci.rs:207-208
- **Fix:**
  - README: `ss-magic-plugin cost [--here] [--backfill REF] [--json]`, `status [--all] [--json]`, `conclude <FILE> [--from BODY_FILE]` (otherwise stdin), `release-check [--refresh] [--json] [--quiet]`.
  - CLAUDE.md: add `status --all` and `conclude --from` to their entries.

**R6. The gate knobs have no meaning, default or clamp range in any doc**

- **Location(s):** README.md:724-753 (Configuration). The same gap is in the CLAUDE.md `config.rs` entry.
- **Claim:** The section shows `threshold_lines`, `inline_byte_budget` and `exemptions`, and says "out-of-range number is clamped".
- **Reality:**
  - `threshold_lines`: default 3000, clamped to 500..=20000. It is the size above which a Read is gated.
  - `inline_byte_budget`: default 10000, clamped to 1000..=100000. It governs only the inline cached conclusion in `additionalContext`.
  - `exemptions`: empty by default.
- **Evidence:** crates/ss-magic-plugin/src/config.rs:75-135 (`GATE_*_DEFAULT/MIN/MAX`)
- **Fix:** Add a three-row knob table (meaning, default, range) to README, and the ranges to CLAUDE.md.

**R7. The shipped skills are not enumerated**

- **Location(s):** README.md:441-470 and :583. The same gap is in CLAUDE.md:85 (`skills/` only) and CONTRIBUTING.md's layout.
- **Claim:** "the shipped skills invoke them".
- **Reality:** Three skills ship:
  - `scratchpad`
  - `operator-checklist`, with `reference.md`
  - `setup-github-ci`, which branches on the `state: absent|identical|differs|pin-stale` token from `setup-github-ci --check`
- **Evidence:** plugin/skills/{scratchpad,operator-checklist,setup-github-ci}/SKILL.md; crates/ss-magic-plugin/src/setup_ci.rs:158-161, :265
- **Fix:** Add a three-line skills list to README naming each skill and the verbs it drives. Name the three skills in CLAUDE.md's `plugin/` tree description.

### Low

**R8. `update` has a fourth outcome**

- **Location(s):** README.md:433-439. Also README.md:889-890 (Self-update, Escape hatches).
- **Claim:** `update` reports the resulting version, "already latest", or that it could not check.
- **Reality:** When another updater holds the lock, it prints "Another update is already in progress; skipped." (`UpdateReport::Skipped`, exit 0).
- **Evidence:** crates/ss-magic/src/main.rs:437-445; update/mod.rs:52-53, :123
- **Fix:** Add "that another update is already in progress (skipped, try again)" in both places.

**R9. The version-drift notice is undocumented**

- **Location(s):** README.md:706 (SessionStart row). The same gap is in CONCEPTS.md:295 (K5).
- **Claim:** SessionStart "on a fresh start only" carries two notices.
- **Reality:** There is also a third, `version_drift_notice`. It runs on every source and ignores quiet mode.
- **Evidence:** crates/ss-magic-plugin/src/hook/session_start.rs:174-187, :311-330
- **Fix:** "... and, on any start, a one-line notice when the running binary is not the version the loaded plugin pins."

**R10. seed-config runs on every fresh session**

- **Location(s):** README.md:493. Also the verb listing at about :603 ("(what the bootstrap runs once)"). The stale code comment is S5.
- **Claim:** It runs "right after a successful install".
- **Reality:** `seed_config` is called from three sites: the fast path, the re-check under the lock, and the end of a fresh install. So it runs on every `startup` with a usable binary, and writes only the first time. README:533 says this correctly, so README contradicts itself.
- **Evidence:** plugin/hooks/bootstrap.sh:252-259, :350-353, :478
- **Fix:** "On every fresh session start where the pinned binary is usable, the bootstrap also runs it (it writes only the first time)". In the listing: "(the bootstrap runs it each session start; it writes only once)".

**R11. The `setup-github-ci` workflow and the checklist format are underdocumented**

- **Location(s):** README.md:612 and :465
- **Claim:** README says only that it writes "the checklist PR-comment workflow".
- **Reality:** README never says:
  - that the workflow is written to `.github/workflows/ss-magic-checklist.yml`
  - that it pins and SHA-verifies an `ss-magic-plugin-vX.Y.Z` release
  - that it runs a read-only `render` job and a `pull-requests: write` `comment` job
  - that it skips fork PRs
  - that the checklist document is `docs/actions/<YYYY-MM-slug>.checklist.json`
- **Evidence:** crates/ss-magic-plugin/src/setup_ci.rs:75; assets/workflow/checklist.yml:50, :81, :183
- **Fix:** Add a two-sentence paragraph.

Cross-refs into README.md:

- C2 (README:248, three rules)
- C3 (README:236 and :425-431, legacy skills removal)
- C13 (bare-menu TTY refusal)
- C24 (data-dir fallback)

---

## CONTRIBUTING.md

### Medium

**N1. The sample `--check` output is one line short**

- **Location(s):** CONTRIBUTING.md:213-220 (sample block) and :223-230 ("In order:" prose). Two confirmed findings and two raw findings are merged here.
- **Claim:** The sample shows seven `ok` lines, and the prose walks through seven assertions.
- **Reality:** `--check` prints eight lines. `release gate blocks publishing` comes between `workspace shape` and `R96 committed digest pin`. CLAUDE.md:1522 says EIGHT correctly.
- **Evidence:** scripts/build-plugin-zip.py:794-857, :1500-1510. Running `--check` in this tree prints eight lines.
- **Fix:**
  - Insert `  ok   release gate blocks publishing` in the block.
  - Add a prose clause before the digest: "the generated `release.yml`'s `host` job needs the `custom-ci` test gate and checks its result, along with every job `build-local-artifacts` waits on (see [A red release gate](#a-red-release-gate))".

**N2. Nothing says that `release.yml` is generated**

- **Location(s):** CONTRIBUTING.md:386 (cargo-dist section)
- **Claim:** The section describes `dist-workspace.toml`, the installer and attestations.
- **Reality:** The section never says that `.github/workflows/release.yml` is generated and must not be hand-edited, or that `dist generate --check` must stay green. CLAUDE.md:1589-1591 has both rules. CONTRIBUTING also does not give `dist build --artifacts=global` as the way to verify an extra-artifacts change (CLAUDE.md Build section).
- **Evidence:** CLAUDE.md:1589-1591; `.github/workflows/release.yml` header; `grep 'dist generate' CONTRIBUTING.md` finds nothing.
- **Fix:** Add: "`release.yml` is generated by cargo-dist; never hand-edit it. After any `dist-workspace.toml` or `[package.metadata.dist]` change, run `dist generate` and keep `dist generate --check` green; verify extra-artifacts changes with `dist build --artifacts=global`, not `dist plan`."

### Low

**N3. The cockpit is misnamed as the reverse-sync cockpit**

- **Location(s):** CONTRIBUTING.md:103, :111, :287. Also .cursor/BUGBOT.md:1396 and the module docs in merge.rs:1 and cockpit.rs:1 (S12).
- **Claim:** It is called the "reverse-sync merge cockpit" and the "reverse-sync ... decision model".
- **Reality:** It is the unified Sync cockpit, opened by `reverse_sync::run` from the worktree menu's Sync entry, and it works in both directions. `ss-magic reverse-sync` is `run_bulk` and never opens it.
- **Evidence:** crates/ss-magic/src/sync/reverse_sync.rs (`run` vs `run_bulk`); CONCEPTS.md:62-67
- **Fix:** Rename to "unified Sync (merge) cockpit" and describe the model as "push/pull/merge/delete".

**N4. `state_tree.rs` has a second lazy caller**

- **Location(s):** CONTRIBUTING.md:93
- **Claim:** It is called "lazily by the plugin's `enable` verb".
- **Reality:** A `config set` that turns `plugin.enabled` on also calls it.
- **Evidence:** crates/ss-magic-plugin/src/config.rs:635-637, :747-749
- **Fix:** Add "... and by a `config set` that turns `plugin.enabled` on".

**N5. The `mark-latest.sh` environment interface is undocumented**

- **Location(s):** CONTRIBUTING.md:486. Also CLAUDE.md:1583.
- **Claim:** Only `MARK_LATEST_DRY_RUN` is named, and only in CLAUDE.md.
- **Reality:**
  - `MARK_LATEST_TAG` holds the tag (falling back to `GITHUB_REF_NAME`).
  - `MARK_LATEST_REPO` defaults to ViktorStiskala/superset-magic.
  - Exit codes: 2 for no tag, 1 when the mark did not take.
- **Evidence:** scripts/mark-latest.sh:17-63
- **Fix:** Add one sentence with the three variables and the exit codes (needed for the documented `workflow_dispatch` fallback and for local dry runs).

**N6. The ledger benchmark variable is undocumented**

- **Location(s):** CONTRIBUTING.md:191 (Tests)
- **Claim:** The section lists the commands for running the suites.
- **Reality:** The `#[ignore]`d ledger benchmark panics unless `SS_MAGIC_LEDGER_BENCH_TREE` names a transcript `.jsonl`. `cargo test -- --ignored` therefore fails with no documented cause.
- **Evidence:** crates/ss-magic-plugin/src/ledger/tests.rs:1111-1118
- **Fix:** One line: the variable, what it points at, and that it is needed only for `--ignored` runs.

Cross-refs into CONTRIBUTING.md:

- C11 (`--check-bump` at :193 and :306)
- R7 (skills list)

---

## CONCEPTS.md

### Low

**K1. Cockpit backups do not live under the root being overwritten**

- **Location(s):** CONCEPTS.md:81-82. Also CLAUDE.md:485 and .cursor/BUGBOT.md:323-327 ("Backups live under the root being OVERWRITTEN").
- **Claim:** Backups live under the `.superset/backups/` "of the root being overwritten".
- **Reality:** The per-direction mapping is right, but the stated rule is not. The cockpit always backs up under the WORKTREE, including main's losing bytes in `<ts>/main/...`, on purpose so recovered secrets are never committed. Only forward sync and the bulk `reverse-sync` back up under the tree they overwrite.
- **Evidence:** crates/ss-magic/src/sync/reverse_sync.rs:374-377, :556, :622
- **Fix:**
  - CONCEPTS: "in the worktree for the merge cockpit (for both sides' losing bytes) and forward sync, and in main for the direct `ss-magic reverse-sync`".
  - CLAUDE.md and BUGBOT: reword the rule header to "Backups live under the root each flow designates" and keep the mapping.

**K2. Not every initialization runs behind a finishing prompt**

- **Location(s):** CONCEPTS.md:27
- **Claim:** "Both stage the whole tree ... behind a finishing prompt."
- **Reality:** `ss-magic init [PATTERN...]` (`run_init_noninteractive`) stages the tree and materializes it with no prompt.
- **Evidence:** crates/ss-magic/src/workspace/migrate.rs:349, :503, :546-575
- **Fix:** Say that the interactive flows prompt and that scripted init skips the prompt.

**K3. Only the wrapper depends on the handoff**

- **Location(s):** CONCEPTS.md:223
- **Claim:** The wrapper and the shim "both find the binary by reading the handoff".
- **Reality:** The shim uses `${CLAUDE_PLUGIN_DATA}` and falls back to the handoff only when the variable is empty. This is consistent with the first clause of the same sentence.
- **Evidence:** plugin/hooks/run-hook.sh:53-75; plugin/bin/ss-magic-plugin:5, :57-84
- **Fix:** Say that only the wrapper depends on the handoff and that the shim falls back to it.

**K4. The Read gate has a sixth escape hatch**

- **Location(s):** CONCEPTS.md:255
- **Claim:** The escape-hatch list has five entries.
- **Reality:** Any Read inside `.superset/.magic/` is allowed unconditionally (decision step 5).
- **Evidence:** crates/ss-magic-plugin/src/hook/pre_tool_use.rs:689, :739-745
- **Fix:** Add "anything inside the plugin's own state tree (so scratchpad notes can always be re-read after a compaction)".

**K5. Session-start notices and several plugin terms are missing from the glossary**

- **Location(s):** CONCEPTS.md:285-304 (Version pin, Release line) and the whole plugin glossary (:150 onward). The confirmed finding and the raw finding are merged here.
- **Claim:** Version-pin drift is the only session-start operator notice the section defines.
- **Reality:**
  - SessionStart sends three notices: drift, compaction advice (once per machine) and the release suggestion (once per release, from a cache refreshed in the background).
  - Quiet mode suppresses the last two.
  - These terms are also undefined: quiet mode, compaction window / peak-context recommendation, release notice / release check, "page-fault gate" (the name in plugin.json and config.rs for what CONCEPTS calls the Read gate), and session identity (the `<repo>-<branch>` slug).
- **Evidence:** crates/ss-magic-plugin/src/hook/session_start.rs:11-16, :173-186; hook/mod.rs:128-170; compact_window.rs:100-105; plugin/.claude-plugin/plugin.json:4; config.rs:11, :77; identity.rs
- **Fix:**
  - Add an "Operator notice" entry (the three notices plus quiet mode).
  - Add entries for "Quiet mode", "Compaction window" and "Session identity".
  - Add "also called the page-fault gate" to the Read gate entry.
  - Optionally add to Release line that the plugin advises about a newer release but never installs it.

Cross-refs into CONCEPTS.md:

- C7 (salvage, :327)
- C15 (commit nudge, :331-336)
- C9 ("renderable to byte-identical Markdown wherever it appears (the CLI, a commit-time nudge, or a pull-request comment)", :273-274)

---

## .cursor/BUGBOT.md

### Medium

**B1. `workspace/superset_files.rs` does not exist**

- **Location(s):** .cursor/BUGBOT.md:158 (pure-modules list), :1211 (heading), :1389 (unit-tested list)
- **Claim:** The path `workspace/superset_files.rs`.
- **Reality:** The module is core's `superset_files.rs`. The CLI's `workspace/mod.rs` only re-exports it, and the CLI's `workspace/` holds only `mod.rs` and `migrate.rs`. BUGBOT's own :52-55 already places the module in core.
- **Evidence:** crates/ss-magic/src/workspace/mod.rs:8; crates/ss-magic-core/src/superset_files.rs
- **Fix:** Replace all three with "`superset_files.rs` (core; the CLI reaches it as `crate::workspace::superset_files`)". The heading becomes "## Config Files (core's `superset_files.rs`)".

**B2. Pack walks a real directory with `append_dir_excluding_trees`**

- **Location(s):** .cursor/BUGBOT.md:369
- **Claim:** "a real dir → `append_dir_all`".
- **Reality:** A real directory goes to `append_dir_excluding_trees`, the guarded walk. The doc's own excluded-tree bullet a few lines later (about 388-401) says the same, so the two bullets contradict each other.
- **Evidence:** crates/ss-magic/src/pack.rs:298-309, :358
- **Fix:** "a real dir → `append_dir_excluding_trees` (the guarded walk that prunes every excluded tree, never a blind `append_dir_all`)".

**B3. The zero-count check lives in `pack_core`, not main.rs**

- **Location(s):** .cursor/BUGBOT.md:379
- **Claim:** "`main.rs` suppresses `PackEvent::Done` at zero".
- **Reality:** `pack_core` prints "No packable files remained" and returns without ever emitting `Done`.
- **Evidence:** crates/ss-magic/src/pack.rs:200-212; main.rs:240-268
- **Fix:** Reword, and flag a change that moves the check into rendering or emits a zero-count `Done`.

**B4. The mode-bit rule is wrong for two files**

- **Location(s):** .cursor/BUGBOT.md:644
- **Claim:** 0600/0700 for "anything machine-local"; 0644 "only" for the CI workflow.
- **Reality:**
  - `compact-window --set` creates a new `.claude/settings.local.json` at 0644 on purpose. The file is machine-local but is a harness settings file, not private state.
  - A new checklist document is also created 0644.
- **Evidence:** crates/ss-magic-plugin/src/compact_window.rs:127-132; setup_ci.rs:82-84; checklist/verbs.rs:107-111
- **Fix:** As in the confirmed fix: 0600/0700 for private plugin state; 0644 for committed content and for a new `settings.local.json`.

**B5. CI does not check skills for a bare `ss-magic`**

- **Location(s):** .cursor/BUGBOT.md:1141
- **Claim:** "No skill body may name `${CLAUDE_PLUGIN_DATA}` or a bare `ss-magic`; CI asserts both."
- **Reality:** CI asserts only the `CLAUDE_PLUGIN_DATA` grep and the `ss-magic plugin ` grep. Skill prose legitimately names the sync CLI (setup-github-ci/SKILL.md:12, :47).
- **Evidence:** .github/workflows/ci.yml:221-258
- **Fix:** Say that CI asserts `CLAUDE_PLUGIN_DATA` and the subcommand form, and that "no skill may tell the model to run a bare `ss-magic` command" is a review rule, not a CI check.

**B6. `--check` does not repeat the hooks-shim invariant**

- **Location(s):** .cursor/BUGBOT.md:1047-1055. The code side is S1.
- **Claim:** `--check` "repeats" the test-bootstrap.sh walk. The doc also says that a check reading only `args[0]` is not enough.
- **Reality:** `check_hooks_shim` reads only `type`, `command` and `args[0]`. Only test-bootstrap.sh asserts the `args[1]` token, the two-argument shape and that the target exists.
- **Evidence:** scripts/build-plugin-zip.py:555-628; scripts/test-bootstrap.sh:617-683
- **Fix:** Either port the checks into `check_hooks_shim` (preferred; raise with the user), or reword: "`--check` asserts the command/args[0] half; the token and existence checks live only in test-bootstrap.sh".

### Low

**B7. The pack summary says "files", not "entries"**

- **Location(s):** .cursor/BUGBOT.md:386
- **Claim:** The rule refers to a message that says "entries".
- **Reality:** The summary reads "Packed N files".
- **Evidence:** crates/ss-magic/src/main.rs:258
- **Fix:** "while the summary still says 'Packed N files'".

**B8. `sync/reverse_sync.rs` has no finishing-action prompt**

- **Location(s):** .cursor/BUGBOT.md:172
- **Claim:** "finishing-action prompts in `workspace/migrate.rs` / `sync/reverse_sync.rs`".
- **Reality:** Only migrate.rs calls `pick_final_action`. reverse_sync.rs is interactive only through the cockpit.
- **Evidence:** crates/ss-magic/src/workspace/migrate.rs:42, :349, :503; sync/reverse_sync.rs:58, :318, :362
- **Fix:** "..., and `sync/reverse_sync.rs::run` (launches the cockpit behind `is_interactive`)".

**B9. Dependency purposes are incomplete**

- **Location(s):** .cursor/BUGBOT.md:62
- **Claim:** `similar` is for the diff model and merge; `ureq` is for self-update.
- **Reality:**
  - The plugin also uses `similar` for the `setup-github-ci` preview.
  - `ureq` is a core dependency for the per-line release check, used by the CLI updater and by `release-check --refresh`.
- **Evidence:** crates/ss-magic-plugin/Cargo.toml:26-29; setup_ci.rs:407; crates/ss-magic-core/Cargo.toml:24
- **Fix:** Reword as in the confirmed finding.

**B10. Quoted cockpit strings use an en dash, the code uses an em dash**

- **Location(s):** .cursor/BUGBOT.md:512
- **Claim:** The strings are quoted with an en dash.
- **Reality:** The code renders them with an em dash, so a grep for the quoted text finds nothing.
- **Evidence:** crates/ss-magic/src/tui/cockpit.rs:984, :994
- **Fix:** Quote only the dash-free tail ("…will be created in main").

**B11. The tests for the four rules live in more files than named**

- **Location(s):** .cursor/BUGBOT.md:695
- **Claim:** "each a test in `release_check/tests.rs` or `hook/session_start/tests.rs`".
- **Reality:** The source scan is in `hook/tests.rs`, and the carry-forward tests are in core's `release/tests.rs`.
- **Evidence:** crates/ss-magic-plugin/src/hook/tests.rs:1031-1051; crates/ss-magic-core/src/release/tests.rs:664-760
- **Fix:** List all four files.

**B12. A check prints a report, not one line**

- **Location(s):** .cursor/BUGBOT.md:1180
- **Claim:** "a check writes one cache file and prints one line".
- **Reality:** The verb prints a multi-row report, or JSON with `--json`, or nothing with `--quiet`.
- **Evidence:** crates/ss-magic-plugin/src/release_check.rs:575-620
- **Fix:** "the verb prints a report, the hook emits one operator line; it never downloads or installs anything."

**B13. No rule covers `setup-github-ci`**

- **Location(s):** .cursor/BUGBOT.md:557 onward (Plugin rules)
- **Claim:** The Plugin rules section has no rule for `setup-github-ci` / `setup_ci.rs` or the workflow it writes.
- **Reality:** BUGBOT never mentions `setup_ci`, `setup-github-ci`, `checklist.yml` or `SS_MAGIC_PLUGIN_VERSION`. The template's plugin-line pin is load-bearing per CLAUDE.md.
- **Evidence:** crates/ss-magic-plugin/src/setup_ci.rs:62-84; assets/workflow/checklist.yml:65-69, :116
- **Fix:** Add the short rule from the confirmed finding. Flag:
  - a bare `v$VERSION` tag
  - a CLI archive name
  - a dropped checksum step
  - a mode other than 0644

Cross-refs into BUGBOT.md:

- C1 (:88-93)
- C2 (:323-335)
- K1 (:323-327)
- C15 (:974-978)
- C9 (:911-912)
- C11 (`--check-bump`)
- N3 (:1396)

---

## docs/runbooks/forge-tag-and-release-protection.md

### Medium

**F1. The runbook names the wrong newest release**

- **Location(s):** runbook :244-247 (and the example at :252)
- **Claim:** "the newest published CLI release is `v0.11.0` and no plugin release has been cut at all".
- **Reality:** The tags `v0.11.1`, `v0.11.2`, `ss-magic-plugin-v1.0.0` (empty) and `ss-magic-plugin-v1.0.1` all exist. The runbook contradicts itself at :162-163, :207-209 and :271-274.
- **Evidence:** `git tag`; crates/ss-magic/Cargo.toml:3; crates/ss-magic-plugin/Cargo.toml:3
- **Fix:** "Substitute a tag that actually exists on each line, such as `ss-magic-plugin-v1.0.1` and `v0.11.2` (or `v0.11.0`, the release the 2026-09-08 check ran against)." Optionally update the :252 example.

---

## Code-suspect items

These are code comments or code behavior to raise with the user. They are not doc edits. All were verified.

| # | Location | Issue | Severity | Suggested action |
|---|---|---|---|---|
| S1 | scripts/build-plugin-zip.py:555-628 (`check_hooks_shim`) | Does not assert the shim event token (`args[1]`), the two-argument shape, or that the target exists, so the "one-command release gate" passes a wrong-token manifest. Only test-bootstrap.sh catches it. | medium | Port SHIM_TOKENS plus the `len(args)==2` and `args[1]` checks into `check_hooks_shim`. A plugin version bump is not needed (script only). |
| S2 | assets/magic.sh:36 (embedded via include_str!, written into every consuming repo) | The install hint uses the unpinned `releases/latest/download/ss-magic-installer.sh`, which README says can 404 right after a plugin release. The CI and BUGBOT guards grep only README. | low | Drop the curl line, or template in the writing binary's tag. Extend the ci.yml grep and the BUGBOT rule to assets/magic.sh. Bump the CLI (output change). |
| S3 | crates/ss-magic-plugin/src/hook/mod.rs:54-55 | The module header says the state-tree rule is written by "`plugin enable`" via `ss-magic`. That subcommand no longer exists. | low | "written only by `ss-magic init` / `migrate` and `ss-magic-plugin enable` / `config set`, never by a hook". |
| S4 | crates/ss-magic-plugin/src/hook/mod.rs:49-51 | Claims the discovery fallback rate is readable from `status` (see C6). | low | Point at `grep -c 'discovery: fallback'` over `hooks.jsonl`. |
| S5 | crates/ss-magic-plugin/src/config.rs:501-502 | The `run_seed_config` doc says the bootstrap invokes it "once, right after it installs the binary". There are three call sites, and it runs every session. | low | Fix the comment. |
| S6 | crates/ss-magic-plugin/src/checklist/validate.rs:29-31 | The `Severity::Error` doc says "the renderer is never handed the file". `list` and `render-md` render regardless (see C10). | low | Fix the comment. |
| S7 | crates/ss-magic-plugin/src/checklist/render.rs:3-6 | The module doc lists five render consumers. There are two (see C9). | low | Fix the comment. |
| S8 | crates/ss-magic/src/update/mod.rs:4 (and :28) | The intra-doc link `[check]` names a module that moved to core's `release.rs`. | low | Relink to `ss_magic_core::release`. |
| S9 | crates/ss-magic/src/cli.rs:4-5 | The module doc lists `sync`, `pack`, `update` and `init`, and omits `reverse-sync`. | low | Add it. |
| S10 | crates/ss-magic-core/src/git/gitignore.rs:46 | `ensure_entry` is `pub`, but its only callers are inside `ensure_path_ignored`. | low | Consider `pub(crate)` or private. |
| S11 | crates/ss-magic-core/src/git/mod.rs:145, :182, :195 | `nothing_to_commit`, `gh_available` and `pr_create` spawn `Command` directly, which BUGBOT's rule (C1) forbids. | low | Decide: relax the rule (recommended, the reasons are legitimate) or route them through a helper. |
| S12 | crates/ss-magic/src/sync/merge.rs:1, crates/ss-magic/src/tui/cockpit.rs:1 | The module docs say "reverse-sync merge cockpit" (see N3). | low | Rename to the unified Sync cockpit. |
| S13 | crates/ss-magic/src/main.rs (`should_run_update_gate` doc comment) | Narrates the old plugin sibling-arm routing. | low | Current-state wording. |
| S14 | crates/ss-magic-plugin/src/status.rs:66-74 (`SCHEMA_VERSION` comment) | Narrates the migration from shape 1. | low | Optional: current-state wording, matching the CLAUDE.md fix. |

---

## Historical-narration passages

These describe how the code or the doc changed, instead of what is true now. Grouped by region, because the planned CLAUDE.md restructure rewrites most of these regions. Each item gives the line, the phrase to remove or rewrite, and, in brackets, any current-state fact that must survive.

### CLAUDE.md (44)

**Group 1: Build and release (lines 1-115)**

- :70 "which now covers BOTH archive names"
- :71 "unchanged and still trusts" [self-update trusts TLS and cargo-dist checksums, not attestations]
- :108 'this doc said "four" while the script checked seven' [keep "do not work from a remembered count"]

**Group 2: Architecture, crate layout (lines 150-180)**

- :154 "extracted from `pack.rs`"
- :157 "formerly `update/check.rs`"
- :168 "under their old `crate::` names ... before the split"
- :172-180 "the former `crates/ss-magic/src/plugin/` tree moved wholesale ... the two names that MOVED ... used to borrow it" [keep: the plugin has no `tui/`, so it reaches `ss_magic_core::style` and `reponame` by real paths]

**Group 3: Module bullets with removed-code parentheticals (lines 210-340)**

- :210 "probe/Mode dispatch was removed in U13"
- :304 "generalizes the retired `reverse_sync::under_backups_dir`"
- :318 "`load_main_config` ... was removed in U13"
- :326 "the `inquire` half of the old `style`"
- :337 "setup-command picker ... removed in U13"

**Group 4: Cockpit changelog (lines 385-404)**

- "This run made four TUI changes: (1)...(4)", "now uses", "y/n bindings were dropped" [restate all four as present-tense cockpit properties]

**Group 5: cli.rs, menu and merge (lines 423-539)**

- :423 "deleted outright ... now takes" [no token, no alias, exit 2]
- :431 "It used to STOP at the plugin token" [keep "do not add a plugin token or a scan stop back"]
- :439 "the old `forward_sync_in_worktree` handler are gone"
- :539 "which now returns `Undecided`"

**Group 6: update, release, hashing and main.rs (lines 618-688)**

- :618 "the former `update/check.rs`, moved verbatim"
- :623 "no longer identifies"
- :637 "byte-identical to before" [the key is never written on the CLI line, so an older binary still parses the file]
- :677 "Replaces the removed `reverse_sync::hash_file`"
- :682 "The plugin no longer appears here ... used to be routed in a SIBLING arm"
- :688 "which now runs a pre-copy backup pass"

**Group 7: Plugin section intro and entry point (lines 703-761)**

- :703 "the former ... tree moved wholesale"
- :725 "Since the split this is STRUCTURAL rather than a routing rule"
- :740 "formerly `mod.rs`"
- :761 "`HumanVerb` gained ... `writes_config` used to double as" [keep the warning not to derive hook-reachability from `writes_config`]

**Group 8: State modules (lines 781-864)**

- :781 the atomic.rs refactor history "used to live in heartbeat.rs ... moved out ... nothing here changed" (see C5 for the factual half)
- :864 "before the split it reached into this module ... now gone"

**Group 9: Hooks (lines 1020-1025)**

- :1020 "because the checklist deny carried the wrapper from the start while the size gate did not"
- :1025 "Usage: strings USED to be a deliberate exception ... not an exception any more"

**Group 10: Human verbs (lines 1109-1263)**

- :1109 "exists because removing the CLI's `plugin` subcommand removed the last terminal path" [no terminal path exists: the binary is off PATH and the CLI has no subcommand]
- :1192 "both renamed with the split", "still runs"
- :1200 "`Versions` section gains"
- :1211 "`SCHEMA_VERSION` is `2` since the split: `versions.cli` became ..." [`2`, bumped when a key changes meaning or goes away; list the current keys]
- :1255 "(R29's literal wording named the reload; see the plan's amendment note)"
- :1263 "now on the PLUGIN's line throughout ... That is not cosmetic" [the template must use the plugin tag because the two lines' versions differ]

**Group 11: Non-Rust assets and shell pieces (lines 1390-1458)**

- :1390 "the correction of a real defect" [called from three sites: installed once per machine, seeded once per repository]
- :1433 "The check was added to the shim first and the wrapper kept the weaker test"
- :1444 "no `plugin` verb to inject any more"
- :1458 "legacy `assets/setup.sh` was deleted in U13" [the binary is the sole file-copy implementation]

**Group 12: Conventions and constraints (lines 1473-1657)**

- :1473 "originally derived from the retired `setup.sh` ... Now owned by" [owned by core's `sync/apply.rs` and `sync/pattern.rs`; qualify as core]
- :1489 "the former `src/tests/support.rs`"
- :1518 "is no longer the whole suite", "now prints EIGHT"
- :1598 the 2026-09-30 incident narrative, "The gate is now" [the gate is `local-artifacts-jobs`, never `plan-jobs`; the two empty pre-releases exist; a failed publish burns its version; the full story is in the linked write-up]
- :1657 "the real incident this run fixed"

### README.md (1)

- :218 "The old `plugin` token was removed outright rather than aliased, so typing it now produces ..." [`ss-magic plugin` is an unknown subcommand like any other]

### CONTRIBUTING.md (7)

- :68 "the token was removed, not aliased"
- :106 "so `crate::sync::apply` still resolves" [the re-exports list]
- :120 "under their old `crate::` names"
- :187 "It was removed outright" [keep the do-not-add rule and its rationale]
- :256 "before the shared `lib/execguard.sh` landed" [both cases must be refused by execguard before `exec`]
- :344 'this file once said "four" while the script checked seven'
- :527 the 2026-09-30 incident narrative [the two empty pre-releases exist, the versions can never be reused, and `plan-jobs` lets `host` publish an empty release]

### .cursor/BUGBOT.md (15)

- :43 "deleted outright when the plugin became its own binary ... now takes"
- :53 "under the names it used before the split" [the CLI re-exports under `crate::` paths; the plugin re-exports only `git` and `hashing`]
- :124 "no `Plugin` variant any more"
- :135 "The scan used to STOP at a `plugin` token" [whole-argv scan, no stop token]
- :181 "no longer appears here ... the sibling-arm arrangement it replaced"
- :405 "the old y/n bindings and the default: No idle path were removed" [only Enter and Esc are bound in `Mode::Confirm`]
- :823 "the CLI dropped its `plugin` subcommand"
- :861 "the bound the split created: the same write used to need a person"
- :1006 "Both were renamed when the plugin became its own binary"
- :1134 "no `plugin` verb to inject any more"
- :1144 "Usage: strings USED to be a deliberate exception ... no longer are"
- :1161 "`2` since the workspace split, where `versions.cli` became ..."
- :1173 "This used to be maintained by keeping the plugin out of the CLI's update gate. It is now structural"
- :1249 "now that both tag shapes really exist"
- :1277 "no longer even the same program"

### docs/runbooks/forge-tag-and-release-protection.md (3)

- :13-14 "one rule beyond the three this runbook originally specified" [the ruleset also carries `required_signatures`]
- :161-164 "When this was first written every published tag was on the CLI line ... Since 2026-09-30" [both lines have published tags; the newest plugin tag is `ss-magic-plugin-v1.0.1`]
- :207-210 the same history repeated [shorten]

---

## Verification notes on raw omissions

All 24 raw omission findings were confirmed against the code. They were merged or adjusted as follows:

- **Merged as duplicates of confirmed findings:**
  - CONTRIBUTING `--check` eight lines (×2) into N1
  - CLAUDE "FIVE checks" (×2) into C11
  - README network into R1
  - README environment variables (×2) into R4
  - menu TTY into C13
- **Merged with a confirmed finding and widened:**
  - README stores (R3)
  - verb flags (R5)
- **New:** N2 (`dist generate`), N5 (mark-latest env), R6 (knob defaults and ranges), R7 (skills list), R11 (setup-github-ci details), C24 (data-dir fallback), C25 (hooks.json matchers and timeouts), C26 (lock and offsets files), N6 (bench variable), K5 (glossary terms)
- **Moved to code-suspect:** cli.rs:4-5 module doc (S9)
- **Corrections to raw claims:**
  - README network: raised to high (the coverage agent's rating), from medium.
  - Bench variable: confirmed that the test `panic!`s when the variable is unset.
  - CONTRIBUTING:306 does describe the R98 check in prose. Only the runnable command is missing.
