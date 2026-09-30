# Bare numeric literals >= 60 in non-test Rust (outside const declarations)

Candidates for a named constant, or for reuse of one that already exists (see consts.md).

## 60

- `crates/ss-magic/src/sync/reverse_sync.rs:710` – `let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);`
- `crates/ss-magic/src/sync/reverse_sync.rs:710` – `let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);`
- `crates/ss-magic/src/tui/cockpit.rs:715` – `if secs < 60 {`
- `crates/ss-magic/src/tui/cockpit.rs:718` – `format!("{}m ago", secs / 60)`
- `crates/ss-magic-plugin/src/checklist/schema.rs:589` – `+ i64::from(minute) * 60`
- `crates/ss-magic-plugin/src/checklist/schema.rs:624` – `Ok(sign * (i64::from(hours) * 3600 + i64::from(minutes) * 60))`
- `crates/ss-magic-plugin/src/release_check.rs:566` – `let (m, s) = (rem / 60, rem % 60);`
- `crates/ss-magic-plugin/src/release_check.rs:566` – `let (m, s) = (rem / 60, rem % 60);`
- `crates/ss-magic-plugin/src/scratchpad.rs:658` – `let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);`
- `crates/ss-magic-plugin/src/scratchpad.rs:658` – `let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);`

## 62

- `crates/ss-magic/src/tui/cockpit.rs:871` – `let panes = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(body);`

## 63

- `crates/ss-magic-plugin/src/cache.rs:607` – `if attempt == 63 {`

## 64

- `crates/ss-magic-core/src/hashing.rs:95` – `while msg.len() % 64 != 56 {`
- `crates/ss-magic-core/src/hashing.rs:103` – `let (blocks, _) = msg.as_chunks::<64>();`
- `crates/ss-magic-core/src/hashing.rs:107` – `let mut w = [0u32; 64];`
- `crates/ss-magic-plugin/src/ledger.rs:683` – `let mut buf = Vec::with_capacity(64 * 1024);`

## 88

- `crates/ss-magic/src/tui/cockpit.rs:1494` – `let popup = centered_rect(88, 90, area);`

## 90

- `crates/ss-magic/src/tui/cockpit.rs:1494` – `let popup = centered_rect(88, 90, area);`

## 100

- `crates/ss-magic/src/sync/reverse_sync.rs:717` – `let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year [0, 365]`
- `crates/ss-magic/src/tui/cockpit.rs:1615` – `Constraint::Percentage((100 - percent_y) / 2),`
- `crates/ss-magic/src/tui/cockpit.rs:1617` – `Constraint::Percentage((100 - percent_y) / 2),`
- `crates/ss-magic/src/tui/cockpit.rs:1621` – `Constraint::Percentage((100 - percent_x) / 2),`
- `crates/ss-magic/src/tui/cockpit.rs:1623` – `Constraint::Percentage((100 - percent_x) / 2),`
- `crates/ss-magic-plugin/src/checklist/schema.rs:654` – `(year % 4 == 0 && year % 100 != 0) || year % 400 == 0`
- `crates/ss-magic-plugin/src/checklist/schema.rs:678` – `let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;`
- `crates/ss-magic-plugin/src/scratchpad.rs:665` – `let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);`

## 117

- `crates/ss-magic-plugin/src/status.rs:675` – `let cut: String = line.chars().take(117).collect();`

## 120

- `crates/ss-magic-plugin/src/status.rs:674` – `if line.chars().count() > 120 {`

## 153

- `crates/ss-magic/src/sync/reverse_sync.rs:718` – `let mp = (5 * doy + 2) / 153; // month index in the Mar-first calendar [0, 11]`
- `crates/ss-magic/src/sync/reverse_sync.rs:719` – `let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]`
- `crates/ss-magic-plugin/src/checklist/schema.rs:677` – `let doy = (153 * i64::from(mp) + 2) / 5 + i64::from(day) - 1;`
- `crates/ss-magic-plugin/src/scratchpad.rs:666` – `let mp = (5 * doy + 2) / 153;`
- `crates/ss-magic-plugin/src/scratchpad.rs:667` – `let d = doy - (153 * mp + 2) / 5 + 1;`

