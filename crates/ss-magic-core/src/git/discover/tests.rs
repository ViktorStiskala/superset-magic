//! The equivalence matrix: for every layout, `discover` either declines or
//! returns roots byte-equal to what the two `git rev-parse` probes say, and
//! `roots` – the pipeline's entry point – equals today's answer in EVERY
//! scenario, declined or not.
//!
//! Every fixture is a real repository built with `git init` /
//! `git worktree add` in a tempdir; nothing here mocks the filesystem. The
//! matrix runs `discover_with_env` under an EMPTY environment so a `GIT_*`
//! variable in the developer's shell cannot turn every scenario into a
//! vacuous decline; the real-environment wiring has its own tests below.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::git::{cwd_repo_root, main_checkout_root};
use crate::testutil::{
    git_run, init_main_repo, neutralize_global_excludes, run_ignored_test_in_child,
    test_path_in_binary,
};

// ── The oracle ────────────────────────────────────────────────────────────────

/// The empty environment the matrix runs under.
fn no_env(_: &str) -> Option<OsString> {
    None
}

/// What the subprocess probes say for `cwd`, computed exactly the way the hook
/// pipeline computed them before this module existed: the worktree root from
/// `--show-toplevel`, and the main root from `--git-common-dir` asked at that
/// root – or at `cwd` itself when there is no worktree root, which is what
/// `config::resolve` did.
struct GitAnswer {
    toplevel: Option<PathBuf>,
    common: Option<PathBuf>,
    main: Option<PathBuf>,
}

fn git_answer(cwd: &Path) -> GitAnswer {
    let toplevel = cwd_repo_root(cwd).ok();
    let main = main_checkout_root(toplevel.as_deref().unwrap_or(cwd)).ok();
    let common = crate::git::git(&["rev-parse", "--git-common-dir"], Some(cwd))
        .ok()
        .and_then(|out| crate::git::resolve(&out, cwd).ok());
    GitAnswer {
        toplevel,
        common,
        main,
    }
}

/// True when none of the declining variables is set in THIS process, so the
/// real-environment `roots` can be expected to take the fast path.
fn process_env_is_clean() -> bool {
    DECLINING_ENV
        .iter()
        .all(|(name, _)| std::env::var_os(name).is_none())
}

/// The matrix assertion. Returns the discovery so a scenario can also pin
/// WHICH answer it expects.
fn assert_agrees(cwd: &Path, label: &str) -> Discovery {
    let found = discover_with_env(cwd, &no_env);
    let git = git_answer(cwd);
    match &found {
        Discovery::Found(roots) => {
            assert_eq!(
                Some(&roots.worktree_root),
                git.toplevel.as_ref(),
                "{label}: worktree root differs from `git rev-parse --show-toplevel`"
            );
            assert_eq!(
                Some(&roots.common_dir),
                git.common.as_ref(),
                "{label}: common dir differs from `git rev-parse --git-common-dir`"
            );
            assert_eq!(
                Some(&roots.main_checkout_root),
                git.main.as_ref(),
                "{label}: main checkout root differs from git's"
            );
        }
        Discovery::NotARepository => {
            assert!(
                git.toplevel.is_none(),
                "{label}: NotARepository but git found {:?}",
                git.toplevel
            );
            assert!(
                git.main.is_none(),
                "{label}: NotARepository but git found a common dir {:?}",
                git.main
            );
        }
        Discovery::Undecided(_) => {}
    }

    // The pipeline's inputs are today's inputs, in every scenario. `roots`
    // reads the real environment; when that is clean, the fast path must have
    // been taken exactly when the matrix says it is decidable.
    let resolved = roots(cwd);
    assert_eq!(
        resolved.repo_root, git.toplevel,
        "{label}: roots().repo_root"
    );
    assert_eq!(resolved.main_root, git.main, "{label}: roots().main_root");
    if process_env_is_clean() {
        let expect_fallback = matches!(found, Discovery::Undecided(_));
        assert_eq!(
            resolved.fallback.is_some(),
            expect_fallback,
            "{label}: fallback ran = {:?}, matrix said {found:?}",
            resolved.fallback
        );
    }
    found
}

