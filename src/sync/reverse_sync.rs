//! Sync engine: reconcile the configured files between a worktree and the main
//! checkout, safely, in both directions.
//!
//! This is the ONE path that can write untracked (often secret) files into the
//! shared main checkout, so it is deliberately conservative. The
//! "Secret-safety boundary": a push into main only appends a `.gitignore` rule
//! for a git-UNTRACKED source (see [`ensure_gitignored_in_main`], gated on
//! `Baseline::source_untracked` in [`apply_decision`]), and that determination
//! is POSITIVE and fail-closed (`!`[`git::tracked_files`]`.contains`) — an
//! unenumerable / oddly-normalized name defaults to "secret". A regression there
//! is a secret leak, not a cosmetic bug.
//!
//! ## Entry points
//!
//! - [`run`] — the interactive unified Sync cockpit (worktree menu). It offers
//!   the [`compute_reconcile_set`] set — every configured file whose worktree
//!   and main copies differ, in EITHER direction ([`DiffStatus::WorktreeOnly`] /
//!   [`DiffStatus::MainOnly`] / [`DiffStatus::Differs`]) — and applies the user's
//!   per-file push / pull / merge / delete decisions via [`apply_decision`].
//! - [`run_bulk`] — the non-interactive `ss-magic reverse-sync`: bulk-push every
//!   git-untracked candidate ([`compute_candidates`]) that differs from main.
//! - [`backup_forward_targets`] — the pre-copy backup pass for the non-interactive
//!   forward `ss-magic sync` (main → worktree), keeping that direction recoverable.
//!
//! ## Push vs pull scope (R23, KTD10)
//!
//! A PULL / create-in-worktree may target ANY configured file (tracked or not).
//! A PUSH into main is the secret-safety concern: only an untracked source gets
//! the gitignore rule; a tracked file is already committed and must NOT gain one.
//! The direct [`run_bulk`] restricts itself to untracked candidates entirely.
//! Backups live under the `.superset/backups/` of the root being overwritten,
//! gitignored there via the unified [`gitignore::ensure_path_ignored`].
//!
//! ## Structure: testable logic vs interactive TUI
//!
//! The candidate computation ([`compute_reconcile_set`] / [`compute_candidates`]),
//! the diff classification ([`classify`]), and the backup-first apply seam
//! ([`apply_decision`]) are pure / UI-free and unit-tested with `tempfile` +
//! shell `git`. The interactive merge cockpit ([`crate::tui::cockpit`]) is
//! TUI/manual-smoke (consistent with the repo's final-action convention); it
//! returns the user's per-file decisions and [`run`] feeds each through
//! [`apply_decision`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::sync::apply;
use crate::sync::merge::{backup_rel_path, BackupSide, Decision};
use crate::git;
use crate::git::gitignore::{self, Ignored, PathKind};
use crate::tui::cockpit::{self, CockpitOutcome};
use crate::tui::style;
use crate::workspace::superset_files;

/// Whether a candidate differs from main (and should be offered in the picker)
/// or is identical (and is hidden — nothing to push). R24.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
    /// The file exists on BOTH sides with DIFFERENT bytes.
    Differs,
    /// The file exists only in the worktree (absent in main) — a push CREATES
    /// it in main.
    WorktreeOnly,
    /// The file exists only in main (absent in the worktree) — a pull CREATES it
    /// locally; a delete removes main's copy. NEW in the unified sync (Task 5).
    MainOnly,
    /// Byte-identical on both sides — hidden from the cockpit. Also the
    /// walk↔classify race outcome (the path vanished on both sides), so a
    /// consumer must treat `Identical` as "nothing to reconcile", NOT as "exists
    /// on both sides and is equal".
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
        // Never re-offer a backed-up secret copy under the tool's own tree.
        .filter(|rel| !under_backups_dir(rel))
        .collect();

    out.sort();
    out.dedup();
    Ok(out)
}

/// One reconcile candidate for the unified cockpit: a path whose worktree and
/// main copies are not byte-identical, its presence classification, and whether
/// its worktree copy is git-UNTRACKED (the secret-safety push gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Repo-relative path (identical on both sides).
    pub rel: PathBuf,
    /// How this path relates to main.
    pub status: DiffStatus,
    /// The worktree copy is git-UNTRACKED (a secret): the gate that decides
    /// whether a push into main must be gitignored there. Derived FAIL-CLOSED —
    /// `true` for anything NOT positively known-tracked, so an unenumerable /
    /// oddly-normalized name defaults to "secret". Always `true` for a
    /// [`DiffStatus::MainOnly`] rel (no worktree copy is tracked), harmless
    /// since a main-only file is never pushed.
    pub wt_untracked: bool,
}

