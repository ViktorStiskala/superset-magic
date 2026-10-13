use super::*;
use crate::testutil::git_run;
use std::fs;
use tempfile::TempDir;

fn init_repo(initial_branch: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_run(&["init", "-q", "-b", initial_branch], dir.path());
    crate::testutil::neutralize_global_excludes(dir.path());
    // empty commit so the branch exists as a ref
    fs::write(dir.path().join("README.md"), "hi").unwrap();
    git_run(&["add", "."], dir.path());
    git_run(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

// ── Reverse-sync probes (U11) ───────────────────────────────────────────

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// `untracked_files` lists every untracked file — INCLUDING gitignored
/// ones — and excludes only tracked files. Including ignored files is the
/// behavior reverse sync depends on: the secrets it pushes are gitignored.
#[test]
fn untracked_files_lists_untracked_including_ignored() {
    let dir = init_repo("main");
    let root = dir.path();
    // tracked file (README.md committed by init_repo).
    // untracked, not ignored:
    write(root, "new.txt", "hi\n");
    // untracked AND gitignored (the secret-file shape reverse sync targets):
    write(root, ".gitignore", "**/.dev.vars\nignored.txt\n");
    write(root, "apps/api/.dev.vars", "SECRET=1\n");
    write(root, "ignored.txt", "nope\n");
    // The .gitignore itself is untracked here, so it would also show up.
    // Track it to keep the assertion focused on the data files.
    git_run(&["add", ".gitignore"], root);

    // Empty pathspecs → list the whole tree.
    let mut got: Vec<String> = untracked_files(root, &[])
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    got.sort();

    assert!(
        got.contains(&"apps/api/.dev.vars".to_string()),
        "gitignored untracked secret MUST be listed; got {got:?}"
    );
    assert!(
        got.contains(&"ignored.txt".to_string()),
        "gitignored untracked file MUST be listed; got {got:?}"
    );
    assert!(got.contains(&"new.txt".to_string()), "got {got:?}");
    assert!(
        !got.contains(&"README.md".to_string()),
        "tracked file must NOT be listed; got {got:?}"
    );
}

/// Pathspecs scope the probe: a gitignored secret reachable via an explicit
/// pathspec is listed (incl. one whose whole parent DIRECTORY is ignored),
/// while untracked files NOT named by a pathspec are omitted — so git never
/// has to walk unrelated ignored trees. Pins the perf-scoping behavior
/// reverse sync relies on.
#[test]
fn untracked_files_scopes_to_pathspecs_and_includes_ignored() {
    let dir = init_repo("main");
    let root = dir.path();
    // `secrets/` is gitignored as a whole DIRECTORY; `noise/` is unrelated.
    write(root, ".gitignore", "secrets/\nnoise/\n");
    git_run(&["add", ".gitignore"], root);
    write(root, "secrets/api.key", "KEY=1\n"); // gitignored via dir rule
    write(root, "noise/junk.bin", "x\n"); // untracked, not named below

    let got: Vec<String> = untracked_files(root, &["secrets/api.key"])
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    assert_eq!(
        got,
        vec!["secrets/api.key".to_string()],
        "pathspec must list the dir-ignored secret and nothing outside the pathspec; got {got:?}"
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

// ── Unified sync (Task 5) ────────────────────────────────────────────────

/// `is_ignored_str` on a directory-shaped query (trailing slash) matches a
/// `foo/bar/` gitignore rule even though `foo/bar` does not exist on disk yet
/// — the whole point of the raw-pathname variant over `is_ignored`, which can
/// only query paths that already exist. The no-slash query for the same
/// (still-absent) path is treated as a file and MISSES the directory rule.
#[test]
fn is_ignored_str_dir_trailing_slash_matches_before_dir_exists() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "foo/bar/\n");
    // `foo/bar` does not exist on disk at all.
    assert!(
        is_ignored_str(root, "foo/bar/").unwrap(),
        "trailing-slash query must match the dir rule before the dir exists"
    );
    assert!(
        !is_ignored_str(root, "foo/bar").unwrap(),
        "no-slash query on an absent path must NOT match a directory-only rule"
    );
}

/// `tracked_files` returns only the git-TRACKED members of `pathspecs`: a
/// committed file is listed, a merely-present-on-disk untracked file is not
/// — scoped to the pathspecs passed, mirroring `untracked_files`'s contract.
#[test]
fn tracked_files_returns_committed_excludes_untracked() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, "tracked.txt", "hi\n");
    git_run(&["add", "tracked.txt"], root);
    git_run(&["commit", "-q", "-m", "add tracked.txt"], root);
    write(root, "untracked.txt", "nope\n");

    let got: Vec<String> = tracked_files(root, &["tracked.txt", "untracked.txt"])
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    assert!(
        got.contains(&"tracked.txt".to_string()),
        "committed file must be listed; got {got:?}"
    );
    assert!(
        !got.contains(&"untracked.txt".to_string()),
        "untracked file must NOT be listed; got {got:?}"
    );
}

