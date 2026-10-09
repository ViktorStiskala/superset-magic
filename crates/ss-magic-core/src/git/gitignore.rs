//! Helpers for managing `.gitignore` at a git repository root.
//!
//! All helpers follow the convention: never reorder or rewrite existing
//! content; only append when the exact line is missing; create the file
//! if absent.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::git;

/// Whether a gitignore rule targets a file or a directory. A directory is
/// queried with a trailing slash so git matches a `foo/bar/` rule even before
/// the directory exists on disk (see [`git::is_ignored_str`]); a directory rule
/// is written with a trailing slash too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// A regular file.
    File,
    /// A directory (queried/written with a trailing slash).
    Dir,
}

/// Where a new ignore rule is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreSink {
    /// A tracked-style `.gitignore` (the committed-install behavior, exactly
    /// [`ensure_path_ignored`]).
    Gitignore,
    /// The repository's shared, untracked `<git-common-dir>/info/exclude`, so a
    /// local install leaves `git status` clean.
    LocalExclude,
}

/// The outcome of [`ensure_path_ignored`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// git already ignored the path — nothing was written.
    Already,
    /// A rule was appended to a `.gitignore` (or, for the `LocalExclude` sink,
    /// to `info/exclude`).
    Appended,
}

/// Append `line` to `<git_root>/.gitignore` if no EXACT line match already
/// exists.  Creates `.gitignore` if the file is absent.
///
/// - Existing content is NEVER reordered or rewritten.
/// - The appended entry is placed on its own line.
/// - A single trailing newline is preserved: if the file's last byte is
///   already `\n` the entry is appended directly; otherwise a newline is
///   inserted before it.
// The append primitive behind [`ensure_path_ignored`]; also called directly by
// callers that already know the exact rule text.
pub fn ensure_entry(git_root: &Path, line: &str) -> Result<()> {
    append_line(&git_root.join(".gitignore"), line)
}

/// The append body shared by [`ensure_entry`] (a `.gitignore`) and the
/// local-exclude writer (`info/exclude`): add `line` to the file at `path` when
/// no exact line match exists, create the file when absent, and never reorder or
/// rewrite existing content.
fn append_line(path: &Path, line: &str) -> Result<()> {
    if path.exists() {
        let contents =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

        // Exact line match: any line in the file equals `line` after stripping
        // the trailing newline.
        let already_present = contents.lines().any(|l| l == line);
        if already_present {
            return Ok(());
        }

        // Append: ensure the new entry starts on its own line.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .with_context(|| format!("opening {} for append", path.display()))?;

        // If the file is non-empty and its last byte is not a newline,
        // emit one first so the entry starts on its own line.
        if !contents.is_empty() && !contents.ends_with('\n') {
            file.write_all(b"\n")
                .with_context(|| format!("writing {}", path.display()))?;
        }
        writeln!(file, "{}", line).with_context(|| format!("writing {}", path.display()))?;
    } else {
        // File absent — create it with just the entry + trailing newline.
        fs::write(path, format!("{}\n", line))
            .with_context(|| format!("creating {}", path.display()))?;
    }

    Ok(())
}

/// A `.gitignore` rule that covers a path, together with the directory that
/// OWNS the file defining it. A rule's meaning depends on where its
/// `.gitignore` sits (a bare `*` in `.scratchpad/.gitignore` ignores only
/// `.scratchpad/`'s subtree; the same `*` at a repo root ignores everything),
/// so [`ensure_path_ignored`] uses `source_dir` to decide whether copying the
/// pattern into another tree would silently widen its scope (R1).
struct CoveringRule {
    /// The bare pattern text (e.g. `**/.dev.vars`, or `*`).
    pattern: String,
    /// The directory, relative to the tree root, holding the `.gitignore`
    /// that defined the rule (empty for the tree root itself). `None` when
    /// the rule comes from outside the tracked tree (e.g. `.git/info/exclude`
    /// or a global excludesfile) — there is no "owning directory" to
    /// re-anchor at, so the caller must fall back to the anchored literal.
    source_dir: Option<PathBuf>,
}

