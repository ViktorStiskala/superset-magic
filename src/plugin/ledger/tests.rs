//! Everything here drives [`record`] and its helpers against transcript trees
//! built in tempdirs. The two properties worth stating up front, because most
//! of the file exists to hold them:
//!
//! - **One row per session id**, whether the second invocation follows the
//!   first or races it. The racing half is tested with real threads behind a
//!   barrier, not by calling `record` twice in a row: a sequential test passes
//!   while a genuine race leaks duplicates, which is exactly the trap the
//!   rename-based claim in `claim.rs` was written to document.
//! - **A re-scan never double-counts.** The incremental path adds a tail to a
//!   previous row's totals, so anything that invalidates the previous marks
//!   has to invalidate the previous totals with it.

use std::sync::Barrier;

use tempfile::TempDir;

use super::*;

const NOW: u64 = 1_788_091_200; // 2026-08-30 12:00:00 UTC, arbitrary and fixed.

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// One assistant record, shaped exactly like a real transcript line: the flat
/// `cache_creation_input_tokens` total AND the nested per-TTL split, because
/// the point of several tests below is that only the second is read.
fn assistant(model: &str, cwd: &str, branch: &str, tokens: Tokens) -> String {
    serde_json::json!({
        "type": "assistant",
        "cwd": cwd,
        "gitBranch": branch,
        "isSidechain": false,
        "message": {
            "model": model,
            "usage": {
                "input_tokens": tokens.input,
                "output_tokens": tokens.output,
                "cache_read_input_tokens": tokens.cache_read,
                "cache_creation_input_tokens": tokens.cache_write_5m + tokens.cache_write_1h,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": tokens.cache_write_5m,
                    "ephemeral_1h_input_tokens": tokens.cache_write_1h,
                },
            },
        },
    })
    .to_string()
}

/// The harness's own priced record. Cumulative over the session, which is why
/// the incremental path takes the largest rather than adding them up.
fn cost_state(session_id: &str, total: f64) -> String {
    serde_json::json!({
        "type": "cost-state",
        "sessionId": session_id,
        "totalCostUSD": total,
        "modelUsage": {},
        "hasUnknownModelCost": false,
    })
    .to_string()
}

/// A user record carrying a large tool result — the shape that makes up most
/// of a transcript's bytes and none of its usage.
fn bulky_user_turn(filler: usize) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "content": [{ "type": "tool_result", "content": "x".repeat(filler) }] },
    })
    .to_string()
}

/// A transcript tree on disk: `<root>/<slug>/<session>.jsonl` plus the sibling
/// `<session>/` directory the subagent transcripts live under.
struct Tree {
    _dir: TempDir,
    main: PathBuf,
    session_dir: PathBuf,
}

impl Tree {
    /// A tree holding just the main transcript.
    fn new(session_id: &str, lines: &[String]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let slug = dir.path().join("-Users-someone-project");
        fs::create_dir_all(&slug).unwrap();
        let main = slug.join(format!("{session_id}.jsonl"));
        fs::write(&main, joined(lines)).unwrap();
        Self {
            session_dir: slug.join(session_id),
            _dir: dir,
            main,
        }
    }

    /// Add a subagent transcript at `rel` beneath the session directory.
    fn subagent(&self, rel: &str, lines: &[String]) -> PathBuf {
        let path = self.session_dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, joined(lines)).unwrap();
        path
    }

    /// Append to the main transcript, the way a resumed session would.
    fn append_main(&self, lines: &[String]) {
        let mut file = OpenOptions::new().append(true).open(&self.main).unwrap();
        file.write_all(joined(lines).as_bytes()).unwrap();
    }
}

/// Newline-terminated JSONL.
fn joined(lines: &[String]) -> String {
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    body
}

/// The ingest for `tree`, with no repository to resolve against — the tempdir
/// `cwd` values in these fixtures are outside any git repository, so they
/// normalize to themselves and the labels stay deterministic.
fn ingest<'a>(session_id: &'a str, tree: &'a Tree) -> Ingest<'a> {
    Ingest {
        session_id,
        transcript: &tree.main,
        cwd: None,
        repo_root: None,
        now: NOW,
    }
}

/// A store directory to record into.
fn store() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// The single row `record` wrote, or a panic naming what it did instead.
fn written(outcome: Recorded) -> Row {
    match outcome {
        Recorded::Written(row) => *row,
        other => panic!("expected a written row, got {other:?}"),
    }
}

// ── Pricing ───────────────────────────────────────────────────────────────────

