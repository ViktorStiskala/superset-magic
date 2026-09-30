# Repeated string literals (Rust, shell, Python)

Literals that look like names, paths, keys or flags and occur in 2+ files or 3+ times. Comments excluded. `nt-files` = files that are not tests. Sorted by non-test file spread.

| literal | nt-files | files | total | where |
|---|---|---|---|---|
| `, style::err(format!(` | 12 | 12 | 23 | `bypass.rs`:241; `cache.rs`:813,841,909,1007; `verbs.rs`:517,690,711,1439,1457; `compact_window.rs`:838,847; `config.rs`:939; `expect_artifact.rs`:450,477,495; `ledger.rs`:1251; `main.rs`:552; `release_check.rs`:683; `scratchpad.rs`:798; `setup_ci.rs`:237; `pack.rs`:154,195 |
| `{}\n` | 11 | 24 | 37 | `tests.rs`:578,648; `gitignore.rs`:75; `superset_files.rs`:145,161,387; `tests.rs`:296,334,358; `testutil.rs`:107; `bypass.rs`:127; `schema.rs`:499; `verbs.rs`:1308; `tests.rs`:122; `compact_window.rs`:323; `expect_artifact.rs`:231; `tests.rs`:246,326; `tests.rs`:2567; `tests.rs`:60; `session_start.rs`:303; `tests.rs`:496; `tests.rs`:576; `ledger.rs`:353,1151; `scratchpad.rs`:616; `tests.rs`:484; `tests.rs`:375,408; `tests.rs`:37,181,607,645,690; `tests.rs`:47; `tests.rs`:516,517 |
| `reading the current directory` | 11 | 11 | 16 | `bypass.rs`:213; `cache.rs`:803,889,968; `verbs.rs`:464; `compact_window.rs`:159,164; `config.rs`:619,686,724; `expect_artifact.rs`:444; `ledger.rs`:1241; `scratchpad.rs`:749; `setup_ci.rs`:222; `spill_index.rs`:356; `status.rs`:1899 |
| `creating {}` | 11 | 11 | 14 | `gitignore.rs`:76; `superset_files.rs`:344; `bypass.rs`:121; `cache.rs`:376; `verbs.rs`:1283; `compact_window.rs`:321; `expect_artifact.rs`:223; `subagent_stop.rs`:458,485; `ledger.rs`:1144; `scratchpad.rs`:489,514,574; `setup_ci.rs`:442 |
| `CARGO_PKG_VERSION` | 8 | 12 | 16 | `tests.rs`:657; `session_start.rs`:332; `tests.rs`:488,496; `main.rs`:395; `release_check.rs`:709,739; `setup_ci.rs`:223; `status.rs`:1903; `tests.rs`:209; `main.rs`:124; `update_gate.rs`:148; `apply.rs`:407; `mod.rs`:71,73,153 |
| `.tmp` | 8 | 9 | 16 | `tests.rs`:52,70,90,91,117,134…; `bypass.rs`:134; `cache.rs`:391; `verbs.rs`:1296,1316; `claim.rs`:71; `compact_window.rs`:331; `expect_artifact.rs`:238; `release_check.rs`:112; `setup_ci.rs`:448 |
| `reading {}` | 8 | 8 | 13 | `gitignore.rs`:51; `superset_files.rs`:98,106,278,289,302,313; `schema.rs`:514; `compact_window.rs`:300; `heartbeat.rs`:368; `subagent_stop.rs`:295; `ledger.rs`:381; `scratchpad.rs`:424 |
| `, style::info(format!(` | 8 | 8 | 11 | `bypass.rs`:256; `cache.rs`:857,858; `verbs.rs`:757; `expect_artifact.rs`:512; `ledger.rs`:1321; `scratchpad.rs`:790; `spill_index.rs`:397,401; `reverse_sync.rs`:409,647 |
| `writing {}` | 7 | 7 | 12 | `gitignore.rs`:70; `superset_files.rs`:202,266,360,388; `config.rs`:850; `pre_compact.rs`:186,199,203; `subagent_stop.rs`:480; `scratchpad.rs`:569; `reverse_sync.rs`:1086 |
| `, style::warn(format!(` | 7 | 7 | 10 | `bypass.rs`:246; `cache.rs`:846; `verbs.rs`:529,712; `expect_artifact.rs`:500; `scratchpad.rs`:782; `spill_index.rs`:407,426,439; `status.rs`:2341 |
| `.superset` | 6 | 21 | 61 | `tests.rs`:383; `superset_files.rs`:26; `tests.rs`:17,96,117,135,211,233…; `tests.rs`:306; `mod.rs`:41,42; `tests.rs`:41; `testutil.rs`:101; `config.rs`:888; `tests.rs`:776,779,952,954,980,1026…; `tests.rs`:215; `tests.rs`:2551; `tests.rs`:301; `tests.rs`:236; `scratchpad.rs`:70; `tests.rs`:78; `tests.rs`:45,47; `tests.rs`:31,180,282,606,689; `tests.rs`:41,2072; `sync.rs`:125; `migrate.rs`:613,628; `tests.rs`:128,210,266,353,407,424… |
| `.git` | 6 | 13 | 33 | `discover.rs`:290; `tests.rs`:182,189,227,233,248,269…; `mod.rs`:70; `reponame.rs`:64; `tests.rs`:368; `mod.rs`:44; `repo_scan.rs`:16; `tests.rs`:21,33; `verbs.rs`:1350; `tests.rs`:453; `tests.rs`:254; `tests.rs`:410; `tests.rs`:125 |
| `--json` | 6 | 9 | 14 | `compact_window.rs`:207,207,210; `tests.rs`:81,85,88; `file_changed.rs`:316; `ledger.rs`:1223; `release_check.rs`:662; `tests.rs`:542; `spill_index.rs`:344; `status.rs`:691,1886; `tests.rs`:185 |
| `HOME` | 6 | 7 | 11 | `compact_window.rs`:366; `pre_tool_use.rs`:343; `tests.rs`:2291,2292,2303,2304; `ledger.rs`:1168; `spill_index.rs`:122; `status.rs`:807,831; `tmproot.rs`:146 |
| `opening {}` | 6 | 6 | 7 | `heartbeat.rs`:279; `file_changed.rs`:382; `pre_compact.rs`:178,181; `subagent_stop.rs`:291; `ledger.rs`:1101; `scratchpad.rs`:602 |
| `ss-magic` | 5 | 9 | 23 | `release.rs`:489,521; `tests.rs`:957; `heartbeat.rs`:192; `tests.rs`:142,146,166; `status.rs`:94; `tests.rs`:99,387; `tests.rs`:384; `apply.rs`:77; `build-plugin-zip.py`:85,255,314,878,927,928… |
| `Option::is_none` | 5 | 5 | 19 | `release.rs`:184; `schema.rs`:318,361,373,385; `heartbeat.rs`:128,134,138; `event.rs`:422,430,438,446,450,453; `ledger.rs`:250,260,277,287,305 |
| `{USAGE}` | 5 | 5 | 10 | `verbs.rs`:431,1440; `compact_window.rs`:168,839; `expect_artifact.rs`:420,451; `release_check.rs`:679,684; `setup_ci.rs`:204,216 |
| `.superset/magic.json` | 4 | 14 | 62 | `tests.rs`:417,561,589,620,649; `tests.rs`:298,311,332,339; `tests.rs`:44; `testutil.rs`:108; `config.rs`:930; `tests.rs`:11,12,59,171,575,608…; `pre_tool_use.rs`:100; `tests.rs`:92,371,372,375,2552; `tests.rs`:98,355,437,485,512; `main.rs`:205; `tests.rs`:38,283,622,664; `tests.rs`:48,68,70,80; `sync.rs`:126; `tests.rs`:312,616,671,690,697 |
| `.claude` | 4 | 8 | 18 | `compact_window.rs`:366; `tests.rs`:132,153,170,191,212,229…; `tests.rs`:625; `ledger.rs`:1169; `spill_index.rs`:123; `status.rs`:807; `tests.rs`:1190; `tests.rs`:879 |
| `ss-magic-plugin` | 4 | 7 | 15 | `tests.rs`:834; `release_check.rs`:739; `tests.rs`:386; `tmproot.rs`:89; `tmproot.sh`:38; `build-plugin-zip.py`:86,256,314,931,932,933…; `test-mark-latest.sh`:131 |
| `{line}` | 4 | 6 | 9 | `heartbeat.rs`:280; `tests.rs`:317,318,341; `mod.rs`:571,639; `ledger.rs`:1102; `tests.rs`:238; `main.rs`:460 |
| `conclusions` | 4 | 6 | 7 | `cache.rs`:84; `main.rs`:224,254; `scratchpad.rs`:113; `status.rs`:2111; `tests.rs`:161; `tests.rs`:30 |
| `expect-artifact` | 4 | 5 | 6 | `expect_artifact.rs`:72; `main.rs`:227,257; `scratchpad.rs`:113; `status.rs`:2113; `tests.rs`:33 |
| `, style::ok(format!(` | 4 | 4 | 6 | `verbs.rs`:566,694; `compact_window.rs`:266; `ledger.rs`:1457; `migrate.rs`:624,654 |
| `, style::info(` | 4 | 4 | 5 | `cache.rs`:931; `reverse_sync.rs`:364,370; `menu.rs`:195; `migrate.rs`:535 |
| `CLAUDE_CONFIG_DIR` | 4 | 4 | 4 | `compact_window.rs`:364; `ledger.rs`:1163; `spill_index.rs`:117; `status.rs`:805 |
| `README.md` | 3 | 11 | 17 | `tests.rs`:156; `tests.rs`:11,61; `testutil.rs`:93; `tests.rs`:501,526,549,569; `tests.rs`:656; `tests.rs`:51; `scratchpad.rs`:90; `tests.rs`:41; `tests.rs`:24,437; `tests.rs`:10; `build-plugin-zip.py`:307,958 |
| `.superset/magic.local.json` | 3 | 10 | 41 | `tests.rs`:13,27,41,61,88; `superset_files.rs`:35; `tests.rs`:422,722,756,802,826,848…; `tests.rs`:45; `config.rs`:928; `tests.rs`:19; `tests.rs`:92,95,99,174,175,178…; `sync.rs`:81,143; `migrate.rs`:65,298,520,570; `tests.rs`:258,325,333,341,511,516… |
| `pre-tool-use` | 3 | 8 | 18 | `tests.rs`:35,101,108,294,375; `tests.rs`:635; `main.rs`:139,153; `status.rs`:84; `tests.rs`:759,772,793; `tests.rs`:16,49; `build-plugin-zip.py`:900,908; `test-bootstrap.sh`:622,911 |
| `--version` | 3 | 6 | 14 | `mod.rs`:183; `status.rs`:704; `tests.rs`:197,227,230; `cli.rs`:158; `tests.rs`:203,213,214,225,230,238; `update_gate.rs`:131,133 |
| `worktree` | 3 | 6 | 12 | `discover.rs`:573; `tests.rs`:162,349; `testutil.rs`:126; `merge.rs`:262; `tests.rs`:21,462,513,949,993,1272; `sync.rs`:200 |
| `$(dirname ` | 3 | 5 | 5 | `ss-magic-plugin`:54; `bootstrap.sh`:91; `run-hook.sh`:51; `test-bootstrap.sh`:34; `test-mark-latest.sh`:24 |
| `--quiet` | 3 | 4 | 7 | `mod.rs`:108,147,392; `release_check.rs`:57,663; `tests.rs`:542; `build-plugin-zip.py`:800 |
| `, style::header(` | 3 | 3 | 10 | `compact_window.rs`:794; `release_check.rs`:580; `status.rs`:2001,2068,2089,2125,2150,2190… |
| `$bin` | 3 | 3 | 6 | `ss-magic-plugin`:101,108; `run-hook.sh`:95,117; `execguard.sh`:48,49 |
| `$data` | 3 | 3 | 6 | `ss-magic-plugin`:73; `bootstrap.sh`:99,138,289,400; `run-hook.sh`:58 |
| `$lib` | 3 | 3 | 6 | `ss-magic-plugin`:65,67; `bootstrap.sh`:115,116; `run-hook.sh`:60,62 |
| `kebab-case` | 3 | 3 | 4 | `schema.rs`:182,216; `heartbeat.rs`:99; `ledger.rs`:216 |
| `locking {}` | 3 | 3 | 4 | `verbs.rs`:1334; `release_check.rs`:275,340; `scratchpad.rs`:606 |
| `{} — {}` | 3 | 3 | 4 | `verbs.rs`:709; `release_check.rs`:632; `status.rs`:2216,2256 |
| `$data/bin/ss-magic-plugin` | 3 | 3 | 3 | `ss-magic-plugin`:89; `bootstrap.sh`:108; `run-hook.sh`:80 |
| `$root/$SS_MAGIC_DATA_ROOT_FILE` | 3 | 3 | 3 | `ss-magic-plugin`:79; `bootstrap.sh`:139; `run-hook.sh`:67 |
| `://` | 3 | 3 | 3 | `reponame.rs`:56; `validate.rs`:394; `verbs.rs`:1362 |
| `CLAUDE_PLUGIN_ROOT` | 3 | 3 | 3 | `session_start.rs`:136; `release_check.rs`:419; `status.rs`:854 |
| `appending to {}` | 3 | 3 | 3 | `heartbeat.rs`:280; `file_changed.rs`:385; `ledger.rs`:1102 |
| `resolving {}` | 3 | 3 | 3 | `cache.rs`:183; `expect_artifact.rs`:472; `scratchpad.rs`:398 |
| `{e:#}` | 3 | 3 | 3 | `mod.rs`:559; `release_check.rs`:758; `scratchpad.rs`:476 |
| `.gitignore` | 2 | 19 | 93 | `gitignore.rs`:47,261; `tests.rs`:15,23,37,57,71,85…; `tests.rs`:36,41,76,77,99,115; `tests.rs`:34,43,53,54,58; `tests.rs`:156,157; `tests.rs`:31,35; `tests.rs`:33,37; `tests.rs`:28; `tests.rs`:424,428,439,450,453,457…; `tests.rs`:33,37; `tests.rs`:31,35; `tests.rs`:30,31,60,61; `tests.rs`:35,39; `tests.rs`:95,312; `tests.rs`:14,15,43,50,62,82…; `tests.rs`:53,453; `tests.rs`:145,146,172,173,198,199…; `migrate.rs`:613,628; `tests.rs`:288,455,558,918,922 |
| `.env` | 2 | 12 | 80 | `tests.rs`:53; `tests.rs`:12,63,192,214,222,251…; `tests.rs`:41,42,45,46,99,100…; `tests.rs`:9,24; `repo_scan.rs`:14; `tests.rs`:18; `file_changed.rs`:120; `tests.rs`:366,372,385,391; `tests.rs`:87,88,96,155,166,204…; `sync.rs`:14,15,21,24,178,179…; `tests.rs`:19,20; `tests.rs`:526,530,752,831 |
| `.superset/.magic/` | 2 | 9 | 16 | `tests.rs`:36,39; `tests.rs`:454,480,583,599; `mod.rs`:93; `tests.rs`:460; `scratchpad.rs`:66; `tests.rs`:54,64; `tests.rs`:627,667; `tests.rs`:2019; `tests.rs`:570,923 |
| `{err:#}` | 2 | 7 | 17 | `tests.rs`:68; `tests.rs`:138,236,405,479,493,679…; `apply.rs`:152; `verbs.rs`:750,1219; `tests.rs`:1036,1038,1047; `tests.rs`:510,510; `tests.rs`:132 |
| `session-start` | 2 | 6 | 24 | `tests.rs`:53,63,101,108,115,122…; `main.rs`:138,152; `status.rs`:83; `tests.rs`:789,790,805,823,825,848…; `tests.rs`:15,81,234; `test-bootstrap.sh`:621,911 |
| `PreToolUse` | 2 | 6 | 21 | `event.rs`:495; `tests.rs`:47,88,183,251; `tests.rs`:105; `tests.rs`:338,386,418,579,609,631…; `build-plugin-zip.py`:989; `test-bootstrap.sh`:622,911 |
| `SessionStart` | 2 | 6 | 16 | `event.rs`:482; `tests.rs`:29,87,119,216,352,358; `tests.rs`:75; `tests.rs`:522,715,1103; `build-plugin-zip.py`:976; `test-bootstrap.sh`:621,627,798,911 |
| `magic.json` | 2 | 6 | 14 | `superset_files.rs`:30; `tests.rs`:258,480,680; `tests.rs`:13,947,1020; `tests.rs`:47; `migrate.rs`:251; `tests.rs`:272,449,489,544,660 |
| `threshold_lines` | 2 | 5 | 22 | `tests.rs`:539; `config.rs`:247,416; `tests.rs`:38,188,201,239,272,280…; `status.rs`:2128; `tests.rs`:170 |
| `session-end` | 2 | 5 | 16 | `tests.rs`:77,101,108,119,122,277…; `main.rs`:142,156; `status.rs`:87; `tests.rs`:19; `test-bootstrap.sh`:625,913 |
| `STATUS.md` | 2 | 5 | 11 | `tests.rs`:127; `session_start.rs`:86; `tests.rs`:418,434; `scratchpad.rs`:85,721; `tests.rs`:167,217,223,419,422 |
| `assistant` | 2 | 5 | 9 | `tests.rs`:35; `subagent_stop.rs`:351; `tests.rs`:146,148,151,578; `ledger.rs`:727; `tests.rs`:29,209 |
| `not-ignored` | 2 | 5 | 8 | `tests.rs`:55,68; `mod.rs`:113; `tests.rs`:458,525; `scratchpad.rs`:203; `tests.rs`:52,299 |
| `--local` | 2 | 5 | 7 | `testutil.rs`:35; `tests.rs`:1087; `config.rs`:615,710; `tests.rs`:653; `tests.rs`:147,150 |
| `{out}` | 2 | 4 | 17 | `release_check.rs`:718; `tests.rs`:483,484; `status.rs`:1948; `tests.rs`:1072,1073,1074,1075,1108,1109… |
| `/tmp` | 2 | 4 | 12 | `tests.rs`:196,212,230,254,276,311; `tests.rs`:299,941,981,985; `status.rs`:833; `tmproot.rs`:149 |
| `magic.sh` | 2 | 4 | 11 | `superset_files.rs`:28; `tests.rs`:256,263; `migrate.rs`:182; `tests.rs`:273,280,452,490,496,545… |
| `HEAD` | 2 | 4 | 10 | `discover.rs`:281,402; `tests.rs`:522,540,642,647; `mod.rs`:392,402; `tests.rs`:11,94 |
| `rev-parse` | 2 | 4 | 9 | `tests.rs`:48; `mod.rs`:62,74,75,84,106,402; `tests.rs`:11; `build-plugin-zip.py`:800 |
| `ss-magic-plugin.version` | 2 | 4 | 9 | `tests.rs`:483,495,678; `status.rs`:106; `tests.rs`:622; `build-plugin-zip.py`:261,879,970,1157 |
| `pre-compact` | 2 | 4 | 6 | `main.rs`:140,154; `status.rs`:85; `tests.rs`:17; `test-bootstrap.sh`:623,912 |
| `subagent-stop` | 2 | 4 | 6 | `main.rs`:141,155; `status.rs`:86; `tests.rs`:18; `test-bootstrap.sh`:624,912 |
| `decision` | 2 | 4 | 5 | `tests.rs`:78,113; `schema.rs`:200; `verbs.rs`:971; `tests.rs`:287 |
| `override` | 2 | 4 | 5 | `compact_window.rs`:400; `tests.rs`:753,754; `status.rs`:393; `tests.rs`:1257 |
| `unroutable-event` | 2 | 4 | 5 | `event.rs`:262; `tests.rs`:176; `mod.rs`:103; `tests.rs`:231,247 |
| `TASKS.md` | 2 | 3 | 6 | `session_start.rs`:89; `scratchpad.rs`:86,730; `tests.rs`:218,226,438 |
| `document` | 2 | 3 | 6 | `validate.rs`:92; `verbs.rs`:866; `tests.rs`:325,458,565,881 |
| `exemptions` | 2 | 3 | 6 | `config.rs`:270,424; `tests.rs`:255,398,809; `status.rs`:2138 |
| `inline_byte_budget` | 2 | 3 | 6 | `config.rs`:257,420; `tests.rs`:214,224,808; `status.rs`:2133 |
| `blocking` | 2 | 3 | 5 | `schema.rs`:230; `verbs.rs`:980; `tests.rs`:295,395,703 |
| `projects` | 2 | 3 | 5 | `ledger.rs`:1165,1169; `spill_index.rs`:119,123; `tests.rs`:18 |
| `OPERATOR-CHECKLIST.md` | 2 | 3 | 4 | `session_start.rs`:82; `scratchpad.rs`:84,714; `tests.rs`:127 |
| `bin/ss-magic-plugin` | 2 | 3 | 4 | `status.rs`:129; `tests.rs`:516; `build-plugin-zip.py`:882,1073 |
| `follow-up` | 2 | 3 | 4 | `tests.rs`:80,114; `schema.rs`:232; `verbs.rs`:982 |
| `release-check` | 2 | 3 | 4 | `main.rs`:233,263; `release_check.rs`:57; `tests.rs`:39 |
| `.venv` | 2 | 3 | 3 | `apply.rs`:28; `repo_scan.rs`:16; `tests.rs`:237 |
| `decision-blocking` | 2 | 3 | 3 | `schema.rs`:231; `tests.rs`:244; `verbs.rs`:981 |
| `node_modules` | 2 | 3 | 3 | `apply.rs`:28; `repo_scan.rs`:16; `tests.rs`:233 |
| `sessions` | 2 | 3 | 3 | `scratchpad.rs`:107; `tests.rs`:510; `status.rs`:1292 |
| `, style::ok(` | 2 | 2 | 20 | `status.rs`:2337; `migrate.rs`:365,366,367,368,369,370… |
| `  {}` | 2 | 2 | 11 | `compact_window.rs`:768,773,775,778,800,810…; `ui.rs`:281 |
| `no reason recorded` | 2 | 2 | 10 | `release_check.rs`:594,606; `status.rs`:1207,1343,1387,1468,1605,1617… |
| `$guard` | 2 | 2 | 4 | `ss-magic-plugin`:97,99; `run-hook.sh`:91,93 |
| `$handoff` | 2 | 2 | 4 | `ss-magic-plugin`:80,84; `run-hook.sh`:68,76 |
| `.jsonl` | 2 | 2 | 4 | `heartbeat.rs`:352; `ledger.rs`:358,1127,1154 |
| `Vec::is_empty` | 2 | 2 | 4 | `schema.rs`:321,388; `ledger.rs`:256,292 |
| `non-UTF-8 path: {}` | 2 | 2 | 4 | `gitignore.rs`:119,285,292; `mod.rs`:276 |
| `unexpected argument `{extra}`` | 2 | 2 | 4 | `cache.rs`:780,882,964; `expect_artifact.rs`:434 |
| `unknown — {}` | 2 | 2 | 4 | `release_check.rs`:593,605; `status.rs`:1960,2284 |
| `CONTEXT.md` | 2 | 2 | 3 | `session_start.rs`:70; `scratchpad.rs`:81,699 |
| `DECISIONS.md` | 2 | 2 | 3 | `session_start.rs`:74; `scratchpad.rs`:82,704 |
| `LEARNINGS.md` | 2 | 2 | 3 | `session_start.rs`:78; `scratchpad.rs`:83,709 |
| `copy {} → {}` | 2 | 2 | 3 | `superset_files.rs`:470; `apply.rs`:132,355 |
| `getting current directory` | 2 | 2 | 3 | `config.rs`:525; `main.rs`:148,167 |
| `{usage}` | 2 | 2 | 3 | `cache.rs`:1008; `config.rs`:612,940 |
| `$marked` | 2 | 2 | 2 | `bootstrap.sh`:198; `mark-latest.sh`:129 |
| `$self_dir/../lib/execguard.sh` | 2 | 2 | 2 | `ss-magic-plugin`:96; `run-hook.sh`:90 |
| `$self_dir/../lib/tmproot.sh` | 2 | 2 | 2 | `ss-magic-plugin`:64; `run-hook.sh`:59 |
| `${data:-}` | 2 | 2 | 2 | `ss-magic-plugin`:85; `run-hook.sh`:77 |
| `--check` | 2 | 2 | 2 | `setup_ci.rs`:207; `build-plugin-zip.py`:1284 |
| `--no-index` | 2 | 2 | 2 | `gitignore.rs`:121; `mod.rs`:309 |
| `--verify` | 2 | 2 | 2 | `mod.rs`:107; `build-plugin-zip.py`:800 |
| `\n{}\n` | 2 | 2 | 2 | `style.rs`:123; `file_changed.rs`:383 |
| `check-ignore` | 2 | 2 | 2 | `gitignore.rs`:121; `mod.rs`:307 |
| `checklist.json` | 2 | 2 | 2 | `verbs.rs`:101; `session_start.rs`:62 |
| `compiling glob `{pattern}`` | 2 | 2 | 2 | `apply.rs`:297; `repo_scan.rs`:27 |
| `current.json` | 2 | 2 | 2 | `scratchpad.rs`:96; `status.rs`:1311 |
| `data-root` | 2 | 2 | 2 | `status.rs`:144; `tmproot.sh`:48 |
| `deleting {}` | 2 | 2 | 2 | `superset_files.rs`:482; `migrate.rs`:384 |
| `error: unexpected argument `{other}`` | 2 | 2 | 2 | `spill_index.rs`:348; `status.rs`:1891 |
| `https://` | 2 | 2 | 2 | `verbs.rs`:1359; `build-plugin-zip.py`:415 |
| `install.lock` | 2 | 2 | 2 | `tmproot.rs`:97; `tmproot.sh`:42 |
| `main → worktree` | 2 | 2 | 2 | `reverse_sync.rs`:494; `cockpit.rs`:488 |
| `malformed JSON in {}` | 2 | 2 | 2 | `superset_files.rs`:328; `schema.rs`:516 |
| `merged → both` | 2 | 2 | 2 | `reverse_sync.rs`:495; `cockpit.rs`:490 |
| `no recommendation` | 2 | 2 | 2 | `compact_window.rs`:811; `status.rs`:1043 |
| `ss-magic: %s\n` | 2 | 2 | 2 | `ss-magic-plugin`:50; `bootstrap.sh`:71 |
| `unexpected argument `{other}`` | 2 | 2 | 2 | `ledger.rs`:1228; `release_check.rs`:664 |
| `worktree → main` | 2 | 2 | 2 | `reverse_sync.rs`:493; `cockpit.rs`:482 |
| `{rel}/` | 2 | 2 | 2 | `subagent_stop.rs`:560; `build-plugin-zip.py`:143 |
| `{tokens} tokens (confidence {})` | 2 | 2 | 2 | `compact_window.rs`:802; `status.rs`:1033 |
| `**/.dev.vars` | 1 | 11 | 38 | `tests.rs`:125,312; `tests.rs`:63,214,223,431,434,442…; `tests.rs`:70,168; `tests.rs`:26; `repo_scan.rs`:14; `tests.rs`:102; `tests.rs`:68,148,200,257,1907,1945; `reverse_sync_flow.rs`:21,48; `sync.rs`:32,79; `tests.rs`:39; `tests.rs`:508,518,695,711 |
| `**/.env` | 1 | 8 | 48 | `tests.rs`:63,430,434,442,447,455…; `tests.rs`:86,195,277; `repo_scan.rs`:14; `tests.rs`:96,98; `tests.rs`:116,154,223,261; `tests.rs`:1530; `sync.rs`:47,60,78,141; `tests.rs`:215,243,475,508,517,541… |
| `.superset/.magic` | 1 | 8 | 20 | `state_tree.rs`:35; `tests.rs`:316,343,346,368; `tests.rs`:17,32; `tests.rs`:23,381,397,406; `tests.rs`:292,334,338,385,388; `tests.rs`:48,672; `tests.rs`:466; `tests.rs`:452 |
| `verification` | 1 | 7 | 29 | `tests.rs`:90; `tests.rs`:46; `schema.rs`:433; `tests.rs`:356,425; `tests.rs`:81; `tests.rs`:193; `tests.rs`:232,293,357,379,380,421… |
| `.superset/backups` | 1 | 7 | 18 | `tests.rs`:204,209,236,389; `tests.rs`:320,368; `tests.rs`:15; `reverse_sync.rs`:661; `tests.rs`:1922,1956,1976,2074,2086; `reverse_sync_flow.rs`:58; `sync.rs`:192,226,249,268 |
| `1.0.0` | 1 | 6 | 74 | `tests.rs`:272,280,281,282,283,284…; `tests.rs`:805,841,887,925,969,1020…; `tests.rs`:92,96,100,104,108,112…; `tests.rs`:1322,1349,1376; `tests.rs`:86,108,124,137,157,165; `build-plugin-zip.py`:917,1119,1119,1120,1121,1122… |
| `.magic` | 1 | 5 | 8 | `tests.rs`:389,396; `mod.rs`:42; `tests.rs`:473; `tests.rs`:689,693,702; `tests.rs`:2082 |
| `BEGIN-UNTRUSTED-DATA` | 1 | 5 | 6 | `cache.rs`:444; `tests.rs`:119,257; `tests.rs`:1053; `tests.rs`:908; `tests.rs`:390 |
| `.scratchpad` | 1 | 5 | 5 | `tests.rs`:346; `tests.rs`:368; `mod.rs`:43; `tests.rs`:19; `tests.rs`:176 |
| `{text}` | 1 | 4 | 18 | `cache.rs`:916; `tests.rs`:502,503,541,542,612,613…; `tests.rs`:397,398,399,461; `tests.rs`:231,232,233,234,235,236 |
| `hookSpecificOutput` | 1 | 4 | 13 | `event.rs`:420,436; `tests.rs`:216,217,250,272,289; `tests.rs`:331,2786; `tests.rs`:348,592,719,793 |
| `permissionDecision` | 1 | 4 | 9 | `event.rs`:446; `tests.rs`:252,274; `tests.rs`:333,340,2788; `tests.rs`:348,592,793 |
| `autoCompactWindow` | 1 | 4 | 8 | `compact_window.rs`:118; `tests.rs`:193,689,698,820,824; `tests.rs`:628; `tests.rs`:1193 |
| `disabled` | 1 | 4 | 8 | `tests.rs`:37; `mod.rs`:111; `tests.rs`:326,366,819,932; `tests.rs`:792,814 |
| `additionalContext` | 1 | 4 | 7 | `event.rs`:430,453; `tests.rs`:217,258,273; `tests.rs`:2791; `tests.rs`:719 |
| `bypassPermissions` | 1 | 4 | 7 | `tests.rs`:353,356; `mod.rs`:158; `tests.rs`:851; `tests.rs`:1113,1161,1164 |
| `magic.local.json` | 1 | 4 | 7 | `superset_files.rs`:31; `tests.rs`:494,760; `tests.rs`:1020; `tests.rs`:274,453,547 |
| `{detail}` | 1 | 4 | 7 | `tests.rs`:236,374; `tests.rs`:465,859; `tests.rs`:460,461; `scratchpad.rs`:215 |
| `9.9.9` | 1 | 4 | 5 | `tests.rs`:29; `tests.rs`:82,122; `build-plugin-zip.py`:878; `test-bootstrap.sh`:161 |
| `END-UNTRUSTED-DATA` | 1 | 4 | 4 | `cache.rs`:445; `tests.rs`:258; `tests.rs`:913; `tests.rs`:396 |
| `claude-sonnet-5` | 1 | 3 | 42 | `tests.rs`:39; `ledger.rs`:126; `tests.rs`:165,166,183,213,237,318… |
| `file_path` | 1 | 3 | 34 | `tests.rs`:48,57; `pre_tool_use.rs`:878; `tests.rs`:122,128,657,665,1034,1112… |
| `$(cat ` | 1 | 3 | 22 | `bootstrap.sh`:286; `test-bootstrap.sh`:261,420,477,507,553,564…; `test-mark-latest.sh`:128,140,145 |
| `config.json` | 1 | 3 | 16 | `superset_files.rs`:27; `tests.rs`:97,101,139,257,287,291…; `tests.rs`:133,219,358,427,445,546… |
| `expected` | 1 | 3 | 15 | `tests.rs`:234,257,282,294,308,326…; `verbs.rs`:958,1025; `tests.rs`:138,365,711,716,746,754 |
| `0.11.0` | 1 | 3 | 11 | `tests.rs`:325; `build-plugin-zip.py`:466; `test-bootstrap.sh`:326,341,441,473,476,477… |
| `priority` | 1 | 3 | 11 | `tests.rs`:228,244,254,263; `verbs.rs`:977,1020; `tests.rs`:295,395,703,712,721 |
| `settings.json` | 1 | 3 | 10 | `compact_window.rs`:369; `tests.rs`:441,585,630,653,769,800; `tests.rs`:1133,1145,1170 |
| `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` | 1 | 3 | 9 | `compact_window.rs`:100; `tests.rs`:586,631,637,770; `tests.rs`:1146,1179,1207,1240 |
| `plugin.enabled` | 1 | 3 | 9 | `tests.rs`:315,409,512,676,699; `status.rs`:2004; `tests.rs`:147,150,270 |
| `completed` | 1 | 3 | 8 | `tests.rs`:235,395; `verbs.rs`:961,1022; `tests.rs`:364,793,797,805 |
| `checklist` | 1 | 3 | 7 | `tests.rs`:800,2509,2732,2745; `main.rs`:235,265; `tests.rs`:41 |
| `file-changed` | 1 | 3 | 7 | `main.rs`:143,157; `tests.rs`:899,912,921,932; `tests.rs`:20 |
| `seed-config` | 1 | 3 | 7 | `main.rs`:231,261; `tests.rs`:37,431; `test-bootstrap.sh`:553,564,577 |
| `setup_config.json` | 1 | 3 | 7 | `superset_files.rs`:29; `tests.rs`:237; `tests.rs`:214,269,356,426,441 |
| `v0.12.0` | 1 | 3 | 7 | `tests.rs`:70,253; `build-plugin-zip.py`:1180; `test-mark-latest.sh`:59,80,103,132 |
| `v0.11.1` | 1 | 3 | 6 | `tests.rs`:719,761; `tests.rs`:351,532,535; `build-plugin-zip.py`:1176 |
| `.env.local` | 1 | 3 | 5 | `tests.rs`:63,641,653; `repo_scan.rs`:14; `tests.rs`:52 |
| `.superset/setup.sh` | 1 | 3 | 5 | `tests.rs`:353,354,357; `migrate.rs`:60; `tests.rs`:311 |
| `Verification` | 1 | 3 | 5 | `tests.rs`:47,152,163; `schema.rs`:433; `tests.rs`:357 |
| `cache_creation_input_tokens` | 1 | 3 | 5 | `tests.rs`:44; `ledger.rs`:779,799; `tests.rs`:39,218 |
| `conclude` | 1 | 3 | 5 | `main.rs`:223,253; `tests.rs`:29,227; `tests.rs`:225 |
| `handler-error` | 1 | 3 | 5 | `mod.rs`:115; `tests.rs`:614,637; `tests.rs`:761,776 |
| `hookEventName` | 1 | 3 | 5 | `event.rs`:428,444; `tests.rs`:216,251; `tests.rs`:340 |
| `no-op` | 1 | 3 | 5 | `tests.rs`:67,90; `status.rs`:523,1852; `tests.rs`:810 |
| `not-an-envelope` | 1 | 3 | 5 | `event.rs`:261; `tests.rs`:150,155,166; `tests.rs`:201 |
| `v0.9.0` | 1 | 3 | 5 | `tests.rs`:200,202,449; `build-plugin-zip.py`:1178; `test-mark-latest.sh`:109 |
| `.superset/setup_config.json` | 1 | 3 | 4 | `tests.rs`:213,234; `migrate.rs`:381; `tests.rs`:409 |
| `cache_read_input_tokens` | 1 | 3 | 4 | `tests.rs`:43; `ledger.rs`:785; `tests.rs`:38,217 |
| `changelog` | 1 | 3 | 4 | `render.rs`:128; `tests.rs`:349; `tests.rs`:441,486 |
| `gitBranch` | 1 | 3 | 4 | `tests.rs`:37; `ledger.rs`:752; `tests.rs`:31,211 |
| `input_tokens` | 1 | 3 | 4 | `tests.rs`:41; `ledger.rs`:783; `tests.rs`:36,215 |
| `output_tokens` | 1 | 3 | 4 | `tests.rs`:42; `ledger.rs`:784; `tests.rs`:37,216 |
| `permission_mode is bypassPermissions` | 1 | 3 | 4 | `mod.rs`:158; `tests.rs`:611; `tests.rs`:189,213 |
| `render-md` | 1 | 3 | 4 | `verbs.rs`:410,411; `tests.rs`:1134; `tests.rs`:2228 |
| `)/..` | 1 | 3 | 3 | `bootstrap.sh`:91; `test-bootstrap.sh`:34; `test-mark-latest.sh`:24 |
| `.claude/settings.local.json` | 1 | 3 | 3 | `compact_window.rs`:89; `tests.rs`:627; `tests.rs`:1192 |
| `ViktorStiskala/superset-magic` | 1 | 3 | 3 | `release.rs`:50; `tests.rs`:941; `tests.rs`:224 |
| `add-item` | 1 | 3 | 3 | `verbs.rs`:362; `tests.rs`:1111; `tests.rs`:2228 |
| `cache_creation` | 1 | 3 | 3 | `tests.rs`:45; `ledger.rs`:770; `tests.rs`:40 |
| `ephemeral_1h_input_tokens` | 1 | 3 | 3 | `tests.rs`:47; `ledger.rs`:774; `tests.rs`:42 |
| `ephemeral_5m_input_tokens` | 1 | 3 | 3 | `tests.rs`:46; `ledger.rs`:773; `tests.rs`:41 |
| `follow-ups` | 1 | 3 | 3 | `tests.rs`:146; `schema.rs`:436; `tests.rs`:81 |
| `malformed-stdin` | 1 | 3 | 3 | `event.rs`:260; `tests.rs`:141; `tests.rs`:190 |
| `permissionDecisionReason` | 1 | 3 | 3 | `event.rs`:449; `tests.rs`:254; `tests.rs`:340 |
| `pre_tool_use` | 1 | 3 | 3 | `mod.rs`:324; `tests.rs`:229; `tests.rs`:275 |
| `undecided` | 1 | 3 | 3 | `reverse_sync.rs`:1199; `tests.rs`:723; `tests.rs`:263 |
| `v0.11.0` | 1 | 3 | 3 | `tests.rs`:761; `build-plugin-zip.py`:1177; `test-mark-latest.sh`:109 |
| `{session_id}.jsonl` | 1 | 3 | 3 | `tests.rs`:59; `ledger.rs`:1176; `tests.rs`:87 |
| `.envrc` | 1 | 2 | 47 | `file_changed.rs`:120; `tests.rs`:229,235,262,268,289,295… |
| `isPrerelease` | 1 | 2 | 22 | `mark-latest.sh`:93; `test-mark-latest.sh`:59,59,59,59,59,59… |
| `$pin` | 1 | 2 | 10 | `bootstrap.sh`:166,170,198,200,453,472…; `test-bootstrap.sh`:378 |
| `setup.sh` | 1 | 2 | 10 | `migrate.rs`:47; `tests.rs`:212,267,355,425,437,468… |
| `./.superset/magic.sh sync` | 1 | 2 | 9 | `tests.rs`:92,102,248,278,286,341…; `migrate.rs`:57 |
| `0.11.1` | 1 | 2 | 9 | `tests.rs`:836; `build-plugin-zip.py`:916,1146,1146,1148,1154,1165… |
| `PRE-COMPACT.md` | 1 | 2 | 8 | `pre_compact.rs`:52; `tests.rs`:144,160,179,194,255,277… |
| `--recommend` | 1 | 2 | 7 | `compact_window.rs`:206,207,207; `tests.rs`:77,81,85,90 |
| `--set` | 1 | 2 | 7 | `compact_window.rs`:196; `tests.rs`:35,41,53,63,90,608 |
| `0.1.0` | 1 | 2 | 7 | `tests.rs`:353,357,384,462,478; `build-plugin-zip.py`:944,950 |
| `ss-magic-core` | 1 | 2 | 6 | `tests.rs`:385; `build-plugin-zip.py`:257,942,943,944,950 |
| `{print $NF}` | 1 | 2 | 6 | `bootstrap.sh`:188,452; `test-bootstrap.sh`:348,425,472,476 |
| `--no-backup` | 1 | 2 | 5 | `cli.rs`:139; `tests.rs`:115,133,150,166 |
| `commondir` | 1 | 2 | 5 | `discover.rs`:285,318,358; `tests.rs`:648,658 |
| `escapes-worktree` | 1 | 2 | 5 | `scratchpad.rs`:204; `tests.rs`:278,377,404,433 |
| `## Operator checklist` | 1 | 2 | 4 | `session_start.rs`:584; `tests.rs`:308,360,399 |
| `--refresh` | 1 | 2 | 4 | `release_check.rs`:57,661; `tests.rs`:509,542 |
| `0.9.0` | 1 | 2 | 4 | `tests.rs`:80,606,654; `build-plugin-zip.py`:466 |
| `GIT_DIR` | 1 | 2 | 4 | `discover.rs`:120; `tests.rs`:854,855,866 |
| `Local file` | 1 | 2 | 4 | `cockpit.rs`:1193; `tests.rs`:111,136,561 |
| `Main branch` | 1 | 2 | 4 | `cockpit.rs`:1200; `tests.rs`:112,136,562 |
| `NotebookEdit` | 1 | 2 | 4 | `pre_tool_use.rs`:671; `tests.rs`:1027,1058,1242 |
| `deny: checklist path` | 1 | 2 | 4 | `pre_tool_use.rs`:718; `tests.rs`:1122,1200,1421 |
| `process environment` | 1 | 2 | 4 | `compact_window.rs`:554,557; `tests.rs`:651,674 |
| `recommended window` | 1 | 2 | 4 | `status.rs`:2180,2181,2184; `tests.rs`:1285 |
| `reverse-sync` | 1 | 2 | 4 | `cli.rs`:113; `tests.rs`:107,141,158 |
| `systemMessage` | 1 | 2 | 4 | `event.rs`:422,438; `tests.rs`:218,259 |
| `unknown-agent` | 1 | 2 | 4 | `subagent_stop.rs`:518,533; `tests.rs`:624,631 |
| `--cached` | 1 | 2 | 3 | `mod.rs`:147,332; `tests.rs`:904 |
| `--git-common-dir` | 1 | 2 | 3 | `tests.rs`:48; `mod.rs`:75,84 |
| `--short` | 1 | 2 | 3 | `mod.rs`:392,402; `tests.rs`:11 |
| `.git/` | 1 | 2 | 3 | `gitignore.rs`:165; `tests.rs`:437,667 |
| `Follow-ups` | 1 | 2 | 3 | `tests.rs`:146,150; `schema.rs`:436 |
| `GIT_CEILING_DIRECTORIES` | 1 | 2 | 3 | `discover.rs`:123; `tests.rs`:869,889 |
| `GIT_DISCOVERY_ACROSS_FILESYSTEM` | 1 | 2 | 3 | `discover.rs`:125; `tests.rs`:870,888 |
| `GIT_OBJECT_DIRECTORY` | 1 | 2 | 3 | `discover.rs`:128; `tests.rs`:871,890 |
| `binary-behind` | 1 | 2 | 3 | `status.rs`:1556,1622; `tests.rs`:656 |
| `compact-window` | 1 | 2 | 3 | `main.rs`:232,262; `tests.rs`:38 |
| `delete (worktree + main)` | 1 | 2 | 3 | `cockpit.rs`:822; `tests.rs`:374,915 |
| `delete (worktree copy)` | 1 | 2 | 3 | `cockpit.rs`:820; `tests.rs`:375,919 |
| `description` | 1 | 2 | 3 | `tests.rs`:233; `verbs.rs`:952,1023 |
| `detached-{sha}` | 1 | 2 | 3 | `identity.rs`:107; `tests.rs`:97,120 |
| `docs/actions` | 1 | 2 | 3 | `verbs.rs`:94; `tests.rs`:1461,1757 |
| `fallback` | 1 | 2 | 3 | `tests.rs`:896,936; `build-plugin-zip.py`:1238 |
| `installed` | 1 | 2 | 3 | `status.rs`:1502,1506; `tests.rs`:548 |
| `newest release` | 1 | 2 | 3 | `status.rs`:2272,2282; `tests.rs`:1338 |
| `notebook_path` | 1 | 2 | 3 | `pre_tool_use.rs`:878; `tests.rs`:1059,1243 |
| `pct override` | 1 | 2 | 3 | `status.rs`:2157,2165; `tests.rs`:1283 |
| `scratchpad` | 1 | 2 | 3 | `main.rs`:222,252; `tests.rs`:28 |
| `setup-github-ci` | 1 | 2 | 3 | `main.rs`:234,264; `tests.rs`:40 |
| `spill-index` | 1 | 2 | 3 | `main.rs`:221,251; `tests.rs`:27 |
| `{} {}` | 1 | 2 | 3 | `tests.rs`:46,54; `ui.rs`:78 |
| `$schema` | 1 | 2 | 2 | `schema.rs`:273; `tests.rs`:343 |
| `--note` | 1 | 2 | 2 | `expect_artifact.rs`:423; `tests.rs`:425 |
| `--porcelain` | 1 | 2 | 2 | `mod.rs`:350; `tests.rs`:468 |
| `.claude/settings.json` | 1 | 2 | 2 | `compact_window.rs`:93; `tests.rs`:192 |
| `.git is a symlink` | 1 | 2 | 2 | `discover.rs`:298; `tests.rs`:420 |
| `0.11.3` | 1 | 2 | 2 | `tests.rs`:297; `build-plugin-zip.py`:1159 |
| `CLAUDE_CODE_ENTRYPOINT` | 1 | 2 | 2 | `mod.rs`:128; `tests.rs`:1168 |
| `CLAUDE_PLUGIN_DATA` | 1 | 2 | 2 | `status.rs`:794; `test-bootstrap.sh`:657 |
| `GIT_COMMON_DIR` | 1 | 2 | 2 | `discover.rs`:122; `tests.rs`:868 |
| `GIT_WORK_TREE` | 1 | 2 | 2 | `discover.rs`:121; `tests.rs`:867 |
| `MAJOR.MINOR.PATCH` | 1 | 2 | 2 | `tests.rs`:499; `build-plugin-zip.py`:1170 |
| `MultiEdit` | 1 | 2 | 2 | `pre_tool_use.rs`:671; `tests.rs`:1026 |
| `NO_COLOR` | 1 | 2 | 2 | `style.rs`:54; `tests.rs`:924 |
| `Navigation` | 1 | 2 | 2 | `cockpit.rs`:1352; `tests.rs`:875 |
| `Untitled checklist` | 1 | 2 | 2 | `render.rs`:152; `tests.rs`:173 |
| `add-entry` | 1 | 2 | 2 | `verbs.rs`:375; `tests.rs`:2228 |
| `allow: non-text extension .{ext}` | 1 | 2 | 2 | `pre_tool_use.rs`:763; `tests.rs`:452 |
| `cache_write_1h_multiplier` | 1 | 2 | 2 | `ledger.rs`:181; `tests.rs`:827 |
| `claude-opus-4-8` | 1 | 2 | 2 | `ledger.rs`:123; `tests.rs`:236 |
| `commondir cannot be canonicalized` | 1 | 2 | 2 | `discover.rs`:368; `tests.rs`:660 |
| `cost-state` | 1 | 2 | 2 | `ledger.rs`:726; `tests.rs`:54 |
| `cost.jsonl` | 1 | 2 | 2 | `tests.rs`:324; `ledger.rs`:82 |
| `cost.lock` | 1 | 2 | 2 | `tests.rs`:325; `ledger.rs`:96 |
| `cwd cannot be canonicalized` | 1 | 2 | 2 | `discover.rs`:207; `tests.rs`:837 |
| `cwd-missing` | 1 | 2 | 2 | `mod.rs`:109; `tests.rs`:562 |
| `decisions` | 1 | 2 | 2 | `schema.rs`:435; `tests.rs`:81 |
| `delete (main copy)` | 1 | 2 | 2 | `cockpit.rs`:821; `tests.rs`:1357 |
| `file_changed` | 1 | 2 | 2 | `mod.rs`:332; `tests.rs`:279 |
| `filesystem boundary` | 1 | 2 | 2 | `discover.rs`:226; `tests.rs`:826 |
| `gitdir: ` | 1 | 2 | 2 | `discover.rs`:342; `tests.rs`:191 |
| `gitfile names no path` | 1 | 2 | 2 | `discover.rs`:349; `tests.rs`:601 |
| `handler-panic` | 1 | 2 | 2 | `mod.rs`:117; `tests.rs`:672 |
| `hooks.lock` | 1 | 2 | 2 | `heartbeat.rs`:67; `tests.rs`:334 |
| `it does not exist` | 1 | 2 | 2 | `expect_artifact.rs`:152; `tests.rs`:203 |
| `never-run` | 1 | 2 | 2 | `status.rs`:1523; `tests.rs`:517 |
| `no-input` | 1 | 2 | 2 | `event.rs`:259; `tests.rs`:176 |
| `not shown yet` | 1 | 2 | 2 | `release_check.rs`:625; `tests.rs`:484 |
| `not-a-directory` | 1 | 2 | 2 | `scratchpad.rs`:205; `tests.rs`:314 |
| `not-requested` | 1 | 2 | 2 | `release_check.rs`:697; `tests.rs`:461 |
| `nothing to delete` | 1 | 2 | 2 | `reverse_sync.rs`:1368; `tests.rs`:1131 |
| `pin-stale` | 1 | 2 | 2 | `setup_ci.rs`:160; `tests.rs`:465 |
| `plugin-release-check.json` | 1 | 2 | 2 | `release.rs`:93; `tests.rs`:1293 |
| `pre_compact` | 1 | 2 | 2 | `mod.rs`:325; `tests.rs`:276 |
| `scratchpad refused` | 1 | 2 | 2 | `tests.rs`:669; `scratchpad.rs`:291 |
| `session_end` | 1 | 2 | 2 | `mod.rs`:327; `tests.rs`:278 |
| `session_start` | 1 | 2 | 2 | `mod.rs`:320; `tests.rs`:274 |
| `ss-magic sync` | 1 | 2 | 2 | `migrate.rs`:182; `tests.rs`:98 |
| `ss-magic-plugin-v` | 1 | 2 | 2 | `release.rs`:92; `tests.rs`:169 |
| `ss-magic-plugin-v1.0.0.zip` | 1 | 2 | 2 | `tests.rs`:100; `build-plugin-zip.py`:1142 |
| `subagent_stop` | 1 | 2 | 2 | `mod.rs`:326; `tests.rs`:277 |
| `this-worktree` | 1 | 2 | 2 | `status.rs`:1703; `tests.rs`:832 |
| `totalCostUSD` | 1 | 2 | 2 | `ledger.rs`:734; `tests.rs`:56 |
| `tracked-paths` | 1 | 2 | 2 | `scratchpad.rs`:207; `tests.rs`:255 |
| `transcript-offsets.json` | 1 | 2 | 2 | `tests.rs`:327; `ledger.rs`:86 |
| `unsupported-platform` | 1 | 2 | 2 | `status.rs`:1475; `tests.rs`:570 |
| `x86_64` | 1 | 2 | 2 | `bootstrap.sh`:281; `test-bootstrap.sh`:773 |
| `{PRICE_TABLE_VERSION}.json` | 1 | 2 | 2 | `ledger.rs`:1146; `tests.rs`:824 |
| `{}/` | 1 | 2 | 2 | `tests.rs`:544; `status.rs`:1270 |
| `utf-8` | 1 | 1 | 28 | `build-plugin-zip.py`:207,220,233,247,294,812… |
| `.claude-plugin` | 1 | 1 | 9 | `build-plugin-zip.py`:232,334,826,831,845,966… |
| `Cargo.toml` | 1 | 1 | 6 | `build-plugin-zip.py`:255,256,257,928,932,943 |
| `$bin_path` | 1 | 1 | 5 | `bootstrap.sh`:187,188,253,467,493 |
| `$staged_bin` | 1 | 1 | 5 | `bootstrap.sh`:443,444,446,452,467 |
| `$euid` | 1 | 1 | 4 | `tmproot.sh`:101,120,127,131 |
| `$newest` | 1 | 1 | 4 | `mark-latest.sh`:111,118,123,129 |
| `$path` | 1 | 1 | 4 | `tmproot.sh`:93,94,95,96 |
| `$state_file` | 1 | 1 | 4 | `bootstrap.sh`:70,70,197,472 |
| `$tmp` | 1 | 1 | 4 | `bootstrap.sh`:138,139,139,141 |
| `({PLUGIN_LINE})` | 1 | 1 | 4 | `build-plugin-zip.py`:1150,1155,1158,1241 |
| `, tui::style::err(format!(` | 1 | 1 | 4 | `main.rs`:225,355,364,474 |
| `dist-workspace.toml` | 1 | 1 | 4 | `build-plugin-zip.py`:292,296,954,1243 |
| `plugin.json` | 1 | 1 | 4 | `build-plugin-zip.py`:334,826,831,967 |
| `self_update` | 1 | 1 | 4 | `build-plugin-zip.py`:634,1208,1211,1213 |
| `store_true` | 1 | 1 | 4 | `build-plugin-zip.py`:1276,1280,1285,1299 |
| `{BYPASS_USAGE}` | 1 | 1 | 4 | `bypass.rs`:181,189,198,209 |
| `$archive_path` | 1 | 1 | 3 | `bootstrap.sh`:418,429,436 |
| `$mode` | 1 | 1 | 3 | `tmproot.sh`:100,126,130 |
| `$obj` | 1 | 1 | 3 | `mark-latest.sh`:87,92,95 |
| `$root` | 1 | 1 | 3 | `tmproot.sh`:130,131,133 |
| `$signature` | 1 | 1 | 3 | `bootstrap.sh`:286,290,292 |
| `.DS_Store` | 1 | 1 | 3 | `build-plugin-zip.py`:74,1083,1084 |
| `Cargo.lock` | 1 | 1 | 3 | `build-plugin-zip.py`:324,330,948 |
| `distinct release lines` | 1 | 1 | 3 | `build-plugin-zip.py`:549,1136,1147 |
| `harness registration` | 1 | 1 | 3 | `status.rs`:2021,2024,2035 |
| `marketplace.json` | 1 | 1 | 3 | `build-plugin-zip.py`:232,845,999 |
| `mkdir -p {}` | 1 | 1 | 3 | `apply.rs`:333,348,352 |
| `refs.` | 1 | 1 | 3 | `verbs.rs`:937,1011,1085 |
| `this plugin pins` | 1 | 1 | 3 | `release_check.rs`:599,600,603 |
| `--help` | 0 | 9 | 14 | `tests.rs`:260; `tests.rs`:1135,1137; `tests.rs`:99; `tests.rs`:643,666; `tests.rs`:432; `tests.rs`:564; `tests.rs`:556; `tests.rs`:170,462; `tests.rs`:39,60,62 |
| `target/\n.superset/.magic/\n` | 0 | 9 | 10 | `tests.rs`:156; `tests.rs`:32; `tests.rs`:34; `tests.rs`:34; `tests.rs`:32; `tests.rs`:30,60; `tests.rs`:36; `tests.rs`:95; `tests.rs`:24 |
| `{:?}` | 0 | 8 | 47 | `tests.rs`:47,63; `tests.rs`:139,149,161,292,335; `tests.rs`:341,920; `tests.rs`:673; `tests.rs`:582,617,830,898,981,1033…; `tests.rs`:235,389,421,498,532,584…; `tests.rs`:48,68,122,253,376,403…; `tests.rs`:436,1171,1234,1332,1366 |
| `apps/api/.env` | 0 | 8 | 19 | `tests.rs`:274,286; `tests.rs`:192,197,231,237; `tests.rs`:19; `tests.rs`:25; `tests.rs`:224,231; `tests.rs`:102,106; `sync.rs`:61,68,85,91; `tests.rs`:99,110,1208 |
| `gitignore` | 0 | 8 | 9 | `tests.rs`:158; `tests.rs`:36; `tests.rs`:38; `tests.rs`:38; `tests.rs`:36; `tests.rs`:32,62; `tests.rs`:40; `tests.rs`:16 |
| `apps/api/.dev.vars` | 0 | 7 | 65 | `tests.rs`:122,153,306,316,320; `tests.rs`:37,52,100; `tests.rs`:68,71,166,171; `tests.rs`:103,109; `tests.rs`:72,76,109,149,153,159…; `reverse_sync_flow.rs`:23,25,30,37,49,62; `sync.rs`:33,39,86,92 |
| `.superset/config.json` | 0 | 6 | 18 | `tests.rs`:123,136,178,290; `tests.rs`:296,309; `tests.rs`:42; `tests.rs`:181,192,607,620,645,664…; `tests.rs`:2061,2064; `tests.rs`:129 |
| `check-dns` | 0 | 5 | 68 | `tests.rs`:48; `tests.rs`:217,240,252,304,321,360…; `tests.rs`:37; `tests.rs`:36,214; `tests.rs`:232,293,295,301,310,317… |
| `2026-08-29T07:00:00Z` | 0 | 5 | 49 | `tests.rs`:61,142,143,163,168,183; `tests.rs`:61,83,97,112,134,144…; `tests.rs`:33,59,161,217,240,255…; `tests.rs`:33,37,77,112,117,141…; `tests.rs`:17,28,32 |
| `ss-magic-plugin-v1.0.0` | 0 | 5 | 22 | `tests.rs`:96,125,129,209,319,719…; `tests.rs`:929,935; `tests.rs`:83,103,188,351,372,387…; `tests.rs`:1353,1374; `test-mark-latest.sh`:103,184 |
| `/repo` | 0 | 5 | 17 | `tests.rs`:975,1017; `tests.rs`:36,54,66; `tests.rs`:20,35,95,184; `tests.rs`:286,331,383; `tests.rs`:210,213,225,232,1103 |
| `.superset/backups/` | 0 | 5 | 11 | `tests.rs`:218,401; `tests.rs`:168,196,631,667; `tests.rs`:2019; `tests.rs`:863,877; `tests.rs`:563,923 |
| `Ship the thing` | 0 | 5 | 10 | `tests.rs`:40; `tests.rs`:344,424,441,488; `tests.rs`:31; `tests.rs`:26; `tests.rs`:325,342,458 |
| `.superset/magic.sh` | 0 | 5 | 9 | `tests.rs`:872,914,953; `tests.rs`:297,310; `tests.rs`:43; `tests.rs`:608,621; `tests.rs`:257 |
| `ss-magic-plugin-v1.1.0` | 0 | 4 | 47 | `tests.rs`:682,701,719,723,730,734…; `tests.rs`:735; `tests.rs`:82,87,87,88,95,120…; `tests.rs`:1323,1327,1331 |
| `sess-1` | 0 | 4 | 20 | `tests.rs`:159; `tests.rs`:54; `tests.rs`:173,193,229,253,413,422…; `tests.rs`:62 |
| `{err}` | 0 | 4 | 12 | `tests.rs`:331,432,475,480; `tests.rs`:321,324,351,361; `tests.rs`:156; `tests.rs`:357,358,365 |
| `{note}` | 0 | 4 | 12 | `tests.rs`:606,607; `tests.rs`:145,161,162,195,262; `tests.rs`:54,257; `tests.rs`:325,345,622 |
| `.superset/.magic/state.json` | 0 | 4 | 8 | `tests.rs`:299,333,358; `tests.rs`:18; `tests.rs`:609,647; `tests.rs`:2012,2065 |
| `SessionEnd` | 0 | 4 | 8 | `tests.rs`:91,105; `tests.rs`:71; `tests.rs`:495,557,807; `test-bootstrap.sh`:625,913 |
| `s-1` | 0 | 4 | 8 | `tests.rs`:776; `tests.rs`:19,34,154; `tests.rs`:102; `tests.rs`:113,198,695 |
| `{not json` | 0 | 4 | 8 | `tests.rs`:136,234,476,676,756; `tests.rs`:140; `sync.rs`:144; `tests.rs`:129 |
| `UNTRUSTED DATA, not instructions` | 0 | 4 | 7 | `tests.rs`:224,305,327,493; `tests.rs`:122; `tests.rs`:904; `tests.rs`:392 |
| `2026-08-30T12:00:00Z` | 0 | 4 | 6 | `tests.rs`:214,347,410; `tests.rs`:1174; `tests.rs`:384; `tests.rs`:778 |
| `apps/*/.env` | 0 | 4 | 6 | `tests.rs`:235; `tests.rs`:6,25; `tests.rs`:38; `tests.rs`:772,784 |
| `/etc/passwd` | 0 | 4 | 5 | `tests.rs`:111,151; `tests.rs`:34; `tests.rs`:208; `tests.rs`:5 |
| `2026-08-ship-the-thing` | 0 | 4 | 5 | `tests.rs`:41; `tests.rs`:345,442; `tests.rs`:32; `tests.rs`:27 |
| `{ not json` | 0 | 4 | 5 | `tests.rs`:478; `tests.rs`:577,601; `tests.rs`:709; `tests.rs`:608 |
| `../oops` | 0 | 4 | 4 | `tests.rs`:124; `tests.rs`:38; `tests.rs`:308; `tests.rs`:11 |
| `.scratchpad/notes.md` | 0 | 4 | 4 | `tests.rs`:357; `tests.rs`:20; `tests.rs`:646; `tests.rs`:2011 |
| `apps/api/config` | 0 | 4 | 4 | `tests.rs`:255; `tests.rs`:10; `tests.rs`:135; `tests.rs`:33 |
| `item {id}` | 0 | 4 | 4 | `tests.rs`:31; `tests.rs`:21; `tests.rs`:12; `tests.rs`:16 |
| `big.rs` | 0 | 3 | 37 | `tests.rs`:26,166,181,205,231; `tests.rs`:54,70,115,137,179,218…; `tests.rs`:326,587,600,606,614,621… |
| `v0.11.10` | 0 | 3 | 16 | `tests.rs`:72,160,178,222,252,257…; `tests.rs`:49; `test-mark-latest.sh`:59,90,128,140,150,171 |
| `v1.0.0` | 0 | 3 | 16 | `tests.rs`:95,112,113,193,195,272…; `tests.rs`:120; `test-mark-latest.sh`:109 |
| `SECRET=1\n` | 0 | 3 | 14 | `tests.rs`:37; `tests.rs`:72,109,149,389,416,963…; `tests.rs`:103,616 |
| `ss-magic-plugin-v1.2.0` | 0 | 3 | 14 | `tests.rs`:68,183,246,743,753; `tests.rs`:287; `test-mark-latest.sh`:59,70,126,138,144,150… |
| `a\nb\nc\n` | 0 | 3 | 12 | `tests.rs`:8,34,44,45; `tests.rs`:94,130,1185; `tests.rs`:47,76,89,124,249 |
| `notification` | 0 | 3 | 10 | `tests.rs`:172; `tests.rs`:224,229,234,264,763; `tests.rs`:95,97,266,451 |
| `teardown` | 0 | 3 | 9 | `tests.rs`:120,286,378; `tests.rs`:844,852; `tests.rs`:220,360,630,656 |
| `#!/bin/sh\n` | 0 | 3 | 8 | `tests.rs`:297; `tests.rs`:533,590,639,665,1014,1087; `tests.rs`:608 |
| `.dev.vars` | 0 | 3 | 8 | `tests.rs`:455,458,558,559; `tests.rs`:165,169; `tests.rs`:495,497 |
| `RECOVERED=1\n` | 0 | 3 | 7 | `tests.rs`:303; `tests.rs`:160,185,613,651; `tests.rs`:2016,2069 |
| `a\nB\nc\n` | 0 | 3 | 7 | `tests.rs`:9,44; `tests.rs`:95,131,1186; `tests.rs`:47,124 |
| `session_id` | 0 | 3 | 7 | `tests.rs`:19,154,189; `tests.rs`:113,198,210; `tests.rs`:277 |
| `**/.dev.vars\n` | 0 | 3 | 6 | `tests.rs`:120,304; `tests.rs`:99; `tests.rs`:145,198,255 |
| `SubagentStop` | 0 | 3 | 6 | `tests.rs`:67,90,164; `tests.rs`:65; `test-bootstrap.sh`:624,912 |
| `{\"token\":\"s3cret\"}\n` | 0 | 3 | 6 | `tests.rs`:299,333; `tests.rs`:609,647; `tests.rs`:2012,2065 |
| `.superset/.magicked/keep.txt` | 0 | 3 | 5 | `tests.rs`:387,394; `tests.rs`:56; `tests.rs`:691,700 |
| `.superset/backups/20260101-000000/main/.env` | 0 | 3 | 5 | `tests.rs`:361; `tests.rs`:16; `tests.rs`:184,612,650 |
| `.superset/backupsfoo/keep.txt` | 0 | 3 | 5 | `tests.rs`:388,395; `tests.rs`:57; `tests.rs`:692,701 |
| `checkout` | 0 | 3 | 5 | `tests.rs`:1402,2102; `tests.rs`:58; `tests.rs`:94,112 |
| `future_key` | 0 | 3 | 5 | `tests.rs`:580,602; `tests.rs`:853; `tests.rs`:691,705 |
| `.superset/backups/20260101-000000/worktree/.env` | 0 | 3 | 4 | `tests.rs`:302; `tests.rs`:159; `tests.rs`:2015,2068 |
| `PreCompact` | 0 | 3 | 4 | `tests.rs`:89; `tests.rs`:57; `test-bootstrap.sh`:623,912 |
| `packages/**/fixtures` | 0 | 3 | 4 | `tests.rs`:193,200; `tests.rs`:27; `tests.rs`:40 |
| `--nope` | 0 | 3 | 3 | `tests.rs`:257; `tests.rs`:423; `tests.rs`:558 |
| `compact-window --recommend` | 0 | 3 | 3 | `tests.rs`:816; `tests.rs`:569; `tests.rs`:1181 |
| `do the thing` | 0 | 3 | 3 | `tests.rs`:11; `tests.rs`:23; `tests.rs`:14 |
| `does-not-exist` | 0 | 3 | 3 | `tests.rs`:155; `tests.rs`:1130; `tests.rs`:444 |
| `scratch\n` | 0 | 3 | 3 | `tests.rs`:357; `tests.rs`:646; `tests.rs`:2011 |
| `ss-magic ` | 0 | 3 | 3 | `tests.rs`:814; `tests.rs`:544; `tests.rs`:238 |
| `the thing is done` | 0 | 3 | 3 | `tests.rs`:12; `tests.rs`:24; `tests.rs`:15 |
| `{bad json` | 0 | 3 | 3 | `tests.rs`:490; `tests.rs`:283; `sync.rs`:126 |
| `/tmp/p` | 0 | 2 | 36 | `tests.rs`:253,310; `tests.rs`:184,210,318,322,326,352… |
| `./.superset/setup.sh` | 0 | 2 | 19 | `tests.rs`:119,126,145,146,154,160…; `tests.rs`:84,106,144,163,182,192… |
| `0.10.0` | 0 | 2 | 18 | `tests.rs`:62,85,102,258,509,518…; `test-bootstrap.sh`:348,425,472,507 |
| `REPORT.md` | 0 | 2 | 17 | `tests.rs`:66,67,83,196,256,273…; `tests.rs`:226,227,239,240,274,291… |
| `{rendered}` | 0 | 2 | 17 | `tests.rs`:235,236,284,286,287,304…; `tests.rs`:356 |
| `MAIN=1\n` | 0 | 2 | 16 | `tests.rs`:288,700,728,1428,1465,1467…; `reverse_sync_flow.rs`:23,49,63 |
| `msg: {msg}` | 0 | 2 | 14 | `tests.rs`:139,140,237,238,406,480…; `tests.rs`:134 |
| `secret.env` | 0 | 2 | 14 | `tests.rs`:1235,1570,1583,1762,1765,1771…; `tests.rs`:399,1007 |
| `sections` | 0 | 2 | 14 | `tests.rs`:355,392,425,431; `tests.rs`:362,385,438,439,440,469… |
| `{found:?}` | 0 | 2 | 14 | `tests.rs`:245,256,266,277,296,309…; `tests.rs`:93,98,104,227,228 |
| `diff.env` | 0 | 2 | 13 | `tests.rs`:287,288,299,1433,1434,1449…; `tests.rs`:696,724,749,1366 |
| `docs/REPORT.md` | 0 | 2 | 11 | `tests.rs`:50,54,296,297,378,383…; `tests.rs`:198,202,253,254 |
| `docs/actions/2026-08-x.checklist.json` | 0 | 2 | 10 | `tests.rs`:979,999; `tests.rs`:786,1096,1216,1239,1326,2193… |
| `{message}` | 0 | 2 | 10 | `tests.rs`:44,45; `tests.rs`:568,569,570,571,813,814… |
| `./drop.sh` | 0 | 2 | 9 | `tests.rs`:120,127,155,173,378,395; `tests.rs`:220,253,393 |
| `v2.0.0` | 0 | 2 | 9 | `tests.rs`:269,372,390,521,530; `tests.rs`:137,141,157,165 |
| `$out` | 0 | 2 | 8 | `test-bootstrap.sh`:120; `test-mark-latest.sh`:120,131,132,133,134,146… |
| `a\nY\nc\n` | 0 | 2 | 8 | `tests.rs`:147,150,554,577,634,765…; `tests.rs`:241 |
| `docs/**` | 0 | 2 | 8 | `tests.rs`:255,259,623,625,631,631; `tests.rs`:468,485 |
| `{outcome:?}` | 0 | 2 | 8 | `tests.rs`:148,220,242,262; `tests.rs`:345,460,486,670 |
| `apps/api` | 0 | 2 | 7 | `tests.rs`:136,143,267,416,418; `tests.rs`:158,161 |
| `malformed JSON` | 0 | 2 | 7 | `tests.rs`:140,238,481,495,681,761; `tests.rs`:134 |
| `new.env` | 0 | 2 | 7 | `tests.rs`:282,291; `tests.rs`:314,342,382,612,925 |
| `secrets/api.key` | 0 | 2 | 7 | `tests.rs`:78,81,89; `tests.rs`:231,232,235,241 |
| `2026-08-29T08:30:00+02:00` | 0 | 2 | 6 | `tests.rs`:60,208,213; `tests.rs`:145,157,183 |
| `2026-08-29T09:00:00+02:00` | 0 | 2 | 6 | `tests.rs`:60; `tests.rs`:34,45,62,204,209 |
| `2026-08-29T09:00:00Z` | 0 | 2 | 6 | `tests.rs`:59,124,207,212; `tests.rs`:146,184 |
| `CARGO_MANIFEST_DIR` | 0 | 2 | 6 | `tests.rs`:294; `tests.rs`:975,987,1033,1064,1086 |
| `a.txt` | 0 | 2 | 6 | `tests.rs`:344,350,716,844,851; `tests.rs`:83 |
| `permission_mode` | 0 | 2 | 6 | `tests.rs`:353; `tests.rs`:1114,1121,1135,1161,1165 |
| `pnpm dev` | 0 | 2 | 6 | `tests.rs`:121,128,156,174; `tests.rs`:220,254 |
| `secret.txt` | 0 | 2 | 6 | `tests.rs`:74,170,186,193,253; `tests.rs`:310 |
| `v0.11.3` | 0 | 2 | 6 | `tests.rs`:69; `test-mark-latest.sh`:59,75,162,162,201 |
| `wrong payload variant` | 0 | 2 | 6 | `tests.rs`:37,54,75,109,121; `tests.rs`:702 |
| `#!/bin/bash\n` | 0 | 2 | 5 | `tests.rs`:353; `tests.rs`:355,468,627,653 |
| `.git/config` | 0 | 2 | 5 | `tests.rs`:387,707,788,804; `tests.rs`:356 |
| `.superset/.gitignore` | 0 | 2 | 5 | `tests.rs`:384,395; `tests.rs`:79,80,86 |
| `SS_MAGIC_TEST_CWD` | 0 | 2 | 5 | `tests.rs`:896,909; `tests.rs`:865,895,917 |
| `apps/*/config` | 0 | 2 | 5 | `tests.rs`:191,199,214,224; `tests.rs`:56 |
| `check dns` | 0 | 2 | 5 | `tests.rs`:253,305,322,361; `tests.rs`:206 |
| `keep.txt` | 0 | 2 | 5 | `tests.rs`:355,367; `tests.rs`:496,500,505 |
| `ss-magic-v0.13.0` | 0 | 2 | 5 | `tests.rs`:71; `test-mark-latest.sh`:59,85,134,171 |
| `the findings` | 0 | 2 | 5 | `tests.rs`:50,55; `tests.rs`:198,205,253 |
| `updatedInput` | 0 | 2 | 5 | `tests.rs`:317; `tests.rs`:334,335,2789,2790 |
| `v0.10.0` | 0 | 2 | 5 | `tests.rs`:200,201,202,203; `test-mark-latest.sh`:109 |
| `$failed` | 0 | 2 | 4 | `test-bootstrap.sh`:947,948; `test-mark-latest.sh`:223,224 |
| `../x` | 0 | 2 | 4 | `tests.rs`:15; `tests.rs`:51,195,247 |
| `.superset/magic.local.json\n` | 0 | 2 | 4 | `tests.rs`:16,48,92; `tests.rs`:172 |
| `/target\n` | 0 | 2 | 4 | `tests.rs`:424,450,593; `tests.rs`:918 |
| `2026-08-ship-it` | 0 | 2 | 4 | `tests.rs`:76,140,179; `tests.rs`:24 |
| `PATH` | 0 | 2 | 4 | `tests.rs`:921,972; `tests.rs`:864,894 |
| `README.txt` | 0 | 2 | 4 | `tests.rs`:404,408; `tests.rs`:1299,1323 |
| `a/b/c/.env` | 0 | 2 | 4 | `tests.rs`:117,123; `sync.rs`:48,53 |
| `apps/*/.dev.vars` | 0 | 2 | 4 | `tests.rs`:96,99; `tests.rs`:215,243 |
| `apps/api/config/a.toml` | 0 | 2 | 4 | `tests.rs`:253,257; `tests.rs`:136,142 |
| `apps/api/config/sub/b.toml` | 0 | 2 | 4 | `tests.rs`:254,261; `tests.rs`:137,144 |
| `apps/web/.dev.vars` | 0 | 2 | 4 | `tests.rs`:104,110; `sync.rs`:34,40 |
| `check the DNS record` | 0 | 2 | 4 | `tests.rs`:48,164,284; `tests.rs`:68 |
| `feature-x` | 0 | 2 | 4 | `tests.rs`:37,155; `tests.rs`:151,180 |
| `guidance` | 0 | 2 | 4 | `tests.rs`:211,217; `tests.rs`:706,720 |
| `new.txt` | 0 | 2 | 4 | `tests.rs`:34,59; `tests.rs`:134,147 |
| `planted by the repository\n` | 0 | 2 | 4 | `tests.rs`:353,370; `tests.rs`:242,251 |
| `recommendation` | 0 | 2 | 4 | `tests.rs`:745,745; `tests.rs`:1257,1263 |
| `sdk-ts` | 0 | 2 | 4 | `tests.rs`:717,866; `tests.rs`:1136,1166 |
| `v0.9.0-rc1` | 0 | 2 | 4 | `tests.rs`:73; `test-mark-latest.sh`:59,95,133 |
| `{\"files\":[]}\n` | 0 | 2 | 4 | `tests.rs`:298,332; `tests.rs`:95,175 |
| `{body:?}` | 0 | 2 | 4 | `tests.rs`:999,1007,1008; `tests.rs`:874 |
| `{name}` | 0 | 2 | 4 | `tests.rs`:20,95,96; `tests.rs`:113 |
| `.git/hooks` | 0 | 2 | 3 | `tests.rs`:431; `tests.rs`:832,886 |
| `.superset/\n` | 0 | 2 | 3 | `tests.rs`:230; `tests.rs`:53,58 |
| `.venv/lib/.env` | 0 | 2 | 3 | `tests.rs`:226; `sync.rs`:63,70 |
| `/etc/hosts` | 0 | 2 | 3 | `tests.rs`:272; `tests.rs`:323,404 |
| `/plugin` | 0 | 2 | 3 | `tests.rs`:815; `tests.rs`:233,479 |
| `/proc/self/cwd` | 0 | 2 | 3 | `tests.rs`:1845; `tests.rs`:68,158 |
| `/t/s-1.jsonl` | 0 | 2 | 3 | `tests.rs`:19,96; `tests.rs`:113 |
| `1.2.3` | 0 | 2 | 3 | `tests.rs`:270,271; `tests.rs`:701 |
| `20260716-153000` | 0 | 2 | 3 | `tests.rs`:102,106; `tests.rs`:1270 |
| `REAL=1\n` | 0 | 2 | 3 | `tests.rs`:155; `tests.rs`:258,2010 |
| `already shown` | 0 | 2 | 3 | `tests.rs`:581,829; `tests.rs`:222 |
| `docs/actions/notes.md` | 0 | 2 | 3 | `tests.rs`:983; `tests.rs`:2167,2374 |
| `filler line\n` | 0 | 2 | 3 | `tests.rs`:300; `tests.rs`:928,958 |
| `garbage\n` | 0 | 2 | 3 | `tests.rs`:522,967; `tests.rs`:227 |
| `mod.rs` | 0 | 2 | 3 | `tests.rs`:204,204; `tests.rs`:993 |
| `never edits` | 0 | 2 | 3 | `tests.rs`:607,614; `tests.rs`:570 |
| `node_modules/pkg/.env` | 0 | 2 | 3 | `tests.rs`:225; `sync.rs`:62,69 |
| `not ours\n` | 0 | 2 | 3 | `tests.rs`:404; `tests.rs`:393,405 |
| `read the conclusion instead` | 0 | 2 | 3 | `tests.rs`:245,255; `tests.rs`:140 |
| `s-gone` | 0 | 2 | 3 | `tests.rs`:559; `tests.rs`:430,1261 |
| `src/deep` | 0 | 2 | 3 | `tests.rs`:143,322; `tests.rs`:856 |
| `src/main.rs` | 0 | 2 | 3 | `tests.rs`:585; `tests.rs`:2371,2490 |
| `stop_hook_active` | 0 | 2 | 3 | `tests.rs`:70,164; `tests.rs`:210 |
| `tool_name` | 0 | 2 | 3 | `tests.rs`:48; `tests.rs`:338,579 |
| `v1.1.0` | 0 | 2 | 3 | `tests.rs`:581,587; `tests.rs`:85 |
| `v1.2.3` | 0 | 2 | 3 | `tests.rs`:267,270; `test-bootstrap.sh`:367 |
| `{label}` | 0 | 2 | 3 | `tests.rs`:753,961; `tests.rs`:1122 |
| `#[cfg(test)]` | 0 | 2 | 2 | `tests.rs`:297; `tests.rs`:1089 |
| `$(basename ` | 0 | 2 | 2 | `test-bootstrap.sh`:947; `test-mark-latest.sh`:223 |
| `$REPO_ROOT/scripts/lib/test-harness.sh` | 0 | 2 | 2 | `test-bootstrap.sh`:39; `test-mark-latest.sh`:29 |
| `$passed` | 0 | 2 | 2 | `test-bootstrap.sh`:947; `test-mark-latest.sh`:223 |
| `).join(` | 0 | 2 | 2 | `tests.rs`:210; `tests.rs`:61 |
| `,{extra}` | 0 | 2 | 2 | `tests.rs`:16; `tests.rs`:110 |
| `---` | 0 | 2 | 2 | `tests.rs`:54; `tests.rs`:27 |
| `.scratchpad/` | 0 | 2 | 2 | `tests.rs`:667; `tests.rs`:2019 |
| `.superset/.magic/\n` | 0 | 2 | 2 | `tests.rs`:312; `tests.rs`:53 |
| `.superset/.magic/cache/conclusions/deadbeef.json` | 0 | 2 | 2 | `tests.rs`:310; `tests.rs`:24 |
| `.superset/.magic/checklist-pointer.json` | 0 | 2 | 2 | `tests.rs`:317; `tests.rs`:31 |
| `.superset/.magic/checklist.json` | 0 | 2 | 2 | `tests.rs`:158; `tests.rs`:1099 |
| `.superset/.magic/claims/pending-one-shot.json` | 0 | 2 | 2 | `tests.rs`:314; `tests.rs`:28 |
| `.superset/.magic/conclusions` | 0 | 2 | 2 | `tests.rs`:89; `tests.rs`:165 |
| `.superset/.magic/sessions/2026-08-30-abc123/session.json` | 0 | 2 | 2 | `tests.rs`:306; `tests.rs`:20 |
| `/nonexistent/worktree/for/this/test` | 0 | 2 | 2 | `tests.rs`:559; `tests.rs`:1261 |
| `/proc/4321/cwd` | 0 | 2 | 2 | `tests.rs`:1845; `tests.rs`:68 |
| `/proc/thread-self/cwd` | 0 | 2 | 2 | `tests.rs`:1845; `tests.rs`:68 |
| `1.2` | 0 | 2 | 2 | `tests.rs`:703; `test-bootstrap.sh`:368 |
| `1.2.3.4` | 0 | 2 | 2 | `tests.rs`:704; `test-bootstrap.sh`:369 |
| `2026-08-29T07:00:00` | 0 | 2 | 2 | `tests.rs`:77; `tests.rs`:342 |
| `2026-08-29T08:00:00Z` | 0 | 2 | 2 | `tests.rs`:46; `tests.rs`:158 |
| `DEEP=1\n` | 0 | 2 | 2 | `tests.rs`:117; `sync.rs`:48 |
| `FileChanged` | 0 | 2 | 2 | `tests.rs`:92; `tests.rs`:162 |
| `Generated by ss-magic` | 0 | 2 | 2 | `tests.rs`:144; `tests.rs`:371 |
| `Notification` | 0 | 2 | 2 | `tests.rs`:175; `tests.rs`:225 |
| `ViktorStiskala` | 0 | 2 | 2 | `tests.rs`:50; `tests.rs`:224 |
| `a claim is one-shot` | 0 | 2 | 2 | `tests.rs`:37; `tests.rs`:39 |
| `a/../b` | 0 | 2 | 2 | `tests.rs`:16; `tests.rs`:309 |
| `a\r\nX\r\nc` | 0 | 2 | 2 | `tests.rs`:147; `tests.rs`:240 |
| `a\r\nb\r\n` | 0 | 2 | 2 | `tests.rs`:193; `tests.rs`:221 |
| `apps/api/config.json` | 0 | 2 | 2 | `tests.rs`:101; `tests.rs`:59 |
| `docs/actions/.checklist.json` | 0 | 2 | 2 | `tests.rs`:986; `tests.rs`:1293 |
| `docs/actions/2026-08-ship-the-thing.checklist.json` | 0 | 2 | 2 | `tests.rs`:54; `tests.rs`:27 |
| `git@github.com:ViktorStiskala/upx.cz.git` | 0 | 2 | 2 | `tests.rs`:11; `tests.rs`:534 |
| `heartbeat` | 0 | 2 | 2 | `tests.rs`:796; `tests.rs`:988 |
| `hook <event>` | 0 | 2 | 2 | `tests.rs`:177; `test-bootstrap.sh`:833 |
| `hook_event_name` | 0 | 2 | 2 | `tests.rs`:20; `tests.rs`:113 |
| `https://github.com/` | 0 | 2 | 2 | `tests.rs`:43; `tests.rs`:173 |
| `link.rs` | 0 | 2 | 2 | `tests.rs`:183; `tests.rs`:571 |
| `new session` | 0 | 2 | 2 | `tests.rs`:816; `tests.rs`:234 |
| `no cache directory` | 0 | 2 | 2 | `tests.rs`:514; `tests.rs`:1400 |
| `no transcript path` | 0 | 2 | 2 | `tests.rs`:241; `tests.rs`:485 |
| `no-such-dir` | 0 | 2 | 2 | `tests.rs`:65; `tests.rs`:1411 |
| `not a directory` | 0 | 2 | 2 | `tests.rs`:406; `tests.rs`:783 |
| `not json at all` | 0 | 2 | 2 | `tests.rs`:92; `tests.rs`:166 |
| `notes.txt` | 0 | 2 | 2 | `tests.rs`:180; `tests.rs`:2494 |
| `once per release` | 0 | 2 | 2 | `tests.rs`:826; `tests.rs`:235 |
| `pins 1.0.0` | 0 | 2 | 2 | `tests.rs`:814; `tests.rs`:232 |
| `s-legacy` | 0 | 2 | 2 | `tests.rs`:557; `tests.rs`:1212 |
| `source `{source}`` | 0 | 2 | 2 | `tests.rs`:949; `tests.rs`:177 |
| `ss-magic plugin` | 0 | 2 | 2 | `tests.rs`:583; `tests.rs`:435 |
| `ss-magic-plugin checklist list` | 0 | 2 | 2 | `tests.rs`:1193; `tests.rs`:364 |
| `ss-magic-plugin-v9.0.0` | 0 | 2 | 2 | `tests.rs`:284; `tests.rs`:109 |
| `transcript_path` | 0 | 2 | 2 | `tests.rs`:19; `tests.rs`:113 |
| `yesterday` | 0 | 2 | 2 | `tests.rs`:127; `tests.rs`:793 |
| `{\"claim\":\"one-shot-42\"}` | 0 | 2 | 2 | `tests.rs`:315; `tests.rs`:29 |
| `{\"conclusion\":\"cached result\"}` | 0 | 2 | 2 | `tests.rs`:311; `tests.rs`:25 |
| `{\"seq\":7}` | 0 | 2 | 2 | `tests.rs`:317; `tests.rs`:31 |
| `{\"status\":\"active\"}` | 0 | 2 | 2 | `tests.rs`:307; `tests.rs`:21 |
| `{after}` | 0 | 2 | 2 | `tests.rs`:295; `tests.rs`:64 |
| `{pin}\n` | 0 | 2 | 2 | `tests.rs`:742; `tests.rs`:1311 |
| `{} must survive byte-for-byte` | 0 | 2 | 2 | `tests.rs`:366; `tests.rs`:52 |
| `config.env` | 0 | 1 | 104 | `tests.rs`:339,340,352,355,356,367… |
| `$sb/fakebin.log` | 0 | 1 | 30 | `test-bootstrap.sh`:94,214,225,229,243,247… |
| `$sb/data/bin/ss-magic-plugin` | 0 | 1 | 25 | `test-bootstrap.sh`:282,283,316,325,340,348… |
| `env.sh` | 0 | 1 | 21 | `tests.rs`:230,263,290,314,342,367… |
| `state/tracker.json` | 0 | 1 | 18 | `tests.rs`:1354,1355,1487,1489,1571,1782… |
| `$log` | 0 | 1 | 16 | `test-mark-latest.sh`:115,129,130,152,153,154… |
| `docs/actions/2026-08-demo.checklist.json` | 0 | 1 | 15 | `tests.rs`:2608,2615,2626,2639,2689,2716… |
| `tracked.env` | 0 | 1 | 15 | `tests.rs`:120,121,122,125,129,1564… |
| `/home/alice` | 0 | 1 | 14 | `tests.rs`:38,43,43,104,106,117… |
| `export SECRET=hunter2\n` | 0 | 1 | 14 | `tests.rs`:228,261,288,312,504,531… |
| `$(stderr_lines)` | 0 | 1 | 13 | `test-bootstrap.sh`:303,315,329,345,382,445… |
| `$sb/data` | 0 | 1 | 13 | `test-bootstrap.sh`:88,209,408,456,458,717… |
| `WT_NEW=1\n` | 0 | 1 | 13 | `tests.rs`:340,368,549,611,637,653… |
| `$RC` | 0 | 1 | 11 | `test-bootstrap.sh`:258,270,281,702,721,737… |
| `MAIN_ORIG=1\n` | 0 | 1 | 11 | `tests.rs`:476,533,548,588,610,640… |
| `{event:?}` | 0 | 1 | 11 | `tests.rs`:262,281,299,323,324,325… |
| `$(wc -c <` | 0 | 1 | 10 | `test-bootstrap.sh`:259,260,383,723,739,782… |
| `0123456789abcdef` | 0 | 1 | 10 | `tests.rs`:163,165,166,168,169,324… |
| `$sb/err` | 0 | 1 | 9 | `test-bootstrap.sh`:203,215,264,346,519,781… |
| `$sb/home` | 0 | 1 | 9 | `test-bootstrap.sh`:88,96,206,223,228,241… |
| `0.10.0\n` | 0 | 1 | 9 | `tests.rs`:500,530,534,587,636,1011… |
| `allow: under the gate` | 0 | 1 | 9 | `tests.rs`:244,503,517,536,1278,1298… |
| `same.env` | 0 | 1 | 9 | `tests.rs`:284,285,295,1430,1431,1445… |
| `src/big.rs` | 0 | 1 | 9 | `tests.rs`:118,123,139,143,189,221… |
| `the convention` | 0 | 1 | 9 | `tests.rs`:1660,1847,1940,1991,1998,2004… |
| `$AE1_COMPACT` | 0 | 1 | 8 | `test-mark-latest.sh`:58,126,150,162,171,177… |
| `$sb/plugin/ss-magic-plugin.version` | 0 | 1 | 8 | `test-bootstrap.sh`:90,326,341,441,473,514… |
| `$sb/tmp` | 0 | 1 | 8 | `test-bootstrap.sh`:88,207,223,228,241,246… |
| `CONCLUSION-BODY` | 0 | 1 | 8 | `tests.rs`:866,1000,1002,1007,1133,1143… |
| `config.local.json` | 0 | 1 | 8 | `tests.rs`:90,109,126,549,629,760… |
| `expected Skipped, got {other:?}` | 0 | 1 | 8 | `tests.rs`:583,680,724,776,833,1039… |
| `the pointer` | 0 | 1 | 8 | `tests.rs`:1661,1848,1941,1993,2008,2136… |
| `$rc` | 0 | 1 | 7 | `test-mark-latest.sh`:127,139,151,163,172,178… |
| `$sb/curl.log` | 0 | 1 | 7 | `test-bootstrap.sh`:91,211,266,292,295,298… |
| `WT_ORIG=1\n` | 0 | 1 | 7 | `tests.rs`:477,529,802,838,916,951… |
| `a\nX\nc\n` | 0 | 1 | 7 | `tests.rs`:149,553,576,633,644,764… |
| `docs/actions/2026-08-new.checklist.json` | 0 | 1 | 7 | `tests.rs`:1342,1448,1660,1847,1940,2129… |
| `payload\n` | 0 | 1 | 7 | `tests.rs`:115,137,179,218,245,263… |
| `tmp-` | 0 | 1 | 7 | `tests.rs`:52,70,90,91,117,134… |
| `$(sha256_of ` | 0 | 1 | 6 | `test-bootstrap.sh`:184,305,330,347,446,520 |
| `$err` | 0 | 1 | 6 | `test-mark-latest.sh`:120,156,165,179,180,187 |
| `20260716-100000` | 0 | 1 | 6 | `tests.rs`:1294,1317,1358,1397,1407,1409 |
| `A.md` | 0 | 1 | 6 | `tests.rs`:111,113,123,126,225,248 |
| `MAIN_OLD=1\n` | 0 | 1 | 6 | `tests.rs`:339,377,1810,1909,1933,1947 |
| `SAME=1\n` | 0 | 1 | 6 | `tests.rs`:284,285,1430,1431,1468,1469 |
| `apps/api/.gitignore` | 0 | 1 | 6 | `tests.rs`:137,269,280,417,419,425 |
| `changed since review` | 0 | 1 | 6 | `tests.rs`:581,678,774,831,1037,1091 |
| `elsewhere/magic.json` | 0 | 1 | 6 | `tests.rs`:979,982,989,1083,1086,1095 |
| `good.env` | 0 | 1 | 6 | `tests.rs`:855,856,869,883,884,896 |
| `got {entries:?}` | 0 | 1 | 6 | `tests.rs`:109,110,142,145,231,433 |
| `got: {reason}` | 0 | 1 | 6 | `tests.rs`:581,678,774,831,1037,1091 |
| `s-cum` | 0 | 1 | 6 | `tests.rs`:655,655,656,659,660,672 |
| `ship-it` | 0 | 1 | 6 | `tests.rs`:46,153,235,271,1105,1107 |
| `{row:?}` | 0 | 1 | 6 | `tests.rs`:199,228,364,365,367,387 |
| `$(ls -d ` | 0 | 1 | 5 | `test-bootstrap.sh`:318,350,426,448,522 |
| `$WF` | 0 | 1 | 5 | `test-mark-latest.sh`:210,212,213,214,215 |
| `$sb/data/.ss-magic-installed` | 0 | 1 | 5 | `test-bootstrap.sh`:317,331,349,447,521 |
| `/data/.ss-magic-stage.* 2>/dev/null)` | 0 | 1 | 5 | `test-bootstrap.sh`:318,350,426,448,522 |
| `/usr/bin:/bin` | 0 | 1 | 5 | `test-bootstrap.sh`:223,228,241,246,931 |
| `/w/.envrc` | 0 | 1 | 5 | `tests.rs`:854,857,880,893,897 |
| `1752624000` | 0 | 1 | 5 | `tests.rs`:1271,1292,1313,1320,1354 |
| `APPEARED=1\n` | 0 | 1 | 5 | `tests.rs`:655,685,1174,1210,1599 |
| `SKILL.md` | 0 | 1 | 5 | `tests.rs`:858,862,867,889,897 |
| `a reason` | 0 | 1 | 5 | `tests.rs`:344,472,484,621,954 |
| `cutover-planned` | 0 | 1 | 5 | `tests.rs`:321,432,656,660,945 |
| `git commit -m 'wip'` | 0 | 1 | 5 | `tests.rs`:2610,2708,2719,2782,2823 |
| `https://example.com/pr/1` | 0 | 1 | 5 | `tests.rs`:312,427,863,868,869 |
| `no claude here` | 0 | 1 | 5 | `tests.rs`:154,183,465,617,981 |
| `sess-2` | 0 | 1 | 5 | `tests.rs`:824,867,914,937,1039 |
| `src/hook/mod.rs` | 0 | 1 | 5 | `tests.rs`:1033,1035,1064,1066,1086 |
| `sync_core must succeed` | 0 | 1 | 5 | `sync.rs`:19,185,219,243,266 |
| `v1.5.0` | 0 | 1 | 5 | `tests.rs`:420,425,430,611,620 |
| `$(wc -l <` | 0 | 1 | 4 | `test-bootstrap.sh`:722,738,752,760 |
| `$SANDBOX` | 0 | 1 | 4 | `test-mark-latest.sh`:32,117,192,201 |
| `$sb/serr` | 0 | 1 | 4 | `test-bootstrap.sh`:239,244,248,933 |
| `$sb/sout` | 0 | 1 | 4 | `test-bootstrap.sh`:239,244,248,933 |
| `**/*.bz2` | 0 | 1 | 4 | `tests.rs`:452,474,550,583 |
| `.superset/.magic/sessions/2026-08-30-abc123` | 0 | 1 | 4 | `tests.rs`:287,332,340,387 |
| `; else fail ` | 0 | 1 | 4 | `test-harness.sh`:21,27,33,39 |
| `B.md` | 0 | 1 | 4 | `tests.rs`:122,127,226,248 |
| `Explore agent` | 0 | 1 | 4 | `tests.rs`:835,991,1008,1144 |
| `JSON` | 0 | 1 | 4 | `test-mark-latest.sh`:58,65,102,108 |
| `\"fresh\"` | 0 | 1 | 4 | `tests.rs`:561,574,720,731 |
| `] == json!(` | 0 | 1 | 4 | `tests.rs`:387,473,481,490 |
| `allow:` | 0 | 1 | 4 | `tests.rs`:2066,2381,2458,2525 |
| `appended` | 0 | 1 | 4 | `tests.rs`:326,377,420,704 |
| `assets/*.png` | 0 | 1 | 4 | `tests.rs`:255,259,623,626 |
| `bundle/real.txt` | 0 | 1 | 4 | `tests.rs`:313,319,346,351 |
| `config.main.json` | 0 | 1 | 4 | `tests.rs`:1289,1310,1338,1350 |
| `does not report` | 0 | 1 | 4 | `tests.rs`:242,274,396,436 |
| `enablement` | 0 | 1 | 4 | `tests.rs`:207,211,214,216 |
| `live.md` | 0 | 1 | 4 | `tests.rs`:155,157,167,169 |
| `loadedRC` | 0 | 1 | 4 | `tests.rs`:74,788,854,859 |
| `l{i}\n` | 0 | 1 | 4 | `tests.rs`:102,104,148,150 |
| `mainonly.env` | 0 | 1 | 4 | `tests.rs`:1428,1441,1465,1482 |
| `otherKey` | 0 | 1 | 4 | `tests.rs`:193,291,314,586 |
| `permissions` | 0 | 1 | 4 | `tests.rs`:144,173,181,774 |
| `picked Thursday` | 0 | 1 | 4 | `tests.rs`:321,432,656,945 |
| `release edit` | 0 | 1 | 4 | `test-mark-latest.sh`:46,130,157,186 |
| `s-also` | 0 | 1 | 4 | `tests.rs`:1255,1271,1277,1282 |
| `s-nopeak` | 0 | 1 | 4 | `tests.rs`:1201,1201,1202,1262 |
| `s-peak-grow` | 0 | 1 | 4 | `tests.rs`:1175,1178,1182,1190 |
| `secret.txt\n` | 0 | 1 | 4 | `tests.rs`:77,166,191,258 |
| `session-a` | 0 | 1 | 4 | `tests.rs`:96,119,156,167 |
| `ss-magic@ss-magic` | 0 | 1 | 4 | `tests.rs`:98,258,288,1110 |
| `target.txt` | 0 | 1 | 4 | `tests.rs`:24,68,115,156 |
| `tracked.txt` | 0 | 1 | 4 | `tests.rs`:134,135,139,146 |
| `unranked` | 0 | 1 | 4 | `tests.rs`:82,115,127,133 |
| `v9.9.9` | 0 | 1 | 4 | `tests.rs`:324,341,359,466 |
| `wtonly.env` | 0 | 1 | 4 | `tests.rs`:1426,1437,1464,1477 |
| `{body}` | 0 | 1 | 4 | `tests.rs`:341,342,562,564 |
| `{detail:?}` | 0 | 1 | 4 | `tests.rs`:841,843,935,936 |
| `{prefix}/{rel}` | 0 | 1 | 4 | `tests.rs`:1850,1943,2336,2377 |
| `{reason}` | 0 | 1 | 4 | `tests.rs`:202,203,231,244 |
| `$SANDBOX/bin:/usr/bin:/bin` | 0 | 1 | 3 | `test-mark-latest.sh`:117,192,201 |
| `$SANDBOX/err` | 0 | 1 | 3 | `test-mark-latest.sh`:116,194,203 |
| `$SANDBOX/gh.log` | 0 | 1 | 3 | `test-mark-latest.sh`:115,191,200 |
| `$SANDBOX/out` | 0 | 1 | 3 | `test-mark-latest.sh`:116,194,203 |
| `$SCRIPT` | 0 | 1 | 3 | `test-mark-latest.sh`:120,194,203 |
| `$sb/out` | 0 | 1 | 3 | `test-bootstrap.sh`:203,215,265 |
| `$sb/out.$i` | 0 | 1 | 3 | `test-bootstrap.sh`:406,411,421 |
| `$sb/plugin` | 0 | 1 | 3 | `test-bootstrap.sh`:89,208,408 |
| `$sb/release` | 0 | 1 | 3 | `test-bootstrap.sh`:88,210,409 |
| `$sb/werr` | 0 | 1 | 3 | `test-bootstrap.sh`:221,226,230 |
| `$sb/wout` | 0 | 1 | 3 | `test-bootstrap.sh`:221,226,230 |
| `$variant` | 0 | 1 | 3 | `test-bootstrap.sh`:161,163,179 |
| `${CLAUDE_PLUGIN_ROOT}/` | 0 | 1 | 3 | `test-bootstrap.sh`:637,663,680 |
| `, &root, ` | 0 | 1 | 3 | `tests.rs`:322,453,739 |
| `--verbose` | 0 | 1 | 3 | `tests.rs`:70,77,216 |
| `.github` | 0 | 1 | 3 | `tests.rs`:417,524,535 |
| `/repo/A.md` | 0 | 1 | 3 | `tests.rs`:111,123,225 |
| `/repo/REPORT.md` | 0 | 1 | 3 | `tests.rs`:64,80,196 |
| `/t.jsonl` | 0 | 1 | 3 | `tests.rs`:557,585,595 |
| `/tmp/root-a` | 0 | 1 | 3 | `tests.rs`:765,766,772 |
| `/tmp/wt` | 0 | 1 | 3 | `tests.rs`:789,790,796 |
| `/w/.env` | 0 | 1 | 3 | `tests.rs`:880,901,904 |
| `0.11.10` | 0 | 1 | 3 | `tests.rs`:52,58,61 |
| `1.0.0-dev` | 0 | 1 | 3 | `tests.rs`:117,121,497 |
| `1752624100` | 0 | 1 | 3 | `tests.rs`:1293,1313,1321 |
| `2026-08-30-abc123` | 0 | 1 | 3 | `tests.rs`:293,335,386 |
| `20260716-110000` | 0 | 1 | 3 | `tests.rs`:1295,1318,1359 |
| `; else pass ` | 0 | 1 | 3 | `test-harness.sh`:24,30,45 |
| `MAIN_SIDE=1\n` | 0 | 1 | 3 | `tests.rs`:429,457,801 |
| `SHIM` | 0 | 1 | 3 | `test-bootstrap.sh`:99,126,769 |
| `SS_MAGIC_TEST_EXPECT` | 0 | 1 | 3 | `tests.rs`:866,896,921 |
| `\"etag-1\"` | 0 | 1 | 3 | `tests.rs`:494,503,507 |
| `\"prior\"` | 0 | 1 | 3 | `tests.rs`:342,671,692 |
| `a-wish` | 0 | 1 | 3 | `tests.rs`:236,243,349 |
| `a.env` | 0 | 1 | 3 | `tests.rs`:279,660,1250 |
| `aaa.txt` | 0 | 1 | 3 | `tests.rs`:96,119,156 |
| `actions/checkout` | 0 | 1 | 3 | `tests.rs`:155,167,171 |
| `apps/api/debug.log` | 0 | 1 | 3 | `tests.rs`:139,421,434 |
| `attempt {attempt}: {reason}` | 0 | 1 | 3 | `tests.rs`:835,838,842 |
| `b.env` | 0 | 1 | 3 | `tests.rs`:285,305,666 |
| `binary — differs` | 0 | 1 | 3 | `tests.rs`:236,535,603 |
| `c.env` | 0 | 1 | 3 | `tests.rs`:294,786,982 |
| `compaction` | 0 | 1 | 3 | `tests.rs`:1258,1263,1268 |
| `contend.lock` | 0 | 1 | 3 | `tests.rs`:320,328,333 |
| `docs/actions/2026-08-ghost.checklist.json` | 0 | 1 | 3 | `tests.rs`:2759,2760,2763 |
| `docs/actions/x.checklist.json` | 0 | 1 | 3 | `tests.rs`:72,143,178 |
| `docs/handbook.md` | 0 | 1 | 3 | `tests.rs`:460,463,481 |
| `expected pretty-printed JSON` | 0 | 1 | 3 | `tests.rs`:179,562,723 |
| `expected trailing newline` | 0 | 1 | 3 | `tests.rs`:180,563,724 |
| `findings` | 0 | 1 | 3 | `tests.rs`:257,378,385 |
| `futureFeature` | 0 | 1 | 3 | `tests.rs`:490,501,521 |
| `gone-new.env` | 0 | 1 | 3 | `tests.rs`:360,375,902 |
| `gone.env` | 0 | 1 | 3 | `tests.rs`:351,374,893 |
| `https://example.com/org/repo` | 0 | 1 | 3 | `tests.rs`:67,73,101 |
| `https://example.com/owner/repo` | 0 | 1 | 3 | `tests.rs`:1072,1076,1080 |
| `huge.rs` | 0 | 1 | 3 | `tests.rs`:393,407,422 |
| `magic.sh must be 0755` | 0 | 1 | 3 | `tests.rs`:284,500,581 |
| `node_modules/\n` | 0 | 1 | 3 | `tests.rs`:151,269,419 |
| `outside the repository` | 0 | 1 | 3 | `tests.rs`:1036,1038,1047 |
| `outside this worktree` | 0 | 1 | 3 | `tests.rs`:321,324,351 |
| `projectPath` | 0 | 1 | 3 | `tests.rs`:395,396,397 |
| `real.env` | 0 | 1 | 3 | `tests.rs`:1505,1506,1518 |
| `refs.-` | 0 | 1 | 3 | `tests.rs`:311,426,862 |
| `s-end` | 0 | 1 | 3 | `tests.rs`:134,136,153 |
| `s-grow` | 0 | 1 | 3 | `tests.rs`:567,571,575 |
| `s-mixed` | 0 | 1 | 3 | `tests.rs`:350,353,361 |
| `s-newsub` | 0 | 1 | 3 | `tests.rs`:634,637,643 |
| `s-nocwd` | 0 | 1 | 3 | `tests.rs`:782,782,787 |
| `s-rotate` | 0 | 1 | 3 | `tests.rs`:597,604,615 |
| `s-tolerant` | 0 | 1 | 3 | `tests.rs`:839,850,858 |
| `s-twice` | 0 | 1 | 3 | `tests.rs`:452,464,467 |
| `s-wt` | 0 | 1 | 3 | `tests.rs`:1259,1271,1277 |
| `secret.bin` | 0 | 1 | 3 | `tests.rs`:227,531,599 |
| `session-new` | 0 | 1 | 3 | `tests.rs`:134,135,145 |
| `short.env` | 0 | 1 | 3 | `tests.rs`:463,1031,1031 |
| `some-future-source` | 0 | 1 | 3 | `tests.rs`:193,203,593 |
| `some-future-trigger` | 0 | 1 | 3 | `tests.rs`:188,195,200 |
| `state/checkout-tracker.json` | 0 | 1 | 3 | `tests.rs`:2121,2132,2165 |
| `subagents/agent-a.jsonl` | 0 | 1 | 3 | `tests.rs`:321,357,1160 |
| `target/\n` | 0 | 1 | 3 | `tests.rs`:42,77,96 |
| `unexpected note: {note}` | 0 | 1 | 3 | `tests.rs`:184,199,213 |
| `untracked.txt` | 0 | 1 | 3 | `tests.rs`:137,139,150 |
| `upx-cz` | 0 | 1 | 3 | `tests.rs`:40,51,64 |
| `v3.0.0` | 0 | 1 | 3 | `tests.rs`:561,568,573 |
| `{case}: {err:#}` | 0 | 1 | 3 | `tests.rs`:1036,1038,1047 |
| `{written}` | 0 | 1 | 3 | `tests.rs`:331,470,496 |