// ── Bulk ignored-and-untracked probe ────────────────────────────────────

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

/// One call answers the whole list: an ignored file and an ignored directory
/// (queried with a trailing slash) come back, a tracked file and an untracked
/// but not-ignored file do not.
#[test]
fn ignored_untracked_returns_exactly_the_ignored_candidates() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", ".env\nnode_modules/\n");
    write(root, ".env", "SECRET=1\n");
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "notes.txt", "untracked, not ignored\n");
    write(root, "node_modules/x/index.js", "x\n");
    git_run(&["add", ".gitignore", "src/main.rs"], root);
    git_run(&["commit", "-q", "-m", "files"], root);

    let got = ignored_untracked(
        root,
        &strings(&[".env", "src/main.rs", "notes.txt", "node_modules/"]),
    )
    .unwrap();

    assert_eq!(sorted(got), strings(&[".env", "node_modules/"]));
}

/// A tracked file is never reported, even when an ignore rule matches it: the
/// probe answers "ignored AND untracked", which is what keeps a committed file
/// out of the Orca snapshot.
#[test]
fn ignored_untracked_never_returns_a_tracked_file_matching_an_ignore_rule() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "*.env\n");
    write(root, "fixtures/sample.env", "FIXTURE=1\n");
    write(root, "apps/web/local.env", "SECRET=1\n");
    git_run(&["add", ".gitignore"], root);
    git_run(&["add", "-f", "fixtures/sample.env"], root);
    git_run(&["commit", "-q", "-m", "fixture"], root);

    let got = ignored_untracked(
        root,
        &strings(&["fixtures/sample.env", "apps/web/local.env"]),
    )
    .unwrap();

    assert_eq!(got, strings(&["apps/web/local.env"]));
}

/// Nothing ignored is git's exit 1, which is an empty answer and not an error;
/// an empty candidate list is answered without asking git at all.
#[test]
fn ignored_untracked_with_nothing_ignored_is_empty_not_an_error() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, "notes.txt", "n\n");

    assert!(ignored_untracked(root, &strings(&["notes.txt", "README.md"]))
        .unwrap()
        .is_empty());
    assert!(ignored_untracked(root, &[]).unwrap().is_empty());
    // No repository is needed for the empty list: nothing is spawned.
    let plain = tempfile::tempdir().unwrap();
    assert!(ignored_untracked(plain.path(), &[]).unwrap().is_empty());
}

/// Outside a repository git cannot answer, and that is an error for the caller
/// to turn into "nothing is positively ignored" – never a guess.
#[test]
fn ignored_untracked_outside_a_repository_is_an_error() {
    let plain = tempfile::tempdir().unwrap();
    write(plain.path(), ".env", "SECRET=1\n");

    let err = ignored_untracked(plain.path(), &strings(&[".env"])).unwrap_err();

    assert!(
        format!("{err:#}").contains("check-ignore"),
        "the error names the probe: {err:#}"
    );
}

