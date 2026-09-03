//! The typed checklist document, plus the ISO-8601 reading the rest of the
//! family orders and renders by.
//!
//! The document is ordinary committed repository content at
//! `docs/actions/<YYYY-MM-slug>.checklist.json` — reviewed on the pull request
//! like any other file, and hand-editable, because the Read/Edit deny that
//! normally routes edits through the CLI cannot fire when the binary is not
//! installed. So the model here has to survive being typed by a person and
//! being written by a NEWER ss-magic than the one reading it.
//!
//! ## Permissive about absence, strict about type
//!
//! Every field carries `#[serde(default)]`, so a hand-edited file that is
//! missing a required key still parses. That is deliberate: `validate` is the
//! single place that reports what is missing or inconsistent, and it reports
//! ALL of it at once. Letting serde reject the file instead would hand the
//! author one error per run, and would stop `verify` from doing the job it is
//! specified to do. A key whose value has the wrong *type* is still a parse
//! error — serde names the field, and no amount of validation can guess what a
//! string was supposed to mean where a list belongs.
//!
//! `kind` is the one field whose default carries meaning: an item with no
//! declared kind reads as [`ItemKind::Check`], the strictest of the three,
//! because a check-kind item may not leave `expected` null. Defaulting the
//! other way would let a missing key quietly switch verification off, and the
//! rule this repository already applies to its secret gate is that the unknown
//! answer must be the safe one.
//!
//! ## Two optionality conventions, which must not be mixed
//!
//! Both render as `Option<T>` in Rust, which is exactly why the distinction is
//! easy to lose on the way to the wire:
//!
//! - `expected` and `completed` are **always-present keys whose value may be
//!   null**. No `skip_serializing_if`. A null `expected` is a deliberate
//!   statement ("there is nothing to check here"), legal only on a record- or
//!   decision-kind item, so it has to be visible in the file rather than
//!   collapsing into an absent key.
//! - `priority`, `why`, `description` and `refs` are **omitted entirely when
//!   unset**. Writing `"priority": null` would make "unranked" look like a
//!   value, when the ordering rule treats it as an absence that sorts last.
//!
//! `expected` goes one step further and is an `Option<Option<String>>`, read
//! through [`deserialize_some`]: a missing key and an explicit null then stay
//! distinguishable, so validation can say "the key is absent" (a shape defect
//! the next write repairs) separately from "you declared no expectation on an
//! item whose kind requires one" (a real error). `completed` needs no such
//! trick, because an absent key and a null both mean the same thing there —
//! no completion was recorded.
//!
//! ## Ordered containers, never maps
//!
//! `sections` is an array of `{ id, title, items }`, not a map: the declared
//! order IS the render order, and a `HashMap` would destroy it while a
//! `BTreeMap` would replace it with a lexical order nobody asked for — worse,
//! because it looks deterministic. `id` carries identity; position carries
//! order.
//!
//! ## Timestamps are instants, not strings
//!
//! [`Timestamp`] keeps the spelling the author wrote (so a rewrite is
//! byte-stable and nobody's `+02:00` is silently normalized to `Z`) and
//! deliberately does **not** derive `Ord`. Comparing the raw text is the bug:
//! `2026-08-29T09:00:00+02:00` is EARLIER than `2026-08-29T08:00:00Z` but
//! sorts after it lexically, and nothing reveals that until a repository has
//! contributors in two timezones. Order by [`Timestamp::instant`] instead.

use std::fmt;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::plugin::scratchpad::format_rfc3339;

/// The `$schema` value this build writes: a stable identifier the binary owns.
///
/// Deliberately not a path. A relative path would point into the plugin
/// directory, which is version-scoped — the pointer would break on every
/// upgrade, and a document written by one version would name a location that
/// no longer exists for the next.
pub const SCHEMA_ID: &str = "https://github.com/ViktorStiskala/superset-magic/schema/checklist/v1";

// ── Scalars ───────────────────────────────────────────────────────────────────