/// Resolve the `.gitignore` rule in the working tree rooted at `worktree_root`
/// that COVERS `rel`, for copying into another checkout's `.gitignore`.
///
/// Returns `Ok(Some(rule))` when a rule matches, and `Ok(None)` when NO rule
/// covers `rel`. The caller ([`ensure_path_ignored`]) reuses `rule.pattern`
/// only when it would land at the same `rule.source_dir` it already owns
/// here, and falls back to the literal relative path otherwise — so a
/// reverse-synced secret lands gitignored in main even if the worktree relied
/// on an inherited glob, without ever lifting a nested rule to a broader
/// scope than it had.
///
/// Shells out to `git check-ignore -v --no-index` directly (rather than
/// hand-parsing `.gitignore` files) so nested `.gitignore`s, inherited rules,
/// and git's full match semantics are respected, AND the `<source>` half of
/// the `-v` output — the owning file — is recovered along with the pattern.
/// A negation rule (`!…`) is reported as `None`.
// consumed by reverse sync (gitignore safety), reachable via the worktree menu
fn find_covering_rule(worktree_root: &Path, rel: &Path) -> Result<Option<CoveringRule>> {
    let rel_str = rel
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?;
    let out = super::git_raw(
        &["check-ignore", "-v", "--no-index", "--", rel_str],
        Some(worktree_root),
    )?;
    match out.status.code() {
        // 0 → matched; parse the pattern + source. 1 → no match. Other → error.
        Some(1) => return Ok(None),
        Some(0) => {}
        _ => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            bail!("`git check-ignore -v --no-index -- {rel_str}` failed: {stderr}");
        }
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().unwrap_or("").trim_end();
    Ok(parse_covering_line(line))
}

/// Parse one `git check-ignore -v --no-index` line (run with the tree root as
/// cwd) into the owning `.gitignore`'s directory and the bare pattern it
/// defines.
///
/// Format: `<source>:<line-no>:<pattern>\t<pathname>`. The pattern is
/// everything after the SECOND colon (so a pattern containing colons
/// survives); the `<source>` half — the path to the `.gitignore` file,
/// relative to the tree root since that's the cwd git ran with — is kept too,
/// so the caller can tell WHICH file defined the rule. A blank pattern or a
/// negation (`!…`) is reported as `None`. A `source` outside the tree (an
/// absolute path, e.g. a global excludesfile, or `.git/info/exclude`) has no
/// meaningful owning directory within the tree and is reported as
/// `source_dir: None`.
fn parse_covering_line(line: &str) -> Option<CoveringRule> {
    if line.is_empty() {
        return None;
    }
    let prefix = line.split('\t').next().unwrap_or(line);
    let first = prefix.find(':')?;
    let source = &prefix[..first];
    let rest = &prefix[first + 1..];
    let second = rest.find(':')?;
    let pattern = rest[second + 1..].trim();
    if pattern.is_empty() || pattern.starts_with('!') {
        return None;
    }
    let source_path = Path::new(source);
    let source_dir = if source_path.is_absolute() || source.starts_with(".git/") {
        None
    } else {
        Some(
            source_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf(),
        )
    };
    Some(CoveringRule {
        pattern: pattern.to_string(),
        source_dir,
    })
}

