use super::*;
use crate::tests::support::{git_run, init_main_repo, make_worktree};
use std::fs;
use std::process::{Command, Stdio};

/// `git rev-parse --short HEAD` run directly (not through the probe under
/// test), so tests that assert on the `detached-<sha>` shape have an
/// independent source for the expected sha.
fn head_short_sha(root: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// ── slugify: pure, no git involved ──────────────────────────────────────────

/// The contract's own example: a name of all dashes sanitizes to nothing.
/// [`branch_component`] is what falls through to the next identity source —
/// this just pins the empty result the fall-through is triggered by.
#[test]
fn slugify_all_dashes_is_empty() {
    assert_eq!(slugify("---"), "");
}

/// The contract's own example: a plain "treat everything non-ASCII as
/// unsafe" slugify mangles accents to junk (`n-c-d-n-me`). Real letters must
/// come out instead.
#[test]
fn slugify_strips_accents_to_real_letters() {
    assert_eq!(slugify("Ünïcödé Nàme"), "unicode-name");
}

/// The load-bearing case named in the contract: a cross-repo PR workspace's
/// branch is `<forkOwner>/<headRefName>`. `/` sanitizes like any other
/// unsafe separator, with no double-dash at the join.
#[test]
fn slugify_sanitizes_path_separator() {
    assert_eq!(slugify("bob-fork/feature-x"), "bob-fork-feature-x");
}

/// Results truncate to 40 characters.
#[test]
fn slugify_truncates_to_40() {
    let long = "a".repeat(60);
    let got = slugify(&long);
    assert_eq!(got.len(), 40);
    assert_eq!(got, "a".repeat(40));
}

/// A precomposed accented letter (`"é"` as the single codepoint U+00E9) and
/// the same letter already NFD-decomposed (`"e"` + combining acute U+0301)
/// must slugify identically — macOS/HFS+ can hand git a branch name in either
/// form for what is, to a human, the same name.
#[test]
fn slugify_is_stable_across_nfc_and_nfd_input() {
    let nfc = "café"; // precomposed é (U+00E9)
    let nfd = "cafe\u{0301}"; // "e" + combining acute accent
    assert_ne!(nfc, nfd, "the two literals really are different byte strings");
    assert_eq!(slugify(nfc), "cafe");
    assert_eq!(slugify(nfc), slugify(nfd));
}

/// A name that is only separators/dots (no traversal actually reaches git —
/// ref names can't contain `..` — but the slugify function itself must
/// degrade harmlessly rather than panic on one) collapses to nothing.
#[test]
fn slugify_handles_dots_and_traversal_shaped_input_without_panicking() {
    assert_eq!(slugify("../../etc"), "etc");
    assert_eq!(slugify(".."), "");
}

// ── identity resolution: git-dependent ──────────────────────────────────────

/// Outside a git repository the probe reports "do nothing" (R15) — there is
/// no other identity source to fall back to.
#[test]
fn resolve_outside_a_git_repo_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(resolve(dir.path()), None);
}

/// AE5. Detached HEAD yields `detached-<short-sha>` and the session proceeds
/// (resolve still returns `Some`), rather than treating a detached checkout
/// like "outside a repository".
#[test]
fn detached_head_yields_detached_short_sha_form() {
    let repo = init_main_repo("main");
    let sha = head_short_sha(repo.path());
    git_run(&["checkout", "--detach", "HEAD"], repo.path());

    let id = resolve(repo.path()).expect("detached HEAD still resolves");
    assert_eq!(id.branch, format!("detached-{sha}"));
    assert_eq!(id.slug, format!("{}-detached-{sha}", id.repo));
}

