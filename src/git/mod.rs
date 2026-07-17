// Git plumbing: probes + mutating primitives (this file) and .gitignore
// helpers (gitignore submodule).
pub(crate) mod gitignore;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

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

/// URL of the `origin` remote, or `None` when no origin is configured
/// (remote config is shared, so this works from linked worktrees too).
pub fn origin_url(cwd_root: &Path) -> Result<Option<String>> {
    git_optional(&["remote", "get-url", "origin"], Some(cwd_root))
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

// ---------------------------------------------------------------------------
// Mutating operations, used by the migration/init final-action step.
// ---------------------------------------------------------------------------

/// `git add -- <p>` for each path in `paths` that exists on disk under
/// `repo_root`. Missing paths are silently skipped so the same call works
/// whether or not optional paths are present.
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

// ---------------------------------------------------------------------------
// Read-only probes used by reverse sync (U11).
// ---------------------------------------------------------------------------

/// Repo-relative paths of files that are git-UNTRACKED in the working tree
/// rooted at `repo_root`, optionally restricted to `pathspecs`.
///
/// Runs `git ls-files --others -z -- [pathspecs…]`, which lists untracked
/// files — INCLUDING gitignored ones — while excluding tracked files. When
/// `pathspecs` is empty the whole working tree is listed; when it carries
/// paths, git reports only the untracked files matching those pathspecs. For
/// exact file paths that is an index lookup, so git does NOT descend into
/// unrelated gitignored directories (`target/`, `node_modules/`) — the caller
/// passes its already-matched paths to keep this off the hot path.
///
/// Including gitignored files is deliberate and load-bearing: reverse sync
/// pushes untracked SECRETS (`.env`, `.dev.vars`, the gitignored
/// `magic.local.json`), and those are gitignored by definition. Adding
/// `--exclude-standard` here would drop exactly the files reverse sync must
/// find, making the candidate set always empty in any repo that gitignores its
/// secrets. `git ls-files` reports files, never directory entries — even a
/// directory pathspec expands to the untracked files within it.
///
/// Output is one repo-relative path per entry, in git's own porcelain form
/// (forward slashes, relative to `repo_root`). Empty output → empty vector.
/// Paths with a leading `..` or that are absolute are defensively dropped —
/// `git ls-files` never emits such paths from the repo root, but reverse
/// sync intersects this set with the (already-validated) pattern matcher and
/// must never let a path escape the tree.
// consumed by U11 (reverse sync candidates); wired into the menu by U10
pub fn untracked_files(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<PathBuf>> {
    let mut args: Vec<&str> = vec!["ls-files", "--others", "-z", "--"];
    args.extend_from_slice(pathspecs);
    let out = git(&args, Some(repo_root))?;
    // `-z` gives NUL-separated, unquoted paths so filenames with spaces or
    // special characters survive intact (the default output quotes them).
    // `git`'s trim() above strips a trailing NUL, so split on NUL and drop
    // any empty trailing segment.
    let mut paths = Vec::new();
    for raw in out.split('\0') {
        if raw.is_empty() {
            continue;
        }
        let p = PathBuf::from(raw);
        if p.is_absolute() || raw.split('/').any(|seg| seg == "..") {
            // Defensive: git never emits these from the repo root.
            continue;
        }
        paths.push(p);
    }
    Ok(paths)
}

/// True when `rel` (a repo-relative path) is gitignored in the working tree
/// rooted at `repo_root`. Runs `git check-ignore -q -- <rel>`: exit 0 means
/// ignored, exit 1 means not ignored, any other exit is a real error.
///
/// Used by reverse sync to decide whether a copied path is already covered
/// by main's `.gitignore` before appending a new rule.
// consumed by U11
pub fn is_ignored(repo_root: &Path, rel: &Path) -> Result<bool> {
    let rel_str = rel
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?;
    is_ignored_str(repo_root, rel_str)
}

/// Raw-pathname variant of [`is_ignored`] so a caller can force git's
/// directory-only match with a trailing slash: a `foo/bar/` rule matches the
/// query `foo/bar/` even before the directory exists on disk, whereas `foo/bar`
/// (no slash, dir absent) is treated as a file and MISSES it. Exit 0 → ignored,
/// exit 1 → not ignored, any other exit is a real error.
pub fn is_ignored_str(repo_root: &Path, rel_str: &str) -> Result<bool> {
    let out = git_raw(&["check-ignore", "-q", "--", rel_str], Some(repo_root))?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            bail!("`git check-ignore -- {rel_str}` failed: {stderr}");
        }
    }
}

