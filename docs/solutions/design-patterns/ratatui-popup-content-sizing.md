---
title: Size ratatui popups to their content, not to a percentage of the frame — percentage popups silently clip the tail
date: 2026-07-16
category: design-patterns
module: cockpit
problem_type: design_pattern
component: tooling
severity: medium
symptoms:
  - "Help/overlay content added at the BOTTOM of a popup never appears on common terminal sizes (80×24, 100×30) while the test terminal (120×40) shows everything"
  - "No wrap, no scroll, no truncation indicator — a `Paragraph` in an undersized `Rect` just stops drawing, so the user cannot tell more content exists"
root_cause: wrong_assumption
resolution_type: code_fix
tags:
  - ratatui
  - popup
  - centered_rect
  - clipping
  - help-overlay
  - testbackend
---

# Size ratatui popups to their content, not to a percentage of the frame

## Problem

The cockpit's help overlay was laid out with the classic `centered_rect(percent_x, percent_y, area)` helper (68% × 90%). The overlay's line list grew to 28 lines; `Paragraph` has no wrap or scroll configured, and ratatui simply stops rendering at the popup's bottom border — no ellipsis, no indicator. Result: on an 80×24 or 100×30 terminal the tail of the help — precisely the newly added safety facts (backup location, retention, EOL normalization) — was silently invisible. The one render test used 120×40, the one size where everything happened to fit.

## Root cause

A percentage-of-frame popup couples the popup's capacity to the terminal size, while the content length is fixed. Any growth in content creates a class of terminal sizes where `content_lines > popup_height - 2` and the overflow is clipped with zero feedback. The failure is invisible in development (developers run tall terminals) and in tests that render at one generous size.

## Fix

Compute the popup's rect FROM the content, clamped to the frame:

```rust
let w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 2; // +2 border
let h = lines.len() as u16 + 2;
let popup = centered_rect_abs(w, h, area);

/// A centered rect of absolute `width` × `height`, each clamped to `area`.
fn centered_rect_abs(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}
```

…and budget the content for the smallest terminal you claim to support: the help was compressed to 22 lines so 22 + 2 border rows fit an 80×24 frame exactly.

## Prevention

- When a popup's content is a fixed list, derive the rect from `lines.len()` / max `Line::width()` — keep percentage rects for panes whose content adapts (lists, diffs with scroll).
- Any time popup content GROWS, add/refresh a `TestBackend` render test at the smallest supported size (80×24) asserting the LAST line of content is present in the buffer — asserting the first line only proves the popup opened.
- If content cannot fit the minimum size, add scrolling plus an indicator; never rely on silent clipping.

## Related incident: horizontal clipping in the diff pane

The same failure class struck horizontally, caught in a real smoke: a candidate whose ONLY difference was a stray character at column 93 (a trailing comment past the pane's right edge) rendered as two visually identical sides — `Paragraph` clips long lines at the pane edge with no indicator, so the changed tail simply did not exist on screen. The general rule: **whenever a widget can clip content, either make the clipping visible (an indicator) or the content reachable (scrolling) — silent clipping of a diff is indistinguishable from "no difference".** Fixed with `←`/`→` horizontal content scroll under fixed line-number gutters plus a pane-title flag ("lines continue →" / "→ col N"); regression test `long_line_change_past_pane_edge_is_flagged_and_reachable` builds the smoke's exact shape.

## Where

- `src/tui/cockpit.rs` — `render_help`, `centered_rect_abs`; `render_gutter_and_content`, `max_content_width`, `visible_content_width` (horizontal case)
- `src/tui/cockpit/tests.rs` — `help_overlay_fits_an_80x24_terminal`, `long_line_change_past_pane_edge_is_flagged_and_reachable`
