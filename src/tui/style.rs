//! Color palette and `inquire` theming.
//!
//! Single source of truth for terminal styling, so ad-hoc prints and the
//! `inquire` prompts feel like one UI. The palette mirrors `setup.sh`:
//! gray info, bold green success, bold red error, bold orange (256-color
//! 208) warning, bold cyan section headers, default-color paths.
//!
//! Color is decided once at startup based on `supports-color` and the
//! `NO_COLOR` env var. The decision is captured in a `OnceLock` and
//! reused everywhere. Tests bypass the global by calling `paint()`
//! directly with an explicit `enabled` flag.

use std::fmt::Display;
use std::sync::OnceLock;

use inquire::ui::{
    Attributes, Color as InqColor, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled,
};
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

/// Initialize the global color decision and install the inquire
/// `RenderConfig`. Call once at startup; subsequent calls are no-ops.
pub fn init() {
    let on = *COLOR_ENABLED.get_or_init(detect);
    inquire::set_global_render_config(render_config(on));
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

fn render_config(enabled: bool) -> RenderConfig<'static> {
    if !enabled {
        return RenderConfig::empty();
    }
    let cyan = InqColor::DarkCyan;
    let green = InqColor::LightGreen;
    let dim_gray = InqColor::DarkGrey;
    let red = InqColor::LightRed;

    RenderConfig::default()
        .with_prompt_prefix(Styled::new("?").with_fg(cyan))
        .with_answered_prompt_prefix(Styled::new("✓").with_fg(green))
        .with_highlighted_option_prefix(Styled::new("›").with_fg(cyan))
        .with_selected_option(Some(StyleSheet::new().with_fg(green)))
        .with_selected_checkbox(
            Styled::new("[x]")
                .with_fg(green)
                .with_attr(Attributes::BOLD),
        )
        .with_unselected_checkbox(Styled::new("[ ]").with_fg(dim_gray))
        .with_help_message(StyleSheet::new().with_fg(dim_gray))
        .with_error_message(
            ErrorMessageRenderConfig::default_colored()
                .with_prefix(Styled::new("✗").with_fg(red))
                .with_message(StyleSheet::new().with_fg(red)),
        )
}

#[cfg(test)]
mod tests;