/// Compute the unified reconcile set for the interactive Sync cockpit: every
/// overlaid-pattern match on EITHER root whose worktree and main copies are not
/// byte-identical, classified, with `Identical` dropped.
///
/// Patterns are expanded against BOTH roots (a main-only file is invisible to
/// the worktree walk, and vice-versa). Directory matches are dropped (reverse
/// sync copies single files; a directory would `EISDIR` in [`classify`] / the
/// cockpit), as is the tool's own `.superset/backups/` tree (so a backed-up
/// secret is never re-offered). `wt_untracked` is derived by POSITIVE tracked
/// determination ([`git::tracked_files`]) so a path that cannot be enumerated as
/// tracked biases to "secret".
pub fn compute_reconcile_set(worktree_root: &Path, main_root: &Path) -> Result<Vec<Candidate>> {
    let cfg = match superset_files::load_overlaid(worktree_root)? {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };
    if cfg.files.is_empty() {
        return Ok(Vec::new());
    }

    let wt_matched = apply::match_paths(worktree_root, &cfg.files)?;
    let main_matched = apply::match_paths(main_root, &cfg.files)?;

    // POSITIVE tracked set, scoped to the worktree matches (an index lookup).
    // Empty pathspecs make `git ls-files` walk the WHOLE tree, so skip the probe
    // entirely when there are no worktree matches (an all-main-only repo).
    let tracked: HashSet<PathBuf> = if wt_matched.is_empty() {
        HashSet::new()
    } else {
        let specs: Vec<&str> = wt_matched.iter().filter_map(|p| p.to_str()).collect();
        git::tracked_files(worktree_root, &specs)?.into_iter().collect()
    };

    let mut rels: Vec<PathBuf> = wt_matched
        .into_iter()
        .chain(main_matched)
        .filter(|rel| is_safe_rel(rel) && !under_backups_dir(rel))
        .collect();
    rels.sort();
    rels.dedup();

    let mut out = Vec::new();
    for rel in rels {
        // Drop directory matches: reverse sync copies single files. A directory
        // present on either side would otherwise EISDIR in classify / the cockpit.
        if worktree_root.join(&rel).is_dir() || main_root.join(&rel).is_dir() {
            continue;
        }
        let status = classify(main_root, worktree_root, &rel)?;
        if status == DiffStatus::Identical {
            continue;
        }
        let wt_untracked = !tracked.contains(&rel);
        out.push(Candidate {
            rel,
            status,
            wt_untracked,
        });
    }
    Ok(out)
}

/// True when `rel` lives under the tool's own `.superset/backups/…` tree, so a
/// broad user pattern (`**/.env`) can never re-offer or pack a backed-up secret
/// copy. Shared with `pack` so an archive never captures a recovered secret.
pub(crate) fn under_backups_dir(rel: &Path) -> bool {
    use std::ffi::OsStr;
    let mut c = rel.components();
    matches!(c.next(), Some(Component::Normal(a)) if a == OsStr::new(".superset"))
        && matches!(c.next(), Some(Component::Normal(b)) if b == OsStr::new("backups"))
}

/// Classify one candidate `rel` against both trees for the unified cockpit (R24).
///
/// Four-way on presence: worktree-only → [`DiffStatus::WorktreeOnly`]; main-only
/// → [`DiffStatus::MainOnly`]; present on both and byte-equal →
/// [`DiffStatus::Identical`]; present on both and different →
/// [`DiffStatus::Differs`]. `(false, false)` — the path vanished on BOTH sides
/// between the pattern walk and this classify — is reported as `Identical`
/// (nothing to reconcile), never a phantom candidate. A read error on main's
/// copy is surfaced as `Differs` (show it rather than silently hide it); the
/// worktree read only happens when the worktree copy exists, so a main-only rel
/// never triggers a spurious read error.
// consumed by the unified cockpit run() and the direct reverse-sync run_bulk()
pub fn classify(main_root: &Path, worktree_root: &Path, rel: &Path) -> Result<DiffStatus> {
    let main_path = main_root.join(rel);
    let wt_path = worktree_root.join(rel);
    match (wt_path.exists(), main_path.exists()) {
        (true, false) => Ok(DiffStatus::WorktreeOnly),
        (false, true) => Ok(DiffStatus::MainOnly),
        (false, false) => Ok(DiffStatus::Identical),
        (true, true) => {
            let wt_bytes = fs::read(&wt_path)
                .with_context(|| format!("reading worktree file {}", wt_path.display()))?;
            match fs::read(&main_path) {
                Ok(mb) if mb == wt_bytes => Ok(DiffStatus::Identical),
                _ => Ok(DiffStatus::Differs),
            }
        }
    }
}

/// Ensure `rel` is gitignored in main, the secret-leak boundary. Returns `true`
/// when a new rule was appended, `false` when it was already ignored (no-op).
///
/// A thin wrapper over the unified [`gitignore::ensure_path_ignored`] (closest
/// `.gitignore`, covering-glob-then-anchored-literal) that adds the STRICT
/// re-verify the secret boundary requires: `ensure_path_ignored` is
/// git-TOLERANT (a git failure reads as "not ignored" and writes a literal),
/// but here a git error must FAIL the push rather than trust a tolerant append.
/// `git::is_ignored` propagates the error, and a still-unignored path bails, so
/// no un-ignored secret ever lands committable in main.
fn ensure_gitignored_in_main(worktree_root: &Path, main_root: &Path, rel: &Path) -> Result<bool> {
    match gitignore::ensure_path_ignored(main_root, worktree_root, rel, PathKind::File)? {
        Ignored::Already => Ok(false),
        Ignored::Appended => {
            if !git::is_ignored(main_root, rel)? {
                anyhow::bail!(
                    "refusing to reverse-sync {}: it is still not gitignored in main after \
                     appending an ignore rule — writing it would leave a secret committable in main",
                    rel.display()
                );
            }
            Ok(true)
        }
    }
}

