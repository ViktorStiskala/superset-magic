use super::*;
use std::fs;
use tempfile::TempDir;

fn write_file(root: &TempDir, rel: &str) {
    let path = root.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

fn opt_hits(root: &Path) -> Vec<bool> {
    matches_for_patterns(root, &OPTIONS).unwrap()
}

#[test]
fn dotenv_at_root_matches_both_dotenv_options() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir, ".env");
    assert_eq!(opt_hits(dir.path()), vec![true, true, false, false]);
}

#[test]
fn nested_dotenv_matches_only_double_star() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir, "apps/api/.env");
    assert_eq!(opt_hits(dir.path()), vec![false, true, false, false]);
}

#[test]
fn node_modules_dotenv_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir, "node_modules/foo/.env");
    assert_eq!(opt_hits(dir.path()), vec![false, false, false, false]);
}

#[test]
fn empty_repo_has_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(opt_hits(dir.path()), vec![false, false, false, false]);
}

#[test]
fn dev_vars_nested_matches_double_star() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir, "projects/foo/.dev.vars");
    assert_eq!(opt_hits(dir.path()), vec![false, false, false, true]);
}

#[test]
fn dotenv_local_at_root_matches_only_local() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir, ".env.local");
    assert_eq!(opt_hits(dir.path()), vec![false, false, true, false]);
}

#[test]
fn pattern_matches_any_returns_true_for_match() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir, "apps/api/config.json");
    assert!(pattern_matches_any(dir.path(), "apps/*/config.json").unwrap());
}

#[test]
fn pattern_matches_any_returns_false_for_no_match() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!pattern_matches_any(dir.path(), "apps/*/config.json").unwrap());
}

#[test]
fn matches_for_patterns_empty_input() {
    let dir = tempfile::tempdir().unwrap();
    let out: Vec<bool> = matches_for_patterns(dir.path(), &[]).unwrap();
    assert!(out.is_empty());
}
