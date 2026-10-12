//! End-to-end tests for forward sync inside an Orca worktree: which paths are
//! left to Orca, and what happens to the main checkout's `.worktreeinclude`
//! afterwards.
//!
//! Every test but the last hands `sync_core` its environment as a closure, so
//! the Orca variables exist only for that one call. The last runs the real
//! `run_sync_flow` in a child process with the variable actually set.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use ss_magic_core::testutil::{
    exit_code_to_u8, git_run, init_main_repo, make_worktree, run_ignored_test_in_child,
    test_path_in_binary, write_file, write_magic,
};
use tempfile::TempDir;

use crate::orca::detect::{WORKTREE_ID_VAR, WORKTREE_PATH_VAR};
use crate::orca::worktreeinclude::{self, Document};
use crate::orca::write::ORCA_YAML;
use crate::sync::apply::{Event, SkipReason};
use crate::orca::write::{Level, Notice};
use crate::{orca_core, run_sync_flow, sync_core, sync_summary_line, SyncOptions};

/// A main checkout set up the way this release leaves one – `.env` files and
/// `.worktreeinclude` gitignored – plus a linked worktree of it.
fn orca_repo(patterns: &[&str]) -> (TempDir, TempDir, PathBuf) {
    let main = init_main_repo("main");
    write_file(main.path(), ".gitignore", ".env\n.worktreeinclude\nnode_modules/\n");
    git_run(&["add", ".gitignore"], main.path());
    git_run(&["commit", "-q", "-m", "ignore rules"], main.path());
    write_magic(main.path(), patterns);
    let (holder, wt_root) = make_worktree(main.path());
    (main, holder, wt_root)
}

/// Write main's `.worktreeinclude` with `listed` as its managed section, the
/// state a previous init or refresh would have left.
fn list_in_main(main: &Path, listed: &[&str]) {
    let entries: Vec<String> = listed.iter().map(|s| s.to_string()).collect();
    fs::write(
        main.join(worktreeinclude::FILE_NAME),
        Document::empty().render(&entries),
    )
    .unwrap();
}

fn listed_in_main(main: &Path) -> Vec<String> {
    worktreeinclude::entries(&fs::read_to_string(main.join(worktreeinclude::FILE_NAME)).unwrap())
}

/// The environment Orca gives a setup script for the worktree at `path`.
fn setup_env(path: &Path) -> impl Fn(&str) -> Option<OsString> {
    let path = path.to_path_buf();
    move |name: &str| (name == WORKTREE_PATH_VAR).then(|| path.clone().into_os_string())
}

fn no_env(_: &str) -> Option<OsString> {
    None
}

/// Run the sync and return its exit code and events.
fn sync(
    wt_root: &Path,
    opts: SyncOptions,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> (u8, Vec<Event>) {
    let mut events = Vec::new();
    let code = sync_core(wt_root, opts, env, |e| events.push(e.clone())).unwrap();
    (exit_code_to_u8(code), events)
}

fn left_to_orca(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Skip {
                reason: SkipReason::Filtered,
                label,
            } => Some(label.clone()),
            _ => None,
        })
        .collect()
}

fn copied(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Copy { rel } => Some(rel.display().to_string()),
            _ => None,
        })
        .collect()
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

// ── Skipping what Orca copied ──────────────────────────────────────────────

