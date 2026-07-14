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
//! git-untracked in the worktree (`git ls-files --others`, via
//! [`git::untracked_files`]). "Untracked" INCLUDES gitignored files — that is
//! the point: the files reverse sync pushes are secrets (`.env`, `.dev.vars`,
//! the gitignored `magic.local.json`), and those are gitignored by definition.
//! Tracked files are EXCLUDED — they reach main through a normal merge. So
//! `magic.local.json` (gitignored ⇒ untracked) flows through with no
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

use crate::sync::apply;
use crate::git;
use crate::git::gitignore;
use crate::tui::style;
use crate::workspace::superset_files;

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
pub fn compute_candidates(worktree_root: &Path) -> Result<Vec<PathBuf>> {
    let cfg = match superset_files::load_overlaid(worktree_root)? {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };
    if cfg.files.is_empty() {
        return Ok(Vec::new());
    }

    let matched = apply::match_paths(worktree_root, &cfg.files)?;
    if matched.is_empty() {
        return Ok(Vec::new());
    }

    // Ask git which of the matched paths are untracked, scoping the probe to
    // those paths (pathspecs) rather than listing every untracked file in the
    // tree. `git ls-files --others` (no `--exclude-standard`) would otherwise
    // walk every gitignored directory (`target/`, `node_modules/`, …) on each
    // reverse sync; restricting it to the matched paths makes it an index
    // lookup. `untracked` is therefore already ≈ the candidate set — but we
    // still intersect with `matched` so a matched DIRECTORY (whose pathspec
    // expands to its untracked inner files) contributes nothing: reverse sync
    // copies single files, never directories.
    let pathspecs: Vec<&str> = matched.iter().filter_map(|p| p.to_str()).collect();
    let untracked = git::untracked_files(worktree_root, &pathspecs)?;

    let matched_set: std::collections::HashSet<&Path> =
        matched.iter().map(|p| p.as_path()).collect();

    let mut out: Vec<PathBuf> = untracked
        .into_iter()
        .filter(|rel| matched_set.contains(rel.as_path()))
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

    // 2. Overwrite gate — only when the path already exists in main. Snapshot
    //    main's metadata first so we can detect a concurrent edit between the
    //    diff/confirm and the copy (TOCTOU guard).
    let pre_meta = fs::metadata(&main_path).ok();
    if pre_meta.is_some() && !overwrite(rel)? {
        return Ok(CopyOutcome::SkippedOverwriteDeclined);
    }
    // 2b. If main's file changed (size or mtime) while the user reviewed the
    //     diff, their confirmation was against stale content — skip rather than
    //     clobber the concurrent edit. Checked before any mutation, so a skip
    //     leaves main (and its .gitignore) fully untouched.
    if let (Some(before), Ok(now)) = (&pre_meta, fs::metadata(&main_path)) {
        if before.len() != now.len() || before.modified().ok() != now.modified().ok() {
            eprintln!(
                "{}",
                crate::tui::style::warn(format!(
                    "{} changed in main since the diff was shown — skipped to avoid \
                     clobbering a concurrent edit.",
                    rel.display()
                ))
            );
            return Ok(CopyOutcome::SkippedOverwriteDeclined);
        }
    }

    // 3. Parent dirs in main.
    if let Some(parent) = main_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", main_path.display()))?;
    }

    // 4. Gitignore safety BEFORE the copy — guarantee `rel` is ignored in main
    //    first, so a git failure here never leaves the (often secret) bytes on
    //    disk un-ignored. `git check-ignore` matches patterns regardless of
    //    whether the file exists yet, so checking before the copy is sound.
    let appended_gitignore = ensure_gitignored_in_main(worktree_root, main_root, rel)?;

    // 5. Copy worktree → main.
    fs::copy(&wt_path, &main_path)
        .with_context(|| format!("copy {} → {}", wt_path.display(), main_path.display()))?;

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

    let literal = rel
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?
        .to_string();

    // Prefer the worktree's covering rule (it generalizes protection, e.g.
    // `**/.dev.vars`). But a directory-anchored rule (e.g. `/.dev.vars` from
    // `apps/api/.gitignore`) does NOT match once appended to main's ROOT
    // `.gitignore`. So append it, then VERIFY it actually ignores `rel`; if it
    // doesn't, fall back to the literal repo-relative path, which is always
    // anchored at the root and therefore always matches. The secret MUST end
    // up ignored in main — this is the secret-leak boundary.
    if let Some(pattern) = gitignore::find_covering_rule(worktree_root, rel)? {
        gitignore::ensure_entry(main_root, &pattern)?;
        if git::is_ignored(main_root, rel)? {
            return Ok(true);
        }
    }

    gitignore::ensure_entry(main_root, &literal)?;
    debug_assert!(
        git::is_ignored(main_root, rel).unwrap_or(false),
        "literal-path fallback must leave the path ignored in main"
    );
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
    let selected = crate::tui::ui::pick_reverse_sync(worktree_root, main_root, &offered)?;
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
            crate::tui::ui::confirm_overwrite_with_diff(
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
mod tests;
