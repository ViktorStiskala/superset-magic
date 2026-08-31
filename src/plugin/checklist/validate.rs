//! What the format leaves implicit, checked and reported.
//!
//! Serde already refuses a value of the wrong type, and the schema is
//! deliberately permissive about absence so that a hand-edited file still
//! parses. Everything between those two — a required key that is missing, an
//! id that is malformed or used twice, a done item with no completion
//! timestamp, a null expectation on an item whose kind requires one, an item
//! with no action step, a reference that is a relative path — is checked here.
//!
//! [`validate`] returns findings and never prints, exits or touches the
//! filesystem. The `checklist verify` verb is what turns them into output and
//! an exit code, the same separation the rest of this crate keeps between a
//! pure core and the rendering at the edge. A caller that wants the document
//! rejected asks [`has_errors`]; warnings describe shape defects the next
//! write repairs on its own and must not fail a repository's CI over
//! something no reader would notice.

use std::collections::HashMap;
use std::fmt;

use super::schema::{ChangelogEntry, Document, Item, Reference, Section, Timestamp, SCHEMA_ID};

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The document is wrong. `verify` exits non-zero and the renderer is
    /// never handed the file.
    Error,
    /// The document is usable but its shape is off, and an ordinary CLI write
    /// will tidy it.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
        })
    }
}

/// One thing wrong with a document.
///
/// `location` names the record — the item, the section, the changelog entry —
/// so a person can go straight to it by id, which is faster and more reliable
/// than being told to read the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Error or warning.
    pub severity: Severity,
    /// Where in the document, by id: `sections[rollout].items[dns-cutover]`.
    pub location: String,
    /// What is wrong, naming the field.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.severity, self.location, self.message)
    }
}

/// True when at least one finding is an error.
pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Error)
}

/// Check a whole document. Findings come back in document order, so the list
/// reads top to bottom the way the file does.
pub fn validate(doc: &Document) -> Vec<Finding> {
    let mut findings = Vec::new();
    // Every id in the document shares one namespace, mapped to where it was
    // first seen so a duplicate can name its twin.
    let mut seen: HashMap<&str, String> = HashMap::new();

    check_document_header(doc, &mut findings);

    for entry in &doc.changelog {
        check_changelog_entry(entry, &mut seen, &mut findings);
    }
    for section in &doc.sections {
        check_section(section, &mut seen, &mut findings);
    }

    findings
}

// ── The document itself ───────────────────────────────────────────────────────

fn check_document_header(doc: &Document, findings: &mut Vec<Finding>) {
    let at = "document";

    if doc.schema.is_empty() {
        error(
            findings,
            at,
            format!("`$schema` is required; write `{SCHEMA_ID}`"),
        );
    } else if !is_absolute_url(&doc.schema) {
        error(
            findings,
            at,
            format!(
                "`$schema` is `{}`, which is a path rather than an identifier; \
                 it must be a stable absolute identifier such as `{SCHEMA_ID}`, \
                 because a path into the installed plugin directory changes on \
                 every upgrade",
                doc.schema
            ),
        );
    } else if doc.schema != SCHEMA_ID {
        warning(
            findings,
            at,
            format!(
                "`$schema` names a different checklist format (`{}`); this build writes `{SCHEMA_ID}`",
                doc.schema
            ),
        );
    }

    require_text(findings, at, "title", &doc.title);
    require_text(findings, at, "slug", &doc.slug);
    check_timestamp(findings, at, "created", &doc.created);
    check_timestamp(findings, at, "updated", &doc.updated);

    if doc.sections.is_empty() {
        warning(
            findings,
            at,
            "the document declares no sections, so it renders as an empty checklist".to_string(),
        );
    }
}

// ── Records ───────────────────────────────────────────────────────────────────