/// AE1. The list names `.env` and `apps/api/.env`; Orca copied only `.env`
/// (the other was past its copy budget). `.env` is left alone and
/// `apps/api/.env` is copied.
#[test]
fn ae1_a_listed_path_orca_copied_is_skipped_and_one_it_did_not_copy_is_synced() {
    let (main, _holder, wt) = orca_repo(&[".env", "apps/api/.env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "apps/api/.env", "API=1\n");
    list_in_main(main.path(), &[".env", "apps/api/.env"]);
    // What Orca left in the worktree: its own copy of `.env`, nothing else.
    write_file(&wt, ".env", "COPIED-BY-ORCA=1\n");

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(
        read(&wt, ".env"),
        "COPIED-BY-ORCA=1\n",
        "the path Orca copied must be left byte-for-byte alone"
    );
    assert_eq!(read(&wt, "apps/api/.env"), "API=1\n");
    assert_eq!(left_to_orca(&events), vec![".env"]);
    assert_eq!(copied(&events), vec!["apps/api/.env"]);
    assert!(
        !wt.join(".superset/backups").exists(),
        "nothing was overwritten, so nothing is backed up – least of all the skipped file"
    );
}

#[test]
fn the_summary_line_reports_paths_left_to_orca_apart_from_skips() {
    let summary = |copied, skipped, filtered| crate::sync::apply::Summary {
        copied,
        skipped,
        filtered,
    };
    assert_eq!(
        sync_summary_line(&summary(1, 0, 1)),
        "Sync done: copied 1 files, skipped 0 files, left 1 path to Orca"
    );
    assert_eq!(
        sync_summary_line(&summary(2, 1, 3)),
        "Sync done: copied 2 files, skipped 1 files, left 3 paths to Orca"
    );
    // Outside Orca the line is exactly what it has always been.
    assert_eq!(
        sync_summary_line(&summary(2, 0, 0)),
        "Sync done: copied 2 files, skipped 0 files"
    );
}

/// A hand-written line is honored like a generated one, and a hand-listed
/// directory covers the files present under it – but not one Orca did not get
/// to.
#[test]
fn hand_written_lines_and_listed_directories_are_honored_file_by_file() {
    let (main, _holder, wt) = orca_repo(&["config/local", "secrets/dev.pem"]);
    write_file(main.path(), ".gitignore", ".env\n.worktreeinclude\nconfig/local/\nsecrets/\n");
    write_file(main.path(), "config/local/a.json", "main-a\n");
    write_file(main.path(), "config/local/b.json", "main-b\n");
    write_file(main.path(), "secrets/dev.pem", "main-pem\n");
    fs::write(
        main.path().join(worktreeinclude::FILE_NAME),
        "# mine\nconfig/local/\nsecrets/dev.pem\n",
    )
    .unwrap();
    write_file(&wt, "config/local/a.json", "orca-a\n");
    write_file(&wt, "secrets/dev.pem", "orca-pem\n");

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(read(&wt, "config/local/a.json"), "orca-a\n");
    assert_eq!(read(&wt, "secrets/dev.pem"), "orca-pem\n");
    assert_eq!(
        read(&wt, "config/local/b.json"),
        "main-b\n",
        "a file under a listed directory that Orca did not copy is still synced"
    );
    let mut left = left_to_orca(&events);
    left.sort();
    assert_eq!(left, vec!["config/local/a.json", "secrets/dev.pem"]);
}

/// R17. `--no-orca-skip` copies the listed paths too – and the list is still
/// refreshed afterwards.
#[test]
fn no_orca_skip_copies_every_listed_path_and_still_refreshes() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "apps/api/.env", "API=1\n");
    list_in_main(main.path(), &[".env"]);
    write_file(&wt, ".env", "COPIED-BY-ORCA=1\n");

    let opts = SyncOptions {
        no_orca_skip: true,
        ..SyncOptions::default()
    };
    let (code, events) = sync(&wt, opts, &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(read(&wt, ".env"), "MAIN=1\n", "the listed path is copied");
    assert!(left_to_orca(&events).is_empty());
    assert_eq!(listed_in_main(main.path()), vec![".env", "apps/api/.env"]);
}

// ── Refreshing main's snapshot ─────────────────────────────────────────────

/// AE2. A file created in main after the snapshot was written is copied by
/// this sync, and afterwards main's list names it for the next worktree.
#[test]
fn ae2_a_file_missing_from_the_list_is_synced_and_then_listed() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "services/billing/.env", "BILLING=1\n");
    list_in_main(main.path(), &[".env"]);
    write_file(&wt, ".env", "COPIED-BY-ORCA=1\n");

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(read(&wt, "services/billing/.env"), "BILLING=1\n");
    assert_eq!(copied(&events), vec!["services/billing/.env"]);
    assert_eq!(
        listed_in_main(main.path()),
        vec![".env", "services/billing/.env"]
    );
}