## 200

- `crates/ss-magic-core/src/release.rs:563` – `if status != 200 {`

## 300

- `crates/ss-magic-plugin/src/identity.rs:142` – `if ('\u{0300}'..='\u{036F}').contains(&ch) {`

## 304

- `crates/ss-magic-core/src/release.rs:560` – `if status == 304 {`
- `crates/ss-magic-plugin/src/release_check.rs:746` – `"GitHub answered 304; the cached tag is still current".to_string(),`

## 365

- `crates/ss-magic/src/sync/reverse_sync.rs:716` – `let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]`
- `crates/ss-magic/src/sync/reverse_sync.rs:717` – `let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year [0, 365]`
- `crates/ss-magic/src/sync/reverse_sync.rs:717` – `let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year [0, 365]`
- `crates/ss-magic-plugin/src/checklist/schema.rs:678` – `let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;`
- `crates/ss-magic-plugin/src/scratchpad.rs:664` – `let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;`
- `crates/ss-magic-plugin/src/scratchpad.rs:665` – `let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);`

## 399

- `crates/ss-magic/src/sync/reverse_sync.rs:716` – `let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]`

## 400

- `crates/ss-magic/src/sync/reverse_sync.rs:721` – `let y = yoe as i64 + era * 400 + i64::from(m <= 2);`
- `crates/ss-magic-plugin/src/checklist/schema.rs:654` – `(year % 4 == 0 && year % 100 != 0) || year % 400 == 0`
- `crates/ss-magic-plugin/src/checklist/schema.rs:674` – `let era = year.div_euclid(400);`
- `crates/ss-magic-plugin/src/checklist/schema.rs:675` – `let yoe = year - era * 400;`
- `crates/ss-magic-plugin/src/scratchpad.rs:669` – `let y = yoe + era * 400 + i64::from(m <= 2);`

## 512

- `crates/ss-magic-core/src/git/discover.rs:598` – `let mut buf = Vec::with_capacity(cap.min(512) + 1);`

## 755

- `crates/ss-magic-core/src/superset_files.rs:372` – `fs::set_permissions(path, perms).with_context(|| format!("chmod 0755 {}", path.display()))?;`

## 999

- `crates/ss-magic-plugin/src/ledger.rs:1536` – `0..=999 => n.to_string(),`

## 1024

- `crates/ss-magic-plugin/src/hook/pre_tool_use.rs:1075` – `size.div_ceil(1024)`

## 1460

- `crates/ss-magic/src/sync/reverse_sync.rs:716` – `let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]`
- `crates/ss-magic-plugin/src/scratchpad.rs:664` – `let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;`

## 2013

