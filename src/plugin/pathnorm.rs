//! Lexical path normalization, shared by every gate that decides from a path.
//!
//! Six separate bypasses of the checklist deny (R88) came from the same
//! mistake: the classifier reduced a path one way and the filesystem reduced it
//! another, so two spellings of one file compared unequal and the deny did not
//! fire. A symlinked ancestor, a case difference, a relative target, a `..`
//! component, a `/proc/self/cwd` prefix and a leading `..` were each found and
//! fixed one at a time.
//!
//! Patching spellings individually is what produced that sequence. The
//! invariant a path gate actually needs is that **both sides are reduced to one
//! basis before they are compared, and no resolution whose answer depends on
//! which process performs it is ever trusted**. This module owns the lexical
//! half of that: [`normalize`] does the reduction `canonicalize` cannot do
//! (because it requires the file to exist, and the interesting case is
//! precisely a file that does not exist yet), and [`process_view`] answers
//! whether resolving the path here would even mean the same thing as resolving
//! it there.

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

/// How much of a path's meaning depends on **which process resolves it**.
///
/// `/proc/self/cwd/x` is absolute by every syntactic test and process-relative
/// in meaning: the kernel resolves `self` against whichever process asks. A
/// hook runs in its own process, so such a path resolves against the hook's
/// working directory while the harness — resolving the same literal string in
/// its own process — reaches a different file. That is the same divergence as
/// an unqualified relative path, wearing a leading slash.
///
/// **This is deliberately a property of the component sequence, not a list of
/// recognized prefixes.** An earlier version matched the fixed four components
/// `/`, `proc`, `<selector>`, `cwd`, and every sibling spelling walked straight
/// past it: `/proc/self/root/…` re-roots on another process's mount namespace,
/// `/proc/self/fd/<n>/…` and `/proc/self/task/<tid>/…` name descriptors and
/// threads only the selected process has, and procfs can be mounted somewhere
/// other than `/proc` entirely. Adding those spellings one at a time is what
/// produced a sequence of bypasses; what they have in common is the property
/// below, so that is what is tested for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProcessView {
    /// The path denotes the same file in every process. Resolve it normally.
    Independent,
    /// `…/proc/<selector>/cwd/<rest>`: the path is *some* process's working
    /// directory with `rest` below it. A caller that knows the working
    /// directory the path was meant for can re-root `rest` on it and get back
    /// a process-independent path, which is the only re-rootable form here.
    Cwd(PathBuf),
    /// Any other per-process view: `root`, `fd/<n>`, `task/<tid>/…`, `ns/…`,
    /// and whatever procfs grows next. These name something only the selected
    /// process can see, so there is no faithful re-rooting — and handing one to
    /// `canonicalize` in the wrong process silently answers a *different*
    /// question than the one asked. A gate must decide about it lexically
    /// instead, in whichever direction is safe.
    Opaque,
}

/// Classify `path` by [`ProcessView`].
///
/// The scan looks for a `proc` component followed by a process selector
/// **anywhere** in the path, not only at the root. Two reasons: procfs is
/// mountable anywhere (a bind mount at `/mnt/proc` is every bit as
/// process-relative as `/proc`), and a nested reference such as
/// `/proc/self/root/proc/self/cwd/x` has to be caught by its *first* selector
/// rather than its last. Over-matching costs an ordinary directory that happens
/// to be named `proc/self` a re-rooting it did not need; under-matching costs
/// the gate its answer, so the scan is deliberately wide.
///
/// Purely lexical — nothing under `/proc` is stat'd, so the answer is the same
/// on a machine that has no `/proc` at all.
pub(crate) fn process_view(path: &Path) -> ProcessView {
    let parts: Vec<&std::ffi::OsStr> = path.components().map(Component::as_os_str).collect();
    for (i, part) in parts.iter().enumerate() {
        if *part != "proc" {
            continue;
        }
        // The component after `proc` has to name a process for the rest to be
        // that process's private view; `/proc/mounts` and `/proc/cpuinfo` are
        // ordinary files that mean the same thing to everyone.
        match parts.get(i + 1) {
            Some(selector) if names_a_process(selector) => {}
            _ => continue,
        }
        return match parts.get(i + 2) {
            // The one form whose meaning a caller can reconstruct.
            Some(entry) if *entry == "cwd" => {
                let mut rest = PathBuf::new();
                for part in &parts[i + 3..] {
                    rest.push(part);
                }
                ProcessView::Cwd(rest)
            }
            // Everything else below a process selector, including the bare
            // `/proc/self` directory itself.
            _ => ProcessView::Opaque,
        };
    }
    ProcessView::Independent
}

/// Whether a path component selects a process: `self`, `thread-self`, or a
/// numeric pid/tid. A non-UTF-8 component can be none of those.
fn names_a_process(component: &std::ffi::OsStr) -> bool {
    let Some(name) = component.to_str() else {
        return false;
    };
    name == "self"
        || name == "thread-self"
        || (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests;
