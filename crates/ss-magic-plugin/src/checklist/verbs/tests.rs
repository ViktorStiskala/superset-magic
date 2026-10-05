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
use ss_magic_core::testutil::{git_run, init_main_repo};

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
        run_core(&root, &Sub::Verify { files: vec![] }, NOW).unwrap(),
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
        run_core(&root, &Sub::Verify { files: vec![] }, NOW).unwrap(),
        ExitCode::SUCCESS
    );
    assert_eq!(run_core(&root, &Sub::List, NOW).unwrap(), ExitCode::SUCCESS);
    assert_eq!(
        run_core(
            &root,
            &Sub::RenderMd {
                files: vec![],
                max_bytes: None
            },
            NOW
        )
        .unwrap(),
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
        run_core(&root, &Sub::Verify { files: vec![] }, NOW).unwrap(),
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
        Sub::Verify { files: vec![] },
        Sub::List,
        Sub::RenderMd {
            files: vec![],
            max_bytes: None,
        },
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
    for sub in [
        Sub::Verify { files: vec![] },
        Sub::List,
        Sub::RenderMd {
            files: vec![],
            max_bytes: None,
        },
    ] {
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
        run_core(&root, &Sub::Verify { files: vec![] }, NOW).unwrap(),
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
        run_core(&root, &Sub::Verify { files: vec![] }, NOW).unwrap(),
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
        run_core(&root, &Sub::Verify { files: vec![] }, NOW).is_err(),
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

/// The suffix comparison must not slice the file name at a byte offset that
/// falls inside a character.
///
/// Folding the suffix's case replaced `name.ends_with(CHECKLIST_SUFFIX)` with a
/// comparison against `name[name.len() - CHECKLIST_SUFFIX.len()..]`, and
/// indexing a `str` at an offset that is not a character boundary panics.
/// `a\u{e9}.checklist.jso` is seventeen bytes with the `\u{e9}` spanning bytes 1..3,
/// so the fifteen-byte suffix starts in the middle of it — and a `Read` or
/// `Write` of a file with that name goes through this predicate, so the panic
/// would have been reachable from a tool call. `str::get` returns `None` at
/// such an offset, which is also the right answer: a name whose last fifteen
/// bytes are not a whole `.checklist.json` is not a checklist.
#[test]
fn a_name_whose_suffix_offset_splits_a_character_is_rejected_without_panicking() {
    let root = Path::new("/repo");

    let split = "a\u{e9}.checklist.jso";
    assert_eq!(split.len(), CHECKLIST_SUFFIX.len() + 2);
    assert!(
        !split.is_char_boundary(split.len() - CHECKLIST_SUFFIX.len()),
        "this test is only meaningful if the offset really does split a character"
    );
    assert!(!matches_convention(
        root,
        &root.join(format!("docs/actions/{split}"))
    ));

    // And a multi-byte stem in front of a whole suffix still matches, folded.
    assert!(matches_convention(
        root,
        &root.join("docs/actions/na\u{ef}ve.CHECKLIST.JSON")
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
    assert_eq!(
        argv(&["verify"]),
        ParsedSub::Run(Sub::Verify { files: vec![] })
    );
    assert_eq!(
        argv(&["render-md"]),
        ParsedSub::Run(Sub::RenderMd {
            files: vec![],
            max_bytes: None
        })
    );
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

// ── Explicit paths (R15) and the render budget (R16) ─────────────────────────

/// Parse `argv` the way `run` does and run the result through the same
/// `run_core_with` it reaches, with both streams captured so the output itself
/// can be asserted on. A parse error is returned as the message `run` would
/// print beside its exit 2.
fn run_argv(cwd: &Path, argv: &[&str]) -> Result<(ExitCode, String, String), String> {
    let args: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    let sub = match parse(&args) {
        ParsedSub::Run(sub) => sub,
        ParsedSub::Error(message) => return Err(message),
        ParsedSub::Help => panic!("{argv:?} is not a help request"),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_core_with(cwd, &sub, NOW, &mut out, &mut err).unwrap();
    Ok((
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    ))
}

/// [`run_argv`] for an argv that must parse.
fn run_ok(cwd: &Path, argv: &[&str]) -> (ExitCode, String, String) {
    run_argv(cwd, argv).unwrap_or_else(|message| panic!("{argv:?} must parse: {message}"))
}

/// Write a checklist at `docs/actions/<stem>.checklist.json` without touching
/// the pointer, and return its repository-relative path. `filler` bytes of
/// description pad the render, so a test can make one document outgrow a
/// budget while another fits.
fn write_checklist(root: &Path, stem: &str, filler: usize) -> String {
    let rel = format!("{ACTIONS_REL}/{stem}{CHECKLIST_SUFFIX}");
    let mut doc = Document::new(
        format!("Checklist {}", stem.len()),
        stem,
        Timestamp::from_epoch_secs(NOW),
    );
    if filler > 0 {
        doc.sections[0].items.push(Item {
            id: "pad".into(),
            title: "padding".into(),
            created: Timestamp::from_epoch_secs(NOW),
            steps: vec!["read it".into()],
            expected: Some(Some("it is long".into())),
            // Many short lines rather than one long one, so the envelope's
            // truncation has line boundaries to cut at.
            description: Some("padding line\n".repeat(filler / 13 + 1)),
            ..Item::default()
        });
    }
    write_document(&root.join(&rel), &mut doc).unwrap();
    rel
}

/// What `render-md` emits for one document on its own, unbounded — the unit
/// every multi-document body is assembled from. The fixtures have no origin,
/// so there is no repository line.
fn single_render(root: &Path, rel: &str) -> String {
    let path = root.join(rel);
    render_markdown(
        &read_document(&path).unwrap().unwrap(),
        &path,
        None,
        Budget::Unbounded,
    )
}

#[test]
fn verify_with_one_explicit_valid_path_reports_it_valid() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);

    let (code, out, _) = run_ok(&root, &["verify", &a]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.contains(&format!("{a} is valid")), "{out}");
}

/// Several paths are verified one by one; an error in any of them fails the
/// whole run with the "invalid document" code, and names the one at fault.
#[test]
fn verify_over_several_paths_exits_1_and_names_the_invalid_one() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);
    let b = write_checklist(&root, "2026-08-b", 0);
    let mut doc = read_document(&root.join(&b)).unwrap().unwrap();
    // A check-kind item with no steps and a null expectation: two errors.
    doc.sections[0].items.push(Item {
        id: "unfinished".into(),
        title: "not filled in".into(),
        created: Timestamp::from_epoch_secs(NOW),
        expected: Some(None),
        ..Item::default()
    });
    write_document(&root.join(&b), &mut doc).unwrap();

    let (code, out, err) = run_ok(&root, &["verify", &a, &b]);
    assert_eq!(code, ExitCode::from(1));
    assert!(out.contains(&format!("{a} is valid")), "{out}");
    assert!(err.contains(&format!("{b} is not valid")), "{err}");
    assert!(
        err.lines()
            .filter(|l| l.contains("error:"))
            .all(|l| l.contains(&b)),
        "every finding is attributed to the document it is in:\n{err}"
    );
}

/// Several documents render as each one's own `render()`, in argument order,
/// joined by the fixed separator and nothing else — no heading of the verb's
/// own, since every render already carries its title inside its envelope.
#[test]
fn render_md_over_several_paths_joins_each_render_in_argument_order() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);
    let b = write_checklist(&root, "2026-08-b", 0);

    let whole = format!(
        "{}{DOCUMENT_SEPARATOR}{}",
        single_render(&root, &b),
        single_render(&root, &a)
    );
    let (code, out, _) = run_ok(&root, &["render-md", &b, &a]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(out, whole);
    assert_eq!(out.matches("BEGIN-UNTRUSTED-DATA").count(), 2);

    // A budget the whole body fits in cuts nothing, even though the room it
    // would set aside for a marker leaves less than the body needs.
    let max = whole.len().max(MARKER_RESERVE).to_string();
    let (code, out, _) = run_ok(&root, &["render-md", "--max-bytes", &max, &b, &a]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(out, whole);
}

/// AE2: the budget cuts at a document boundary. The first document fits and
/// renders whole; the second is omitted entirely and named in the marker.
#[test]
fn render_md_budget_stops_before_a_document_that_would_cross_it() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);
    let b = write_checklist(&root, "2026-08-b", 20_000);
    let first = single_render(&root, &a);
    let max = first.len() + MARKER_RESERVE + 100;

    let (code, out, _) = run_ok(
        &root,
        &["render-md", "--max-bytes", &max.to_string(), &a, &b],
    );
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.len() <= max, "{} > {max}", out.len());
    assert!(
        out.starts_with(&format!("{first}{DOCUMENT_SEPARATOR}")),
        "{out}"
    );
    let marker = &out[first.len() + DOCUMENT_SEPARATOR.len()..];
    assert!(marker.contains(&prose_inline(&b)), "{marker}");
    assert!(
        !out.contains("padding line"),
        "nothing of the omitted document leaks in"
    );
}