/// The understatement this whole feature was nearly shipped with. A 1-hour
/// cache write costs 2x base input and a 5-minute write 1.25x, so reading the
/// flat `cache_creation_input_tokens` total — which collapses them — prices a
/// mixed session too low. The two figures below must not be equal.
#[test]
fn the_two_cache_ttls_price_differently() {
    let split = Tokens {
        cache_write_5m: 1_000_000,
        cache_write_1h: 1_000_000,
        ..Tokens::default()
    };
    let flat = Tokens {
        cache_write_5m: 2_000_000,
        ..Tokens::default()
    };

    // Sonnet 5 is $2/MTok input: 1.25x = $2.50, 2x = $4.00.
    let correct = price_tokens("claude-sonnet-5", &split).unwrap();
    let understated = price_tokens("claude-sonnet-5", &flat).unwrap();

    assert!((correct - 6.50).abs() < 1e-9, "got {correct}");
    assert!((understated - 5.00).abs() < 1e-9, "got {understated}");
    assert!(correct > understated);
}

/// The same trap, one level up: a real transcript line carries BOTH the flat
/// `cache_creation_input_tokens` total and the nested per-TTL split, and only
/// the split may reach the row. Reading the flat total would land every cache
/// write in one bucket and price the session low.
#[test]
fn a_transcript_with_both_ttls_lands_them_in_separate_buckets() {
    let store = store();
    let tree = Tree::new(
        "s-ttl",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                cache_write_5m: 1_000_000,
                cache_write_1h: 1_000_000,
                ..Tokens::default()
            },
        )],
    );

    let row = written(record(store.path(), &ingest("s-ttl", &tree)).unwrap());
    assert_eq!(row.tokens.cache_write_5m, 1_000_000);
    assert_eq!(row.tokens.cache_write_1h, 1_000_000);
    // $2.50 for the 5-minute half plus $4.00 for the 1-hour half. Collapsing
    // them into one bucket at the 5-minute rate would give $5.00.
    assert!((row.cost_usd - 6.50).abs() < 1e-9, "{row:?}");
}

/// An older transcript that predates the nested split carries only the flat
/// total. It still has to be counted — attributed to the 5-minute TTL, which
/// under-states rather than over-states.
#[test]
fn a_transcript_without_the_nested_split_falls_back_to_the_flat_total() {
    let store = store();
    let line = serde_json::json!({
        "type": "assistant",
        "cwd": "/tmp/p",
        "gitBranch": "main",
        "message": {
            "model": "claude-sonnet-5",
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 1_000_000,
            },
        },
    })
    .to_string();
    let tree = Tree::new("s-flat", &[line]);

    let row = written(record(store.path(), &ingest("s-flat", &tree)).unwrap());
    assert_eq!(row.tokens.cache_write_5m, 1_000_000);
    assert_eq!(row.tokens.cache_write_1h, 0);
    assert!((row.cost_usd - 2.50).abs() < 1e-9, "{row:?}");
}

/// A dated model id prices as its family, and the longest matching prefix
/// wins so a family id never shadows a more specific one.
#[test]
fn model_ids_match_by_longest_prefix() {
    assert_eq!(price_for("claude-haiku-4-5-20251001"), Some((1.0, 5.0)));
    assert_eq!(price_for("claude-opus-4-8"), Some((5.0, 25.0)));
    assert_eq!(price_for("claude-sonnet-5"), Some((2.0, 10.0)));
    assert_eq!(price_for("gpt-imaginary"), None);
}

/// A model the table cannot price contributes its tokens and no dollars, and
/// says so in the row rather than quietly pricing it at zero.
#[test]
fn an_unknown_model_is_declared_unpriced() {
    let store = store();
    let tree = Tree::new(
        "s-unknown",
        &[assistant(
            "claude-from-the-future-9",
            "/tmp/proj",
            "main",
            Tokens {
                input: 1_000,
                output: 500,
                ..Tokens::default()
            },
        )],
    );

    let row = written(record(store.path(), &ingest("s-unknown", &tree)).unwrap());
    assert_eq!(row.unpriced_models, vec!["claude-from-the-future-9"]);
    assert_eq!(row.tokens.input, 1_000);
    assert_eq!(row.cost_usd, 0.0);
    // Nothing was priced, so no table version is claimed either.
    assert_eq!(row.price_table, None);
}