/// The reason callers pre-filter: one candidate beneath a symlinked directory
/// makes git abort the whole batch, so the answer for every OTHER path is lost
/// too. With that candidate dropped first, the remaining answers are intact.
#[cfg(unix)]
#[test]
fn ignored_untracked_needs_symlinked_candidates_dropped_before_the_call() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", ".env\n");
    write(root, ".env", "ROOT=1\n");
    write(root, "real/.env", "REAL=1\n");
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

    let unfiltered = ignored_untracked(root, &strings(&[".env", "link/.env", "real/.env"]));
    assert!(
        unfiltered.is_err(),
        "git aborts the batch on a path beyond a symbolic link: {unfiltered:?}"
    );

    let prefiltered = ignored_untracked(root, &strings(&[".env", "real/.env"])).unwrap();
    assert_eq!(sorted(prefiltered), strings(&[".env", "real/.env"]));
}

/// The same abort, for a path inside a submodule: git refuses the pathspec, and
/// the caller's pre-filter (stop at any directory holding a `.git` entry) is
/// what keeps the other candidates answerable.
#[test]
fn ignored_untracked_needs_submodule_candidates_dropped_before_the_call() {
    let sub = init_repo("main");
    let dir = init_repo("main");
    let root = dir.path();
    git_run(
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            sub.path().to_str().unwrap(),
            "vendor/sub",
        ],
        root,
    );
    write(root, ".gitignore", ".env\n");
    write(root, ".env", "ROOT=1\n");
    write(root, "vendor/sub/.env", "SUB=1\n");

    let unfiltered = ignored_untracked(root, &strings(&[".env", "vendor/sub/.env"]));
    assert!(
        unfiltered.is_err(),
        "git aborts the batch on a path inside a submodule: {unfiltered:?}"
    );

    assert_eq!(
        ignored_untracked(root, &strings(&[".env"])).unwrap(),
        strings(&[".env"])
    );
}

/// A name in decomposed (NFD) Unicode form comes back byte-for-byte as it was
/// asked – git echoes the pathname it was given, so the caller's own string is
/// what ends up in the snapshot.
#[test]
fn ignored_untracked_returns_a_decomposed_unicode_name_byte_exact() {
    let dir = init_repo("main");
    let root = dir.path();
    // "café.secret" with the accent as a separate combining character (U+0301).
    let nfd = "cafe\u{301}.secret".to_string();
    write(root, ".gitignore", "*.secret\n");
    write(root, &nfd, "S=1\n");

    let got = ignored_untracked(root, std::slice::from_ref(&nfd)).unwrap();

    assert_eq!(got.len(), 1);
    assert_eq!(got[0].as_bytes(), nfd.as_bytes());
}

/// Git parses every line of the batch as a PATHSPEC, so a root-level file
/// literally named `:(top)a.env` would be answered as if it were `a.env` and
/// echoed back under its own name – reporting a file git does NOT ignore as
/// ignored. A candidate starting with `:` is therefore never asked about.
#[test]
fn ignored_untracked_never_reports_a_name_that_reads_as_pathspec_magic() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "a.env\n");
    write(root, "a.env", "SECRET=1\n");
    write(root, ":(top)a.env", "not ignored, merely named oddly\n");

    let got = ignored_untracked(root, &strings(&["a.env", ":(top)a.env"])).unwrap();

    assert_eq!(got, strings(&["a.env"]));
}

/// Other magic names make git exit 128 for the whole batch (`:(icase)z` is an
/// unsupported magic combination, `:!x` an exclusion), and an empty line is a
/// pathspec error too. Dropping them keeps the answer for the other candidates.
#[test]
fn ignored_untracked_keeps_the_batch_alive_past_unaskable_names() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "a.env\n");
    write(root, "a.env", "SECRET=1\n");

    let got = ignored_untracked(root, &strings(&["a.env", ":(icase)z", ":!x", ""])).unwrap();

    assert_eq!(got, strings(&["a.env"]));
}

/// Only a name at the repository root can begin a pathspec with `:`; the same
/// name below a directory starts with the directory's name and is an ordinary
/// path, so it is still answered (and still dropped from nothing).
#[test]
fn ignored_untracked_still_answers_a_colon_name_below_the_root() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", ":x\n");
    write(root, "dir/:x", "S=1\n");

    let got = ignored_untracked(root, &strings(&["dir/:x"])).unwrap();

    assert_eq!(got, strings(&["dir/:x"]));
}