/// AE2 when the first document fills nearly the whole budget: the room set
/// aside is the marker this cut actually needs, not a flat reserve, so a first
/// document that fits beside its real marker renders whole rather than being
/// truncated to make space nothing uses.
#[test]
fn render_md_budget_renders_a_first_document_whole_when_it_fits_beside_the_marker() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 3_000);
    let b = write_checklist(&root, "2026-08-b", 20_000);
    let first = single_render(&root, &a);
    // Less slack than the flat reserve, but more than a one-name marker takes.
    let max = first.len() + 1_000;
    assert!(
        max >= MARKER_RESERVE,
        "the fixture must be a budget the flag accepts"
    );

    let (code, out, _) = run_ok(
        &root,
        &["render-md", "--max-bytes", &max.to_string(), &a, &b],
    );
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.len() <= max, "{} > {max}", out.len());
    assert!(
        out.starts_with(&format!("{first}{DOCUMENT_SEPARATOR}")),
        "the first document renders whole:\n{out}"
    );
    assert!(!out.contains("body truncated"), "{out}");
    let marker = &out[first.len() + DOCUMENT_SEPARATOR.len()..];
    assert!(marker.contains(&prose_inline(&b)), "{marker}");
}

/// When not even a truncated first document fits beside the marker, the
/// marker is the whole body, and the run still succeeds: an over-budget
/// comment is a fact to report, not a failure of the job. Enough long names
/// follow the first document that the marker naming them fills the smallest
/// budget the flag accepts, leaving no room for any of the first one.
#[test]
fn render_md_budget_with_nothing_fitting_emits_the_marker_alone() {
    let (_dir, root) = ignored_repo();
    let names: Vec<String> = (0..20)
        .map(|i| {
            write_checklist(
                &root,
                &format!("2026-08-{}-{i:03}", "n".repeat(180)),
                20_000,
            )
        })
        .collect();
    let max = MARKER_RESERVE.to_string();
    let mut argv = vec!["render-md", "--max-bytes", &max];
    argv.extend(names.iter().map(String::as_str));

    let (code, out, _) = run_ok(&root, &argv);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.len() <= MARKER_RESERVE, "{}", out.len());
    assert!(!out.contains("BEGIN-UNTRUSTED-DATA"), "{out}");
    assert!(!out.contains(DOCUMENT_SEPARATOR), "{out}");
    assert!(
        out.contains(&format!("{} checklist(s) left out", names.len())),
        "{out}"
    );
    assert!(out.contains(&prose_inline(&names[0])), "{out}");
}