/// A row written by some other build carrying a price-table version this
/// binary has never heard of is reported exactly as written. Recorded figures
/// are never re-priced — that is the whole point of snapshotting a version
/// with the row.
#[test]
fn an_unrecognized_price_table_version_is_reported_verbatim() {
    let store = store();
    fs::create_dir_all(store.path()).unwrap();
    let row = serde_json::json!({
        "session_id": "s-future",
        "at": "2027-01-01T00:00:00Z",
        "ts": 1_800_000_000u64,
        "root": "/tmp/somewhere",
        "files": 1,
        "bytes": 10,
        "tokens": { "input": 1, "output": 1, "cache_read": 0,
                    "cache_write_5m": 0, "cache_write_1h": 0 },
        "cost_usd": 41.5,
        "basis": "table",
        "main_table_usd": 41.5,
        "sub_table_usd": 0.0,
        "price_table": "2099-12-31",
    });
    fs::write(ledger_path(store.path()), format!("{row}\n")).unwrap();

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].price_table.as_deref(), Some("2099-12-31"));
    assert_eq!(rows[0].cost_usd, 41.5);
    // And the report renders it without complaint.
    assert_eq!(
        report(store.path(), Path::new("/tmp"), false, false).unwrap(),
        ExitCode::SUCCESS
    );
}

// ── The scan ──────────────────────────────────────────────────────────────────

/// The subagent transcripts hold most of a delegating session's usage, and
/// nothing links them to their parent but where they sit on disk — so the tree
/// is walked, at every depth, rather than the main file being read alone.
#[test]
fn the_whole_tree_is_walked_including_nested_subagents() {
    let store = store();
    let one = Tokens {
        output: 1_000,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-tree",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", one)],
    );
    tree.subagent(
        "subagents/agent-a.jsonl",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", one)],
    );
    tree.subagent(
        "subagents/workflows/wf-1/agent-b.jsonl",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", one)],
    );

    let row = written(record(store.path(), &ingest("s-tree", &tree)).unwrap());
    assert_eq!(row.files, 3);
    assert_eq!(row.tokens.output, 3_000);
}

/// The harness's own cumulative figure is believed for the main thread; the
/// table prices the subagents, which cost-state never mentions. The main
/// thread's table price is computed but deliberately not added — doing so
/// would count the main thread twice.
#[test]
fn the_harness_figure_covers_the_main_thread_and_the_table_covers_subagents() {
    let store = store();
    let main_tokens = Tokens {
        output: 1_000_000,
        ..Tokens::default()
    };
    let sub_tokens = Tokens {
        output: 100_000,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-mixed",
        &[
            assistant("claude-sonnet-5", "/tmp/p", "main", main_tokens),
            cost_state("s-mixed", 4.25),
        ],
    );
    tree.subagent(
        "subagents/agent-a.jsonl",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", sub_tokens)],
    );

    let row = written(record(store.path(), &ingest("s-mixed", &tree)).unwrap());
    assert_eq!(row.harness_cost_usd, Some(4.25));
    // Subagent: 100k output at $10/MTok = $1.00.
    assert!((row.sub_table_usd - 1.00).abs() < 1e-9, "{row:?}");
    assert!((row.main_table_usd - 10.00).abs() < 1e-9, "{row:?}");
    // …and the main thread's own $10 is NOT in the total, the harness's is.
    assert!((row.cost_usd - 5.25).abs() < 1e-9, "{row:?}");
    assert_eq!(row.basis, Basis::Mixed);
}

/// With no cost-state record anywhere — a subagent-only tree, or a transcript
/// older than the harness's priced records — the table prices everything.
#[test]
fn without_a_harness_figure_the_table_prices_the_whole_tree() {
    let store = store();
    let tokens = Tokens {
        output: 1_000_000,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-table",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", tokens)],
    );

    let row = written(record(store.path(), &ingest("s-table", &tree)).unwrap());
    assert_eq!(row.basis, Basis::Table);
    assert!((row.cost_usd - 10.00).abs() < 1e-9, "{row:?}");
    assert_eq!(row.price_table.as_deref(), Some(PRICE_TABLE_VERSION));
}

/// A transcript is a log, not a validated document: a truncated write or a
/// line from a newer harness must cost us that line and nothing else.
#[test]
fn malformed_lines_are_skipped_and_the_good_ones_still_count() {
    let store = store();
    let good = assistant(
        "claude-sonnet-5",
        "/tmp/p",
        "main",
        Tokens {
            output: 500,
            ..Tokens::default()
        },
    );
    let tree = Tree::new(
        "s-mixedlines",
        &[
            "{ this is not json at all".to_string(),
            good.clone(),
            "{\"type\":\"assistant\",\"message\":{\"usage\":".to_string(),
            good.clone(),
            String::new(),
        ],
    );

    let row = written(record(store.path(), &ingest("s-mixedlines", &tree)).unwrap());
    assert_eq!(row.tokens.output, 1_000);
}

