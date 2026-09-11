# Function names declared in more than one non-test file

Same name in two places is a lead for duplicated logic, or for a name that means two things.

## `as_str`

- `crates/ss-magic-plugin/src/checklist/schema.rs:112` pub `(&self) -> &str`
- `crates/ss-magic-plugin/src/checklist/schema.rs:196` pub `(&self) -> &'static str`
- `crates/ss-magic-plugin/src/checklist/schema.rs:228` pub `(&self) -> &'static str`
- `crates/ss-magic-plugin/src/hook/event.rs:345` private `(self) -> &'static str`
- `crates/ss-magic-plugin/src/main.rs:150` pub `(&self) -> &str`
- `crates/ss-magic-plugin/src/main.rs:247` pub `(&self) -> &'static str`

## `check`

- `crates/ss-magic-core/src/release.rs:595` pub `(current_version: &str) -> UpdateCheck`
- `crates/ss-magic-plugin/src/expect_artifact.rs:173` pub `(path: &Path) -> Option<Unmet>`

## `chmod_executable`

- `crates/ss-magic-core/src/superset_files.rs:366` private `(path: &Path) -> Result<()>`
- `crates/ss-magic-core/src/superset_files.rs:377` private `(_path: &Path) -> Result<()>`

## `classify`

- `crates/ss-magic/src/sync/reverse_sync.rs:242` pub `(main_root: &Path, worktree_root: &Path, rel: &Path) -> Result<DiffStatus>`
- `crates/ss-magic-plugin/src/setup_ci.rs:175` pub `(existing: Option<&str>, version: &str) -> State`

## `clear_legacy_skills_install`

- `crates/ss-magic/src/workspace/migrate.rs:136` private `() `
- `crates/ss-magic/src/workspace/migrate.rs:160` private `() `

## `code`

- `crates/ss-magic-plugin/src/expect_artifact.rs:159` pub `(self) -> &'static str`
- `crates/ss-magic-plugin/src/scratchpad.rs:201` pub `(&self) -> &'static str`

## `collect`

- `crates/ss-magic-plugin/src/spill_index.rs:173` pub `(projects_root: &Path, root: &Path) -> Index`
- `crates/ss-magic-plugin/src/status.rs:885` pub `(inputs: &Inputs, probes: &Probes) -> Status`

## `commit`

- `crates/ss-magic-core/src/git/mod.rs:157` pub `(repo_root: &Path, msg: &str) -> Result<()>`
- `crates/ss-magic-plugin/src/ledger.rs:885` private `(store: &Path, row: Row, base: Option<Row>, scan: &Scan) -> Result<Recorded>`

## `dir_for_root`

- `crates/ss-magic-plugin/src/bypass.rs:93` pub `(root: &Path) -> PathBuf`
- `crates/ss-magic-plugin/src/cache.rs:203` pub `(root: &Path) -> PathBuf`

## `dispatch`

- `crates/ss-magic/src/main.rs:166` private `(cmd: Command) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/main.rs:501` private `(inv: Invocation) -> Result<ExitCode>`

## `display_rel`

- `crates/ss-magic-plugin/src/checklist/verbs.rs:1374` private `(root: &Path, path: &Path) -> String`
- `crates/ss-magic-plugin/src/hook/session_start.rs:495` private `(root: &Path, path: &Path) -> String`

## `emit`

- `crates/ss-magic-plugin/src/spill_index.rs:386` private `(index: &Index, json: bool) -> Result<()>`
- `crates/ss-magic-plugin/src/status.rs:1941` private `(status: &Status, json: bool) -> Result<()>`

## `ensure_store`

- `crates/ss-magic-plugin/src/heartbeat.rs:221` private `(dir: &Path) -> Result<()>`
- `crates/ss-magic-plugin/src/ledger.rs:1080` private `(dir: &Path) -> Result<()>`

## `fail`

- `crates/ss-magic-plugin/src/checklist/verbs.rs:1456` private `(message: String) -> ExitCode`
- `crates/ss-magic-plugin/src/compact_window.rs:846` private `(message: String) -> ExitCode`

## `fetch_releases`

- `crates/ss-magic-core/src/release.rs:148` private `(&self, etag: Option<&str>) -> FetchOutcome;`
- `crates/ss-magic-core/src/release.rs:538` private `(&self, etag: Option<&str>) -> FetchOutcome`

