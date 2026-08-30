//! `cost.jsonl` — one row per session, and the `cost` verb that reads it.
//!
//! A session's cost is not something the harness hands us. The `SessionEnd`
//! payload carries six keys and none of them is a number of tokens, so the
//! only place the figure exists is the session's own transcript tree. This
//! module walks that tree, adds the usage up, and leaves exactly one row per
//! session id in a machine-level log — which is what lets somebody ask, a week
//! later, whether the branch they spent Tuesday on cost three times what the
//! branch they spent Wednesday on did.
//!
//! ## What "one row per session id" has to survive
//!
//! Two things make idempotency a hard requirement rather than a nicety:
//!
//! - A single CLI process produces **more than one session id**. `/clear`
//!   mints a new one and fires `SessionEnd(reason:"clear")` for the old,
//!   `SessionStart(source:"clear")` for the new, and a final
//!   `SessionEnd(reason:"other")` on exit.
//! - The harness spawns hooks per event and is free to spawn duplicates, so
//!   two `session-end` invocations for one id can run **at the same instant**.
//!
//! Both end the same way here: the scan happens outside the lock (it is the
//! expensive part, and two unrelated sessions ending at once must not queue
//! behind each other's scans), and the commit happens inside it. Under the
//! lock the ledger is re-read; a row for this session id that already
//! describes the same tree means somebody else got there first and this
//! invocation writes nothing at all. See [`commit`].
//!
//! ## Where the numbers come from, in order
//!
//! 1. **Claude Code's own priced records.** Main-session transcripts carry
//!    `{"type":"cost-state", totalCostUSD, modelUsage{…}}` lines — the
//!    harness's own arithmetic, cumulative over the session. Where one exists
//!    it is believed over anything computed here.
//! 2. **The bundled price table**, for everything cost-state cannot answer:
//!    subagent transcripts (which never carry a cost-state record), and
//!    transcripts written before the harness started emitting them.
//!
//! The table is [`PRICE_TABLE_VERSION`]-stamped and snapshotted into the store
//! the first time it is used, so a row written today keeps meaning what it
//! meant when it was written even after a later ss-magic ships different
//! prices. A model the table does not know is recorded in
//! [`Row::unpriced_models`] rather than silently priced at zero.
//!
//! **Read the nested cache fields, not the flat total.** `usage` carries both
//! `cache_creation_input_tokens` (a sum) and `usage.cache_creation.{
//! ephemeral_1h_input_tokens, ephemeral_5m_input_tokens}` (the split). The two
//! TTLs price differently — a 1-hour cache write costs 2x base input, a
//! 5-minute write 1.25x — so reading only the flat total understates the bill.
//! On the session this feature was measured against it understated the main
//! thread by ~13%.
//!
//! ## It is a signal, not an invoice
//!
//! Nothing derived from a transcript can know about negotiated rates, org
//! discounts, or billing reconciliation, and the fallback table goes stale the
//! moment prices move. The figures are sound for comparing one branch against
//! another and unsound as a bill, and the `cost` output says exactly that on
//! its face rather than leaving somebody to find out later.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write as _};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::git;
use crate::plugin::heartbeat;
use crate::plugin::scratchpad::format_rfc3339;
use crate::plugin::tmproot;
use crate::tui::style;

// ── Layout ────────────────────────────────────────────────────────────────────

/// The ledger itself, beside `hooks.jsonl` in the machine-level store.
pub const LEDGER_FILE_NAME: &str = "cost.jsonl";

/// Where each scanned transcript file was left off, so a re-scan reads only
/// what has been appended since. See [`Offsets`].
pub const OFFSETS_FILE_NAME: &str = "transcript-offsets.json";

/// Directory holding one snapshot of the price table per version that has ever
/// been used to price a row here.
pub const PRICES_DIR_NAME: &str = "prices";

/// The lock guarding the commit. A sibling file rather than the ledger itself,
/// for the reason `heartbeat.rs` gives: an update rewrites the ledger
/// wholesale, and locking a file we then replace would leave the lock on an
/// unlinked inode.
const LOCK_FILE_NAME: &str = "cost.lock";

/// Owner-only modes, matching the rest of the machine-level store. A row names
/// repository paths and, through them, what somebody works on.
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

// ── The price table ───────────────────────────────────────────────────────────

/// The version stamped into every row this table prices, and the name of its
/// snapshot file in the store. It is the date the rates below were published,
/// not the date ss-magic was built — a row saying `2026-06-24` tells a reader
/// exactly which price list produced its figure.
pub const PRICE_TABLE_VERSION: &str = "2026-06-24";

/// List prices in USD per million tokens: `(model id, input, output)`.
///
/// Deliberately short. Every row here is a documented first-party rate; a
/// model that is not on the list is reported as unpriced rather than priced
/// from a guess, because a plausible-looking wrong number is worse than a
/// visible gap in a figure people are asked to reason about. Matching is by
/// longest id prefix, so a dated snapshot (`claude-haiku-4-5-20251001`) prices
/// as its family.
const PRICES: &[(&str, f64, f64)] = &[
    ("claude-fable-5", 10.0, 50.0),
    ("claude-mythos-5", 10.0, 50.0),
    ("claude-opus-5", 5.0, 25.0),
    ("claude-opus-4-8", 5.0, 25.0),
    ("claude-opus-4-7", 5.0, 25.0),
    ("claude-opus-4-6", 5.0, 25.0),
    ("claude-sonnet-5", 2.0, 10.0),
    ("claude-sonnet-4-6", 3.0, 15.0),
    ("claude-haiku-4-5", 1.0, 5.0),
];

