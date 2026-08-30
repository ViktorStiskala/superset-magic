//! The verbs end to end, against a real git repository in a tempdir.
//!
//! Everything drives [`run_core`], which takes the working directory and the
//! clock as arguments, so no test spawns the binary, reads a real stdin, or
//! depends on the wall clock. The one thing that IS real is git: the state
//! tree's ignore rule (R63) has to be answered by git itself, which is what
//! `scratchpad::ensure` asks, so the fixtures commit a `.gitignore` the way a
//! repository that ran `ss-magic init` would have.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use serde_json::{json, Value};
use tempfile::TempDir;

use super::*;
use crate::tests::support::{git_run, init_main_repo};

/// 2026-08-30 12:00:00 UTC — inside the `2026-08` the file names carry.
const NOW: u64 = 1_788_091_200;

/// The stem every fixture uses.
const STEM: &str = "2026-08-ship-it";

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// A repository whose state tree git already ignores — the ordinary case,
/// after `ss-magic init` or `plugin enable`.
fn ignored_repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    fs::write(
        dir.path().join(".gitignore"),
        "target/\n.superset/.magic/\n",
    )
    .unwrap();
    git_run(&["add", ".gitignore"], dir.path());
    git_run(&["commit", "-q", "-m", "gitignore"], dir.path());
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

/// The same repository with `checklist init ship-it` already run.
fn initialized() -> (TempDir, PathBuf) {
    let (dir, root) = ignored_repo();
    assert_eq!(init(&root, "ship-it"), ExitCode::SUCCESS);
    (dir, root)
}

fn init(root: &Path, slug: &str) -> ExitCode {
    run_core(
        root,
        &Sub::Init {
            slug: slug.to_string(),
        },
        NOW,
    )
    .unwrap()
}

fn add_item_verb(root: &Path, section: &str, id: &str, title: &str) -> ExitCode {
    run_core(
        root,
        &Sub::AddItem {
            section: section.to_string(),
            id: id.to_string(),
            title: Some(title.to_string()),
        },
        NOW,
    )
    .unwrap()
}

fn add_entry_verb(root: &Path, id: &str, summary: &str) -> ExitCode {
    run_core(
        root,
        &Sub::AddEntry {
            id: id.to_string(),
            summary: Some(summary.to_string()),
        },
        NOW,
    )
    .unwrap()
}

/// `set` with a value; `None` is the caller having written the literal `null`.
fn set(root: &Path, id: &str, key: &str, value: Option<&str>) -> ExitCode {
    run_core(
        root,
        &Sub::Set {
            id: id.to_string(),
            key: key.to_string(),
            value: value.map(str::to_string),
            from_stdin: false,
        },
        NOW,
    )
    .unwrap()
}

fn done(root: &Path, id: &str) -> ExitCode {
    run_core(root, &Sub::Done { id: id.to_string() }, NOW).unwrap()
}

fn checklist_path(root: &Path) -> PathBuf {
    root.join(format!("{ACTIONS_REL}/{STEM}{CHECKLIST_SUFFIX}"))
}

fn doc_at(root: &Path) -> Document {
    read_document(&checklist_path(root)).unwrap().unwrap()
}

/// The file as raw JSON, for the checks that are about what is on the wire
/// rather than what the typed model says.
fn raw_at(root: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(checklist_path(root)).unwrap()).unwrap()
}