/// An ISO-8601 timestamp exactly as it appears in the file.
///
/// Stored as the raw string so a rewrite reproduces what the author wrote.
/// `Ord` is intentionally absent — see the module doc; sort by
/// [`Timestamp::instant`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// Wrap a spelling as-is. No parsing happens here: an unreadable value has
    /// to reach `validate` to be reported, not vanish at construction.
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Format `secs` since the Unix epoch as UTC, through the one formatter
    /// the rest of the plugin already uses. This is what the CLI stamps a new
    /// item with, so everything ss-magic writes is `...Z` and second-precise.
    pub fn from_epoch_secs(secs: u64) -> Self {
        Self(format_rfc3339(secs))
    }

    /// The spelling as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when nothing was recorded at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The instant this names, or why it could not be read.
    pub fn instant(&self) -> Result<Instant, TimeError> {
        parse_iso8601(&self.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A point in time, as seconds since the Unix epoch plus a sub-second part.
///
/// The derived ordering compares `secs` first and `nanos` only to break a tie,
/// which is the whole point: two spellings at different offsets reduce to the
/// same pair of numbers and compare correctly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    /// Seconds since 1970-01-01T00:00:00Z, negative before it.
    pub secs: i64,
    /// Sub-second part, always below one billion.
    pub nanos: u32,
}

/// Why an ISO-8601 spelling could not be read.
///
/// Split by cause rather than collapsed into one message, because the three
/// map onto genuinely different author mistakes and validation quotes them
/// back verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    /// Not `YYYY-MM-DDTHH:MM:SS` followed by an offset at all.
    Shape,
    /// A well-shaped local time with no offset — the one spelling that looks
    /// right and is not, because an instant cannot be recovered from it.
    NoOffset,
    /// An offset that is present but unreadable.
    BadOffset,
    /// A component that parsed but names no real time; the string is the
    /// component.
    OutOfRange(&'static str),
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape => {
                f.write_str("not an ISO-8601 timestamp of the form YYYY-MM-DDTHH:MM:SS±HH:MM")
            }
            Self::NoOffset => {
                f.write_str("no UTC offset; write `Z` or `±HH:MM` so the instant is unambiguous")
            }
            Self::BadOffset => f.write_str("unreadable UTC offset; write `Z` or `±HH:MM`"),
            Self::OutOfRange(part) => write!(f, "{part} is out of range"),
        }
    }
}

/// What an item is for, which decides whether a null `expected` is legal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    /// Something is verified against a stated expectation. The default, and
    /// the only kind that may not leave `expected` null.
    #[default]
    Check,
    /// Something is written down — a measurement, an observation, a value.
    Record,
    /// A choice is made and its reasoning captured.
    Decision,
}

impl ItemKind {
    /// The token as it appears on the wire.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Record => "record",
            Self::Decision => "decision",
        }
    }

    /// True when `expected: null` says something coherent for this kind.
    /// A record has nothing to compare against and a decision is its own
    /// outcome; a check with no expectation is a verification that can never
    /// fail.
    pub fn allows_null_expectation(&self) -> bool {
        matches!(self, Self::Record | Self::Decision)
    }
}

/// What an item blocks, stated in terms of consequence rather than of any one
/// project's release process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    /// Not safe to ship until this is done.
    Blocking,
    /// A decision is still open, and shipping commits to an answer by default.
    DecisionBlocking,
    /// Real work that outlives the change, but does not gate it.
    FollowUp,
}

impl Priority {
    /// The token as it appears on the wire.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blocking => "blocking",
            Self::DecisionBlocking => "decision-blocking",
            Self::FollowUp => "follow-up",
        }
    }

    /// Sort rank, lowest first. An item with no priority is unranked and sorts
    /// after all of these — see `order::UNRANKED_RANK`.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Blocking => 0,
            Self::DecisionBlocking => 1,
            Self::FollowUp => 2,
        }
    }
}

/// A link an item or a changelog entry points at.
///
/// The render is read outside the repository — in a pull-request comment —
/// where a relative path resolves to nothing, so the URL has to be absolute.
/// Validation enforces that; parsing does not, so a bad one is reported rather
/// than rejected on load.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    /// Free-form prose naming what is on the other end.
    #[serde(default)]
    pub label: String,
    /// An absolute URL.
    #[serde(default)]
    pub url: String,
    /// Keys this build has no field for, carried through unchanged.
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

