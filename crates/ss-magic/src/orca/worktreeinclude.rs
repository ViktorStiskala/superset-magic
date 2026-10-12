//! `.worktreeinclude` – the list of literal, gitignored paths Orca copies from
//! the main checkout into a new worktree.
//!
//! The file has two owners (KTD8). ss-magic owns the lines between two marker
//! comments, which it regenerates from `magic.json`; every other line is the
//! person's and survives each rewrite byte for byte. A file with no section
//! gains one at the end; a missing file is created holding only the section.
//!
//! Three things happen here:
//!
//! - [`read`] loads the file, telling "absent" apart from "present but not
//!   usable" – the second must never be rewritten, because rebuilding it from an
//!   empty parse would drop the person's lines.
//! - [`Document`] splits the text into the person's part and the managed
//!   section, says how much of Orca's budget the managed section may use, and
//!   renders the file with a new section.
//! - [`entries`] reads every literal entry of the file, both parts, the way
//!   Orca itself does – the list forward sync turns into its skip set.
//!
//! Orca's reader, as measured against the 1.4.221 bundle: lines are split on
//! `\r?\n` and trimmed; a blank line or one starting with `#` is skipped; a
//! backslash becomes `/`, one leading `./` and any trailing `/` are stripped;
//! an entry starting with `!` or holding `*` or `?` is rejected (no globs), as
//! is an absolute path, a `..` or empty segment, or a path starting in `.git`;
//! entries are de-duplicated. A file larger than 256 KiB is ignored whole, and
//! only the first 1000 entries are used.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::sync::apply::PathFilter;

/// The file's name, at the root of the main checkout.
pub const FILE_NAME: &str = ".worktreeinclude";

/// Orca uses at most this many entries.
pub const MAX_ENTRIES: usize = 1000;

/// Orca ignores the whole file when it is larger than this.
pub const MAX_BYTES: usize = 256 * 1024;

/// How the two marker lines are recognized: by their start, so the explanatory
/// wording after it can change between releases without orphaning a section an
/// older release wrote.
const BEGIN_PREFIX: &str = "# >>> ss-magic";
const END_PREFIX: &str = "# <<< ss-magic";

/// The marker lines as written. Both start with `#`, which Orca reads as a
/// comment, and both are plain ASCII.
const BEGIN_LINE: &str = "# >>> ss-magic: generated from .superset/magic.json - do not edit this \
                          section; lines outside it are kept >>>";
const END_LINE: &str = "# <<< ss-magic <<<";

/// What is on disk at `<main>/.worktreeinclude`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    /// No such file.
    Absent,
    /// The file's text.
    Text(String),
    /// Something is there that cannot be read as text – an I/O error, bytes
    /// that are not UTF-8, a directory. The string says what, for the notice.
    /// Never rewritten by any path.
    Unreadable(String),
}

/// Read `<main_root>/.worktreeinclude`.
pub fn read(main_root: &Path) -> Existing {
    let path = main_root.join(FILE_NAME);
    match fs::read(&path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Existing::Text(text),
            Err(_) => Existing::Unreadable("it is not valid UTF-8 text".to_string()),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Existing::Absent,
        Err(err) => Existing::Unreadable(format!("it could not be read ({err})")),
    }
}

/// Every literal entry in `text`, in file order, read the way Orca reads the
/// file: comments and blank lines skipped, each entry normalized, anything Orca
/// would reject left out, duplicates dropped. Covers the whole file – the
/// managed section and the hand-written lines alike.
pub fn entries(text: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    // Orca trims with JavaScript's `trim()`, which also strips a byte-order
    // mark; Rust's does not, so it is removed up front.
    for line in text.trim_start_matches('\u{feff}').split('\n') {
        let Some(entry) = normalize_entry(line) else {
            continue;
        };
        if seen.insert(entry.clone()) {
            out.push(entry);
        }
    }
    out
}

/// One line of the file as the entry Orca would use, or `None` for a comment,
/// a blank line, or an entry Orca rejects.
fn normalize_entry(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let slashed = trimmed.replace('\\', "/");
    let entry = slashed
        .strip_prefix("./")
        .unwrap_or(&slashed)
        .trim_end_matches('/');
    // Globs and negations are not supported by Orca; it warns and skips them.
    if entry.starts_with('!') || entry.contains('*') || entry.contains('?') {
        return None;
    }
    if entry.is_empty() || Path::new(entry).is_absolute() {
        return None;
    }
    let mut segments = entry.split('/');
    if segments.clone().next() == Some(".git") {
        return None;
    }
    if segments.any(|segment| segment.is_empty() || segment == "..") {
        return None;
    }
    Some(entry.to_string())
}