fn write_raw(root: &Path, value: &Value) {
    fs::write(
        checklist_path(root),
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}

/// One item, filled in far enough that the document validates clean.
fn complete_item(root: &Path, section: &str, id: &str) {
    assert_eq!(
        add_item_verb(root, section, id, "check the thing"),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(root, id, "steps.-", Some("run the check")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(root, id, "expected", Some("it passes")),
        ExitCode::SUCCESS
    );
}

// ── AE75: init, and the pointer it records ────────────────────────────────────

/// AE75. `init` on a branch with no checklist yet: the pointer lands inside the
/// state root, records the intended path, and is a plain file rather than a
/// symlink. Nothing is written into `.scratchpad/`.
#[test]
fn init_records_the_pointer_inside_the_state_root_and_nothing_in_scratchpad() {
    let (_dir, root) = ignored_repo();
    assert!(!checklist_path(&root).exists());

    assert_eq!(init(&root, "ship-it"), ExitCode::SUCCESS);

    let pointer = pointer_path(&root);
    assert_eq!(
        pointer,
        root.join(".superset/.magic/checklist.json"),
        "the pointer's location is what SessionStart already tells every model"
    );
    let meta = fs::symlink_metadata(&pointer).unwrap();
    assert!(meta.is_file(), "the pointer is a manifest file");
    assert!(
        !meta.is_symlink(),
        "ss-magic creates no symlinks — sync skips them and pack never follows them"
    );

    let recorded: Pointer = serde_json::from_str(&fs::read_to_string(&pointer).unwrap()).unwrap();
    assert_eq!(
        recorded.path,
        format!("{ACTIONS_REL}/{STEM}{CHECKLIST_SUFFIX}")
    );
    assert_eq!(recorded.slug, STEM);

    assert!(
        !root.join(".scratchpad").exists(),
        "`.scratchpad/` belongs to other tooling and is never written to"
    );
}

/// AE75, second half. A pointer whose target does not exist is still a pointer:
/// resolving it stats nothing, so U28's classifier can recognise the path
/// before the document is ever written.
#[test]
fn a_dangling_pointer_still_resolves_to_a_checklist_path() {
    let (_dir, root) = initialized();
    let target = checklist_path(&root);

    fs::remove_file(&target).unwrap();
    assert!(!target.exists());

    let resolved = pointer_target(&root).expect("a dangling pointer still resolves");
    assert_eq!(resolved, target);
    assert!(
        matches_convention(&root, &resolved),
        "and the naming convention recognises it without a stat too"
    );
}

/// The pointer is data on disk, so a path in it that would leave the
/// repository is refused rather than followed into a write anywhere on the
/// filesystem. Checked lexically, because the target legitimately may not
/// exist yet and so cannot be resolved.
#[test]
fn a_pointer_that_escapes_the_repository_is_refused() {
    let (_dir, root) = initialized();

    for escape in ["../elsewhere/x.checklist.json", "/etc/passwd", ""] {
        fs::write(
            pointer_path(&root),
            serde_json::to_string(&json!({
                "path": escape,
                "slug": STEM,
                "recorded_at": "2026-08-30T12:00:00Z",
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(
            pointer_target(&root).is_none(),
            "`{escape}` must not resolve to a writable target"
        );
    }
}

/// `init` twice on one branch adopts the document that is already there rather
/// than overwriting it, and only re-records the pointer. That is also how a
/// repository holding several checklists says which one is live.
#[test]
fn init_twice_adopts_the_existing_document_rather_than_overwriting_it() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    let before = fs::read_to_string(checklist_path(&root)).unwrap();

    assert_eq!(init(&root, "ship-it"), ExitCode::SUCCESS);

    assert_eq!(
        fs::read_to_string(checklist_path(&root)).unwrap(),
        before,
        "a second init must not throw away the work in the document"
    );
    assert_eq!(pointer_target(&root).unwrap(), checklist_path(&root));
}

/// A slug that already carries the `YYYY-MM-` prefix — the name a previous run
/// printed — addresses the same document instead of nesting a second date.
#[test]
fn a_slug_that_already_carries_a_date_prefix_is_taken_whole() {
    let (_dir, root) = ignored_repo();
    assert_eq!(init(&root, STEM), ExitCode::SUCCESS);
    assert!(checklist_path(&root).exists());
}

#[test]
fn a_malformed_slug_is_refused_before_anything_is_written() {
    let (_dir, root) = ignored_repo();
    assert_eq!(init(&root, "Ship_It"), ExitCode::from(2));
    assert!(!root.join(ACTIONS_REL).exists());
    assert!(!pointer_path(&root).exists());
}

/// R89 wants the pointer created only after R56's containment and R63's
/// ignored-tree checks pass. A repository that never got the ignore rule is
/// refused outright — no pointer AND no document, because half of `init` is
/// worse than none.
#[test]
fn init_writes_nothing_while_the_state_tree_is_not_ignored() {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();

    assert_eq!(init(&root, "ship-it"), ExitCode::from(1));
    assert!(!pointer_path(&root).exists());
    assert!(
        !root.join(ACTIONS_REL).exists(),
        "the document must not be written without a pointer naming it"
    );
}

// ── Round-tripping through verify ─────────────────────────────────────────────

/// Every verb, in the order a person would actually use them, and `verify`
/// clean at the end.
#[test]
fn every_verb_round_trips_through_verify() {
    let (_dir, root) = initialized();

    assert_eq!(
        run_core(&root, &Sub::Verify, NOW).unwrap(),
        ExitCode::SUCCESS,
        "a freshly initialized checklist is valid"
    );

    complete_item(&root, "verification", "check-dns");
    assert_eq!(
        set(&root, "check-dns", "priority", Some("blocking")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(
            &root,
            "check-dns",
            "why",
            Some("the old record has a 24h TTL")
        ),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(
            &root,
            "check-dns",
            "refs.-",
            Some("https://example.com/pr/1")
        ),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(&root, "check-dns", "refs.0.label", Some("the PR")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        add_entry_verb(&root, "cutover-planned", "picked Thursday"),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(&root, "document", "title", Some("Ship the thing")),
        ExitCode::SUCCESS
    );
    assert_eq!(done(&root, "check-dns"), ExitCode::SUCCESS);

    assert_eq!(
        run_core(&root, &Sub::Verify, NOW).unwrap(),
        ExitCode::SUCCESS
    );
    assert_eq!(run_core(&root, &Sub::List, NOW).unwrap(), ExitCode::SUCCESS);
    assert_eq!(
        run_core(&root, &Sub::RenderMd, NOW).unwrap(),
        ExitCode::SUCCESS
    );

    let doc = doc_at(&root);
    assert!(validate(&doc).is_empty(), "{:?}", validate(&doc));
    assert_eq!(doc.title, "Ship the thing");
    let item = &doc.sections[0].items[0];
    assert!(item.done);
    assert_eq!(
        item.completed.as_ref().unwrap().as_str(),
        "2026-08-30T12:00:00Z"
    );
    assert_eq!(item.refs[0].label, "the PR");
}

/// `verify` reports the two defects of AE73 and exits non-zero. It never
/// renders the document it just called malformed.
#[test]
fn verify_fails_on_a_done_item_with_no_timestamp_and_a_null_expectation() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    // Hand-edited into both defects at once, which is exactly the case the
    // Read/Edit deny cannot prevent when the binary is not installed.
    let mut raw = raw_at(&root);
    let item = &mut raw["sections"][0]["items"][0];
    item["done"] = json!(true);
    item["completed"] = Value::Null;
    item["expected"] = Value::Null;
    write_raw(&root, &raw);

    assert_eq!(
        run_core(&root, &Sub::Verify, NOW).unwrap(),
        ExitCode::from(1)
    );
}

/// Every write re-establishes canonical order, so the diff a reviewer reads is
/// about content rather than about where things moved.
#[test]
fn a_write_re_sorts_the_document_and_re_stamps_updated() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "later");
    complete_item(&root, "verification", "earlier");

    // Reach into the file and give `earlier` the earlier instant, spelled at a
    // different UTC offset so a lexical sort would get it wrong.
    let mut raw = raw_at(&root);
    let items = raw["sections"][0]["items"].as_array_mut().unwrap();
    for item in items.iter_mut() {
        if item["id"] == json!("earlier") {
            item["created"] = json!("2026-08-30T13:00:00+02:00"); // 11:00Z
        }
    }
    write_raw(&root, &raw);

    // Any write at all is enough to re-sort.
    assert_eq!(
        set(&root, "later", "priority", Some("blocking")),
        ExitCode::SUCCESS
    );

    let doc = doc_at(&root);
    let ids: Vec<&str> = doc.sections[0]
        .items
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["later", "earlier"],
        "blocking outranks unranked, so priority decides before the instant does"
    );
    assert_eq!(doc.updated.as_str(), "2026-08-30T12:00:00Z");
}

// ── The unknown-field round-trip (the unit's verification line) ───────────────

/// No verb rebuilds the document from parts, so a key this build has never
/// heard of survives every one of them — at the top level, on a section, on an
/// item, on a changelog entry, and on a reference.
#[test]
fn no_verb_drops_a_key_this_build_does_not_know() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    assert_eq!(
        set(
            &root,
            "check-dns",
            "refs.-",
            Some("https://example.com/pr/1")
        ),
        ExitCode::SUCCESS
    );
    assert_eq!(
        add_entry_verb(&root, "cutover-planned", "picked Thursday"),
        ExitCode::SUCCESS
    );

    let mut raw = raw_at(&root);
    raw["x-document"] = json!({"written-by": "a newer ss-magic"});
    raw["sections"][0]["x-section"] = json!("kept");
    raw["sections"][0]["items"][0]["x-item"] = json!([1, 2, 3]);
    raw["sections"][0]["items"][0]["refs"][0]["x-ref"] = json!("kept");
    raw["changelog"][0]["x-entry"] = json!("kept");
    write_raw(&root, &raw);

    // Every writing verb, one after another, over the document carrying them.
    assert_eq!(
        add_item_verb(&root, "rollout", "flip-flag", "flip the flag"),
        ExitCode::SUCCESS
    );
    assert_eq!(
        add_entry_verb(&root, "flag-decided", "agreed on the flag"),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(&root, "check-dns", "why", Some("the TTL is 24h")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        set(&root, "document", "title", Some("Ship the thing")),
        ExitCode::SUCCESS
    );
    assert_eq!(done(&root, "check-dns"), ExitCode::SUCCESS);

    let after = raw_at(&root);
    assert_eq!(
        after["x-document"],
        json!({"written-by": "a newer ss-magic"})
    );

    let section = after["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == json!("verification"))
        .unwrap();
    assert_eq!(section["x-section"], json!("kept"));

    let item = section["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == json!("check-dns"))
        .unwrap();
    assert_eq!(item["x-item"], json!([1, 2, 3]));
    assert_eq!(item["refs"][0]["x-ref"], json!("kept"));

    let entry = after["changelog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == json!("cutover-planned"))
        .unwrap();
    assert_eq!(entry["x-entry"], json!("kept"));
}

// ── Loud failures ─────────────────────────────────────────────────────────────

/// `set` on an id that addresses nothing fails loudly and leaves the file
/// byte-identical — a typo must not half-apply.
#[test]
fn set_on_an_unknown_id_fails_and_changes_nothing() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    let before = fs::read_to_string(checklist_path(&root)).unwrap();

    assert_eq!(
        set(&root, "check-dsn", "title", Some("typo")),
        ExitCode::from(2)
    );
    assert_eq!(fs::read_to_string(checklist_path(&root)).unwrap(), before);
}

/// A dotted key the schema has no field for is refused rather than being
/// invented as a new key — the `extras` maps preserve what a NEWER build
/// wrote, they are not a place for this build to stash typos.
#[test]
fn set_with_a_key_outside_the_schema_fails_and_changes_nothing() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    let before = fs::read_to_string(checklist_path(&root)).unwrap();

    for key in ["nonesuch", "priority.high", "steps.9", "refs.0.url"] {
        assert_eq!(
            set(&root, "check-dns", key, Some("x")),
            ExitCode::from(2),
            "`{key}` must be refused"
        );
    }
    assert_eq!(fs::read_to_string(checklist_path(&root)).unwrap(), before);
}

#[test]
fn add_item_naming_a_section_that_does_not_exist_fails_and_changes_nothing() {
    let (_dir, root) = initialized();
    let before = fs::read_to_string(checklist_path(&root)).unwrap();

    assert_eq!(
        add_item_verb(&root, "nowhere", "check-dns", "x"),
        ExitCode::from(2)
    );
    assert_eq!(fs::read_to_string(checklist_path(&root)).unwrap(), before);
}

/// Ids are permanent, so a duplicate is refused at the point of entry rather
/// than left for `verify` to find after something already points at it.
#[test]
fn a_duplicate_or_malformed_id_is_refused() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    assert_eq!(
        add_item_verb(&root, "rollout", "check-dns", "x"),
        ExitCode::from(2)
    );
    assert_eq!(add_entry_verb(&root, "check-dns", "x"), ExitCode::from(2));
    assert_eq!(
        add_item_verb(&root, "rollout", "verification", "x"),
        ExitCode::from(2),
        "a section's id is in the same namespace, since `set <id>` addresses both"
    );
    assert_eq!(
        add_item_verb(&root, "rollout", "Check_DNS", "x"),
        ExitCode::from(2)
    );
    assert_eq!(
        add_item_verb(&root, "rollout", "document", "x"),
        ExitCode::from(2),
        "`document` addresses the header, so no record may claim it"
    );
}

/// A checklist hand-edited into invalid JSON stops every verb with the parse
/// error, rather than being silently replaced by something this build could
/// build from scratch.
#[test]
fn a_document_edited_into_invalid_json_stops_every_verb() {
    let (_dir, root) = initialized();
    fs::write(checklist_path(&root), "{ not json").unwrap();

    for sub in [
        Sub::Verify,
        Sub::List,
        Sub::RenderMd,
        Sub::Done {
            id: "check-dns".into(),
        },
        Sub::Set {
            id: "check-dns".into(),
            key: "title".into(),
            value: Some("x".into()),
            from_stdin: false,
        },
    ] {
        assert_eq!(
            run_core(&root, &sub, NOW).unwrap(),
            ExitCode::from(2),
            "{sub:?}"
        );
    }
    assert_eq!(
        fs::read_to_string(checklist_path(&root)).unwrap(),
        "{ not json",
        "and the bytes the author has yet to fix are left exactly as they are"
    );
}

/// Nothing to work on at all is a usage failure with a pointer at `init`,
/// never a panic or a silently-created document.
#[test]
fn a_repository_with_no_checklist_reports_it() {
    let (_dir, root) = ignored_repo();
    for sub in [Sub::Verify, Sub::List, Sub::RenderMd] {
        assert_eq!(run_core(&root, &sub, NOW).unwrap(), ExitCode::from(2));
    }
    assert!(!root.join(ACTIONS_REL).exists());
}

// ── Field semantics ───────────────────────────────────────────────────────────

/// `done` twice is idempotent: the second run keeps the timestamp the work was
/// actually completed at, because that is a historical fact.
#[test]
fn done_on_an_already_done_item_keeps_the_original_timestamp() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    assert_eq!(done(&root, "check-dns"), ExitCode::SUCCESS);
    let first = doc_at(&root).sections[0].items[0]
        .completed
        .clone()
        .unwrap();

    // An hour later, same verb.
    assert_eq!(
        run_core(
            &root,
            &Sub::Done {
                id: "check-dns".into()
            },
            NOW + 3600
        )
        .unwrap(),
        ExitCode::SUCCESS
    );

    let item = doc_at(&root).sections[0].items[0].clone();
    assert!(item.done);
    assert_eq!(item.completed.unwrap(), first);
}

/// `done` only applies to items; a changelog entry or a section is not
/// something that gets completed.
#[test]
fn done_on_something_that_is_not_an_item_is_refused() {
    let (_dir, root) = initialized();
    assert_eq!(
        add_entry_verb(&root, "cutover-planned", "picked Thursday"),
        ExitCode::SUCCESS
    );

    assert_eq!(done(&root, "cutover-planned"), ExitCode::from(2));
    assert_eq!(done(&root, "verification"), ExitCode::from(2));
    assert_eq!(done(&root, "nowhere"), ExitCode::from(2));
}

/// An empty value clears a field that may be absent, and is refused on one
/// that may not. This is what an empty stdin body does: `why` disappears
/// rather than becoming `"why": ""`, which would render as a heading with
/// nothing under it.
#[test]
fn an_empty_value_clears_an_optional_field_and_is_refused_on_a_required_one() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    assert_eq!(
        set(&root, "check-dns", "why", Some("because")),
        ExitCode::SUCCESS
    );
    assert!(doc_at(&root).sections[0].items[0].why.is_some());

    assert_eq!(set(&root, "check-dns", "why", Some("")), ExitCode::SUCCESS);
    let raw = raw_at(&root);
    assert!(
        raw["sections"][0]["items"][0].get("why").is_none(),
        "an unset optional key is omitted entirely, never written as null"
    );

    let before = fs::read_to_string(checklist_path(&root)).unwrap();
    assert_eq!(
        set(&root, "check-dns", "title", Some("   ")),
        ExitCode::from(2)
    );
    assert_eq!(set(&root, "check-dns", "title", None), ExitCode::from(2));
    assert_eq!(fs::read_to_string(checklist_path(&root)).unwrap(), before);
}

/// The two optionality conventions the schema keeps apart survive a `set`:
/// `expected` stays an always-present key whose value may be null, and
/// `priority` is omitted entirely rather than written as one.
#[test]
fn set_keeps_the_two_optionality_conventions_apart() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    assert_eq!(
        set(&root, "check-dns", "priority", Some("blocking")),
        ExitCode::SUCCESS
    );

    assert_eq!(
        set(&root, "check-dns", "kind", Some("record")),
        ExitCode::SUCCESS
    );
    assert_eq!(set(&root, "check-dns", "expected", None), ExitCode::SUCCESS);
    assert_eq!(set(&root, "check-dns", "priority", None), ExitCode::SUCCESS);

    let item = &raw_at(&root)["sections"][0]["items"][0];
    assert_eq!(
        item["expected"],
        Value::Null,
        "`expected` is always written"
    );
    assert!(
        item.get("priority").is_none(),
        "`priority` is an absence that sorts last, not a null"
    );
    // A null expectation is legal on a record-kind item, so this validates.
    assert_eq!(
        run_core(&root, &Sub::Verify, NOW).unwrap(),
        ExitCode::SUCCESS
    );
}

