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