/// The paths forward sync leaves alone in `worktree_root` because Orca already
/// put them there: every `listed` entry whose worktree path exists (KTD10).
///
/// Existence is checked without following a symlink, so a link Orca copied
/// counts even when its target does not resolve here. A listed DIRECTORY that
/// exists covers everything under it: the returned filter holds the directory
/// itself, and a [`PathFilter`] entry covers whatever lies beneath it. A listed
/// path that is absent from the worktree – Orca's copy budget ran out before
/// it, or the file is new – contributes nothing, so forward sync still copies
/// it (R14).
pub fn copied_by_orca(worktree_root: &Path, listed: &[String]) -> PathFilter {
    PathFilter::new(
        listed
            .iter()
            .map(PathBuf::from)
            .filter(|rel| fs::symlink_metadata(worktree_root.join(rel)).is_ok()),
    )
}

/// Whether `path` can be written as a line that Orca reads back as exactly
/// that path. A name with leading or trailing white space, a leading `#` or
/// `!`, a `*`, a `?`, a backslash or a line break cannot – such a file is left
/// out of the snapshot and copied by ss-magic's own sync instead.
pub fn is_listable(path: &str) -> bool {
    !path.contains(['\n', '\r']) && normalize_entry(path).as_deref() == Some(path)
}

/// How much of Orca's budget the managed section may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Entries the section may hold.
    pub max_entries: usize,
    /// Bytes the section's entry lines may take, line breaks included.
    pub max_bytes: usize,
    /// Bytes each entry costs on top of its own text: its line break.
    pub per_entry_overhead: usize,
}

/// A `.worktreeinclude` split into the person's text and the managed section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Everything before the section, verbatim.
    before: String,
    /// Everything after the section, verbatim.
    after: String,
    /// The line ending the file uses, so the section matches it.
    eol: &'static str,
}

impl Document {
    /// Split `text`. The section runs from the first begin marker to the first
    /// end marker after it, both included.
    ///
    /// A begin marker with NO end marker is not treated as a section reaching
    /// to the end of the file: that would hand every following line to ss-magic
    /// to delete, and one of them may be the person's. Only the stray marker
    /// line is dropped; the lines after it are kept as theirs.
    pub fn parse(text: &str) -> Self {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let begin = lines
            .iter()
            .position(|line| line.trim().starts_with(BEGIN_PREFIX));
        let (before, after) = match begin {
            None => (text.to_string(), String::new()),
            Some(begin) => {
                let end = lines[begin + 1..]
                    .iter()
                    .position(|line| line.trim().starts_with(END_PREFIX))
                    .map(|offset| begin + 1 + offset);
                let resume = end.unwrap_or(begin) + 1;
                (lines[..begin].concat(), lines[resume..].concat())
            }
        };
        Self { before, after, eol }
    }

    /// The document for a file that does not exist yet.
    pub fn empty() -> Self {
        Self::parse("")
    }

    /// The literal entries outside the managed section – the person's.
    pub fn hand_entries(&self) -> Vec<String> {
        entries(&format!("{}\n{}", self.before, self.after))
    }

    /// What is left of Orca's limits once the person's lines and the two marker
    /// lines are accounted for. Their lines come first: the managed section
    /// only ever gets the remainder, so a rewrite can never push one of their
    /// entries past Orca's cap.
    pub fn budget(&self) -> Budget {
        let fixed = self.render(&[]).len();
        Budget {
            max_entries: MAX_ENTRIES.saturating_sub(self.hand_entries().len()),
            max_bytes: MAX_BYTES.saturating_sub(fixed),
            per_entry_overhead: self.eol.len(),
        }
    }

    /// The file's text with `managed` as the section's entries. The person's
    /// text is reproduced verbatim around it; rendering the result again with
    /// the same entries gives the same bytes.
    pub fn render(&self, managed: &[String]) -> String {
        let eol = self.eol;
        let mut out = String::with_capacity(self.before.len() + self.after.len() + 256);
        out.push_str(&self.before);
        // A last line without a line break must not have the marker glued on.
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(BEGIN_LINE);
        out.push_str(eol);
        for entry in managed {
            out.push_str(entry);
            out.push_str(eol);
        }
        out.push_str(END_LINE);
        out.push_str(eol);
        out.push_str(&self.after);
        out
    }
}

#[cfg(test)]
mod tests;
