//! Thin wrappers around `inquire` prompts. All styling comes from the
//! global `RenderConfig` installed by `style::init()`; the wrappers exist
//! to keep the prompt strings in one place and to coerce the results into
//! the shapes the rest of the binary expects.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use inquire::validator::Validation;
use inquire::{Confirm, CustomUserError, Select, Text};

use crate::git;
use crate::sync::pattern;
use crate::sync::repo_scan;
use crate::sync::reverse_sync::DiffStatus;
use crate::tui::style;

/// One of the three finishing actions offered at the end of bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalAction {
    CommitPushMain,
    FeatureBranchPR,
    Done,
}

impl fmt::Display for FinalAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            FinalAction::CommitPushMain => "Commit and push to main branch",
            FinalAction::FeatureBranchPR => "Create feature branch, commit and open a PR",
            FinalAction::Done => "Done for now",
        };
        f.write_str(label)
    }
}

/// One row in an action-loop picker. Every row is one action: tap to
/// toggle the checkbox (re-renders, keeps the cursor on the row), or tap
/// one of the trailing sentinels (`+ Add new …` / `✔ Done`).
///
/// `dim_suffix` is rendered after the row label in [`style::warn`] when
/// set. Pickers use this to flag rows that didn't trip their detection
/// signal (`(no matches)` for the patterns picker, `(not detected)` for
/// the setup-commands picker).
#[derive(Debug, Clone)]
struct Row {
    raw: String,
    checked: bool,
    dim_suffix: Option<&'static str>,
}

/// One row in the bootstrap action prompt. Every row is an action: tap
/// a pattern row to toggle it; tap "+ Add new …" to enter one via text;
/// tap "Done" to commit.
#[derive(Clone)]
enum Action {
    Toggle { idx: usize, label: String },
    AddNew { label: &'static str },
    Done,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Toggle { label, .. } => f.write_str(label),
            Action::AddNew { label } => f.write_str(label),
            Action::Done => f.write_str("✔ Done"),
        }
    }
}

fn render_row(row: &Row) -> String {
    let mark = if row.checked { "[x]" } else { "[ ]" };
    match row.dim_suffix {
        Some(suffix) => format!("{} {}  {}", mark, row.raw, style::warn(suffix)),
        None => format!("{} {}", mark, row.raw),
    }
}

/// Static text for one action-loop picker. Both in-tree pickers
/// construct one of these as a const.
struct PickerStrings {
    prompt: &'static str,
    help: &'static str,
    add_row_label: &'static str,
    add_prompt_label: &'static str,
    add_prompt_help: &'static str,
    cancel_context: &'static str,
    add_cancel_context: &'static str,
}

/// Shared action-loop driver for the bootstrap pickers. Every row is one
/// action: tap a row to toggle, tap `+ Add new …` to open a text
/// sub-prompt, tap `✔ Done` to commit.
///
/// Callers supply the validator (`(trimmed, taken) -> Result<(), String>`)
/// and a closure that decides whether a newly added row carries a dim
/// suffix and what label it uses. Function pointers satisfy the
/// `Clone + 'static` bound on the validator for free, which is what both
/// in-tree call sites use.
///
/// The cursor lands on the first unchecked row at open; if every row is
/// already checked, the cursor falls past the rows onto `✔ Done` so the
/// user can commit immediately.
fn pick_with_actions<V, D>(
    strings: &PickerStrings,
    mut rows: Vec<Row>,
    validator: V,
    dim_for_new_row: D,
) -> Result<Vec<String>>
where
    V: Fn(&str, &[String]) -> std::result::Result<(), String> + Clone + Send + Sync + 'static,
    D: Fn(&str) -> Result<Option<&'static str>>,
{
    // Cursor starts on the first unchecked row; if everything is already
    // checked, fall through to "Done" (one index past the AddNew sentinel).
    let mut cursor: usize = rows
        .iter()
        .position(|r| !r.checked)
        .unwrap_or(rows.len() + 1);

    loop {
        let mut actions: Vec<Action> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| Action::Toggle {
                idx: i,
                label: render_row(r),
            })
            .collect();
        actions.push(Action::AddNew {
            label: strings.add_row_label,
        });
        actions.push(Action::Done);

        let action = Select::new(strings.prompt, actions)
            .with_starting_cursor(cursor.min(rows.len() + 1))
            .with_help_message(strings.help)
            .prompt()
            .context(strings.cancel_context)?;

        match action {
            Action::Toggle { idx, .. } => {
                rows[idx].checked = !rows[idx].checked;
                cursor = idx;
            }
            Action::AddNew { .. } => {
                let taken: Vec<String> = rows.iter().map(|r| r.raw.clone()).collect();
                if let Some(new_value) = prompt_one_custom_entry(
                    strings.add_prompt_label,
                    strings.add_prompt_help,
                    strings.add_cancel_context,
                    &taken,
                    validator.clone(),
                )? {
                    let dim_suffix = dim_for_new_row(&new_value)?;
                    rows.push(Row {
                        raw: new_value,
                        checked: true,
                        dim_suffix,
                    });
                    cursor = rows.len() - 1;
                } else {
                    cursor = rows.len(); // back to AddNew row on Esc
                }
            }
            Action::Done => {
                return Ok(rows
                    .into_iter()
                    .filter(|r| r.checked)
                    .map(|r| r.raw)
                    .collect());
            }
        }
    }
}

