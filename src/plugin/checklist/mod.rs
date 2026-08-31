//! The operator checklist: one JSON document per action, at
//! `docs/actions/<YYYY-MM-slug>.checklist.json`.
//!
//! A checklist tracks the operational steps a change needs before it is safe
//! to ship — what has to be verified, what is rolled out, which decisions are
//! still open, and the follow-ups that outlive the code change. It is
//! committed repository content, reviewed on the pull request like any other
//! file, and rendered into a comment there on every push.
//!
//! ## What this family knows, and what it deliberately does not
//!
//! It is a document model with a validator, a canonical ordering and (from
//! U26) a renderer. It knows nothing about hooks, tool envelopes or permission
//! decisions — the same dependency direction [`crate::plugin::cache`] follows.
//! The `PreToolUse` gate that denies a direct `Read` or `Edit` of a checklist
//! is a *caller*: it asks [`pointer_target`] and [`matches_convention`]
//! whether a resolved path is a checklist file and acts on the answer. Inverting that, by teaching the schema about tool payloads,
//! would make the format untestable without a harness envelope.
//!
//! ## The pieces
//!
//! - [`schema`] — the typed document, and the binary-owned ISO-8601 reader the
//!   rest of the family orders and renders by.
//! - [`order`] — the canonical arrangement every write re-establishes.
//! - [`validate`] — what the format leaves implicit, returned as findings
//!   rather than printed.
//! - [`render`] (the module) — Markdown rendering, wrapped in the R64
//!   untrusted-data envelope. [`render`] (the function) is its one entry
//!   point.
//! - [`verbs`] — the `ss-magic plugin checklist …` command surface: the only
//!   write path, the atomic file replacement behind it, and the
//!   `.superset/.magic/checklist.json` pointer that records which document is
//!   live.
//!
//! Only [`verbs`] writes a file. Everything below it is a pure document model:
//! reading one is offered ([`read_document`]) because parsing is this module's
//! business, but the write, the verb dispatch and the pointer belong to the
//! verb layer.

// Both production callers are now wired: the verbs are the write path, and the
// `PreToolUse` deny consumes `pointer_target` and `matches_convention`. What
// remains unreferenced outside this family's own tests is the rest of the
// re-exported surface — the constants and `Pointer` that let a caller name a
// checklist path, and a handful of model accessors the verbs happen not to
// need. They are the format's vocabulary rather than leftovers, so one pair of
// module-wide allows keeps them without a dozen individual attributes.
#![allow(dead_code, unused_imports)]

mod order;
mod render;
mod schema;
mod validate;
mod verbs;

// The family's public surface. Submodules stay private so a caller cannot
// reach past these — the ordering rule and the validation rules are the
// contract, and a caller building its own sort or its own field checks would
// be a second, drifting definition of "canonical" and "valid".
pub use order::{canonicalize, UNRANKED_RANK};
pub use render::render;
pub use schema::{
    default_sections, from_json, parse_iso8601, read_document, to_json, ChangelogEntry, Document,
    Instant, Item, ItemKind, Priority, Reference, Section, TimeError, Timestamp, SCHEMA_ID,
};
pub use validate::{has_errors, is_absolute_url, is_well_formed_id, validate, Finding, Severity};
pub use verbs::{
    matches_convention, pointer_path, pointer_target, run, Pointer, ACTIONS_REL, CHECKLIST_SUFFIX,
    POINTER_NAME,
};

#[cfg(test)]
mod tests;
