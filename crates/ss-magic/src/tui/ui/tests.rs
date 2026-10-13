use super::*;

#[test]
fn validate_rejects_absolute_paths() {
    let err = validate_pattern("/etc/passwd", &[]).unwrap_err();
    assert!(err.contains("absolute"));
}

#[test]
fn validate_rejects_parent_segments() {
    let err = validate_pattern("../oops", &[]).unwrap_err();
    assert!(err.contains(".."));
    let err = validate_pattern("apps/../oops", &[]).unwrap_err();
    assert!(err.contains(".."));
}

#[test]
fn validate_rejects_duplicates() {
    let taken = vec![".env".to_string()];
    let err = validate_pattern(".env", &taken).unwrap_err();
    assert!(err.contains("already"));
}

#[test]
fn validate_rejects_uncompilable_globs() {
    // unmatched `[` is a syntax error in globset
    let err = validate_pattern("foo[bar", &[]).unwrap_err();
    assert!(err.contains("invalid glob"));
}

#[test]
fn validate_accepts_literal_paths() {
    validate_pattern("apps/api/config", &[]).unwrap();
}

#[test]
fn validate_accepts_well_formed_globs() {
    validate_pattern("apps/*/.env", &[]).unwrap();
    validate_pattern("**/.dev.vars", &[]).unwrap();
    validate_pattern("packages/**/fixtures", &[]).unwrap();
}

#[test]
fn validate_rejects_empty() {
    let err = validate_pattern("", &[]).unwrap_err();
    assert!(err.contains("empty"));
}

// ── The shared-directory picker ─────────────────────────────────────────────

/// The shared-directory picker validates a typed pattern with the same
/// function as the sync-pattern picker, so the same things are refused.
#[test]
fn a_shared_directory_pattern_is_validated_like_a_sync_pattern() {
    assert!(validate_pattern("/abs/node_modules", &[]).unwrap_err().contains("absolute"));
    assert!(validate_pattern("../node_modules", &[]).unwrap_err().contains(".."));
    assert!(validate_pattern("apps/../node_modules", &[]).unwrap_err().contains(".."));
    assert!(validate_pattern("vendor/[bundle", &[]).unwrap_err().contains("invalid glob"));
    let taken = vec!["**/node_modules".to_string()];
    assert!(validate_pattern("**/node_modules", &taken).unwrap_err().contains("already"));
    validate_pattern("**/dist", &taken).unwrap();
    validate_pattern("vendor/bundle", &taken).unwrap();
}

fn dirs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_shared_row_names_its_first_matches_and_counts_the_rest() {
    assert_eq!(shared_suffix(&[]), RowSuffix::Warn("(no matches)".to_string()));
    assert_eq!(
        shared_suffix(&dirs(&["node_modules"])),
        RowSuffix::Dim("(node_modules)".to_string())
    );
    assert_eq!(
        shared_suffix(&dirs(&["a/node_modules", "b/node_modules", "node_modules"])),
        RowSuffix::Dim("(a/node_modules, b/node_modules, node_modules)".to_string())
    );
    assert_eq!(
        shared_suffix(&dirs(&["a", "b", "c", "d", "e"])),
        RowSuffix::Dim("(a, b, c, +2 more)".to_string())
    );
}

#[test]
fn a_row_renders_its_mark_label_and_suffix() {
    let row = |checked, suffix| Row {
        raw: "**/node_modules".to_string(),
        checked,
        suffix,
    };
    assert_eq!(render_row(&row(true, None)), "[x] **/node_modules");
    assert_eq!(render_row(&row(false, None)), "[ ] **/node_modules");
    let dim = render_row(&row(true, Some(RowSuffix::Dim("(node_modules)".to_string()))));
    assert!(dim.starts_with("[x] **/node_modules  ") && dim.contains("(node_modules)"), "{dim}");
    let warn = render_row(&row(false, Some(no_matches())));
    assert!(warn.contains("(no matches)"), "{warn}");
}