/// The marker is itself bounded: a hundred long names cannot push the body
/// past the budget, and the names that did not fit are counted, not dropped
/// silently.
#[test]
fn render_md_budget_caps_the_marker_listing() {
    let (_dir, root) = ignored_repo();
    let names: Vec<String> = (0..100)
        .map(|i| write_checklist(&root, &format!("2026-08-{}-{i:03}", "n".repeat(180)), 0))
        .collect();
    let max = MARKER_RESERVE + 1_000;
    let mut argv = vec!["render-md", "--max-bytes"];
    let max_text = max.to_string();
    argv.push(&max_text);
    argv.extend(names.iter().map(String::as_str));

    let (code, out, _) = run_ok(&root, &argv);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.len() <= max, "{} > {max}", out.len());

    let last = out.trim_end().lines().last().unwrap();
    let more: usize = last
        .strip_prefix("- … and ")
        .and_then(|rest| rest.strip_suffix(" more"))
        .unwrap_or_else(|| panic!("the marker ends with the overflow count, not {last:?}"))
        .parse()
        .unwrap();
    let rendered = out.matches("BEGIN-UNTRUSTED-DATA").count();
    let listed = names
        .iter()
        .filter(|name| out.contains(&format!("- {}\n", prose_inline(name))))
        .count();
    assert!(listed > 0, "the marker lists what fits:\n{out}");
    assert_eq!(rendered + listed + more, names.len(), "{out}");
}

