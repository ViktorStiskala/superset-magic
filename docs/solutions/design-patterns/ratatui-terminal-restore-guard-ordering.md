---
title: Construct the terminal-restore RAII guard right after the first fallible setup step, not after the second
date: 2026-07-16
category: design-patterns
module: cockpit
problem_type: design_pattern
component: tooling
severity: medium
symptoms:
  - "A failure entering the alternate screen (broken pipe, redirected/non-TTY stdout) leaves the terminal in raw mode with no code path that disables it"
  - "The user's shell is left wedged (no echo, no line buffering) until they manually run `reset` or `stty sane`"
  - "The panic hook does NOT help here — it only fires on an unwind, not on a `?` early return, which is exactly how this failure exits"
root_cause: wrong_ordering
resolution_type: code_fix
tags:
  - ratatui
  - crossterm
  - raii
  - drop-guard
  - terminal-restore
  - raw-mode
  - panic-hook
  - tui
---

# Construct the terminal-restore RAII guard right after the first fallible setup step, not after the second

## Problem

`ss-magic`'s reverse-sync merge cockpit (`src/tui/cockpit.rs`, `run_cockpit`)
is a full-screen `ratatui`/`crossterm` TUI. Entering it requires two ordered,
independently fallible steps: `enable_raw_mode()`, then
`EnterAlternateScreen`. Both must be undone on the way out — raw mode
disabled, the alternate screen left — or the user's shell is left broken.

The RAII guard (`TerminalGuard`, whose `Drop` calls `restore_terminal()`) was
originally constructed *after* `EnterAlternateScreen` succeeded:

```rust
// BEFORE (src/tui/cockpit.rs)
enable_raw_mode().context("enabling terminal raw mode")?;
io::stdout()
    .execute(EnterAlternateScreen)
    .context("entering the alternate screen")?;
// Restores the terminal on scope exit, INCLUDING during unwinding.
let _guard = TerminalGuard;
```

If `EnterAlternateScreen` (or a later fallible step before the guard's
construction) fails — a broken pipe, stdout redirected to a non-TTY, a
`crossterm` error — the function returns via `?` with raw mode **already
enabled** and no guard yet in scope to undo it. The panic hook installed
alongside (`install_panic_hook`) does not help: it only runs when the process
unwinds from a panic, and this is a normal `Result::Err` early return, not a
panic. The net effect: an ordinary, non-catastrophic setup failure (e.g. the
user piped `ss-magic`'s output somewhere odd) permanently leaves their shell
in raw mode until they run `reset` or `stty sane` by hand.

## Symptoms

- Any fallible step between `enable_raw_mode()` and the guard's construction
  that fails leaves raw mode on with nothing left to disable it.
- The break is invisible until the user notices their terminal is echo-less /
  unbuffered after the command exits — there's no crash, no obvious error
  correlating cause and effect.
- Reaching for the panic hook as the safety net is a mistake here: panic
  hooks and `Drop` guards cover different exit paths (unwind vs. early
  return), and this bug lives entirely in the early-return path.

## Solution

Construct the guard the instant the *first* fallible step succeeds, before
attempting the second:

```rust
// AFTER (src/tui/cockpit.rs, run_cockpit)
install_panic_hook();
enable_raw_mode().context("enabling terminal raw mode")?;
// Construct the RAII guard the instant raw mode is on, BEFORE entering the
// alternate screen: its Drop restores the terminal on EVERY later exit path
// (normal return, a `?` error from EnterAlternateScreen or terminal setup,
// or an unwinding panic) — a guard built after EnterAlternateScreen would
// leak raw mode if that step (or `Terminal::new`) failed.
let _guard = TerminalGuard;
io::stdout()
    .execute(EnterAlternateScreen)
    .context("entering the alternate screen")?;

let mut terminal =
    Terminal::new(CrosstermBackend::new(io::stdout())).context("creating the terminal")?;
event_loop(&mut terminal, &mut app)
```

`TerminalGuard`'s `Drop` remains a best-effort, error-swallowing restore (it
runs during teardown/panic paths where there's nothing sensible to do with a
further error):

```rust
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = io::stdout().execute(LeaveAlternateScreen);
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
```

`restore_terminal()` unconditionally attempts to leave the alternate screen
even when the guard is dropped right after `enable_raw_mode()` (i.e. before
`EnterAlternateScreen` ever ran) — that's harmless, since `LeaveAlternateScreen`
on a terminal that never entered the alternate screen is a no-op-ish
best-effort call, not an error path worth special-casing.

## Why This Works

Every fallible step taken *after* the guard is constructed is now covered by
its `Drop`, because Rust runs destructors for all live locals on any exit from
their scope — a normal return, a `?` short-circuit, or an unwinding panic. By
moving `let _guard = TerminalGuard;` to immediately follow the *first*
fallible operation (`enable_raw_mode()`), every subsequent fallible step
(`EnterAlternateScreen`, `Terminal::new`, and the whole `event_loop`) is
inside the guard's protected scope. There is no longer a window where raw
mode is on and nothing will undo it.

## Prevention

When a resource acquisition is a *sequence* of fallible steps that each need
undoing, install the RAII cleanup guard immediately after the **first**
fallible step succeeds — not after the last one, and not after "the setup
that felt done enough to worry about." Each fallible step between resource
acquisition and guard construction is an uncovered leak window. Separately:
remember that a panic hook and a `Drop` guard cover different failure shapes —
the hook only fires on an unwind, the guard's `Drop` is what covers an
ordinary `?` early return — so a panic hook is not a substitute for getting
the guard's placement right, and reviewing "is cleanup panic-safe?" is not the
same question as "is cleanup safe on every fallible early-return?".

## Related Issues

- Originating plan: [docs/plans/2026-07-16-001-feat-reverse-sync-merge-cockpit-plan.md](../../plans/2026-07-16-001-feat-reverse-sync-merge-cockpit-plan.md)
  (R16: the cockpit requires — and must safely tear down — a real terminal).
- Fixed in commit `92a858d` (`fix(sync,tui): address code-review findings on
  the merge cockpit`), alongside the TOCTOU-baseline and merge-assembly fixes
  documented in
  [docs/solutions/logic-errors/toctou-guard-review-time-baseline.md](../logic-errors/toctou-guard-review-time-baseline.md)
  and
  [docs/solutions/logic-errors/merge-assembly-line-fusion-missing-separator.md](../logic-errors/merge-assembly-line-fusion-missing-separator.md).
- [docs/solutions/design-patterns/inquire-action-loop-2026-05-26.md](./inquire-action-loop-2026-05-26.md) —
  another interactive-terminal-layer design-pattern note in this codebase.
