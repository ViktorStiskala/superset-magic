use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

/// Outcome of inspecting the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Main checkout, on the main branch. Allowed to write `.superset/`.
    Bootstrap { repo_root: PathBuf },
    /// Linked worktree, or main checkout on a non-main branch.
    /// Reads `main_checkout/.superset/setup_config.json` and copies files
    /// into `cwd_root`.
    Apply {
        cwd_root: PathBuf,
        main_checkout: PathBuf,
    },
    /// Unrecoverable state — e.g. detached HEAD in the main checkout or
    /// cwd outside any repository.
    Error(String),
}

/// Spawn `git` with the given args and capture the full `Output`. Used
/// by `git` and `git_optional`, which differ only in how they interpret
/// a non-zero exit.
fn git_raw(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.args(args).stdin(Stdio::null());
    cmd.output()
        .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))
}

/// Run `git` and return trimmed stdout. A non-zero exit is an error
/// carrying the verbatim stderr.
fn git(args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let out = git_raw(args, cwd)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("`git {}` failed: {stderr}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Same as `git` but treats a non-zero exit as `Ok(None)` — for queries
/// where a failed check is itself meaningful (e.g. detached HEAD).
fn git_optional(args: &[&str], cwd: Option<&Path>) -> Result<Option<String>> {
    let out = git_raw(args, cwd)?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// Resolve `path` against `base` if it's relative, then canonicalize.
fn resolve(path: &str, base: &Path) -> Result<PathBuf> {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    abs.canonicalize()
        .with_context(|| format!("could not canonicalize {}", abs.display()))
}

/// Working-tree root for `cwd` (the path of the linked worktree if `cwd` is
/// inside one, otherwise the main checkout root).
pub fn cwd_repo_root(cwd: &Path) -> Result<PathBuf> {
    let out = git(&["rev-parse", "--show-toplevel"], Some(cwd))?;
    resolve(&out, cwd)
}

/// True when `cwd_root` is a linked worktree rather than the main checkout.
/// Belt-and-suspenders: `<root>/.git` is a regular file in linked worktrees,
/// AND `git-dir` ≠ `git-common-dir`.
pub fn is_worktree(cwd_root: &Path) -> Result<bool> {
    let dot_git = cwd_root.join(".git");
    if dot_git.is_file() {
        return Ok(true);
    }
    let git_dir = git(&["rev-parse", "--git-dir"], Some(cwd_root))?;
    let common = git(&["rev-parse", "--git-common-dir"], Some(cwd_root))?;
    let git_dir = resolve(&git_dir, cwd_root)?;
    let common = resolve(&common, cwd_root)?;
    Ok(git_dir != common)
}

/// Filesystem root of the main checkout (the directory containing the
/// shared `.git`), derived from `git rev-parse --git-common-dir`.
pub fn main_checkout_root(cwd_root: &Path) -> Result<PathBuf> {
    let common = git(&["rev-parse", "--git-common-dir"], Some(cwd_root))?;
    let common = resolve(&common, cwd_root)?;
    let parent = common
        .parent()
        .with_context(|| format!("git-common-dir has no parent: {}", common.display()))?;
    parent
        .canonicalize()
        .with_context(|| format!("could not canonicalize {}", parent.display()))
}

/// Branch HEAD points at in `path`. Returns `None` on detached HEAD.
pub fn current_branch_in(path: &Path) -> Result<Option<String>> {
    git_optional(&["symbolic-ref", "--short", "HEAD"], Some(path))
}

/// `"main"` when a local `refs/heads/main` exists, else `"master"` when it
/// does, else an error.
pub fn main_branch_name(main_root: &Path) -> Result<String> {
    for candidate in ["main", "master"] {
        let ok = git_optional(
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{candidate}"),
            ],
            Some(main_root),
        )?
        .is_some();
        if ok {
            return Ok(candidate.to_string());
        }
    }
    bail!(
        "neither `main` nor `master` exists as a local branch in {}",
        main_root.display()
    )
}

/// Top-level decision: Bootstrap vs Apply vs Error.
pub fn probe(cwd: &Path) -> Result<Mode> {
    let repo_root = match cwd_repo_root(cwd) {
        Ok(r) => r,
        Err(_) => {
            return Ok(Mode::Error(format!(
                "not inside a git repository: {}",
                cwd.display()
            )))
        }
    };
    let worktree = is_worktree(&repo_root)?;
    let main_checkout = main_checkout_root(&repo_root)?;

    if worktree {
        return Ok(Mode::Apply {
            cwd_root: repo_root,
            main_checkout,
        });
    }

    let branch = match current_branch_in(&repo_root)? {
        Some(b) => b,
        None => return Ok(Mode::Error(
            "detached HEAD in the main checkout: cannot decide between bootstrap and apply mode"
                .to_string(),
        )),
    };
    let main_name = main_branch_name(&main_checkout)?;
    if branch == main_name {
        Ok(Mode::Bootstrap { repo_root })
    } else {
        Ok(Mode::Apply {
            cwd_root: repo_root,
            main_checkout,
        })
    }
}

// ---------------------------------------------------------------------------
// Mutating operations, used by the bootstrap final-action step.
// ---------------------------------------------------------------------------

/// `git add -- <p>` for each path in `paths` that exists on disk under
/// `repo_root`. Missing paths are silently skipped so the same call works
/// across "with .envrc" and "no .envrc" runs.
pub fn stage_paths(repo_root: &Path, paths: &[&str]) -> Result<()> {
    for p in paths {
        let abs = repo_root.join(p);
        if !abs.exists() {
            continue;
        }
        git(&["add", "--", p], Some(repo_root))?;
    }
    Ok(())
}

/// True when there are no staged changes (`git diff --cached --quiet`
/// exits 0).
pub fn nothing_to_commit(repo_root: &Path) -> Result<bool> {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(["diff", "--cached", "--quiet"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to spawn `git diff --cached`")?;
    Ok(status.success())
}

/// `git commit -m <msg>`.
pub fn commit(repo_root: &Path, msg: &str) -> Result<()> {
    git(&["commit", "-m", msg], Some(repo_root))?;
    Ok(())
}

/// `git push <remote> <branch>`, returning stdout (kept short).
pub fn push(repo_root: &Path, remote: &str, branch: &str) -> Result<String> {
    git(&["push", remote, branch], Some(repo_root))
}

/// `git push -u <remote> <branch>`.
pub fn push_upstream(repo_root: &Path, remote: &str, branch: &str) -> Result<String> {
    git(&["push", "-u", remote, branch], Some(repo_root))
}

/// `git switch -c <name>`.
pub fn create_branch(repo_root: &Path, name: &str) -> Result<()> {
    git(&["switch", "-c", name], Some(repo_root))?;
    Ok(())
}

/// Whether `gh` is installed and at least responds to `--version`.
/// Authentication state is not probed here — `gh pr create` will surface
/// its own auth error if any, which is more informative than a pre-check.
pub fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `gh pr create --fill --base <base>`. Returns the PR URL captured from
/// stdout.
pub fn pr_create(repo_root: &Path, base: &str) -> Result<String> {
    let out = Command::new("gh")
        .current_dir(repo_root)
        .args(["pr", "create", "--fill", "--base", base])
        .stdin(Stdio::null())
        .output()
        .context("failed to spawn `gh`")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("`gh pr create` failed: {err}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Shell out to `date +%Y%m%d-%H%M%S` for the feature-branch suffix.
/// Local timezone — matches what a developer would type by hand.
pub fn timestamp_branch_suffix() -> Result<String> {
    let out = Command::new("date")
        .arg("+%Y%m%d-%H%M%S")
        .stdin(Stdio::null())
        .output()
        .context("failed to spawn `date`")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("`date` failed: {err}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn run(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {}", cwd.display());
    }

    fn init_repo(initial_branch: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        run(&["init", "-q", "-b", initial_branch], dir.path());
        // empty commit so the branch exists as a ref
        fs::write(dir.path().join("README.md"), "hi").unwrap();
        run(&["add", "."], dir.path());
        run(&["commit", "-q", "-m", "init"], dir.path());
        dir
    }

    #[test]
    fn main_checkout_on_main_is_bootstrap() {
        let dir = init_repo("main");
        let root = dir.path().canonicalize().unwrap();
        match probe(&root).unwrap() {
            Mode::Bootstrap { repo_root } => assert_eq!(repo_root, root),
            other => panic!("expected Bootstrap, got {other:?}"),
        }
    }

    #[test]
    fn main_checkout_master_fallback_is_bootstrap() {
        let dir = init_repo("master");
        let root = dir.path().canonicalize().unwrap();
        match probe(&root).unwrap() {
            Mode::Bootstrap { repo_root } => assert_eq!(repo_root, root),
            other => panic!("expected Bootstrap, got {other:?}"),
        }
    }

    #[test]
    fn feature_branch_in_main_checkout_is_apply() {
        let dir = init_repo("main");
        run(&["switch", "-q", "-c", "feature/x"], dir.path());
        let root = dir.path().canonicalize().unwrap();
        match probe(&root).unwrap() {
            Mode::Apply {
                cwd_root,
                main_checkout,
            } => {
                assert_eq!(cwd_root, root);
                assert_eq!(main_checkout, root);
            }
            other => panic!("expected Apply, got {other:?}"),
        }
    }

    #[test]
    fn linked_worktree_is_apply() {
        let dir = init_repo("main");
        let wt = tempfile::tempdir().unwrap();
        let wt_path = wt.path().join("wt");
        run(
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature/wt",
                wt_path.to_str().unwrap(),
            ],
            dir.path(),
        );
        let main_root = dir.path().canonicalize().unwrap();
        let wt_root = wt_path.canonicalize().unwrap();
        match probe(&wt_root).unwrap() {
            Mode::Apply {
                cwd_root,
                main_checkout,
            } => {
                assert_eq!(cwd_root, wt_root);
                assert_eq!(main_checkout, main_root);
            }
            other => panic!("expected Apply, got {other:?}"),
        }
    }

    #[test]
    fn detached_head_in_main_checkout_is_error() {
        let dir = init_repo("main");
        let sha = git(&["rev-parse", "HEAD"], Some(dir.path())).unwrap();
        run(&["checkout", "-q", "--detach", &sha], dir.path());
        let root = dir.path().canonicalize().unwrap();
        match probe(&root).unwrap() {
            Mode::Error(msg) => assert!(msg.contains("detached"), "msg: {msg}"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn outside_repo_is_error() {
        let dir = tempfile::tempdir().unwrap();
        match probe(dir.path()).unwrap() {
            Mode::Error(msg) => assert!(msg.contains("not inside a git repository")),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
