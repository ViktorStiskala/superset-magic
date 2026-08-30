//! The validator earns its keep on the cases the format cannot express, so
//! that is what these tests are: a done item with nothing saying when, a null
//! expectation on the one kind that may not have one, an id used twice in two
//! different kinds of record, and a reference that only resolves inside the
//! repository it was written in.
//!
//! Each test starts from a document that passes cleanly and breaks exactly one
//! thing, so a finding can only come from the change under test.

use super::super::schema::{from_json, to_json, ItemKind};
use super::*;

fn valid_item(id: &str) -> Item {
    Item {
        id: id.to_string(),
        title: format!("item {id}"),
        created: Timestamp::new("2026-08-29T07:00:00Z"),
        steps: vec!["run the command".to_string()],
        expected: Some(Some("it succeeds".to_string())),
        ..Item::default()
    }
}

fn valid_doc() -> Document {
    let mut doc = Document::new(
        "Ship the thing",
        "2026-08-ship-the-thing",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );
    doc.changelog.push(ChangelogEntry {
        id: "opened".to_string(),
        created: Timestamp::new("2026-08-29T07:00:00Z"),
        summary: "checklist opened".to_string(),
        ..ChangelogEntry::default()
    });
    doc.sections[0].items.push(valid_item("check-dns"));
    doc
}

/// Every error message, so an assertion can look for the substring that names
/// the defect rather than depending on the exact wording.
fn errors(doc: &Document) -> Vec<String> {
    validate(doc)
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .map(|f| format!("{} {}", f.location, f.message))
        .collect()
}

fn warnings(doc: &Document) -> Vec<String> {
    validate(doc)
        .into_iter()
        .filter(|f| f.severity == Severity::Warning)
        .map(|f| format!("{} {}", f.location, f.message))
        .collect()
}

fn assert_one_error_mentioning(doc: &Document, needle: &str) {
    let found = errors(doc);
    assert_eq!(found.len(), 1, "expected exactly one error, got {found:?}");
    assert!(
        found[0].contains(needle),
        "{found:?} should mention `{needle}`"
    );
}

#[test]
fn a_well_formed_document_has_nothing_to_report() {
    let doc = valid_doc();
    let findings = validate(&doc);
    assert!(findings.is_empty(), "{findings:?}");
    assert!(!has_errors(&findings));
}

// ── AE73 ──────────────────────────────────────────────────────────────────────

#[test]
fn a_done_item_with_no_timestamp_and_a_null_expectation_on_a_check_are_both_reported() {
    let mut doc = valid_doc();
    let mut done = valid_item("rollback-tested");
    done.done = true;
    done.completed = None;
    let mut unchecked = valid_item("nothing-to-check");
    unchecked.kind = ItemKind::Check;
    unchecked.expected = Some(None);
    doc.sections[0].items.push(done);
    doc.sections[0].items.push(unchecked);

    let findings = validate(&doc);
    assert!(has_errors(&findings), "verify must exit non-zero on these");

    let found = errors(&doc);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .any(|m| m.contains("rollback-tested") && m.contains("completion timestamp")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("nothing-to-check") && m.contains("`expected` is null")),
        "{found:?}"
    );
}

#[test]
fn a_null_expectation_is_fine_on_a_record_or_a_decision() {
    for kind in [ItemKind::Record, ItemKind::Decision] {
        let mut doc = valid_doc();
        let mut it = valid_item("observed-value");
        it.kind = kind;
        it.expected = Some(None);
        doc.sections[0].items.push(it);

        assert!(
            errors(&doc).is_empty(),
            "{kind:?} may declare no expectation"
        );
    }
}