/// Interactive unified Sync entry point (TUI / manual-smoke): compute the
/// bidirectional [`compute_reconcile_set`], present the merge cockpit, and apply
/// the user's per-file push / pull / merge / delete decisions with the overwrite
/// backup + gitignore secret gate.
///
/// Empty reconcile set → print a gray info line and return WITHOUT opening the
/// cockpit (R22). Decline at the cockpit → both sides fully untouched.
///
/// NOT unit-tested — the merge cockpit is interactive. The logic it orchestrates
/// is covered through [`compute_reconcile_set`], [`classify`], and
/// [`apply_decision`]. Wired into the worktree menu's single "Sync" entry.
pub fn run(worktree_root: &Path, main_root: &Path) -> Result<ExitCode> {
    // The unified reconcile set: every configured file that differs between the
    // worktree and main, in EITHER direction (worktree-only / main-only /
    // differing), with the per-file untracked flag that gates a push into main.
    let reconcile = compute_reconcile_set(worktree_root, main_root)?;
    if reconcile.is_empty() {
        println!(
            "{}",
            style::info("Everything configured is already in sync with main.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    let offered: Vec<(PathBuf, DiffStatus)> =
        reconcile.iter().map(|c| (c.rel.clone(), c.status)).collect();

    // R16: the merge cockpit is a full-screen TUI and MUST have a terminal.
    // A piped / CI / hook invocation refuses to launch and writes NOTHING,
    // pointing at the (forward-only) non-interactive path instead.
    if !cockpit::is_interactive() {
        eprintln!(
            "{}",
            style::err(
                "error: interactive sync needs a terminal — the merge cockpit cannot run \
                 piped or in CI."
            )
        );
        eprintln!(
            "{}",
            style::info(
                "Use the non-interactive `ss-magic sync` (main → worktree) or \
                 `ss-magic reverse-sync` (worktree → main) instead."
            )
        );
        // Non-zero so a piped / CI caller can tell "couldn't run, wrote nothing"
        // apart from a real success (Esc/cancel and empty-candidate stay 0).
        return Ok(ExitCode::from(2));
    }

    // Capture a review-time baseline of every offered file's metadata on BOTH
    // sides BEFORE the (possibly long) interactive review opens, plus the
    // per-file untracked flag carried straight from `compute_reconcile_set` (the
    // ONE place tracked-ness is determined — no recomputation, no drift). The
    // apply-time guard compares the metadata baseline against on-disk metadata
    // at write time, so any change during the review→apply window (edit / create
    // / delete) is detected and that file skipped rather than clobbered
    // (KD4 / R13–R15). The baseline is derived COHERENTLY with each file's
    // reviewed status (see `review_baseline`), so the confirm and apply agree.
    let mut baseline: HashMap<PathBuf, Baseline> = HashMap::new();
    for c in &reconcile {
        let (wt, main) = review_baseline(worktree_root, main_root, &c.rel, c.status)?;
        baseline.insert(
            c.rel.clone(),
            Baseline {
                wt,
                main,
                source_untracked: c.wt_untracked,
            },
        );
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

    // Backups live under a gitignored `.superset/backups/` in the worktree so
    // recovered secret bytes are never committed.
    let ts = apply_timestamp();
    let backups_root = worktree_root.join(".superset/backups");
    gitignore::ensure_path_ignored(
        worktree_root,
        worktree_root,
        Path::new(".superset/backups"),
        PathKind::Dir,
    )?;

    let ctx = ApplyContext {
        worktree_root,
        main_root,
        backups_root: &backups_root,
        ts: &ts,
        backup: true,
    };
    let summary = apply_batch(&ctx, &decisions, &baseline);
    Ok(finish_batch(summary, &backups_root, &ts, true, "Sync"))
}

/// The shared batch tail: print the recorded backup paths, best-effort-prune old
/// batches (skipped when `backup` is false — nothing was written to prune),
/// print the applied/skipped/failed summary (prefixed with `label`, since the
/// interactive [`run`] is bidirectional "Sync" while [`run_bulk`] is a one-way
/// "Reverse sync"), and choose the exit code (non-zero iff a file failed).
fn finish_batch(
    summary: BatchSummary,
    backups_root: &Path,
    ts: &str,
    backup: bool,
    label: &str,
) -> ExitCode {
    if !summary.backups.is_empty() {
        println!();
        println!(
            "{}",
            style::info("Backups of overwritten files (recover a mistake here):")
        );
        for b in &summary.backups {
            println!("{}", style::info(format!("  {}", b.display())));
        }
    }

    // Retention: keep only the newest batches so `.superset/backups/` cannot grow
    // without bound. Best-effort — a pruning failure never fails the sync (the
    // writes already landed and their backups are intact); the batch just written
    // is protected by name even under a backward clock jump. Skipped entirely
    // when backups were disabled (nothing new to prune against).
    if backup {
        match prune_old_backups(backups_root, BACKUP_BATCHES_KEPT, Some(ts)) {
            Ok(pruned) if !pruned.is_empty() => println!(
                "{}",
                style::info(format!(
                    "Pruned {} old backup batch(es), keeping the newest {BACKUP_BATCHES_KEPT}.",
                    pruned.len()
                ))
            ),
            Ok(_) => {}
            Err(err) => println!(
                "{}",
                style::warn(format!("Backup pruning failed (backups left as-is): {err:#}"))
            ),
        }
    }

    println!();
    let line = format!(
        "{label} done: applied {}, skipped {}, failed {}",
        summary.applied, summary.skipped, summary.failed
    );
    if summary.failed > 0 {
        println!("{}", style::err(line));
        // Some files did not apply — signal partial failure to scripts/CI
        // rather than exiting 0 on a batch that only partly succeeded.
        return ExitCode::from(1);
    } else if summary.skipped > 0 {
        println!("{}", style::warn(line));
    } else {
        println!("{}", style::ok(line));
    }
    ExitCode::SUCCESS
}

/// Tallies + collected backups from applying a whole batch of cockpit
/// decisions. `failed` counts files whose apply raised an I/O error.
struct BatchSummary {
    applied: usize,
    skipped: usize,
    failed: usize,
    backups: Vec<PathBuf>,
}

/// Apply every `(rel, decision)` in order, threading each file's review-time
/// baseline from `baseline`, and print one line per file as it goes.
///
/// Each file's [`apply_decision`] result is handled independently: an `Err` is
/// reported and counted in `failed` but does NOT abort the batch (a single I/O
/// error must neither roll back nor hide the files already written + backed up
/// before it). Skips and applies are tallied and reported as before.
fn apply_batch(
    ctx: &ApplyContext,
    decisions: &[(PathBuf, Decision)],
    baseline: &HashMap<PathBuf, Baseline>,
) -> BatchSummary {
    let mut summary = BatchSummary {
        applied: 0,
        skipped: 0,
        failed: 0,
        backups: Vec::new(),
    };
    for (rel, decision) in decisions {
        // Fail-closed on a miss: an absent baseline defaults to
        // `source_untracked: true` (treat as a secret) so a bookkeeping gap
        // never silently skips the gitignore-in-main gate.
        let bl = baseline.get(rel).cloned().unwrap_or(Baseline {
            wt: None,
            main: None,
            source_untracked: true,
        });
        match apply_decision(ctx, rel, decision, bl) {
            Ok(ApplyOutcome::Applied(result)) => {
                summary.applied += 1;
                let dir = match result.direction {
                    WriteDirection::PushToMain => "worktree → main",
                    WriteDirection::PullFromMain => "main → worktree",
                    WriteDirection::MergeBoth => "merged → both",
                    WriteDirection::DeleteBoth => "deleted from both sides",
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
                summary.backups.extend(result.backups);
            }
            Ok(ApplyOutcome::Skipped(reason)) => {
                summary.skipped += 1;
                println!(
                    "{}",
                    style::warn(format!("Skipped {}: {reason}", rel.display()))
                );
            }
            Err(err) => {
                summary.failed += 1;
                eprintln!(
                    "{}",
                    style::err(format!("Failed {}: {err:#}", rel.display()))
                );
            }
        }
    }
    summary
}

/// Direct, non-interactive `ss-magic reverse-sync`: bulk-push every git-untracked
/// candidate that differs from main INTO main (creating or overwriting), backing
/// up each overwritten main file first (unless `no_backup`) and running the
/// gitignore-in-main secret gate. No TUI. Every candidate is untracked by
/// construction ([`compute_candidates`]), so the secret gate always fires and
/// `source_untracked` is hard-coded `true`.
pub fn run_bulk(worktree_root: &Path, main_root: &Path, no_backup: bool) -> Result<ExitCode> {
    let candidates = compute_candidates(worktree_root)?;
    let mut work: Vec<(PathBuf, DiffStatus)> = Vec::new();
    for rel in &candidates {
        let status = classify(main_root, worktree_root, rel)?;
        // Untracked worktree candidates are never MainOnly (they exist in the
        // worktree); Identical is nothing to push.
        if status != DiffStatus::Identical {
            work.push((rel.clone(), status));
        }
    }
    if work.is_empty() {
        println!(
            "{}",
            style::info("Nothing to reverse-sync — no untracked candidates differ from main.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Backups of main's overwritten bytes live under MAIN's gitignored
    // `.superset/backups/` — the side this direction overwrites.
    let ts = apply_timestamp();
    let backups_root = main_root.join(".superset/backups");
    if !no_backup {
        gitignore::ensure_path_ignored(
            main_root,
            main_root,
            Path::new(".superset/backups"),
            PathKind::Dir,
        )?;
    }

    let ctx = ApplyContext {
        worktree_root,
        main_root,
        backups_root: &backups_root,
        ts: &ts,
        backup: !no_backup,
    };

    // No interactive review window, so the baseline is the current on-disk
    // metadata captured immediately before apply.
    let mut baseline: HashMap<PathBuf, Baseline> = HashMap::new();
    let mut decisions: Vec<(PathBuf, Decision)> = Vec::new();
    for (rel, status) in &work {
        let (wt, main) = review_baseline(worktree_root, main_root, rel, *status)?;
        baseline.insert(
            rel.clone(),
            Baseline {
                wt,
                main,
                source_untracked: true,
            },
        );
        decisions.push((rel.clone(), Decision::Push));
    }

    let summary = apply_batch(&ctx, &decisions, &baseline);
    Ok(finish_batch(summary, &backups_root, &ts, !no_backup, "Reverse sync"))
}

/// Back up a file OR directory `target` to `dest` before it is overwritten,
/// returning the backup path (`Ok(None)` when `target` is absent — a fresh
/// create has nothing to lose). A directory is copied recursively.
fn backup_if_exists(target: &Path, dest: &Path) -> Result<Option<PathBuf>> {
    if !target.exists() {
        return Ok(None);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating backup dir {}", parent.display()))?;
    }
    if target.is_dir() {
        apply::copy_dir_recursive(target, dest)?;
    } else {
        fs::copy(target, dest)
            .with_context(|| format!("backing up {} → {}", target.display(), dest.display()))?;
    }
    Ok(Some(dest.to_path_buf()))
}

/// Back up every existing worktree file/dir that a forward `ss-magic sync`
/// (main → worktree) is about to overwrite, under `cwd_root/.superset/backups/
/// <ts>/…` (the worktree side forward sync overwrites), gitignored there. A
/// no-op when the configured patterns match nothing. The copy engine itself
/// stays untouched — this is a deliberate pre-pass with a narrow, documented
/// same-process TOCTOU window (nothing else writes the worktree between this
/// pass and the copy). Best-effort pruning keeps the backups dir bounded.
pub fn backup_forward_targets(main_root: &Path, cwd_root: &Path, patterns: &[String]) -> Result<()> {
    let matches = apply::match_paths(main_root, patterns)
        .context("expanding sync patterns for the pre-copy backup pass")?;
    if matches.is_empty() {
        return Ok(());
    }
    gitignore::ensure_path_ignored(
        cwd_root,
        cwd_root,
        Path::new(".superset/backups"),
        PathKind::Dir,
    )?;
    let ts = apply_timestamp();
    let backups_root = cwd_root.join(".superset/backups");
    let mut backed_up = Vec::new();
    for rel in &matches {
        if under_backups_dir(rel) {
            continue;
        }
        let dest = backups_root.join(&ts).join(rel);
        if let Some(p) = backup_if_exists(&cwd_root.join(rel), &dest)? {
            backed_up.push(p);
        }
    }
    if !backed_up.is_empty() {
        println!();
        println!(
            "{}",
            style::info("Backups of files about to be overwritten (recover a mistake here):")
        );
        for p in &backed_up {
            println!("{}", style::info(format!("  {}", p.display())));
        }
    }
    // Best-effort prune; a failure warns but never fails the forward sync.
    if let Err(err) = prune_old_backups(&backups_root, BACKUP_BATCHES_KEPT, Some(&ts)) {
        println!(
            "{}",
            style::warn(format!("Backup pruning failed (backups left as-is): {err:#}"))
        );
    }
    Ok(())
}

/// Timestamp string for a batch of backups: the current UTC time as a
/// human-readable `YYYYmmdd-HHMMSS` directory name. The per-side namespaces
/// keep the worktree and main copies of the SAME file collision-free within
/// one batch; two batches inside one second would share the directory, and a
/// same-side collision would overwrite the earlier backup — practically
/// unreachable, since every batch requires completing a full interactive
/// cockpit review.
fn apply_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_timestamp(secs)
}

/// Format `secs` since the Unix epoch as a UTC `YYYYmmdd-HHMMSS` string —
/// pure and dependency-free (Howard Hinnant's civil-from-days algorithm), so
/// backup batch directories get human-readable names without pulling in a
/// date crate.
fn format_timestamp(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);

    // Civil-from-days: days since 1970-01-01 → (year, month, day).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64; // day-of-era [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year [0, 365]
    let mp = (5 * doy + 2) / 153; // month index in the Mar-first calendar [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = yoe as i64 + era * 400 + i64::from(m <= 2);

    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// How many of the newest backup batch directories are KEPT under
/// `.superset/backups/` — older batches are pruned after each apply so the
/// backups dir cannot grow without bound. Each batch is a handful of small
/// secret files, so ten is cheap insurance.
const BACKUP_BATCHES_KEPT: usize = 10;

/// True when `name` looks like a backup batch directory THIS TOOL created:
/// the current `YYYYmmdd-HHMMSS` shape or the legacy all-digits epoch shape.
/// Anything else under the backups root is never pruned — retention must not
/// delete a directory it did not name.
fn is_backup_batch_name(name: &str) -> bool {
    fn all_digits(s: &[u8]) -> bool {
        !s.is_empty() && s.iter().all(u8::is_ascii_digit)
    }
    let b = name.as_bytes();
    all_digits(b) || (b.len() == 15 && b[8] == b'-' && all_digits(&b[..8]) && all_digits(&b[9..]))
}

/// Prune old backup batches under `backups_root`, keeping the newest `keep`.
///
/// A batch is keyed by its timestamp name, and lexicographic name order is
/// chronological across both name shapes: within each shape the digits are
/// fixed-width, and every legacy epoch name (`17…`) sorts before every
/// `YYYYmmdd-HHMMSS` name (`20…`), matching their real ages. One batch can
/// own several directories: the modern `<ts>/` layout, plus the legacy
/// (unreleased-0.4.0) merge layout that wrote `local/<epoch>/` and
/// `main/<epoch>/` at the TOP level — those children are folded into their
/// epoch's batch so pre-upgrade backups honor the same keep budget instead
/// of surviving forever. Only names matching [`is_backup_batch_name`] are
/// ever touched (a foreign entry — or a non-batch child of `local`/`main` —
/// is never deleted); the legacy side dirs themselves are removed once
/// emptied. A missing backups root prunes nothing. Returns the pruned
/// directory paths.
///
/// `protect` names the batch WRITTEN BY THIS RUN: it is never pruned, no
/// matter where it sorts — a backward clock jump could otherwise name the
/// fresh batch "older" than ten existing ones and delete the very backups
/// whose recovery paths were just printed. (Normally the fresh batch is the
/// newest name and the protection is a no-op; when it does fire, one extra
/// old batch survives until the next healthy run.)
fn prune_old_backups(
    backups_root: &Path,
    keep: usize,
    protect: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(backups_root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("listing backup batches in {}", backups_root.display()))
        }
    };

    // Batch name → every directory belonging to that batch. BTreeMap keeps
    // the names sorted ascending, i.e. oldest first.
    let mut batches: std::collections::BTreeMap<String, Vec<PathBuf>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        let entry = entry
            .with_context(|| format!("listing backup batches in {}", backups_root.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        if is_backup_batch_name(name) {
            batches.entry(name.to_string()).or_default().push(entry.path());
        } else if name == "local" || name == "main" {
            // Legacy merge layout: fold `<side>/<epoch>/` into its batch.
            for child in fs::read_dir(entry.path())
                .with_context(|| format!("listing legacy backups in {}", entry.path().display()))?
            {
                let child = child.with_context(|| {
                    format!("listing legacy backups in {}", entry.path().display())
                })?;
                let cname = child.file_name();
                let Some(cname) = cname.to_str() else { continue };
                let child_is_dir = child.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if child_is_dir && is_backup_batch_name(cname) {
                    batches.entry(cname.to_string()).or_default().push(child.path());
                }
            }
        }
    }

    let prune_count = batches.len().saturating_sub(keep);
    let mut pruned = Vec::new();
    for (name, dirs) in batches.into_iter().take(prune_count) {
        // Never the batch this run just wrote (see the doc comment).
        if protect == Some(name.as_str()) {
            continue;
        }
        for dir in dirs {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("pruning old backup batch {}", dir.display()))?;
            pruned.push(dir);
        }
    }

    // Drop a legacy side dir once it holds nothing — but only when this run
    // actually pruned from it (never remove a foreign dir that merely shares
    // the name). Best-effort; a non-empty dir is left alone.
    for side in ["local", "main"] {
        let dir = backups_root.join(side);
        let pruned_from_side = pruned.iter().any(|p| p.parent() == Some(dir.as_path()));
        if pruned_from_side
            && fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(false)
        {
            let _ = fs::remove_dir(&dir);
        }
    }
    Ok(pruned)
}

// ── Apply seam (reverse-sync merge cockpit) ──────────────────────────────
//
// The cockpit's safe, backup-first apply path. It writes a decision (push /
// pull / merge) to disk: a path-safety guard, a review-time-baseline re-check
// that skips a file when EITHER the side it reads or the side it overwrites
// changed since the user reviewed it, a timestamped pre-write backup of the
// losing bytes, and `ensure_gitignored_in_main` BEFORE any secret bytes land in
// main. Driven by [`run`] (via [`apply_batch`]) with the decisions returned
// from the cockpit — plus the per-file `(worktree, main)` metadata baseline
// captured before the cockpit opened — including the `Decision::Merge` produced
// by the cockpit's interactive-merge overlay, whose assembled bytes this seam
// writes to BOTH sides.

/// Lightweight metadata snapshot backing the review-time TOCTOU baseline: a
/// byte length plus a best-effort mtime (some filesystems omit mtime, hence the
/// `Option`), plus a content hash captured ONLY when the mtime is unavailable —
/// without it, a same-length edit during the review window would pass the
/// guard on length alone.
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// The file's length in bytes.
    pub len: u64,
    /// The file's modification time, when the platform / filesystem reports one.
    pub mtime: Option<SystemTime>,
    /// A content fingerprint, present only when `mtime` is unavailable (the
    /// fallback change signal for filesystems that report no mtime).
    pub content_hash: Option<u64>,
}

/// Snapshot `path`'s metadata for the TOCTOU baseline.
///
/// Returns `Ok(None)` ONLY when the path does not exist (`ErrorKind::NotFound`);
/// `Ok(Some(..))` when it exists; and propagates any OTHER io error (permissions,
/// I/O) via `?`. A non-`NotFound` stat error must NEVER be silently read as
/// "missing" — doing so would skip the mandatory pre-overwrite backup for a
/// target that actually exists. When the filesystem reports no mtime, the
/// content is hashed instead (these are small secret files — the read is
/// cheap) so the guard never has to trust a bare length.
pub fn meta_of(path: &Path) -> Result<Option<FileMeta>> {
    match fs::metadata(path) {
        Ok(m) => {
            let mtime = m.modified().ok();
            let content_hash = if mtime.is_none() {
                Some(hash_file(path)?)
            } else {
                None
            };
            Ok(Some(FileMeta {
                len: m.len(),
                mtime,
                content_hash,
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading metadata of {}", path.display())),
    }
}

/// Content fingerprint for the mtime-less TOCTOU fallback. Non-cryptographic —
/// the threat model is a concurrent edit, not an adversary.
fn hash_file(path: &Path) -> Result<u64> {
    use std::hash::{Hash, Hasher};
    let bytes = fs::read(path)
        .with_context(|| format!("reading {} for the review baseline", path.display()))?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    Ok(h.finish())
}

/// Capture one offered candidate's review-time `(worktree, main)` baseline,
/// COHERENT with the `status` the user reviews — not with whatever the disk
/// happens to hold at capture time.
///
/// The reviewed-ABSENT side is pinned to `None`, symmetric on both directions:
/// a [`DiffStatus::WorktreeOnly`] file was reviewed with main absent (main side
/// `None`), and a [`DiffStatus::MainOnly`] file was reviewed with the worktree
/// copy absent (worktree side `None`). So if the pinned side gains a copy in the
/// classify→capture window, the apply-time guard sees a file the review never
/// covered (`baseline None` vs a present file → [`Guard::Changed`]) and SKIPS
/// it — instead of silently overwriting or deleting a copy the user was told did
/// not exist.
fn review_baseline(
    worktree_root: &Path,
    main_root: &Path,
    rel: &Path,
    status: DiffStatus,
) -> Result<(Option<FileMeta>, Option<FileMeta>)> {
    let wt = match status {
        DiffStatus::MainOnly => None,
        _ => meta_of(&worktree_root.join(rel))?,
    };
    let main = match status {
        DiffStatus::WorktreeOnly => None,
        _ => meta_of(&main_root.join(rel))?,
    };
    Ok((wt, main))
}

/// Whether two snapshots of the same path can be trusted as "unchanged".
/// Lengths must match; beyond that, mtimes present on BOTH sides decide;
/// without an mtime the content hashes decide; and with neither signal the
/// answer is a fail-safe `false` — a bare length must never pass a
/// same-length edit as unchanged.
fn metas_match(b: &FileMeta, c: &FileMeta) -> bool {
    if b.len != c.len {
        return false;
    }
    match (b.mtime, c.mtime) {
        (Some(bm), Some(cm)) => bm == cm,
        _ => match (b.content_hash, c.content_hash) {
            (Some(bh), Some(ch)) => bh == ch,
            _ => false,
        },
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
    /// The file was deleted from BOTH sides (whichever existed).
    DeleteBoth,
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
#[derive(Clone, Copy)]
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
///   [`metas_match`] (length + mtime, with a content-hash fallback when the
///   filesystem reports no mtime), else [`Guard::Changed`]
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
            if metas_match(b, &c) {
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

/// Write `bytes` to `target` ATOMICALLY, creating any missing parent
/// directories first.
///
/// The bytes are staged in a temp file in the SAME directory as `target`, then
/// `persist`ed (renamed) over it, so an interrupted or failing write can never
/// leave a truncated secret at `target` — the rename either fully replaces the
/// file or leaves the old bytes intact. An existing target's permissions are
/// preserved across the replace (the temp file is created 0600 by default) so a
/// reverse sync never silently changes a file's mode.
fn write_bytes(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("creating parent dirs for {}", target.display()))?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temp file in {}", parent.display()))?;
    tmp.write_all(bytes)
        .with_context(|| format!("writing {}", target.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(target) {
            let _ = tmp
                .as_file()
                .set_permissions(fs::Permissions::from_mode(meta.permissions().mode()));
        }
    }

    // Flush to disk before the rename, then persist atomically over the target.
    tmp.as_file().sync_all().ok();
    tmp.persist(target)
        .with_context(|| format!("persisting {}", target.display()))?;
    Ok(())
}

/// Back up `target` to `dest` iff backups are enabled AND `guard` is
/// [`Guard::Unchanged`] — the "back up the losing bytes before a safe overwrite"
/// rule shared by every destructive-overwrite site in [`apply_decision`]. A
/// no-op (`Ok(None)`) for [`Guard::Missing`] (no prior bytes to lose), or when
/// `take_backup` is false (`--no-backup`). The caller has already bailed out on
/// [`Guard::Changed`] before reaching here, so disabling backups never disables
/// the concurrent-edit guard.
fn backup_if_unchanged(
    target: &Path,
    guard: Guard,
    dest: &Path,
    take_backup: bool,
) -> Result<Option<PathBuf>> {
    if take_backup && matches!(guard, Guard::Unchanged) {
        Ok(Some(backup(target, dest)?))
    } else {
        Ok(None)
    }
}

/// The per-batch context threaded through every [`apply_decision`] call: the
/// two tree roots and the shared backup destination for the batch.
pub struct ApplyContext<'a> {
    /// The worktree root (reverse-sync source for `Push`, destination for `Pull`).
    pub worktree_root: &'a Path,
    /// The main checkout root (reverse-sync destination for `Push`, source for
    /// `Pull`, and the secret-safety boundary).
    pub main_root: &'a Path,
    /// Root directory backups are written under (gitignored via
    /// [`gitignore::ensure_path_ignored`] at the closest `.gitignore`).
    pub backups_root: &'a Path,
    /// The batch's single timestamp, shared by every backup path in the run.
    pub ts: &'a str,
    /// Whether to take a pre-overwrite backup of the losing bytes. `false`
    /// (`--no-backup` on a direct path) skips ONLY the backup copy — the TOCTOU
    /// [`Guard::Changed`] skip and the secret-safety gitignore step are
    /// unaffected.
    pub backup: bool,
}

/// One file's review-time metadata baseline for [`apply_decision`]'s TOCTOU
/// guard, one side each — see [`check_target`].
#[derive(Debug, Clone)]
pub struct Baseline {
    /// The worktree side's metadata at review time (`None` if it didn't exist).
    pub wt: Option<FileMeta>,
    /// The main side's metadata at review time (`None` if it didn't exist).
    pub main: Option<FileMeta>,
    /// Whether the PUSH SOURCE (the worktree copy) is git-untracked — the
    /// secret-safety gate. `true` fires [`ensure_gitignored_in_main`] before a
    /// push/merge writes into main; `false` (a positively-tracked file) skips it
    /// so a committed path never gets a `.gitignore` rule. Fail-closed: when
    /// tracked-ness cannot be determined, this is `true` (treat as a secret).
    pub source_untracked: bool,
}

/// Apply one cockpit [`Decision`] for `rel`, safely (KD4, R13–R15).
///
/// Safety order: an unsafe `rel` bails; an [`Decision::Undecided`] is a no-op
/// skip; BOTH the side we OVERWRITE and the side we READ are guarded against
/// their review-time baselines (`baseline.wt` for the worktree side,
/// `baseline.main` for the main side — each captured by [`meta_of`] before the
/// cockpit opened): `Push` guards worktree(source)+main(target), `Pull` guards
/// main(source)+worktree(target), `Merge` guards both. A source that changed
/// since review is stale content the user never saw, so it is skipped just like
/// a changed target. The overwritten target's losing bytes are backed up under
/// `ctx.backups_root` (via [`backup_rel_path`]) BEFORE the write; every write
/// into main is preceded by [`ensure_gitignored_in_main`] so a git failure
/// never leaves un-ignored secret bytes on disk. `Merge` writes the assembled
/// text to BOTH sides (distinct per-side backup namespaces so neither
/// original is lost). `Delete` removes the file from BOTH sides (whichever
/// exist), backing each existing side up first — no gitignore step, since no
/// secret bytes land in main. Any side that no longer matches its baseline
/// (edited, created, or deleted since review) yields
/// [`ApplyOutcome::Skipped`] with nothing written — no overwrite and no
/// partial backup.
pub fn apply_decision(
    ctx: &ApplyContext,
    rel: &Path,
    decision: &Decision,
    baseline: Baseline,
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
            let source = ctx.worktree_root.join(rel);
            let target = ctx.main_root.join(rel); // main is the destination
            // Defense-in-depth: Push reads the worktree copy, so a file with no
            // worktree side at review (a MainOnly candidate) is out of contract —
            // skip rather than read an absent source. In-contract callers never
            // reach here (the cockpit's `set_push` gates it and `run_bulk` only
            // pushes worktree-existing candidates), but the guard keeps this
            // "one safety path" self-enforcing rather than trusting the caller.
            if baseline.wt.is_none() {
                return Ok(ApplyOutcome::Skipped(
                    "push has no worktree source".to_string(),
                ));
            }
            // Guard BOTH sides against their review-time baselines: the target
            // we OVERWRITE (main) and the source we READ (worktree). A source
            // that changed since review means we'd push bytes the user never
            // saw, so skip rather than push stale content.
            if matches!(check_target(&source, baseline.wt.as_ref()), Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let guard = check_target(&target, baseline.main.as_ref());
            if matches!(guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let mut backups = Vec::new();
            backups.extend(backup_if_unchanged(
                &target,
                guard,
                &ctx.backups_root
                    .join(backup_rel_path(ctx.ts, BackupSide::Main, rel)),
                ctx.backup,
            )?);
            // Secret-safety BEFORE the bytes land in main — but ONLY for an
            // untracked (secret) source. A TRACKED file is already committed in
            // main and must NOT gain a `.gitignore` rule; `source_untracked` is
            // derived fail-closed, so an undeterminable source still gates on.
            let gitignore_appended = if baseline.source_untracked {
                ensure_gitignored_in_main(ctx.worktree_root, ctx.main_root, rel)?
            } else {
                false
            };
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
            let source = ctx.main_root.join(rel);
            let target = ctx.worktree_root.join(rel); // worktree is the destination
            // Guard BOTH sides: the target we OVERWRITE (worktree) and the
            // source we READ (main). A source changed since review is stale, so
            // skip rather than pull bytes the user never reviewed.
            if matches!(check_target(&source, baseline.main.as_ref()), Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let guard = check_target(&target, baseline.wt.as_ref());
            if matches!(guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let mut backups = Vec::new();
            backups.extend(backup_if_unchanged(
                &target,
                guard,
                &ctx.backups_root
                    .join(backup_rel_path(ctx.ts, BackupSide::Worktree, rel)),
                ctx.backup,
            )?);
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
            let wt_target = ctx.worktree_root.join(rel);
            let main_target = ctx.main_root.join(rel);
            // Defense-in-depth: a merge reconciles two EXISTING sides, so a file
            // that was one-sided at review (WorktreeOnly / MainOnly) is out of
            // contract — skip rather than write the assembled text onto a side
            // the review pinned absent. The cockpit only offers merge for a
            // two-sided Differs file, so this never fires in practice.
            if baseline.wt.is_none() || baseline.main.is_none() {
                return Ok(ApplyOutcome::Skipped("merge needs both sides".to_string()));
            }
            // Baseline-check BOTH sides first so a skip writes nothing at all
            // (no partial backup either).
            let wt_guard = check_target(&wt_target, baseline.wt.as_ref());
            if matches!(wt_guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let main_guard = check_target(&main_target, baseline.main.as_ref());
            if matches!(main_guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            // Back up whichever side exists — the per-side namespaces keep the
            // same `rel` from colliding into one backup file.
            let mut backups = Vec::new();
            backups.extend(backup_if_unchanged(
                &wt_target,
                wt_guard,
                &ctx.backups_root
                    .join(backup_rel_path(ctx.ts, BackupSide::Worktree, rel)),
                ctx.backup,
            )?);
            backups.extend(backup_if_unchanged(
                &main_target,
                main_guard,
                &ctx.backups_root
                    .join(backup_rel_path(ctx.ts, BackupSide::Main, rel)),
                ctx.backup,
            )?);
            // Secret-safety FIRST, then write MAIN, then the worktree. Ordering
            // the main write before the worktree means a failure at or before
            // it leaves BOTH sides untouched (no divergence); only the final
            // worktree write can fail after main is updated, and `write_bytes`
            // is atomic so even that leaves no truncated file. The
            // gitignore-before-any-main-write ordering is preserved. Gated on an
            // untracked source (see the Push arm) so a tracked merge target never
            // gains a `.gitignore` rule.
            let gitignore_appended = if baseline.source_untracked {
                ensure_gitignored_in_main(ctx.worktree_root, ctx.main_root, rel)?
            } else {
                false
            };
            write_bytes(&main_target, text.as_bytes())?;
            write_bytes(&wt_target, text.as_bytes())?;
            Ok(ApplyOutcome::Applied(ApplyResult {
                direction: WriteDirection::MergeBoth,
                backups,
                gitignore_appended,
            }))
        }

        Decision::Delete => {
            let wt_target = ctx.worktree_root.join(rel);
            let main_target = ctx.main_root.join(rel);
            // Baseline-check BOTH sides first so a skip removes nothing at all
            // (no partial backup either).
            let wt_guard = check_target(&wt_target, baseline.wt.as_ref());
            if matches!(wt_guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            let main_guard = check_target(&main_target, baseline.main.as_ref());
            if matches!(main_guard, Guard::Changed) {
                return Ok(ApplyOutcome::Skipped(changed_reason()));
            }
            // Neither side exists (and neither did at review) — defensive; an
            // offered candidate always exists in the worktree at review time.
            if matches!(wt_guard, Guard::Missing) && matches!(main_guard, Guard::Missing) {
                return Ok(ApplyOutcome::Skipped("nothing to delete".to_string()));
            }
            // Back up every existing side BEFORE its unlink — a deleted
            // untracked secret has no git undo, so the backup is the only
            // recovery path.
            let mut backups = Vec::new();
            backups.extend(backup_if_unchanged(
                &wt_target,
                wt_guard,
                &ctx.backups_root
                    .join(backup_rel_path(ctx.ts, BackupSide::Worktree, rel)),
                ctx.backup,
            )?);
            backups.extend(backup_if_unchanged(
                &main_target,
                main_guard,
                &ctx.backups_root
                    .join(backup_rel_path(ctx.ts, BackupSide::Main, rel)),
                ctx.backup,
            )?);
            // Remove main first, then the worktree (mirrors Merge's ordering):
            // a failure at or before the main unlink leaves the worktree copy
            // intact, so the file is still offered on the next run rather than
            // half-vanishing from the side the user works in.
            if matches!(main_guard, Guard::Unchanged) {
                fs::remove_file(&main_target)
                    .with_context(|| format!("deleting main file {}", main_target.display()))?;
            }
            if matches!(wt_guard, Guard::Unchanged) {
                fs::remove_file(&wt_target)
                    .with_context(|| format!("deleting worktree file {}", wt_target.display()))?;
            }
            Ok(ApplyOutcome::Applied(ApplyResult {
                direction: WriteDirection::DeleteBoth,
                backups,
                gitignore_appended: false,
            }))
        }
    }
}

#[cfg(test)]
mod tests;