/// A cache read costs a tenth of base input.
const CACHE_READ_MULT: f64 = 0.10;
/// A 5-minute cache write costs 1.25x base input.
const CACHE_WRITE_5M_MULT: f64 = 1.25;
/// A 1-hour cache write costs 2x base input — the whole reason the nested
/// `cache_creation` fields have to be read instead of the flat total.
const CACHE_WRITE_1H_MULT: f64 = 2.00;

/// Per-million-token rates for `model`, or `None` when the table has no entry.
fn price_for(model: &str) -> Option<(f64, f64)> {
    PRICES
        .iter()
        .filter(|(id, _, _)| model.starts_with(id))
        // Longest prefix wins, so `claude-opus-4-8` is not shadowed by a
        // shorter family id that happens to also match.
        .max_by_key(|(id, _, _)| id.len())
        .map(|(_, input, output)| (*input, *output))
}

/// What [`Tokens`] costs at `model`'s rates, or `None` for an unknown model.
fn price_tokens(model: &str, tokens: &Tokens) -> Option<f64> {
    let (input, output) = price_for(model)?;
    let per_token = |mtok_rate: f64, count: u64| mtok_rate * (count as f64) / 1_000_000.0;
    Some(
        per_token(input, tokens.input)
            + per_token(output, tokens.output)
            + per_token(input * CACHE_READ_MULT, tokens.cache_read)
            + per_token(input * CACHE_WRITE_5M_MULT, tokens.cache_write_5m)
            + per_token(input * CACHE_WRITE_1H_MULT, tokens.cache_write_1h),
    )
}

/// The table as it goes into a snapshot file: self-describing, so somebody
/// reading `prices/2026-06-24.json` next year can see both the rates and the
/// multipliers that produced a row.
fn price_table_snapshot() -> Value {
    let models: Vec<Value> = PRICES
        .iter()
        .map(|(id, input, output)| {
            serde_json::json!({
                "model": id,
                "input_usd_per_mtok": input,
                "output_usd_per_mtok": output,
            })
        })
        .collect();
    serde_json::json!({
        "version": PRICE_TABLE_VERSION,
        "cache_read_multiplier": CACHE_READ_MULT,
        "cache_write_5m_multiplier": CACHE_WRITE_5M_MULT,
        "cache_write_1h_multiplier": CACHE_WRITE_1H_MULT,
        "models": models,
    })
}

// ── The row ───────────────────────────────────────────────────────────────────

/// Token counts, summed across every transcript in one session's tree.
///
/// The two cache-write fields are kept apart all the way through rather than
/// being added up, because they do not cost the same — see the module docs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// 5-minute-TTL cache writes (`ephemeral_5m_input_tokens`).
    pub cache_write_5m: u64,
    /// 1-hour-TTL cache writes (`ephemeral_1h_input_tokens`).
    pub cache_write_1h: u64,
}

impl Tokens {
    /// Fold `other` into this total.
    fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
    }
}

/// Where a row's dollar figure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Basis {
    /// Entirely from the harness's own priced records.
    Harness,
    /// Entirely from the bundled price table.
    Table,
    /// The harness priced the main thread; the table priced the subagents.
    Mixed,
}

impl Basis {
    /// How the `cost` output names it.
    fn label(self) -> &'static str {
        match self {
            Self::Harness => "harness-priced",
            Self::Table => "table-priced",
            Self::Mixed => "harness + table",
        }
    }
}

/// One line of `cost.jsonl` — everything known about one ended session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// The harness's session id. The ledger's key: there is never more than
    /// one row carrying a given value here.
    pub session_id: String,
    /// UTC, RFC 3339, when the row was last written.
    pub at: String,
    /// The same instant as whole seconds since the epoch, so nothing has to
    /// parse `at` back to compare two rows.
    pub ts: u64,
    /// The worktree root this session mostly worked in — the grouping label.
    /// `None` when no `cwd` in the transcript resolved to anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Any other root the same session touched. Normally empty: a session's
    /// `cwd` values differ (a nested scratchpad, say) but normalize to one
    /// worktree. A non-empty list means the session genuinely spanned two, and
    /// saying so beats attributing all of it to the busier one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_roots: Vec<String>,
    /// The branch label, from the transcript's own `gitBranch` field. Not a
    /// key: two worktrees can be on the same branch name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// How many transcript files the tree held, and how many bytes they came
    /// to, when this row was written. Together they are the staleness check: a
    /// later invocation that measures the same pair knows the row is current
    /// and writes nothing.
    pub files: usize,
    pub bytes: u64,
    /// Token totals across the whole tree, main thread and subagents alike.
    pub tokens: Tokens,
    /// The figure the rest of this row exists to produce, in USD.
    pub cost_usd: f64,
    /// Where that figure came from.
    pub basis: Basis,
    /// The harness's own cumulative `totalCostUSD`, when the transcript had
    /// one. Kept separately from the computed halves so a re-scan can refresh
    /// one without disturbing the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_cost_usd: Option<f64>,
    /// Table-priced cost of the MAIN transcript. Only counted toward
    /// [`Self::cost_usd`] when there is no harness figure to believe instead —
    /// otherwise it would double-count what cost-state already covers.
    pub main_table_usd: f64,
    /// Table-priced cost of the subagent transcripts. Always counted: the
    /// harness's records never mention subagents.
    pub sub_table_usd: f64,
    /// The price-table version used, when the table was used at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_table: Option<String>,
    /// Models seen in the tree that the table could not price. Their tokens
    /// are counted; their dollars are not, and this is where that shortfall is
    /// declared rather than hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpriced_models: Vec<String>,
}

// ── The offsets store ─────────────────────────────────────────────────────────