/// An `expected` key an older hand-edit left out is repaired by the next
/// write, exactly as the validator's warning promises.
#[test]
fn the_next_write_adds_an_absent_expected_key_as_null() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    assert_eq!(
        set(&root, "check-dns", "kind", Some("record")),
        ExitCode::SUCCESS
    );

    let mut raw = raw_at(&root);
    raw["sections"][0]["items"][0]
        .as_object_mut()
        .unwrap()
        .remove("expected");
    write_raw(&root, &raw);

    assert_eq!(
        set(&root, "check-dns", "why", Some("because")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        raw_at(&root)["sections"][0]["items"][0]["expected"],
        Value::Null
    );
}

/// `done` and the completion timestamp are kept agreeing in both directions,
/// which is what the validator checks: setting `done` true stamps the time,
/// setting it false drops a stamp that would describe unfinished work.
#[test]
fn setting_done_keeps_the_completion_timestamp_consistent() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    assert_eq!(
        set(&root, "check-dns", "done", Some("true")),
        ExitCode::SUCCESS
    );
    assert!(doc_at(&root).sections[0].items[0].completed.is_some());

    assert_eq!(
        set(&root, "check-dns", "done", Some("false")),
        ExitCode::SUCCESS
    );
    assert_eq!(doc_at(&root).sections[0].items[0].completed, None);

    assert_eq!(
        set(&root, "check-dns", "done", Some("yes")),
        ExitCode::from(2)
    );
}

