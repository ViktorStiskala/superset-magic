use super::*;

/// With color off the theme is a no-op: the config it hands `inquire` is the
/// empty one, so no prefix, attribute or color is added to any prompt.
/// `RenderConfig` has no `PartialEq`, so the comparison is field by field
/// against `RenderConfig::empty()` on the fields the themed config sets.
#[test]
fn render_config_is_empty_when_color_off() {
    let off = render_config(false);
    let empty = RenderConfig::empty();
    assert_eq!(off.prompt_prefix.style, empty.prompt_prefix.style);
    assert_eq!(off.prompt_prefix.content, empty.prompt_prefix.content);
    assert_eq!(off.answered_prompt_prefix.content, empty.answered_prompt_prefix.content);
    assert_eq!(off.highlighted_option_prefix.content, empty.highlighted_option_prefix.content);
    assert_eq!(off.selected_option, empty.selected_option);
    assert_eq!(off.help_message, empty.help_message);
    assert_eq!(off.prompt_prefix.style, StyleSheet::empty());
}

/// With color on the themed config differs from the empty one where the
/// theme sets something – the prompt prefix carries a color.
#[test]
fn render_config_is_themed_when_color_on() {
    let on = render_config(true);
    assert_eq!(on.prompt_prefix.content, "?");
    assert_ne!(on.prompt_prefix.style, StyleSheet::empty());
    assert_eq!(on.prompt_prefix.style.fg, Some(InqColor::DarkCyan));
}
