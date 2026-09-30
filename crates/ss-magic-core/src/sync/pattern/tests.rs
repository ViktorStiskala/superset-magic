use super::*;

#[test]
fn meta_detects_glob_chars() {
    assert!(has_glob_meta("*.txt"));
    assert!(has_glob_meta("apps/*/.env"));
    assert!(has_glob_meta("a?b"));
    assert!(has_glob_meta("[abc]"));
    assert!(!has_glob_meta(".env"));
    assert!(!has_glob_meta("apps/api/config"));
}

#[test]
fn parent_segment_detects_dotdot() {
    assert!(has_parent_segment("../x"));
    assert!(has_parent_segment("a/../b"));
    assert!(!has_parent_segment("..foo"));
    assert!(!has_parent_segment("foo..bar"));
    assert!(!has_parent_segment("apps/api/.env"));
}

#[test]
fn check_syntax_accepts_valid_patterns() {
    check_syntax(".env").unwrap();
    check_syntax("apps/*/.env").unwrap();
    check_syntax("**/.dev.vars").unwrap();
    check_syntax("packages/**/fixtures").unwrap();
}

#[test]
fn check_syntax_rejects_invalid_patterns() {
    assert_eq!(check_syntax("").unwrap_err(), SyntaxError::Empty);
    assert_eq!(
        check_syntax("/etc/passwd").unwrap_err(),
        SyntaxError::AbsolutePath
    );
    assert_eq!(
        check_syntax("../oops").unwrap_err(),
        SyntaxError::ParentSegment
    );
    assert!(matches!(
        check_syntax("foo[bar").unwrap_err(),
        SyntaxError::BadGlob(_)
    ));
}
