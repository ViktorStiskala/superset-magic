use super::*;

/// Every URL form of the same remote must normalize to the same stem
/// (example from the spec: github.com/ViktorStiskala/upx.cz).
#[test]
fn origin_forms_normalize_identically() {
    let expect = Some("viktorstiskala_upx-cz".to_string());
    for url in [
        "https://github.com/ViktorStiskala/upx.cz.git",
        "https://github.com/ViktorStiskala/upx.cz",
        "git@github.com:ViktorStiskala/upx.cz.git",
        "ssh://git@github.com/ViktorStiskala/upx.cz.git",
        "ssh://git@github.com:22/ViktorStiskala/upx.cz",
        "git://github.com/ViktorStiskala/upx.cz.git",
        "https://github.com/ViktorStiskala/upx.cz/",
    ] {
        assert_eq!(stem_from_origin(url), expect, "url: {url}");
    }
}

/// Nested paths (GitLab groups) keep every segment, joined with `_`.
#[test]
fn nested_group_origin_keeps_all_segments() {
    assert_eq!(
        stem_from_origin("https://gitlab.com/group/sub.group/repo.git"),
        Some("group_sub-group_repo".to_string())
    );
    assert_eq!(
        stem_from_origin("git@gitlab.com:group/sub/repo.git"),
        Some("group_sub_repo".to_string())
    );
}

/// A local-path origin contributes only its final segment; junk-only input
/// yields None.
#[test]
fn local_and_degenerate_origins() {
    assert_eq!(
        stem_from_origin("/srv/git/upx.cz.git"),
        Some("upx-cz".to_string())
    );
    assert_eq!(stem_from_origin(""), None);
    assert_eq!(stem_from_origin("https://github.com/"), None);
}

/// Segment sanitization: lowercase, non-alphanumeric runs collapse to one
/// `-`, edges trimmed, `_` reserved for the joiner.
#[test]
fn sanitize_segment_rules() {
    assert_eq!(sanitize_segment("ViktorStiskala"), "viktorstiskala");
    assert_eq!(sanitize_segment("upx.cz"), "upx-cz");
    assert_eq!(sanitize_segment("my_repo..name"), "my-repo-name");
    assert_eq!(sanitize_segment(".env"), "env");
    assert_eq!(sanitize_segment("---"), "");
}

/// `file://` origins are local paths: only the final segment names the repo,
/// identical to the equivalent bare path — local directory hierarchy must
/// never leak into the archive name.
#[test]
fn file_scheme_origin_uses_only_the_final_segment() {
    assert_eq!(
        stem_from_origin("file:///srv/git/upx.cz.git"),
        Some("upx-cz".to_string())
    );
    assert_eq!(
        stem_from_origin("file:///srv/git/upx.cz.git"),
        stem_from_origin("/srv/git/upx.cz.git"),
    );
    assert_eq!(
        stem_from_origin("file:///Users/alice/client/repo.git"),
        Some("repo".to_string()),
        "local hierarchy must not leak into the name"
    );
}