/// Git-TRACKED files among `pathspecs`, via `git ls-files --cached -z --`. The
/// mirror of [`untracked_files`], used for POSITIVE tracked determination in the
/// reverse-sync secret gate — a path NOT in this set is treated as untracked (a
/// secret), so an unenumerable / oddly-normalized name fails closed. Output is
/// repo-relative porcelain paths; leading-`..`/absolute paths are defensively
/// dropped. Empty `pathspecs` list the WHOLE index, so callers should skip the
/// probe when there are no matches.
pub fn tracked_files(repo_root: &Path, pathspecs: &[&str]) -> Result<Vec<PathBuf>> {
    let mut args: Vec<&str> = vec!["ls-files", "--cached", "-z", "--"];
    args.extend_from_slice(pathspecs);
    let out = git(&args, Some(repo_root))?;
    let mut paths = Vec::new();
    for raw in out.split('\0') {
        if raw.is_empty() {
            continue;
        }
        let p = PathBuf::from(raw);
        if p.is_absolute() || raw.split('/').any(|seg| seg == "..") {
            // Defensive: git never emits these from the repo root.
            continue;
        }
        paths.push(p);
    }
    Ok(paths)
}

/// Resolve the `.gitignore` rule that COVERS `rel` in the working tree rooted
/// at `repo_root`, via `git check-ignore -v --no-index -- <rel>`.
///
/// Returns `Ok(Some(pattern))` when a rule matches (the bare pattern text,
/// e.g. `**/.dev.vars`), `Ok(None)` when no rule covers the path.
///
/// `-v` prints `<source>:<line>:<pattern>\t<pathname>`. We parse the pattern
/// out of the colon-delimited prefix before the tab. `--no-index` makes the
/// check independent of whether the path is tracked, so a covering rule is
/// found even for a path that git would otherwise consider already tracked.
///
/// A leading `!` (negation) pattern means the path is explicitly *un*-ignored;
/// such a match is reported as `None` so the caller falls back to the literal
/// path rather than copying a negation rule into main.
// consumed by U11 (gitignore::find_covering_rule)
pub fn check_ignore_pattern(repo_root: &Path, rel: &Path) -> Result<Option<String>> {
    let rel_str = rel
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?;
    let out = git_raw(
        &["check-ignore", "-v", "--no-index", "--", rel_str],
        Some(repo_root),
    )?;
    match out.status.code() {
        // 0 → matched; parse the pattern. 1 → no match. Other → error.
        Some(1) => return Ok(None),
        Some(0) => {}
        _ => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            bail!("`git check-ignore -v --no-index -- {rel_str}` failed: {stderr}");
        }
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().unwrap_or("").trim_end();
    Ok(parse_check_ignore_line(line))
}

/// Parse one `git check-ignore -v` line into its bare pattern.
///
/// Format: `<source>:<line>:<pattern>\t<pathname>`. The source path may itself
/// contain colons, so we split off the trailing `\t<pathname>` first, then
/// take everything AFTER the second colon as the pattern (so a pattern that
/// contains colons survives). A blank pattern (no matching gitignore source —
/// e.g. a command-line `-e` exclude renders as `::pattern`) still parses. A
/// negation pattern (`!…`) is reported as `None`.
fn parse_check_ignore_line(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    // Strip the trailing `\t<pathname>` if present.
    let prefix = line.split('\t').next().unwrap_or(line);
    // `<source>:<line>:<pattern>` — find the first two colons. The pattern is
    // everything after the second colon (it may contain further colons).
    let first = prefix.find(':')?;
    let rest = &prefix[first + 1..];
    let second = rest.find(':')?;
    let pattern = rest[second + 1..].trim();
    if pattern.is_empty() || pattern.starts_with('!') {
        return None;
    }
    Some(pattern.to_string())
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
mod tests;