/// A timestamp that cannot be read is refused at the point of entry, so the
/// file never holds a value the ordering cannot compare.
#[test]
fn an_unreadable_timestamp_is_refused_when_it_is_typed() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    assert_eq!(
        set(&root, "check-dns", "completed", Some("yesterday")),
        ExitCode::from(2)
    );
    assert_eq!(
        set(&root, "check-dns", "completed", Some("2026-08-30T12:00:00")),
        ExitCode::from(2),
        "a local time with no offset names no instant"
    );
    assert_eq!(
        set(
            &root,
            "check-dns",
            "completed",
            Some("2026-08-30T12:00:00+02:00")
        ),
        ExitCode::SUCCESS
    );
}

/// The step keys: the bare list replaces, `-` appends, an index replaces one,
/// and an index with `null` removes it.
#[test]
fn the_step_keys_replace_append_and_remove() {
    let (_dir, root) = initialized();
    assert_eq!(
        add_item_verb(&root, "verification", "check-dns", "check it"),
        ExitCode::SUCCESS
    );

    assert_eq!(
        set(&root, "check-dns", "steps", Some("one\ntwo\n\nthree")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        doc_at(&root).sections[0].items[0].steps,
        ["one", "two", "three"]
    );

    assert_eq!(
        set(&root, "check-dns", "steps.-", Some("four\nand a half")),
        ExitCode::SUCCESS
    );
    assert_eq!(
        doc_at(&root).sections[0].items[0].steps[3],
        "four\nand a half",
        "an appended step keeps its own newlines"
    );

    assert_eq!(
        set(&root, "check-dns", "steps.1", Some("second")),
        ExitCode::SUCCESS
    );
    assert_eq!(doc_at(&root).sections[0].items[0].steps[1], "second");

    assert_eq!(set(&root, "check-dns", "steps.0", None), ExitCode::SUCCESS);
    assert_eq!(doc_at(&root).sections[0].items[0].steps[0], "second");
}

/// `refs.-` appends a reference already carrying a label, so a one-command
/// append never leaves the bare-URL rendering the validator warns about.
#[test]
fn appending_a_reference_labels_it_with_its_own_url() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    assert_eq!(
        set(
            &root,
            "check-dns",
            "refs.-",
            Some("https://example.com/pr/1")
        ),
        ExitCode::SUCCESS
    );
    let reference = doc_at(&root).sections[0].items[0].refs[0].clone();
    assert_eq!(reference.url, "https://example.com/pr/1");
    assert_eq!(reference.label, "https://example.com/pr/1");

    assert_eq!(set(&root, "check-dns", "refs", None), ExitCode::SUCCESS);
    assert!(doc_at(&root).sections[0].items[0].refs.is_empty());
}

