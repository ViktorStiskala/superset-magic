//! Family-level tests: the three modules used together, the way a verb will
//! use them — create or read a document, put it in canonical order, validate
//! it, write it back.

use std::fs;

use super::*;

fn item(id: &str, created: &str) -> Item {
    Item {
        id: id.to_string(),
        title: format!("item {id}"),
        created: Timestamp::new(created),
        steps: vec!["do the thing".to_string()],
        expected: Some(Some("the thing is done".to_string())),
        ..Item::default()
    }
}

#[test]
fn a_checklist_is_authored_where_no_docs_actions_directory_exists() {
    // The repository has nothing at all under `docs/`, which is the ordinary
    // starting state.
    let repo = tempfile::tempdir().unwrap();
    let path = repo
        .path()
        .join("docs/actions/2026-08-ship-the-thing.checklist.json");
    assert!(!path.parent().unwrap().exists());

    let mut doc = Document::new(
        "Ship the thing",
        "2026-08-ship-the-thing",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );
    doc.sections[0]
        .items
        .push(item("check-dns", "2026-08-29T07:00:00Z"));
    canonicalize(&mut doc);

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, to_json(&doc).unwrap()).unwrap();

    let read_back = read_document(&path).unwrap().unwrap();
    assert_eq!(read_back, doc);
    assert!(
        validate(&read_back).is_empty(),
        "{:?}",
        validate(&read_back)
    );

    // And it stays hand-editable: an ordinary text edit of the file — the
    // fallback whenever the binary is not installed to deny one — parses and
    // validates like anything the CLI wrote.
    let edited = fs::read_to_string(&path).unwrap().replace(
        "\"title\": \"item check-dns\"",
        "\"title\": \"check the DNS record\"",
    );
    fs::write(&path, edited).unwrap();

    let after_edit = read_document(&path).unwrap().unwrap();
    assert!(
        validate(&after_edit).is_empty(),
        "{:?}",
        validate(&after_edit)
    );
    assert_eq!(
        after_edit.sections[0].items[0].title,
        "check the DNS record"
    );
}

#[test]
fn a_project_that_declares_no_sections_gets_the_binary_default_set() {
    let doc = Document::new(
        "Ship it",
        "2026-08-ship-it",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );

    let ids: Vec<&str> = doc.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["verification", "rollout", "decisions", "follow-ups"]);
    assert_eq!(ids.len(), default_sections().len(), "nothing is appended");

    // Nothing resembling a fixed trailing release-approval block: the last
    // section is the default set's own last entry, and every section is empty.
    assert!(doc.sections.iter().all(|s| s.items.is_empty()));
    assert!(validate(&doc).iter().all(|f| f.severity != Severity::Error));
}

#[test]
fn a_project_that_declares_its_own_sections_keeps_that_order_through_a_round_trip() {
    let declared = vec![
        Section {
            id: "prep".into(),
            title: "Preparation".into(),
            ..Section::default()
        },
        Section {
            id: "cutover".into(),
            title: "Cutover".into(),
            ..Section::default()
        },
        Section {
            id: "aftercare".into(),
            title: "Aftercare".into(),
            ..Section::default()
        },
    ];
    let mut doc = Document::with_sections(
        "Move the database",
        "2026-08-move-the-database",
        Timestamp::new("2026-08-29T07:00:00Z"),
        declared,
    );
    doc.sections[1]
        .items
        .push(item("swap-dns", "2026-08-29T07:00:00Z"));

    canonicalize(&mut doc);
    let reread = from_json(&to_json(&doc).unwrap()).unwrap();

    let ids: Vec<&str> = reread.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        ["prep", "cutover", "aftercare"],
        "the declared order is the render order, and nothing is appended to it"
    );
    assert!(validate(&reread)
        .iter()
        .all(|f| f.severity != Severity::Error));
}

#[test]
fn a_document_written_across_two_timezones_orders_by_instant() {
    // The same working day recorded by two contributors, one on UTC and one on
    // UTC+2. Read as text these sort into the wrong order; read as instants
    // they interleave correctly.
    let mut doc = Document::new(
        "Ship it",
        "2026-08-ship-it",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );
    doc.sections[0].items = vec![
        item("third", "2026-08-29T10:00:00Z"),
        item("first", "2026-08-29T08:30:00+02:00"), // 06:30Z
        item("second", "2026-08-29T09:00:00Z"),
    ];
    doc.changelog = vec![
        ChangelogEntry {
            id: "later-note".into(),
            created: Timestamp::new("2026-08-29T10:00:00Z"),
            summary: "second thoughts".into(),
            ..ChangelogEntry::default()
        },
        ChangelogEntry {
            id: "earlier-note".into(),
            created: Timestamp::new("2026-08-29T08:30:00+02:00"),
            summary: "first thoughts".into(),
            ..ChangelogEntry::default()
        },
    ];

    canonicalize(&mut doc);

    let items: Vec<&str> = doc.sections[0]
        .items
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(items, ["first", "second", "third"]);
    let entries: Vec<&str> = doc.changelog.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(entries, ["earlier-note", "later-note"]);
}

#[test]
fn canonicalizing_and_writing_an_unchanged_document_is_byte_stable() {
    let mut doc = Document::new(
        "Ship it",
        "2026-08-ship-it",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );
    doc.sections[0].items = vec![
        item("beta", "2026-08-29T08:30:00+02:00"),
        item("alpha", "2026-08-29T09:00:00Z"),
    ];
    canonicalize(&mut doc);

    let once = to_json(&doc).unwrap();
    let mut again = from_json(&once).unwrap();
    canonicalize(&mut again);
    let twice = to_json(&again).unwrap();

    assert_eq!(once, twice, "a no-op write must produce no diff");
}

#[test]
fn the_checklist_family_depends_on_nothing_from_the_hook_layer() {
    // The dependency direction is the one the conclusion cache follows: the
    // gate that denies a direct read of a checklist is a CALLER of this
    // family, never the other way round. Teaching the schema about tool
    // envelopes would make it untestable without a harness payload, so the
    // check is on the imports themselves.
    for (name, source) in [
        ("mod.rs", include_str!("mod.rs")),
        ("schema.rs", include_str!("schema.rs")),
        ("order.rs", include_str!("order.rs")),
        ("validate.rs", include_str!("validate.rs")),
    ] {
        for line in source.lines().map(str::trim) {
            if line.starts_with("use ") {
                assert!(
                    !line.contains("hook"),
                    "{name} imports from the hook layer: {line}"
                );
            }
        }
    }
}
