//! Reverse sync (U11): push git-UNTRACKED worktree files back into the main
//! checkout, safely.
//!
//! This is the ONE path that writes untracked (often secret) files into the
//! shared main checkout, so it is deliberately conservative. The plan's
//! "Secret-safety boundary": the gitignore-safety step (see
//! [`ensure_gitignored_in_main`], invoked from [`apply_decision`]) is what
//! prevents a reverse-synced `.dev.vars` from becoming committable in main — a
//! regression there is a secret leak, not a cosmetic bug.
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
//! ([`classify`]), and the backup-first apply seam ([`apply_decision`]) are
//! pure / UI-free and unit-tested with `tempfile` + shell `git`. The
//! interactive merge cockpit ([`crate::tui::cockpit`]) is TUI/manual-smoke
//! (consistent with the repo's final-action convention); it returns the user's
//! per-file decisions and [`run`] feeds each through [`apply_decision`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::sync::apply;
use crate::sync::merge::{backup_rel_path, Decision};
use crate::git;
use crate::git::gitignore;
use crate::tui::cockpit::{self, CockpitOutcome};
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
/// NOT unit-tested — the merge cockpit is interactive. The logic it orchestrates
/// is covered through [`compute_candidates`], [`classify`], and
/// [`apply_decision`]. Wired into the menu by U10.
pub fn run(worktree_root: &Path, main_root: &Path) -> Result<ExitCode> {
    let candidates = compute_candidates(worktree_root)?;
    if candidates.is_empty() {
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
        println!(
            "{}",
            style::info("All untracked candidates are identical to main — nothing to push.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // R16: the merge cockpit is a full-screen TUI and MUST have a terminal.
    // A piped / CI / hook invocation refuses to launch and writes NOTHING,
    // pointing at the (forward-only) non-interactive path instead.
    if !cockpit::is_interactive() {
        eprintln!(
            "{}",
            style::err(
                "error: reverse sync needs an interactive terminal — the merge cockpit \
                 cannot run piped or in CI."
            )
        );
        eprintln!(
            "{}",
            style::info(
                "`ss-magic sync` is forward-only (main → worktree); reverse sync has no \
                 non-interactive mode. Re-run it in a terminal."
            )
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Capture a review-time baseline of every offered file's metadata on BOTH
    // sides, BEFORE the (possibly long) interactive review opens. The apply-time
    // guard compares this baseline against on-disk metadata at write time, so
    // any change that lands during the review→apply window — an edit, a create,
    // or a delete — is detected and that file is skipped rather than clobbered
    // (KD4 / R13–R15).
    let mut baseline: HashMap<PathBuf, (Option<FileMeta>, Option<FileMeta>)> = HashMap::new();
    for (rel, _status) in &offered {
        let wt_meta = meta_of(&worktree_root.join(rel))?;
        let main_meta = meta_of(&main_root.join(rel))?;
        baseline.insert(rel.clone(), (wt_meta, main_meta));
    }

    // Full-screen cockpit: the user sets each file's direction and either
    // cancels (main untouched) or confirms a batch of decisions.
    let decisions = match cockpit::run_cockpit(worktree_root, main_root, &offered)? {
        CockpitOutcome::Cancel => {
            println!("{}", style::info("Nothing selected — main untouched."));
            return Ok(ExitCode::SUCCESS);
        }
        CockpitOutcome::Apply(d) => d,
    };
    if decisions.is_empty() {
        println!("{}", style::info("Nothing selected — main untouched."));
        return Ok(ExitCode::SUCCESS);
    }

    // One timestamp for the whole apply; backups live under a gitignored
    // `.superset/backups/` in the worktree so recovered secret bytes are never
    // committed (reuse `ensure_entry` — idempotent, appends only when absent).
    let ts = apply_timestamp();
    let backups_root = worktree_root.join(".superset/backups");
    gitignore::ensure_entry(worktree_root, ".superset/backups/")?;

    let mut applied = 0usize;
    let mut skipped = 0usize;
    let mut all_backups: Vec<PathBuf> = Vec::new();
    for (rel, decision) in &decisions {
        let (wt_baseline, main_baseline) = baseline.get(rel).cloned().unwrap_or((None, None));
        match apply_decision(
            worktree_root,
            main_root,
            &backups_root,
            &ts,
            rel,
            decision,
            wt_baseline,
            main_baseline,
        )? {
            ApplyOutcome::Applied(result) => {
                applied += 1;
                let dir = match result.direction {
                    WriteDirection::PushToMain => "worktree → main",
                    WriteDirection::PullFromMain => "main → worktree",
                    WriteDirection::MergeBoth => "merged → both",
                };
                let ign = if result.gitignore_appended {
                    " (gitignore rule added)"
                } else {
                    ""
                };
                println!(
                    "{}",
                    style::ok(format!("Applied {} [{dir}]{ign}", rel.display()))
                );
                all_backups.extend(result.backups);
            }
            ApplyOutcome::Skipped(reason) => {
                skipped += 1;
                println!(
                    "{}",
                    style::warn(format!("Skipped {}: {reason}", rel.display()))
                );
            }
        }
    }

    if !all_backups.is_empty() {
        println!();
        println!(
            "{}",
            style::info("Backups of overwritten files (recover a mistake here):")
        );
        for backup in &all_backups {
            println!("{}", style::info(format!("  {}", backup.display())));
        }
    }

    println!();
    let line = format!("Reverse sync done: applied {applied}, skipped {skipped}");
    if skipped == 0 {
        println!("{}", style::ok(line));
    } else {
        println!("{}", style::warn(line));
    }
    Ok(ExitCode::SUCCESS)
}

/// Timestamp string for a batch of backups. Seconds since the Unix epoch — a
/// unique, monotonic-enough directory name with no extra dependency (the plan
/// permits a plain epoch instead of `%Y%m%d-%H%M%S`).
fn apply_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

// ── Apply seam (reverse-sync merge cockpit) ──────────────────────────────
//
// The cockpit's safe, backup-first apply path. It writes a decision (push /
// pull / merge) to disk: a path-safety guard, a review-time-baseline re-check
// that skips any target changed since the user reviewed it, a timestamped
// pre-write backup of the losing bytes, and `ensure_gitignored_in_main` BEFORE
// any secret bytes land in main. Driven by [`run`] with the decisions returned
// from the cockpit — plus the per-file `(worktree, main)` metadata baseline
// captured before the cockpit opened — including the `Decision::Merge` produced
// by the cockpit's interactive-merge overlay, whose assembled bytes this seam
// writes to BOTH sides.

/// Lightweight metadata snapshot backing the review-time TOCTOU baseline: a
/// byte length plus a best-effort mtime (some filesystems omit mtime, hence the
/// `Option`).
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// The file's length in bytes.
    pub len: u64,
    /// The file's modification time, when the platform / filesystem reports one.
    pub mtime: Option<SystemTime>,
}

/// Snapshot `path`'s metadata for the TOCTOU baseline.
///
/// Returns `Ok(None)` ONLY when the path does not exist (`ErrorKind::NotFound`);
/// `Ok(Some(..))` when it exists; and propagates any OTHER io error (permissions,
/// I/O) via `?`. A non-`NotFound` stat error must NEVER be silently read as
/// "missing" — doing so would skip the mandatory pre-overwrite backup for a
/// target that actually exists.
pub fn meta_of(path: &Path) -> Result<Option<FileMeta>> {
    match fs::metadata(path) {
        Ok(m) => Ok(Some(FileMeta {
            len: m.len(),
            mtime: m.modified().ok(),
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading metadata of {}", path.display())),
    }
}

/// Which direction (and how many sides) an applied decision wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteDirection {
    /// Worktree bytes were written to main.
    PushToMain,
    /// Main bytes were written to the worktree.
    PullFromMain,
    /// Assembled merge bytes were written to BOTH sides.
    MergeBoth,
}

/// The successful outcome of [`apply_decision`]: what was written, the
/// timestamped backups taken of the overwritten (losing) bytes, and whether a
/// gitignore rule was appended in main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    /// The direction the bytes moved.
    pub direction: WriteDirection,
    /// Backup paths written before each destructive overwrite (empty when the
    /// target was newly created and had no prior bytes).
    pub backups: Vec<PathBuf>,
    /// True when a rule was appended to main's `.gitignore` for this path.
    pub gitignore_appended: bool,
}

/// The result of [`apply_decision`]: either an applied write or a skip with a
/// human-readable reason (undecided, or a concurrent edit since review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The decision was applied; carries the write details.
    Applied(ApplyResult),
    /// Nothing was written; the string is a human-readable reason.
    Skipped(String),
}

/// Per-target verdict from [`check_target`], comparing a review-time baseline
/// against the target's current metadata.
enum Guard {
    /// The target did not exist at review time and still does not — a fresh
    /// write, no backup needed.
    Missing,
    /// The target exists and is byte-for-byte unchanged (len + mtime) since the
    /// review baseline — safe to back up and overwrite.
    Unchanged,
    /// The target changed since the review baseline — edited, created, or
    /// deleted during the review→apply window — so the caller must skip it.
    Changed,
}

/// Compare `target`'s review-time `baseline` against its CURRENT metadata to
/// decide whether the review→apply window stayed quiet:
///
/// - baseline `None` + now `None`   → [`Guard::Missing`] (fresh write)
/// - baseline `None` + now `Some`   → [`Guard::Changed`] (appeared during review)
/// - baseline `Some` + now `None`   → [`Guard::Changed`] (vanished during review)
/// - baseline `Some(b)` + now `Some(c)` → [`Guard::Unchanged`] iff
///   `b.len == c.len && b.mtime == c.mtime`, else [`Guard::Changed`]
///
/// A non-`NotFound` stat error at apply time is treated as [`Guard::Changed`]
/// (fail safe: never overwrite a target we cannot reliably re-stat).
fn check_target(target: &Path, baseline: Option<&FileMeta>) -> Guard {
    let current = match meta_of(target) {
        Ok(c) => c,
        Err(_) => return Guard::Changed,
    };
    match (baseline, current) {
        (None, None) => Guard::Missing,
        (None, Some(_)) | (Some(_), None) => Guard::Changed,
        (Some(b), Some(c)) => {
            if b.len == c.len && b.mtime == c.mtime {
                Guard::Unchanged
            } else {
                Guard::Changed
            }
        }
    }
}

/// Copy `target`'s CURRENT bytes to `dest` (creating parent dirs) before it is
/// overwritten, returning the backup path.
fn backup(target: &Path, dest: &Path) -> Result<PathBuf> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating backup dir {}", parent.display()))?;
    }
    fs::copy(target, dest)
        .with_context(|| format!("backing up {} → {}", target.display(), dest.display()))?;
    Ok(dest.to_path_buf())
}

/// Write `bytes` to `target`, creating any missing parent directories first.
fn write_bytes(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", target.display()))?;
    }
    fs::write(target, bytes).with_context(|| format!("writing {}", target.display()))?;
    Ok(())
}

/// Apply one cockpit [`Decision`] for `rel`, safely (KD4, R13–R15).
///
/// Safety order: an unsafe `rel` bails; an [`Decision::Undecided`] is a no-op
/// skip; every destructive overwrite of an existing target is guarded against
/// its review-time baseline (`wt_baseline` for the worktree side, `main_baseline`
/// for the main side — each captured by [`meta_of`] before the cockpit opened)
/// and its losing bytes are backed up under `backups_root` (via
/// [`backup_rel_path`]) BEFORE the write; every write into main is preceded by
/// [`ensure_gitignored_in_main`] so a git failure never leaves un-ignored secret
/// bytes on disk. `Merge` writes the assembled text to BOTH sides (distinct
/// `local/`+`main/` backup dirs so neither original is lost). A target that no
/// longer matches its baseline (edited, created, or deleted since review) yields
/// [`ApplyOutcome::Skipped`] with nothing written — no overwrite and no partial
/// backup.
#[allow(clippy::too_many_arguments)]
pub fn apply_decision(
    worktree_root: &Path,
    main_root: &Path,
    backups_root: &Path,
    ts: &str,
    rel: &Path,
    decision: &Decision,
    wt_baseline: Option<FileMeta>,
    main_baseline: Option<FileMeta>,
) -> Result<ApplyOutcome> {
    // Universal path-safety guard (defense-in-depth) — reject anything that
    // could escape a tree.
    if !is_safe_rel(rel) {
        anyhow::bail!(
            "refusing to reverse-sync unsafe path (escapes the tree): {}",
            rel.display()
        );
    }

    let changed_reason = || format!("{} changed since review", rel.display());

    match decision {
        Decision::Undecided => Ok(ApplyOutcome::Skipped("undecided".to_string())),

        Decision::Push => {
            let source = worktree_root.join(rel);
            let target = main_root.join(rel); // main is the destination
            let guard = check_target(&target, main_baseline.as_ref());
            if matches!(guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let mut backups = Vec::new();
            if matches!(guard, Guard::Unchanged) {
                backups.push(backup(
                    &target,
                    &backups_root.join(backup_rel_path(ts, rel)),
                )?);
            }
            // Secret-safety BEFORE the bytes land in main.
            let gitignore_appended = ensure_gitignored_in_main(worktree_root, main_root, rel)?;
            let bytes = fs::read(&source)
                .with_context(|| format!("reading worktree file {}", source.display()))?;
            write_bytes(&target, &bytes)?;
            Ok(ApplyOutcome::Applied(ApplyResult {
                direction: WriteDirection::PushToMain,
                backups,
                gitignore_appended,
            }))
        }

        Decision::Pull => {
            let source = main_root.join(rel);
            let target = worktree_root.join(rel); // worktree is the destination
            let guard = check_target(&target, wt_baseline.as_ref());
            if matches!(guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let mut backups = Vec::new();
            if matches!(guard, Guard::Unchanged) {
                backups.push(backup(
                    &target,
                    &backups_root.join(backup_rel_path(ts, rel)),
                )?);
            }
            // No gitignore step — the worktree side is not the secret boundary.
            let bytes = fs::read(&source)
                .with_context(|| format!("reading main file {}", source.display()))?;
            write_bytes(&target, &bytes)?;
            Ok(ApplyOutcome::Applied(ApplyResult {
                direction: WriteDirection::PullFromMain,
                backups,
                gitignore_appended: false,
            }))
        }

        Decision::Merge(text) => {
            let wt_target = worktree_root.join(rel);
            let main_target = main_root.join(rel);
            // Baseline-check BOTH sides first so a skip writes nothing at all
            // (no partial backup either).
            let wt_guard = check_target(&wt_target, wt_baseline.as_ref());
            if matches!(wt_guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let main_guard = check_target(&main_target, main_baseline.as_ref());
            if matches!(main_guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            // Back up whichever side exists, under distinct dirs so the same
            // `rel` on both sides does not collide to one backup file.
            let mut backups = Vec::new();
            if matches!(wt_guard, Guard::Unchanged) {
                backups.push(backup(
                    &wt_target,
                    &backups_root.join("local").join(backup_rel_path(ts, rel)),
                )?);
            }
            if matches!(main_guard, Guard::Unchanged) {
                backups.push(backup(
                    &main_target,
                    &backups_root.join("main").join(backup_rel_path(ts, rel)),
                )?);
            }
            // Write the assembled text to the worktree, then — secret-safe —
            // to main.
            write_bytes(&wt_target, text.as_bytes())?;
            let gitignore_appended = ensure_gitignored_in_main(worktree_root, main_root, rel)?;
            write_bytes(&main_target, text.as_bytes())?;
            Ok(ApplyOutcome::Applied(ApplyResult {
                direction: WriteDirection::MergeBoth,
                backups,
                gitignore_appended,
            }))
        }
    }
}

#[cfg(test)]
mod tests;
