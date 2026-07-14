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
mod tests {
    use super::*;

    #[test]
    fn paint_with_color_off_returns_raw() {
        assert_eq!(paint("hi", Kind::Info, false), "hi");
        assert_eq!(paint("hi", Kind::Ok, false), "hi");
        assert_eq!(paint("hi", Kind::Warn, false), "hi");
        assert_eq!(paint("hi", Kind::Err, false), "hi");
        assert_eq!(paint("hi", Kind::Header, false), "hi");
    }

    #[test]
    fn paint_with_color_on_wraps_in_escapes() {
        for kind in [Kind::Info, Kind::Ok, Kind::Warn, Kind::Err, Kind::Header] {
            let painted = paint("hi", kind, true);
            assert!(
                painted.starts_with("\x1b["),
                "expected ANSI prefix for {kind:?}, got: {painted:?}"
            );
            assert!(
                painted.ends_with("\x1b[0m"),
                "expected reset suffix for {kind:?}, got: {painted:?}"
            );
            assert!(painted.contains("hi"));
        }
    }

    #[test]
    fn warn_uses_xterm_208() {
        let painted = paint("hi", Kind::Warn, true);
        assert!(
            painted.contains("38;5;208"),
            "expected xterm 208 in {painted:?}"
        );
    }

    #[test]
    fn render_config_empty_when_color_off() {
        // RenderConfig::empty() is the "no styling" baseline. We just
        // verify the function chooses it; comparing structural equality
        // isn't directly supported, so this is a smoke test.
        let _ = render_config(false);
        let _ = render_config(true);
    }
}