/// Where one transcript file was left off.
///
/// `inode` and `size` are the rotation guard, not bookkeeping: transcript
/// JSONL is append-only *empirically*, and this is what keeps a file that was
/// truncated or replaced from being read at a byte position that now means
/// something else entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Mark {
    /// Bytes already accounted for. Always immediately after a `\n`: a
    /// trailing partial line is left for the next scan rather than parsed.
    offset: u64,
    /// The file's length when the mark was taken.
    size: u64,
    /// The inode it had. A different one means a different file wearing the
    /// same name.
    inode: u64,
}

/// `transcript-offsets.json`: absolute path → [`Mark`]. A `BTreeMap` so the
/// file's key order is stable and a diff of two snapshots is readable.
type Offsets = BTreeMap<String, Mark>;

/// Read the offsets store. A missing or unreadable file is an empty map, which
/// simply means the next scan reads every file in full.
fn read_offsets(store: &Path) -> Offsets {
    fs::read_to_string(store.join(OFFSETS_FILE_NAME))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Replace the offsets store, merging `updates` over what is already there so
/// one session's marks never drop another session's.
///
/// Written through a temp file and a rename: AE33 requires that two racing
/// `session-end` invocations leave this file readable, and a partial write is
/// exactly how it would stop being.
fn write_offsets(store: &Path, updates: &Offsets) -> Result<()> {
    let mut merged = read_offsets(store);
    for (path, mark) in updates {
        merged.insert(path.clone(), *mark);
    }
    let body = format!("{}\n", serde_json::to_string_pretty(&merged)?);
    heartbeat::write_atomically(&store.join(OFFSETS_FILE_NAME), &body, ".offsets-")
}

// ── Reading the ledger ────────────────────────────────────────────────────────

/// The ledger's path inside `store`.
pub fn ledger_path(store: &Path) -> PathBuf {
    store.join(LEDGER_FILE_NAME)
}

/// Every row, oldest first. An unparseable line is skipped rather than failing
/// the read: one bad line must not hide a month of history.
///
/// An absent ledger is an empty `Vec` — nothing has ended yet.
pub fn read(store: &Path) -> Result<Vec<Row>> {
    let path = ledger_path(store);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Row>(line).ok())
        .collect())
}

// ── The transcript tree ───────────────────────────────────────────────────────

/// Every `.jsonl` belonging to one session: the main transcript first, then
/// every subagent transcript beneath the sibling `<session id>/` directory,
/// at any depth (backgrounded agents land in `subagents/`, workflow agents a
/// level deeper still).
///
/// Walking is the only way to find them — nothing in the transcript ties a
/// subagent's file to its parent beyond where it sits on disk. Skipping the
/// walk and reading the main file alone would miss most of the usage records
/// in a session that delegates.
pub fn transcript_tree(transcript: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if transcript.is_file() {
        files.push(transcript.to_path_buf());
    }

    // `…/<slug>/<session id>.jsonl` has its subagents in `…/<slug>/<session
    // id>/`, so the directory is the transcript path with the extension
    // dropped rather than something reconstructed from the session id — which
    // keeps this working if the harness ever names the file differently.
    let subagents = transcript.with_extension("");
    if subagents.is_dir() {
        collect_jsonl(&subagents, &mut files);
    }
    files
}

/// Depth-first walk collecting `*.jsonl`, sorted at each level so a scan reads
/// the same tree in the same order every time. Unreadable directories are
/// skipped: one of them must not cost us the rest of the tree.
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        // No-follow: a symlink into somebody else's tree is not this session's
        // usage, and following one could walk out of the transcript root
        // entirely.
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect_jsonl(&path, out);
        } else if meta.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

// ── Scanning ──────────────────────────────────────────────────────────────────

/// What one pass over a session's tree found.
#[derive(Debug, Default)]
struct Scan {
    /// Token totals, split so the main thread's can be priced separately from
    /// the subagents' — the harness's own records cover the first and never
    /// the second.
    main_tokens: Tokens,
    sub_tokens: Tokens,
    main_table_usd: f64,
    sub_table_usd: f64,
    /// The largest `totalCostUSD` any cost-state record reported. Those
    /// records are cumulative over the session, so the largest is the latest —
    /// and taking the largest rather than the last is what makes an
    /// incremental re-scan safe when the tail happens to hold none.
    harness_cost_usd: Option<f64>,
    /// Models with no entry in the price table.
    unpriced: BTreeSet<String>,
    /// How often each distinct `cwd` appeared, and each `gitBranch`.
    cwds: HashMap<String, usize>,
    branches: HashMap<String, usize>,
    /// Whether the table priced anything at all, which decides whether the row
    /// carries a price-table version.
    used_table: bool,
    /// Where each file was left off.
    offsets: Offsets,
    /// The tree's shape, from metadata alone.
    files: usize,
    bytes: u64,
    /// True when a stored mark failed validation and the whole tree had to be
    /// re-read from the start.
    full_rescan: bool,
}

/// A line matching this had a `usage` block, which is the only thing worth
/// parsing a multi-megabyte tool-result line for.
const USAGE_NEEDLE: &str = "\"usage\"";

/// A cost-state record names its own type in its first few keys, so only the
/// head of a line is searched for it. `modelUsage` deliberately does not match
/// [`USAGE_NEEDLE`] (capital U), so the two checks never fight over one line.
const COST_STATE_NEEDLE: &[u8] = b"\"cost-state\"";

/// How much of a line's head is searched for [`COST_STATE_NEEDLE`]. Generous
/// for a record whose `type` is the first key, and small enough that the check
/// costs nothing on the huge lines that make up the bulk of a transcript.
const HEAD_SCAN_BYTES: usize = 256;

