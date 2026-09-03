//! The `<repo>` name stem: what this repository is called, derived from git
//! alone.
//!
//! Two consumers must never disagree about it. The pack engine names its
//! archive `ss-magic-<stem>.tar.bz2`, and the Claude Code plugin keys a
//! worktree's session directory on `<stem>-<branch>`. Both call
//! [`repo_name_stem`]; the CLI's `pack` module re-exports it so its callers
//! are unchanged by the move out of `pack.rs`.

use std::path::Path;

use crate::git;

/// Derive a repo-name stem for `root`: the `origin` remote when one is
/// configured (normalized so every URL form of the same repo yields the same
/// name — see [`stem_from_origin`]), falling back to the primary (main)
/// worktree directory's basename, sanitized the same way, otherwise. Returns
/// `None` when neither source yields usable characters (no origin AND a
/// basename that itself sanitizes to empty), leaving the choice of last-resort
/// fallback to the caller — `archive_file_name` falls back further to the
/// literal `files` stem; the plugin's `identity` module (KTD12) falls back to
/// its next identity source instead.
pub fn repo_name_stem(root: &Path) -> Option<String> {
    git::origin_url(root)
        .ok()
        .flatten()
        .and_then(|url| stem_from_origin(&url))
        .or_else(|| {
            let main_root = git::main_checkout_root(root).unwrap_or_else(|_| root.to_path_buf());
            main_root
                .file_name()
                .map(|n| sanitize_segment(&n.to_string_lossy()))
                .filter(|s| !s.is_empty())
        })
}

/// Normalize a git remote URL into a filename stem.
///
/// Strips the scheme (`https://`, `ssh://`, `git://`, …), userinfo, host, and
/// port; drops a trailing `.git`; lowercases; maps every non-alphanumeric run
/// inside a path segment to a single `-`; joins path segments with `_`. The
/// scp-like form (`git@host:owner/repo`) and a scheme form of the same remote
/// produce identical stems, and nested paths (GitLab groups) keep every
/// segment: `gitlab.com/group/sub/repo` → `group_sub_repo`. A local-path
/// origin — bare (`/srv/git/repo.git`) or `file://` — contributes only its
/// final segment, so local directory hierarchies never leak into the archive
/// name. Returns `None` when nothing usable remains.
pub fn stem_from_origin(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    // Split off the scheme, then the host: with a scheme the host ends at the
    // first `/`; without one, the scp-like `[user@]host:path` uses the first
    // `:`. A `file://` or bare local path has no host — only the basename
    // identifies the repo (the rest is local filesystem hierarchy).
    let path = if let Some(rest) = url.strip_prefix("file://") {
        final_segment(rest)
    } else if let Some(i) = url.find("://") {
        url[i + 3..].split_once('/')?.1
    } else if let Some((_host, path)) = url.split_once(':') {
        path
    } else {
        final_segment(url)
    };
    let trimmed = path.trim_matches('/');
    let path = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    let segments: Vec<String> = path
        .split('/')
        .map(sanitize_segment)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("_"))
}

/// Final `/`-separated segment of a path-like string (the whole string when
/// it contains no `/`).
fn final_segment(s: &str) -> &str {
    s.rfind('/').map_or(s, |i| &s[i + 1..])
}

/// Lowercase `seg` and collapse every run of non-alphanumeric characters
/// (dots, spaces, unicode, …) into a single `-`, trimming the edges — so a
/// repo named `upx.cz` becomes `upx-cz`. `_` is reserved as the segment
/// joiner, so it too maps to `-` here.
pub fn sanitize_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    let mut pending_dash = false;
    for ch in seg.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

#[cfg(test)]
mod tests;