fn check_changelog_entry<'a>(
    entry: &'a ChangelogEntry,
    seen: &mut HashMap<&'a str, String>,
    findings: &mut Vec<Finding>,
) {
    let at = format!("changelog[{}]", display_id(&entry.id));
    check_id(findings, &at, &entry.id, seen);
    require_text(findings, &at, "summary", &entry.summary);
    check_timestamp(findings, &at, "created", &entry.created);
    check_refs(findings, &at, &entry.refs);
}

fn check_section<'a>(
    section: &'a Section,
    seen: &mut HashMap<&'a str, String>,
    findings: &mut Vec<Finding>,
) {
    let at = format!("sections[{}]", display_id(&section.id));
    check_id(findings, &at, &section.id, seen);
    require_text(findings, &at, "title", &section.title);

    for item in &section.items {
        check_item(item, &at, seen, findings);
    }
}

fn check_item<'a>(
    item: &'a Item,
    section_at: &str,
    seen: &mut HashMap<&'a str, String>,
    findings: &mut Vec<Finding>,
) {
    let at = format!("{section_at}.items[{}]", display_id(&item.id));
    check_id(findings, &at, &item.id, seen);
    require_text(findings, &at, "title", &item.title);
    check_timestamp(findings, &at, "created", &item.created);
    check_steps(findings, &at, &item.steps);
    check_completion(findings, &at, item);
    check_expected(findings, &at, item);
    check_refs(findings, &at, &item.refs);
}

fn check_steps(findings: &mut Vec<Finding>, at: &str, steps: &[String]) {
    if steps.iter().all(|s| s.trim().is_empty()) {
        error(
            findings,
            at,
            "no action step; an item with none describes a wish rather than work".to_string(),
        );
        return;
    }
    for (index, step) in steps.iter().enumerate() {
        if step.trim().is_empty() {
            warning(findings, at, format!("action step {index} is empty"));
        }
    }
}

/// `done` and the completion timestamp have to agree in both directions: a
/// done item without one leaves no record of when the work was verified, and a
/// timestamp on an item that is not done is usually a half-finished edit.
fn check_completion(findings: &mut Vec<Finding>, at: &str, item: &Item) {
    let recorded = item.completed.as_ref().filter(|ts| !ts.is_empty());

    match (item.done, recorded) {
        (true, None) => error(
            findings,
            at,
            "`done` is true but no completion timestamp is recorded".to_string(),
        ),
        (false, Some(_)) => warning(
            findings,
            at,
            "a completion timestamp is recorded but `done` is false".to_string(),
        ),
        _ => {}
    }

    if let Some(ts) = recorded {
        if let Err(err) = ts.instant() {
            error(
                findings,
                at,
                format!("`completed` (`{ts}`) cannot be read: {err}"),
            );
        }
    }
}

/// A null expectation is a claim — "there is deliberately nothing to check
/// here" — and only a record- or decision-kind item is entitled to make it.
/// On a check-kind item it describes a verification that can never fail, which
/// is worse than no item at all because it renders as covered.
fn check_expected(findings: &mut Vec<Finding>, at: &str, item: &Item) {
    let Some(value) = item.expected.as_ref() else {
        warning(
            findings,
            at,
            "`expected` is absent; it is an always-present key, and the next \
             write will add it as null"
                .to_string(),
        );
        if !item.kind.allows_null_expectation() {
            error(
                findings,
                at,
                format!(
                    "`expected` is absent on a {}-kind item, so it declares no \
                     expectation; a check with none can never fail, and only a \
                     record- or decision-kind item may leave it out",
                    item.kind.as_str()
                ),
            );
        }
        return;
    };

    match value.as_deref() {
        None if !item.kind.allows_null_expectation() => {
            error(
                findings,
                at,
                format!(
                    "`expected` is null on a {}-kind item; a null expectation is \
                     a check that can never fail, and is legal only on a record- \
                     or decision-kind item",
                    item.kind.as_str()
                ),
            );
        }
        Some(text) if text.trim().is_empty() => warning(
            findings,
            at,
            "`expected` is an empty string; write null if there is deliberately \
             nothing to check"
                .to_string(),
        ),
        _ => {}
    }
}