## `fmt`

- `crates/ss-magic/src/tui/menu.rs:59` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic/src/tui/ui.rs:29` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic/src/tui/ui.rs:65` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic-plugin/src/checklist/schema.rs:128` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic-plugin/src/checklist/schema.rs:166` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic-plugin/src/checklist/validate.rs:35` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic-plugin/src/checklist/validate.rs:59` private `(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`
- `crates/ss-magic-plugin/src/hook/event.rs:268` private `(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`
- `crates/ss-magic-plugin/src/scratchpad.rs:213` private `(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`
- `crates/ss-magic-plugin/src/tmproot.rs:117` private `(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`

## `found`

- `crates/ss-magic-plugin/src/status.rs:173` private `(value: impl Into<String>, source: impl Into<String>) -> Self`
- `crates/ss-magic-plugin/src/status.rs:216` private `(path: PathBuf, source: impl Into<String>) -> Self`

## `from_process`

- `crates/ss-magic-plugin/src/compact_window.rs:362` pub `() -> Self`
- `crates/ss-magic-plugin/src/hook/session_start.rs:134` private `() -> Self`

## `from_token`

- `crates/ss-magic-plugin/src/main.rs:136` pub `(token: &str) -> Self`
- `crates/ss-magic-plugin/src/main.rs:217` pub `(token: &str) -> Option<Self>`

## `handle`

- `crates/ss-magic-plugin/src/hook/file_changed.rs:128` pub(crate) `(ctx: &HookContext<'_>) -> Result<Outcome>`
- `crates/ss-magic-plugin/src/hook/pre_compact.rs:70` pub(crate) `(ctx: &HookContext<'_>) -> Result<Outcome>`
- `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:642` pub(crate) `(ctx: &HookContext<'_>) -> Result<Outcome>`
- `crates/ss-magic-plugin/src/hook/session_end.rs:50` pub(crate) `(ctx: &HookContext<'_>) -> Result<Outcome>`
- `crates/ss-magic-plugin/src/hook/session_start.rs:147` pub(crate) `(ctx: &HookContext<'_>) -> Result<Outcome>`
- `crates/ss-magic-plugin/src/hook/subagent_stop.rs:89` pub(crate) `(ctx: &HookContext<'_>) -> Result<Outcome>`

## `handle_with`

- `crates/ss-magic-plugin/src/hook/file_changed.rs:134` private `(ctx: &HookContext<'_>, program: &str, raw_target: &str) -> Result<Outcome>`
- `crates/ss-magic-plugin/src/hook/session_start.rs:152` pub(crate) `(ctx: &HookContext<'_>, surroundings: &Surroundings) -> Result<Outcome>`

## `is_empty`

- `crates/ss-magic-plugin/src/checklist/schema.rs:117` pub `(&self) -> bool`
- `crates/ss-magic-plugin/src/hook/event.rs:376` private `(&self) -> bool`

## `is_fresh`

- `crates/ss-magic-core/src/release.rs:192` pub `(&self, now: u64) -> bool`
- `crates/ss-magic-core/src/release.rs:228` private `(checked_at: u64, now: u64) -> bool`

## `is_tracked`

- `crates/ss-magic-plugin/src/hook/pre_compact.rs:140` private `(report: &Report, rel: &str) -> bool`
- `crates/ss-magic-plugin/src/scratchpad.rs:363` private `(&self, path: &Path) -> bool`

## `label`

- `crates/ss-magic-core/src/sync/apply.rs:65` pub `(&self) -> &str`
- `crates/ss-magic-core/src/sync/pattern.rs:31` pub `(&self) -> String`
- `crates/ss-magic-plugin/src/ledger.rs:228` private `(self) -> &'static str`

## `main`

- `crates/ss-magic/src/main.rs:470` private `() -> ExitCode`
- `crates/ss-magic-plugin/src/main.rs:544` private `() -> ExitCode`

## `missing`

- `crates/ss-magic-plugin/src/status.rs:182` private `(note: impl Into<String>) -> Self`
- `crates/ss-magic-plugin/src/status.rs:224` private `(note: impl Into<String>) -> Self`

## `new`