/// Read buffer per transcript file. Large enough that a 400 MB tree is a few
/// thousand reads rather than a few hundred thousand.
const READ_BUF_BYTES: usize = 256 * 1024;

/// Scan a session's tree, continuing from `prior` where every mark still
/// validates.
///
/// The incremental path is deliberately all-or-nothing. If any file with a
/// stored mark has shrunk below it or changed inode, the mark is meaningless
/// and that file must be re-read from the start — but its earlier contribution
/// is already folded into the caller's previous row, so re-reading it alone
/// would count it twice. So one bad mark discards them all and the whole tree
/// is re-read from zero, which the caller detects through
/// [`Scan::full_rescan`] and answers by dropping the previous totals. A file
/// with no mark at all is new, has contributed nothing yet, and is simply read
/// from the start.
fn scan_tree(files: &[PathBuf], prior: &Offsets) -> Scan {
    let (starts, full_rescan) = plan_starts(files, prior);
    let mut scan = Scan {
        full_rescan,
        ..Scan::default()
    };

    for (index, path) in files.iter().enumerate() {
        let Ok(meta) = fs::metadata(path) else {
            continue;
        };
        scan.files += 1;
        scan.bytes += meta.len();

        let is_main = index == 0;
        let start = starts[index];
        let mut per_model: HashMap<String, Tokens> = HashMap::new();
        let end = match scan_file(path, start, &mut per_model, &mut scan) {
            Ok(end) => end,
            // A file we cannot read contributes nothing and stops nothing. Its
            // mark is left untouched so a later run picks it up if the problem
            // was transient.
            Err(_) => continue,
        };

        for (model, tokens) in per_model {
            let target = if is_main {
                &mut scan.main_tokens
            } else {
                &mut scan.sub_tokens
            };
            target.add(&tokens);

            match price_tokens(&model, &tokens) {
                Some(usd) => {
                    scan.used_table = true;
                    if is_main {
                        scan.main_table_usd += usd;
                    } else {
                        scan.sub_table_usd += usd;
                    }
                }
                None => {
                    scan.unpriced.insert(model);
                }
            }
        }

        scan.offsets.insert(
            path.to_string_lossy().into_owned(),
            Mark {
                offset: end,
                size: meta.len(),
                inode: meta.ino(),
            },
        );
    }

    scan
}

/// The byte to start each file at, and whether a stored mark had to be thrown
/// away.
///
/// A mark still describing its file on disk is used; anything else starts at
/// zero. The flag is what the caller needs, not just the zeroes: a file
/// starting at zero because it is NEW is ordinary and adds to the previous
/// totals, while a file starting at zero because its mark was INVALID means
/// the previous totals cover bytes about to be read again — so one invalid
/// mark zeroes every start AND tells the caller to discard the base. See
/// [`scan_tree`].
fn plan_starts(files: &[PathBuf], prior: &Offsets) -> (Vec<u64>, bool) {
    let mut starts = Vec::with_capacity(files.len());
    let mut rotated = false;

    for path in files {
        let key = path.to_string_lossy();
        let start = match prior.get(key.as_ref()) {
            None => 0,
            Some(mark) => match fs::metadata(path) {
                // Shorter than we already read, or a different file wearing
                // the same name: the mark is meaningless.
                Ok(meta) if meta.len() < mark.offset || meta.ino() != mark.inode => {
                    rotated = true;
                    0
                }
                Ok(_) => mark.offset,
                Err(_) => 0,
            },
        };
        starts.push(start);
    }

    if rotated {
        return (vec![0; files.len()], true);
    }
    (starts, false)
}

/// Read one transcript from `start`, folding what it holds into `scan` and
/// `per_model`. Returns the byte immediately after the last complete line.
fn scan_file(
    path: &Path,
    start: u64,
    per_model: &mut HashMap<String, Tokens>,
    scan: &mut Scan,
) -> Result<u64> {
    let mut file = File::open(path)?;
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut reader = BufReader::with_capacity(READ_BUF_BYTES, file);

    let mut buf = Vec::with_capacity(64 * 1024);
    let mut pos = start;
    loop {
        buf.clear();
        let read = reader.read_until(b'\n', &mut buf)?;
        if read == 0 {
            break;
        }
        // A line with no terminator is still being written. Leave it — and the
        // offset short of it — for the next scan rather than parsing a
        // fragment.
        if !buf.ends_with(b"\n") {
            break;
        }
        pos += read as u64;
        read_line(&buf, per_model, scan);
    }
    Ok(pos)
}

/// Classify one transcript line and fold whatever it carries into the scan.
///
/// The two cheap checks in front of the JSON parse are what makes the whole
/// thing fit in a hook's budget: most bytes in a transcript are tool-result
/// text on `user` lines, and those match neither needle, so they are never
/// parsed.
fn read_line(line: &[u8], per_model: &mut HashMap<String, Tokens>, scan: &mut Scan) {
    let head = &line[..line.len().min(HEAD_SCAN_BYTES)];
    let looks_like_cost_state = contains(head, COST_STATE_NEEDLE);

    // Invalid UTF-8 in a transcript line is not something we can interpret, and
    // it is not worth failing a scan over.
    let Ok(text) = std::str::from_utf8(line) else {
        return;
    };
    if !looks_like_cost_state && !text.contains(USAGE_NEEDLE) {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };

    match value.get("type").and_then(Value::as_str) {
        Some("cost-state") => read_cost_state(&value, scan),
        Some("assistant") => read_usage(&value, per_model, scan),
        _ => {}
    }
}

