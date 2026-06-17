---
title: Inquire row-as-action via Select-loop
date: "2026-05-26"
category: design-patterns
module: superset-setup
problem_type: design_pattern
component: tooling
severity: medium
applies_when:
  - Building an interactive Rust CLI with inquire where a multi-select needs at least one row to behave as an action (open a sub-prompt, add an item, finish) rather than as a checkbox
  - Users expect "cursor on row + Enter" to act on that row, without a separate Space-toggle gesture
  - The row list is small enough (typically under 20 rows) that one Enter per checkbox toggle is acceptable
symptoms:
  - Pressing Enter on an action row in an `inquire::MultiSelect` without first pressing Space does nothing — the prompt commits the current selection without triggering the action
  - Users report the action row "does nothing" because their natural gesture (navigate + Enter) is interpreted by MultiSelect as "commit current state"
root_cause: wrong_api
resolution_type: code_fix
tags:
  - inquire
  - multiselect
  - select-loop
  - rust
  - tui
  - cli-ergonomics
  - action-sentinel
---

# Inquire row-as-action via Select-loop

## Context

`inquire::MultiSelect` splits two gestures across two keys: Space toggles a row's checkbox, Enter commits the entire selection. That model works for pure pick-lists, but breaks the moment one row is conceptually an *action* — e.g. `+ Add new pattern…`, `✔ Done`, `Open detail…` — rather than a selectable item.

The natural mental model in a terminal form is "navigate to the row I want, press Enter, something happens." In a `MultiSelect`, that gesture lands on an unchecked sentinel row, the prompt interprets the Enter as "commit current selection," and the action never fires. The user sees the prompt disappear with no follow-up — "the row did nothing."

The mismatch only surfaces when at least one row is an action rather than a checkbox. A `MultiSelect` over pure data ("pick files to copy") is fine; the trouble starts when you bolt a sentinel onto the same widget.

## Guidance

Replace `MultiSelect` with `inquire::Select` run in a loop. Every row becomes an `Action` variant; Enter on any row dispatches that action. Checkbox state lives in your own `Vec<Row>` outside inquire, and you render the `[x]` / `[ ]` markers manually inside each row label.

**1. Define an `Action` enum that covers every row category.**

```rust
#[derive(Clone)]
enum Action {
    Toggle { idx: usize, label: String },
    AddNew,
    Done,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Toggle { label, .. } => f.write_str(label),
            Action::AddNew => f.write_str("+ Add new pattern…"),
            Action::Done => f.write_str("✔ Done"),
        }
    }
}
```

**2. Hold per-row state outside inquire.**

```rust
#[derive(Clone)]
struct PatternRow {
    raw: String,
    checked: bool,
    no_match: bool,
}

fn render_row(row: &PatternRow) -> String {
    let mark = if row.checked { "[x]" } else { "[ ]" };
    if row.no_match {
        format!("{} {}  {}", mark, row.raw, style::warn("(no matches)"))
    } else {
        format!("{} {}", mark, row.raw)
    }
}
```

**3. Run `Select` in a loop with per-iteration cursor persistence.**

```rust
let mut cursor: usize = rows
    .iter()
    .position(|r| !r.checked)
    .unwrap_or(rows.len() + 1);            // fall through to Done if all are checked

loop {
    let mut actions: Vec<Action> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| Action::Toggle { idx: i, label: render_row(r) })
        .collect();
    actions.push(Action::AddNew);
    actions.push(Action::Done);

    let action = Select::new("Files to copy automatically:", actions)
        .with_starting_cursor(cursor.min(rows.len() + 1))   // inquire 0.9 API
        .with_help_message("↑↓ navigate, enter to toggle / add / confirm")
        .prompt()
        .context("pattern selection cancelled")?;

    match action {
        Action::Toggle { idx, .. } => {
            rows[idx].checked = !rows[idx].checked;
            cursor = idx;                                    // stay on the same row
        }
        Action::AddNew => {
            let taken: Vec<String> = rows.iter().map(|r| r.raw.clone()).collect();
            if let Some(new_pattern) = prompt_one_custom_pattern(&taken)? {
                let no_match = !repo_scan::pattern_matches_any(repo_root, &new_pattern)?;
                rows.push(PatternRow { raw: new_pattern, checked: true, no_match });
                cursor = rows.len() - 1;                     // land on the new row
            } else {
                cursor = rows.len();                         // Esc → back to AddNew
            }
        }
        Action::Done => {
            return Ok(rows.into_iter().filter(|r| r.checked).map(|r| r.raw).collect());
        }
    }
}
```