#[test]
fn an_absent_expectation_is_a_shape_warning_and_a_check_kind_error() {
    // The two are distinguishable on purpose: an absent key is a defect the
    // next CLI write repairs, while an undeclared expectation on a check is a
    // verification that can never fail.
    let mut doc = valid_doc();
    let mut it = valid_item("check-dns-2");
    it.expected = None;
    doc.sections[0].items.push(it);

    assert_one_error_mentioning(&doc, "`expected` is absent on a check-kind item");
    assert!(
        warnings(&doc)
            .iter()
            .any(|m| m.contains("`expected` is absent")),
        "{:?}",
        warnings(&doc)
    );

    // On a record the same absence is only the shape warning.
    let mut doc = valid_doc();
    let mut it = valid_item("a-measurement");
    it.kind = ItemKind::Record;
    it.expected = None;
    doc.sections[0].items.push(it);
    assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
    assert_eq!(warnings(&doc).len(), 1);
}

#[test]
fn a_completion_timestamp_on_an_unfinished_item_is_a_warning() {
    let mut doc = valid_doc();
    let mut it = valid_item("half-edited");
    it.done = false;
    it.completed = Some(Timestamp::new("2026-08-29T08:00:00Z"));
    doc.sections[0].items.push(it);

    assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
    assert!(warnings(&doc)[0].contains("`done` is false"));
}

#[test]
fn an_unreadable_completion_timestamp_is_an_error() {
    let mut doc = valid_doc();
    let mut it = valid_item("finished");
    it.done = true;
    it.completed = Some(Timestamp::new("2026-08-29T08:00:00"));
    doc.sections[0].items.push(it);

    assert_one_error_mentioning(&doc, "no UTC offset");
}

// ── Ids ───────────────────────────────────────────────────────────────────────

#[test]
fn ids_are_unique_across_the_changelog_and_every_section_jointly() {
    // The two records are of different kinds and live in different arrays,
    // which is exactly the collision a per-array uniqueness check misses.
    let mut doc = valid_doc();
    doc.sections[0].items.push(valid_item("opened"));

    assert_one_error_mentioning(&doc, "already used by changelog[opened]");
}

#[test]
fn an_item_may_not_take_a_sections_id() {
    // Stricter than the format requires, because `checklist set <id> …`
    // addresses a record by id alone and an ambiguous id resolves to nothing.
    let mut doc = valid_doc();
    doc.sections[0].items.push(valid_item("verification"));

    assert_one_error_mentioning(&doc, "already used by sections[verification]");
}

#[test]
fn a_malformed_id_is_reported_and_the_reason_is_stated() {
    for bad in [
        "2fa-rollout", // starts with a digit
        "Check-DNS",   // not lowercase
        "check_dns",   // underscore
        "check--dns",  // doubled hyphen
        "check-dns-",  // trailing hyphen
        "check dns",   // space
    ] {
        let mut doc = valid_doc();
        doc.sections[0].items.push(valid_item(bad));
        assert_one_error_mentioning(&doc, "is not well formed");
        assert!(!is_well_formed_id(bad), "`{bad}` should not be well formed");
    }

    for good in ["a", "check-dns", "check-2fa", "rollback-plan-v2"] {
        assert!(is_well_formed_id(good), "`{good}` should be well formed");
    }
}

#[test]
fn a_record_with_no_id_is_reported_at_a_location_that_still_points_somewhere() {
    let mut doc = valid_doc();
    let mut it = valid_item("");
    it.id = String::new();
    doc.sections[0].items.push(it);

    let found = errors(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("items[<no id>]"), "{found:?}");
}

// ── Steps, references and the header ──────────────────────────────────────────

#[test]
fn an_item_with_no_action_step_is_an_error() {
    let mut doc = valid_doc();
    let mut it = valid_item("a-wish");
    it.steps = Vec::new();
    doc.sections[0].items.push(it);
    assert_one_error_mentioning(&doc, "no action step");

    // Whitespace is not a step either.
    let mut doc = valid_doc();
    let mut it = valid_item("a-wish");
    it.steps = vec!["   ".to_string()];
    doc.sections[0].items.push(it);
    assert_one_error_mentioning(&doc, "no action step");
}

