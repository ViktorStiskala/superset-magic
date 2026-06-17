//! Apply mode: read the main checkout's `setup_config.json` and copy the
//! configured files into the current worktree. Re-implements `setup.sh`'s
//! semantics in Rust so users don't need bash 4 + jq.
//!
//! Parity surface with `setup.sh`:
//!
//! - Reject absolute patterns and patterns containing a `..` segment.
//! - Literal patterns must exist; missing literal → counted skip.
//! - Glob patterns may match zero entries → logged, not counted.
//! - `DEFAULT_EXCLUDES` (`node_modules`, `.venv`) drop matches at any
//!   depth; logged, not counted.
//! - De-duplicate matches across patterns by relative path.
//! - Directories are copied recursively (mirrors `cp -R src/. dst/`).
//! - Summary line is OK when `skipped == 0`, WARN otherwise.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use globset::{Glob, GlobMatcher};
use walkdir::WalkDir;

use crate::pattern::{self, SyntaxError};
use crate::superset_files::{self, SetupConfig};

/// Directory names that drop matches at any depth, matching `setup.sh`.
const DEFAULT_EXCLUDES: [&str; 2] = ["node_modules", ".venv"];

/// Result tally returned to the caller after `run`. Mirrors `setup.sh`'s
/// summary line.
#[derive(Debug, Default, Clone, Copy)]
pub struct Summary {
    pub copied: usize,
    pub skipped: usize,
}

/// Reason a pattern or path was skipped. `counts()` reports whether the
/// skip should bump the summary's `skipped` counter (matches `setup.sh`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    AbsolutePathRejected,
    ParentSegmentRejected,
    BadGlob(String),
    MissingLiteral,
    NoMatches,
    Excluded,
    NotAFileOrDir,
    CopyFailed(String),
}

impl SkipReason {
    pub fn counts(&self) -> bool {
        matches!(
            self,
            SkipReason::AbsolutePathRejected
                | SkipReason::ParentSegmentRejected
                | SkipReason::BadGlob(_)
                | SkipReason::MissingLiteral
                | SkipReason::NotAFileOrDir
                | SkipReason::CopyFailed(_)
        )
    }

    pub fn label(&self) -> &str {
        match self {
            SkipReason::AbsolutePathRejected => "absolute path rejected",
            SkipReason::ParentSegmentRejected => "\"..\" not allowed",
            SkipReason::BadGlob(_) => "bad glob",
            SkipReason::MissingLiteral => "missing",
            SkipReason::NoMatches => "no matches",
            SkipReason::Excluded => "excluded",
            SkipReason::NotAFileOrDir => "not a file or dir",
            SkipReason::CopyFailed(_) => "copy failed",
        }
    }
}

/// Per-item event emitted while expanding and copying. Callers convert
/// these to user-facing output (`main.rs`) or assertions (tests).
#[derive(Debug, Clone)]
pub enum Event {
    Copy {
        rel: PathBuf,
    },
    Skip {
        reason: SkipReason,
        /// Label printed after the reason — pattern text for pattern-level
        /// skips, relative path for per-file skips.
        label: String,
    },
}

/// Load the main checkout's `setup_config.json`, with a tailored error
/// when the file is absent — the user should run bootstrap mode first.
pub fn load_main_config(main_root: &Path) -> Result<SetupConfig> {
    match superset_files::load_setup_config(main_root)? {
        Some(cfg) => Ok(cfg),
        None => bail!(
            "no `.superset/setup_config.json` in {}; run `superset-setup` from the main \
             checkout on the main branch to bootstrap it first",
            main_root.display()
        ),
    }
}

