//! Deterministic `<repo>-<branch>` session identity (R14, R15), derived from
//! git alone — never from the Superset workspace name.
//!
//! This is the one thing the scratchpad contract is emphatic about: a
//! Superset workspace can be renamed with `superset ws update --name` at any
//! time, with no signal, so anything derived from the workspace name (or from
//! the worktree directory's own basename) would silently orphan the whole
//! scratchpad on a rename. The slug instead comes from two git-only sources —
//! the `origin` remote (or a directory-basename fallback, shared with
//! [`crate::pack`]'s archive naming) and the current branch — so it is stable
//! across renames, across days, and across Superset entirely. See
//! `docs/plans/2026-08-29-001-ss-magic-plugin/scratchpad-contract.md`'s
//! Identity section for the derivation this mirrors exactly (KTD12).
//!
//! Outside a git repository [`resolve`] returns `None` — the plugin does
//! nothing at all (R15) — and there is deliberately no other code path: since
//! identity never touches Superset, the same probe runs whether or not this
//! session happens to be inside a Superset workspace.

use std::path::Path;

use crate::git;
use crate::pack;

/// The resolved session identity: the two git-derived halves plus the
/// combined slug that names the scratchpad's session directory
/// (`sessions/<slug>/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The repo-name half of the slug (see [`repo_component`]).
    pub repo: String,
    /// The branch half of the slug (see [`branch_component`]) — already
    /// slugified, never the raw ref name.
    pub branch: String,
    /// `<repo>-<branch>`, the full session directory name.
    pub slug: String,
}

/// Resolve the session identity for `cwd`.
///
/// `cwd` must be the directory the caller actually cares about — for the
/// hook wrapper (added by a later unit) that is the `cwd` field read from the
/// hook's stdin envelope, never `${CLAUDE_PLUGIN_ROOT}` (which stays put
/// across a Superset workspace switch) and never the process's own current
/// directory. This function itself is agnostic to where `cwd` came from; it
/// only resolves the git repository reachable from it.
///
/// Returns `None` when `cwd` is not inside a git repository at all (R15) —
/// there is no other identity to fall back to, and the plugin does nothing.
/// Otherwise this cannot fail: every git probe below degrades to its own
/// documented fallback rather than propagating an error.
// consumed by later units: the scratchpad bootstrap, the hook wrapper, and
// every human verb that reports or acts on a session directory.
pub fn resolve(cwd: &Path) -> Option<Identity> {
    let root = git::cwd_repo_root(cwd).ok()?;
    let repo = repo_component(&root);
    let branch = branch_component(&root)?;
    Some(Identity {
        slug: format!("{repo}-{branch}"),
        repo,
        branch,
    })
}

/// The repo-name half: [`pack::repo_name_stem`] (the `origin` remote,
/// normalized, falling back to the main checkout directory's basename) —
/// reusing pack's archive-naming derivation verbatim rather than
/// reimplementing it, per KTD12. `"repo"` is a last-resort constant for the
/// rare case where NEITHER source yields usable characters (no origin, and a
/// main-checkout directory whose name sanitizes to nothing at all — e.g. a
/// bare root path) — pack has its own equally arbitrary last resort (`files`)
/// for the same case; there is no principled "next source" documented for the
/// repo half once both of KTD12's sources are exhausted.
fn repo_component(root: &Path) -> String {
    pack::repo_name_stem(root).unwrap_or_else(|| "repo".to_string())
}

/// The branch half: the current branch's short name, slugified, or a
/// `detached-<short-sha>` form when there is no usable branch name at all.
///
/// Two situations reach the detached form, both treated identically because
/// both mean "HEAD names no usable branch identity":
/// - `git symbolic-ref` itself fails (HEAD is genuinely detached, R15).
/// - `symbolic-ref` succeeds, but the branch's own name has nothing
///   slug-safe in it (e.g. a branch literally named `"---"`, which
///   [`slugify`] reduces to the empty string) — the scratchpad contract's
///   "guard the empty result by falling through to the next identity
///   source" guidance, applied here because a detached-HEAD-shaped fallback
///   is the only other branch identity KTD12 defines.
fn branch_component(root: &Path) -> Option<String> {
    if let Ok(Some(name)) = git::symbolic_ref_head(root) {
        let slug = slugify(&name);
        if !slug.is_empty() {
            return Some(slug);
        }
    }
    detached_component(root)
}

