use super::*;
use std::path::{Path, PathBuf};

/// The bug this module exists to prevent, stated as a test.
///
/// A naive reduction walks `parent()`/`file_name()` and looks equivalent.
/// It is not: `Path::file_name()` returns `None` for a `..` component, so such
/// a walk *skips* the hop rather than cancelling it, and the segment the `..`
/// was meant to remove survives. The classifier then checks
/// `docs/actions/foo/foo.checklist.json` while the filesystem — which does
/// cancel — writes to `docs/actions/foo.checklist.json`, and a deny keyed on the
/// former never fires for the latter.
#[test]
fn a_parent_component_cancels_the_segment_before_it() {
    assert_eq!(
        normalize(Path::new("/r/docs/actions/foo/../foo.checklist.json")),
        Path::new("/r/docs/actions/foo.checklist.json"),
        "`..` must cancel `foo`, not be skipped over"
    );

    // The shape a naive parent()/file_name() walk gets wrong, isolated.
    assert_eq!(
        Path::new("/r/docs/actions/foo/..").file_name(),
        None,
        "file_name() is None for a trailing `..` — this is why the naive walk drops the hop"
    );
}

#[test]
fn current_dir_components_disappear() {
    assert_eq!(
        normalize(Path::new("/r/./docs/./actions/x.json")),
        Path::new("/r/docs/actions/x.json")
    );
}

#[test]
fn repeated_and_trailing_parents_cancel_in_order() {
    assert_eq!(normalize(Path::new("/r/a/b/c/../../d")), Path::new("/r/a/d"));
    assert_eq!(normalize(Path::new("/r/a/b/../..")), Path::new("/r"));
}

/// A `..` that would climb past the root is kept rather than dropped.
///
/// Dropping it would turn a path that reaches outside its tree into a
/// different, valid-looking path inside it — the failure mode a containment
/// check exists to catch, manufactured by the normalizer itself.
#[test]
fn a_parent_above_the_root_is_kept_not_swallowed() {
    assert_eq!(normalize(Path::new("../../x")), Path::new("../../x"));
    assert_eq!(normalize(Path::new("a/../../x")), Path::new("../x"));
}

#[test]
fn a_normal_path_is_unchanged() {
    assert_eq!(
        normalize(Path::new("/r/docs/actions/x.checklist.json")),
        Path::new("/r/docs/actions/x.checklist.json")
    );
}

/// `/proc/self/cwd/x` is absolute by every syntactic test and process-relative
/// in meaning, so it slipped past the round-2 fix that only handled paths
/// without a leading slash. The hook resolves `self` against its own process;
/// the harness resolves the same string against the agent's.
#[test]
fn a_proc_cwd_path_is_recognized_as_re_rootable() {
    for prefix in ["/proc/self/cwd", "/proc/thread-self/cwd", "/proc/4321/cwd"] {
        let path = format!("{prefix}/docs/actions/x.checklist.json");
        assert_eq!(
            process_view(Path::new(&path)),
            ProcessView::Cwd(PathBuf::from("docs/actions/x.checklist.json")),
            "{prefix} names a process's cwd and its remainder must be re-rooted"
        );
    }
}

/// The sibling `/proc` forms that a fixed four-component prefix match walked
/// straight past. Each names something only the selected process can see, so
/// none of them can be re-rooted — and `canonicalize`ing one in the hook's own
/// process answers a question about the wrong process, which is exactly the
/// resolution the invariant forbids.
///
/// `/proc/self/root/proc/self/cwd/…` is the sharpest of them: it *contains* the
/// re-rootable spelling, and a scan that took the LAST match would hand back a
/// remainder rooted in another process's mount namespace. The first selector
/// wins for that reason.
#[test]
fn other_per_process_views_are_opaque_not_re_rootable() {
    for path in [
        "/proc/self/root/proc/self/cwd/docs/actions/x.checklist.json",
        "/proc/self/task/991/cwd/docs/actions/x.checklist.json",
        "/proc/self/fd/3/docs/actions/x.checklist.json",
        "/proc/1234/root/etc/passwd",
        "/proc/self/ns/mnt",
        // The bare selector directory: still that process's view, nothing to
        // re-root.
        "/proc/self",
    ] {
        assert_eq!(
            process_view(Path::new(path)),
            ProcessView::Opaque,
            "{path} is a per-process view with no faithful re-rooting"
        );
    }
}

/// Procfs is mountable anywhere, and a `..` can put the selector where a
/// prefix match would never look — so the property is tested for over the whole
/// component sequence, and the caller normalizes before asking.
#[test]
fn the_selector_is_found_wherever_it_sits_in_the_sequence() {
    // A bind mount somewhere other than `/proc`.
    assert_eq!(
        process_view(Path::new("/mnt/proc/self/cwd/x")),
        ProcessView::Cwd(PathBuf::from("x"))
    );
    // The ordering bug, as the caller hands it over: `normalize` first, then
    // ask. Asking first — which is what the previous shape did — sees `/tmp`
    // and answers `Independent`.
    assert_eq!(
        process_view(&normalize(Path::new("/tmp/../proc/self/cwd/x"))),
        ProcessView::Cwd(PathBuf::from("x")),
        "a `..` that lexically produces the prefix has to be cancelled first"
    );
    // A `proc` whose next component is not a process selector does not stop the
    // scan; a later one that is still counts.
    assert_eq!(
        process_view(Path::new("/a/proc/notaprocess/proc/self/cwd/x")),
        ProcessView::Cwd(PathBuf::from("x"))
    );
}

#[test]
fn an_ordinary_path_names_no_process() {
    for path in [
        "/docs/actions/x.json",
        // The `/proc` files that mean the same thing to every reader.
        "/proc/mounts",
        "/proc/cpuinfo",
        "/procyon/self/cwd/x",
        "/proc/notaprocess/cwd/x",
        "docs/actions/x.checklist.json",
    ] {
        assert_eq!(
            process_view(Path::new(path)),
            ProcessView::Independent,
            "{path} resolves the same way in every process"
        );
    }
}

/// The bare `/proc/self/cwd` with nothing after it names the directory itself,
/// which re-roots onto the caller's cwd with an empty remainder.
#[test]
fn a_bare_proc_cwd_re_roots_with_an_empty_remainder() {
    assert_eq!(
        process_view(Path::new("/proc/self/cwd")),
        ProcessView::Cwd(PathBuf::new()),
    );
}
