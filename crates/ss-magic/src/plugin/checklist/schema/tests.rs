//! Two things are worth testing here and the rest is serde doing its job.
//!
//! The first is the ISO-8601 reader, because the whole reason it exists is
//! that a string comparison passes a naive test: any two timestamps written in
//! one timezone sort correctly as text, so a test that only ever writes `Z`
//! proves nothing. Every ordering test below therefore mixes offsets on
//! purpose.
//!
//! The second is the pair of optionality conventions. They both compile to
//! `Option<T>` and differ only in what reaches the file, so the assertions are
//! on the SERIALIZED bytes, in both directions.

use serde_json::json;

use super::*;

/// A minimal, well-formed item.
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

// ── ISO-8601 reading ──────────────────────────────────────────────────────────

#[test]
fn two_spellings_of_one_instant_are_equal() {
    let utc = parse_iso8601("2026-08-29T07:00:00Z").unwrap();
    let prague = parse_iso8601("2026-08-29T09:00:00+02:00").unwrap();
    let honolulu = parse_iso8601("2026-08-28T21:00:00-10:00").unwrap();

    assert_eq!(utc, prague);
    assert_eq!(utc, honolulu);
}

#[test]
fn ordering_follows_the_instant_and_not_the_text() {
    // `09:00+02:00` is 07:00Z — an hour EARLIER than `08:00Z` — but sorts
    // after it as text. This is the failure the whole reader exists to avoid.
    let earlier_text = "2026-08-29T09:00:00+02:00";
    let later_text = "2026-08-29T08:00:00Z";
    assert!(
        earlier_text > later_text,
        "the strings sort the wrong way round"
    );

    let earlier = parse_iso8601(earlier_text).unwrap();
    let later = parse_iso8601(later_text).unwrap();
    assert!(earlier < later, "the instants sort correctly");
}

#[test]
fn every_offset_form_reads_the_same() {
    let expected = parse_iso8601("2026-08-29T07:00:00Z").unwrap();

    for spelling in [
        "2026-08-29T09:00:00+02:00",
        "2026-08-29T09:00:00+0200",
        "2026-08-29T09:00:00+02",
        "2026-08-29t09:00:00+02:00",
    ] {
        assert_eq!(parse_iso8601(spelling).unwrap(), expected, "{spelling}");
    }
    assert_eq!(parse_iso8601("2026-08-29T07:00:00z").unwrap(), expected);
}

#[test]
fn a_timestamp_with_no_offset_is_refused() {
    // Well-shaped, and the one spelling that looks right while naming no
    // instant at all.
    assert_eq!(
        parse_iso8601("2026-08-29T07:00:00"),
        Err(TimeError::NoOffset)
    );
    assert_eq!(
        parse_iso8601("2026-08-29T07:00:00.5"),
        Err(TimeError::NoOffset)
    );
}

#[test]
fn end_of_day_and_leap_seconds_are_refused_rather_than_guessed() {
    // Both are legal ISO-8601 spellings with no ordinary instant. Mapping them
    // onto a neighbouring second would make ordering depend on which
    // neighbour was chosen, so they are reported instead.
    assert_eq!(
        parse_iso8601("2026-08-29T24:00:00Z"),
        Err(TimeError::OutOfRange("hour"))
    );
    assert_eq!(
        parse_iso8601("2026-12-31T23:59:60Z"),
        Err(TimeError::OutOfRange("second"))
    );
}

#[test]
fn out_of_range_components_are_named() {
    assert_eq!(
        parse_iso8601("2026-13-01T00:00:00Z"),
        Err(TimeError::OutOfRange("month"))
    );
    assert_eq!(
        parse_iso8601("2026-02-30T00:00:00Z"),
        Err(TimeError::OutOfRange("day"))
    );
    assert_eq!(
        parse_iso8601("2026-00-10T00:00:00Z"),
        Err(TimeError::OutOfRange("month"))
    );
    // 2026 is not a leap year; 2024 is.
    assert_eq!(
        parse_iso8601("2026-02-29T00:00:00Z"),
        Err(TimeError::OutOfRange("day"))
    );
    assert!(parse_iso8601("2024-02-29T00:00:00Z").is_ok());
}

