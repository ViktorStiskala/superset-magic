//! Canonical order: the one arrangement a checklist is ever stored in.
//!
//! Every write re-sorts the document, so a diff on a pull request shows what
//! changed rather than where things moved. Two rules, and they are not the
//! same rule:
//!
//! - **Changelog entries ascend by `created`.** Oldest first, the way a log
//!   reads.
//! - **Items within a section sort by `(done, priority rank, created)`**, so
//!   the open work is at the top, the most consequential of it first, and
//!   anything unranked falls to the end of its group.
//!
//! Section order is left alone. The declared order IS the render order, so a
//! project that lists its own sections gets exactly that arrangement; sorting
//! them would quietly overrule the author.
//!
//! ## Why the comparison is on instants
//!
//! Timestamps are compared through [`Timestamp::instant`], never as text.
//! `2026-08-29T09:00:00+02:00` is one hour EARLIER than
//! `2026-08-29T08:00:00Z`, and sorts after it as a string — a defect that a
//! single-timezone repository never sees, and that appears the day a second
//! contributor in a second timezone writes an entry. A timestamp that cannot
//! be read at all sorts after every readable one instead of aborting the sort;
//! validation is what reports it, and an unreadable value in one item must not
//! stop the other items from being arranged.
//!
//! ## Canonical means "a function of content"
//!
//! Ids are unique across the whole document, so using the id as the final
//! tie-break makes the order a total one. That matters more than it looks: the
//! result is then determined by what the document CONTAINS, not by the order
//! it happened to already be in, so two people whose editors wrote the same
//! items in different orders produce the same file.

use super::schema::{ChangelogEntry, Document, Instant, Item, Timestamp};

/// Sort rank of an item with no `priority`. One past the last real rank, so
/// unranked items land after every ranked one.
pub const UNRANKED_RANK: u8 = 3;

/// Put a document into canonical order, in place. Idempotent: running it on an
/// already-canonical document changes nothing.
pub fn canonicalize(doc: &mut Document) {
    doc.changelog.sort_by(|a, b| {
        entry_key(a)
            .cmp(&entry_key(b))
            .then_with(|| a.id.cmp(&b.id))
    });
    for section in &mut doc.sections {
        section
            .items
            .sort_by(|a, b| item_key(a).cmp(&item_key(b)).then_with(|| a.id.cmp(&b.id)));
    }
}

/// The sort key of one item: not-done before done, then by priority rank, then
/// oldest first.
fn item_key(item: &Item) -> (u8, u8, (u8, Instant)) {
    let rank = item.priority.map_or(UNRANKED_RANK, |p| p.rank());
    (u8::from(item.done), rank, instant_key(&item.created))
}

/// The sort key of one changelog entry: oldest first.
fn entry_key(entry: &ChangelogEntry) -> (u8, Instant) {
    instant_key(&entry.created)
}

/// A timestamp as something orderable, with unreadable values pushed to the
/// end by the leading flag rather than compared as text.
fn instant_key(ts: &Timestamp) -> (u8, Instant) {
    match ts.instant() {
        Ok(instant) => (0, instant),
        Err(_) => (1, Instant::default()),
    }
}

#[cfg(test)]
mod tests;