/// Take the harness's own cumulative figure, keeping the largest seen.
fn read_cost_state(value: &Value, scan: &mut Scan) {
    let Some(total) = value.get("totalCostUSD").and_then(Value::as_f64) else {
        return;
    };
    if !total.is_finite() {
        return;
    }
    scan.harness_cost_usd = Some(match scan.harness_cost_usd {
        Some(previous) if previous > total => previous,
        _ => total,
    });
}

/// Fold one assistant message's `usage` block into the per-model totals, and
/// record the `cwd` / `gitBranch` labels the row is grouped by.
fn read_usage(value: &Value, per_model: &mut HashMap<String, Tokens>, scan: &mut Scan) {
    if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
        *scan.cwds.entry(cwd.to_string()).or_insert(0) += 1;
    }
    if let Some(branch) = value.get("gitBranch").and_then(Value::as_str) {
        if !branch.is_empty() {
            *scan.branches.entry(branch.to_string()).or_insert(0) += 1;
        }
    }

    let Some(message) = value.get("message") else {
        return;
    };
    let Some(usage) = message.get("usage") else {
        return;
    };
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let num = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0);
    let creation = usage.get("cache_creation");
    let (write_5m, write_1h) = match creation {
        Some(c) => (
            num(c, "ephemeral_5m_input_tokens"),
            num(c, "ephemeral_1h_input_tokens"),
        ),
        // Older transcripts predate the split. Their flat total is all there
        // is, and the 5-minute TTL is the harness's default — so attributing
        // it there under-states rather than over-states.
        None => (num(usage, "cache_creation_input_tokens"), 0),
    };

    let tokens = Tokens {
        input: num(usage, "input_tokens"),
        output: num(usage, "output_tokens"),
        cache_read: num(usage, "cache_read_input_tokens"),
        cache_write_5m: write_5m,
        cache_write_1h: write_1h,
    };
    per_model.entry(model.to_string()).or_default().add(&tokens);
}

/// Naive substring search over bytes. Only ever called on a 256-byte head, so
/// the quadratic worst case is bounded at nothing worth a smarter algorithm.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ── Recording ─────────────────────────────────────────────────────────────────

/// What one session's ingest needs to know beyond the transcript itself.
pub struct Ingest<'a> {
    /// The harness's session id — the ledger's key.
    pub session_id: &'a str,
    /// Absolute path to the main transcript.
    pub transcript: &'a Path,
    /// The directory the session was working in, when the caller knows it,
    /// paired with [`Self::repo_root`] below to seed the `cwd` normalization
    /// cache — which is what spares the ordinary session a `git` subprocess.
    pub cwd: Option<&'a Path>,
    /// The worktree root [`Self::cwd`] resolves to, when the caller already
    /// knows it. Also the fallback label for a transcript with no `cwd` of its
    /// own to normalize.
    pub repo_root: Option<&'a Path>,
    /// Seconds since the epoch, stamped into the row.
    pub now: u64,
}

/// What [`record`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// A row was appended or refreshed.
    Written(Box<Row>),
    /// The ledger already held a current row for this session id — a duplicate
    /// invocation, sequential or racing.
    Unchanged,
    /// There was no transcript to scan.
    NoTranscript,
}

/// Scan one session's tree and leave exactly one row for it in `store`.
///
/// Safe to call any number of times for one session id, from any number of
/// processes at once. The scan runs outside the lock so two unrelated sessions
/// ending together do not queue behind each other; the decision and the write
/// happen inside it.
pub fn record(store: &Path, ingest: &Ingest<'_>) -> Result<Recorded> {
    ensure_store(store)?;

    let files = transcript_tree(ingest.transcript);
    if files.is_empty() {
        return Ok(Recorded::NoTranscript);
    }

    // Read the previous row and the marks it left behind. Without a previous
    // row there is nothing to add a tail to, so the marks are ignored and the
    // tree is read in full.
    let previous = find_row(&read(store)?, ingest.session_id);
    let offsets = match previous {
        Some(_) => read_offsets(store),
        None => Offsets::default(),
    };

    let scan = scan_tree(&files, &offsets);
    // A rotated file invalidated the whole incremental base, so the previous
    // row's totals must not be added to what was just re-read from zero.
    let base = if scan.full_rescan { None } else { previous };
    let row = build_row(ingest, base.as_ref(), &scan);

    let outcome = tmproot::with_lock(store, LOCK_FILE_NAME, || commit(store, row, base, &scan))
        .with_context(|| format!("locking the cost ledger in {}", store.display()))?;
    outcome
}

/// The critical section: decide whether this row is still worth writing, then
/// write it along with the marks and the price snapshot.
///
/// `base` is the row the scan was incremental against, exactly as it was read
/// before the scan started — `None` when the tree was read in full and the
/// result stands on its own.
fn commit(store: &Path, row: Row, base: Option<Row>, scan: &Scan) -> Result<Recorded> {
    let rows = read(store)?;

    if let Some(existing) = find_row(&rows, &row.session_id) {
        // Somebody else already recorded this session, and their row describes
        // the same tree this one does. Whichever of the two racers arrived
        // second, it has nothing to add — and it must not append a duplicate.
        if existing.files == row.files && existing.bytes == row.bytes {
            return Ok(Recorded::Unchanged);
        }

        // An incremental row is only meaningful on top of the exact row it
        // added a tail to. If that row has been replaced while this
        // invocation was scanning, the arithmetic underneath this one is
        // built on a ledger that no longer exists — so leave the newer row
        // alone rather than overwriting it with a stale sum. The next ending
        // picks the difference up. A full scan carries no such dependency and
        // is written regardless.
        if base.is_some_and(|base| base != existing) {
            return Ok(Recorded::Unchanged);
        }
    }

    if rows.iter().any(|r| r.session_id == row.session_id) {
        replace_row(store, &rows, &row)?;
    } else {
        append_row(store, &row)?;
    }

    write_offsets(store, &scan.offsets)?;
    if scan.used_table {
        // Best-effort: the row's figure is already on disk, and a missing
        // snapshot costs a reader provenance, not the number itself.
        let _ = snapshot_prices(store);
    }

    Ok(Recorded::Written(Box::new(row)))
}