/// Changing the slug is metadata, not a rename: the path is what the pointer,
/// the pull request and every reference already point at.
#[test]
fn setting_the_slug_does_not_move_the_file() {
    let (_dir, root) = initialized();
    assert_eq!(
        set(&root, "document", "slug", Some("2026-08-other")),
        ExitCode::SUCCESS
    );

    assert!(checklist_path(&root).exists());
    assert_eq!(doc_at(&root).slug, "2026-08-other");
    assert_eq!(pointer_target(&root).unwrap(), checklist_path(&root));
}

// ── Concurrency ───────────────────────────────────────────────────────────────

/// Eight `add-entry`s at once. The temp-then-rename makes each write whole, and
/// the lock spanning the read-modify-write is what stops one racer's entry
/// from being read away by another — so all eight land and the document is
/// valid.
#[test]
fn a_concurrent_double_write_leaves_one_valid_document() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    const RACERS: usize = 8;
    let barrier = Arc::new(Barrier::new(RACERS));
    let root = Arc::new(root);

    let handles: Vec<_> = (0..RACERS)
        .map(|n| {
            let barrier = Arc::clone(&barrier);
            let root = Arc::clone(&root);
            std::thread::spawn(move || {
                barrier.wait();
                add_entry_verb(&root, &format!("entry-{n}"), "raced")
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), ExitCode::SUCCESS);
    }

    let doc = doc_at(&root);
    assert!(validate(&doc).is_empty(), "{:?}", validate(&doc));
    let mut ids: Vec<&str> = doc.changelog.iter().map(|e| e.id.as_str()).collect();
    ids.sort_unstable();
    let expected: Vec<String> = (0..RACERS).map(|n| format!("entry-{n}")).collect();
    assert_eq!(
        ids, expected,
        "no racer's entry may be read away by another"
    );
}