#[test]
fn a_relative_reference_is_an_error_and_an_absolute_one_is_not() {
    let mut doc = valid_doc();
    let mut it = valid_item("with-refs");
    it.refs = vec![Reference {
        label: "the runbook".to_string(),
        url: "docs/runbook.md".to_string(),
        ..Reference::default()
    }];
    doc.sections[0].items.push(it);

    assert_one_error_mentioning(&doc, "not an absolute URL");

    for url in [
        "https://example.com/runbook",
        "http://example.com",
        "ssh://git@example.com/repo",
    ] {
        assert!(is_absolute_url(url), "`{url}` is absolute");
    }
    for url in [
        "docs/runbook.md",
        "./runbook.md",
        "/etc/hosts",
        "#anchor",
        "",
        "://x",
    ] {
        assert!(!is_absolute_url(url), "`{url}` is not absolute");
    }
}

#[test]
fn a_reference_with_no_label_still_renders_and_is_only_a_warning() {
    let mut doc = valid_doc();
    let mut it = valid_item("with-refs");
    it.refs = vec![Reference {
        label: String::new(),
        url: "https://example.com/runbook".to_string(),
        ..Reference::default()
    }];
    doc.sections[0].items.push(it);

    assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
    assert!(warnings(&doc)[0].contains("bare URL"));
}

#[test]
fn required_header_fields_are_reported_rather_than_refused_at_parse_time() {
    // A hand-edited file that dropped several keys still parses, so the author
    // gets the whole list at once instead of one error per run.
    let doc: Document = from_json("{}").unwrap();
    let found = errors(&doc);

    for expected in ["`$schema`", "`title`", "`slug`", "`created`", "`updated`"] {
        assert!(
            found.iter().any(|m| m.contains(expected)),
            "{expected} missing from {found:?}"
        );
    }
    assert!(
        warnings(&doc).iter().any(|m| m.contains("no sections")),
        "an empty document renders as an empty checklist"
    );
}

#[test]
fn a_schema_value_that_is_a_path_is_refused_and_a_different_version_only_warns() {
    let mut doc = valid_doc();
    doc.schema = "../../schemas/checklist.json".to_string();
    assert_one_error_mentioning(&doc, "path rather than an identifier");

    let mut doc = valid_doc();
    doc.schema = "https://github.com/ViktorStiskala/superset-magic/schema/checklist/v9".to_string();
    assert!(errors(&doc).is_empty(), "a newer format is readable");
    assert!(warnings(&doc)[0].contains("different checklist format"));
}

#[test]
fn a_declared_but_empty_section_set_is_a_warning_not_an_error() {
    // A project that deliberately declares no sections gets what it declared;
    // nothing is substituted for it and no approval block is appended.
    let mut doc = valid_doc();
    doc.changelog.clear();
    doc.sections.clear();

    assert!(errors(&doc).is_empty(), "{:?}", errors(&doc));
    assert_eq!(warnings(&doc).len(), 1);
}

#[test]
fn a_timestamp_with_no_offset_anywhere_is_an_error() {
    let mut doc = valid_doc();
    doc.created = Timestamp::new("2026-08-29T07:00:00");
    assert_one_error_mentioning(&doc, "no UTC offset");
}

#[test]
fn findings_read_as_one_line_naming_the_record() {
    let mut doc = valid_doc();
    let mut it = valid_item("a-wish");
    it.steps = Vec::new();
    doc.sections[0].items.push(it);

    let rendered = validate(&doc)[0].to_string();
    assert!(
        rendered.starts_with("error sections[verification].items[a-wish]: "),
        "{rendered}"
    );
}

#[test]
fn validation_reads_the_document_and_writes_nothing() {
    // The validator is a pure function over the parsed document: `verify` owns
    // the printing and the exit code, so nothing here may mutate or persist.
    let doc = valid_doc();
    let before = to_json(&doc).unwrap();
    let _ = validate(&doc);
    assert_eq!(to_json(&doc).unwrap(), before);
}