/// A list made only of unaskable candidates is an empty answer without asking
/// git at all: outside a repository the probe would otherwise be an error.
#[test]
fn ignored_untracked_with_only_unaskable_candidates_spawns_nothing() {
    let plain = tempfile::tempdir().unwrap();

    let got = ignored_untracked(plain.path(), &strings(&[":(top)a.env", ":!x", ""])).unwrap();

    assert!(got.is_empty());
}

/// A list far larger than a pipe buffer is still answered by ONE process: the
/// candidates are written from a helper thread while the answers are read, so
/// the call neither deadlocks nor falls back to asking path by path.
#[test]
fn ignored_untracked_answers_a_list_larger_than_a_pipe_buffer() {
    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "*.secret\n");
    // Roughly 300 KiB of pathnames in each direction; none needs to exist,
    // since check-ignore answers from the rules.
    let candidates: Vec<String> = (0..6000)
        .map(|i| format!("some/fairly/long/directory/name/for/padding/file-{i:05}.secret"))
        .collect();

    let got = ignored_untracked(root, &candidates).unwrap();

    assert_eq!(got.len(), candidates.len());
    assert_eq!(got, candidates, "git answers in the order it was asked");
}

// ── One process, whatever the candidate count ────────────────────────────

const SHIM_REPO: &str = "SS_MAGIC_TEST_PROBE_REPO";
const SHIM_LOG: &str = "SS_MAGIC_TEST_PROBE_LOG";

/// The bulk probe spawns ONE `git` process for a whole candidate list, however
/// long. Proven by counting: a `git` shim placed first on `PATH` logs every
/// invocation and then runs the real git, in a child process so the changed
/// `PATH` exists nowhere else in the suite.
#[cfg(unix)]
#[test]
fn ignored_untracked_spawns_one_process_whatever_the_candidate_count() {
    use std::os::unix::fs::PermissionsExt;

    let dir = init_repo("main");
    let root = dir.path();
    write(root, ".gitignore", "*.secret\n");

    let real_git = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let shim_dir = tempfile::tempdir().unwrap();
    let log = shim_dir.path().join("invocations.log");
    let shim = shim_dir.path().join("git");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"${SHIM_LOG}\"\nexec '{}' \"$@\"\n",
            real_git.trim()
        ),
    )
    .unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = std::ffi::OsString::from(shim_dir.path());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());

    crate::testutil::run_ignored_test_in_child(
        &crate::testutil::test_path_in_binary(module_path!(), "child_probe_under_a_counting_git"),
        &[
            ("PATH", path.as_os_str()),
            (SHIM_REPO, root.as_os_str()),
            (SHIM_LOG, log.as_os_str()),
        ],
        &[],
    );
}

#[test]
#[ignore = "child half of ignored_untracked_spawns_one_process_whatever_the_candidate_count"]
fn child_probe_under_a_counting_git() {
    let (Some(root), Some(log)) = (std::env::var_os(SHIM_REPO), std::env::var_os(SHIM_LOG)) else {
        return;
    };
    let (root, log) = (std::path::PathBuf::from(root), std::path::PathBuf::from(log));
    let invocations = || fs::read_to_string(&log).unwrap_or_default().lines().count();

    for count in [1usize, 50, 5000] {
        let before = invocations();
        let candidates: Vec<String> = (0..count).map(|i| format!("dir/file-{i}.secret")).collect();

        let ignored = ignored_untracked(&root, &candidates).unwrap();

        assert_eq!(ignored.len(), count);
        assert_eq!(
            invocations() - before,
            1,
            "{count} candidates must cost exactly one git process"
        );
    }
    assert!(
        fs::read_to_string(&log).unwrap().lines().all(|line| line == "check-ignore"),
        "the shim really was the git that ran"
    );
    // And none at all for an empty list.
    let before = invocations();
    assert!(ignored_untracked(&root, &[]).unwrap().is_empty());
    assert_eq!(invocations(), before);
}