// ── Falling back to the naming convention ─────────────────────────────────────

/// The pointer lives in the gitignored state tree, so a fresh clone has the
/// committed checklist and no pointer at all. The `docs/actions/` naming
/// convention answers instead, as long as it answers unambiguously.
#[test]
fn a_repository_with_no_pointer_falls_back_to_the_naming_convention() {
    let (_dir, root) = initialized();
    fs::remove_file(pointer_path(&root)).unwrap();

    assert_eq!(
        run_core(&root, &Sub::Verify, NOW).unwrap(),
        ExitCode::SUCCESS
    );
    assert_eq!(
        add_entry_verb(&root, "cutover-planned", "picked Thursday"),
        ExitCode::SUCCESS
    );
    assert_eq!(doc_at(&root).changelog.len(), 1);
}

/// Two checklists and no pointer is genuinely ambiguous, so it is reported
/// rather than guessed at.
#[test]
fn two_checklists_and_no_pointer_is_reported_rather_than_guessed() {
    let (_dir, root) = initialized();
    fs::remove_file(pointer_path(&root)).unwrap();
    fs::copy(
        checklist_path(&root),
        root.join(format!("{ACTIONS_REL}/2026-08-other{CHECKLIST_SUFFIX}")),
    )
    .unwrap();

    assert!(
        run_core(&root, &Sub::Verify, NOW).is_err(),
        "the ambiguity is surfaced, not resolved by picking one"
    );
}