/// `detached-<short-sha>`, or `None` when there is no commit to abbreviate
/// yet (an unborn HEAD) — the one case where this whole module has nothing
/// usable to name a session directory with, so [`resolve`] reports `None`
/// the same as being outside a repository entirely.
fn detached_component(root: &Path) -> Option<String> {
    let sha = git::short_head_sha(root).ok().flatten()?;
    Some(format!("detached-{sha}"))
}

/// Hand-rolled slugify for a branch name, per the scratchpad contract's
/// pseudocode: `/` and every other unsafe (non-alphanumeric) character map to
/// `-`, runs of `-` collapse to one, the result is trimmed of leading/trailing
/// `-`, lowercased, and truncated to 40 characters. `/` gets no special
/// handling beyond "unsafe" — it is called out in the contract only because
/// it is the load-bearing case (a cross-repo PR workspace's branch is
/// `<forkOwner>/<headRefName>`), not because it needs different treatment
/// from any other separator.
///
/// Two additional guards the contract calls for, both found by running the
/// spike's shell version (which used BSD `sed`, silently a no-op on macOS —
/// see `archive_file_name`'s sibling history in `pack.rs`):
/// - A name that sanitizes to nothing (`"---"`) returns the empty string; the
///   caller ([`branch_component`]) is what falls through to the next source.
/// - An accented name (`"Ünïcödé Nàme"`) must strip to real letters
///   (`unicode-name`), not mangle to `n-c-d-n-me` — which is what happens if
///   every non-ASCII byte is treated as merely "unsafe". [`strip_diacritic`]
///   maps the common precomposed Latin letters to their ASCII base.
///
/// The same accented name can also reach here already NFD-decomposed instead
/// of precomposed — e.g. macOS/HFS+ can hand git a branch name as `"e"` plus
/// the combining acute accent U+0301, rather than the single precomposed
/// codepoint U+00E9, even though both spell "é" to a human. Skipping every
/// codepoint in the combining-marks block (U+0300..=U+036F) makes the two
/// forms slugify identically: the precomposed form maps straight to `e` via
/// [`strip_diacritic`], and the decomposed form maps its base `'e'` through
/// unchanged once its trailing combining mark is dropped rather than
/// (incorrectly) treated as a separate "unsafe" character.
fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_dash = false;
    for ch in input.chars() {
        if ('\u{0300}'..='\u{036F}').contains(&ch) {
            continue;
        }
        let mapped = strip_diacritic(ch).unwrap_or(ch);
        if mapped.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(mapped.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    let mut result = out.trim_matches('-').to_string();
    result.truncate(40);
    result
}

/// Map a precomposed Latin-1 Supplement / Latin Extended-A letter to its
/// unaccented ASCII base (case-insensitive — the result is lowercased by the
/// caller). Covers the accented letters a Western-European branch name is
/// likely to contain; anything outside this table (CJK, Cyrillic, emoji, …)
/// falls through unchanged to [`slugify`]'s generic "not ASCII alphanumeric"
/// handling, which maps it to `-` like any other unsafe character.
fn strip_diacritic(ch: char) -> Option<char> {
    Some(match ch {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'Ā' | 'ā' => 'a',
        'Ç' | 'ç' | 'Ć' | 'ć' | 'Č' | 'č' => 'c',
        'È' | 'É' | 'Ê' | 'Ë' | 'è' | 'é' | 'ê' | 'ë' | 'Ē' | 'ē' | 'Ě' | 'ě' => 'e',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'ì' | 'í' | 'î' | 'ï' | 'Ī' | 'ī' => 'i',
        'Ñ' | 'ñ' | 'Ń' | 'ń' => 'n',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ō' | 'ō' => 'o',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'ù' | 'ú' | 'û' | 'ü' | 'Ū' | 'ū' => 'u',
        'Ý' | 'ý' | 'ÿ' => 'y',
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