#[test]
fn malformed_spellings_are_refused() {
    for spelling in [
        "",
        "yesterday",
        "2026-08-29",
        "2026-08-29T07:00Z", // no seconds
        "2026/08/29T07:00:00Z",
        "2026-08-29 07:00:00Z",  // space separator
        "2026-08-29T07:00:00.Z", // a dot with no digits
        "20260829T070000Z",      // basic format
        "2026-08-29T07:00:00Ω",
    ] {
        assert!(
            parse_iso8601(spelling).is_err(),
            "`{spelling}` should not parse"
        );
    }
}

#[test]
fn an_unreadable_offset_is_reported_as_such() {
    assert_eq!(
        parse_iso8601("2026-08-29T07:00:00+99:00"),
        Err(TimeError::BadOffset)
    );
    assert_eq!(
        parse_iso8601("2026-08-29T07:00:00+2:00"),
        Err(TimeError::BadOffset)
    );
    assert_eq!(
        parse_iso8601("2026-08-29T07:00:00 UTC"),
        Err(TimeError::BadOffset)
    );
}

#[test]
fn fractional_seconds_break_a_tie_and_never_reorder_a_second() {
    let whole = parse_iso8601("2026-08-29T07:00:00Z").unwrap();
    let half = parse_iso8601("2026-08-29T07:00:00.5Z").unwrap();
    let next = parse_iso8601("2026-08-29T07:00:01Z").unwrap();

    assert!(whole < half && half < next);
    assert_eq!(half.nanos, 500_000_000);
    // Finer than a nanosecond truncates rather than rounding up into the next
    // second.
    assert_eq!(
        parse_iso8601("2026-08-29T07:00:00.9999999999Z")
            .unwrap()
            .nanos,
        999_999_999
    );
}

#[test]
fn the_reader_inverts_the_formatter_it_shares_a_crate_with() {
    // Every value ss-magic itself writes goes through `format_rfc3339`, so the
    // pair has to round-trip exactly — including across a leap day and a
    // century boundary.
    for secs in [
        0u64,
        1,
        86_399,
        951_782_400,
        1_709_164_800,
        1_756_454_400,
        2_147_483_647,
    ] {
        let formatted = format_rfc3339(secs);
        let parsed = parse_iso8601(&formatted)
            .unwrap_or_else(|err| panic!("{formatted} did not parse: {err}"));
        assert_eq!(parsed.secs, secs as i64, "{formatted}");
        assert_eq!(parsed.nanos, 0);
    }
}

#[test]
fn a_timestamp_keeps_the_spelling_it_was_given() {
    // Round-tripping must not normalize somebody's `+02:00` into `Z`: the file
    // is reviewed as a diff, and a rewrite that touches every timestamp hides
    // the change that actually happened.
    let ts = Timestamp::new("2026-08-29T09:00:00+02:00");
    let json = serde_json::to_string(&ts).unwrap();
    assert_eq!(json, "\"2026-08-29T09:00:00+02:00\"");
    assert_eq!(
        serde_json::from_str::<Timestamp>(&json).unwrap().as_str(),
        "2026-08-29T09:00:00+02:00"
    );
}

// ── The two optionality conventions ───────────────────────────────────────────

#[test]
fn an_unset_priority_is_omitted_and_an_unset_expectation_is_written_as_null() {
    let mut it = item("check-dns", "2026-08-29T07:00:00Z");
    it.kind = ItemKind::Record;
    it.priority = None;
    it.why = None;
    it.expected = Some(None);
    it.completed = None;

    let value = serde_json::to_value(&it).unwrap();
    let object = value.as_object().unwrap();

    assert!(
        !object.contains_key("priority"),
        "an unranked item writes no key"
    );
    assert!(!object.contains_key("why"));
    assert!(!object.contains_key("refs"));
    assert!(!object.contains_key("description"));
    assert_eq!(object.get("expected"), Some(&Value::Null));
    assert_eq!(object.get("completed"), Some(&Value::Null));
}

