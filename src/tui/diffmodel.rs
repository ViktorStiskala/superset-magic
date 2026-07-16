//! Pure diff model for the reverse-sync merge cockpit.
//!
//! This module turns two file versions (*local* worktree copy vs *main*
//! branch copy) into flat, render-ready row models. It owns three concerns
//! and nothing else:
//!
//! - [`classify_content`] decides whether a byte buffer is diffable text,
//!   opaque binary, or too large to render.
//! - [`side_by_side`] and [`unified`] compute the two diff layouts from a
//!   `similar` line diff, carrying per-side line numbers, folded context
//!   gaps, and intra-line (word-level) emphasis.
//! - [`should_split`] picks the layout that fits the terminal width.
//!
//! It is deliberately free of any TUI / `ratatui` dependency so the logic
//! stays unit-testable in isolation; the interactive [`crate::tui::cockpit`]
//! layer consumes these models to render the diff pane.

use std::borrow::Cow;

use similar::{ChangeTag, DiffTag, TextDiff};

/// Upper bound on a single side's byte length before we refuse to diff it
/// and fall back to whole-file push/pull. Tunable.
pub const MAX_DIFF_BYTES: u64 = 2 * 1024 * 1024; // 2 MiB

/// Classification of a file's bytes for diffing purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentKind {
    /// Diffable UTF-8 text, carrying the decoded string.
    Text(String),
    /// Contains a NUL byte or is not valid UTF-8; only whole-file
    /// push/pull is offered and interactive merge is disabled.
    Binary,
    /// Larger than [`MAX_DIFF_BYTES`]; carries the observed byte size.
    TooLarge(u64),
}

/// Classify a byte buffer for diffing.
///
/// The size cap is checked *before* UTF-8 validation so an oversized buffer
/// never pays for a full decode. A buffer is [`ContentKind::Binary`] when it
/// contains a NUL byte or is not valid UTF-8; otherwise it is
/// [`ContentKind::Text`].
pub fn classify_content(bytes: &[u8]) -> ContentKind {
    if bytes.len() as u64 > MAX_DIFF_BYTES {
        return ContentKind::TooLarge(bytes.len() as u64);
    }
    if bytes.contains(&0) {
        return ContentKind::Binary;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => ContentKind::Text(s.to_string()),
        Err(_) => ContentKind::Binary,
    }
}

/// One intra-line segment of a rendered line.
///
/// A line is a sequence of segments; `emphasized` marks the portion that
/// actually changed (word-level highlight) so equal spans and changed spans
/// can be styled differently within the same line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    /// The segment text (trailing `\n` stripped for display).
    pub text: String,
    /// True when this span is the changed portion of the line.
    pub emphasized: bool,
}

/// The role of a side-by-side row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTag {
    /// Unchanged line, present identically on both sides.
    Equal,
    /// Line removed relative to main (present on the left only).
    Delete,
    /// Line added relative to main (present on the right only).
    Insert,
    /// Line changed on both sides (a zipped delete/insert pair).
    Replace,
    /// Folded gap: `n` unchanged lines hidden between shown hunks.
    Fold(usize),
}

/// A single row of the two-column side-by-side diff.
///
/// For a pure [`RowTag::Insert`] row `left_no` is `None` and `left` is empty;
/// for a pure [`RowTag::Delete`] row `right_no` is `None` and `right` is
/// empty. A [`RowTag::Fold`] row has no line numbers and empty cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    /// 1-based line number on the local (left) side, if present.
    pub left_no: Option<usize>,
    /// Left-side line segments.
    pub left: Vec<Seg>,
    /// 1-based line number on the main (right) side, if present.
    pub right_no: Option<usize>,
    /// Right-side line segments.
    pub right: Vec<Seg>,
    /// The row's role.
    pub tag: RowTag,
}

/// The role of a unified (single-column) row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedTag {
    /// Unchanged context line.
    Context,
    /// Removed line.
    Delete,
    /// Added line.
    Insert,
    /// Folded gap: `n` unchanged lines hidden.
    Fold(usize),
}

/// A single row of the unified full-width diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedRow {
    /// 1-based old-side line number, if the row exists there.
    pub old_no: Option<usize>,
    /// 1-based new-side line number, if the row exists there.
    pub new_no: Option<usize>,
    /// The row's role.
    pub tag: UnifiedTag,
    /// Line segments (empty for a fold row).
    pub segs: Vec<Seg>,
}

/// Minimum terminal width at which a legible two-column split fits.
const SPLIT_MIN_WIDTH: u16 = 100;

/// True when the terminal is wide enough for a two-column side-by-side
/// diff; below the threshold the caller should fall back to [`unified`].
pub fn should_split(term_width: u16) -> bool {
    term_width >= SPLIT_MIN_WIDTH
}

/// Convert 0-based `similar` indices to a 1-based display line number.
fn line_no(idx: Option<usize>) -> Option<usize> {
    idx.map(|i| i + 1)
}