/// Nothing to scan is not an error — a session can end before the harness has
/// written anything, and the hook must not turn that into a failure.
#[test]
fn a_missing_transcript_tree_records_nothing() {
    let store = store();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nowhere.jsonl");
    let outcome = record(
        store.path(),
        &Ingest {
            session_id: "s-gone",
            transcript: &missing,
            cwd: None,
            repo_root: None,
            now: NOW,
        },
    )
    .unwrap();

    assert_eq!(outcome, Recorded::NoTranscript);
    assert!(read(store.path()).unwrap().is_empty());
}

// ── Idempotency (AE11) ────────────────────────────────────────────────────────

/// AE11. `SessionEnd` fires twice for one session id — the ordinary case, not
/// an edge one, since `/clear` produces two endings inside a single CLI
/// process. The second invocation must find nothing to do.
#[test]
fn two_sequential_records_leave_exactly_one_row() {
    let store = store();
    let tree = Tree::new(
        "s-twice",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 1_000,
                ..Tokens::default()
            },
        )],
    );

    let first = record(store.path(), &ingest("s-twice", &tree)).unwrap();
    assert!(matches!(first, Recorded::Written(_)));

    let second = record(store.path(), &ingest("s-twice", &tree)).unwrap();
    assert_eq!(second, Recorded::Unchanged);

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tokens.output, 1_000);
}

/// AE33. The same id, but the two invocations race. A sequential test passes
/// while a real race leaks — `unlink` measured five winners out of eight
/// racers on this platform — so this one uses a barrier and real threads.
#[test]
fn concurrent_records_leave_one_row_and_readable_offsets() {
    let store = store();
    let tree = Tree::new(
        "s-race",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 2_000,
                ..Tokens::default()
            },
        )],
    );

    const RACERS: usize = 8;
    let barrier = Barrier::new(RACERS);
    std::thread::scope(|scope| {
        for _ in 0..RACERS {
            scope.spawn(|| {
                barrier.wait();
                record(store.path(), &ingest("s-race", &tree)).unwrap();
            });
        }
    });

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1, "one row per session id, got {rows:?}");
    assert_eq!(rows[0].tokens.output, 2_000);

    // The offsets store must still parse. A torn write here would silently
    // reset every session's mark on the next run.
    let offsets = read_offsets(store.path());
    assert_eq!(offsets.len(), 1);
    let mark = offsets.values().next().unwrap();
    assert_eq!(mark.offset, mark.size);
}

/// Two sessions ending at once must each get their own row — the lock
/// serializes the commit, it does not deduplicate across ids.
#[test]
fn concurrent_records_for_different_ids_leave_a_row_each() {
    let store = store();
    let line = assistant(
        "claude-sonnet-5",
        "/tmp/p",
        "main",
        Tokens {
            output: 10,
            ..Tokens::default()
        },
    );
    let trees: Vec<(String, Tree)> = (0..4)
        .map(|n| {
            let id = format!("s-multi-{n}");
            let tree = Tree::new(&id, std::slice::from_ref(&line));
            (id, tree)
        })
        .collect();

    let barrier = Barrier::new(trees.len());
    std::thread::scope(|scope| {
        for (id, tree) in &trees {
            scope.spawn(|| {
                barrier.wait();
                record(store.path(), &ingest(id, tree)).unwrap();
            });
        }
    });

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 4, "{rows:?}");
}

// ── Incremental scanning and the rotation guard ───────────────────────────────

/// A resumed session ends twice and its transcript has grown in between. The
/// second scan reads only the tail and adds it to the row that is already
/// there — the row is refreshed in place, never duplicated, and the earlier
/// lines are not counted twice.
#[test]
fn a_grown_transcript_is_tailed_rather_than_re_read() {
    let store = store();
    let one = Tokens {
        output: 1_000,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-grow",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", one)],
    );

    let first = written(record(store.path(), &ingest("s-grow", &tree)).unwrap());
    assert_eq!(first.tokens.output, 1_000);

    tree.append_main(&[assistant("claude-sonnet-5", "/tmp/p", "main", one)]);
    let second = written(record(store.path(), &ingest("s-grow", &tree)).unwrap());

    assert_eq!(second.tokens.output, 2_000);
    assert_eq!(read(store.path()).unwrap().len(), 1);
}