- `crates/ss-magic/src/tui/cockpit.rs:300` private `(worktree_root: &Path, main_root: &Path, offered: &[(PathBuf, DiffStatus) `
- `crates/ss-magic-core/src/release.rs:520` pub `(user_agent_version: &str) -> Self`
- `crates/ss-magic-plugin/src/checklist/schema.rs:100` pub `(raw: impl Into<String>) -> Self`
- `crates/ss-magic-plugin/src/checklist/schema.rs:450` pub `(title: impl Into<String>, slug: impl Into<String>, now: Timestamp) -> Self`
- `crates/ss-magic-plugin/src/heartbeat.rs:144` pub `(event: &str, now: u64, outcome: Outcome) -> Self`
- `crates/ss-magic-plugin/src/hook/mod.rs:268` pub `(response: Response) -> Self`

## `open_lock_file`

- `crates/ss-magic/src/update/apply.rs:234` private `(path: &Path) -> std::io::Result<File>`
- `crates/ss-magic-plugin/src/tmproot.rs:221` private `(path: &Path) -> io::Result<File>`

## `parse`

- `crates/ss-magic/src/cli.rs:97` pub `(args: &[String]) -> Parsed`
- `crates/ss-magic-plugin/src/cache.rs:248` private `(path: &Path, key: &str, text: &str) -> Self`
- `crates/ss-magic-plugin/src/checklist/verbs.rs:346` private `(args: &[String]) -> ParsedSub`
- `crates/ss-magic-plugin/src/main.rs:409` pub `(args: &[String]) -> Parsed`

## `parse_args`

- `crates/ss-magic-plugin/src/compact_window.rs:193` private `(args: &[String]) -> ParsedArgs`
- `crates/ss-magic-plugin/src/release_check.rs:652` private `(args: &[String]) -> ParsedArgs`

## `plural`

- `crates/ss-magic-plugin/src/ledger.rs:1525` private `(n: usize) -> &'static str`
- `crates/ss-magic-plugin/src/spill_index.rs:459` private `(n: usize) -> &'static str`

## `read`

- `crates/ss-magic-plugin/src/heartbeat.rs:363` pub `(store: &Path) -> Result<Vec<Row>>`
- `crates/ss-magic-plugin/src/ledger.rs:376` pub `(store: &Path) -> Result<Vec<Row>>`

## `record`

- `crates/ss-magic-plugin/src/bypass.rs:116` pub `(dir: &Path, realpath: &Path, now: u64) -> Result<PathBuf>`
- `crates/ss-magic-plugin/src/ledger.rs:851` pub `(store: &Path, ingest: &Ingest<'_>) -> Result<Recorded>`

## `render`

- `crates/ss-magic-plugin/src/cache.rs:481` pub `(entry: &Entry, budget: Budget) -> String`
- `crates/ss-magic-plugin/src/checklist/render.rs:81` pub `(doc: &Document, path: &Path, repo_url: Option<&str>, budget: Budget) -> String`
- `crates/ss-magic-plugin/src/setup_ci.rs:111` pub `(version: &str) -> String`
- `crates/ss-magic-plugin/src/status.rs:191` private `(&self) -> String`

## `render_text`

- `crates/ss-magic-plugin/src/compact_window.rs:754` pub(crate) `(report: &Report) -> String`
- `crates/ss-magic-plugin/src/release_check.rs:575` private `(out: &mut String, report: &Report) `
- `crates/ss-magic-plugin/src/status.rs:1990` private `(out: &mut String, status: &Status) `

## `resolve`

- `crates/ss-magic-core/src/git/mod.rs:48` private `(path: &str, base: &Path) -> Result<PathBuf>`
- `crates/ss-magic-plugin/src/config.rs:175` pub `(cwd_root: &Path) -> PluginConfig`
- `crates/ss-magic-plugin/src/identity.rs:55` pub `(cwd: &Path) -> Option<Identity>`

## `resolve_target`

- `crates/ss-magic-plugin/src/hook/file_changed.rs:253` private `(raw: &str) -> Result<PathBuf, String>`
- `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:272` private `(cwd: &Path, raw: &Path) -> Target`

## `run`