/// Ensure `rel` (of `kind`) is gitignored under `target_root`, adding a rule
/// only when git does not already ignore it. The new rule lands in the closest
/// EXISTING `.gitignore` among `rel`'s ancestor directories (else `target_root`),
/// preferring a covering glob resolved from `rule_source_root` (verified to
/// actually ignore `rel`, and reused ONLY when it would land at the same
/// relative directory that owns it in `rule_source_root` — see R1) over an
/// anchored literal. The single gitignore entry point shared by reverse sync
/// (the secret-safety boundary), the backups dir, and the migrate/init
/// bootstrap.
///
/// git-TOLERANT: when git itself fails (a non-git root, e.g. a unit-test
/// tempdir), the "already ignored?" probe reads `None` and the literal is
/// written anyway, so bootstrap still seeds a `.gitignore`. A hard secret
/// boundary re-checks STRICTLY on top of this (see
/// [`crate::sync::reverse_sync`]'s `ensure_gitignored_in_main`), so a git error
/// there fails the push rather than trusting this tolerant path.
pub fn ensure_path_ignored(
    target_root: &Path,
    rule_source_root: &Path,
    rel: &Path,
    kind: PathKind,
) -> Result<Ignored> {
    if is_ignored_opt(target_root, rel, kind) == Some(true) {
        return Ok(Ignored::Already);
    }
    let gi_dir = closest_gitignore_dir(target_root, rel);

    // Prefer the source tree's covering glob (it generalizes protection, e.g.
    // `**/.dev.vars`) — but ONLY when it would land at the SAME relative
    // directory it already owns in the source tree, and only trust it after
    // verifying it now ignores `rel`. A rule's meaning depends on where its
    // `.gitignore` sits, so copying it to a DIFFERENT (broader) directory
    // than `gi_dir` would silently widen its scope — e.g. a bare `*` scoped
    // to a nested `.scratchpad/.gitignore` would ignore the ENTIRE repo if
    // lifted to the target's root just because the target has no matching
    // nested file (R1). When the scopes don't line up, fall through to the
    // anchored literal below instead.
    if let Some(rule) = find_covering_rule(rule_source_root, rel).ok().flatten() {
        let gi_dir_rel = gi_dir.strip_prefix(target_root).unwrap_or(Path::new(""));
        if rule.source_dir.as_deref() == Some(gi_dir_rel) {
            ensure_entry(&gi_dir, &rule.pattern)?;
            if is_ignored_opt(target_root, rel, kind) == Some(true) {
                return Ok(Ignored::Appended);
            }
        }
    }

    // Anchored literal in the closest `.gitignore`.
    ensure_entry(&gi_dir, &anchored_literal(target_root, &gi_dir, rel, kind)?)?;
    // If a nested `.gitignore` still does not cover it (git available but the
    // nested anchor did not take), add a root-anchored literal as a last resort.
    if gi_dir != target_root && is_ignored_opt(target_root, rel, kind) == Some(false) {
        ensure_entry(target_root, &anchored_literal(target_root, target_root, rel, kind)?)?;
    }
    Ok(Ignored::Appended)
}

/// Probe whether `rel` (of `kind`) is ignored under `root`, returning `None`
/// when git itself fails (a non-git root) so [`ensure_path_ignored`] can degrade
/// to a literal append. A `Dir` is queried with a trailing slash so a `foo/bar/`
/// rule matches before the directory exists on disk.
fn is_ignored_opt(root: &Path, rel: &Path, kind: PathKind) -> Option<bool> {
    git::is_ignored_str(root, &with_dir_slash(rel.to_str()?, kind)).ok()
}

/// `path` with a trailing `/` added when `kind` is [`PathKind::Dir`] and it
/// does not already end in one, else `path` unchanged. The one spelling rule
/// for a directory, shared by the probes (git matches a directory-only `foo/`
/// rule against a slash-terminated query even before the directory exists) and
/// by the `.gitignore` rule written for it.
fn with_dir_slash(path: &str, kind: PathKind) -> String {
    match kind {
        PathKind::Dir if !path.ends_with('/') => format!("{path}/"),
        _ => path.to_string(),
    }
}

/// The deepest ancestor directory of `rel` (under `target_root`) that already
/// holds a `.gitignore`, or `target_root` itself when none do — so a nested
/// `.gitignore` keeps ownership of its subtree instead of a rule leaking to the
/// repo root.
fn closest_gitignore_dir(target_root: &Path, rel: &Path) -> PathBuf {
    let mut best = target_root.to_path_buf();
    let mut cur = target_root.to_path_buf();
    let parent = rel.parent().unwrap_or_else(|| Path::new(""));
    for comp in parent.components() {
        if let std::path::Component::Normal(seg) = comp {
            cur = cur.join(seg);
            if cur.join(".gitignore").is_file() {
                best = cur.clone();
            }
        }
    }
    best
}

/// The literal gitignore rule for `rel` (of `kind`) to write into the
/// `.gitignore` at `gi_dir`. When `gi_dir == target_root` the rule is the
/// repo-relative path verbatim — anchored at the root only when `rel` itself
/// contains a `/`; a bare top-level filename (e.g. `.env`) is intentionally left
/// UNANCHORED (matches at any depth, like `**/.env`), which is safe-directioned
/// for secret protection (only widens the ignore scope). When `gi_dir` is
/// nested, the rule is `/`-anchored and relative to `gi_dir`. A `Dir` gains a
/// trailing slash.
fn anchored_literal(
    target_root: &Path,
    gi_dir: &Path,
    rel: &Path,
    kind: PathKind,
) -> Result<String> {
    let base = if gi_dir == target_root {
        rel.to_str()
            .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?
            .to_string()
    } else {
        let gi_rel = gi_dir.strip_prefix(target_root).unwrap_or(gi_dir);
        let sub = rel.strip_prefix(gi_rel).unwrap_or(rel);
        let sub_str = sub
            .to_str()
            .with_context(|| format!("non-UTF-8 path: {}", sub.display()))?;
        format!("/{sub_str}")
    };
    Ok(with_dir_slash(&base, kind))
}