#[test]
fn a_set_priority_is_written() {
    let mut it = item("check-dns", "2026-08-29T07:00:00Z");
    it.priority = Some(Priority::DecisionBlocking);

    let value = serde_json::to_value(&it).unwrap();
    assert_eq!(value["priority"], json!("decision-blocking"));
}

#[test]
fn an_explicit_null_priority_is_normalized_to_an_absent_key() {
    // `"priority": null` reads as unranked, and the next write drops the key,
    // so "unranked" never looks like a value the ordering rule has to rank.
    let it: Item = serde_json::from_value(json!({
        "id": "check-dns",
        "title": "check dns",
        "priority": null,
        "created": "2026-08-29T07:00:00Z",
        "steps": ["dig"],
        "expected": "an A record"
    }))
    .unwrap();

    assert_eq!(it.priority, None);
    let value = serde_json::to_value(&it).unwrap();
    assert!(!value.as_object().unwrap().contains_key("priority"));
}

#[test]
fn an_absent_expectation_is_distinguishable_from_an_explicit_null() {
    let absent: Item = serde_json::from_value(json!({
        "id": "a-record",
        "title": "record it",
        "kind": "record",
        "created": "2026-08-29T07:00:00Z",
        "steps": ["write it down"]
    }))
    .unwrap();
    let null: Item = serde_json::from_value(json!({
        "id": "a-record",
        "title": "record it",
        "kind": "record",
        "created": "2026-08-29T07:00:00Z",
        "steps": ["write it down"],
        "expected": null
    }))
    .unwrap();

    assert!(!absent.expected_declared(), "no key at all");
    assert!(null.expected_declared(), "a key holding null");
    assert_eq!(absent.expected_text(), None);
    assert_eq!(null.expected_text(), None);

    // Both write the key, so the next read of either sees a declared null.
    for it in [absent, null] {
        let value = serde_json::to_value(&it).unwrap();
        assert_eq!(value.get("expected"), Some(&Value::Null));
    }
}

#[test]
fn a_missing_kind_reads_as_the_strictest_one() {
    // Fail-closed: an item with no declared kind is a check, the only kind
    // that may not leave `expected` null, so a dropped key can never weaken
    // what `verify` enforces.
    let it: Item = serde_json::from_value(json!({
        "id": "check-dns",
        "title": "check dns",
        "created": "2026-08-29T07:00:00Z",
        "steps": ["dig"],
        "expected": null
    }))
    .unwrap();

    assert_eq!(it.kind, ItemKind::Check);
    assert!(!it.kind.allows_null_expectation());
}

#[test]
fn a_kind_this_build_does_not_know_is_a_loud_parse_error() {
    // The closed set is deliberate: guessing at an unknown kind would decide
    // whether a null expectation is legal, so serde names the field instead.
    let err = serde_json::from_value::<Item>(json!({
        "id": "check-dns",
        "title": "check dns",
        "kind": "audit",
        "created": "2026-08-29T07:00:00Z",
        "steps": ["dig"],
        "expected": null
    }))
    .unwrap_err()
    .to_string();

    assert!(err.contains("audit"), "{err}");
}

// ── Unknown keys ──────────────────────────────────────────────────────────────