/// R16. The skip set comes from the list as it was BEFORE the refresh. A path
/// the refresh adds was not in the list Orca used, so a stale copy of it in the
/// worktree is overwritten, not skipped.
#[test]
fn a_path_the_refresh_adds_is_not_skipped_on_the_strength_of_the_new_list() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "new/.env", "NEW-IN-MAIN=1\n");
    list_in_main(main.path(), &[".env"]);
    write_file(&wt, ".env", "COPIED-BY-ORCA=1\n");
    write_file(&wt, "new/.env", "STALE=1\n");

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(read(&wt, "new/.env"), "NEW-IN-MAIN=1\n");
    assert_eq!(left_to_orca(&events), vec![".env"]);
    assert_eq!(listed_in_main(main.path()), vec![".env", "new/.env"]);
}

/// AE6. A `node_modules` that appeared in main since `orca.yaml` was written
/// does not make the refresh touch `orca.yaml`: not one byte, and git sees no
/// modification.
#[test]
fn ae6_the_refresh_leaves_orca_yaml_byte_identical() {
    let (main, _holder, wt) = orca_repo(&[".env"]);
    let yaml = "scripts:\n  setup: |\n    ./.superset/magic.sh sync\nworktree:\n  sharedDirectories:\n    - node_modules\n";
    write_file(main.path(), ORCA_YAML, yaml);
    git_run(&["add", ORCA_YAML], main.path());
    git_run(&["commit", "-q", "-m", "orca.yaml"], main.path());
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "node_modules/a/index.js", "x");
    write_file(main.path(), "packages/new/node_modules/b/index.js", "x");

    let (code, _) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(read(main.path(), ORCA_YAML), yaml);
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain", "--", ORCA_YAML])
        .current_dir(main.path())
        .output()
        .unwrap();
    assert!(
        status.stdout.is_empty(),
        "git must see no change to orca.yaml: {}",
        String::from_utf8_lossy(&status.stdout)
    );
    assert_eq!(listed_in_main(main.path()), vec![".env"], "the list was refreshed");
}

/// R26. A tracked `.worktreeinclude` is never rewritten by the automatic
/// refresh; the sync itself still succeeds.
#[test]
fn a_tracked_list_is_not_refreshed_and_the_sync_still_succeeds() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "apps/api/.env", "API=1\n");
    fs::write(main.path().join(worktreeinclude::FILE_NAME), "committed/line\n").unwrap();
    git_run(&["add", "-f", worktreeinclude::FILE_NAME], main.path());
    git_run(&["commit", "-q", "-m", "track the list"], main.path());

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0, "a refresh that cannot run never fails the sync");
    assert_eq!(copied(&events).len(), 2);
    assert_eq!(
        read(main.path(), worktreeinclude::FILE_NAME),
        "committed/line\n"
    );
}

/// R26. Without the ignore rule in main nothing is written there.
#[test]
fn a_main_checkout_without_the_ignore_rule_is_not_refreshed() {
    let main = init_main_repo("main");
    write_file(main.path(), ".gitignore", ".env\n");
    git_run(&["add", ".gitignore"], main.path());
    git_run(&["commit", "-q", "-m", "ignore .env only"], main.path());
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    let (_holder, wt) = make_worktree(main.path());

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert_eq!(copied(&events), vec![".env"]);
    assert!(
        !main.path().join(worktreeinclude::FILE_NAME).exists(),
        "no untracked file may appear in the main checkout"
    );
}