/// A name the repository chose reaches the marker escaped: a backtick cannot
/// open a code span and a newline cannot start a Markdown line of its own.
#[test]
fn an_omitted_name_cannot_break_out_of_the_marker() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);
    let hostile = write_checklist(&root, "2026-08-a`b\n# c", 20_000);
    // A lone carriage return is a CommonMark line ending too, though
    // `str::lines` does not split on it: unescaped, it would start a fenced
    // code block swallowing the rest of the comment.
    let fence = write_checklist(&root, "2026-08-a\r~~~x", 20_000);
    let max = single_render(&root, &a).len() + MARKER_RESERVE + 100;

    let (code, out, _) = run_ok(
        &root,
        &[
            "render-md",
            "--max-bytes",
            &max.to_string(),
            &a,
            &hostile,
            &fence,
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS);
    let marker = out.rsplit(DOCUMENT_SEPARATOR).next().unwrap();
    assert!(marker.contains("a\\`b<br>\\# c"), "{marker}");
    assert!(!marker.contains("a`b"), "{marker}");
    assert!(marker.contains("a<br>~~~x"), "{marker:?}");
    assert!(!marker.contains('\r'), "{marker:?}");
    assert!(
        !marker
            .split(['\r', '\n'])
            .any(|line| line.starts_with('#') || line.starts_with("~~~")),
        "no line of the marker is a block the name opened:\n{marker}"
    );
}

/// A first document too big for the budget on its own still shows its head:
/// it is truncated INSIDE its envelope, with the envelope's own notice naming
/// where the whole text is, and the body stays within the budget.
#[test]
fn render_md_budget_truncates_an_oversized_first_document_inside_its_envelope() {
    let (_dir, root) = ignored_repo();
    let big = write_checklist(&root, "2026-08-big", 40_000);
    let max = 8_000;

    let (code, out, _) = run_ok(&root, &["render-md", "--max-bytes", &max.to_string(), &big]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(
        single_render(&root, &big).len() > max,
        "the fixture must outgrow the budget"
    );
    assert!(out.len() <= max, "{} > {max}", out.len());
    assert!(out.contains("The whole text is at"), "{out}");
    assert!(
        out.contains("END-UNTRUSTED-DATA"),
        "the envelope is closed:\n{out}"
    );
    assert!(out.contains("padding line"), "the document's head is shown");
}

/// Each lexical refusal of R15 exits 2 and says which rule the path broke.
#[test]
fn explicit_paths_that_break_a_lexical_rule_are_refused_with_the_reason() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);
    fs::create_dir_all(root.join("docs/actions/sub")).unwrap();
    fs::copy(
        root.join(&a),
        root.join("docs/actions/sub/x.checklist.json"),
    )
    .unwrap();
    fs::copy(root.join(&a), root.join("docs/actions/notes.json")).unwrap();
    let absolute = root.join(&a).to_string_lossy().into_owned();

    for (path, reason) in [
        (absolute.as_str(), "absolute"),
        ("docs/actions/../actions/2026-08-a.checklist.json", "`..`"),
        (
            "docs/actions/notes.json",
            "docs/actions/<stem>.checklist.json",
        ),
        (
            "docs/actions/sub/x.checklist.json",
            "docs/actions/<stem>.checklist.json",
        ),
    ] {
        for verb in ["verify", "render-md"] {
            let (code, out, err) = run_ok(&root, &[verb, path]);
            assert_eq!(code, ExitCode::from(2), "{verb} {path}");
            assert!(err.contains(reason), "{verb} {path}: {err}");
            assert!(out.is_empty(), "{verb} {path} printed {out}");
        }
    }
}