fn check_refs(findings: &mut Vec<Finding>, at: &str, refs: &[Reference]) {
    for (index, reference) in refs.iter().enumerate() {
        if reference.url.is_empty() {
            error(findings, at, format!("reference {index} has no `url`"));
        } else if !is_absolute_url(&reference.url) {
            error(
                findings,
                at,
                format!(
                    "reference {index} (`{}`) is not an absolute URL; the render \
                     is read outside the repository, where a relative path \
                     resolves to nothing",
                    reference.url
                ),
            );
        }
        if reference.label.trim().is_empty() {
            warning(
                findings,
                at,
                format!("reference {index} has no `label`, so it renders as a bare URL"),
            );
        }
    }
}

// ── Shared field checks ───────────────────────────────────────────────────────

/// Record an id, reporting it when it is missing, malformed, or already taken.
///
/// One namespace covers sections, items and changelog entries together. R83
/// only requires items and changelog entries to be jointly unique, but the CLI
/// addresses a record by id alone (`checklist set <id> …`), so an item sharing
/// a section's id would be an instruction nothing could resolve.
fn check_id<'a>(
    findings: &mut Vec<Finding>,
    at: &str,
    id: &'a str,
    seen: &mut HashMap<&'a str, String>,
) {
    if id.is_empty() {
        error(findings, at, "`id` is required".to_string());
        return;
    }
    if !is_well_formed_id(id) {
        error(
            findings,
            at,
            format!(
                "id `{id}` is not well formed; ids are kebab-case, begin with a \
                 letter, and hold only lowercase letters, digits and single hyphens"
            ),
        );
    }
    if let Some(first) = seen.get(id) {
        error(
            findings,
            at,
            format!(
                "id `{id}` is already used by {first}; ids are unique across the whole document"
            ),
        );
    } else {
        seen.insert(id, at.to_string());
    }
}

fn require_text(findings: &mut Vec<Finding>, at: &str, field: &str, value: &str) {
    if value.trim().is_empty() {
        error(findings, at, format!("`{field}` is required"));
    }
}

fn check_timestamp(findings: &mut Vec<Finding>, at: &str, field: &str, ts: &Timestamp) {
    if ts.is_empty() {
        error(findings, at, format!("`{field}` is required"));
        return;
    }
    if let Err(err) = ts.instant() {
        error(
            findings,
            at,
            format!("`{field}` (`{ts}`) cannot be read: {err}"),
        );
    }
}

// ── Predicates ────────────────────────────────────────────────────────────────

/// Kebab-case, letter-initial: `a-z` first, then lowercase letters, digits and
/// single interior hyphens.
///
/// Strict on purpose. An id is permanent — references and history hang off it
/// — so the moment to refuse `Deploy_Step2` is before anything points at it.
pub fn is_well_formed_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    if id.ends_with('-') || id.contains("--") {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// True for `scheme://authority…`.
///
/// Deliberately shallow: this is the difference between a link that survives
/// being pasted into a pull-request comment and one that does not, not a URL
/// validator. A relative path, a bare filename and a `#anchor` all fail it.
pub fn is_absolute_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    let mut chars = scheme.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

// ── Finding constructors ──────────────────────────────────────────────────────

fn error(findings: &mut Vec<Finding>, at: &str, message: String) {
    findings.push(Finding {
        severity: Severity::Error,
        location: at.to_string(),
        message,
    });
}

fn warning(findings: &mut Vec<Finding>, at: &str, message: String) {
    findings.push(Finding {
        severity: Severity::Warning,
        location: at.to_string(),
        message,
    });
}

/// How an id reads inside a location. An empty one still has to point
/// somewhere, so it shows as `<no id>` rather than as an empty bracket.
fn display_id(id: &str) -> &str {
    if id.is_empty() {
        "<no id>"
    } else {
        id
    }
}

#[cfg(test)]
mod tests;