/// A list that cannot be read is neither trusted nor replaced: nothing is
/// skipped, and its bytes stay exactly as they were.
#[test]
fn an_unreadable_list_skips_nothing_and_is_not_rewritten() {
    let (main, _holder, wt) = orca_repo(&[".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    let bytes = b".env\n# caf\xe9\n";
    fs::write(main.path().join(worktreeinclude::FILE_NAME), bytes).unwrap();
    write_file(&wt, ".env", "COPIED-BY-ORCA=1\n");

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert!(left_to_orca(&events).is_empty());
    assert_eq!(read(&wt, ".env"), "MAIN=1\n", "the sync copied it itself");
    assert_eq!(
        fs::read(main.path().join(worktreeinclude::FILE_NAME)).unwrap(),
        bytes
    );
}

/// An empty `files` list returns before any Orca step: no refresh.
#[test]
fn an_empty_files_list_returns_before_the_refresh() {
    let (main, _holder, wt) = orca_repo(&[]);

    let (code, events) = sync(&wt, SyncOptions::default(), &setup_env(&wt));

    assert_eq!(code, 0);
    assert!(events.is_empty());
    assert!(!main.path().join(worktreeinclude::FILE_NAME).exists());
}

// ── Not an Orca worktree ───────────────────────────────────────────────────

/// R18. With no Orca variables the sync is exactly today's: a listed path is
/// copied like any other, and main's list is not touched.
#[test]
fn without_orca_variables_nothing_is_skipped_and_nothing_is_refreshed() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "apps/api/.env", "API=1\n");
    list_in_main(main.path(), &[".env"]);
    let before = read(main.path(), worktreeinclude::FILE_NAME);
    write_file(&wt, ".env", "LOCAL=1\n");

    let (code, events) = sync(&wt, SyncOptions::default(), &no_env);

    assert_eq!(code, 0);
    assert_eq!(read(&wt, ".env"), "MAIN=1\n");
    assert!(left_to_orca(&events).is_empty());
    assert_eq!(read(main.path(), worktreeinclude::FILE_NAME), before);
}

/// AE3. An Orca terminal whose id names another worktree, changed into this
/// one: nothing is skipped and nothing is refreshed.
#[test]
fn ae3_orca_variables_naming_another_worktree_change_nothing() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "apps/api/.env", "API=1\n");
    list_in_main(main.path(), &[".env"]);
    let before = read(main.path(), worktreeinclude::FILE_NAME);
    write_file(&wt, ".env", "LOCAL=1\n");
    let elsewhere = tempfile::tempdir().unwrap();
    let id = format!("repo-id::{}", elsewhere.path().display());
    let env = move |name: &str| (name == WORKTREE_ID_VAR).then(|| OsString::from(&id));

    let (code, events) = sync(&wt, SyncOptions::default(), &env);

    assert_eq!(code, 0);
    assert_eq!(read(&wt, ".env"), "MAIN=1\n");
    assert!(left_to_orca(&events).is_empty());
    assert_eq!(read(main.path(), worktreeinclude::FILE_NAME), before);
}

/// AE8. `ss-magic sync` in the main checkout, with an id naming it: the main
/// checkout is never an Orca worktree, so its list is neither obeyed nor
/// rewritten.
#[test]
fn ae8_sync_in_the_main_checkout_never_runs_in_orca_mode() {
    // A pattern that matches nothing, so the main-to-main run copies nothing
    // and the assertion is about the Orca steps alone.
    let (main, _holder, _wt) = orca_repo(&["**/.no-such-file"]);
    list_in_main(main.path(), &["stale/entry"]);
    let before = read(main.path(), worktreeinclude::FILE_NAME);
    let root = main.path().canonicalize().unwrap();
    let id = format!("repo-id::{}", root.display());
    let env = move |name: &str| match name {
        n if n == WORKTREE_ID_VAR => Some(OsString::from(&id)),
        n if n == WORKTREE_PATH_VAR => Some(root.clone().into_os_string()),
        _ => None,
    };

    let (code, events) = sync(main.path(), SyncOptions::default(), &env);

    assert_eq!(code, 0);
    assert!(left_to_orca(&events).is_empty());
    assert_eq!(
        read(main.path(), worktreeinclude::FILE_NAME),
        before,
        "no refresh runs in the main checkout"
    );
}

