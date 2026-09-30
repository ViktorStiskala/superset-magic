//! Color palette and the process-wide color decision.
//!
//! Single source of truth for terminal styling, so every binary's ad-hoc
//! prints feel like one UI. The palette mirrors `setup.sh`: gray info, bold
//! green success, bold red error, bold orange (256-color 208) warning, bold
//! cyan section headers, default-color paths.
//!
//! Color is decided once at startup based on `supports-color` and the
//! `NO_COLOR` env var. The decision is captured in a `OnceLock` and reused
//! everywhere. Tests bypass the global by calling `paint()` directly with an
//! explicit `enabled` flag.
//!
//! This module knows nothing about `inquire`: the CLI's `tui::theme` reads
//! [`enabled`] and installs the matching prompt render config on top, so the
//! plugin binary can share the palette without linking a prompt library.

use std::fmt::Display;
use std::sync::OnceLock;

use supports_color::Stream;

/// Semantic role for a piece of styled text.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// Dim/gray: paths, "Copied:" lines, help text.
    Info,
    /// Bold green: success summary.
    Ok,
    /// Bold orange (256-color 208): non-fatal skips that count.
    Warn,
    /// Bold red: failures, rejected patterns.
    Err,
    /// Bold cyan: section banner ("── Bootstrap mode ──").
    Header,
}

impl Kind {
    fn ansi(self) -> &'static str {
        match self {
            Kind::Info => "\x1b[90m",
            Kind::Ok => "\x1b[1;32m",
            Kind::Warn => "\x1b[1;38;5;208m",
            Kind::Err => "\x1b[1;31m",
            Kind::Header => "\x1b[1;36m",
        }
    }
}

const ANSI_RESET: &str = "\x1b[0m";

static COLOR_ENABLED: OnceLock<bool> = OnceLock::new();

fn detect() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    supports_color::on(Stream::Stdout).is_some()
}

/// Initialize the global color decision from the terminal (`NO_COLOR`, then
/// `supports-color` on stdout). Call once at startup; subsequent calls are
/// no-ops. A caller that also drives `inquire` prompts installs its render
/// config AFTER this, from [`enabled`] (the CLI's `tui::theme::install`).
pub fn init() {
    COLOR_ENABLED.get_or_init(detect);
}

/// Initialize the global color decision as OFF, skipping terminal detection
/// entirely.
///
/// This is what an `ss-magic-plugin hook` verb calls instead of [`init`]. Such a
/// verb answers the harness with a single JSON object on stdout and puts every
/// diagnostic on stderr; an ANSI escape would make the first unparseable and
/// the second harder to read in a transcript. Detection would usually reach the
/// same answer — a hook's streams are pipes, not a terminal — but "usually" is
/// not a contract, and `FORCE_COLOR` in the environment would flip it.
///
/// Like [`init`] this is a no-op once the decision has already been made, since
/// both write through the same `OnceLock`. That is why `main.rs` does not
/// initialize style before parsing argv: whichever of the two runs first wins,
/// so the choice has to be made after the verb is known.
pub fn init_no_color() {
    COLOR_ENABLED.get_or_init(|| false);
}

/// Whether color output is enabled in this process.
pub fn enabled() -> bool {
    *COLOR_ENABLED.get_or_init(detect)
}

/// Wrap `text` in ANSI escapes for `kind` when `enabled`, else return it
/// unchanged.
pub fn paint(text: &str, kind: Kind, enabled: bool) -> String {
    if enabled {
        format!("{}{}{}", kind.ansi(), text, ANSI_RESET)
    } else {
        text.to_string()
    }
}

fn role<S: Display>(s: S, kind: Kind) -> String {
    paint(&s.to_string(), kind, enabled())
}

pub fn info<S: Display>(s: S) -> String {
    role(s, Kind::Info)
}
pub fn ok<S: Display>(s: S) -> String {
    role(s, Kind::Ok)
}
pub fn warn<S: Display>(s: S) -> String {
    role(s, Kind::Warn)
}
pub fn err<S: Display>(s: S) -> String {
    role(s, Kind::Err)
}
pub fn header<S: Display>(s: S) -> String {
    role(s, Kind::Header)
}

/// Print a cyan section banner like `── Apply Superset config ──`.
pub fn print_section(title: &str) {
    println!("\n{}\n", header(format!("── {title} ──")));
}

#[cfg(test)]
mod tests;