/// Ensure `rel` (of `kind`) is ignored under `target_root`, writing any new rule
/// to the place `sink` names.
///
/// - [`IgnoreSink::Gitignore`] is exactly [`ensure_path_ignored`] (the committed
///   install's behavior), including its git-tolerant degradation and its use of
///   `rule_source_root` to copy a covering glob.
/// - [`IgnoreSink::LocalExclude`] writes to `<git-common-dir>/info/exclude`
///   instead, so a local install leaves every tracked file and `git status`
///   untouched (KTD3: git reads that one untracked file for the main checkout
///   and for every linked worktree). It never copies a glob from
///   `rule_source_root` (which it ignores): a rule written to a shared file must
///   not depend on which checkout it was resolved from, so the new rule is
///   always the root-anchored literal for `rel`. It is git-STRICT, unlike the
///   `Gitignore` sink: a git failure (no common dir, an unanswerable probe) is an
///   error, never a guess, because there is no non-git local install to degrade
///   for.
///
/// The `LocalExclude` writer probes git first and skips the write when the path
/// is already ignored; otherwise it appends the rule (creating `info/` and the
/// file when missing, never reordering) and then asks git AGAIN. `info/exclude`
/// has LOWER precedence than any tracked `.gitignore`, so a tracked negation such
/// as `!/.superset/backups/` can still win; when the re-check says the path is
/// not ignored the function errors naming the path, turning what would be a
/// silent leak of a secret into a loud failure.
pub fn ensure_path_ignored_in(
    sink: IgnoreSink,
    target_root: &Path,
    rule_source_root: &Path,
    rel: &Path,
    kind: PathKind,
) -> Result<Ignored> {
    match sink {
        IgnoreSink::Gitignore => ensure_path_ignored(target_root, rule_source_root, rel, kind),
        IgnoreSink::LocalExclude => ensure_in_local_exclude(target_root, rel, kind),
    }
}

/// The [`IgnoreSink::LocalExclude`] writer behind [`ensure_path_ignored_in`].
fn ensure_in_local_exclude(target_root: &Path, rel: &Path, kind: PathKind) -> Result<Ignored> {
    let rel_str = rel
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", rel.display()))?;
    // Probe with the same trailing-slash convention the rule is written with.
    let probe = with_dir_slash(rel_str, kind);
    if git::is_ignored_str(target_root, &probe)? {
        return Ok(Ignored::Already);
    }

    let rule = exclude_literal(rel_str, kind)?;
    let info_dir = git::git_common_dir(target_root)?.join("info");
    fs::create_dir_all(&info_dir).with_context(|| format!("creating {}", info_dir.display()))?;
    append_line(&info_dir.join("exclude"), &rule)?;

    if !git::is_ignored_str(target_root, &probe)? {
        bail!(
            "wrote `{rule}` to {} but git still does not ignore `{rel_str}` – a tracked \
             .gitignore rule (for example a `!` negation) takes precedence over info/exclude",
            info_dir.join("exclude").display()
        );
    }
    Ok(Ignored::Appended)
}

/// The root-anchored literal `info/exclude` rule for `rel`: a leading `/`, the
/// path with gitignore glob metacharacters (`\`, `*`, `?`, `[`) and a trailing
/// space backslash-escaped so the rule matches exactly that path, and a trailing
/// slash for a directory. A name containing a newline cannot be one line of the
/// file and is refused rather than split into two rules.
fn exclude_literal(rel: &str, kind: PathKind) -> Result<String> {
    if rel.contains('\n') || rel.contains('\r') {
        bail!("cannot write an ignore rule for a path containing a line break: {rel:?}");
    }
    let rel = rel.trim_end_matches('/');
    let mut out = String::with_capacity(rel.len() + 2);
    out.push('/');
    for c in rel.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            out.push('\\');
        }
        out.push(c);
    }
    if out.ends_with(' ') {
        // git strips unescaped trailing spaces from a rule.
        out.pop();
        out.push_str("\\ ");
    }
    if kind == PathKind::Dir {
        out.push('/');
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
