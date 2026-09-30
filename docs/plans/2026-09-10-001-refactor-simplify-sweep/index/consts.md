# Constants and statics (all crates)

One row per declaration. `dup-name` marks a name declared in more than one file.

| name | file:line | vis | type | value | flags |
|---|---|---|---|---|---|
| `ACTIONS_REL` | `crates/ss-magic-plugin/src/checklist/verbs.rs:94` | pub | `&str` | `"docs/actions"` |  |
| `AE2_TAGS` | `crates/ss-magic-core/src/release/tests.rs:94` | private | `&[&str]` | `&[` | test |
| `ALLOWED` | `crates/ss-magic-plugin/src/hook/file_changed.rs:114` | private | `i64` | `0` |  |
| `ALLOWED_FROM` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:299` | private | `&str` | `"// ── Only reached after a match: checklist + git status ──"` | test |
| `ANSI_RESET` | `crates/ss-magic-core/src/style.rs:49` | private | `&str` | `"\x1b[0m"` |  |
| `BACKUPS_REL` | `crates/ss-magic/src/sync/reverse_sync.rs:661` | private | `&str` | `".superset/backups"` |  |
| `BACKUP_BATCHES_KEPT` | `crates/ss-magic/src/sync/reverse_sync.rs:730` | private | `usize` | `10` |  |
| `BINARY_REL` | `crates/ss-magic-plugin/src/status.rs:129` | private | `&str` | `"bin/ss-magic-plugin"` |  |
| `BIN_NAME` | `crates/ss-magic/src/update/apply.rs:77` | private | `&str` | `"ss-magic"` |  |
| `BYPASS_USAGE` | `crates/ss-magic-plugin/src/bypass.rs:71` | private | `&str` | `"\` |  |
| `BYTES_PER_LINE` | `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:79` | private | `u64` | `40` |  |
| `CACHE_READ_MULT` | `crates/ss-magic-plugin/src/ledger.rs:132` | private | `f64` | `0.10` |  |
| `CACHE_WRITE_1H_MULT` | `crates/ss-magic-plugin/src/ledger.rs:137` | private | `f64` | `2.00` |  |
| `CACHE_WRITE_5M_MULT` | `crates/ss-magic-plugin/src/ledger.rs:134` | private | `f64` | `1.25` |  |
| `CHECKLIST_POINTER_NAME` | `crates/ss-magic-plugin/src/hook/session_start.rs:62` | private | `&str` | `"checklist.json"` |  |
| `CHECKLIST_SUFFIX` | `crates/ss-magic-plugin/src/checklist/verbs.rs:97` | pub | `&str` | `".checklist.json"` |  |
| `CHECKLIST_VERBS` | `crates/ss-magic-plugin/src/hook/session_start.rs:49` | private | `&str` | `"\` |  |
| `CHILD_RAN_MARKER` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:1640` | private | `&str` | `"child_relative_checklist_target: past the docs/ guard"` | test |
| `CHROME_LINES` | `crates/ss-magic/src/tui/cockpit.rs:1432` | private | `u16` | `7` |  |
| `CLAIM_DIRS` | `crates/ss-magic-plugin/src/scratchpad.rs:113` | private | `[&str; 3]` | `["conclusions", "bypass", "expect-artifact"]` |  |
| `CLAIM_EXT` | `crates/ss-magic-plugin/src/bypass.rs:59` | private | `&str` | `"json"` |  |
| `CLI_LINE` | `crates/ss-magic-core/src/release.rs:81` | pub | `Line` | `Line {` |  |
| `COLOR_ENABLED` | `crates/ss-magic-core/src/style.rs:51` | private | `OnceLock<bool>` | `OnceLock::new()` |  |
| `COMPACT_ADVICE_MARKER` | `crates/ss-magic-plugin/src/hook/session_start.rs:94` | private | `&str` | `"compact-advice-shown"` |  |
| `COMPACT_ADVICE_TEXT` | `crates/ss-magic-plugin/src/hook/session_start.rs:217` | private | `&str` | `"\` |  |
| `CONCLUDE_USAGE` | `crates/ss-magic-plugin/src/cache.rs:733` | private | `&str` | `"\` |  |
| `CONCLUSIONS_USAGE` | `crates/ss-magic-plugin/src/cache.rs:746` | private | `&str` | `"\` |  |
| `CONFIG_JSON` | `crates/ss-magic-core/src/superset_files.rs:27` | private | `&str` | `"config.json"` |  |
| `CONFIG_LOCK_NAME` | `crates/ss-magic-plugin/src/config.rs:51` | private | `&str` | `"magic-json.lock"` |  |
| `CONFIG_USAGE` | `crates/ss-magic-plugin/src/config.rs:348` | pub(crate) | `&str` | `"\` |  |
| `CONTEXT` | `crates/ss-magic/src/tui/cockpit.rs:54` | private | `usize` | `3` |  |
| `COST_STATE_NEEDLE` | `crates/ss-magic-plugin/src/ledger.rs:546` | private | `&[u8]` | `b"\"cost-state\""` |  |
| `COST_USAGE` | `crates/ss-magic-plugin/src/ledger.rs:1190` | private | `&str` | `"\` |  |
| `DATA_ROOT_FILE` | `crates/ss-magic-plugin/src/status.rs:144` | private | `&str` | `"data-root"` |  |
| `DAY` | `crates/ss-magic-plugin/src/release_check/tests.rs:18` | private | `u64` | `24 * 60 * 60` | test |
| `DECLARED_EVENTS` | `crates/ss-magic-plugin/src/status.rs:82` | pub | `[&str; 5]` | `[` |  |
| `DECLINING_ENV` | `crates/ss-magic-core/src/git/discover.rs:119` | pub | `[(&str, &str); 6]` | `[` |  |
| `DEFAULT_EXCLUDES` | `crates/ss-magic-core/src/sync/apply.rs:28` | private | `[&str; 2]` | `["node_modules", ".venv"]` |  |
| `DEFAULT_LIMIT_BYTES` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:28` | private | `u64` | `120_000` | test |
| `DIFF_LINE_BUDGET` | `crates/ss-magic-plugin/src/setup_ci.rs:89` | private | `usize` | `120` |  |
| `DIRENV` | `crates/ss-magic-plugin/src/hook/file_changed.rs:85` | private | `&str` | `"direnv"` |  |
| `DIR_MODE` | `crates/ss-magic-plugin/src/bypass.rs:67` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/cache.rs:106` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/expect_artifact.rs:92` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/heartbeat.rs:71` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/hook/subagent_stop.rs:85` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/ledger.rs:100` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/scratchpad.rs:119` | private | `u32` | `0o700` | dup-name |
| `DIR_MODE` | `crates/ss-magic-plugin/src/tmproot.rs:101` | private | `u32` | `0o700` | dup-name |
| `DIR_NAME` | `crates/ss-magic-plugin/src/bypass.rs:56` | pub | `&str` | `"bypass"` | dup-name |
| `DIR_NAME` | `crates/ss-magic-plugin/src/cache.rs:84` | pub | `&str` | `"conclusions"` | dup-name |
| `DIR_NAME` | `crates/ss-magic-plugin/src/expect_artifact.rs:72` | pub | `&str` | `"expect-artifact"` | dup-name |
| `DISABLE_USAGE` | `crates/ss-magic-plugin/src/config.rs:336` | pub(crate) | `&str` | `"\` |  |
| `DOCUMENT_ID` | `crates/ss-magic-plugin/src/checklist/verbs.rs:866` | private | `&str` | `"document"` |  |
| `EACH` | `crates/ss-magic-plugin/src/heartbeat/tests.rs:368` | private | `usize` | `12` | test |
| `ENABLE_USAGE` | `crates/ss-magic-plugin/src/config.rs:320` | pub(crate) | `&str` | `"\` |  |
| `ENTRIES_KEPT` | `crates/ss-magic-plugin/src/cache.rs:94` | pub | `usize` | `200` |  |
| `ENTRYPOINT_ENV` | `crates/ss-magic-plugin/src/hook/mod.rs:128` | pub(crate) | `&str` | `"CLAUDE_CODE_ENTRYPOINT"` |  |
| `ENTRY_EXT` | `crates/ss-magic-plugin/src/cache.rs:88` | private | `&str` | `"md"` |  |
| `ENVELOPE_CLOSE` | `crates/ss-magic-plugin/src/cache.rs:445` | private | `&str` | `"END-UNTRUSTED-DATA"` |  |
| `ENVELOPE_OPEN` | `crates/ss-magic-plugin/src/cache.rs:444` | private | `&str` | `"BEGIN-UNTRUSTED-DATA"` |  |
| `ENV_FILE_VAR` | `crates/ss-magic-plugin/src/hook/file_changed.rs:89` | private | `&str` | `"CLAUDE_ENV_FILE"` |  |
| `ENV_LOCK` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:2275` | private | `Mutex<()>` | `Mutex::new(())` | dup-name test |
| `ENV_LOCK` | `crates/ss-magic/src/update/apply/tests.rs:7` | private | `Mutex<()>` | `Mutex::new(())` | dup-name test |
| `EOL_ONLY_NOTE` | `crates/ss-magic/src/tui/cockpit.rs:73` | private | `&str` | `"no content differences — the sides differ only by line endings \` |  |
| `EVENT` | `crates/ss-magic-plugin/src/hook/file_changed/tests.rs:39` | private | `HookEvent` | `HookEvent::FileChanged` | dup-name test |
| `EVENT` | `crates/ss-magic-plugin/src/hook/pre_compact/tests.rs:90` | private | `HookEvent` | `HookEvent::PreCompact` | dup-name test |
| `EVENT` | `crates/ss-magic-plugin/src/hook/session_end/tests.rs:27` | private | `HookEvent` | `HookEvent::SessionEnd` | dup-name test |
| `EVENT` | `crates/ss-magic-plugin/src/hook/subagent_stop/tests.rs:26` | private | `HookEvent` | `HookEvent::SubagentStop` | dup-name test |
| `EVENTS` | `crates/ss-magic-plugin/src/tests.rs:14` | private | `&[(&str, HookEvent)]` | `&[` | test |
| `EXCLUDED_TREES` | `crates/ss-magic-core/src/sync/mod.rs:40` | pub | `[&[&str]; 4]` | `[` |  |
| `FILE_MODE` | `crates/ss-magic-plugin/src/bypass.rs:68` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/cache.rs:107` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/expect_artifact.rs:93` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/heartbeat.rs:73` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/hook/file_changed.rs:95` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/hook/pre_compact.rs:67` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/hook/subagent_stop.rs:86` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/ledger.rs:101` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/scratchpad.rs:122` | private | `u32` | `0o600` | dup-name |
| `FILE_MODE` | `crates/ss-magic-plugin/src/setup_ci.rs:84` | private | `u32` | `0o644` | dup-name |
| `FNV_OFFSET_BASIS` | `crates/ss-magic-core/src/hashing.rs:36` | private | `u64` | `0xcbf29ce484222325` |  |
| `FNV_PRIME` | `crates/ss-magic-core/src/hashing.rs:38` | private | `u64` | `0x100000001b3` |  |
| `FOOTER_LEGEND` | `crates/ss-magic/src/tui/cockpit.rs:1337` | private | `&str` | `"↑↓/jk move · PgUp/PgDn/Space/b scroll · p push · l pull · m merge · d delete · u undecided · Enter apply · ? help · Esc` |  |
| `FRAMING` | `crates/ss-magic-plugin/src/cache.rs:451` | private | `&str` | `"\` |  |
| `FRESH_FOR` | `crates/ss-magic-core/src/release.rs:53` | private | `Duration` | `Duration::from_secs(24 * 60 * 60)` |  |
| `GATE_INLINE_BYTE_BUDGET_DEFAULT` | `crates/ss-magic-plugin/src/config.rs:92` | pub | `u32` | `10_000` |  |
| `GATE_INLINE_BYTE_BUDGET_MAX` | `crates/ss-magic-plugin/src/config.rs:96` | pub | `u32` | `100_000` |  |
| `GATE_INLINE_BYTE_BUDGET_MIN` | `crates/ss-magic-plugin/src/config.rs:94` | pub | `u32` | `1_000` |  |
| `GATE_THRESHOLD_LINES_DEFAULT` | `crates/ss-magic-plugin/src/config.rs:82` | pub | `u32` | `3_000` |  |
| `GATE_THRESHOLD_LINES_MAX` | `crates/ss-magic-plugin/src/config.rs:86` | pub | `u32` | `20_000` |  |
| `GATE_THRESHOLD_LINES_MIN` | `crates/ss-magic-plugin/src/config.rs:84` | pub | `u32` | `500` |  |
| `GC_USAGE` | `crates/ss-magic-plugin/src/cache.rs:753` | private | `&str` | `"\` |  |
| `GITFILE_MAX_BYTES` | `crates/ss-magic-core/src/git/discover.rs:107` | pub | `usize` | `4096` |  |
| `HARNESS_TIMEOUT` | `crates/ss-magic-plugin/src/status.rs:149` | private | `Duration` | `Duration::from_secs(5)` |  |
| `HEADER` | `crates/ss-magic-plugin/src/hook/pre_compact.rs:55` | private | `&str` | `"\` |  |
| `HEADER_TITLE_PREFIX` | `crates/ss-magic-plugin/src/cache.rs:118` | private | `&str` | `"# ss-magic conclusion"` |  |
| `HEAD_READ_BYTES` | `crates/ss-magic-core/src/git/discover.rs:112` | private | `usize` | `255` |  |
| `HEAD_SCAN_BYTES` | `crates/ss-magic-plugin/src/ledger.rs:551` | private | `usize` | `256` |  |
| `HIGHLIGHT_SYMBOL` | `crates/ss-magic/src/tui/cockpit.rs:749` | private | `&str` | `"› "` |  |
| `HIGHLIGHT_SYMBOL_WIDTH` | `crates/ss-magic/src/tui/cockpit.rs:754` | private | `u16` | `2` |  |
| `HIGH_CONFIDENCE_ROWS` | `crates/ss-magic-plugin/src/compact_window.rs:108` | private | `usize` | `3` |  |
| `HTTP_TIMEOUT` | `crates/ss-magic-core/src/release.rs:57` | private | `Duration` | `Duration::from_secs(5)` |  |
| `H_SCROLL_STEP` | `crates/ss-magic/src/tui/cockpit.rs:57` | private | `u16` | `8` |  |
| `IDENTIFIER_HEX_LEN` | `crates/ss-magic-plugin/src/tmproot.rs:105` | private | `usize` | `16` |  |
| `INIT_COMMIT_MESSAGE` | `crates/ss-magic/src/workspace/migrate.rs:668` | private | `&str` | `"chore(superset): initialize ss-magic layout"` |  |
| `INSTALL_LOCK_NAME` | `crates/ss-magic-plugin/src/tmproot.rs:97` | pub | `&str` | `"install.lock"` |  |
| `INVOKED_BY_A_HOOK` | `crates/ss-magic-plugin/src/tests.rs:332` | private | `&[HumanVerb]` | `&[HumanVerb::SeedConfig, HumanVerb::ReleaseCheck]` | test |
| `KEY_HEX_LEN` | `crates/ss-magic-plugin/src/cache.rs:103` | private | `usize` | `16` |  |
| `KIB` | `crates/ss-magic-plugin/src/spill_index.rs:470` | private | `u64` | `1024` |  |
| `KNOWN_EXTENSIONS` | `crates/ss-magic-core/src/git/discover.rs:135` | private | `[&str; 8]` | `[` |  |
| `LABEL_WIDTH` | `crates/ss-magic-plugin/src/status.rs:1953` | private | `usize` | `22` |  |
| `LEDGER_FILE_NAME` | `crates/ss-magic-plugin/src/ledger.rs:82` | pub | `&str` | `"cost.jsonl"` |  |
| `LEGACY_PACK_FILE_NAME` | `crates/ss-magic/src/pack/tests.rs:13` | private | `&str` | `"ss-magic-files.tar.bz2"` | test |
| `LEGACY_SKILLS_REL` | `crates/ss-magic/src/workspace/migrate.rs:103` | private | `&str` | `".claude/skills/ss-magic"` |  |
| `LIST_BORDER_WIDTH` | `crates/ss-magic/src/tui/cockpit.rs:758` | private | `u16` | `2` |  |
| `LIST_BYTE_BUDGET` | `crates/ss-magic-plugin/src/checklist/verbs.rs:121` | private | `usize` | `24_000` |  |
| `LOCK_FILE_NAME` | `crates/ss-magic-plugin/src/heartbeat.rs:67` | private | `&str` | `"hooks.lock"` | dup-name |
| `LOCK_FILE_NAME` | `crates/ss-magic-plugin/src/ledger.rs:96` | private | `&str` | `"cost.lock"` | dup-name |
| `LOCK_FILE_NAME` | `crates/ss-magic/src/update/apply.rs:80` | pub | `&str` | `"update.lock"` | dup-name |
| `LOCK_NAME` | `crates/ss-magic-plugin/src/checklist/verbs.rs:105` | private | `&str` | `"checklist.lock"` | dup-name |
| `LOCK_NAME` | `crates/ss-magic-plugin/src/hook/file_changed.rs:106` | private | `&str` | `"file-changed.lock"` | dup-name |
| `LOCK_NAME` | `crates/ss-magic-plugin/src/release_check.rs:53` | pub | `&str` | `"release-check.lock"` | dup-name |
| `LOG_FILE_NAME` | `crates/ss-magic-plugin/src/heartbeat.rs:61` | pub | `&str` | `"hooks.jsonl"` |  |
| `MAGIC_JSON` | `crates/ss-magic-core/src/superset_files.rs:30` | private | `&str` | `"magic.json"` |  |
| `MAGIC_LOCAL_JSON` | `crates/ss-magic-core/src/superset_files.rs:31` | private | `&str` | `"magic.local.json"` |  |
| `MAGIC_LOCAL_PATTERN` | `crates/ss-magic-core/src/superset_files.rs:35` | private | `&str` | `".superset/magic.local.json"` |  |
| `MAGIC_LOCAL_REL` | `crates/ss-magic/src/workspace/migrate.rs:65` | private | `&str` | `".superset/magic.local.json"` |  |
| `MAGIC_SH` | `crates/ss-magic-core/src/superset_files.rs:24` | pub | `&str` | `include_str!("../../../assets/magic.sh")` |  |
| `MAGIC_SH_NAME` | `crates/ss-magic-core/src/superset_files.rs:28` | private | `&str` | `"magic.sh"` |  |
| `MAGIC_WRAPPER_ENTRY` | `crates/ss-magic/src/workspace/migrate.rs:57` | pub | `&str` | `"./.superset/magic.sh sync"` |  |
| `MANIFEST_NAME` | `crates/ss-magic-plugin/src/status.rs:94` | private | `&str` | `"ss-magic"` |  |
| `MARKER_DISCLOSED` | `crates/ss-magic-plugin/src/status.rs:135` | private | `&str` | `".ss-magic-disclosed"` |  |
| `MARKER_INSTALLED` | `crates/ss-magic-plugin/src/status.rs:133` | private | `&str` | `".ss-magic-installed"` |  |
| `MARKER_UNSUPPORTED` | `crates/ss-magic-plugin/src/status.rs:138` | private | `&str` | `".ss-magic-unsupported"` |  |
| `MAX_AGE_SECS` | `crates/ss-magic-plugin/src/bypass.rs:64` | pub | `u64` | `24 * 60 * 60` | dup-name |
| `MAX_AGE_SECS` | `crates/ss-magic-plugin/src/cache.rs:100` | pub | `u64` | `30 * 24 * 60 * 60` | dup-name |
| `MAX_AGE_SECS` | `crates/ss-magic-plugin/src/expect_artifact.rs:84` | pub | `u64` | `6 * 60 * 60` | dup-name |
| `MAX_AGE_SECS` | `crates/ss-magic-plugin/src/heartbeat.rs:84` | pub | `u64` | `30 * 24 * 60 * 60` | dup-name |
| `MAX_DIFF_BYTES` | `crates/ss-magic/src/tui/diffmodel.rs:24` | pub | `u64` | `2 * 1024 * 1024; // 2 MiB` |  |
| `MAX_LISTED_TRACKED_PATHS` | `crates/ss-magic-plugin/src/hook/session_start.rs:467` | private | `usize` | `20` |  |
| `MAX_NAME_ATTEMPTS` | `crates/ss-magic-plugin/src/hook/subagent_stop.rs:80` | private | `u32` | `20` |  |
| `MAX_NOTE_LEN` | `crates/ss-magic-plugin/src/expect_artifact.rs:89` | private | `usize` | `500` |  |
| `MAX_TRANSCRIPT_BYTES` | `crates/ss-magic-plugin/src/hook/subagent_stop.rs:75` | private | `u64` | `16 * 1024 * 1024` |  |
| `MAX_WALK_DEPTH` | `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:95` | private | `usize` | `64` |  |
| `MEMO` | `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:1030` | private | `OnceLock<Mutex<HashMap<PathBuf, Option<PathBuf>>>>` | `OnceLock::new()` |  |
| `MIB` | `crates/ss-magic-plugin/src/spill_index.rs:471` | private | `u64` | `1024 * KIB` |  |
| `MIGRATE_COMMIT_MESSAGE` | `crates/ss-magic/src/workspace/migrate.rs:667` | private | `&str` | `"chore(superset): migrate to ss-magic layout"` |  |
| `MIN_SPLIT_COL` | `crates/ss-magic/src/tui/diffmodel.rs:174` | private | `u16` | `40` |  |
| `NAMESPACE_DIR` | `crates/ss-magic-plugin/src/tmproot.rs:89` | pub | `&str` | `"ss-magic-plugin"` |  |
| `NEWER` | `crates/ss-magic-plugin/src/hook/session_start/tests.rs:735` | private | `&str` | `"ss-magic-plugin-v1.1.0"` | test |
| `NEW_FILE_MODE` | `crates/ss-magic-plugin/src/checklist/verbs.rs:111` | private | `u32` | `0o644` | dup-name |
| `NEW_FILE_MODE` | `crates/ss-magic-plugin/src/compact_window.rs:132` | private | `u32` | `0o644` | dup-name |
| `NEW_GUTTER` | `crates/ss-magic/src/tui/cockpit.rs:69` | private | `u16` | `7` |  |
| `NONCE_HEX_LEN` | `crates/ss-magic-plugin/src/cache.rs:448` | private | `usize` | `16` |  |
| `NON_TEXT_EXTENSIONS` | `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:88` | private | `[&str; 11]` | `[` |  |
| `NOTE_NAME` | `crates/ss-magic-plugin/src/hook/pre_compact.rs:52` | private | `&str` | `"PRE-COMPACT.md"` |  |
| `NOT_A_SHIPPING_ACTION` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:2600` | private | `&str` | `` | test |
| `NOT_VERBS` | `crates/ss-magic-plugin/src/tests.rs:383` | private | `&[&str]` | `&[` | test |
| `NOW` | `crates/ss-magic-plugin/src/bypass/tests.rs:19` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/checklist/verbs/tests.rs:21` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/expect_artifact/tests.rs:17` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/heartbeat/tests.rs:18` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/file_changed/tests.rs:37` | private | `u64` | `1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/pre_compact/tests.rs:22` | private | `u64` | `1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:25` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/session_end/tests.rs:26` | private | `u64` | `1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/session_start/tests.rs:22` | private | `u64` | `1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/subagent_stop/tests.rs:24` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/hook/tests.rs:27` | private | `u64` | `1_788_091_200` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/ledger/tests.rs:20` | private | `u64` | `1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/release_check/tests.rs:17` | private | `u64` | `1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.` | dup-name test |
| `NOW` | `crates/ss-magic-plugin/src/status/tests.rs:31` | private | `u64` | `1_788_091_200` | dup-name test |
| `NO_UPDATE_ENV` | `crates/ss-magic/src/update/apply.rs:87` | pub | `&str` | `"SS_MAGIC_NO_UPDATE"` |  |
| `OFFSETS_FILE_NAME` | `crates/ss-magic-plugin/src/ledger.rs:86` | pub | `&str` | `"transcript-offsets.json"` |  |
| `OPTIONS` | `crates/ss-magic-core/src/superset_files/tests.rs:63` | private | `[&str; 4]` | `[".env", "**/.env", ".env.local", "**/.dev.vars"]` | dup-name test |
| `OPTIONS` | `crates/ss-magic-core/src/sync/repo_scan.rs:14` | pub | `[&str; 4]` | `[".env", "**/.env", ".env.local", "**/.dev.vars"]` | dup-name |
| `OVERRIDE_ENV` | `crates/ss-magic-plugin/src/compact_window.rs:100` | pub | `&str` | `"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE"` |  |
| `PIN_FILE` | `crates/ss-magic-plugin/src/status.rs:106` | pub(crate) | `&str` | `"ss-magic-plugin.version"` |  |
| `PIN_KEY` | `crates/ss-magic-plugin/src/setup_ci.rs:80` | private | `&str` | `"SS_MAGIC_PLUGIN_VERSION:"` |  |
| `PLUGIN_KEY` | `crates/ss-magic-plugin/src/config.rs:318` | private | `&str` | `"plugin"` |  |
| `PLUGIN_LINE` | `crates/ss-magic-core/src/release.rs:91` | pub | `Line` | `Line {` |  |
| `POINTER_LOCK_NAME` | `crates/ss-magic-plugin/src/scratchpad.rs:104` | private | `&str` | `"current.lock"` |  |
| `POINTER_NAME` | `crates/ss-magic-plugin/src/checklist/verbs.rs:101` | pub | `&str` | `"checklist.json"` | dup-name |
| `POINTER_NAME` | `crates/ss-magic-plugin/src/scratchpad.rs:96` | private | `&str` | `"current.json"` | dup-name |
| `POLL_INTERVAL` | `crates/ss-magic-plugin/src/status.rs:154` | private | `Duration` | `Duration::from_millis(20)` |  |
| `PRICES` | `crates/ss-magic-plugin/src/ledger.rs:119` | private | `&[(&str, f64, f64)]` | `&[` |  |
| `PRICES_DIR_NAME` | `crates/ss-magic-plugin/src/ledger.rs:90` | pub | `&str` | `"prices"` |  |
| `PRICE_TABLE_VERSION` | `crates/ss-magic-plugin/src/ledger.rs:109` | pub | `&str` | `"2026-06-24"` |  |
| `PRUNE_TRIGGER_BYTES` | `crates/ss-magic-plugin/src/heartbeat.rs:91` | pub | `u64` | `256 * 1024` |  |
| `RACERS` | `crates/ss-magic-plugin/src/checklist/verbs/tests.rs:901` | private | `usize` | `8` | dup-name test |
| `RACERS` | `crates/ss-magic-plugin/src/hook/pre_compact/tests.rs:312` | private | `usize` | `10` | dup-name test |
| `RACERS` | `crates/ss-magic-plugin/src/ledger/tests.rs:494` | private | `usize` | `8` | dup-name test |
| `RC_NAMES` | `crates/ss-magic-plugin/src/hook/file_changed.rs:120` | private | `[&str; 2]` | `[".envrc", ".env"]` |  |
| `READERS` | `crates/ss-magic-plugin/src/hook/pre_tool_use/tests.rs:724` | private | `usize` | `8` | test |
| `README_BODY` | `crates/ss-magic-plugin/src/scratchpad.rs:678` | private | `&str` | `"\` |  |
| `README_NAME` | `crates/ss-magic-plugin/src/scratchpad.rs:90` | private | `&str` | `"README.md"` |  |
| `READ_BUF_BYTES` | `crates/ss-magic-plugin/src/ledger.rs:555` | private | `usize` | `256 * 1024` |  |
| `REASON_CWD_MISSING` | `crates/ss-magic-plugin/src/hook/mod.rs:109` | private | `&str` | `"cwd-missing"` |  |
| `REASON_DISABLED` | `crates/ss-magic-plugin/src/hook/mod.rs:111` | private | `&str` | `"disabled"` |  |
| `REASON_ENCODE_FAILED` | `crates/ss-magic-plugin/src/hook/mod.rs:119` | private | `&str` | `"encode-failed"` |  |
| `REASON_HANDLER_ERROR` | `crates/ss-magic-plugin/src/hook/mod.rs:115` | private | `&str` | `"handler-error"` |  |
| `REASON_HANDLER_PANIC` | `crates/ss-magic-plugin/src/hook/mod.rs:117` | private | `&str` | `"handler-panic"` |  |
| `REASON_NOT_IGNORED` | `crates/ss-magic-plugin/src/hook/mod.rs:113` | private | `&str` | `"not-ignored"` |  |
| `REASON_STDIN` | `crates/ss-magic-plugin/src/hook/mod.rs:105` | private | `&str` | `"stdin-read-failed"` |  |
| `REASON_STDOUT` | `crates/ss-magic-plugin/src/hook/mod.rs:107` | private | `&str` | `"stdout-write-failed"` |  |
| `REASON_UNROUTABLE` | `crates/ss-magic-plugin/src/hook/mod.rs:103` | private | `&str` | `"unroutable-event"` |  |
| `RECOMMEND_ROWS` | `crates/ss-magic-plugin/src/compact_window.rs:105` | pub | `usize` | `20` |  |
| `RECOMMEND_SCHEMA_VERSION` | `crates/ss-magic-plugin/src/compact_window.rs:115` | pub | `u32` | `1` |  |
| `RECORD_EXT` | `crates/ss-magic-plugin/src/expect_artifact.rs:75` | private | `&str` | `"json"` |  |
| `REFRESH_ARGV` | `crates/ss-magic-plugin/src/release_check.rs:57` | pub | `[&str; 3]` | `["release-check", "--refresh", "--quiet"]` |  |
| `RELEASES_PER_PAGE` | `crates/ss-magic-core/src/release.rs:64` | private | `u32` | `100` |  |
| `REMEDY` | `crates/ss-magic-plugin/src/release_check.rs:68` | pub | `&str` | `"run /plugin, update ss-magic there, then start a new session \` |  |
| `REPO_SLUG` | `crates/ss-magic-core/src/release.rs:50` | pub | `&str` | `"ViktorStiskala/superset-magic"` |  |
| `ROOT_MARKER` | `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:100` | private | `&str` | `".superset/magic.json"` |  |
| `ROUNDING` | `crates/ss-magic-plugin/src/compact_window.rs:111` | private | `u64` | `10_000` |  |
| `ROWS_KEPT` | `crates/ss-magic-plugin/src/heartbeat.rs:80` | pub | `usize` | `2_000` |  |
| `SALVAGE_BYTE_BUDGET` | `crates/ss-magic-plugin/src/hook/subagent_stop.rs:70` | private | `usize` | `64 * 1024` |  |
| `SALVAGE_DIR` | `crates/ss-magic-plugin/src/hook/subagent_stop.rs:64` | private | `&str` | `"research-salvage"` |  |
| `SAMPLE_PATH` | `crates/ss-magic-plugin/src/checklist/render/tests.rs:54` | private | `&str` | `"docs/actions/2026-08-ship-the-thing.checklist.json"` | test |
| `SCHEMA_ID` | `crates/ss-magic-plugin/src/checklist/schema.rs:84` | pub | `&str` | `"https://github.com/ViktorStiskala/superset-magic/schema/checklist/v1"` |  |
| `SCHEMA_VERSION` | `crates/ss-magic-plugin/src/release_check.rs:60` | pub | `u32` | `1` | dup-name |
| `SCHEMA_VERSION` | `crates/ss-magic-plugin/src/status.rs:74` | pub | `u32` | `2` | dup-name |
| `SCRATCHPAD_USAGE` | `crates/ss-magic-plugin/src/scratchpad.rs:125` | private | `&str` | `"\` |  |
| `SEED_CONFIG_USAGE` | `crates/ss-magic-plugin/src/config.rs:366` | pub(crate) | `&str` | `"\` |  |
| `SEPARATOR` | `crates/ss-magic-plugin/src/cache.rs:112` | private | `&str` | `"\n---\n"` |  |
| `SEQ` | `crates/ss-magic-core/src/superset_files.rs:198` | private | `std::sync::atomic::AtomicU64` | `std::sync::atomic::AtomicU64::new(0)` |  |
| `SESSIONS_DIR` | `crates/ss-magic-plugin/src/scratchpad.rs:107` | private | `&str` | `"sessions"` |  |
| `SETTINGS_LOCAL_REL` | `crates/ss-magic-plugin/src/compact_window.rs:89` | private | `&str` | `".claude/settings.local.json"` |  |
| `SETTINGS_PROJECT_REL` | `crates/ss-magic-plugin/src/compact_window.rs:93` | private | `&str` | `".claude/settings.json"` |  |
| `SETUP_CONFIG_JSON` | `crates/ss-magic-core/src/superset_files.rs:29` | private | `&str` | `"setup_config.json"` |  |
| `SETUP_SH_MARKER` | `crates/ss-magic/src/workspace/migrate.rs:47` | private | `&str` | `"setup.sh"` |  |
| `SETUP_SH_REL` | `crates/ss-magic/src/workspace/migrate.rs:60` | private | `&str` | `".superset/setup.sh"` |  |
| `SHA256_H0` | `crates/ss-magic-core/src/hashing.rs:64` | private | `[u32; 8]` | `[` |  |
| `SHA256_K` | `crates/ss-magic-core/src/hashing.rs:71` | private | `[u32; 64]` | `[` |  |
| `SKIP_DIRS` | `crates/ss-magic-core/src/sync/repo_scan.rs:16` | private | `[&str; 4]` | `["node_modules", ".venv", ".git", "target"]` |  |
| `SPILL_INDEX_USAGE` | `crates/ss-magic-plugin/src/spill_index.rs:323` | private | `&str` | `"\` |  |
| `SPLIT_GUTTER` | `crates/ss-magic/src/tui/diffmodel.rs:179` | pub | `u16` | `5` |  |
| `SPLIT_MIN_PANE_WIDTH` | `crates/ss-magic/src/tui/diffmodel.rs:190` | private | `u16` | `2 * (MIN_SPLIT_COL + SPLIT_GUTTER) + 1` |  |
| `STALE_TTL` | `crates/ss-magic/src/update/apply.rs:93` | private | `Duration` | `Duration::from_secs(60)` |  |
| `STATE_FILES` | `crates/ss-magic-plugin/src/scratchpad.rs:80` | pub | `[&str; 6]` | `[` |  |
| `STATE_FILE_NOTES` | `crates/ss-magic-plugin/src/hook/session_start.rs:68` | private | `[(&str, &str); 6]` | `[` |  |
| `STATE_QUERY` | `crates/ss-magic-plugin/src/hook/mod.rs:93` | private | `&str` | `".superset/.magic/"` | dup-name |
| `STATE_QUERY` | `crates/ss-magic-plugin/src/scratchpad.rs:66` | private | `&str` | `".superset/.magic/"` | dup-name |
| `STATE_REL` | `crates/ss-magic-core/src/state_tree.rs:35` | pub | `&str` | `".superset/.magic"` |  |
| `STATUS_USAGE` | `crates/ss-magic-plugin/src/status.rs:1860` | private | `&str` | `"\` |  |
| `STEM` | `crates/ss-magic-plugin/src/checklist/verbs/tests.rs:24` | private | `&str` | `"2026-08-ship-it"` | test |
| `STORE_SUBDIR` | `crates/ss-magic-plugin/src/heartbeat.rs:58` | pub | `&str` | `"plugin"` |  |
| `STRINGS` | `crates/ss-magic/src/tui/ui.rs:240` | private | `PickerStrings` | `PickerStrings {` |  |
| `SUPERSET_DIR` | `crates/ss-magic-core/src/superset_files.rs:26` | private | `&str` | `".superset"` |  |
| `SUPERSET_REL` | `crates/ss-magic-plugin/src/scratchpad.rs:70` | private | `&str` | `".superset"` |  |
| `T0` | `crates/ss-magic-plugin/src/cache/tests.rs:42` | private | `u64` | `1_760_000_000` | test |
| `TEMPLATE` | `crates/ss-magic-plugin/src/setup_ci.rs:62` | pub | `&str` | `include_str!("../../../assets/workflow/checklist.yml")` |  |
| `THREADS` | `crates/ss-magic-plugin/src/bypass/tests.rs:105` | private | `usize` | `8` | dup-name test |
| `THREADS` | `crates/ss-magic-plugin/src/claim/tests.rs:77` | private | `usize` | `8` | dup-name test |
| `THREADS` | `crates/ss-magic-plugin/src/claim/tests.rs:115` | private | `usize` | `8` | dup-name test |
| `THREADS` | `crates/ss-magic-plugin/src/expect_artifact/tests.rs:192` | private | `usize` | `8` | dup-name test |
| `TOOLS` | `crates/ss-magic/src/pack.rs:58` | private | `&[(&str, &[&str])]` | `&[` |  |
| `TOOL_RESULTS_DIR` | `crates/ss-magic-plugin/src/spill_index.rs:47` | private | `&str` | `"tool-results"` |  |
| `TRIALS` | `crates/ss-magic-plugin/src/claim/tests.rs:81` | private | `usize` | `25` | test |
| `TS` | `crates/ss-magic/src/sync/reverse_sync/tests.rs:315` | private | `&str` | `"20260716-000000"` | test |
| `UNIFIED_GUTTER` | `crates/ss-magic/src/tui/cockpit.rs:62` | private | `u16` | `12` |  |
| `UNRANKED_RANK` | `crates/ss-magic-plugin/src/checklist/order.rs:40` | pub | `u8` | `3` |  |
| `UPDATED_ENV` | `crates/ss-magic/src/update/apply.rs:84` | pub | `&str` | `"SS_MAGIC_UPDATED"` |  |
| `USAGE` | `crates/ss-magic-plugin/src/checklist/verbs.rs:220` | private | `&str` | `"\` | dup-name |
| `USAGE` | `crates/ss-magic-plugin/src/compact_window.rs:134` | private | `&str` | `"\` | dup-name |
| `USAGE` | `crates/ss-magic-plugin/src/expect_artifact.rs:96` | private | `&str` | `"\` | dup-name |
| `USAGE` | `crates/ss-magic-plugin/src/main.rs:341` | pub | `&str` | `"\` | dup-name |
| `USAGE` | `crates/ss-magic-plugin/src/release_check.rs:71` | private | `&str` | `"\` | dup-name |
| `USAGE` | `crates/ss-magic-plugin/src/setup_ci.rs:92` | private | `&str` | `"\` | dup-name |
| `USAGE` | `crates/ss-magic/src/cli.rs:61` | pub | `&str` | `"\` | dup-name |
| `USAGE_NEEDLE` | `crates/ss-magic-plugin/src/ledger.rs:541` | private | `&str` | `"\"usage\""` |  |
| `V` | `crates/ss-magic-plugin/src/setup_ci/tests.rs:29` | private | `&str` | `"9.9.9"` | test |
| `VERBS` | `crates/ss-magic-plugin/src/tests.rs:24` | private | `&[(&str, HumanVerb)]` | `&[` | test |
| `VERSION_PLACEHOLDER` | `crates/ss-magic-plugin/src/setup_ci.rs:68` | pub | `&str` | `"@SS_MAGIC_PLUGIN_VERSION@"` |  |
| `VERSION_TIMEOUT` | `crates/ss-magic-plugin/src/status.rs:152` | private | `Duration` | `Duration::from_secs(5)` |  |
| `WIDTH` | `crates/ss-magic-plugin/src/release_check.rs:576` | private | `usize` | `22` |  |
| `WINDOW_KEY` | `crates/ss-magic-plugin/src/compact_window.rs:118` | private | `&str` | `"autoCompactWindow"` |  |
| `WINDOW_MAX` | `crates/ss-magic-plugin/src/compact_window.rs:125` | private | `u64` | `1_000_000` |  |
| `WINDOW_MIN` | `crates/ss-magic-plugin/src/compact_window.rs:123` | private | `u64` | `100_000` |  |
| `WITNESS` | `crates/ss-magic-plugin/src/hook/file_changed/tests.rs:43` | private | `&str` | `"EXECUTED"` | test |
| `WORKFLOW_REL` | `crates/ss-magic-plugin/src/setup_ci.rs:75` | pub | `&str` | `".github/workflows/ss-magic-checklist.yml"` |  |
| `WRITERS` | `crates/ss-magic-plugin/src/heartbeat/tests.rs:367` | private | `usize` | `8` | test |