/// AE3: a checklist path that is a symlink is refused before anything is read
/// through it — the link's target never reaches the output.
#[test]
fn a_symlinked_checklist_path_is_refused_and_never_read() {
    let (_dir, root) = ignored_repo();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("secret.checklist.json");
    let mut doc = Document::new(
        "OUTSIDE-SECRET-TITLE",
        "secret",
        Timestamp::from_epoch_secs(NOW),
    );
    write_document(&target, &mut doc).unwrap();
    fs::create_dir_all(root.join(ACTIONS_REL)).unwrap();
    let link = format!("{ACTIONS_REL}/2026-08-x{CHECKLIST_SUFFIX}");
    std::os::unix::fs::symlink(&target, root.join(&link)).unwrap();

    for verb in ["verify", "render-md"] {
        let (code, out, err) = run_ok(&root, &[verb, &link]);
        assert_eq!(code, ExitCode::from(2), "{verb}");
        assert!(err.contains("is a symlink"), "{verb}: {err}");
        assert!(!out.contains("OUTSIDE-SECRET") && !err.contains("OUTSIDE-SECRET"));
    }

    // A link is refused for being a link, not for where it leads: one to a
    // valid checklist inside the repository is refused just the same.
    let a = write_checklist(&root, "2026-08-a", 0);
    let inner = format!("{ACTIONS_REL}/2026-08-y{CHECKLIST_SUFFIX}");
    std::os::unix::fs::symlink(root.join(&a), root.join(&inner)).unwrap();
    let (code, _, err) = run_ok(&root, &["verify", &inner]);
    assert_eq!(code, ExitCode::from(2));
    assert!(err.contains("is a symlink"), "{err}");
}

/// The convention's case fold is shared with the checklist deny: an uppercase
/// directory is never refused for its spelling. Whether it then reads depends
/// only on whether the filesystem folds case too.
#[test]
fn an_uppercase_directory_is_treated_exactly_as_the_deny_treats_it() {
    let (_dir, root) = ignored_repo();
    let a = write_checklist(&root, "2026-08-a", 0);
    let upper = a.replacen("docs", "DOCS", 1);
    assert!(matches_convention(&root, &root.join(&upper)));

    let (code, _, err) = run_ok(&root, &["verify", &upper]);
    assert!(!err.contains("docs/actions/<stem>.checklist.json"), "{err}");
    if root.join("DOCS").exists() {
        assert_eq!(code, ExitCode::SUCCESS, "{err}");
    } else {
        assert_eq!(code, ExitCode::from(2));
        assert!(err.contains("does not exist"), "{err}");
    }
}

/// Containment is judged on the canonical form: a `docs/actions` that is a
/// symlinked directory leading out of the repository is refused, while a
/// repository whose own root sits behind a symlink (as macOS's `/tmp` does)
/// still accepts its own files.
#[test]
fn canonical_containment_refuses_an_escape_but_not_a_symlinked_root() {
    let (dir, root) = ignored_repo();
    let outside = TempDir::new().unwrap();
    let mut doc = Document::new("OUTSIDE-SECRET-TITLE", "x", Timestamp::from_epoch_secs(NOW));
    write_document(&outside.path().join("2026-08-x.checklist.json"), &mut doc).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join(ACTIONS_REL)).unwrap();

    let escaping = format!("{ACTIONS_REL}/2026-08-x{CHECKLIST_SUFFIX}");
    let (code, out, err) = run_ok(&root, &["render-md", &escaping]);
    assert_eq!(code, ExitCode::from(2));
    assert!(err.contains("outside the repository"), "{err}");
    assert!(!out.contains("OUTSIDE-SECRET") && !err.contains("OUTSIDE-SECRET"));

    // The same repository, entered through a link to its root.
    fs::remove_file(root.join(ACTIONS_REL)).unwrap();
    let a = write_checklist(&root, "2026-08-a", 0);
    let links = TempDir::new().unwrap();
    let via = links.path().join("repo");
    std::os::unix::fs::symlink(dir.path(), &via).unwrap();
    let (code, out, err) = run_ok(&via, &["verify", &a]);
    assert_eq!(code, ExitCode::SUCCESS, "{err}");
    assert!(out.contains(&format!("{a} is valid")), "{out}");

    // `git` already hands back a physical root, so the entry point alone
    // cannot show the root being canonicalized too; the loader is asked
    // directly with the root spelled through the link.
    let loaded = load_explicit(&via, &[a.clone()], &mut Vec::new()).unwrap();
    assert_eq!(loaded.map(|docs| docs.len()), Some(1));
}