// ── The document ──────────────────────────────────────────────────────────────

/// One checklist file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// The identifier of the format this document is written in; [`SCHEMA_ID`]
    /// for anything this build wrote.
    #[serde(rename = "$schema", default)]
    pub schema: String,
    /// Human title of the action this checklist tracks.
    #[serde(default)]
    pub title: String,
    /// The `<YYYY-MM-slug>` stem the file is named after.
    #[serde(default)]
    pub slug: String,
    /// When the document was created.
    #[serde(default)]
    pub created: Timestamp,
    /// When it was last written. Re-stamped by every CLI write.
    #[serde(default)]
    pub updated: Timestamp,
    /// What happened, oldest first.
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
    /// The work, in the order it renders.
    #[serde(default)]
    pub sections: Vec<Section>,
    /// Every top-level key this build has no named field for, carried through
    /// a read-modify-write unchanged.
    ///
    /// The checklist is hand-editable and a newer ss-magic may have written
    /// keys this one has never heard of. Preserving them is the same rule
    /// `magic.json` follows: a writer must never silently delete configuration
    /// it does not understand. Rejecting them instead would make an older
    /// binary refuse a document it could otherwise render perfectly well.
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

/// One entry in the changelog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangelogEntry {
    /// Kebab-case, letter-initial, unique across the whole document.
    #[serde(default)]
    pub id: String,
    /// When it happened. Entries ascend by this instant.
    #[serde(default)]
    pub created: Timestamp,
    /// One line saying what changed.
    #[serde(default)]
    pub summary: String,
    /// The longer body, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Supporting links, omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<Reference>,
    /// Keys this build has no field for, carried through unchanged.
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

/// A named group of items. Position in `Document::sections` is its render
/// position, so ordering never touches this list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Section {
    /// Kebab-case, letter-initial, unique across the whole document. This is
    /// what `checklist add-item <section-id> <item-id>` addresses.
    #[serde(default)]
    pub id: String,
    /// The heading this section renders under.
    #[serde(default)]
    pub title: String,
    /// The items, kept in canonical order by `order::canonicalize`.
    #[serde(default)]
    pub items: Vec<Item>,
    /// Keys this build has no field for, carried through unchanged.
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

/// One thing that has to happen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Kebab-case, letter-initial, unique across the whole document, and never
    /// renamed: references and history hang off it.
    #[serde(default)]
    pub id: String,
    /// What this item is, in one line.
    #[serde(default)]
    pub title: String,
    /// Which of the three kinds this is; an absent key reads as `check`.
    #[serde(default)]
    pub kind: ItemKind,
    /// What this blocks. Absent means unranked, which sorts last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// When the item was added. Part of the sort key.
    #[serde(default)]
    pub created: Timestamp,
    /// Whether it is done AND verified.
    #[serde(default)]
    pub done: bool,
    /// When it was completed. Always written, null until it is done.
    #[serde(default)]
    pub completed: Option<Timestamp>,
    /// The prose body, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The action steps. At least one is required — an item with none
    /// describes a wish rather than work.
    #[serde(default)]
    pub steps: Vec<String>,
    /// What the steps should produce, or an explicit null on a record- or
    /// decision-kind item. Always written; see the module doc for why this is
    /// a double `Option`.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub expected: Option<Option<String>>,
    /// Why this item exists, when that is not obvious.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Supporting links, omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<Reference>,
    /// Keys this build has no field for, carried through unchanged.
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

impl Item {
    /// The stated expectation, if one was declared and is not null.
    pub fn expected_text(&self) -> Option<&str> {
        self.expected.as_ref()?.as_deref()
    }

    /// Whether the `expected` key is present in the file at all. A missing key
    /// is a shape defect the next write repairs; an explicit null is a claim
    /// about the item, and only some kinds may make it.
    pub fn expected_declared(&self) -> bool {
        self.expected.is_some()
    }
}

