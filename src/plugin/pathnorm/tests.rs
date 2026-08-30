use super::*;
use std::path::Path;

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
fn a_proc_cwd_prefix_is_recognized_and_stripped() {
    for prefix in ["/proc/self/cwd", "/proc/thread-self/cwd", "/proc/4321/cwd"] {
        let path = format!("{prefix}/docs/actions/x.checklist.json");
        assert_eq!(
            strip_proc_cwd(Path::new(&path)).as_deref(),
            Some(Path::new("docs/actions/x.checklist.json")),
            "{prefix} names a process's cwd and must be treated as relative"
        );
    }
}

#[test]
fn an_ordinary_path_is_not_mistaken_for_a_proc_cwd() {
    for path in [
        "/docs/actions/x.json",
        "/proc/self/environ",
        "/proc/self/cwdish/x",
        "/proc/notaprocess/cwd/x",
        "/procyon/self/cwd/x",
        "proc/self/cwd/x", // relative: absolutize already joins it
    ] {
        assert_eq!(
            strip_proc_cwd(Path::new(path)),
            None,
            "{path} is not a /proc cwd reference"
        );
    }
}

/// The bare `/proc/self/cwd` with nothing after it names the directory itself.
#[test]
fn a_bare_proc_cwd_strips_to_an_empty_remainder() {
    assert_eq!(
        strip_proc_cwd(Path::new("/proc/self/cwd")).as_deref(),
        Some(Path::new("")),
    );
}