// ── Naming convention, for U28's classifier ──────────────────────────────────

/// The convention is purely lexical: a path that does not exist still matches,
/// and a same-named file outside `docs/actions/` does not.
#[test]
fn the_naming_convention_is_lexical_and_scoped_to_docs_actions() {
    let root = Path::new("/repo");

    assert!(matches_convention(
        root,
        &root.join("docs/actions/2026-08-x.checklist.json")
    ));
    assert!(!matches_convention(
        root,
        &root.join("docs/actions/notes.md")
    ));
    assert!(
        !matches_convention(root, &root.join("docs/actions/.checklist.json")),
        "the stem is not optional"
    );
    assert!(
        !matches_convention(root, &root.join("docs/actions/nested/x.checklist.json")),
        "only the directory itself, not a subtree of it"
    );
    assert!(!matches_convention(
        root,
        &root.join("elsewhere/2026-08-x.checklist.json")
    ));
    assert!(!matches_convention(
        Path::new("/other"),
        &root.join("docs/actions/2026-08-x.checklist.json")
    ));
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Both rendering verbs go through the shared renderer, so both carry the
/// untrusted-data envelope; only `list` is bounded, because only `list` is read
/// into somebody's context.
#[test]
fn list_and_render_md_both_emit_the_envelope_and_differ_only_in_their_budget() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");

    let doc = doc_at(&root);
    let path = checklist_path(&root);
    let bounded = render_markdown(&doc, &path, None, Budget::Bytes(LIST_BYTE_BUDGET));
    let unbounded = render_markdown(&doc, &path, None, Budget::Unbounded);

    for text in [&bounded, &unbounded] {
        assert!(text.contains("BEGIN-UNTRUSTED-DATA"));
        assert!(text.contains("check the thing"));
    }
    assert_eq!(bounded, unbounded, "a small checklist fits either way");
}