/// Read an `Option<T>` field in a way that tells a missing key from a null.
///
/// serde's own `Option` deserializer maps both to `None`, which is exactly the
/// distinction `expected` needs to keep. Deserializing the inner type directly
/// and wrapping the result means only an absent key can reach the `default`,
/// so `None` is "absent" and `Some(None)` is "written as null".
fn deserialize_some<'de, T, D>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

// ── Construction ──────────────────────────────────────────────────────────────

/// The section set a checklist starts from when a project declares none.
///
/// Binary-owned and domain-neutral: nothing here assumes a deploy, a release
/// train or a web property. A project that declares its own set gets exactly
/// that set in exactly that order, and nothing is ever appended to either —
/// there is no fixed trailing approval block.
pub fn default_sections() -> Vec<Section> {
    [
        ("verification", "Verification"),
        ("rollout", "Rollout"),
        ("decisions", "Open decisions"),
        ("follow-ups", "Follow-ups"),
    ]
    .into_iter()
    .map(|(id, title)| Section {
        id: id.to_string(),
        title: title.to_string(),
        items: Vec::new(),
        extras: Map::new(),
    })
    .collect()
}

impl Document {
    /// A new, empty checklist carrying the default section set.
    pub fn new(title: impl Into<String>, slug: impl Into<String>, now: Timestamp) -> Self {
        Self::with_sections(title, slug, now, default_sections())
    }

    /// A new, empty checklist over a section set the project declares. The
    /// order given is the order rendered.
    pub fn with_sections(
        title: impl Into<String>,
        slug: impl Into<String>,
        now: Timestamp,
        sections: Vec<Section>,
    ) -> Self {
        Self {
            schema: SCHEMA_ID.to_string(),
            title: title.into(),
            slug: slug.into(),
            created: now.clone(),
            updated: now,
            changelog: Vec::new(),
            sections,
            extras: Map::new(),
        }
    }

    /// The section with this id, if the document has one.
    pub fn section(&self, id: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.id == id)
    }

    /// Every item in the document, paired with the id of the section holding
    /// it, in render order.
    pub fn items(&self) -> impl Iterator<Item = (&str, &Item)> {
        self.sections
            .iter()
            .flat_map(|s| s.items.iter().map(move |i| (s.id.as_str(), i)))
    }
}

// ── JSON I/O ──────────────────────────────────────────────────────────────────

/// Parse a checklist from JSON text.
pub fn from_json(raw: &str) -> Result<Document> {
    serde_json::from_str::<Document>(raw).context("malformed checklist JSON")
}

/// Serialize a checklist, pretty-printed with a trailing newline — the shape
/// the rest of this crate writes JSON in, and the one a reviewer reads as a
/// diff.
pub fn to_json(doc: &Document) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(doc)?))
}

/// Read a checklist from disk. `Ok(None)` when the file is absent; an error
/// when it exists and cannot be read or parsed.
///
/// Mirrors `workspace::superset_files::read_json`, including its refusal to
/// treat a directory or a device node as a missing file.
pub fn read_document(path: &Path) -> Result<Option<Document>> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        bail!("`{}` exists but is not a regular file", path.display());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed =
        from_json(&raw).with_context(|| format!("malformed JSON in {}", path.display()))?;
    Ok(Some(parsed))
}

// ── ISO-8601 reading ──────────────────────────────────────────────────────────

