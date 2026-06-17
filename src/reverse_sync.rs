//! Reverse sync (U11): push git-UNTRACKED worktree files back into the main
//! checkout, safely.
//!
//! This is the ONE path that writes untracked (often secret) files into the
//! shared main checkout, so it is deliberately conservative. The plan's
//! "Secret-safety boundary": the gitignore-safety step (see
//! [`copy_candidate_into_main`]) is what prevents a reverse-synced `.dev.vars`
//! from becoming committable in main — a regression there is a secret leak,
//! not a cosmetic bug.
//!
//! ## What moves, and what doesn't (R23, KTD10)
//!
//! Candidates are files that BOTH match the worktree's overlaid patterns
//! (`magic.json` + `magic.local.json`, via [`apply::match_paths`]) AND are
//! git-untracked in the worktree (`git ls-files --others --exclude-standard`).
//! Tracked files are EXCLUDED — they reach main through a normal merge. This
//! naturally includes `magic.local.json` (gitignored ⇒ untracked) with no
//! special-casing.
//!
//! ## Structure: testable logic vs interactive TUI
//!
//! The candidate computation ([`compute_candidates`]), the diff classification
//! ([`classify`]), and the copy-into-main logic ([`copy_candidate_into_main`])
//! are pure / UI-free and unit-tested with `tempfile` + shell `git`. The
//! interactive picker, the "show diff" pager, and the overwrite confirm are
//! TUI/manual-smoke (consistent with the repo's final-action convention).
//! [`copy_candidate_into_main`] takes an `overwrite` decision via a closure
//! seam so the overwrite-needs-confirm / decline-leaves-intact logic is
//! testable without driving `inquire`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::apply;
use crate::git;
use crate::gitignore;
use crate::style;
use crate::superset_files;

/// Whether a candidate differs from main (and should be offered in the picker)
/// or is identical (and is hidden — nothing to push). R24.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
    /// The file exists in main with DIFFERENT bytes.
    Differs,
    /// The file does NOT exist in main (worktree-only / new).
    WorktreeOnly,
    /// The file exists in main with IDENTICAL bytes — hidden from the picker.
    Identical,
}

/// Outcome of attempting to copy one candidate into main, for caller-side
/// reporting. R25, R26.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    /// Copied; `gitignored` is true when a rule was appended (or already
    /// present) so the path is ignored in main.
    Copied { appended_gitignore: bool },
    /// The path already existed in main and the overwrite decision declined —
    /// main's copy is left intact.
    SkippedOverwriteDeclined,
}

/// Reject any relative path that could escape the main tree. The matcher and
/// the untracked probe already validate paths (`pattern::check_syntax` rejects
/// `..`/absolute; `git ls-files` emits only in-tree paths), so this is a
/// defense-in-depth guard right before we create dirs / copy into main.
fn is_safe_rel(rel: &Path) -> bool {
    if rel.is_absolute() {
        return false;
    }
    use std::path::Component;
    rel.components().all(|c| {
        matches!(
            c,
            Component::Normal(_) | Component::CurDir
        )
    })
}

/// Compute reverse-sync candidates for `worktree_root` (R23, KTD10):
/// files matching the worktree's overlaid patterns that are git-UNTRACKED.
///
/// Returns repo-relative paths, de-duped and sorted for stable ordering.
/// An absent `magic.json` in the worktree yields an empty candidate set
/// (nothing configured to sync). Defensively drops any path that fails the
/// in-tree safety check.
// consumed by U11 run(); wired into the menu by U10
#[allow(dead_code)]
pub fn compute_candidates(worktree_root: &Path) -> Result<Vec<PathBuf>> {
    let cfg = match superset_files::load_overlaid(worktree_root)? {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };
    if cfg.files.is_empty() {
        return Ok(Vec::new());
    }

    let matched = apply::match_paths(worktree_root, &cfg.files)?;
    let untracked = git::untracked_files(worktree_root)?;

    // Intersect: keep matched paths that are also untracked. `untracked` is
    // small enough that a HashSet is overkill; build one anyway for clarity
    // and O(1) lookup.
    let untracked_set: std::collections::HashSet<&Path> =
        untracked.iter().map(|p| p.as_path()).collect();

    let mut out: Vec<PathBuf> = matched
        .into_iter()
        .filter(|rel| untracked_set.contains(rel.as_path()))
        .filter(|rel| is_safe_rel(rel))
        .collect();

    out.sort();
    out.dedup();
    Ok(out)
}