/// The row for `session_id`, if the ledger holds one.
fn find_row(rows: &[Row], session_id: &str) -> Option<Row> {
    rows.iter().find(|r| r.session_id == session_id).cloned()
}

/// Assemble the row, folding `base`'s totals in when this was an incremental
/// scan of a tree that has only grown.
fn build_row(ingest: &Ingest<'_>, base: Option<&Row>, scan: &Scan) -> Row {
    let mut tokens = scan.main_tokens;
    tokens.add(&scan.sub_tokens);

    let mut main_table_usd = scan.main_table_usd;
    let mut sub_table_usd = scan.sub_table_usd;
    let mut harness_cost_usd = scan.harness_cost_usd;
    let mut unpriced: BTreeSet<String> = scan.unpriced.clone();
    let mut used_table = scan.used_table;

    if let Some(base) = base {
        tokens.add(&base.tokens);
        main_table_usd += base.main_table_usd;
        sub_table_usd += base.sub_table_usd;
        // cost-state is cumulative, so the previous row's figure is a floor,
        // not something to add to.
        harness_cost_usd = match (harness_cost_usd, base.harness_cost_usd) {
            (Some(fresh), Some(old)) => Some(fresh.max(old)),
            (fresh, old) => fresh.or(old),
        };
        unpriced.extend(base.unpriced_models.iter().cloned());
        used_table |= base.price_table.is_some();
    }

    // The harness's figure already covers everything on the main thread, so
    // the main transcript's table price is only used when there is no harness
    // figure to believe instead. The subagents' price is always added: no
    // cost-state record has ever mentioned one.
    let cost_usd = match harness_cost_usd {
        Some(harness) => harness + sub_table_usd,
        None => main_table_usd + sub_table_usd,
    };
    let basis = match (harness_cost_usd.is_some(), sub_table_usd > 0.0) {
        (true, false) => Basis::Harness,
        (true, true) => Basis::Mixed,
        (false, _) => Basis::Table,
    };

    let (root, also_roots) = resolve_roots(ingest, scan, base);
    let branch = dominant(&scan.branches).or_else(|| base.and_then(|b| b.branch.clone()));

    Row {
        session_id: ingest.session_id.to_string(),
        at: format_rfc3339(ingest.now),
        ts: ingest.now,
        root,
        also_roots,
        branch,
        files: scan.files,
        bytes: scan.bytes,
        tokens,
        cost_usd,
        basis,
        harness_cost_usd,
        main_table_usd,
        sub_table_usd,
        price_table: used_table.then(|| PRICE_TABLE_VERSION.to_string()),
        unpriced_models: unpriced.into_iter().collect(),
    }
}

/// Normalize the session's `cwd` values to worktree roots and pick the label.
///
/// A session's `cwd` is per-line and NOT worktree-stable — stepping into a
/// nested directory changes it — so grouping on the raw value would scatter
/// one worktree's cost across several paths. Each distinct value is resolved
/// once, through a cache, because the resolution shells out to git.
///
/// The busiest resolved root becomes the label; any other is listed beside it
/// rather than dropped. A value git cannot resolve (its worktree is gone, or
/// it was never in a repository) is kept as-is: a path is still a usable
/// grouping key, and losing the session entirely would be worse.
fn resolve_roots(
    ingest: &Ingest<'_>,
    scan: &Scan,
    base: Option<&Row>,
) -> (Option<String>, Vec<String>) {
    let mut cache: HashMap<String, String> = HashMap::new();
    // The caller usually knows the answer for the session's own directory
    // already, which spares the common case a subprocess.
    if let (Some(cwd), Some(root)) = (ingest.cwd, ingest.repo_root) {
        cache.insert(
            cwd.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        );
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for (cwd, hits) in &scan.cwds {
        let resolved = match cache.get(cwd) {
            Some(hit) => hit.clone(),
            None => {
                let resolved = git::cwd_repo_root(Path::new(cwd))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| cwd.clone());
                cache.insert(cwd.clone(), resolved.clone());
                resolved
            }
        };
        *counts.entry(resolved).or_insert(0) += *hits;
    }

    if counts.is_empty() {
        // Nothing in the transcript to normalize — an empty tail, or a
        // transcript with no assistant turns at all.
        return match base {
            Some(base) => (base.root.clone(), base.also_roots.clone()),
            None => (
                ingest.repo_root.map(|r| r.to_string_lossy().into_owned()),
                Vec::new(),
            ),
        };
    }

    let root = dominant(&counts);
    let mut also: BTreeSet<String> = counts
        .into_keys()
        .filter(|candidate| Some(candidate) != root.as_ref())
        .collect();
    if let Some(base) = base {
        also.extend(base.also_roots.iter().cloned());
        also.remove(root.as_deref().unwrap_or_default());
    }
    (root, also.into_iter().collect())
}

/// The most frequent key, ties broken alphabetically so one session's label
/// does not flip between two equally busy values from one scan to the next.
fn dominant(counts: &HashMap<String, usize>) -> Option<String> {
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(key, _)| key.clone())
}

// ── Writing ───────────────────────────────────────────────────────────────────

/// Create the store owner-only if it is not there yet.
fn ensure_store(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
        .with_context(|| format!("creating the plugin store at {}", dir.display()))
}