/// Prompt for one new custom entry. Returns `Ok(None)` when the user
/// hits Esc; `Ok(Some(value))` on confirmed valid input. Empty input is
/// rejected by the validator — Esc is the cancel path.
fn prompt_one_custom_entry<V>(
    prompt_label: &str,
    prompt_help: &str,
    cancel_context: &'static str,
    taken: &[String],
    validator: V,
) -> Result<Option<String>>
where
    V: Fn(&str, &[String]) -> std::result::Result<(), String> + Clone + Send + Sync + 'static,
{
    let taken_clone = taken.to_vec();
    let inquire_validator = move |s: &str| -> std::result::Result<Validation, CustomUserError> {
        match validator(s.trim(), &taken_clone) {
            Ok(()) => Ok(Validation::Valid),
            Err(msg) => Ok(Validation::Invalid(msg.into())),
        }
    };
    let answer = Text::new(prompt_label)
        .with_help_message(prompt_help)
        .with_validator(inquire_validator)
        .prompt_skippable()
        .context(cancel_context)?;
    Ok(answer.map(|s| s.trim().to_string()))
}

/// Pick the patterns the user wants written to `magic.json`.
///
/// `options` carries the four preconfigured patterns followed by any
/// existing custom patterns from `magic.json` (use
/// [`super::superset_files::existing_unknown_entries`] to compute the
/// tail). `preselected` is the set of indices that should start checked.
/// `repo_root` is needed to compute filesystem-match status for the
/// no-match warning suffix on each row, including patterns the user adds
/// inside the loop.
pub fn pick_patterns(
    options: &[String],
    preselected: &[usize],
    repo_root: &Path,
) -> Result<Vec<String>> {
    let pattern_strs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let initial_match = repo_scan::matches_for_patterns(repo_root, &pattern_strs)?;

    let rows: Vec<Row> = options
        .iter()
        .enumerate()
        .map(|(i, raw)| Row {
            raw: raw.clone(),
            checked: preselected.contains(&i),
            dim_suffix: if initial_match[i] {
                None
            } else {
                Some("(no matches)")
            },
        })
        .collect();

    let repo_root = repo_root.to_path_buf();
    const STRINGS: PickerStrings = PickerStrings {
        prompt: "Files to copy automatically:",
        help: "↑↓ navigate, enter to toggle / add / confirm",
        add_row_label: "+ Add new pattern…",
        add_prompt_label: "New pattern (Esc to cancel):",
        add_prompt_help: "e.g. `apps/*/.env` — standard glob syntax",
        cancel_context: "pattern selection cancelled",
        add_cancel_context: "custom pattern prompt cancelled",
    };
    pick_with_actions(&STRINGS, rows, validate_pattern, move |s| {
        let matched = repo_scan::pattern_matches_any(&repo_root, s)?;
        Ok(if matched { None } else { Some("(no matches)") })
    })
}

/// Validate a single user-entered pattern. Wraps `pattern::check_syntax`
/// and layers on the duplicate-of-already-taken check.
fn validate_pattern(pattern_str: &str, taken: &[String]) -> std::result::Result<(), String> {
    pattern::check_syntax(pattern_str).map_err(|e| e.label())?;
    if taken.iter().any(|p| p == pattern_str) {
        return Err(format!("`{pattern_str}` is already in the list"));
    }
    Ok(())
}

/// Final action picker after bootstrap finishes writing files.
pub fn pick_final_action() -> Result<FinalAction> {
    let options = vec![
        FinalAction::CommitPushMain,
        FinalAction::FeatureBranchPR,
        FinalAction::Done,
    ];
    Select::new("What next?", options)
        .prompt()
        .context("final action cancelled")
}

/// Render a list of patterns as a dim/gray bulleted list for the
/// "here's what will happen" confirmation views.
pub fn print_pattern_list(patterns: &[String]) {
    for p in patterns {
        println!("  {}", style::info(format!("• {p}")));
    }
}

// ── Reverse-sync picker (U11) ────────────────────────────────────────────────

/// One action in the reverse-sync action loop. Mirrors the `pick_with_actions`
/// pattern (every row is one Enter-action), with an extra per-row "show diff"
/// action so the user can inspect a candidate against main without committing.
#[derive(Clone)]
enum ReverseAction {
    /// Toggle a candidate's checkbox (index into the offered slice).
    Toggle { idx: usize },
    /// Show the diff of a candidate against main, paged.
    ShowDiff { idx: usize },
    /// Commit the current selection.
    Done,
}

/// One row's display state in the reverse-sync picker.
struct ReverseRow {
    rel: PathBuf,
    status: DiffStatus,
    checked: bool,
}