// ── The explicit refresh: `ss-magic orca` ──────────────────────────────────

/// Store `shared` as `magic.json`'s `sharedDirectories`, keeping its `files`.
fn store_shared(main: &Path, shared: &[&str]) {
    use ss_magic_core::superset_files as files;
    let existing = files::load_magic_json(main).unwrap();
    let patterns = existing.as_ref().map(|m| m.files.clone()).unwrap_or_default();
    let shared = shared.iter().map(|s| s.to_string()).collect();
    files::write_magic_json(
        main,
        &files::merge_files_into_magic_config(existing.as_ref(), patterns, Some(shared)),
    )
    .unwrap();
}

fn refresh(cwd: &Path) -> Vec<Notice> {
    orca_core(cwd).unwrap_or_else(|_| panic!("the refresh must not be a hard error"))
}

fn shared_in(main: &Path) -> Vec<String> {
    crate::orca::orca_yaml::shared_directories(&read(main, ORCA_YAML))
}

/// F3. From a worktree, the refresh writes the MAIN checkout's files and
/// leaves the worktree's alone.
#[test]
fn the_refresh_run_from_a_worktree_writes_the_main_checkouts_files() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    store_shared(main.path(), &["**/node_modules"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "node_modules/a/index.js", "x");
    // The worktree has its own, different state.
    write_file(&wt, "packages/only-here/node_modules/b/index.js", "x");
    write_file(&wt, "only-in-worktree/.env", "WT=1\n");

    refresh(&wt);

    assert_eq!(shared_in(main.path()), vec!["node_modules"]);
    assert_eq!(listed_in_main(main.path()), vec![".env"]);
    assert!(!wt.join(ORCA_YAML).exists(), "nothing is written in the worktree");
    assert!(!wt.join(worktreeinclude::FILE_NAME).exists());
}

/// R13 + R25. A directory that appeared since `orca.yaml` was written is added
/// by the refresh, while `scripts.setup` stays byte-identical – even though
/// someone removed the `magic.sh` line from it.
#[test]
fn the_refresh_adds_a_new_directory_and_never_touches_an_existing_setup_script() {
    let (main, _holder, _wt) = orca_repo(&[".env"]);
    store_shared(main.path(), &["**/node_modules"]);
    write_file(main.path(), "node_modules/a/index.js", "x");
    let yaml = "# ours\nscripts:\n  setup: |\n    pnpm install   # the magic.sh line was removed on purpose\nworktree:\n  sharedDirectories:\n    - node_modules\n";
    write_file(main.path(), ORCA_YAML, yaml);
    write_file(main.path(), "packages/x/node_modules/b/index.js", "x");

    refresh(main.path());

    let after = read(main.path(), ORCA_YAML);
    assert_eq!(
        shared_in(main.path()),
        vec!["node_modules", "packages/x/node_modules"]
    );
    let script = "scripts:\n  setup: |\n    pnpm install   # the magic.sh line was removed on purpose\n";
    assert!(after.contains(script), "scripts.setup must be byte-identical:\n{after}");
    assert!(
        !crate::orca::orca_yaml::has_setup_line(&after),
        "a line removed on purpose is not added back by a refresh"
    );
}

