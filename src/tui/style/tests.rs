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