fn reverse_row_label(row: &ReverseRow) -> String {
    let mark = if row.checked { "[x]" } else { "[ ]" };
    let tag = match row.status {
        DiffStatus::WorktreeOnly => style::ok("(new)"),
        DiffStatus::Differs => style::warn("(differs)"),
        // Identical rows are filtered out before reaching the picker.
        DiffStatus::Identical => style::info("(identical)"),
    };
    format!("{} {}  {}", mark, row.rel.display(), tag)
}

/// Diff-aware reverse-sync picker (R24). `offered` is the differing /
/// worktree-only candidate set (identical files already filtered out). Returns
/// the user-selected subset of relative paths (empty when the user confirms
/// with nothing checked or cancels).
///
/// Every row is one Enter-action: toggle, "show diff" (paged via
/// [`git::diff_no_index_paged`]), or "✔ Push selected". This is the
/// interactive / manual-smoke surface; the copy logic it feeds is unit-tested
/// in `reverse_sync`.
///
/// Cancelling the prompt (Esc/Ctrl-C) propagates as an error context so the
/// caller leaves main untouched.
// consumed by reverse_sync::run; manual-smoke (no unit test)
pub fn pick_reverse_sync(
    worktree_root: &Path,
    main_root: &Path,
    offered: &[(PathBuf, DiffStatus)],
) -> Result<Vec<PathBuf>> {
    let mut rows: Vec<ReverseRow> = offered
        .iter()
        .map(|(rel, status)| ReverseRow {
            rel: rel.clone(),
            status: *status,
            checked: false,
        })
        .collect();

    // Start on the first row.
    let mut cursor: usize = 0;

    loop {
        // Build the action list: per-row Toggle + ShowDiff, then Done.
        let mut actions: Vec<ReverseAction> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            actions.push(ReverseAction::Toggle { idx: i });
            labels.push(reverse_row_label(row));
            actions.push(ReverseAction::ShowDiff { idx: i });
            labels.push(format!("    {}", style::info("↳ show diff")));
        }
        actions.push(ReverseAction::Done);
        labels.push(style::ok("✔ Push selected").to_string());

        // inquire wants Display options; wrap (label, action) in a tiny struct.
        let options: Vec<LabeledAction> = labels
            .into_iter()
            .zip(actions)
            .map(|(label, action)| LabeledAction { label, action })
            .collect();

        // Clamp to the actual option count (self-correcting if the row→option
        // expansion ever changes) per the action-loop pattern doc.
        let last_idx = options.len().saturating_sub(1);
        let chosen = Select::new("Untracked files to push back to main:", options)
            .with_starting_cursor(cursor.min(last_idx))
            .with_help_message(
                "↑↓ navigate · enter to toggle / show diff / push · Esc to cancel (main untouched)",
            )
            .prompt()
            .context("reverse sync selection cancelled")?;

        match chosen.action {
            ReverseAction::Toggle { idx } => {
                rows[idx].checked = !rows[idx].checked;
                cursor = idx * 2;
            }
            ReverseAction::ShowDiff { idx } => {
                let rel = &rows[idx].rel;
                show_candidate_diff(worktree_root, main_root, rel)?;
                cursor = idx * 2 + 1;
            }
            ReverseAction::Done => {
                return Ok(rows
                    .into_iter()
                    .filter(|r| r.checked)
                    .map(|r| r.rel)
                    .collect());
            }
        }
    }
}

/// inquire `Select` option pairing a rendered label with its action.
struct LabeledAction {
    label: String,
    action: ReverseAction,
}

impl fmt::Display for LabeledAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Show the diff of a candidate against main, paged. For a worktree-only
/// (new) file there is nothing in main to diff against — print a note instead
/// of shelling out (git diff --no-index against a missing path is noisy).
fn show_candidate_diff(worktree_root: &Path, main_root: &Path, rel: &Path) -> Result<()> {
    let main_path = main_root.join(rel);
    let wt_path = worktree_root.join(rel);
    if !main_path.exists() {
        println!(
            "{}",
            style::info(format!(
                "{} does not exist in main yet — it will be created.",
                rel.display()
            ))
        );
        return Ok(());
    }
    git::diff_no_index_paged(&main_path, &wt_path)
}

/// Per-file overwrite confirm with a diff (R26, KTD10): a candidate that
/// already EXISTS in main must show a diff and require explicit confirmation
/// before its bytes are overwritten. Returns `Ok(true)` to overwrite,
/// `Ok(false)` to keep main's copy. Declining leaves main intact.
///
/// Used as the `overwrite` decision seam passed to
/// `reverse_sync::copy_candidate_into_main`. Manual-smoke (interactive).
// consumed by reverse_sync::run
pub fn confirm_overwrite_with_diff(
    worktree_root: &Path,
    main_root: &Path,
    rel: &Path,
) -> Result<bool> {
    println!();
    println!(
        "{}",
        style::warn(format!(
            "{} already exists in main. Review the diff before overwriting:",
            rel.display()
        ))
    );
    show_candidate_diff(worktree_root, main_root, rel)?;
    Confirm::new(&format!("Overwrite main's copy of {}?", rel.display()))
        .with_default(false)
        .with_help_message("N keeps main's copy untouched")
        .prompt()
        .context("overwrite confirm cancelled")
}

#[cfg(test)]
mod tests;