/// The upgrade path. A repository set up before Orca support has a
/// `magic.json` and nothing else: one refresh creates `orca.yaml` WITH the
/// setup line, writes the snapshot and gitignores it.
#[test]
fn the_refresh_sets_up_a_repository_initialized_before_orca_support() {
    let main = init_main_repo("main");
    write_file(main.path(), ".gitignore", ".env\nnode_modules/\n");
    git_run(&["add", ".gitignore"], main.path());
    git_run(&["commit", "-q", "-m", "ignore"], main.path());
    write_magic(main.path(), &[".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "node_modules/a/index.js", "x");

    let notices = refresh(main.path());

    assert_eq!(
        read(main.path(), ORCA_YAML),
        "scripts:\n  setup: |\n    ./.superset/magic.sh sync\nworktree:\n  sharedDirectories: []\n"
    );
    assert_eq!(listed_in_main(main.path()), vec![".env"]);
    assert!(read(main.path(), ".gitignore")
        .lines()
        .any(|l| l == worktreeinclude::FILE_NAME));
    // Nobody chose shared directories yet: the list is empty, magic.json is
    // not changed, and the output says where to choose.
    let magic = ss_magic_core::superset_files::load_magic_json(main.path())
        .unwrap()
        .unwrap();
    assert_eq!(magic.shared_directories, None);
    assert!(
        notices
            .iter()
            .any(|n| n.level == Level::Info && n.text.contains("Edit synced files")),
        "{notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|n| n.level == Level::Ok && n.text.starts_with("Created orca.yaml")),
        "{notices:?}"
    );
}

/// With stored patterns the same first refresh lists the directories too.
#[test]
fn a_first_refresh_with_stored_patterns_creates_the_file_with_its_directories() {
    let (main, _holder, _wt) = orca_repo(&[".env"]);
    store_shared(main.path(), &["**/node_modules"]);
    write_file(main.path(), "node_modules/a/index.js", "x");
    write_file(main.path(), "apps/web/node_modules/b/index.js", "x");

    refresh(main.path());

    let yaml = read(main.path(), ORCA_YAML);
    assert!(crate::orca::orca_yaml::has_setup_line(&yaml), "{yaml}");
    assert_eq!(
        shared_in(main.path()),
        vec!["apps/web/node_modules", "node_modules"]
    );
}

/// The refresh adds the ignore rule a repository never had, which is exactly
/// what the automatic end-of-sync refresh was waiting for.
#[test]
fn the_refresh_adds_the_ignore_rule_so_the_automatic_refresh_can_run() {
    let main = init_main_repo("main");
    write_file(main.path(), ".gitignore", ".env\n");
    git_run(&["add", ".gitignore"], main.path());
    git_run(&["commit", "-q", "-m", "ignore .env only"], main.path());
    write_magic(main.path(), &["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    let (_holder, wt) = make_worktree(main.path());

    // Before: the automatic refresh has nowhere safe to write.
    sync(&wt, SyncOptions::default(), &setup_env(&wt));
    assert!(!main.path().join(worktreeinclude::FILE_NAME).exists());

    refresh(main.path());
    assert_eq!(listed_in_main(main.path()), vec![".env"]);

    // After: a file that appears in main reaches the list through the sync.
    write_file(main.path(), "apps/api/.env", "API=1\n");
    let (code, _) = sync(&wt, SyncOptions::default(), &setup_env(&wt));
    assert_eq!(code, 0);
    assert_eq!(listed_in_main(main.path()), vec![".env", "apps/api/.env"]);
}

/// Without a `magic.json` in the main checkout the refresh is a hard error and
/// writes nothing.
#[test]
fn the_refresh_without_magic_json_is_an_error_and_writes_nothing() {
    let main = init_main_repo("main");
    let (_holder, wt) = make_worktree(main.path());

    let code = orca_core(&wt).unwrap_err();

    assert_eq!(exit_code_to_u8(code), 1);
    assert!(!main.path().join(ORCA_YAML).exists());
    assert!(!main.path().join(worktreeinclude::FILE_NAME).exists());
    assert!(!main.path().join(".gitignore").exists());
}

#[test]
fn the_refresh_outside_a_repository_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(exit_code_to_u8(orca_core(dir.path()).unwrap_err()), 1);
}

/// An `orca.yaml` that cannot be edited is a notice, not a failure, and the
/// snapshot is still written.
#[test]
fn the_refresh_reports_an_uneditable_orca_yaml_and_still_writes_the_snapshot() {
    let (main, _holder, _wt) = orca_repo(&[".env"]);
    store_shared(main.path(), &["**/node_modules"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "node_modules/a/index.js", "x");
    let yaml = "worktree: {sharedDirectories: [old]}\n";
    write_file(main.path(), ORCA_YAML, yaml);

    let notices = refresh(main.path());

    assert_eq!(read(main.path(), ORCA_YAML), yaml);
    assert_eq!(listed_in_main(main.path()), vec![".env"]);
    let warning = notices
        .iter()
        .find(|n| n.level == Level::Warn)
        .expect("a warning about orca.yaml");
    assert!(warning.text.contains("flow mapping"), "{}", warning.text);
    let by_hand = notices
        .iter()
        .find(|n| n.text.contains("add this to orca.yaml"))
        .expect("the block to add by hand");
    assert!(by_hand.text.contains("- node_modules"), "{}", by_hand.text);
    assert!(
        !by_hand.text.contains("magic.sh"),
        "the refresh does not manage the setup script of an existing file: {}",
        by_hand.text
    );
}

// ── The real environment ───────────────────────────────────────────────────

const CHILD_MAIN: &str = "SS_MAGIC_TEST_ORCA_MAIN";
const CHILD_WORKTREE: &str = "SS_MAGIC_TEST_ORCA_WORKTREE";

/// The one test that reads the real process environment: `run_sync_flow` in a
/// child whose `ORCA_WORKTREE_PATH` names the worktree. Both Orca behaviors
/// happen – the listed path is left alone and main's list is refreshed.
#[test]
fn a_real_orca_worktree_path_variable_switches_both_behaviors_on() {
    let (main, _holder, wt) = orca_repo(&["**/.env", ".env"]);
    write_file(main.path(), ".env", "MAIN=1\n");
    write_file(main.path(), "apps/api/.env", "API=1\n");
    list_in_main(main.path(), &[".env"]);
    write_file(&wt, ".env", "COPIED-BY-ORCA=1\n");

    run_ignored_test_in_child(
        &test_path_in_binary(module_path!(), "child_sync_under_a_real_orca_variable"),
        &[
            (WORKTREE_PATH_VAR, wt.as_os_str()),
            (CHILD_MAIN, main.path().as_os_str()),
            (CHILD_WORKTREE, wt.as_os_str()),
        ],
        // The suite may itself run in an Orca terminal; its own worktree id
        // must not be what the child is tested against.
        &[WORKTREE_ID_VAR],
    );
}

#[test]
#[ignore = "runs only as the child of a_real_orca_worktree_path_variable_switches_both_behaviors_on"]
fn child_sync_under_a_real_orca_variable() {
    let (Some(main), Some(wt)) = (
        std::env::var_os(CHILD_MAIN),
        std::env::var_os(CHILD_WORKTREE),
    ) else {
        return;
    };
    let (main, wt) = (PathBuf::from(main), PathBuf::from(wt));
    assert_eq!(
        std::env::var_os(WORKTREE_PATH_VAR).as_deref(),
        Some(OsStr::new(&wt)),
        "the variable is really set in this process"
    );

    let code = run_sync_flow(&wt, SyncOptions::default()).unwrap();

    assert_eq!(exit_code_to_u8(code), 0);
    assert_eq!(read(&wt, ".env"), "COPIED-BY-ORCA=1\n", "left to Orca");
    assert_eq!(read(&wt, "apps/api/.env"), "API=1\n", "copied by ss-magic");
    assert_eq!(listed_in_main(&main), vec![".env", "apps/api/.env"]);
}