/// Classify a single candidate against main for the diff-aware picker (R24).
///
/// Compares bytes: missing-in-main → [`DiffStatus::WorktreeOnly`]; present and
/// byte-equal → [`DiffStatus::Identical`]; present and different →
/// [`DiffStatus::Differs`]. A read error on main's copy is treated as
/// `Differs` (surface it in the picker rather than silently hide it).
// consumed by U11 run()
#[allow(dead_code)]
pub fn classify(main_root: &Path, worktree_root: &Path, rel: &Path) -> Result<DiffStatus> {
    let main_path = main_root.join(rel);
    if !main_path.exists() {
        return Ok(DiffStatus::WorktreeOnly);
    }
    let wt_path = worktree_root.join(rel);
    let main_bytes = fs::read(&main_path);
    let wt_bytes = fs::read(&wt_path)
        .with_context(|| format!("reading worktree file {}", wt_path.display()))?;
    match main_bytes {
        Ok(mb) if mb == wt_bytes => Ok(DiffStatus::Identical),
        _ => Ok(DiffStatus::Differs),
    }
}

/// Copy ONE selected candidate from the worktree into main, with all the
/// safety steps (R25, R26, KTD10):
///
/// 1. **Path safety:** reject any `rel` that escapes the main tree (`..`,
///    absolute) — returns an error rather than touching the filesystem.
/// 2. **Overwrite gate:** if `rel` already EXISTS in main, call `overwrite`
///    (the per-file diff + confirm decision seam). `Ok(false)` → leave main's
///    copy intact, return [`CopyOutcome::SkippedOverwriteDeclined`]. Only
///    `Ok(true)` proceeds to overwrite. A brand-new path skips the gate.
/// 3. **Parent dirs:** create any missing parent directories under main.
/// 4. **Copy** the worktree bytes over main's path.
/// 5. **Gitignore safety:** if the copied path is NOT already gitignored in
///    main, append a rule to main's root `.gitignore` — the worktree's
///    COVERING rule (via [`gitignore::find_covering_rule`]) when one exists,
///    else the literal relative path.
///
/// The `overwrite` closure is the test seam: production passes a closure that
/// shows the diff and prompts; tests pass a fixed decision.
// consumed by U11 run()
#[allow(dead_code)]
pub fn copy_candidate_into_main<O>(
    worktree_root: &Path,
    main_root: &Path,
    rel: &Path,
    overwrite: O,
) -> Result<CopyOutcome>
where
    O: FnOnce(&Path) -> Result<bool>,
{
    // 1. Path safety — never let a candidate escape main.
    if !is_safe_rel(rel) {
        anyhow::bail!(
            "refusing to reverse-sync unsafe path (escapes the main tree): {}",
            rel.display()
        );
    }

    let main_path = main_root.join(rel);
    let wt_path = worktree_root.join(rel);

    // 2. Overwrite gate — only when the path already exists in main.
    if main_path.exists() && !overwrite(rel)? {
        return Ok(CopyOutcome::SkippedOverwriteDeclined);
    }

    // 3. Parent dirs in main.
    if let Some(parent) = main_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", main_path.display()))?;
    }

    // 4. Copy worktree → main.
    fs::copy(&wt_path, &main_path)
        .with_context(|| format!("copy {} → {}", wt_path.display(), main_path.display()))?;

    // 5. Gitignore safety.
    let appended_gitignore = ensure_gitignored_in_main(worktree_root, main_root, rel)?;

    Ok(CopyOutcome::Copied { appended_gitignore })
}