/// The explicit route neither reads nor writes the pointer: one naming a
/// document that does not exist changes nothing about which file is read, and
/// is left byte-for-byte as it was.
#[test]
fn the_explicit_route_never_reads_or_writes_the_pointer() {
    let (_dir, root) = initialized();
    let pointer = pointer_path(&root);
    fs::write(
        &pointer,
        r#"{"path":"docs/actions/2026-08-gone.checklist.json","slug":"2026-08-gone","recorded_at":"2026-08-30T12:00:00Z"}"#,
    )
    .unwrap();
    let before = fs::read(&pointer).unwrap();
    let rel = format!("{ACTIONS_REL}/{STEM}{CHECKLIST_SUFFIX}");

    let (code, out, _) = run_ok(&root, &["verify", &rel]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.contains(&format!("{rel} is valid")), "{out}");
    let (code, _, _) = run_ok(&root, &["render-md", &rel]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(fs::read(&pointer).unwrap(), before);
}

/// Any `-`-prefixed token the verb does not know is a usage error, never a
/// path — and `--max-bytes` belongs to `render-md` alone, needs a number, and
/// refuses one too small to hold the marker the budget guarantees.
#[test]
fn unknown_flags_and_malformed_budgets_are_usage_errors() {
    let (_dir, root) = ignored_repo();
    for argv in [
        vec!["verify", "--bogus"],
        vec![
            "render-md",
            "--bogus",
            "docs/actions/2026-08-a.checklist.json",
        ],
        vec!["verify", "--max-bytes", "60000"],
        vec!["render-md", "--max-bytes"],
        vec!["render-md", "--max-bytes", "lots"],
        vec!["render-md", "--max-bytes", "10"],
        vec!["render-md", "--max-bytes", "60000", "--max-bytes", "60000"],
        vec!["list", "docs/actions/2026-08-a.checklist.json"],
    ] {
        assert!(run_argv(&root, &argv).is_err(), "{argv:?} must be refused");
    }
    let message = run_argv(&root, &["verify", "--bogus"]).unwrap_err();
    assert!(message.contains("--bogus"), "{message}");

    let max = (MARKER_RESERVE + 1).to_string();
    let files = |sub: Sub| match sub {
        Sub::RenderMd { files, max_bytes } => (files, max_bytes),
        other => panic!("{other:?}"),
    };
    let ParsedSub::Run(sub) = parse(
        &[
            "render-md",
            "--max-bytes",
            &max,
            "docs/actions/a.checklist.json",
        ]
        .map(String::from),
    ) else {
        panic!("the documented flag order must parse");
    };
    assert_eq!(
        files(sub),
        (
            vec!["docs/actions/a.checklist.json".to_string()],
            Some(MARKER_RESERVE + 1)
        )
    );
}

/// With no FILE the verbs keep today's route: two documents and no pointer is
/// still the ambiguity it always was.
#[test]
fn with_no_paths_and_no_pointer_two_documents_are_still_ambiguous() {
    let (_dir, root) = ignored_repo();
    write_checklist(&root, "2026-08-a", 0);
    write_checklist(&root, "2026-08-b", 0);

    for argv in [["verify"], ["render-md"]] {
        let args: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        let ParsedSub::Run(sub) = parse(&args) else {
            panic!("{argv:?}")
        };
        let err = run_core_with(&root, &sub, NOW, &mut Vec::new(), &mut Vec::new()).unwrap_err();
        assert!(format!("{err:#}").contains("holds several"), "{err:#}");
    }
}

/// With no FILE and a pointer, the pointed document is the one used.
#[test]
fn with_no_paths_the_pointer_still_picks_the_document() {
    let (_dir, root) = ignored_repo();
    write_checklist(&root, "2026-08-a", 0);
    assert_eq!(init(&root, "2026-08-b"), ExitCode::SUCCESS);
    let b = format!("{ACTIONS_REL}/2026-08-b{CHECKLIST_SUFFIX}");

    let (code, out, _) = run_ok(&root, &["verify"]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.contains(&format!("{b} is valid")), "{out}");

    let (code, out, _) = run_ok(&root, &["render-md"]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(out, single_render(&root, &b));
}