/// A branch that resolves fine via `symbolic-ref` but whose name has nothing
/// slug-safe in it (all separators, like the contract's `"---"` example)
/// falls through to the SAME detached form as an actually-detached HEAD —
/// the only other branch identity KTD12 defines. `"___"` stands in for the
/// contract's `"---"` here only because a leading `-` would make `git
/// checkout -b` parse the name as a flag; `slugify_all_dashes_is_empty`
/// above already pins the literal `"---"` case at the pure-function level.
#[test]
fn branch_name_that_slugifies_empty_falls_through_to_detached_form() {
    let repo = init_main_repo("main");
    let sha = head_short_sha(repo.path());
    git_run(&["checkout", "-b", "___"], repo.path());
    assert_eq!(
        git::symbolic_ref_head(repo.path()).unwrap().as_deref(),
        Some("___"),
        "sanity: HEAD really is attached to the all-underscore branch"
    );

    let id = resolve(repo.path()).expect("still inside a repo");
    assert_eq!(id.branch, format!("detached-{sha}"));
}

/// AE4. The workspace (a linked worktree, standing in for a Superset
/// workspace) is renamed. The slug is a pure function of (origin, branch) —
/// nothing path- or Superset-derived enters it — so a raw directory rename
/// (exactly what `superset ws update --name` does under the hood) leaves the
/// slug unchanged.
#[test]
fn resolve_is_stable_across_a_workspace_rename() {
    let main = init_main_repo("main");
    git_run(
        &["remote", "add", "origin", "git@github.com:acme/widget.git"],
        main.path(),
    );
    let (wt_dir, wt_root) = make_worktree(main.path());

    let before = resolve(&wt_root).expect("inside the worktree");
    assert_eq!(before.repo, "acme_widget");
    assert_eq!(before.branch, "feature-sync-flow-test");

    let renamed_root = wt_dir.path().join("renamed-workspace");
    fs::rename(&wt_root, &renamed_root).unwrap();

    let after = resolve(&renamed_root).expect("still inside the renamed worktree");
    assert_eq!(before, after, "slug must survive a directory rename");
}

/// AE4, restated without a rename: the same (origin, branch) pair resolves
/// to the same slug regardless of which repo-relative directory `cwd` names
/// — nothing about *where inside the tree* you ask from can enter it either.
#[test]
fn resolve_does_not_depend_on_which_subdirectory_is_passed() {
    let repo = init_main_repo("main");
    git_run(
        &["remote", "add", "origin", "git@github.com:acme/widget.git"],
        repo.path(),
    );
    fs::create_dir_all(repo.path().join("apps/api")).unwrap();

    let from_root = resolve(repo.path()).unwrap();
    let from_subdir = resolve(&repo.path().join("apps/api")).unwrap();
    assert_eq!(from_root, from_subdir);
}

/// An origin URL with no path segments at all (`stem_from_origin` returns
/// `None` for it — see `pack::tests::local_and_degenerate_origins`) falls
/// through to the main-checkout-basename source, exactly like having no
/// origin configured — this pins that `identity` reaches the same fallback
/// `pack` already covers, not the URL parsing itself.
#[test]
fn origin_with_no_path_segments_falls_back_to_basename() {
    let repo = init_main_repo("main");
    git_run(&["remote", "add", "origin", "https://github.com/"], repo.path());

    let id = resolve(repo.path()).unwrap();
    assert_eq!(id.repo, pack::repo_name_stem(repo.path()).unwrap());
}

/// A local filesystem `origin` (bare path or `file://`) contributes only its
/// final path segment — local directory hierarchy must never leak into the
/// identity, matching pack's own archive-naming behavior.
#[test]
fn local_filesystem_origin_uses_final_segment_only() {
    let repo = init_main_repo("main");
    git_run(
        &["remote", "add", "origin", "file:///home/dev/my-project.git"],
        repo.path(),
    );

    let id = resolve(repo.path()).unwrap();
    assert_eq!(id.repo, "my-project");
}

/// No `origin` at all: the repo half falls back to the main checkout
/// directory's own (sanitized) basename.
#[test]
fn no_origin_falls_back_to_main_checkout_basename() {
    let repo = init_main_repo("main");

    let id = resolve(repo.path()).unwrap();
    assert_eq!(id.repo, pack::repo_name_stem(repo.path()).unwrap());
}