- `crates/ss-magic-plugin/src/checklist/render.rs:96` – `everything below \u{2013} the metadata, every changelog entry, and \`
- `crates/ss-magic-plugin/src/checklist/render.rs:98` – `reference labels \u{2013} is free-form prose the repository \`
- `crates/ss-magic-plugin/src/checklist/render.rs:262` – `let _ = write!(out, "- **{when}** \u{2013} {summary}");`

## 3000

- `crates/ss-magic-plugin/src/config.rs:358` – `set parses VALUE as JSON when it parses (true, 3000, [\"docs/**\"], null,`

## 3600

- `crates/ss-magic/src/sync/reverse_sync.rs:710` – `let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);`
- `crates/ss-magic/src/sync/reverse_sync.rs:710` – `let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);`
- `crates/ss-magic/src/tui/cockpit.rs:717` – `} else if secs < 3600 {`
- `crates/ss-magic/src/tui/cockpit.rs:720` – `format!("{}h ago", secs / 3600)`
- `crates/ss-magic-plugin/src/checklist/schema.rs:588` – `+ i64::from(hour) * 3600`
- `crates/ss-magic-plugin/src/checklist/schema.rs:624` – `Ok(sign * (i64::from(hours) * 3600 + i64::from(minutes) * 60))`
- `crates/ss-magic-plugin/src/release_check.rs:565` – `let (h, rem) = (rem / 3_600, rem % 3_600);`
- `crates/ss-magic-plugin/src/release_check.rs:565` – `let (h, rem) = (rem / 3_600, rem % 3_600);`
- `crates/ss-magic-plugin/src/scratchpad.rs:658` – `let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);`
- `crates/ss-magic-plugin/src/scratchpad.rs:658` – `let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);`

## 8601

- `crates/ss-magic-plugin/src/checklist/schema.rs:169` – `f.write_str("not an ISO-8601 timestamp of the form YYYY-MM-DDTHH:MM:SS±HH:MM")`

## 36524

- `crates/ss-magic/src/sync/reverse_sync.rs:716` – `let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]`
- `crates/ss-magic-plugin/src/scratchpad.rs:664` – `let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;`

## 86400

- `crates/ss-magic/src/sync/reverse_sync.rs:708` – `let days = secs / 86_400;`
- `crates/ss-magic/src/sync/reverse_sync.rs:709` – `let rem = secs % 86_400;`
- `crates/ss-magic/src/tui/cockpit.rs:719` – `} else if secs < 86_400 {`
- `crates/ss-magic/src/tui/cockpit.rs:722` – `format!("{}d ago", secs / 86_400)`
- `crates/ss-magic-plugin/src/checklist/schema.rs:587` – `let secs = days_from_civil(year, month, day) * 86_400`
- `crates/ss-magic-plugin/src/release_check.rs:564` – `let (d, rem) = (secs / 86_400, secs % 86_400);`
- `crates/ss-magic-plugin/src/release_check.rs:564` – `let (d, rem) = (secs / 86_400, secs % 86_400);`
- `crates/ss-magic-plugin/src/scratchpad.rs:656` – `let days = secs / 86_400;`
- `crates/ss-magic-plugin/src/scratchpad.rs:657` – `let rem = secs % 86_400;`

## 100000

- `crates/ss-magic-plugin/src/compact_window.rs:143` – `--set writes an absolute auto-compact window (100000-1000000 tokens) into`

## 146096

- `crates/ss-magic/src/sync/reverse_sync.rs:715` – `let doe = z.rem_euclid(146_097) as u64; // day-of-era [0, 146096]`
- `crates/ss-magic/src/sync/reverse_sync.rs:716` – `let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]`
- `crates/ss-magic-plugin/src/scratchpad.rs:664` – `let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;`

## 146097

- `crates/ss-magic/src/sync/reverse_sync.rs:714` – `let era = z.div_euclid(146_097);`
- `crates/ss-magic/src/sync/reverse_sync.rs:715` – `let doe = z.rem_euclid(146_097) as u64; // day-of-era [0, 146096]`
- `crates/ss-magic-plugin/src/checklist/schema.rs:679` – `era * 146_097 + doe - 719_468`
- `crates/ss-magic-plugin/src/scratchpad.rs:662` – `let era = z.div_euclid(146_097);`
- `crates/ss-magic-plugin/src/scratchpad.rs:663` – `let doe = z.rem_euclid(146_097);`

## 719468

- `crates/ss-magic/src/sync/reverse_sync.rs:713` – `let z = days as i64 + 719_468;`
- `crates/ss-magic-plugin/src/checklist/schema.rs:679` – `era * 146_097 + doe - 719_468`
- `crates/ss-magic-plugin/src/scratchpad.rs:661` – `let z = days as i64 + 719_468;`

## 999999

- `crates/ss-magic-plugin/src/ledger.rs:1537` – `1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),`

## 1000000

- `crates/ss-magic-plugin/src/compact_window.rs:143` – `--set writes an absolute auto-compact window (100000-1000000 tokens) into`