- `crates/ss-magic/src/main.rs:65` private `() -> Result<ExitCode>`
- `crates/ss-magic/src/sync/reverse_sync.rs:299` pub `(worktree_root: &Path, main_root: &Path) -> Result<ExitCode>`
- `crates/ss-magic/src/tui/menu.rs:115` pub `(cwd: &Path) -> Result<ExitCode>`
- `crates/ss-magic-core/src/sync/apply.rs:96` pub `(src: &Path, dest: &Path, patterns: &[String], mut on_event: F) -> Result<Summary>`
- `crates/ss-magic-plugin/src/bypass.rs:176` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/checklist/verbs.rs:427` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/compact_window.rs:156` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/expect_artifact.rs:412` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/hook/mod.rs:358` pub `(event: &HookEvent) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/ledger.rs:1210` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/main.rs:446` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/release_check.rs:675` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/scratchpad.rs:746` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/setup_ci.rs:198` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/spill_index.rs:336` pub `(args: &[String]) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/status.rs:1877` pub `(args: &[String]) -> Result<ExitCode>`

## `run_core`

- `crates/ss-magic-plugin/src/bypass.rs:219` private `(cwd: &Path, target: &str, now: u64) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/checklist/verbs.rs:470` private `(cwd: &Path, sub: &Sub, now: u64) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/compact_window.rs:222` private `(root: &Path, window: u64) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/expect_artifact.rs:457` private `(cwd: &Path, target: &str, note: Option<&str>, now: u64) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/setup_ci.rs:233` pub `(cwd: &Path, version: &str, check: bool, force: bool) -> Result<ExitCode>`

## `run_init`

- `crates/ss-magic/src/workspace/migrate.rs:463` pub `(repo_root: &Path, existing: Option<&Config>) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/checklist/verbs.rs:503` private `(root: &Path, cwd: &Path, slug: &str, now: u64) -> Result<ExitCode>`

## `slugify`

- `crates/ss-magic-plugin/src/checklist/render.rs:455` private `(text: &str) -> String`
- `crates/ss-magic-plugin/src/identity.rs:138` private `(input: &str) -> String`

## `spawn_and_wait`

- `crates/ss-magic/src/update/apply.rs:143` private `(&self, exe: &Path, args: &[OsString]) -> std::io::Result<Option<i32>>;`
- `crates/ss-magic/src/update/apply.rs:151` private `(&self, exe: &Path, args: &[OsString]) -> std::io::Result<Option<i32>>`

## `usage`

- `crates/ss-magic/src/cli.rs:85` pub `() -> &'static str`
- `crates/ss-magic-plugin/src/main.rs:379` pub `() -> &'static str`

## `usage_error`

- `crates/ss-magic-plugin/src/cache.rs:1006` private `(message: &str, usage: &str) -> Result<ExitCode>`
- `crates/ss-magic-plugin/src/checklist/verbs.rs:1438` private `(message: &str) -> ExitCode`
- `crates/ss-magic-plugin/src/compact_window.rs:837` private `(message: &str) -> ExitCode`
- `crates/ss-magic-plugin/src/config.rs:938` private `(usage: &str, message: &str) -> ExitCode`
- `crates/ss-magic-plugin/src/ledger.rs:1250` private `(message: &str) -> Result<ExitCode>`

## `version_line`

- `crates/ss-magic/src/main.rs:123` pub `() -> String`
- `crates/ss-magic-plugin/src/main.rs:394` pub `() -> String`

## `with_detail`

- `crates/ss-magic-plugin/src/heartbeat.rs:176` pub `(mut self, detail: impl Into<String>) -> Self`
- `crates/ss-magic-plugin/src/hook/mod.rs:276` pub `(mut self, detail: impl Into<String>) -> Self`

## `with_lock`

- `crates/ss-magic-plugin/src/checklist/verbs.rs:1329` private `(state_root: &Path, f: impl FnOnce() -> T) -> Result<T>`
- `crates/ss-magic-plugin/src/tmproot.rs:242` pub `(root: &Path, name: &str, f: impl FnOnce() -> T) -> io::Result<T>`

## `write_cache`

- `crates/ss-magic-core/src/release.rs:218` private `(path: &Path, cache: &Cache) `
- `crates/ss-magic-plugin/src/release_check.rs:106` pub `(path: &Path, cache: &Cache) -> Result<()>`