/// Build the segment list for one inline change, stripping the FULL trailing
/// line terminator (`\r\n` or `\n`) from the last segment (and dropping any
/// segment left empty by that strip, e.g. a blank line).
///
/// Popping only the `\n` would leave a bare `\r` on CRLF lines — it would render
/// as a stray control character AND make an otherwise-identical CRLF/LF pair
/// register as changed. So the trailing `\r` is dropped too.
fn segs_from_strings<'s>(strings: impl Iterator<Item = (bool, Cow<'s, str>)>) -> Vec<Seg> {
    let mut segs: Vec<Seg> = strings
        .map(|(emphasized, value)| Seg {
            text: value.into_owned(),
            emphasized,
        })
        .collect();
    if let Some(last) = segs.last_mut() {
        if last.text.ends_with('\n') {
            last.text.pop();
            // CRLF: drop the carriage return the `\n` pop left behind.
            if last.text.ends_with('\r') {
                last.text.pop();
            }
        }
    }
    while segs.last().is_some_and(|s| s.text.is_empty()) {
        segs.pop();
    }
    segs
}

/// Build the two-column side-by-side model for `local` vs `main`, folding
/// unchanged runs to `context` lines around each change.
///
/// Equal lines appear on both sides with matching line numbers and no
/// emphasis; delete-only ops fill the left and blank the right; insert-only
/// ops fill the right and blank the left; replace ops zip the changed old
/// lines (left) against the changed new lines (right), padding the shorter
/// side with a blank cell and carrying word-level emphasis on each side.
pub fn side_by_side(local: &str, main: &str, context: usize) -> Vec<DiffRow> {
    let diff = TextDiff::from_lines(local, main);
    let mut rows: Vec<DiffRow> = Vec::new();
    let mut prev_end = 0usize;

    for group in diff.grouped_ops(context) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let start = first.old_range().start;
        if start > prev_end {
            rows.push(fold_row(start - prev_end));
        }
        prev_end = last.old_range().end;

        for op in &group {
            match op.tag() {
                DiffTag::Equal => {
                    for change in diff.iter_inline_changes(op) {
                        let segs = segs_from_strings(change.iter_strings_lossy());
                        rows.push(DiffRow {
                            left_no: line_no(change.old_index()),
                            left: segs.clone(),
                            right_no: line_no(change.new_index()),
                            right: segs,
                            tag: RowTag::Equal,
                        });
                    }
                }
                DiffTag::Delete => {
                    for change in diff.iter_inline_changes(op) {
                        rows.push(DiffRow {
                            left_no: line_no(change.old_index()),
                            left: segs_from_strings(change.iter_strings_lossy()),
                            right_no: None,
                            right: Vec::new(),
                            tag: RowTag::Delete,
                        });
                    }
                }
                DiffTag::Insert => {
                    for change in diff.iter_inline_changes(op) {
                        rows.push(DiffRow {
                            left_no: None,
                            left: Vec::new(),
                            right_no: line_no(change.new_index()),
                            right: segs_from_strings(change.iter_strings_lossy()),
                            tag: RowTag::Insert,
                        });
                    }
                }
                DiffTag::Replace => {
                    let mut dels: Vec<(Option<usize>, Vec<Seg>)> = Vec::new();
                    let mut inss: Vec<(Option<usize>, Vec<Seg>)> = Vec::new();
                    for change in diff.iter_inline_changes(op) {
                        let segs = segs_from_strings(change.iter_strings_lossy());
                        match change.tag() {
                            ChangeTag::Delete => dels.push((line_no(change.old_index()), segs)),
                            ChangeTag::Insert => inss.push((line_no(change.new_index()), segs)),
                            ChangeTag::Equal => {}
                        }
                    }
                    for idx in 0..dels.len().max(inss.len()) {
                        let (left_no, left) = dels.get(idx).cloned().unwrap_or_default();
                        let (right_no, right) = inss.get(idx).cloned().unwrap_or_default();
                        rows.push(DiffRow {
                            left_no,
                            left,
                            right_no,
                            right,
                            tag: RowTag::Replace,
                        });
                    }
                }
            }
        }
    }

    rows
}

/// A side-by-side fold row: no line numbers, empty cells, `n` hidden lines.
fn fold_row(n: usize) -> DiffRow {
    DiffRow {
        left_no: None,
        left: Vec::new(),
        right_no: None,
        right: Vec::new(),
        tag: RowTag::Fold(n),
    }
}

/// Build the unified full-width model for `local` vs `main`, folding
/// unchanged runs to `context` lines around each change.
///
/// Each inline change becomes one row tagged [`UnifiedTag::Context`],
/// [`UnifiedTag::Delete`], or [`UnifiedTag::Insert`], with a
/// [`UnifiedTag::Fold`] row inserted for each hidden context gap.
pub fn unified(local: &str, main: &str, context: usize) -> Vec<UnifiedRow> {
    let diff = TextDiff::from_lines(local, main);
    let mut rows: Vec<UnifiedRow> = Vec::new();
    let mut prev_end = 0usize;

    for group in diff.grouped_ops(context) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let start = first.old_range().start;
        if start > prev_end {
            rows.push(UnifiedRow {
                old_no: None,
                new_no: None,
                tag: UnifiedTag::Fold(start - prev_end),
                segs: Vec::new(),
            });
        }
        prev_end = last.old_range().end;

        for op in &group {
            for change in diff.iter_inline_changes(op) {
                let tag = match change.tag() {
                    ChangeTag::Equal => UnifiedTag::Context,
                    ChangeTag::Delete => UnifiedTag::Delete,
                    ChangeTag::Insert => UnifiedTag::Insert,
                };
                rows.push(UnifiedRow {
                    old_no: line_no(change.old_index()),
                    new_no: line_no(change.new_index()),
                    tag,
                    segs: segs_from_strings(change.iter_strings_lossy()),
                });
            }
        }
    }

    rows
}

#[cfg(test)]
mod tests;