/// Read `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM|±HHMM|±HH)` into an instant.
///
/// This is the inverse of the crate's existing UTC formatter, written here
/// because ordering needs it and no date crate is (or will be) a dependency.
/// It is deliberately narrow: an offset is required, seconds are required, and
/// the two spellings ISO-8601 allows that name no ordinary instant — `24:00`
/// for end-of-day and a `:60` leap second — are refused rather than quietly
/// mapped onto a neighbouring second, because ordering would then depend on
/// which neighbour was picked.
pub fn parse_iso8601(raw: &str) -> std::result::Result<Instant, TimeError> {
    let bytes = raw.as_bytes();
    // The date and time are a fixed 19 bytes. A shorter string cannot carry
    // them; a string of exactly 19 carries them and no offset, which is a
    // different mistake and is reported as one by `parse_offset` below.
    if !raw.is_ascii() || bytes.len() < 19 {
        return Err(TimeError::Shape);
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(TimeError::Shape);
    }

    let year = digits(bytes, 0, 4).ok_or(TimeError::Shape)? as i64;
    let month = digits(bytes, 5, 2).ok_or(TimeError::Shape)?;
    let day = digits(bytes, 8, 2).ok_or(TimeError::Shape)?;
    let hour = digits(bytes, 11, 2).ok_or(TimeError::Shape)?;
    let minute = digits(bytes, 14, 2).ok_or(TimeError::Shape)?;
    let second = digits(bytes, 17, 2).ok_or(TimeError::Shape)?;

    if !(1..=12).contains(&month) {
        return Err(TimeError::OutOfRange("month"));
    }
    if day < 1 || day > days_in_month(year, month) {
        return Err(TimeError::OutOfRange("day"));
    }
    if hour > 23 {
        return Err(TimeError::OutOfRange("hour"));
    }
    if minute > 59 {
        return Err(TimeError::OutOfRange("minute"));
    }
    if second > 59 {
        return Err(TimeError::OutOfRange("second"));
    }

    let mut rest = &raw[19..];
    let mut nanos = 0u32;
    if let Some(after_dot) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(',')) {
        let taken = after_dot
            .as_bytes()
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if taken == 0 {
            return Err(TimeError::Shape);
        }
        nanos = fraction_to_nanos(&after_dot[..taken]);
        rest = &after_dot[taken..];
    }

    let offset = parse_offset(rest)?;
    let secs = days_from_civil(year, month, day) * 86_400
        + i64::from(hour) * 3600
        + i64::from(minute) * 60
        + i64::from(second)
        - offset;

    Ok(Instant { secs, nanos })
}

/// Offset in seconds east of UTC, subtracted to reach the instant.
fn parse_offset(raw: &str) -> std::result::Result<i64, TimeError> {
    if raw.is_empty() {
        return Err(TimeError::NoOffset);
    }
    if matches!(raw, "Z" | "z") {
        return Ok(0);
    }
    let bytes = raw.as_bytes();
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return Err(TimeError::BadOffset),
    };
    // `±HH`, `±HHMM` and `±HH:MM` are all in circulation; accept all three and
    // nothing else.
    let (hours, minutes) = match raw.len() {
        3 => (digits(bytes, 1, 2), Some(0)),
        5 => (digits(bytes, 1, 2), digits(bytes, 3, 2)),
        6 if bytes[3] == b':' => (digits(bytes, 1, 2), digits(bytes, 4, 2)),
        _ => return Err(TimeError::BadOffset),
    };
    let (Some(hours), Some(minutes)) = (hours, minutes) else {
        return Err(TimeError::BadOffset);
    };
    if hours > 23 || minutes > 59 {
        return Err(TimeError::BadOffset);
    }
    Ok(sign * (i64::from(hours) * 3600 + i64::from(minutes) * 60))
}

/// `count` ASCII digits starting at `start`, or `None` if any of them is not a
/// digit or the slice is short.
fn digits(bytes: &[u8], start: usize, count: usize) -> Option<u32> {
    let slice = bytes.get(start..start + count)?;
    let mut value = 0u32;
    for b in slice {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(b - b'0');
    }
    Some(value)
}

/// Scale a fractional-second digit string to nanoseconds, truncating anything
/// finer than a nanosecond rather than rounding — truncation cannot push an
/// instant past its neighbour.
fn fraction_to_nanos(digits: &str) -> u32 {
    let mut nanos = 0u32;
    for i in 0..9 {
        let digit = digits.as_bytes().get(i).map_or(0, |b| u32::from(b - b'0'));
        nanos = nanos * 10 + digit;
    }
    nanos
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from the Unix epoch to a civil date — Howard Hinnant's algorithm, the
/// exact inverse of the civil-from-days conversion the plugin's UTC formatter
/// runs, so a value formatted by one and read by the other survives the trip.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    // Shift the year to start in March so a leap day lands at the end of the
    // cycle and needs no special case.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * i64::from(mp) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests;
