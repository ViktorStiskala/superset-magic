//! The `inquire` prompt theme, installed from the palette's color decision.
//!
//! `style` (in core) decides once whether this process colors its output and
//! paints the CLI's own lines; this module is the one place that turns the
//! same decision into an `inquire` `RenderConfig`, so ad-hoc prints and the
//! prompts feel like one UI. It is split from the palette because the plugin
//! binary shares the palette but never opens a prompt, and must not link a
//! prompt library to get it.

use inquire::ui::{
    Attributes, Color as InqColor, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled,
};

use super::style;

/// Install the global `inquire` render config matching [`style::enabled`].
/// Call once at startup, after `style::init()`; with color off the installed
/// config is `RenderConfig::empty()` – no prefixes, no colors – so the prompts
/// print as plain text exactly like the rest of the output.
pub fn install() {
    inquire::set_global_render_config(render_config(style::enabled()));
}

/// The render config for a color decision: the themed palette when `enabled`,
/// otherwise the empty (unstyled) config.
pub fn render_config(enabled: bool) -> RenderConfig<'static> {
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