/// Append one row. Called only under the lock, with the ledger already known
/// not to hold this session id.
fn append_row(store: &Path, row: &Row) -> Result<()> {
    let path = ledger_path(store);
    let line = serde_json::to_string(row).context("encoding a ledger row")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(FILE_MODE)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(file, "{line}").with_context(|| format!("appending to {}", path.display()))
}

/// Rewrite the ledger with `row` in place of the existing row for its session
/// id, preserving every other row's position.
///
/// This is what keeps "one row per session id" true for a session that was
/// resumed and ended twice: the second ending refreshes the first row rather
/// than appending a second one that reports the same session at two different
/// costs.
fn replace_row(store: &Path, rows: &[Row], row: &Row) -> Result<()> {
    let mut body = String::new();
    for existing in rows {
        let keep = if existing.session_id == row.session_id {
            row
        } else {
            existing
        };
        body.push_str(&serde_json::to_string(keep).context("encoding a ledger row")?);
        body.push('\n');
    }
    heartbeat::write_atomically(&ledger_path(store), &body, ".cost-")
}

/// Write the price table's snapshot for this version, once. A version already
/// on disk is left alone — that file is the record of what a row written then
/// was priced with, and rewriting it would defeat the point of snapshotting.
fn snapshot_prices(store: &Path) -> Result<()> {
    let dir = store.join(PRICES_DIR_NAME);
    if !dir.is_dir() {
        DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let path = dir.join(format!("{PRICE_TABLE_VERSION}.json"));
    if path.exists() {
        return Ok(());
    }
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(&price_table_snapshot())?
    );
    heartbeat::write_atomically(&path, &body, ".prices-")
}

// ── Locating a transcript ─────────────────────────────────────────────────────

/// Where the harness keeps its transcripts: `$CLAUDE_CONFIG_DIR/projects`, or
/// `~/.claude/projects`. Only needed by the backfill path, which has a session
/// id and no path to go with it.
fn transcript_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("projects"));
        }
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// Find `<session id>.jsonl` under the transcript root. One `read_dir` per
/// project slug — the transcripts are one directory deep and there is no index
/// to consult.
fn find_transcript(root: &Path, session_id: &str) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let candidate = entry.path().join(&name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Seconds since the Unix epoch, or 0 if the clock is set before it.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── The verb ──────────────────────────────────────────────────────────────────

/// Usage for `cost`.
const COST_USAGE: &str = "\
Usage: ss-magic plugin cost [OPTIONS]

Report what recorded sessions cost, grouped by the worktree they ran in.
Every machine-local root is reported, including worktrees that have since
been deleted.

Options:
  --here              Only the worktree this command is run in
  --backfill <REF>    Scan a session that never left a row and record it.
                      <REF> is a session id or a path to a transcript .jsonl
  --json              Machine-readable output
  -h, --help          This text

Figures are derived from session transcripts: Claude Code's own priced
records where it wrote them, a bundled price table otherwise. They are a
relative signal for comparing branches, not a bill.";

/// `ss-magic plugin cost` — a human verb, so problems go to stderr with a
/// non-zero exit.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut here = false;
    let mut json = false;
    let mut backfill: Option<String> = None;

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{COST_USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--here" => here = true,
            "--json" => json = true,
            "--backfill" => match rest.next() {
                Some(reference) => backfill = Some(reference.clone()),
                None => return usage_error("`--backfill` needs a session id or transcript path"),
            },
            other => return usage_error(&format!("unexpected argument `{other}`")),
        }
    }

    let Some(store) = heartbeat::store_dir() else {
        eprintln!(
            "{}",
            style::err(
                "error: this platform has no application data directory to read the ledger from"
            )
        );
        return Ok(ExitCode::from(1));
    };
    let cwd = std::env::current_dir().context("reading the current directory")?;

    if let Some(reference) = backfill {
        return run_backfill(&store, &reference);
    }
    report(&store, &cwd, here, json)
}

/// Print `message` and the usage banner on stderr, and fail.
fn usage_error(message: &str) -> Result<ExitCode> {
    eprintln!("{}", style::err(format!("error: {message}")));
    eprintln!("{COST_USAGE}");
    Ok(ExitCode::from(2))
}

/// `--backfill <REF>`: record a session whose `SessionEnd` never ran.
///
/// The hook cannot cover every ending — a `SIGKILL`ed CLI runs no hook at all
/// — so the transcript, which is already complete on disk, is the fallback
/// source. Recording it goes through exactly the same [`record`] path the hook
/// uses, which is what makes running this on a session that DID leave a row
/// harmless.
fn run_backfill(store: &Path, reference: &str) -> Result<ExitCode> {
    let candidate = PathBuf::from(reference);
    let transcript = if candidate.is_file() {
        candidate
    } else {
        let Some(root) = transcript_root() else {
            eprintln!(
                "{}",
                style::err("error: cannot locate the harness's transcript directory")
            );
            return Ok(ExitCode::from(1));
        };
        match find_transcript(&root, reference) {
            Some(path) => path,
            None => {
                eprintln!(
                    "{}",
                    style::err(format!(
                        "error: no transcript for `{reference}` under {}",
                        root.display()
                    ))
                );
                return Ok(ExitCode::from(1));
            }
        }
    };

    // A transcript path names its session in the file name; a session id given
    // directly is already the answer.
    let session_id = transcript
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| reference.to_string());

    // No `cwd`/`repo_root` seed, deliberately, unlike the hook: the terminal
    // this is typed in is almost never the directory the session ran in, and
    // labeling a backfilled row with the backfiller's own worktree would be
    // worse than leaving it unlabeled. A transcript that records its own `cwd`
    // — nearly all of them — is labeled from that, exactly as the hook's would
    // be.
    let ingest = Ingest {
        session_id: &session_id,
        transcript: &transcript,
        cwd: None,
        repo_root: None,
        now: now_secs(),
    };

    match record(store, &ingest)? {
        Recorded::Written(row) => {
            println!(
                "{}",
                style::ok(format!(
                    "Recorded {} from {}",
                    row.session_id,
                    transcript.display()
                ))
            );
            println!("{}", style::info(format!("  {}", row_summary(&row))));
            Ok(ExitCode::SUCCESS)
        }
        Recorded::Unchanged => {
            println!(
                "{}",
                style::info(format!(
                    "{session_id} is already recorded and the transcript has not grown; nothing to do."
                ))
            );
            Ok(ExitCode::SUCCESS)
        }
        Recorded::NoTranscript => {
            eprintln!(
                "{}",
                style::err(format!(
                    "error: {} holds no transcript",
                    transcript.display()
                ))
            );
            Ok(ExitCode::from(1))
        }
    }
}