/// A transcript that shrank below its stored offset — rotated, truncated, or
/// replaced — makes the mark meaningless. The fix is a full re-read of the
/// whole tree WITHOUT the previous row's totals underneath it; keeping them
/// would double-count everything the re-read finds again.
#[test]
fn a_rotated_transcript_forces_a_full_rescan_without_double_counting() {
    let store = store();
    let big = Tokens {
        output: 5_000,
        ..Tokens::default()
    };
    let small = Tokens {
        output: 7,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-rotate",
        &[
            assistant("claude-sonnet-5", "/tmp/p", "main", big),
            assistant("claude-sonnet-5", "/tmp/p", "main", big),
        ],
    );

    let first = written(record(store.path(), &ingest("s-rotate", &tree)).unwrap());
    assert_eq!(first.tokens.output, 10_000);

    // Rotation: a shorter file at the same path. Its size is now below the
    // stored offset, which is the signal.
    fs::write(
        &tree.main,
        joined(&[assistant("claude-sonnet-5", "/tmp/p", "main", small)]),
    )
    .unwrap();

    let second = written(record(store.path(), &ingest("s-rotate", &tree)).unwrap());
    assert_eq!(
        second.tokens.output, 7,
        "the rescan must replace the old totals, not add to them"
    );
    assert_eq!(read(store.path()).unwrap().len(), 1);
}

/// A subagent transcript that appears after the first scan is new, not
/// rotated: it has contributed nothing yet, so it is simply read from the
/// start and added, and the files already tailed keep their marks.
#[test]
fn a_subagent_appearing_later_is_added_not_treated_as_rotation() {
    let store = store();
    let one = Tokens {
        output: 1_000,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-newsub",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", one)],
    );
    written(record(store.path(), &ingest("s-newsub", &tree)).unwrap());

    tree.subagent(
        "subagents/agent-late.jsonl",
        &[assistant("claude-sonnet-5", "/tmp/p", "main", one)],
    );
    let second = written(record(store.path(), &ingest("s-newsub", &tree)).unwrap());

    assert_eq!(second.files, 2);
    assert_eq!(second.tokens.output, 2_000);
}

/// cost-state records are cumulative, so a tail that carries a newer one
/// replaces the previous figure rather than adding to it. A tail with none at
/// all keeps what the row already had.
#[test]
fn the_cumulative_harness_figure_is_replaced_not_summed() {
    let store = store();
    let tree = Tree::new("s-cum", &[cost_state("s-cum", 3.00)]);
    let first = written(record(store.path(), &ingest("s-cum", &tree)).unwrap());
    assert_eq!(first.harness_cost_usd, Some(3.00));

    tree.append_main(&[cost_state("s-cum", 7.50)]);
    let second = written(record(store.path(), &ingest("s-cum", &tree)).unwrap());
    assert_eq!(second.harness_cost_usd, Some(7.50));

    tree.append_main(&[assistant(
        "claude-sonnet-5",
        "/tmp/p",
        "main",
        Tokens {
            output: 1,
            ..Tokens::default()
        },
    )]);
    let third = written(record(store.path(), &ingest("s-cum", &tree)).unwrap());
    assert_eq!(third.harness_cost_usd, Some(7.50));
}

/// The interleaving the equal-shape check alone cannot catch: an incremental
/// row whose base row was replaced by somebody else while this invocation was
/// still scanning. Its totals rest on a ledger state that no longer exists, so
/// it must leave the newer row alone rather than overwrite it with a stale
/// sum. Driven through [`commit`] directly, since the window it closes is
/// between the scan and the lock and cannot be reached from `record`.
#[test]
fn an_incremental_row_built_on_a_replaced_base_is_dropped() {
    let store = store();
    let tree = Tree::new(
        "s-stale",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 1_000,
                ..Tokens::default()
            },
        )],
    );
    let landed = written(record(store.path(), &ingest("s-stale", &tree)).unwrap());

    // What this invocation THOUGHT the ledger held when it started scanning —
    // a row somebody has since replaced.
    let mut stale_base = landed.clone();
    stale_base.tokens.output = 999;

    // Its own row describes a different tree shape, so the equal-shape check
    // does not catch it; only the base comparison does.
    let mut ours = landed.clone();
    ours.bytes += 1;
    ours.tokens.output = 4_242;

    let scan = Scan::default();
    let outcome = commit(store.path(), ours, Some(stale_base), &scan).unwrap();
    assert_eq!(outcome, Recorded::Unchanged);

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tokens.output, 1_000, "the landed row is untouched");
}

