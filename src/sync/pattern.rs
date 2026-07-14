//! Shared pattern utilities used by both the apply-mode expansion and the
//! bootstrap UI's validator. The two layers must agree on what counts as
//! a valid pattern — keeping the predicates here means a change in one
//! place propagates everywhere.

use globset::Glob;

/// True when `s` contains a glob metacharacter (`*`, `?`, or `[`).
pub fn has_glob_meta(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '*' | '?' | '['))
}

/// True when any `/`-separated segment of `s` is exactly `..`.
pub fn has_parent_segment(s: &str) -> bool {
    s.split('/').any(|seg| seg == "..")
}

/// Reasons a pattern is structurally invalid. Mirrors the rejection
/// outcomes in `apply::SkipReason` but lives at the syntax layer so the
/// UI validator can use it before the user ever submits a flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxError {
    Empty,
    AbsolutePath,
    ParentSegment,
    BadGlob(String),
}

impl SyntaxError {
    /// Short one-line label suitable for an `inquire` validator.
    pub fn label(&self) -> String {
        match self {
            SyntaxError::Empty => "pattern is empty".to_string(),
            SyntaxError::AbsolutePath => "absolute paths are not allowed".to_string(),
            SyntaxError::ParentSegment => "`..` segments are not allowed".to_string(),
            SyntaxError::BadGlob(msg) => format!("invalid glob: {msg}"),
        }
    }
}

/// Check a pattern for the same syntactic guards that apply-mode enforces
/// at execution time: non-empty, no absolute prefix, no `..` segment, and
/// (for glob patterns) compilable.
pub fn check_syntax(pattern: &str) -> Result<(), SyntaxError> {
    if pattern.is_empty() {
        return Err(SyntaxError::Empty);
    }
    if pattern.starts_with('/') {
        return Err(SyntaxError::AbsolutePath);
    }
    if has_parent_segment(pattern) {
        return Err(SyntaxError::ParentSegment);
    }
    if has_glob_meta(pattern) {
        if let Err(e) = Glob::new(pattern) {
            return Err(SyntaxError::BadGlob(e.to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
