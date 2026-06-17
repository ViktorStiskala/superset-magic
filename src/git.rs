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

// ---------------------------------------------------------------------------
// Read-only probes used by reverse sync (U11).
// ---------------------------------------------------------------------------

/// Repo-relative paths of files that are git-UNTRACKED in the working tree
/// rooted at `repo_root`. Runs `git ls-files --others --exclude-standard`,
/// which lists untracked files honoring `.gitignore`/exclude rules (so an
/// ignored, untracked file is NOT returned) but excludes tracked files.
///
/// Output is one repo-relative path per line, in git's own porcelain form
/// (forward slashes, relative to `repo_root`). Empty output → empty vector.
/// Paths with a leading `..` or that are absolute are defensively dropped —
/// `git ls-files` never emits such paths from the repo root, but reverse
/// sync intersects this set with the (already-validated) pattern matcher and
/// must never let a path escape the tree.
// consumed by U11 (reverse sync candidates); wired into the menu by U10
pub fn untracked_files(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let out = git(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        Some(repo_root),
    )?;
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

/// Run `git diff --no-index` between two absolute paths and stream the
/// (colored) output through the user's pager. `--no-index` lets git diff two
/// arbitrary files outside any repo; `--color=always` forces color even when
/// piped. The pager is `$PAGER` or `less -R` so a large diff isn't buried by
/// the picker's re-render. A "files differ" exit (1) is NOT an error — git
/// diff exits 1 when the inputs differ.
///
/// This is a TUI/manual-smoke path: it inherits the terminal and blocks until
/// the pager is dismissed. It is not unit-tested (consistent with the repo's
/// final-action convention).
// consumed by U11 (reverse-sync picker "show diff" action)
pub fn diff_no_index_paged(left: &Path, right: &Path) -> Result<()> {
    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());

    // `git diff --no-index --color=always <left> <right> | $PAGER`. Wire the
    // pipeline through `git`'s own `core.pager` would require config; spawn
    // the two processes explicitly so we control the pager and force color.
    let mut diff = Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--color=always",
            "--",
            left.to_str()
                .with_context(|| format!("non-UTF-8 path: {}", left.display()))?,
            right
                .to_str()
                .with_context(|| format!("non-UTF-8 path: {}", right.display()))?,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to spawn `git diff --no-index`")?;

    let diff_out = diff
        .stdout
        .take()
        .context("git diff produced no stdout pipe")?;

    // Run the pager via the shell so `$PAGER` may carry arguments (`less -R`).
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut pager_child = Command::new(shell)
        .arg("-c")
        .arg(&pager)
        .stdin(Stdio::from(diff_out))
        .spawn()
        .with_context(|| format!("failed to spawn pager `{pager}`"))?;

    // Wait for both: the pager drains stdout, git diff exits 0/1.
    let _ = pager_child.wait();
    let _ = diff.wait();
    Ok(())
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
            // Isolate from machine-level git config (e.g. commit.gpgsign=true)
            // so commits don't intermittently fail on a slow/absent gpg agent.
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false")
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

    // ── Reverse-sync probes (U11) ───────────────────────────────────────────

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// `untracked_files` lists untracked-but-not-ignored files and excludes
    /// both tracked files and gitignored files.
    #[test]
    fn untracked_files_lists_only_untracked_unignored() {
        let dir = init_repo("main");
        let root = dir.path();
        // tracked file (README.md committed by init_repo).
        // untracked, not ignored:
        write(root, "apps/api/.dev.vars", "SECRET=1\n");
        write(root, "new.txt", "hi\n");
        // untracked, but ignored:
        write(root, ".gitignore", "ignored.txt\n");
        write(root, "ignored.txt", "nope\n");
        // The .gitignore itself is untracked here, so it would also show up.
        // Track it to keep the assertion focused on the data files.
        run(&["add", ".gitignore"], root);

        let mut got: Vec<String> = untracked_files(root)
            .unwrap()
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        got.sort();

        assert!(
            got.contains(&"apps/api/.dev.vars".to_string()),
            "untracked secret must be listed; got {got:?}"
        );
        assert!(got.contains(&"new.txt".to_string()), "got {got:?}");
        assert!(
            !got.contains(&"README.md".to_string()),
            "tracked file must NOT be listed; got {got:?}"
        );
        assert!(
            !got.contains(&"ignored.txt".to_string()),
            "gitignored file must NOT be listed; got {got:?}"
        );
    }

    /// `is_ignored` reports true for a gitignored path, false otherwise.
    #[test]
    fn is_ignored_reflects_gitignore() {
        let dir = init_repo("main");
        let root = dir.path();
        write(root, ".gitignore", "**/.dev.vars\n");
        assert!(is_ignored(root, Path::new("apps/api/.dev.vars")).unwrap());
        assert!(!is_ignored(root, Path::new("apps/api/config.json")).unwrap());
    }

    /// `check_ignore_pattern` returns the covering glob, not the literal path,
    /// when a glob rule matches.
    #[test]
    fn check_ignore_pattern_returns_covering_glob() {
        let dir = init_repo("main");
        let root = dir.path();
        write(root, ".gitignore", "**/.dev.vars\n");
        let pat = check_ignore_pattern(root, Path::new("apps/api/.dev.vars")).unwrap();
        assert_eq!(pat, Some("**/.dev.vars".to_string()));
    }

    /// `check_ignore_pattern` returns the literal pattern when the rule is an
    /// exact path entry.
    #[test]
    fn check_ignore_pattern_returns_literal_when_exact() {
        let dir = init_repo("main");
        let root = dir.path();
        write(root, ".gitignore", "secrets/api.key\n");
        let pat = check_ignore_pattern(root, Path::new("secrets/api.key")).unwrap();
        assert_eq!(pat, Some("secrets/api.key".to_string()));
    }

    /// `check_ignore_pattern` returns None when no rule covers the path.
    #[test]
    fn check_ignore_pattern_none_when_no_rule() {
        let dir = init_repo("main");
        let root = dir.path();
        write(root, ".gitignore", "node_modules/\n");
        let pat = check_ignore_pattern(root, Path::new("apps/api/.dev.vars")).unwrap();
        assert_eq!(pat, None);
    }

    /// A negation rule (`!…`) is reported as None, so callers fall back to the
    /// literal path rather than copying a negation into main.
    #[test]
    fn check_ignore_pattern_negation_is_none() {
        let dir = init_repo("main");
        let root = dir.path();
        // Ignore everything, then un-ignore the secret — git check-ignore -v
        // reports the LAST matching rule, which is the negation.
        write(root, ".gitignore", "**/.dev.vars\n!apps/api/.dev.vars\n");
        let pat = check_ignore_pattern(root, Path::new("apps/api/.dev.vars")).unwrap();
        assert_eq!(pat, None, "negation rule must not be copied as a pattern");
    }

    /// Pure parser unit: pattern after the second colon, before the tab.
    #[test]
    fn parse_check_ignore_line_extracts_pattern() {
        let line = ".gitignore:1:**/.dev.vars\tapps/api/.dev.vars";
        assert_eq!(
            parse_check_ignore_line(line),
            Some("**/.dev.vars".to_string())
        );
    }

    #[test]
    fn parse_check_ignore_line_handles_negation_and_empty() {
        assert_eq!(
            parse_check_ignore_line(".gitignore:2:!foo\tfoo"),
            None,
            "negation → None"
        );
        assert_eq!(parse_check_ignore_line(""), None);
    }
}