/// The same shape, but the base IS still what the ledger holds — the ordinary
/// incremental refresh, which must go through.
#[test]
fn an_incremental_row_built_on_the_current_base_is_written() {
    let store = store();
    let tree = Tree::new(
        "s-fresh",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 1_000,
                ..Tokens::default()
            },
        )],
    );
    let landed = written(record(store.path(), &ingest("s-fresh", &tree)).unwrap());

    let mut ours = landed.clone();
    ours.bytes += 1;
    ours.tokens.output = 4_242;

    let outcome = commit(store.path(), ours, Some(landed), &Scan::default()).unwrap();
    assert!(matches!(outcome, Recorded::Written(_)));

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tokens.output, 4_242);
}

// ── Labels ────────────────────────────────────────────────────────────────────

/// `cwd` is per-line and not worktree-stable, so the busiest resolved root is
/// the label. A session that genuinely worked in two roots names the second
/// one beside it rather than having it silently folded into the first.
#[test]
fn a_session_spanning_two_roots_names_both() {
    let store = store();
    let one = Tokens {
        output: 10,
        ..Tokens::default()
    };
    let tree = Tree::new(
        "s-roots",
        &[
            assistant("claude-sonnet-5", "/tmp/root-a", "feature", one),
            assistant("claude-sonnet-5", "/tmp/root-a", "feature", one),
            assistant("claude-sonnet-5", "/tmp/root-b", "feature", one),
        ],
    );

    let row = written(record(store.path(), &ingest("s-roots", &tree)).unwrap());
    assert_eq!(row.root.as_deref(), Some("/tmp/root-a"));
    assert_eq!(row.also_roots, vec!["/tmp/root-b".to_string()]);
    assert_eq!(row.branch.as_deref(), Some("feature"));
}

/// With no `cwd` to normalize — a transcript with no assistant turns at all —
/// the caller's own resolved root stands in, so the row is still groupable.
#[test]
fn the_callers_root_labels_a_transcript_with_no_cwd() {
    let store = store();
    let tree = Tree::new("s-nocwd", &[cost_state("s-nocwd", 1.0)]);
    let row = written(
        record(
            store.path(),
            &Ingest {
                session_id: "s-nocwd",
                transcript: &tree.main,
                cwd: Some(Path::new("/tmp/wt")),
                repo_root: Some(Path::new("/tmp/wt")),
                now: NOW,
            },
        )
        .unwrap(),
    );
    assert_eq!(row.root.as_deref(), Some("/tmp/wt"));
}

// ── The store ─────────────────────────────────────────────────────────────────

/// The table used to price a row is written out beside the ledger, once per
/// version, so a figure recorded today can still be explained after prices
/// move.
#[test]
fn the_price_table_is_snapshotted_at_ingest() {
    let store = store();
    let tree = Tree::new(
        "s-snap",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 10,
                ..Tokens::default()
            },
        )],
    );
    written(record(store.path(), &ingest("s-snap", &tree)).unwrap());

    let snapshot = store
        .path()
        .join(PRICES_DIR_NAME)
        .join(format!("{PRICE_TABLE_VERSION}.json"));
    let body: Value = serde_json::from_str(&fs::read_to_string(snapshot).unwrap()).unwrap();
    assert_eq!(body["version"], PRICE_TABLE_VERSION);
    assert_eq!(body["cache_write_1h_multiplier"], 2.0);
    assert!(body["models"].as_array().unwrap().len() >= 9);
}

/// One unreadable line must not hide the rest of the history, and an absent
/// ledger is an empty history rather than an error.
#[test]
fn reading_tolerates_a_bad_line_and_a_missing_file() {
    let store = store();
    assert!(read(store.path()).unwrap().is_empty());

    let tree = Tree::new(
        "s-tolerant",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 1,
                ..Tokens::default()
            },
        )],
    );
    written(record(store.path(), &ingest("s-tolerant", &tree)).unwrap());

    let path = ledger_path(store.path());
    let good = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("not a row at all\n{good}")).unwrap();

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "s-tolerant");
}

// ── The verb ──────────────────────────────────────────────────────────────────

/// AE12. The CLI was killed, no `SessionEnd` ever fired, and the row has to be
/// reconstructed from the transcript that is nonetheless complete on disk.
#[test]
fn backfill_records_a_session_whose_session_end_never_ran() {
    let store = store();
    let tree = Tree::new(
        "s-killed",
        &[assistant(
            "claude-sonnet-5",
            "/tmp/p",
            "main",
            Tokens {
                output: 1_000_000,
                ..Tokens::default()
            },
        )],
    );
    assert!(read(store.path()).unwrap().is_empty());

    let code = run_backfill(store.path(), &tree.main.to_string_lossy()).unwrap();
    assert_eq!(code, ExitCode::SUCCESS);

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "s-killed");
    assert!((rows[0].cost_usd - 10.00).abs() < 1e-9);

    // Backfilling the same session again goes through the same idempotent
    // path, so it is safe rather than duplicating the row.
    run_backfill(store.path(), &tree.main.to_string_lossy()).unwrap();
    assert_eq!(read(store.path()).unwrap().len(), 1);
}