Inquire 0.9 API call-outs:
- `with_starting_cursor(n)` sets the highlighted row at prompt open; clamp with `.min(actions.len() - 1)` to guard against stale cursor values after the action list shrinks.
- `with_help_message(…)` overrides the default "(↑↓ to move, enter to select, type to filter)" — use it to communicate the single-key model explicitly so the user doesn't reach for Space.

## Why This Matters

Every row does exactly one thing on Enter, regardless of which row it is. The user's mental model — *navigate, press Enter, something happens* — holds without exception. There is no "Space first" prerequisite, no hidden two-step gesture, no distinction between "action rows" and "checkbox rows" from the user's point of view.

The rendered label carries full meaning: `[x] .env` communicates state, `+ Add new pattern…` communicates intent, `✔ Done` communicates finality. None of these require the user to understand how inquire renders checkboxes internally.

Trade-off: bulk-toggling N items costs N Enter presses instead of `MultiSelect`'s N Spaces + 1 Enter. For lists of 5–15 rows the cost is inconsequential. For 30+ rows, `MultiSelect`'s batch-commit model is meaningfully faster — if action rows are still needed there, prefer dedicated keybindings (where supported) over mixing actions into the item list.

## When to Apply

- A multi-select prompt contains at least one row that triggers an action (open a sub-prompt, insert an item, a "Done" sentinel) rather than toggling a checkbox.
- The expected gesture is cursor-on-row + Enter, not Space-to-mark + Enter-to-commit.
- The list is small enough (typically under 20 rows) that one Enter per toggle is not a burden.
- Rows need to re-render after each interaction — e.g. a `(no matches)` filesystem-check suffix, or showing newly added rows checked — which the loop affords naturally; `MultiSelect` does not.

## Examples

**Before — `MultiSelect` with an action sentinel (broken UX).**

```rust
let mut choices: Vec<String> = options.to_vec();
choices.push("+ Add new pattern…".to_string());            // sentinel inside MultiSelect

let selected = MultiSelect::new("Files to copy:", choices).prompt()?;

// User navigates to "+ Add new pattern…", presses Enter without Space:
//   → inquire commits the current selection; sentinel is NOT in `selected`.
// User navigates to "+ Add new pattern…", presses Space then Enter:
//   → sentinel appears in `selected`; caller must filter it out post-hoc.
if selected.contains(&"+ Add new pattern…".to_string()) {
    // open text prompt — but we've already left the MultiSelect screen
}
```

The sentinel row behaves differently from every other row: it needs Space *and* Enter. The caller then has to unpick the sentinel from the returned vec. There is no way to re-render rows after the user adds a new pattern — the next render is a brand-new `MultiSelect` invocation with all the visual flash that entails.

**After — Select-loop (the action-loop pattern), from `projects/superset-setup/src/ui.rs:pick_patterns`.**

```rust
loop {
    let mut actions: Vec<Action> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| Action::Toggle { idx: i, label: render_row(r) })
        .collect();
    actions.push(Action::AddNew);
    actions.push(Action::Done);

    let action = Select::new("Files to copy automatically:", actions)
        .with_starting_cursor(cursor.min(rows.len() + 1))
        .with_help_message("↑↓ navigate, enter to toggle / add / confirm")
        .prompt()
        .context("pattern selection cancelled")?;

    match action {
        Action::Toggle { idx, .. } => {
            rows[idx].checked = !rows[idx].checked;
            cursor = idx;
        }
        Action::AddNew => {
            // … prompt for input, insert as a checked row above the sentinel …
        }
        Action::Done => {
            return Ok(rows.into_iter().filter(|r| r.checked).map(|r| r.raw).collect());
        }
    }
}
```

Enter on any row does exactly what the row says. Cursor returns to the toggled row after a flip. `AddNew` opens a sub-prompt and inserts the result above the sentinels. `Done` exits.

## Related

- Origin commit: [`410bba3`](https://github.com/ViktorStiskala/monorepo-general/commit/410bba3) on branch `feat/superset-setup`, PR [#18](https://github.com/ViktorStiskala/monorepo-general/pull/18).
- Canonical implementation: `projects/superset-setup/src/ui.rs` — `pick_patterns`, `render_row`, `Action` enum, `PatternRow` struct.
- inquire `Select` docs: https://docs.rs/inquire/0.9/inquire/struct.Select.html
- inquire `RenderConfig` (for matching prompt styling to the rest of the CLI): https://docs.rs/inquire/0.9/inquire/ui/struct.RenderConfig.html
