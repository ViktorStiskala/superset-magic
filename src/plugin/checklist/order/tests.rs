//! Every fixture here mixes UTC offsets on purpose.
//!
//! A checklist written in one timezone sorts identically whether the code
//! compares instants or strings, so a single-offset fixture would pass against
//! the exact bug this module exists to prevent. The ordering assertions below
//! are all built so that the lexical order and the instant order DISAGREE.

use super::super::schema::{Priority, Section};
use super::*;

fn doc_with(sections: Vec<Section>, changelog: Vec<ChangelogEntry>) -> Document {
    Document {
        changelog,
        sections,
        ..Document::default()
    }
}

fn entry(id: &str, created: &str) -> ChangelogEntry {
    ChangelogEntry {
        id: id.to_string(),
        created: Timestamp::new(created),
        summary: format!("entry {id}"),
        ..ChangelogEntry::default()
    }
}

fn item(id: &str, created: &str) -> Item {
    Item {
        id: id.to_string(),
        title: format!("item {id}"),
        created: Timestamp::new(created),
        steps: vec!["do it".to_string()],
        expected: Some(Some("done".to_string())),
        ..Item::default()
    }
}

fn section(id: &str, items: Vec<Item>) -> Section {
    Section {
        id: id.to_string(),
        title: id.to_string(),
        items,
        ..Section::default()
    }
}

fn item_ids(section: &Section) -> Vec<&str> {
    section.items.iter().map(|i| i.id.as_str()).collect()
}

#[test]
fn changelog_entries_ascend_by_instant_and_not_by_text() {
    // `08:30+02:00` is 06:30Z, the earliest of the three, and the LAST of them
    // as a string.
    let mut doc = doc_with(
        Vec::new(),
        vec![
            entry("third", "2026-08-29T09:00:00Z"),
            entry("first", "2026-08-29T08:30:00+02:00"),
            entry("second", "2026-08-29T07:00:00Z"),
        ],
    );

    canonicalize(&mut doc);

    let ids: Vec<&str> = doc.changelog.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["first", "second", "third"]);
}

#[test]
fn items_sort_by_done_then_priority_then_instant() {
    let mut blocking_later = item("blocking-later", "2026-08-29T12:00:00Z");
    blocking_later.priority = Some(Priority::Blocking);
    // 11:00+02:00 is 09:00Z — earlier than the item above, later as a string.
    let mut blocking_earlier = item("blocking-earlier", "2026-08-29T11:00:00+02:00");
    blocking_earlier.priority = Some(Priority::Blocking);
    let mut decision = item("decision", "2026-08-29T06:00:00Z");
    decision.priority = Some(Priority::DecisionBlocking);
    let mut follow_up = item("follow-up", "2026-08-29T05:00:00Z");
    follow_up.priority = Some(Priority::FollowUp);
    let unranked = item("unranked", "2026-08-29T04:00:00Z");
    let mut done = item("done-blocking", "2026-08-29T03:00:00Z");
    done.priority = Some(Priority::Blocking);
    done.done = true;
    done.completed = Some(Timestamp::new("2026-08-29T03:30:00Z"));

    let mut doc = doc_with(
        vec![section(
            "verification",
            vec![
                done.clone(),
                unranked.clone(),
                follow_up.clone(),
                decision.clone(),
                blocking_later.clone(),
                blocking_earlier.clone(),
            ],
        )],
        Vec::new(),
    );

    canonicalize(&mut doc);

    assert_eq!(
        item_ids(&doc.sections[0]),
        [
            // Open work first, most consequential first, oldest first inside a
            // rank — and the unranked item after every ranked one, even though
            // it is the oldest of the open items.
            "blocking-earlier",
            "blocking-later",
            "decision",
            "follow-up",
            "unranked",
            // Done work last, regardless of how blocking it was.
            "done-blocking",
        ]
    );
}

#[test]
fn an_unranked_done_item_still_sorts_after_a_ranked_done_one() {
    let mut ranked = item("ranked", "2026-08-29T09:00:00Z");
    ranked.priority = Some(Priority::FollowUp);
    ranked.done = true;
    let mut unranked = item("unranked", "2026-08-29T01:00:00Z");
    unranked.done = true;

    let mut doc = doc_with(vec![section("s", vec![unranked, ranked])], Vec::new());

    canonicalize(&mut doc);
    assert_eq!(item_ids(&doc.sections[0]), ["ranked", "unranked"]);
}

#[test]
fn section_order_is_left_exactly_as_declared() {
    // The declared order IS the render order. Sorting sections would quietly
    // overrule a project that listed its own.
    let mut doc = doc_with(
        vec![
            section("zulu", vec![item("a-one", "2026-08-29T07:00:00Z")]),
            section("alpha", vec![item("b-two", "2026-08-29T07:00:00Z")]),
        ],
        Vec::new(),
    );

    canonicalize(&mut doc);

    let ids: Vec<&str> = doc.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["zulu", "alpha"]);
}

#[test]
fn an_unreadable_timestamp_sorts_last_instead_of_stopping_the_sort() {
    // One bad value in one item must not leave the other items unarranged;
    // reporting it is validation's job, not ordering's.
    let mut doc = doc_with(
        vec![section(
            "s",
            vec![
                item("broken", "sometime last tuesday"),
                item("good", "2026-08-29T07:00:00Z"),
            ],
        )],
        vec![
            entry("broken-entry", ""),
            entry("good-entry", "2026-08-29T07:00:00Z"),
        ],
    );

    canonicalize(&mut doc);

    assert_eq!(item_ids(&doc.sections[0]), ["good", "broken"]);
    let ids: Vec<&str> = doc.changelog.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["good-entry", "broken-entry"]);
}

#[test]
fn the_canonical_order_depends_on_content_and_not_on_the_starting_arrangement() {
    // Two people whose editors wrote the same records in different orders must
    // produce the same file, so identical sort keys break the tie on the id.
    let same_instant = "2026-08-29T07:00:00Z";
    let forwards = vec![
        item("aaa", same_instant),
        item("bbb", same_instant),
        item("ccc", same_instant),
    ];
    let mut backwards = forwards.clone();
    backwards.reverse();

    let mut a = doc_with(vec![section("s", forwards)], Vec::new());
    let mut b = doc_with(vec![section("s", backwards)], Vec::new());
    canonicalize(&mut a);
    canonicalize(&mut b);

    assert_eq!(item_ids(&a.sections[0]), ["aaa", "bbb", "ccc"]);
    assert_eq!(a, b);
}

#[test]
fn canonicalizing_twice_changes_nothing() {
    let mut doc = doc_with(
        vec![section(
            "s",
            vec![
                item("later", "2026-08-29T09:00:00Z"),
                item("earlier", "2026-08-29T08:30:00+02:00"),
            ],
        )],
        vec![
            entry("b", "2026-08-29T09:00:00Z"),
            entry("a", "2026-08-29T08:30:00+02:00"),
        ],
    );

    canonicalize(&mut doc);
    let once = doc.clone();
    canonicalize(&mut doc);

    assert_eq!(doc, once);
}

#[test]
fn an_unranked_item_ranks_after_every_named_priority() {
    for priority in [
        Priority::Blocking,
        Priority::DecisionBlocking,
        Priority::FollowUp,
    ] {
        assert!(priority.rank() < UNRANKED_RANK, "{priority:?}");
    }
}