/// Apply `patterns` from `src` into `dest`, calling `on_event` for each
/// copy and skip. Returns the final `Summary`.
pub fn run<F>(src: &Path, dest: &Path, patterns: &[String], mut on_event: F) -> Result<Summary>
where
    F: FnMut(&Event),
{
    let mut summary = Summary::default();

    let matches = {
        let mut counted = |ev: &Event| {
            if let Event::Skip { reason, .. } = ev {
                if reason.counts() {
                    summary.skipped += 1;
                }
            }
            on_event(ev);
        };
        expand_patterns(src, patterns, &mut counted)?
    };

    for rel in &matches {
        let src_path = src.join(rel);
        let dest_path = dest.join(rel);
        if let Some(parent) = dest_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                on_event(&Event::Skip {
                    reason: SkipReason::CopyFailed(e.to_string()),
                    label: rel.display().to_string(),
                });
                summary.skipped += 1;
                continue;
            }
        }
        let outcome: Result<()> = if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)
        } else if src_path.is_file() {
            fs::copy(&src_path, &dest_path).map(|_| ()).map_err(|e| {
                anyhow::Error::from(e).context(format!(
                    "copy {} → {}",
                    src_path.display(),
                    dest_path.display()
                ))
            })
        } else {
            on_event(&Event::Skip {
                reason: SkipReason::NotAFileOrDir,
                label: rel.display().to_string(),
            });
            summary.skipped += 1;
            continue;
        };
        match outcome {
            Ok(()) => {
                on_event(&Event::Copy { rel: rel.clone() });
                summary.copied += 1;
            }
            Err(err) => {
                on_event(&Event::Skip {
                    reason: SkipReason::CopyFailed(format!("{err:#}")),
                    label: rel.display().to_string(),
                });
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

/// Expand `patterns` against `src`, emitting skip events through
/// `on_event`. Returns the deduped list of relative paths to copy.
fn expand_patterns<F>(src: &Path, patterns: &[String], on_event: &mut F) -> Result<Vec<PathBuf>>
where
    F: FnMut(&Event),
{
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut matches: Vec<PathBuf> = Vec::new();

    // Cache the source walk for any glob pattern that needs it. We walk
    // once, lazily, sharing across patterns.
    let mut all_files: Option<Vec<PathBuf>> = None;

    for pat in patterns {
        if let Err(err) = pattern::check_syntax(pat) {
            let reason = match err {
                SyntaxError::Empty | SyntaxError::AbsolutePath => SkipReason::AbsolutePathRejected,
                SyntaxError::ParentSegment => SkipReason::ParentSegmentRejected,
                SyntaxError::BadGlob(msg) => SkipReason::BadGlob(msg),
            };
            // Empty patterns are reported as absolute-path rejects only as
            // a fallback — `check_syntax` would have returned Empty, but
            // a real `setup_config.json` can't carry one through serde.
            // Map the rare case rather than add a SkipReason variant.
            on_event(&Event::Skip {
                reason,
                label: pat.clone(),
            });
            continue;
        }
        if pattern::has_glob_meta(pat) {
            // `pattern::check_syntax` above already verified the glob
            // compiles, so build_matcher can't fail here.
            let matcher = build_matcher(pat).expect("glob compiled in check_syntax");
            let entries = all_files.get_or_insert_with(|| walk_source(src));

            let mut raw_hit = false;
            for rel in entries.iter() {
                if !matcher.is_match(rel) {
                    continue;
                }
                raw_hit = true;
                if is_excluded(rel) {
                    on_event(&Event::Skip {
                        reason: SkipReason::Excluded,
                        label: rel.display().to_string(),
                    });
                    continue;
                }
                if seen.insert(rel.clone()) {
                    matches.push(rel.clone());
                }
            }
            if !raw_hit {
                on_event(&Event::Skip {
                    reason: SkipReason::NoMatches,
                    label: pat.clone(),
                });
            }
        } else {
            // Literal pattern.
            let abs = src.join(pat);
            if !abs.exists() {
                on_event(&Event::Skip {
                    reason: SkipReason::MissingLiteral,
                    label: pat.clone(),
                });
                continue;
            }
            let rel = PathBuf::from(pat);
            if is_excluded(&rel) {
                on_event(&Event::Skip {
                    reason: SkipReason::Excluded,
                    label: pat.clone(),
                });
                continue;
            }
            if seen.insert(rel.clone()) {
                matches.push(rel);
            }
        }
    }
    Ok(matches)
}

/// Walk `src` once and return every file/dir's path relative to `src`,
/// including directories so a pattern like `apps/*/config` can match them.
fn walk_source(src: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in WalkDir::new(src).follow_links(false).into_iter().flatten() {
        if entry.depth() == 0 {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(src) {
            out.push(rel.to_path_buf());
        }
    }
    out
}

fn build_matcher(pattern: &str) -> Result<GlobMatcher> {
    Ok(Glob::new(pattern)
        .with_context(|| format!("compiling glob `{pattern}`"))?
        .compile_matcher())
}

fn is_excluded(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        std::path::Component::Normal(name) => DEFAULT_EXCLUDES
            .iter()
            .any(|ex| name == std::ffi::OsStr::new(ex)),
        _ => false,
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("mkdir -p {}", dst.display()))?;
    for entry in WalkDir::new(src).follow_links(false) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("mkdir -p {}", target.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("mkdir -p {}", parent.display()))?;
            }
            fs::copy(entry.path(), &target).with_context(|| {
                format!("copy {} → {}", entry.path().display(), target.display())
            })?;
        }
        // Skip symlinks and other special files — matches `cp -R`'s
        // pragmatic posture without leaking outside the source tree.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn collect(src: &Path, dest: &Path, patterns: &[&str]) -> (Summary, Vec<Event>) {
        let pats: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let mut events = Vec::new();
        let summary = run(src, dest, &pats, |e| events.push(e.clone())).unwrap();
        (summary, events)
    }

    fn copy_events_of(events: &[Event]) -> Vec<&Path> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Copy { rel } => Some(rel.as_path()),
                _ => None,
            })
            .collect()
    }

    fn skip_events_of(events: &[Event]) -> Vec<(&SkipReason, &str)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Skip { reason, label } => Some((reason, label.as_str())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn copies_dotenv_at_root() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), ".env", "FOO=1\n");
        let (summary, events) = collect(src.path(), dest.path(), &[".env"]);
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.skipped, 0);
        assert!(dest.path().join(".env").is_file());
        assert_eq!(copy_events_of(&events), vec![Path::new(".env")]);
    }

    #[test]
    fn glob_matches_multiple_directories_recursively() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), "apps/api/config/a.txt", "a");
        write(src.path(), "apps/web/config/b.txt", "b");
        write(src.path(), "apps/api/other.txt", "c");
        let (summary, _) = collect(src.path(), dest.path(), &["apps/*/config"]);
        assert!(dest.path().join("apps/api/config/a.txt").is_file());
        assert!(dest.path().join("apps/web/config/b.txt").is_file());
        assert!(!dest.path().join("apps/api/other.txt").exists());
        assert_eq!(summary.skipped, 0);
        assert!(summary.copied >= 2);
    }

    #[test]
    fn node_modules_matches_are_dropped() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), "apps/api/.dev.vars", "ok");
        write(src.path(), "node_modules/foo/.dev.vars", "drop");
        let (summary, events) = collect(src.path(), dest.path(), &["**/.dev.vars"]);
        assert!(dest.path().join("apps/api/.dev.vars").is_file());
        assert!(!dest.path().join("node_modules/foo/.dev.vars").exists());
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.skipped, 0);
        let skips = skip_events_of(&events);
        assert!(
            skips.iter().any(|(r, _)| matches!(r, SkipReason::Excluded)),
            "expected an Excluded skip event, got: {skips:?}"
        );
    }

    #[test]
    fn glob_with_zero_matches_is_non_fatal_and_uncounted() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let (summary, events) = collect(src.path(), dest.path(), &["**/.env"]);
        assert_eq!(summary.copied, 0);
        assert_eq!(summary.skipped, 0, "no-matches must not count");
        let skips = skip_events_of(&events);
        assert!(skips
            .iter()
            .any(|(r, _)| matches!(r, SkipReason::NoMatches)));
    }

    #[test]
    fn existing_destination_files_are_overwritten() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), ".env", "NEW=1\n");
        write(dest.path(), ".env", "OLD=1\n");
        let (summary, _) = collect(src.path(), dest.path(), &[".env"]);
        assert_eq!(summary.copied, 1);
        let body = fs::read_to_string(dest.path().join(".env")).unwrap();
        assert_eq!(body, "NEW=1\n");
    }

    #[test]
    fn absolute_pattern_is_rejected_and_counted() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let (summary, events) = collect(src.path(), dest.path(), &["/etc/passwd"]);
        assert_eq!(summary.copied, 0);
        assert_eq!(summary.skipped, 1);
        let skips = skip_events_of(&events);
        assert!(skips
            .iter()
            .any(|(r, _)| matches!(r, SkipReason::AbsolutePathRejected)));
    }

    #[test]
    fn parent_segment_pattern_is_rejected_and_counted() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let (summary, events) = collect(src.path(), dest.path(), &["../oops"]);
        assert_eq!(summary.copied, 0);
        assert_eq!(summary.skipped, 1);
        let skips = skip_events_of(&events);
        assert!(skips
            .iter()
            .any(|(r, _)| matches!(r, SkipReason::ParentSegmentRejected)));
    }

    #[test]
    fn missing_literal_is_counted() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let (summary, events) = collect(src.path(), dest.path(), &[".env"]);
        assert_eq!(summary.copied, 0);
        assert_eq!(summary.skipped, 1);
        let skips = skip_events_of(&events);
        assert!(skips
            .iter()
            .any(|(r, _)| matches!(r, SkipReason::MissingLiteral)));
    }

    #[test]
    fn rejected_patterns_dont_abort_processing() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), ".env", "x");
        let (summary, _) = collect(src.path(), dest.path(), &["/etc/passwd", ".env"]);
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.skipped, 1);
        assert!(dest.path().join(".env").is_file());
    }

    #[test]
    fn missing_main_config_has_helpful_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_main_config(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("setup_config.json"));
        assert!(
            msg.contains("bootstrap") || msg.contains("main checkout"),
            "expected hint mentioning bootstrap/main checkout, got: {msg}"
        );
    }

    // ── Characterization tests: pin engine semantics before config source changes ──

    /// `**` depth: a `**/<name>` pattern must match at any nesting depth,
    /// including deep (3+ levels) and shallow (1 level).
    #[test]
    fn double_glob_matches_at_any_depth() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), ".dev.vars", "root");
        write(src.path(), "apps/api/.dev.vars", "l1");
        write(src.path(), "apps/api/nested/deep/.dev.vars", "deep");
        let (summary, events) = collect(src.path(), dest.path(), &["**/.dev.vars"]);
        assert!(dest.path().join(".dev.vars").is_file(), "root-level match");
        assert!(
            dest.path().join("apps/api/.dev.vars").is_file(),
            "one-level match"
        );
        assert!(
            dest.path().join("apps/api/nested/deep/.dev.vars").is_file(),
            "deep match"
        );
        assert_eq!(summary.copied, 3, "all three depths must be copied");
        assert_eq!(
            copy_events_of(&events).len(),
            3,
            "three Copy events expected"
        );
    }

    /// `.venv` exclusion: matches inside a `.venv` dir at any depth are
    /// silently dropped (non-fatal, not counted).
    #[test]
    fn venv_matches_are_dropped() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), "apps/api/.env", "ok");
        write(src.path(), ".venv/lib/python3.11/.env", "drop");
        write(src.path(), "packages/foo/.venv/pyvenv.cfg", "drop");
        let (summary, events) = collect(src.path(), dest.path(), &["**/.env", "**/*.cfg"]);
        assert!(
            dest.path().join("apps/api/.env").is_file(),
            "real .env must be copied"
        );
        assert!(
            !dest.path().join(".venv/lib/python3.11/.env").exists(),
            ".venv/.env must be excluded"
        );
        assert!(
            !dest.path()
                .join("packages/foo/.venv/pyvenv.cfg")
                .exists(),
            ".venv/*.cfg must be excluded"
        );
        assert_eq!(summary.copied, 1, "only the real .env is copied");
        assert_eq!(summary.skipped, 0, "excluded items must not count");
        let skips = skip_events_of(&events);
        assert!(
            skips
                .iter()
                .filter(|(r, _)| matches!(r, SkipReason::Excluded))
                .count()
                >= 2,
            "expected at least two Excluded skip events, got: {skips:?}"
        );
    }

    /// Characterization: `*` in globset matches path separators (unlike POSIX shell
    /// glob). `apps/*/.env` therefore matches both `apps/api/.env` AND
    /// `apps/api/nested/.env`. This pins the engine semantics so callers don't
    /// silently rely on `*` = "one component only".
    #[test]
    fn single_star_matches_across_path_separators_in_globset() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), "apps/api/.env", "ok");
        write(src.path(), "apps/api/nested/.env", "nested");
        // globset's `*` is NOT literal-separator-aware by default, so
        // `apps/*/.env` matches paths at any depth below `apps/` ending in `/.env`.
        let (summary, _) = collect(src.path(), dest.path(), &["apps/*/.env"]);
        assert!(
            dest.path().join("apps/api/.env").is_file(),
            "direct child must match"
        );
        assert!(
            dest.path().join("apps/api/nested/.env").is_file(),
            "globset `*` crosses path separators — nested path also matches"
        );
        assert_eq!(summary.copied, 2);
    }

    /// Directory copies via `apps/*/config` pattern: the directory itself is
    /// matched (not its entries), so all files inside are copied recursively.
    #[test]
    fn matched_directory_is_copied_recursively() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), "apps/api/config/a.toml", "a");
        write(src.path(), "apps/api/config/sub/b.toml", "b");
        let (summary, _) = collect(src.path(), dest.path(), &["apps/api/config"]);
        assert!(
            dest.path().join("apps/api/config/a.toml").is_file(),
            "top-level file in dir"
        );
        assert!(
            dest.path().join("apps/api/config/sub/b.toml").is_file(),
            "nested file in dir"
        );
        // The directory itself is one matched entry; its contents are copied
        // recursively — summary.copied == 1 (the dir match), not 2 (the files).
        assert_eq!(summary.copied, 1, "directory counts as one copy event");
        assert_eq!(summary.skipped, 0);
    }

    /// De-duplication: a path matched by two different patterns appears only once.
    #[test]
    fn duplicate_match_across_patterns_is_deduplicated() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write(src.path(), ".env", "x");
        // Both patterns match `.env`.
        let (summary, events) = collect(src.path(), dest.path(), &[".env", "**/.env"]);
        assert_eq!(summary.copied, 1, ".env must be copied exactly once");
        assert_eq!(
            copy_events_of(&events).len(),
            1,
            "only one Copy event for a deduped match"
        );
    }
}