fn reason_of(found: &Discovery) -> &'static str {
    match found {
        Discovery::Undecided(reason) => reason,
        other => panic!("expected Undecided, got {other:?}"),
    }
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// A plain repository; the root is canonical.
fn repo() -> (TempDir, PathBuf) {
    let dir = init_main_repo("main");
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

/// `root/src/deep`, created.
fn nested(root: &Path) -> PathBuf {
    let deep = root.join("src/deep");
    fs::create_dir_all(&deep).unwrap();
    deep
}

/// A main checkout and a linked worktree as SIBLINGS under one tempdir, so a
/// relative `gitdir:` path between them is short and deterministic.
fn pair() -> (TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    fs::create_dir(&main).unwrap();
    git_run(&["init", "-q", "-b", "main"], &main);
    neutralize_global_excludes(&main);
    fs::write(main.join("README.md"), "hi").unwrap();
    git_run(&["add", "."], &main);
    git_run(&["commit", "-q", "-m", "init"], &main);
    let wt = dir.path().join("wt");
    git_run(
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
        &main,
    );
    let main = main.canonicalize().unwrap();
    let wt = wt.canonicalize().unwrap();
    (dir, main, wt)
}

/// A repository with a hand-made `sub/.git` of the caller's design, so a
/// broken inner layout has a real outer repository for git to walk up to.
fn outer_with_inner(build_inner_git: impl FnOnce(&Path)) -> (TempDir, PathBuf, PathBuf) {
    let (dir, outer) = repo();
    let inner = outer.join("inner");
    fs::create_dir_all(inner.join("deeper")).unwrap();
    build_inner_git(&inner.join(".git"));
    (dir, outer, inner)
}

/// The path git's `.git` gitfile in `wt` points at, as written by
/// `git worktree add`.
fn gitfile_target(wt: &Path) -> String {
    let body = fs::read_to_string(wt.join(".git")).unwrap();
    body.trim_end()
        .strip_prefix("gitdir: ")
        .unwrap()
        .to_string()
}

// ── Ownership declines ────────────────────────────────────────────────────────
//
// A real second uid is not available to a single-user test run, so these
// drive the private steps with an euid that is deliberately NOT this
// process's: every directory then reads as owned by somebody else, which is
// exactly the "dubious ownership" shape git refuses.

/// An euid that owns nothing in the tempdir.
fn somebody_else() -> u32 {
    effective_uid().wrapping_add(1)
}

/// A plain repository whose `.git` directory is (as far as the walk can tell)
/// owned by another user declines instead of answering.
#[test]
fn a_repository_owned_by_another_user_declines() {
    let (_dir, root) = repo();
    let step = inspect_dot_git(&root, somebody_else());
    assert!(
        matches!(step, Step::Decline("repository is not owned by this user")),
        "{step:?}"
    );
    // The same layout with the real euid is the Found the matrix pins.
    assert!(matches!(inspect_dot_git(&root, effective_uid()), Step::Found(_)));
}

/// A linked worktree whose gitfile target is owned by another user declines
/// at the target check, before the commondir is ever read.
#[test]
fn a_gitfile_target_owned_by_another_user_declines() {
    let (_dir, _main, wt) = pair();
    let step = from_gitfile(&wt, &wt.join(".git"), somebody_else());
    assert!(
        matches!(step, Step::Decline("repository is not owned by this user")),
        "{step:?}"
    );
    assert!(matches!(
        from_gitfile(&wt, &wt.join(".git"), effective_uid()),
        Step::Found(_)
    ));
}

// ── Decidable layouts (R20, AE5) ──────────────────────────────────────────────

#[test]
fn plain_repo_at_its_root_is_found() {
    let (_dir, root) = repo();
    let found = assert_agrees(&root, "plain repo, cwd at root");
    let Discovery::Found(roots) = found else {
        panic!("{found:?}")
    };
    assert_eq!(roots.worktree_root, root);
    assert_eq!(roots.common_dir, root.join(".git"));
    assert_eq!(roots.main_checkout_root, root);
}

#[test]
fn plain_repo_from_a_nested_directory_is_found() {
    let (_dir, root) = repo();
    let found = assert_agrees(&nested(&root), "plain repo, cwd nested");
    assert!(matches!(found, Discovery::Found(_)), "{found:?}");
}

/// AE5: a linked worktree from a nested cwd resolves to the worktree and to
/// the main checkout, without a subprocess.
#[test]
fn ae5_linked_worktree_nested_agrees_with_git() {
    let (_dir, main, wt) = pair();
    let found = assert_agrees(&nested(&wt), "linked worktree, cwd nested");
    let Discovery::Found(roots) = found else {
        panic!("{found:?}")
    };
    assert_eq!(roots.worktree_root, wt);
    assert_eq!(roots.common_dir, main.join(".git"));
    assert_eq!(roots.main_checkout_root, main);
}

#[test]
fn linked_worktree_at_its_root_agrees_with_git() {
    let (_dir, _main, wt) = pair();
    let found = assert_agrees(&wt, "linked worktree, cwd at root");
    assert!(matches!(found, Discovery::Found(_)), "{found:?}");
}

/// A gitfile with a hand-written RELATIVE `gitdir:` path – the shape
/// `git worktree add --relative-paths` (and the `relativeWorktrees`
/// extension) writes – resolves against the directory holding the gitfile.
#[test]
fn gitfile_with_a_relative_gitdir_path_is_found() {
    let (_dir, main, wt) = pair();
    let target = gitfile_target(&wt);
    let name = Path::new(&target).file_name().unwrap().to_str().unwrap();
    fs::write(
        wt.join(".git"),
        format!("gitdir: ../main/.git/worktrees/{name}\n"),
    )
    .unwrap();

    let found = assert_agrees(&nested(&wt), "relative gitdir");
    let Discovery::Found(roots) = found else {
        panic!("{found:?}")
    };
    assert_eq!(roots.worktree_root, wt);
    assert_eq!(roots.main_checkout_root, main);
}

/// git strips a trailing CR/LF from the gitfile's path and nothing else.
#[test]
fn gitfile_with_crlf_is_found() {
    let (_dir, _main, wt) = pair();
    let target = gitfile_target(&wt);
    fs::write(wt.join(".git"), format!("gitdir: {target}\r\n")).unwrap();
    let found = assert_agrees(&wt, "gitfile CRLF");
    assert!(matches!(found, Discovery::Found(_)), "{found:?}");
}

/// A cwd reached THROUGH a symlink resolves to the canonical roots, because
/// git's own `getcwd` sees the resolved path too.
#[test]
fn cwd_reached_through_a_symlink_is_found_canonical() {
    let (_dir, root) = repo();
    nested(&root);
    let link_dir = tempfile::tempdir().unwrap();
    let link = link_dir.path().join("link");
    std::os::unix::fs::symlink(&root, &link).unwrap();

    let found = assert_agrees(&link.join("src/deep"), "cwd via symlink");
    let Discovery::Found(roots) = found else {
        panic!("{found:?}")
    };
    assert_eq!(roots.worktree_root, root);
    assert!(!roots.worktree_root.starts_with(link_dir.path()));
}

/// A linked worktree of a BARE repository: git's common dir is the bare
/// directory and its "main checkout root" is that directory's parent. Odd,
/// but it is what the probes say, and byte-equality is the bar.
#[test]
fn worktree_of_a_bare_repository_agrees_with_git() {
    let (_src, src_root) = repo();
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("bare.git");
    git_run(
        &[
            "clone",
            "-q",
            "--bare",
            src_root.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        dir.path(),
    );
    let wt = dir.path().join("wt");
    git_run(&["worktree", "add", "-q", wt.to_str().unwrap()], &bare);

    let found = assert_agrees(&nested(&wt.canonicalize().unwrap()), "worktree of bare");
    let Discovery::Found(roots) = found else {
        panic!("{found:?}")
    };
    assert_eq!(roots.common_dir, bare.canonicalize().unwrap());
    assert_eq!(roots.main_checkout_root, dir.path().canonicalize().unwrap());
}

/// git accepts a detached `HEAD` by its first 40 hex characters alone, and a
/// symref with any run of whitespace after `ref:`; both shapes are decidable.
#[test]
fn head_shapes_git_accepts_are_found() {
    let (_dir, root) = repo();
    let head = root.join(".git/HEAD");

    fs::write(
        &head,
        "0123456789abcdef0123456789abcdef01234567 trailing junk",
    )
    .unwrap();
    assert!(matches!(
        assert_agrees(&root, "HEAD: 40 hex + junk"),
        Discovery::Found(_)
    ));

    fs::write(&head, "ref:\t \trefs/heads/main").unwrap();
    assert!(matches!(
        assert_agrees(&root, "HEAD: ref: with whitespace"),
        Discovery::Found(_)
    ));
}

/// A repository whose `.git/config` is missing is still a repository to git.
#[test]
fn missing_config_file_is_found() {
    let (_dir, root) = repo();
    fs::remove_file(root.join(".git/config")).unwrap();
    assert!(matches!(
        assert_agrees(&root, "no config"),
        Discovery::Found(_)
    ));
}

/// The known repository extensions do not touch the two roots.
#[test]
fn known_extensions_are_found() {
    let (_dir, root) = repo();
    git_run(&["config", "core.repositoryformatversion", "1"], &root);
    git_run(&["config", "extensions.worktreeConfig", "true"], &root);
    git_run(&["config", "extensions.preciousObjects", "true"], &root);
    assert!(matches!(
        assert_agrees(&nested(&root), "known extensions"),
        Discovery::Found(_)
    ));
}

// ── Declines (R21, AE6) ───────────────────────────────────────────────────────

/// AE6: `.git` is a symlink. git follows it happily; the fast path declines
/// and the fallback answers the same.
#[test]
fn ae6_symlinked_dot_git_declines() {
    let (_target, target_root) = repo();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(target_root.join(".git"), root.join(".git")).unwrap();

    let found = assert_agrees(&root, "symlinked .git");
    assert_eq!(reason_of(&found), ".git is a symlink");
    // git does answer here – the decline is not "not a repository".
    assert!(git_answer(&root).toplevel.is_some());
}

/// AE6: cwd inside `.git/hooks`. git errors on `--show-toplevel` but answers
/// `--git-common-dir`, and today's pipeline reads the main root from the
/// latter; `roots` reproduces exactly that pair.
#[test]
fn ae6_cwd_inside_the_git_directory_declines_and_git_errors() {
    let (_dir, root) = repo();
    let hooks = root.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();

    let found = assert_agrees(&hooks, "cwd in .git/hooks");
    assert_eq!(
        reason_of(&found),
        "inside a git directory or bare repository"
    );
    let git = git_answer(&hooks);
    assert!(git.toplevel.is_none());
    assert_eq!(git.main.as_deref(), Some(root.as_path()));
    let resolved = roots(&hooks);
    assert_eq!(resolved.repo_root, None);
    assert_eq!(resolved.main_root, Some(root));
}

/// AE6: a gitfile whose target has no `commondir` – the submodule shape.
#[test]
fn ae6_gitfile_without_commondir_declines() {
    let dir = tempfile::tempdir().unwrap();
    let (_src, src_root) = repo();
    let root = dir.path().join("sub");
    fs::create_dir(&root).unwrap();
    // A full git directory copied beside the working tree, referenced by a
    // gitfile: what `git submodule` produces (under `.git/modules/`).
    copy_dir(&src_root.join(".git"), &root.join("gitdir"));
    fs::write(root.join(".git"), "gitdir: gitdir\n").unwrap();

    let found = assert_agrees(&root.canonicalize().unwrap(), "gitfile without commondir");
    assert_eq!(reason_of(&found), "gitfile target has no commondir");
    assert!(git_answer(&root).toplevel.is_some());
}

/// AE6: a bare repository. git's `--show-toplevel` fails inside it.
#[test]
fn ae6_bare_repository_declines() {
    let (_src, src_root) = repo();
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("bare.git");
    git_run(
        &[
            "clone",
            "-q",
            "--bare",
            src_root.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        dir.path(),
    );
    let found = assert_agrees(&bare, "bare repository");
    assert_eq!(
        reason_of(&found),
        "inside a git directory or bare repository"
    );
    let inside = bare.join("hooks");
    fs::create_dir_all(&inside).unwrap();
    let found = assert_agrees(&inside, "bare repository, nested");
    assert_eq!(
        reason_of(&found),
        "inside a git directory or bare repository"
    );
    assert!(git_answer(&inside).toplevel.is_none());
}

/// A `.git` directory without `HEAD`. git does not consider it a repository
/// and walks up to the outer one.
#[test]
fn dot_git_without_head_declines_and_git_walks_up() {
    let (_dir, outer, inner) = outer_with_inner(|dot_git| {
        fs::create_dir_all(dot_git.join("objects")).unwrap();
        fs::create_dir_all(dot_git.join("refs")).unwrap();
    });
    let found = assert_agrees(&inner.join("deeper"), ".git without HEAD");
    assert_eq!(
        reason_of(&found),
        ".git directory has no valid HEAD, objects and refs"
    );
    assert_eq!(
        git_answer(&inner).toplevel.as_deref(),
        Some(outer.as_path())
    );
}

/// KTD8 asked only for a regular `HEAD`. git additionally requires its
/// CONTENT to be a symref or an object id, and walks up otherwise – so a
/// `HEAD` full of garbage is a decline here, never a `Found`.
#[test]
fn dot_git_with_a_garbage_head_declines_and_git_walks_up() {
    let (_dir, outer, inner) = outer_with_inner(|dot_git| {
        fs::create_dir_all(dot_git.join("objects")).unwrap();
        fs::create_dir_all(dot_git.join("refs")).unwrap();
        fs::write(dot_git.join("HEAD"), "garbage\n").unwrap();
    });
    let found = assert_agrees(&inner, ".git with garbage HEAD");
    assert_eq!(
        reason_of(&found),
        ".git directory has no valid HEAD, objects and refs"
    );
    assert_eq!(
        git_answer(&inner).toplevel.as_deref(),
        Some(outer.as_path())
    );
}

/// Likewise a `.git` with a valid `HEAD` but no `objects/`: git walks up.
#[test]
fn dot_git_without_objects_declines_and_git_walks_up() {
    let (_dir, outer, inner) = outer_with_inner(|dot_git| {
        fs::create_dir_all(dot_git.join("refs")).unwrap();
        fs::write(dot_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    });
    let found = assert_agrees(&inner, ".git without objects");
    assert_eq!(
        reason_of(&found),
        ".git directory has no valid HEAD, objects and refs"
    );
    assert_eq!(
        git_answer(&inner).toplevel.as_deref(),
        Some(outer.as_path())
    );
}

/// An old-style symlinked `HEAD` is a repository to git; the fast path
/// declines rather than following it.
#[test]
fn symlinked_head_declines() {
    let (_dir, root) = repo();
    let head = root.join(".git/HEAD");
    fs::remove_file(&head).unwrap();
    std::os::unix::fs::symlink("refs/heads/main", &head).unwrap();
    let found = assert_agrees(&root, "symlinked HEAD");
    assert_eq!(
        reason_of(&found),
        ".git directory has no valid HEAD, objects and refs"
    );
    assert!(git_answer(&root).toplevel.is_some());
}

/// A `.git` DIRECTORY carrying a `commondir` file is a worktree git-dir
/// sitting where a main checkout's would: git's common dir is then somewhere
/// else, so `main_checkout_root` is not `D`.
#[test]
fn dot_git_directory_with_a_commondir_declines() {
    let (_dir, root) = repo();
    let (_other, other_root) = repo();
    fs::write(
        root.join(".git/commondir"),
        format!("{}\n", other_root.join(".git").display()),
    )
    .unwrap();
    let found = assert_agrees(&root, ".git with commondir");
    assert_eq!(reason_of(&found), ".git directory carries a commondir");
    assert_eq!(
        git_answer(&root).main.as_deref(),
        Some(other_root.as_path())
    );
}

/// A gitfile that does not start with `gitdir: `, or names nothing, is a
/// fatal error to git.
#[test]
fn malformed_gitfile_declines() {
    let (_dir, _main, wt) = pair();
    fs::write(wt.join(".git"), "not a gitfile\n").unwrap();
    let found = assert_agrees(&wt, "gitfile without prefix");
    assert_eq!(reason_of(&found), "gitfile lacks the gitdir: prefix");
    assert!(git_answer(&wt).toplevel.is_none());

    fs::write(wt.join(".git"), "gitdir: \n").unwrap();
    let found = assert_agrees(&wt, "gitfile with empty path");
    assert_eq!(reason_of(&found), "gitfile names no path");

    fs::write(wt.join(".git"), "").unwrap();
    let found = assert_agrees(&wt, "empty gitfile");
    assert_eq!(reason_of(&found), "gitfile lacks the gitdir: prefix");
}

/// A gitfile whose target cannot be canonicalized – including the trailing
/// space git keeps as part of the path.
#[test]
fn gitfile_with_an_unresolvable_target_declines() {
    let (_dir, _main, wt) = pair();
    let target = gitfile_target(&wt);
    fs::write(wt.join(".git"), format!("gitdir: {target} \n")).unwrap();
    let found = assert_agrees(&wt, "gitfile trailing space");
    assert_eq!(reason_of(&found), "gitfile target cannot be canonicalized");
    assert!(git_answer(&wt).toplevel.is_none());

    fs::write(wt.join(".git"), "gitdir: /nonexistent/gitdir\n").unwrap();
    let found = assert_agrees(&wt, "gitfile missing target");
    assert_eq!(reason_of(&found), "gitfile target cannot be canonicalized");
}

/// KTD8 caps the gitfile read at 4 KiB. A larger file is declined outright
/// rather than read truncated, since a truncated path could name something.
#[test]
fn oversized_gitfile_declines() {
    let (_dir, _main, wt) = pair();
    let target = gitfile_target(&wt);
    let padding = " ".repeat(GITFILE_MAX_BYTES);
    fs::write(wt.join(".git"), format!("gitdir: {target}{padding}\n")).unwrap();
    let found = assert_agrees(&wt, "oversized gitfile");
    assert_eq!(reason_of(&found), "gitfile is larger than 4 KiB");
}

/// A gitfile pointing at a directory that has a `commondir` but is not a
/// git directory (no `HEAD`): git fails with "not a git repository".
#[test]
fn gitfile_target_that_is_not_a_git_directory_declines() {
    let (_dir, main, wt) = pair();
    let target = PathBuf::from(gitfile_target(&wt));
    fs::remove_file(target.join("HEAD")).unwrap();
    let found = assert_agrees(&wt, "gitfile target without HEAD");
    assert_eq!(reason_of(&found), "gitfile target is not a git directory");
    assert!(git_answer(&wt).toplevel.is_none());
    // And a commondir whose target lacks objects/refs.
    fs::write(target.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
    fs::write(target.join("commondir"), format!("{}\n", main.display())).unwrap();
    let found = assert_agrees(&wt, "commondir target without objects");
    assert_eq!(reason_of(&found), "gitfile target is not a git directory");
}

/// A `commondir` naming something that does not exist.
#[test]
fn unresolvable_commondir_declines() {
    let (_dir, _main, wt) = pair();
    let target = PathBuf::from(gitfile_target(&wt));
    fs::write(target.join("commondir"), "../../nonexistent\n").unwrap();
    let found = assert_agrees(&wt, "unresolvable commondir");
    assert_eq!(reason_of(&found), "commondir cannot be canonicalized");
}

/// `core.worktree` moves git's worktree root away from the directory that
/// holds `.git`; the fast path cannot see that from the layout, so it
/// declines whenever the repository config sets it.
#[test]
fn core_worktree_declines_and_git_points_elsewhere() {
    let (_dir, root) = repo();
    let elsewhere = tempfile::tempdir().unwrap();
    git_run(
        &[
            "config",
            "core.worktree",
            elsewhere.path().to_str().unwrap(),
        ],
        &root,
    );
    let found = assert_agrees(&nested(&root), "core.worktree");
    assert_eq!(
        reason_of(&found),
        "repository config sets core.bare or core.worktree"
    );
    assert_eq!(
        git_answer(&root).toplevel.as_deref(),
        Some(elsewhere.path().canonicalize().unwrap().as_path())
    );
}

/// `core.bare = true` makes git refuse `--show-toplevel` even with a working
/// tree beside `.git`.
#[test]
fn core_bare_true_declines_and_git_errors() {
    let (_dir, root) = repo();
    git_run(&["config", "core.bare", "true"], &root);
    let found = assert_agrees(&root, "core.bare = true");
    assert_eq!(
        reason_of(&found),
        "repository config sets core.bare or core.worktree"
    );
    assert!(git_answer(&root).toplevel.is_none());
}

/// git parses a key on the same line as its section header.
#[test]
fn core_bare_on_the_section_header_line_declines() {
    let (_dir, root) = repo();
    let config = root.join(".git/config");
    let mut body = fs::read_to_string(&config).unwrap();
    body.push_str("[core] bare = true\n");
    fs::write(&config, body).unwrap();
    let found = assert_agrees(&root, "[core] bare = true on one line");
    assert_eq!(
        reason_of(&found),
        "repository config sets core.bare or core.worktree"
    );
    assert!(git_answer(&root).toplevel.is_none());
}

/// A linked worktree ignores the common config's `core.bare` – until
/// `extensions.worktreeConfig` is on, when git honors it again.
#[test]
fn worktree_config_extension_revives_core_bare_for_a_linked_worktree() {
    let (_dir, _main, wt) = pair();
    let git_dir = PathBuf::from(gitfile_target(&wt));
    let common = git_dir.parent().unwrap().parent().unwrap().to_path_buf();
    git_run(
        &[
            "config",
            "--file",
            common.join("config").to_str().unwrap(),
            "core.bare",
            "true",
        ],
        &wt,
    );
    // Without the extension git still works in the worktree, and so does the
    // fast path.
    assert!(matches!(
        assert_agrees(&wt, "linked worktree, common core.bare=true, no extension"),
        Discovery::Found(_)
    ));

    git_run(
        &[
            "config",
            "--file",
            common.join("config").to_str().unwrap(),
            "extensions.worktreeConfig",
            "true",
        ],
        &wt,
    );
    let found = assert_agrees(&wt, "linked worktree, common core.bare=true, extension on");
    assert_eq!(
        reason_of(&found),
        "repository config sets core.bare or core.worktree"
    );
    assert!(git_answer(&wt).toplevel.is_none());
}

/// A per-worktree `config.worktree` can carry `core.bare` too.
#[test]
fn core_bare_in_config_worktree_declines() {
    let (_dir, root) = repo();
    fs::write(root.join(".git/config.worktree"), "[core]\n\tbare = true\n").unwrap();
    let found = assert_agrees(&root, "config.worktree core.bare");
    assert_eq!(
        reason_of(&found),
        "repository config sets core.bare or core.worktree"
    );
}

/// A repository format git refuses to open is a decline, not a `Found`.
#[test]
fn unsupported_repository_format_declines_and_git_errors() {
    let (_dir, root) = repo();
    git_run(&["config", "core.repositoryformatversion", "2"], &root);
    let found = assert_agrees(&root, "format version 2");
    assert_eq!(
        reason_of(&found),
        "repository config declares an unsupported format"
    );
    assert!(git_answer(&root).toplevel.is_none());

    // git refuses to run `git config` in a repository it cannot open, so the
    // file is rewritten directly.
    fs::write(
        root.join(".git/config"),
        "[core]\n\trepositoryformatversion = 1\n\tbare = false\n[extensions]\n\tbogus = 1\n",
    )
    .unwrap();
    let found = assert_agrees(&root, "unknown extension");
    assert_eq!(
        reason_of(&found),
        "repository config declares an unsupported format"
    );
    assert!(git_answer(&root).toplevel.is_none());
}

/// A config git cannot parse is a fatal error to git.
#[test]
fn unparseable_config_declines() {
    let (_dir, root) = repo();
    fs::write(root.join(".git/config"), "bare = false\n[core]\n").unwrap();
    let found = assert_agrees(&root, "key before any section");
    assert_eq!(reason_of(&found), "repository config could not be scanned");
}

/// A tempdir outside any repository. `NotARepository` is reachable only when
/// the walk to `/` stays on one filesystem; on a machine whose temp dir is
/// its own mount, git stops at the boundary and so does the fast path.
#[test]
fn non_repository_directory_is_not_a_repository_or_a_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let found = assert_agrees(&cwd, "no repository");
    let root_dev = fs::metadata("/").unwrap().dev();
    let same_filesystem_up_to_root = cwd.ancestors().all(|a| {
        fs::metadata(a)
            .map(|m| m.dev() == root_dev)
            .unwrap_or(false)
    });
    if same_filesystem_up_to_root {
        assert!(matches!(found, Discovery::NotARepository), "{found:?}");
    } else {
        assert_eq!(reason_of(&found), "filesystem boundary");
    }
    let resolved = roots(&cwd);
    assert_eq!(resolved.repo_root, None);
    assert_eq!(resolved.main_root, None);
}

/// A cwd that cannot be canonicalized.
#[test]
fn a_missing_cwd_declines() {
    let found = discover_with_env(Path::new("/no/such/directory"), &no_env);
    assert_eq!(reason_of(&found), "cwd cannot be canonicalized");
}

// ── The environment (R21, AE6) ────────────────────────────────────────────────

/// Each of the declining variables, injected: the reason names the variable.
#[test]
fn ae6_each_git_variable_declines_when_injected() {
    let (_dir, root) = repo();
    for (name, _) in DECLINING_ENV {
        let env = |asked: &str| (asked == name).then(|| OsString::from("x"));
        let found = discover_with_env(&root, &env);
        let reason = reason_of(&found);
        assert!(reason.contains(name), "{name}: {reason}");
    }
    // The empty string is still "set" – git treats an empty GIT_DIR as an
    // explicit (and broken) git directory, not as unset.
    let env = |asked: &str| (asked == "GIT_DIR").then(OsString::new);
    assert!(reason_of(&discover_with_env(&root, &env)).contains("GIT_DIR"));
}

/// The five variables R21 names are all in the table, plus the sixth git's
/// own git-directory test consults.
#[test]
fn the_declining_variables_are_the_documented_set() {
    let names: Vec<&str> = DECLINING_ENV.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_CEILING_DIRECTORIES",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_OBJECT_DIRECTORY",
        ]
    );
}

/// `discover` reads the process environment. Proven per variable in a CHILD
/// process (see `run_ignored_test_in_child` for why not `set_var`): the
/// parent builds the fixture with real git, the child runs discovery with
/// exactly one variable set and checks that the fallback still equals git's
/// answer under that same variable.
#[test]
fn ae6_each_git_variable_declines_in_the_real_environment() {
    let (_dir, root) = repo();
    let cwd = nested(&root);
    let child = test_path_in_binary(module_path!(), "child_real_environment_declines");
    for (name, _) in DECLINING_ENV {
        let value: OsString = match name {
            "GIT_DISCOVERY_ACROSS_FILESYSTEM" => "1".into(),
            "GIT_CEILING_DIRECTORIES" => "/nonexistent-ceiling".into(),
            "GIT_OBJECT_DIRECTORY" => root.join(".git/objects").into(),
            _ => root.join(".git").into(),
        };
        run_ignored_test_in_child(
            &child,
            &[
                ("SS_MAGIC_TEST_CWD", cwd.as_os_str()),
                ("SS_MAGIC_TEST_EXPECT_VAR", OsStr::new(name)),
                (name, value.as_os_str()),
            ],
            &[],
        );
    }
}

/// The child half of [`ae6_each_git_variable_declines_in_the_real_environment`].
#[test]
#[ignore = "child half of ae6_each_git_variable_declines_in_the_real_environment; run by the parent in a subprocess"]
fn child_real_environment_declines() {
    let Some(cwd) = std::env::var_os("SS_MAGIC_TEST_CWD") else {
        return;
    };
    let cwd = PathBuf::from(cwd);
    let var = std::env::var("SS_MAGIC_TEST_EXPECT_VAR").unwrap();
    assert!(
        std::env::var_os(&var).is_some(),
        "{var} is not set in the child"
    );

    let found = discover(&cwd);
    let reason = reason_of(&found);
    assert!(reason.contains(&var), "{var}: {reason}");

    let git = git_answer(&cwd);
    let resolved = roots(&cwd);
    assert_eq!(resolved.fallback, Some(reason));
    assert_eq!(resolved.repo_root, git.toplevel);
    assert_eq!(resolved.main_root, git.main);
}

/// With a clean environment the real `discover` and the injected empty
/// environment agree, so the matrix above speaks for production.
#[test]
fn discover_matches_the_empty_environment_when_the_process_env_is_clean() {
    if !process_env_is_clean() {
        eprintln!("skipped: a GIT_* variable is set in this process");
        return;
    }
    let (_dir, root) = repo();
    let cwd = nested(&root);
    assert_eq!(discover(&cwd), discover_with_env(&cwd, &no_env));
}

// ── The pure pieces ───────────────────────────────────────────────────────────

#[test]
fn head_content_rules_mirror_git() {
    assert!(head_content_is_valid(b"ref: refs/heads/main\n"));
    assert!(head_content_is_valid(b"ref:refs/heads/main"));
    assert!(head_content_is_valid(b"ref: \t\r\n refs/x"));
    assert!(head_content_is_valid(
        b"0123456789abcdef0123456789abcdef01234567\n"
    ));
    assert!(head_content_is_valid(
        b"0123456789abcdef0123456789abcdef01234567junk"
    ));
    // 64 hex (sha256) starts with 40 hex.
    assert!(head_content_is_valid(&[b'a'; 64]));
    assert!(!head_content_is_valid(b""));
    assert!(!head_content_is_valid(b"ref: heads/main"));
    assert!(!head_content_is_valid(b"refs/heads/main"));
    assert!(!head_content_is_valid(
        b"0123456789abcdef0123456789abcdef0123456"
    ));
    assert!(!head_content_is_valid(
        b"0123456789abcdef0123456789abcdef0123456g"
    ));
    assert!(!head_content_is_valid(b"garbage\n"));
}

#[test]
fn format_scan_accepts_what_git_init_writes() {
    let scan = scan_format(
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\
         \tlogallrefupdates = true\n\tignorecase = true\n\tprecomposeunicode = true\n\
         [remote \"origin\"]\n\turl = git@github.com:a/b.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
         [branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n",
    );
    assert!(!scan.unparseable);
    assert!(!scan.unsupported_format);
    assert!(!scan.core_override);
    assert!(!scan.worktree_config);
}

#[test]
fn format_scan_flags_core_overrides_in_every_spelling() {
    for body in [
        "[core]\n\tbare = true\n",
        "[core]\n\tbare\n",
        "[core]\n\tBare = TRUE\n",
        "[core]\n\tbare = 1\n",
        "[core]\n\tbare = \"false\"\n",
        "[core]\n\tbare = false # comment\n",
        "[Core] bare = true\n",
        "[core]bare=true\n",
        "[core]\n\tworktree = /x\n",
        "[core]\n\tWorkTree = ../y\n",
        "[core]\n\tbare = false\r\n\tworktree = /x\r\n",
    ] {
        assert!(scan_format(body).core_override, "{body:?}");
    }
    for body in [
        "[core]\n\tbare = false\n",
        "[core]\n\tbare=false\n",
        "[core]\n\tbare = False\r\n",
        "\u{feff}[core]\n\tbare = false\n",
    ] {
        assert!(!scan_format(body).core_override, "{body:?}");
        assert!(!scan_format(body).unparseable, "{body:?}");
    }
}

#[test]
fn format_scan_flags_unsupported_formats() {
    assert!(scan_format("[core]\n\trepositoryformatversion = 2\n").unsupported_format);
    assert!(!scan_format("[core]\n\trepositoryformatversion = 1\n").unsupported_format);
    assert!(scan_format("[extensions]\n\tbogus = 1\n").unsupported_format);
    assert!(!scan_format("[extensions]\n\tworktreeConfig = true\n").unsupported_format);
    assert!(scan_format("[extensions]\n\tworktreeConfig = true\n").worktree_config);
    assert!(scan_format("[extensions]\n\tWORKTREECONFIG = false\n").worktree_config);
    assert!(!scan_format("[core]\n\tworktreeConfig = true\n").worktree_config);
}

#[test]
fn format_scan_declines_what_it_cannot_follow() {
    assert!(scan_format("bare = false\n").unparseable);
    assert!(scan_format("[core\n\tbare = false\n").unparseable);
    assert!(scan_format("[core]\n\tbare = false \\\n\tmore\n").unparseable);
    assert!(scan_format("[core]\n\t1bad = x\n").unparseable);
    assert!(scan_format("[core]\n\t= x\n").unparseable);
    assert!(!scan_format("[core]\n\t# bare = true\n\t; worktree = x\n").core_override);
    assert!(!scan_format("").unparseable);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            copy_dir(&entry.path(), &target);
        } else if ft.is_file() {
            // Skip an entry that disappears between `read_dir` and the copy.
            // Walking a live `.git` is inherently racy - git's own housekeeping
            // may repack objects underneath us - so a vanished entry is an
            // expected outcome here, not a failure. The fixtures also set
            // `gc.auto = 0` to remove the usual cause.
            if fs::copy(entry.path(), &target).is_err() {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let mode = meta.permissions().mode();
                let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode));
            }
        }
    }
}
