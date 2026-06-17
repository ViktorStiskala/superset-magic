//! Helpers for managing `.gitignore` at a git repository root.
//!
//! All helpers follow the convention: never reorder or rewrite existing
//! content; only append when the exact line is missing; create the file
//! if absent.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

/// Append `line` to `<git_root>/.gitignore` if no EXACT line match already
/// exists.  Creates `.gitignore` if the file is absent.
///
/// - Existing content is NEVER reordered or rewritten.
/// - The appended entry is placed on its own line.
/// - A single trailing newline is preserved: if the file's last byte is
///   already `\n` the entry is appended directly; otherwise a newline is
///   inserted before it.
// consumed by U9 (bootstrap) and U11 (reverse sync)
#[allow(dead_code)]
pub fn ensure_entry(git_root: &Path, line: &str) -> Result<()> {
    let path = git_root.join(".gitignore");

    if path.exists() {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        // Exact line match: any line in the file equals `line` after stripping
        // the trailing newline.
        let already_present = contents.lines().any(|l| l == line);
        if already_present {
            return Ok(());
        }

        // Append: ensure the new entry starts on its own line.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {} for append", path.display()))?;

        // If the file is non-empty and its last byte is not a newline,
        // emit one first so the entry starts on its own line.
        if !contents.is_empty() && !contents.ends_with('\n') {
            file.write_all(b"\n")
                .with_context(|| format!("writing {}", path.display()))?;
        }
        writeln!(file, "{}", line).with_context(|| format!("writing {}", path.display()))?;
    } else {
        // File absent — create it with just the entry + trailing newline.
        fs::write(&path, format!("{}\n", line))
            .with_context(|| format!("creating {}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    /// .gitignore absent → created containing exactly the entry + trailing NL.
    #[test]
    fn creates_file_when_absent() {
        let dir = fresh();
        ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

        let got = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(got, ".superset/magic.local.json\n");
    }

    /// Entry already present → file is byte-identical (no modification).
    #[test]
    fn idempotent_when_entry_present() {
        let dir = fresh();
        let gi = dir.path().join(".gitignore");
        let initial = "# auto-generated\n.superset/magic.local.json\nnode_modules/\n";
        fs::write(&gi, initial).unwrap();

        ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

        let after = fs::read_to_string(&gi).unwrap();
        assert_eq!(after, initial, "file must be byte-identical");
    }

    /// Entry absent among other lines → appended; existing lines untouched.
    #[test]
    fn appends_when_entry_absent_among_others() {
        let dir = fresh();
        let gi = dir.path().join(".gitignore");
        let initial = "# keep\nnode_modules/\n.env\n";
        fs::write(&gi, initial).unwrap();

        ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

        let after = fs::read_to_string(&gi).unwrap();
        // Existing lines must still be there.
        assert!(after.starts_with(initial), "existing content must be preserved at the start");
        // The new entry must appear at the end.
        assert!(
            after.ends_with(".superset/magic.local.json\n"),
            "new entry must be appended with trailing newline; got: {after:?}"
        );
    }

    /// File missing trailing newline → newline inserted before the entry.
    #[test]
    fn inserts_newline_when_file_lacks_trailing_newline() {
        let dir = fresh();
        let gi = dir.path().join(".gitignore");
        // No trailing newline.
        fs::write(&gi, "node_modules/").unwrap();

        ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

        let after = fs::read_to_string(&gi).unwrap();
        assert_eq!(after, "node_modules/\n.superset/magic.local.json\n");
    }

    /// Empty file → entry appended normally.
    #[test]
    fn handles_empty_file() {
        let dir = fresh();
        let gi = dir.path().join(".gitignore");
        fs::write(&gi, "").unwrap();

        ensure_entry(dir.path(), "secret.txt").unwrap();

        let after = fs::read_to_string(&gi).unwrap();
        assert_eq!(after, "secret.txt\n");
    }

    /// Partial match (line is a prefix of an existing entry) is not treated
    /// as "already present" — the entry must be exact.
    #[test]
    fn partial_match_is_not_exact_match() {
        let dir = fresh();
        let gi = dir.path().join(".gitignore");
        fs::write(&gi, ".superset/magic.local.json.bak\n").unwrap();

        ensure_entry(dir.path(), ".superset/magic.local.json").unwrap();

        let after = fs::read_to_string(&gi).unwrap();
        assert!(
            after.contains(".superset/magic.local.json\n"),
            "entry must be appended; got: {after:?}"
        );
    }
}