/// A backfill reference naming nothing at all reports it and fails, rather
/// than silently recording an empty session.
#[test]
fn backfill_reports_a_reference_it_cannot_resolve() {
    let store = store();
    let code = run_backfill(store.path(), "no-such-session-id").unwrap();
    assert_ne!(code, ExitCode::SUCCESS);
    assert!(read(store.path()).unwrap().is_empty());
}

/// AE31. The worktree these sessions ran in has been deleted. The rows are
/// machine-level, so they survive it and still group under the root that is
/// gone — which is the entire reason the ledger does not live inside a
/// worktree.
#[test]
fn rows_for_a_deleted_worktree_root_still_report() {
    let store = store();
    let gone = tempfile::tempdir().unwrap();
    let gone_root = gone.path().to_string_lossy().into_owned();

    let one = Tokens {
        output: 1_000,
        ..Tokens::default()
    };
    for n in 0..3 {
        let id = format!("s-deleted-{n}");
        let tree = Tree::new(
            &id,
            &[assistant("claude-sonnet-5", &gone_root, "feat", one)],
        );
        written(record(store.path(), &ingest(&id, &tree)).unwrap());
    }

    // The worktree goes away. Nothing about the ledger moves with it.
    drop(gone);
    assert!(!Path::new(&gone_root).is_dir());

    let rows = read(store.path()).unwrap();
    assert_eq!(rows.len(), 3);

    let groups = group_rows(&rows);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].root.as_deref(), Some(gone_root.as_str()));
    assert_eq!(groups[0].sessions, 3);
    assert_eq!(
        report(store.path(), Path::new("/tmp"), false, false).unwrap(),
        ExitCode::SUCCESS
    );
}

/// Sessions from several worktrees are reported together by default, busiest
/// first — the ledger is machine-level and reporting only the current
/// worktree would hide most of it.
#[test]
fn rows_are_grouped_by_root_costliest_first() {
    let rows = vec![
        row_for("a", Some("/tmp/cheap"), 1.0),
        row_for("b", Some("/tmp/dear"), 10.0),
        row_for("c", Some("/tmp/dear"), 5.0),
    ];
    let groups = group_rows(&rows);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].root.as_deref(), Some("/tmp/dear"));
    assert_eq!(groups[0].sessions, 2);
    assert!((groups[0].cost_usd - 15.0).abs() < 1e-9);
    assert_eq!(groups[1].root.as_deref(), Some("/tmp/cheap"));
}

/// A row with no root at all still appears — grouped on its own, and never
/// reported as a deleted worktree, since there is no path to have been
/// deleted.
#[test]
fn a_row_with_no_root_is_still_reported() {
    let groups = group_rows(&[row_for("a", None, 2.0)]);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].root, None);
    assert_eq!(groups[0].sessions, 1);
}

/// An empty ledger is a normal state — nothing has ended yet — and reports
/// cleanly rather than failing.
#[test]
fn an_empty_ledger_reports_successfully() {
    let store = store();
    assert_eq!(
        report(store.path(), Path::new("/tmp"), false, false).unwrap(),
        ExitCode::SUCCESS
    );
    assert_eq!(
        report(store.path(), Path::new("/tmp"), false, true).unwrap(),
        ExitCode::SUCCESS
    );
}

/// A bare row for grouping tests, with only the fields those tests read.
fn row_for(session_id: &str, root: Option<&str>, cost_usd: f64) -> Row {
    Row {
        session_id: session_id.to_string(),
        at: format_rfc3339(NOW),
        ts: NOW,
        root: root.map(str::to_string),
        also_roots: Vec::new(),
        branch: None,
        files: 1,
        bytes: 1,
        tokens: Tokens::default(),
        cost_usd,
        basis: Basis::Table,
        harness_cost_usd: None,
        main_table_usd: cost_usd,
        sub_table_usd: 0.0,
        price_table: Some(PRICE_TABLE_VERSION.to_string()),
        unpriced_models: Vec::new(),
    }
}

