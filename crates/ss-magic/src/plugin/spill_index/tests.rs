//! Spill-index tests.
//!
//! Every test builds a fixture that mimics the harness's own layout —
//! `<projects>/<encoded-cwd>/<session-uuid>/tool-results/<id>.txt` — under a
//! tempdir. Nothing here reads the developer's real `~/.claude`, which would
//! make the assertions depend on whatever they happened to run last week.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

/// A projects root plus a worktree root, neither of which exists on disk yet.
fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects");
    let root = dir.path().join("work").join("repo");
    (dir, projects, root)
}

/// Create `<projects>/<encoded root>/<session>/tool-results/<name>` holding
/// `body`, and return its path.
fn spill(projects: &Path, root: &Path, session: &str, name: &str, body: &str) -> PathBuf {
    let dir = projects
        .join(encode_root(root, false))
        .join(session)
        .join(TOOL_RESULTS_DIR);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

/// Every path under `dir`, sorted — for asserting nothing was created or
/// removed.
fn tree(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.path().display().to_string())
        .collect();
    out.sort();
    out
}

// ── The directory name ────────────────────────────────────────────────────────

/// The harness flattens the absolute path's punctuation to `-`, one character
/// for one character: no run collapsing and no trimming, so the leading `/`
/// and a `/.` both survive as dashes.
#[test]
fn the_project_directory_name_flattens_the_path() {
    assert_eq!(
        encode_root(Path::new("/Users/me/.superset/worktrees/wt"), false),
        "-Users-me--superset-worktrees-wt"
    );
}

/// Digits and existing dashes pass through untouched, which is what keeps a
/// uuid-bearing worktree path readable.
#[test]
fn the_project_directory_name_keeps_alphanumerics_and_dashes() {
    assert_eq!(
        encode_root(Path::new("/a/487339a1-9fc3/b"), false),
        "-a-487339a1-9fc3-b"
    );
}

/// Underscores are the one class the encoding was not directly observed on, so
/// both spellings are tried. Either directory is found.
#[test]
fn either_underscore_spelling_of_the_project_directory_is_found() {
    for keep_underscore in [false, true] {
        let (_dir, projects, root) = fixture();
        let root = root.join("has_underscore");
        let dir = projects
            .join(encode_root(&root, keep_underscore))
            .join("s1")
            .join(TOOL_RESULTS_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "x").unwrap();

        let index = collect(&projects, &root);

        assert_eq!(index.files, 1, "keep_underscore = {keep_underscore}");
    }
}

// ── Listing ───────────────────────────────────────────────────────────────────

#[test]
fn it_lists_the_spill_files_for_this_worktree() {
    let (_dir, projects, root) = fixture();
    let a = spill(&projects, &root, "session-a", "aaa.txt", "hello");
    let b = spill(&projects, &root, "session-b", "bbb.txt", "wider body");

    let index = collect(&projects, &root);

    assert_eq!(index.files, 2);
    assert_eq!(index.bytes, 5 + 10);
    assert_eq!(index.note, None);
    let paths: Vec<&str> = index
        .sessions
        .iter()
        .flat_map(|s| s.files.iter())
        .map(|f| f.path.as_str())
        .collect();
    assert!(paths.contains(&a.display().to_string().as_str()));
    assert!(paths.contains(&b.display().to_string().as_str()));
}

/// The verb is a listing, not a manager: the harness's tree is byte-identical
/// afterwards, and so is the worktree.
#[test]
fn listing_writes_nothing() {
    let (dir, projects, root) = fixture();
    spill(&projects, &root, "session-a", "aaa.txt", "hello");
    fs::create_dir_all(&root).unwrap();
    let before = tree(dir.path());

    let _ = collect(&projects, &root);

    assert_eq!(tree(dir.path()), before);
}

/// Newest first at both levels — the file somebody wants is almost always from
/// the run they just watched.
#[test]
fn files_and_sessions_are_newest_first() {
    let (_dir, projects, root) = fixture();
    let old = spill(&projects, &root, "session-old", "old.txt", "o");
    let new = spill(&projects, &root, "session-new", "new.txt", "n");
    let newer = spill(&projects, &root, "session-new", "newer.txt", "nn");

    // Explicit mtimes rather than relying on creation order, which a coarse
    // filesystem clock can collapse into one instant.
    set_mtime(&old, 1_000);
    set_mtime(&new, 2_000);
    set_mtime(&newer, 3_000);

    let index = collect(&projects, &root);

    assert_eq!(index.sessions[0].session_id, "session-new");
    assert_eq!(index.sessions[0].files[0].name, "newer.txt");
    assert_eq!(index.sessions[0].files[1].name, "new.txt");
    assert_eq!(index.sessions[1].session_id, "session-old");
}

/// A session that never spilled has no `tool-results/` at all. That is the
/// common case, so it is skipped silently rather than listed as empty.
#[test]
fn a_session_that_never_spilled_is_skipped() {
    let (_dir, projects, root) = fixture();
    spill(&projects, &root, "session-a", "aaa.txt", "hello");
    fs::create_dir_all(
        projects
            .join(encode_root(&root, false))
            .join("session-quiet"),
    )
    .unwrap();

    let index = collect(&projects, &root);

    assert_eq!(index.sessions.len(), 1);
    assert_eq!(index.sessions[0].session_id, "session-a");
}

// ── Nothing found ─────────────────────────────────────────────────────────────

/// The harness layout missing entirely is an answer, not an error — and it is
/// an answer that has to say so, because an empty listing with no note reads as
/// "there are no spills".
#[test]
fn an_absent_harness_layout_is_reported_not_silently_empty() {
    let (_dir, projects, root) = fixture();

    let index = collect(&projects, &root);

    assert!(index.sessions.is_empty());
    assert!(index.project_dir.is_none());
    let note = index.note.expect("an empty result must carry a reason");
    assert!(note.contains("no project state"), "unexpected note: {note}");
}

/// The harness has a projects tree, but nothing for this worktree.
#[test]
fn a_worktree_the_harness_never_saw_is_reported() {
    let (_dir, projects, root) = fixture();
    fs::create_dir_all(&projects).unwrap();

    let index = collect(&projects, &root);

    assert_eq!(index.project_dir, None);
    let note = index.note.expect("an empty result must carry a reason");
    assert!(
        note.contains("no harness project directory"),
        "unexpected note: {note}"
    );
}

/// A project directory whose sessions have all been pruned.
#[test]
fn a_project_directory_with_no_spills_is_reported() {
    let (_dir, projects, root) = fixture();
    fs::create_dir_all(projects.join(encode_root(&root, false))).unwrap();

    let index = collect(&projects, &root);

    assert!(index.project_dir.is_some());
    let note = index.note.expect("an empty result must carry a reason");
    assert!(note.contains("has spilled"), "unexpected note: {note}");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Set a file's modification time to `secs` since the epoch, so ordering
/// assertions do not depend on how fast the test ran.
fn set_mtime(path: &Path, secs: u64) {
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(when).unwrap();
}