#[test]
fn unknown_keys_survive_a_round_trip_at_every_level() {
    // The file is hand-editable and a newer ss-magic may have written keys
    // this build has never heard of. Dropping them silently is the defect
    // `magic.json` already fixes; rejecting them would make an older binary
    // refuse a document it can render perfectly well.
    let raw = serde_json::to_string_pretty(&json!({
        "$schema": SCHEMA_ID,
        "title": "Ship the thing",
        "slug": "2026-08-ship-the-thing",
        "created": "2026-08-29T07:00:00Z",
        "updated": "2026-08-29T07:00:00Z",
        "owner": "someone-else",
        "changelog": [{
            "id": "first-note",
            "created": "2026-08-29T07:00:00Z",
            "summary": "started",
            "unreleased_field": 7
        }],
        "sections": [{
            "id": "verification",
            "title": "Verification",
            "collapsed": true,
            "items": [{
                "id": "check-dns",
                "title": "check dns",
                "created": "2026-08-29T07:00:00Z",
                "steps": ["dig"],
                "expected": "an A record",
                "assignee": "nobody"
            }]
        }]
    }))
    .unwrap();

    let doc = from_json(&raw).unwrap();
    assert_eq!(doc.extras.get("owner"), Some(&json!("someone-else")));
    assert_eq!(doc.sections[0].extras.get("collapsed"), Some(&json!(true)));
    assert_eq!(
        doc.sections[0].items[0].extras.get("assignee"),
        Some(&json!("nobody"))
    );
    assert_eq!(
        doc.changelog[0].extras.get("unreleased_field"),
        Some(&json!(7))
    );

    // Everything that was written stays written, at every level. The rewrite
    // does ADD the always-present keys a document is supposed to carry
    // (`kind`, `done`, `completed`), which is the writer repairing the shape
    // rather than inventing content — asserted separately below so the two
    // cannot be confused.
    let written: Value = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    let original: Value = serde_json::from_str(&raw).unwrap();
    assert_contains(&original, &written, "$");

    let item = &written["sections"][0]["items"][0];
    assert_eq!(item["kind"], json!("check"));
    assert_eq!(item["done"], json!(false));
    assert_eq!(item["completed"], Value::Null);
}

/// Assert every key and value of `subset` appears in `full`, recursively.
fn assert_contains(subset: &Value, full: &Value, at: &str) {
    match (subset, full) {
        (Value::Object(subset), Value::Object(full)) => {
            for (key, value) in subset {
                let found = full
                    .get(key)
                    .unwrap_or_else(|| panic!("{at}.{key} was dropped"));
                assert_contains(value, found, &format!("{at}.{key}"));
            }
        }
        (Value::Array(subset), Value::Array(full)) => {
            assert_eq!(subset.len(), full.len(), "{at} changed length");
            for (index, value) in subset.iter().enumerate() {
                assert_contains(value, &full[index], &format!("{at}[{index}]"));
            }
        }
        (subset, full) => assert_eq!(subset, full, "{at} changed"),
    }
}

#[test]
fn a_value_of_the_wrong_type_is_still_a_parse_error() {
    // Permissive about absence, strict about type: no validator can guess what
    // a string was supposed to mean where a list belongs.
    let err = serde_json::from_value::<Document>(json!({
        "title": "Ship the thing",
        "sections": "verification"
    }))
    .unwrap_err()
    .to_string();

    assert!(
        err.contains("sections") || err.contains("sequence"),
        "{err}"
    );
}

// ── Serialization shape ───────────────────────────────────────────────────────

#[test]
fn serializing_is_stable_and_pretty_printed_with_a_trailing_newline() {
    let mut doc = Document::new(
        "Ship the thing",
        "2026-08-ship-the-thing",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );
    doc.sections[0]
        .items
        .push(item("check-dns", "2026-08-29T07:00:00Z"));

    let once = to_json(&doc).unwrap();
    let twice = to_json(&from_json(&once).unwrap()).unwrap();

    assert_eq!(
        once, twice,
        "a rewrite of an unchanged document is byte-identical"
    );
    assert!(once.ends_with("}\n"));
    assert!(once.contains("\n  \"title\": \"Ship the thing\""));
    assert!(once.starts_with("{\n  \"$schema\""));
}

// ── Reading from disk ─────────────────────────────────────────────────────────

#[test]
fn reading_an_absent_file_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_document(&dir.path().join("nothing.json"))
        .unwrap()
        .is_none());
}

#[test]
fn reading_a_directory_or_malformed_json_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_document(dir.path()).unwrap_err().to_string();
    assert!(err.contains("not a regular file"), "{err}");

    let path = dir.path().join("broken.checklist.json");
    fs::write(&path, "{ not json").unwrap();
    let err = format!("{:#}", read_document(&path).unwrap_err());
    assert!(err.contains("broken.checklist.json"), "{err}");
}

#[test]
fn a_document_round_trips_through_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-08-ship.checklist.json");
    let doc = Document::new(
        "Ship the thing",
        "2026-08-ship",
        Timestamp::new("2026-08-29T07:00:00Z"),
    );

    fs::write(&path, to_json(&doc).unwrap()).unwrap();
    assert_eq!(read_document(&path).unwrap().unwrap(), doc);
}