/// A repository URL is only offered to the renderer when a reader could
/// actually open it; the transport-only forms are rewritten or dropped.
#[test]
fn only_a_browsable_origin_reaches_the_render() {
    let (dir, root) = ignored_repo();
    assert!(
        browsable_origin(&root).is_none(),
        "a repository with no origin renders without a repository line"
    );

    for (remote, expected) in [
        (
            "https://example.com/owner/repo.git",
            Some("https://example.com/owner/repo"),
        ),
        (
            "git@example.com:owner/repo.git",
            Some("https://example.com/owner/repo"),
        ),
        (
            "ssh://git@example.com:22/owner/repo",
            Some("https://example.com/owner/repo"),
        ),
        ("git://example.com/owner/repo.git", None),
        (dir.path().to_str().unwrap(), None),
    ] {
        // `config` rather than `remote add`, so the loop can rewrite the same
        // remote without first having to remove one that may not be there.
        git_run(&["config", "--local", "remote.origin.url", remote], &root);
        assert_eq!(
            browsable_origin(&root).as_deref(),
            expected,
            "for remote `{remote}`"
        );
    }
}

// ── The parse ─────────────────────────────────────────────────────────────────

#[test]
fn the_subverb_parse_covers_the_whole_documented_surface() {
    let argv = |tokens: &[&str]| -> ParsedSub {
        parse(&tokens.iter().map(|t| t.to_string()).collect::<Vec<_>>())
    };

    assert_eq!(
        argv(&["init", "ship-it"]),
        ParsedSub::Run(Sub::Init {
            slug: "ship-it".into()
        })
    );
    assert_eq!(
        argv(&["add-item", "rollout", "flip"]),
        ParsedSub::Run(Sub::AddItem {
            section: "rollout".into(),
            id: "flip".into(),
            title: None
        })
    );
    assert_eq!(
        argv(&["set", "flip", "why", "null"]),
        ParsedSub::Run(Sub::Set {
            id: "flip".into(),
            key: "why".into(),
            value: None,
            from_stdin: false
        }),
        "the literal `null` is how a field is cleared"
    );
    assert!(
        matches!(argv(&["set", "flip", "why"]), ParsedSub::Run(sub) if sub.wants_stdin()),
        "a missing value is the signal to read stdin"
    );
    assert_eq!(argv(&["list"]), ParsedSub::Run(Sub::List));
    assert_eq!(argv(&["verify"]), ParsedSub::Run(Sub::Verify));
    assert_eq!(argv(&["render-md"]), ParsedSub::Run(Sub::RenderMd));
    assert_eq!(argv(&["--help"]), ParsedSub::Help);
    assert_eq!(
        argv(&["init", "--help"]),
        ParsedSub::Help,
        "help wins wherever it appears, rather than becoming a slug"
    );

    for bad in [
        vec![],
        vec!["nonesuch"],
        vec!["init"],
        vec!["init", "a", "b"],
        vec!["done"],
        vec!["list", "extra"],
        vec!["set", "flip"],
    ] {
        assert!(
            matches!(argv(&bad), ParsedSub::Error(_)),
            "{bad:?} must be a loud error"
        );
    }
}

/// `set` is the one verb whose body is the whole point, so it refuses rather
/// than silently doing nothing when there is no value anywhere.
#[test]
fn only_set_treats_a_missing_body_as_a_failure() {
    assert!(Sub::Set {
        id: "x".into(),
        key: "why".into(),
        value: None,
        from_stdin: true
    }
    .stdin_is_required());
    assert!(!Sub::AddItem {
        section: "s".into(),
        id: "x".into(),
        title: None
    }
    .stdin_is_required());
    assert!(!Sub::AddEntry {
        id: "x".into(),
        summary: None
    }
    .stdin_is_required());
}

// ── Ambient facts the module depends on ──────────────────────────────────────

/// `init` writes the checklist as ordinary, world-readable repository content
/// rather than with the owner-only mode the state tree uses — it is committed,
/// reviewed and read by CI, not private state.
#[test]
fn the_checklist_is_written_as_ordinary_repository_content() {
    use std::os::unix::fs::PermissionsExt as _;

    let (_dir, root) = initialized();
    let mode = fs::metadata(checklist_path(&root))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, NEW_FILE_MODE);

    // And a rewrite keeps whatever mode the file has, rather than imposing one.
    fs::set_permissions(checklist_path(&root), fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(add_entry_verb(&root, "noted", "a note"), ExitCode::SUCCESS);
    let after = fs::metadata(checklist_path(&root))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(after, 0o640);
}

/// A write leaves no litter behind in the repository's working tree — the temp
/// file is renamed away, so `docs/actions/` holds exactly one file.
#[test]
fn a_write_leaves_no_temp_file_behind() {
    let (_dir, root) = initialized();
    complete_item(&root, "verification", "check-dns");
    assert_eq!(done(&root, "check-dns"), ExitCode::SUCCESS);

    let entries: Vec<String> = fs::read_dir(root.join(ACTIONS_REL))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, [format!("{STEM}{CHECKLIST_SUFFIX}")]);
}