/// One session on one line, for the backfill confirmation.
fn row_summary(row: &Row) -> String {
    format!(
        "{} in / {} out / {} cache read — ${:.2} ({})",
        compact(row.tokens.input),
        compact(row.tokens.output),
        compact(row.tokens.cache_read),
        row.cost_usd,
        row.basis.label()
    )
}

/// One root's rolled-up figures.
struct Group {
    /// The worktree root, or `None` for rows that never resolved one.
    root: Option<String>,
    branches: BTreeSet<String>,
    sessions: usize,
    tokens: Tokens,
    cost_usd: f64,
    unpriced: BTreeSet<String>,
}

/// Roll every row up by its root label, busiest root first.
fn group_rows(rows: &[Row]) -> Vec<Group> {
    let mut by_root: BTreeMap<Option<String>, Group> = BTreeMap::new();
    for row in rows {
        let root = row.root.clone();
        let group = by_root.entry(root.clone()).or_insert_with(|| Group {
            root,
            branches: BTreeSet::new(),
            sessions: 0,
            tokens: Tokens::default(),
            cost_usd: 0.0,
            unpriced: BTreeSet::new(),
        });
        group.sessions += 1;
        group.tokens.add(&row.tokens);
        group.cost_usd += row.cost_usd;
        if let Some(branch) = &row.branch {
            group.branches.insert(branch.clone());
        }
        group.unpriced.extend(row.unpriced_models.iter().cloned());
    }

    let mut groups: Vec<Group> = by_root.into_values().collect();
    groups.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.root.cmp(&b.root))
    });
    groups
}

/// The report itself.
fn report(store: &Path, cwd: &Path, here: bool, json: bool) -> Result<ExitCode> {
    let mut rows = read(store)?;

    if here {
        // Filtering on the RESOLVED root, not on `cwd`, so running this from a
        // subdirectory reports the same worktree the rows were labeled with.
        let root = git::cwd_repo_root(cwd)
            .unwrap_or_else(|_| cwd.to_path_buf())
            .to_string_lossy()
            .into_owned();
        rows.retain(|row| row.root.as_deref() == Some(root.as_str()));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }

    if rows.is_empty() {
        println!(
            "{}",
            style::info(if here {
                "No sessions recorded for this worktree yet."
            } else {
                "No sessions recorded yet."
            })
        );
        return Ok(ExitCode::SUCCESS);
    }

    let groups = group_rows(&rows);
    let total: f64 = groups.iter().map(|g| g.cost_usd).sum();

    println!(
        "{}",
        style::header(format!(
            "{} session{} across {} root{}",
            rows.len(),
            plural(rows.len()),
            groups.len(),
            plural(groups.len())
        ))
    );
    println!();

    for group in &groups {
        // AE31: a worktree that has since been deleted still has its history
        // here, and saying the path is gone is more useful than hiding it. A
        // row that never resolved a root has no path to check, so it is only
        // named — never called deleted.
        let (label, gone) = match &group.root {
            Some(root) if Path::new(root).is_dir() => (root.as_str(), ""),
            Some(root) => (root.as_str(), " (deleted)"),
            None => ("(no worktree recorded)", ""),
        };
        println!("{}", style::ok(format!("{label}{gone}")));

        let branches = if group.branches.is_empty() {
            "-".to_string()
        } else {
            group
                .branches
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "{}",
            style::info(format!(
                "  {} session{}   {}   ${:.2}",
                group.sessions,
                plural(group.sessions),
                branches,
                group.cost_usd
            ))
        );
        println!(
            "{}",
            style::info(format!(
                "  {} in / {} out / {} cache read / {} cache write",
                compact(group.tokens.input),
                compact(group.tokens.output),
                compact(group.tokens.cache_read),
                compact(group.tokens.cache_write_5m + group.tokens.cache_write_1h)
            ))
        );
        if !group.unpriced.is_empty() {
            println!(
                "{}",
                style::warn(format!(
                    "  not priced (no rate for these models): {}",
                    group
                        .unpriced
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            );
        }
    }

    println!();
    println!("{}", style::header(format!("Total ${total:.2}")));
    println!(
        "{}",
        style::info(format!(
            "Derived from transcripts — Claude Code's own priced records where present, \
             otherwise the bundled {PRICE_TABLE_VERSION} price table."
        ))
    );
    println!(
        "{}",
        style::info(
            "A relative signal for comparing branches, not a bill: it cannot know about \
             negotiated rates or billing reconciliation."
        )
    );
    Ok(ExitCode::SUCCESS)
}

/// `""` or `"s"`, so the report reads as English at every count.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// A token count in as few characters as stays readable.
fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests;
