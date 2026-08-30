//! Lexical path normalization, shared by every gate that decides from a path.
//!
//! Three separate bypasses of the checklist deny (R88) came from the same
//! mistake: the classifier reduced a path one way and the filesystem reduced it
//! another, so two spellings of one file compared unequal and the deny did not
//! fire. A symlinked ancestor, a case difference, and a relative target were
//! each found and fixed one at a time; a `..` component was the fourth.
//!
//! Patching spellings individually is what produced that sequence. The
//! invariant a path gate actually needs is that **both sides are reduced to one
//! basis before they are compared**, and this module owns the lexical half of
//! that reduction — the half `canonicalize` cannot do, because it requires the
//! file to exist and the interesting case is precisely a file that does not
//! exist yet.

use std::path::{Component, Path, PathBuf};

/// Remove `.` components and resolve `..` textually.
///
/// Purely lexical: the caller canonicalizes the existing part afterwards, which
/// is what handles symlinks. That order matters and is not interchangeable — a
/// `..` cancelled after a symlink is resolved means something different from one
/// cancelled before, and the filesystem cancels after.
///
/// A leading `..` that cannot be cancelled is kept rather than dropped, so a
/// path reaching above its root stays visibly wrong instead of quietly becoming
/// a different, valid-looking path.
///
/// **Do not reimplement this by walking `parent()` and `file_name()`.** That
/// shape looks equivalent and is not: `Path::file_name()` returns `None` for a
/// component of `..`, so a naive walk *skips* the hop instead of cancelling it
/// and silently keeps the segment the `..` was meant to remove. That was the
/// fourth bypass.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    // Leading `..`s are counted rather than pushed. Pushing them makes them
    // poppable by the next `..`, so `../../x` collapses to `x` - a path that
    // escapes its tree quietly becoming one inside it, which is the exact
    // failure a containment check downstream is looking for.
    let mut leading_parents = 0usize;
    let mut out = PathBuf::new();
    let mut rooted = false;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    // Nothing left to cancel. Above a root there is nowhere to
                    // go, so the `..` is dropped the way the kernel drops it;
                    // otherwise it is owed to the caller.
                    if !rooted {
                        leading_parents += 1;
                    }
                }
            }
            other => {
                if matches!(other, Component::RootDir | Component::Prefix(_)) {
                    rooted = true;
                }
                out.push(other.as_os_str());
            }
        }
    }
    if leading_parents == 0 {
        return out;
    }
    let mut prefixed = PathBuf::new();
    for _ in 0..leading_parents {
        prefixed.push("..");
    }
    prefixed.push(out);
    prefixed
}

/// Strip a `/proc`-relative cwd prefix, returning the remainder as relative.
///
/// `/proc/self/cwd/x` is absolute by every syntactic test and process-relative
/// in meaning: the kernel resolves `self` against whichever process calls it.
/// A hook runs in its own process, so such a path resolves against the hook's
/// working directory while the harness — resolving the same literal string in
/// its own process — reaches a different file. That is the same divergence as
/// an unqualified relative path, wearing a leading slash.
///
/// Returns `None` when the path is not `/proc`-relative, which is the ordinary
/// case. Linux only in effect; harmless elsewhere.
pub(crate) fn strip_proc_cwd(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    if components.next()?.as_os_str() != "proc" {
        return None;
    }
    // `self`, `thread-self`, or a numeric pid — all name a process whose cwd is
    // not necessarily this one's.
    let who = components.next()?;
    let who = who.as_os_str().to_str()?;
    let names_a_process =
        who == "self" || who == "thread-self" || who.chars().all(|c| c.is_ascii_digit());
    if !names_a_process {
        return None;
    }
    if components.next()?.as_os_str() != "cwd" {
        return None;
    }
    Some(components.as_path().to_path_buf())
}

#[cfg(test)]
mod tests;