/// Token counts are rendered short enough to scan a column of them.
#[test]
fn token_counts_render_compactly() {
    assert_eq!(compact(0), "0");
    assert_eq!(compact(999), "999");
    assert_eq!(compact(1_500), "1.5k");
    assert_eq!(compact(2_300_000), "2.3M");
}

// ── Budget ────────────────────────────────────────────────────────────────────

/// A transcript tree shaped like a real one: 50 main-thread turns, each a
/// bulky tool-result line paired with a usage line, plus 40 subagent
/// transcripts of the same shape (41 files, matching a session with real
/// subagent activity). Shared by the two budget tests below so "realistic"
/// means the same tree for both of them.
fn realistic_tree(session_id: &str) -> Tree {
    let usage_line = assistant(
        "claude-sonnet-5",
        "/tmp/p",
        "main",
        Tokens {
            output: 100,
            ..Tokens::default()
        },
    );
    let mut lines: Vec<String> = Vec::new();
    for _ in 0..50 {
        lines.push(bulky_user_turn(20_000));
        lines.push(usage_line.clone());
    }

    let tree = Tree::new(session_id, &lines);
    for n in 0..40 {
        tree.subagent(&format!("subagents/agent-{n}.jsonl"), &lines);
    }
    tree
}

/// The scan has to fit inside the ~1.15 s a hook body actually gets — the
/// nominal timeout is 1500 ms, but spawn and start-up have already spent the
/// difference by the time the handler runs.
///
/// The tree here is deliberately shaped like a real one: most of the bytes are
/// tool-result text on `user` lines, which the two cheap pre-checks in
/// [`read_line`] skip without parsing. A regression that parsed every line
/// would show up here long before it reached anyone's session. The bound is
/// generous on purpose — this guards against an accidental full-parse or a
/// quadratic walk, not against a slow CI runner.
#[test]
fn scanning_a_realistic_tree_fits_the_hook_budget() {
    let tree = realistic_tree("s-budget");

    let files = transcript_tree(&tree.main);
    assert_eq!(files.len(), 41);

    let started = std::time::Instant::now();
    let scan = scan_tree(&files, &Offsets::default());
    let elapsed = started.elapsed();

    assert_eq!(scan.main_tokens.output + scan.sub_tokens.output, 41 * 5_000);
    assert!(
        elapsed < std::time::Duration::from_millis(1_150),
        "scanned {} bytes in {elapsed:?}, past the ~1.15 s a hook body gets",
        scan.bytes
    );
}

/// The same budget, against the function the `SessionEnd` hook actually
/// calls. `scan_tree` above is only part of what the hook pays: `record`
/// additionally resolves every distinct `cwd` it saw through one `git`
/// subprocess each (`resolve_roots`, uncached here the same way the standard
/// `ingest` fixture leaves it uncached throughout this file), and reads the
/// ledger twice — once to find the previous row, again inside the locked
/// commit. A budget asserted only against `scan_tree` would miss a regression
/// in either of those.
#[test]
fn recording_a_realistic_session_fits_the_hook_budget() {
    let store = store();
    let tree = realistic_tree("s-record-budget");

    let started = std::time::Instant::now();
    let outcome = record(store.path(), &ingest("s-record-budget", &tree)).unwrap();
    let elapsed = started.elapsed();

    let row = written(outcome);
    assert_eq!(row.tokens.output, 41 * 5_000);
    assert!(
        elapsed < std::time::Duration::from_millis(1_150),
        "recorded in {elapsed:?}, past the ~1.15 s a hook body gets",
    );
}

/// The measurement the budget above stands in for, against a real transcript
/// corpus rather than a synthetic one. Ignored by default because it needs a
/// tree that only exists on a machine that has actually used Claude Code:
///
/// ```text
/// SS_MAGIC_LEDGER_BENCH_TREE=~/.claude/projects/<slug>/<session>.jsonl \
///   cargo test --release ledger::tests::scanning_a_real_tree -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a real transcript tree, named by SS_MAGIC_LEDGER_BENCH_TREE"]
fn scanning_a_real_tree_reports_its_time() {
    let Ok(path) = std::env::var("SS_MAGIC_LEDGER_BENCH_TREE") else {
        panic!("set SS_MAGIC_LEDGER_BENCH_TREE to a transcript .jsonl");
    };
    let files = transcript_tree(Path::new(&path));
    let started = std::time::Instant::now();
    let scan = scan_tree(&files, &Offsets::default());
    let elapsed = started.elapsed();
    println!(
        "{} files, {:.1} MiB, {elapsed:?} — budget ~1.15 s",
        scan.files,
        scan.bytes as f64 / (1024.0 * 1024.0),
    );
}