/// Ensure `rel` is gitignored in main. Returns `true` when a new rule was
/// appended, `false` when it was already ignored (no-op).
///
/// "Already ignored in main" is checked via `git check-ignore` run in main.
/// When not ignored, the rule appended is the worktree's COVERING rule if one
/// exists (so `apps/api/.dev.vars` lands as `**/.dev.vars`, not the literal
/// path), else the literal relative path. `ensure_entry` is idempotent on the
/// exact line, so a covering rule that already happens to be present in main
/// produces no duplicate.
fn ensure_gitignored_in_main(
    worktree_root: &Path,
    main_root: &Path,
    rel: &Path,
) -> Result<bool> {
    // Already ignored in main → nothing to do.
    if git::is_ignored(main_root, rel)? {
        return Ok(false);
    }

    // Prefer the worktree's covering rule; fall back to the literal path.
    let rule = match gitignore::find_covering_rule(worktree_root, rel)? {
        Some(pattern) => pattern,
        None => rel
            .to_str()
            .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?
            .to_string(),
    };

    gitignore::ensure_entry(main_root, &rule)?;
    Ok(true)
}

/// Interactive reverse-sync entry point (TUI / manual-smoke): compute
/// candidates, hide identical ones, present the diff-aware picker, and copy
/// the user-selected subset into main with the overwrite + gitignore safety.
///
/// Empty candidate set → print a gray info line and return WITHOUT opening the
/// picker (R22). Decline at the picker → main fully untouched.
///
/// NOT unit-tested — the picker, the show-diff pager, and the overwrite confirm
/// are interactive. The logic it orchestrates is covered through
/// [`compute_candidates`], [`classify`], and [`copy_candidate_into_main`].
/// Wired into the menu by U10.
#[allow(dead_code)]
pub fn run(worktree_root: &Path, main_root: &Path) -> Result<ExitCode> {
    style::print_section("Reverse sync (untracked → main)");
    println!(
        "{}",
        style::info(format!("Worktree: {}", worktree_root.display()))
    );
    println!(
        "{}",
        style::info(format!("Main:     {}", main_root.display()))
    );

    let candidates = compute_candidates(worktree_root)?;
    if candidates.is_empty() {
        println!();
        println!(
            "{}",
            style::info("No untracked files match the configured patterns.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Diff-aware: keep only differing / worktree-only candidates (R24).
    let mut offered: Vec<(PathBuf, DiffStatus)> = Vec::new();
    for rel in &candidates {
        let status = classify(main_root, worktree_root, rel)?;
        if status != DiffStatus::Identical {
            offered.push((rel.clone(), status));
        }
    }

    if offered.is_empty() {
        println!();
        println!(
            "{}",
            style::info(
                "All untracked candidates are identical to main — nothing to push."
            )
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Hand off to the interactive picker. Selection + per-file overwrite
    // confirm live in `ui`; copying flows through `copy_candidate_into_main`.
    let selected = crate::ui::pick_reverse_sync(worktree_root, main_root, &offered)?;
    if selected.is_empty() {
        println!();
        println!("{}", style::info("Nothing selected — main untouched."));
        return Ok(ExitCode::SUCCESS);
    }

    let mut copied = 0usize;
    let mut skipped = 0usize;
    for rel in &selected {
        let main_root_for_overwrite = main_root.to_path_buf();
        let wt_root_for_overwrite = worktree_root.to_path_buf();
        let outcome = copy_candidate_into_main(worktree_root, main_root, rel, |rel| {
            crate::ui::confirm_overwrite_with_diff(
                &wt_root_for_overwrite,
                &main_root_for_overwrite,
                rel,
            )
        })?;
        match outcome {
            CopyOutcome::Copied { appended_gitignore } => {
                copied += 1;
                let ign = if appended_gitignore {
                    " (gitignore rule added)"
                } else {
                    ""
                };
                println!(
                    "{}",
                    style::ok(format!("Pushed to main: {}{ign}", rel.display()))
                );
            }
            CopyOutcome::SkippedOverwriteDeclined => {
                skipped += 1;
                println!(
                    "{}",
                    style::info(format!("Skipped (kept main's copy): {}", rel.display()))
                );
            }
        }
    }

    println!();
    let line = format!("Reverse sync done: copied {copied}, skipped {skipped}");
    if skipped == 0 {
        println!("{}", style::ok(line));
    } else {
        println!("{}", style::warn(line));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use tempfile::TempDir;

    // ── Git test helpers (mirror git.rs / main.rs sync_tests) ───────────────

    fn git_run(args: &[&str], cwd: &Path) {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            // Isolate from machine-level git config (e.g. commit.gpgsign=true).
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false")
            .stdout(Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed in {}:\n{}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_main_repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_run(&["init", "-q", "-b", "main"], dir.path());
        fs::write(dir.path().join("README.md"), "hi").unwrap();
        git_run(&["add", "."], dir.path());
        git_run(&["commit", "-q", "-m", "init"], dir.path());
        dir
    }

    fn make_worktree(main_dir: &Path) -> (TempDir, PathBuf) {
        let wt = tempfile::tempdir().unwrap();
        let wt_path = wt.path().join("wt");
        git_run(
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature/rs-test",
                wt_path.to_str().unwrap(),
            ],
            main_dir,
        );
        let wt_root = wt_path.canonicalize().unwrap();
        (wt, wt_root)
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn write_magic(root: &Path, patterns: &[&str]) {
        fs::create_dir_all(root.join(".superset")).unwrap();
        let files: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let cfg = superset_files::MagicConfig { files };
        let body = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
        fs::write(root.join(".superset/magic.json"), body).unwrap();
    }

    fn rels(v: &[PathBuf]) -> Vec<String> {
        let mut s: Vec<String> = v.iter().map(|p| p.to_string_lossy().to_string()).collect();
        s.sort();
        s
    }

    // ── compute_candidates ──────────────────────────────────────────────────

    /// AE9 (candidate side): untracked `apps/api/.dev.vars` IS a candidate;
    /// tracked `magic.json` is NOT (tracked files reach main via merge).
    #[test]
    fn ae9_untracked_is_candidate_tracked_is_not() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        // magic.json (tracked once committed) matches itself via a pattern,
        // plus an untracked secret.
        write_magic(&wt, &["**/.dev.vars", ".superset/magic.json"]);
        // Track magic.json so it's NOT untracked.
        git_run(&["add", ".superset/magic.json"], &wt);
        // Untracked secret matching the pattern.
        write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

        let cands = rels(&compute_candidates(&wt).unwrap());
        assert!(
            cands.contains(&"apps/api/.dev.vars".to_string()),
            "untracked secret must be a candidate; got {cands:?}"
        );
        assert!(
            !cands.contains(&".superset/magic.json".to_string()),
            "tracked magic.json must NOT be a candidate; got {cands:?}"
        );
    }

    /// magic.local.json (gitignored ⇒ untracked) flows through the same path
    /// with no special-casing.
    #[test]
    fn magic_local_json_is_candidate_when_matched_and_untracked() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        write_magic(&wt, &[".superset/magic.local.json"]);
        // magic.local.json present and untracked (gitignore not even needed —
        // it's simply not added).
        write(&wt, ".superset/magic.local.json", "{\"files\":[]}\n");

        let cands = rels(&compute_candidates(&wt).unwrap());
        assert!(
            cands.contains(&".superset/magic.local.json".to_string()),
            "magic.local.json must be a candidate; got {cands:?}"
        );
    }

    /// No magic.json in the worktree → empty candidate set (nothing configured).
    #[test]
    fn no_magic_json_yields_no_candidates() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());
        write(&wt, "apps/api/.dev.vars", "SECRET=1\n");
        let cands = compute_candidates(&wt).unwrap();
        assert!(cands.is_empty(), "got {cands:?}");
    }

    /// A tracked file matching the pattern is excluded even when its content
    /// differs from HEAD (modified-but-tracked still goes via merge).
    #[test]
    fn modified_tracked_file_is_not_a_candidate() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());
        write_magic(&wt, &["tracked.env"]);
        write(&wt, "tracked.env", "ORIG=1\n");
        git_run(&["add", "tracked.env"], &wt);
        git_run(&["commit", "-q", "-m", "add tracked.env"], &wt);
        // Modify it without committing.
        write(&wt, "tracked.env", "CHANGED=1\n");

        let cands = compute_candidates(&wt).unwrap();
        assert!(
            !rels(&cands).contains(&"tracked.env".to_string()),
            "modified-but-tracked file must NOT be a candidate; got {cands:?}"
        );
    }

    // ── classify (diff-aware) ────────────────────────────────────────────────

    /// Worktree-only file → WorktreeOnly; identical file → Identical (hidden);
    /// differing file → Differs.
    #[test]
    fn classify_distinguishes_new_identical_and_differing() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        // worktree-only
        write(&wt, "new.env", "NEW=1\n");
        // identical in both
        write(&wt, "same.env", "SAME=1\n");
        write(main.path(), "same.env", "SAME=1\n");
        // differing
        write(&wt, "diff.env", "WT=1\n");
        write(main.path(), "diff.env", "MAIN=1\n");

        assert_eq!(
            classify(main.path(), &wt, Path::new("new.env")).unwrap(),
            DiffStatus::WorktreeOnly
        );
        assert_eq!(
            classify(main.path(), &wt, Path::new("same.env")).unwrap(),
            DiffStatus::Identical
        );
        assert_eq!(
            classify(main.path(), &wt, Path::new("diff.env")).unwrap(),
            DiffStatus::Differs
        );
    }

    // ── copy_candidate_into_main ─────────────────────────────────────────────

    /// AE9 (copy side): copying `apps/api/.dev.vars` creates `apps/api/` in
    /// main and ensures the path is gitignored in main via its COVERING rule
    /// (`**/.dev.vars`), appended when absent.
    #[test]
    fn ae9_copy_creates_dirs_and_appends_covering_rule() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        // Worktree ignores the secret via a glob.
        write(&wt, ".gitignore", "**/.dev.vars\n");
        git_run(&["add", ".gitignore"], &wt);
        write(&wt, "apps/api/.dev.vars", "SECRET=1\n");

        let outcome = copy_candidate_into_main(
            &wt,
            main.path(),
            Path::new("apps/api/.dev.vars"),
            |_| Ok(true),
        )
        .unwrap();
        assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

        // Directory + file created in main.
        assert!(
            main.path().join("apps/api/.dev.vars").is_file(),
            "secret must be copied into main with parent dirs"
        );
        let copied = fs::read_to_string(main.path().join("apps/api/.dev.vars")).unwrap();
        assert_eq!(copied, "SECRET=1\n");

        // main's .gitignore now carries the COVERING glob, not the literal path.
        let gi = fs::read_to_string(main.path().join(".gitignore")).unwrap();
        assert!(
            gi.contains("**/.dev.vars"),
            "covering glob must be appended to main's .gitignore; got: {gi:?}"
        );
        assert!(
            !gi.contains("apps/api/.dev.vars"),
            "literal path must NOT be used when a covering rule exists; got: {gi:?}"
        );
        // And the path is now actually ignored in main.
        assert!(git::is_ignored(main.path(), Path::new("apps/api/.dev.vars")).unwrap());
    }

    /// No covering rule in the worktree → the literal relative path is appended.
    #[test]
    fn copy_appends_literal_path_when_no_covering_rule() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());
        // No worktree .gitignore covering the secret.
        write(&wt, "secrets/api.key", "KEY=1\n");

        let outcome =
            copy_candidate_into_main(&wt, main.path(), Path::new("secrets/api.key"), |_| {
                Ok(true)
            })
            .unwrap();
        assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

        let gi = fs::read_to_string(main.path().join(".gitignore")).unwrap();
        assert!(
            gi.contains("secrets/api.key"),
            "literal path must be appended when no covering rule; got: {gi:?}"
        );
    }

    /// Candidate already gitignored in main via an exact line → no duplicate
    /// line appended (already-ignored ⇒ no-op).
    #[test]
    fn copy_no_duplicate_when_already_gitignored_in_main() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        // main already ignores the path via an exact line.
        write(main.path(), ".gitignore", "secrets/api.key\n");
        git_run(&["add", ".gitignore"], main.path());
        write(&wt, "secrets/api.key", "KEY=1\n");

        let outcome =
            copy_candidate_into_main(&wt, main.path(), Path::new("secrets/api.key"), |_| {
                Ok(true)
            })
            .unwrap();
        assert_eq!(
            outcome,
            CopyOutcome::Copied { appended_gitignore: false },
            "already-ignored ⇒ no rule appended"
        );

        let gi = fs::read_to_string(main.path().join(".gitignore")).unwrap();
        assert_eq!(
            gi.matches("secrets/api.key").count(),
            1,
            "must not duplicate the existing line; got: {gi:?}"
        );
    }

    /// Candidate exists in main → overwrite requires the decision; decline
    /// leaves main's copy intact.
    #[test]
    fn copy_overwrite_declined_leaves_main_intact() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        write(main.path(), "config.env", "MAIN_ORIGINAL=1\n");
        write(&wt, "config.env", "WORKTREE_NEW=1\n");

        // Decision returns false → skip.
        let outcome = copy_candidate_into_main(&wt, main.path(), Path::new("config.env"), |_| {
            Ok(false)
        })
        .unwrap();
        assert_eq!(outcome, CopyOutcome::SkippedOverwriteDeclined);

        let after = fs::read_to_string(main.path().join("config.env")).unwrap();
        assert_eq!(
            after, "MAIN_ORIGINAL=1\n",
            "declining overwrite must leave main's copy untouched"
        );
    }

    /// Candidate exists in main → overwrite CONFIRMED replaces main's copy.
    #[test]
    fn copy_overwrite_confirmed_replaces_main() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        write(main.path(), "config.env", "MAIN_ORIGINAL=1\n");
        write(&wt, "config.env", "WORKTREE_NEW=1\n");

        let outcome = copy_candidate_into_main(&wt, main.path(), Path::new("config.env"), |_| {
            Ok(true)
        })
        .unwrap();
        assert!(matches!(outcome, CopyOutcome::Copied { .. }));

        let after = fs::read_to_string(main.path().join("config.env")).unwrap();
        assert_eq!(after, "WORKTREE_NEW=1\n", "confirmed overwrite must replace");
    }

    /// magic.local.json flows through the same copy path and lands gitignored.
    #[test]
    fn magic_local_json_lands_gitignored_in_main() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        // Worktree gitignores magic.local.json (the canonical bootstrap rule).
        write(&wt, ".gitignore", ".superset/magic.local.json\n");
        git_run(&["add", ".gitignore"], &wt);
        write(&wt, ".superset/magic.local.json", "{\"files\":[]}\n");

        let outcome = copy_candidate_into_main(
            &wt,
            main.path(),
            Path::new(".superset/magic.local.json"),
            |_| Ok(true),
        )
        .unwrap();
        assert_eq!(outcome, CopyOutcome::Copied { appended_gitignore: true });

        assert!(main.path().join(".superset/magic.local.json").is_file());
        assert!(
            git::is_ignored(main.path(), Path::new(".superset/magic.local.json")).unwrap(),
            "magic.local.json must be gitignored in main after copy"
        );
    }

    /// Path-safety: an absolute or `..`-bearing rel is rejected before any
    /// filesystem mutation.
    #[test]
    fn copy_rejects_unsafe_paths() {
        let main = init_main_repo();
        let (_wt, wt) = make_worktree(main.path());

        let err =
            copy_candidate_into_main(&wt, main.path(), Path::new("../escape.env"), |_| Ok(true))
                .unwrap_err();
        assert!(
            format!("{err:#}").contains("unsafe path"),
            "must reject `..` paths; got: {err:#}"
        );

        let err =
            copy_candidate_into_main(&wt, main.path(), Path::new("/etc/passwd"), |_| Ok(true))
                .unwrap_err();
        assert!(
            format!("{err:#}").contains("unsafe path"),
            "must reject absolute paths; got: {err:#}"
        );
    }

    #[test]
    fn is_safe_rel_accepts_normal_rejects_escapes() {
        assert!(is_safe_rel(Path::new("apps/api/.dev.vars")));
        assert!(is_safe_rel(Path::new(".superset/magic.local.json")));
        assert!(!is_safe_rel(Path::new("../oops")));
        assert!(!is_safe_rel(Path::new("a/../b")));
        assert!(!is_safe_rel(Path::new("/abs")));
    }
}
