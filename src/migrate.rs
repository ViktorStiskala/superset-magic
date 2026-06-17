//! Migration and init branching (U9).
//!
//! Three-way branch on `.superset/config.json`'s `setup` array (KTD8, R10):
//!
//! - An entry referencing the old `setup.sh` (including the both-markers
//!   case) → [`Branch::Migrate`]: rewrite the old layout to the new one.
//! - An entry referencing the `magic.sh` / `ss-magic sync` marker *only* →
//!   [`Branch::Normal`]: already migrated, nothing to do.
//! - Neither marker present, or `config.json` absent → [`Branch::Init`]:
//!   first-time bootstrap of the NEW layout.
//!
//! A *malformed* `config.json` is NEVER classified here — it is a hard error
//! surfaced by the caller (`superset_files::load_config` returns the parse
//! error, naming the path). `detect_branch` only sees a successfully parsed
//! `Option<&Config>`.
//!
//! ## Safety: stage → prompt → materialize (mirrors `bootstrap_flow`)
//!
//! Both [`run_migrate`] and [`run_init`] build a change summary, print it
//! (including the bold-orange stale-worktree advisory for migration), call
//! `ui::pick_final_action()`, and ONLY THEN stage the new tree into a tempdir
//! and materialize it via `superset_files::copy_into_repo`. An Esc/Ctrl-C at
//! the prompt returns via `?` before anything is staged, so the old layout
//! stays intact on disk — never a half-migrated tree. "Done" still
//! materializes (changes on disk, not committed), consistent with bootstrap.
//!
//! The destructive file transforms (rename/marker-replace/delete) are kept in
//! pure, UI-free helpers ([`migrated_setup`], [`stage_migration`]) so they are
//! unit-testable without driving the interactive prompt.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::git;
use crate::gitignore;
use crate::superset_files::{self, Config};
use crate::style;
use crate::ui::{self, FinalAction};

/// Marker substring identifying the retired `setup.sh` reference in a
/// `config.json` `setup` entry (e.g. `./.superset/setup.sh`).
const SETUP_SH_MARKER: &str = "setup.sh";

/// The wrapper invocation written into `config.json` `setup` by migration
/// and init. Superset reads this array verbatim during workspace creation.
pub const MAGIC_WRAPPER_ENTRY: &str = "./.superset/magic.sh sync";

/// Relative path of the retired `setup.sh`, deleted on migration.
const SETUP_SH_REL: &str = ".superset/setup.sh";

/// Relative path of `magic.local.json` as it appears inside the repo. Added
/// to the git-root `.gitignore` during migration and init.
const MAGIC_LOCAL_REL: &str = ".superset/magic.local.json";

/// Which branch the main-checkout bare invocation should take, decided from
/// the parsed `config.json` `setup` array (KTD8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// Old `setup.sh` reference present (with or without the new marker) —
    /// migrate the layout. Migrate wins when both markers are present.
    Migrate,
    /// Only the new `magic.sh` / `ss-magic sync` marker — already migrated.
    Normal,
    /// Neither marker present, or `config.json` absent — first-time init.
    Init,
}

/// True when a `setup` entry references the retired `setup.sh`.
fn entry_is_setup_sh(entry: &str) -> bool {
    entry.contains(SETUP_SH_MARKER)
}

/// True when a `setup` entry references the new wrapper / sync marker.
fn entry_is_magic_marker(entry: &str) -> bool {
    entry.contains("magic.sh") || entry.contains("ss-magic sync")
}

/// Pure branch decision (KTD8, R10).
///
/// Truth table over the parsed `config.json`:
///
/// | config.json        | setup contents                  | Branch  |
/// |--------------------|---------------------------------|---------|
/// | `None` (absent)    | —                               | Init    |
/// | `Some`             | references `setup.sh`           | Migrate |
/// | `Some`             | `setup.sh` AND magic marker     | Migrate |
/// | `Some`             | magic marker only               | Normal  |
/// | `Some`             | neither marker                  | Init    |
///
/// Migrate wins over Normal whenever a `setup.sh` reference is present, so a
/// half-migrated `setup` array (both markers) is repaired by migration rather
/// than treated as already-done. A malformed `config.json` is handled by the
/// caller as a hard error and never reaches this function.
pub fn detect_branch(config: Option<&Config>) -> Branch {
    let Some(cfg) = config else {
        return Branch::Init;
    };
    let has_setup_sh = cfg.setup.iter().any(|e| entry_is_setup_sh(e));
    if has_setup_sh {
        return Branch::Migrate;
    }
    let has_magic = cfg.setup.iter().any(|e| entry_is_magic_marker(e));
    if has_magic {
        Branch::Normal
    } else {
        Branch::Init
    }
}

/// Build the migrated `setup` array: replace the FIRST `setup.sh` entry in
/// place with [`MAGIC_WRAPPER_ENTRY`], strip any further `setup.sh` entries,
/// and preserve the order of every other entry verbatim.
///
/// Mirrors `merge_setup_into_config`'s preservation discipline: only the
/// `setup.sh` reference is touched; all other commands keep their position.
/// If the wrapper entry is already present elsewhere, the replaced slot is
/// dropped (it would otherwise duplicate the wrapper).
fn migrated_setup(old_setup: &[String]) -> Vec<String> {
    let already_has_wrapper = old_setup.iter().any(|e| e == MAGIC_WRAPPER_ENTRY);
    let mut out: Vec<String> = Vec::with_capacity(old_setup.len());
    let mut replaced = false;
    for entry in old_setup {
        if entry_is_setup_sh(entry) {
            if !replaced && !already_has_wrapper {
                out.push(MAGIC_WRAPPER_ENTRY.to_string());
                replaced = true;
            }
            // Subsequent setup.sh entries (or any when the wrapper already
            // exists) are dropped — the wrapper replaces the file-copy role.
            continue;
        }
        out.push(entry.clone());
    }
    out
}

/// Lines describing the migration, shared by the on-screen summary and tests.
/// Pure: computes the after-`setup` array but writes nothing.
fn migration_summary(existing: &Config) -> Vec<String> {
    let new_setup = migrated_setup(&existing.setup);
    vec![
        format!(
            "Rename .superset/setup_config.json → .superset/{}",
            "magic.json"
        ),
        "Write .superset/magic.sh (executable wrapper)".to_string(),
        format!(
            "Rewrite .superset/config.json setup → [{}]",
            new_setup.join(", ")
        ),
        "Delete .superset/setup.sh".to_string(),
        "Bootstrap .superset/magic.local.json + add it to .gitignore".to_string(),
    ]
}

/// Stage the migrated `.superset` tree into `stage_root` (a tempdir).
///
/// Reads the live `setup_config.json` (carrying its `files` across to
/// `magic.json`) and `config.json` (preserving `teardown`/`run` verbatim and
/// rewriting `setup` via [`migrated_setup`]) from `repo_root`, writing the new
/// layout into `stage_root/.superset/`. Also bootstraps `magic.local.json` in
/// the stage. Writes nothing into `repo_root`; the caller materializes via
/// `copy_into_repo` after the finishing-action prompt.
///
/// The retired `setup.sh` is NOT staged; the caller passes
/// [`SETUP_SH_REL`] to `copy_into_repo`'s delete set to remove it from the
/// repo.
fn stage_migration(repo_root: &Path, stage_root: &Path, existing: &Config) -> Result<()> {
    // setup_config.json `files` carry across to magic.json. Absent → empty.
    let files = superset_files::load_setup_config(repo_root)?
        .map(|c| c.files)
        .unwrap_or_default();
    superset_files::write_magic_json(stage_root, &files)?;

    // magic.sh wrapper.
    superset_files::write_magic_sh(stage_root)?;

    // config.json: preserve teardown/run, rewrite setup in place.
    let new_setup = migrated_setup(&existing.setup);
    let merged = superset_files::merge_setup_into_config(Some(existing), new_setup);
    superset_files::write_config_json(stage_root, &merged)?;

    // magic.local.json bootstrap (in the stage so it materializes atomically).
    superset_files::bootstrap_magic_local_json(stage_root)?;

    Ok(())
}

/// Interactive migration entry point (R11–R13, AE6). Main-checkout-only.
///
/// `existing` is the already-parsed live `config.json` (the caller proved it
/// parses; a malformed file is a hard error upstream). Prints the change
/// summary + stale-worktree advisory, prompts for the finishing action, then
/// stages + materializes the new layout. An idempotent re-run against an
/// already-migrated repo is detected upstream (`Branch::Normal`) and never
/// reaches here; this function additionally reports "nothing changed" if the
/// `setup` array already lacks a `setup.sh` reference (defensive).
pub fn run_migrate(repo_root: &Path, existing: &Config) -> Result<ExitCode> {
    style::print_section("Migrate .superset layout");
    println!(
        "{}",
        style::info(format!("Repo root: {}", repo_root.display()))
    );

    // Defensive idempotency: if there's no setup.sh reference there is nothing
    // to migrate. (The Normal branch already guards this upstream.)
    if !existing.setup.iter().any(|e| entry_is_setup_sh(e)) {
        println!(
            "{}",
            style::info("Already migrated — nothing changed.")
        );
        return Ok(ExitCode::SUCCESS);
    }

    // ---- Change summary (printed BEFORE the prompt). ----
    println!();
    println!("This will:");
    ui::print_pattern_list(&migration_summary(existing));
    println!();

    // ---- Stale-worktree advisory (bold orange, R/System-Wide Impact). ----
    println!(
        "{}",
        style::warn(
            "Worktrees created before migration keep the old setup.sh/setup_config.json \
and must be recreated."
        )
    );
    println!();

    // ---- Prompt FIRST. Esc/Ctrl-C returns here (via `?`) before anything is
    // ---- staged or materialized: the old layout stays intact on disk. ----
    let action = ui::pick_final_action()?;

    // ---- Stage into a tempdir. Drop on early return cleans up. ----
    let staging = tempfile::Builder::new()
        .prefix("ss-magic-migrate-")
        .tempdir()
        .context("creating migration staging tempdir")?;
    stage_migration(repo_root, staging.path(), existing)?;

    // ---- Materialize: copy staged files in, delete the retired setup.sh. ----
    superset_files::copy_into_repo(staging.path(), repo_root, &[SETUP_SH_REL])?;
    rename_setup_config(repo_root)?;
    gitignore::ensure_entry(repo_root, MAGIC_LOCAL_REL)?;

    println!();
    println!("{}", style::ok("Wrote .superset/magic.json"));
    println!("{}", style::ok("Wrote .superset/magic.sh"));
    println!("{}", style::ok("Updated .superset/config.json"));
    println!("{}", style::ok("Removed .superset/setup.sh"));
    println!("{}", style::ok("Bootstrapped .superset/magic.local.json"));

    execute_final_action(repo_root, action)
}

/// Remove the retired `setup_config.json` from the repo after `magic.json`
/// has been materialized in its place. The rename is realized as
/// write-magic.json (done by `copy_into_repo`) + delete-setup_config.json so
/// the staged tree stays a flat copy. A missing file is not an error.
fn rename_setup_config(repo_root: &Path) -> Result<()> {
    let old = repo_root.join(".superset/setup_config.json");
    if old.exists() {
        std::fs::remove_file(&old)
            .with_context(|| format!("deleting {}", old.display()))?;
    }
    Ok(())
}

/// Seed `magic.json`'s `files` for init: `default_magic_files()` first
/// (so `.superset/magic.local.json` is always synced), then the chosen
/// patterns, deduped (a chosen pattern already in the defaults is dropped).
/// Pure so the seeding rule is unit-testable without the UI.
fn init_magic_files(chosen: &[String]) -> Vec<String> {
    let mut files = superset_files::default_magic_files();
    for p in chosen {
        if !files.iter().any(|f| f == p) {
            files.push(p.clone());
        }
    }
    files
}

/// Interactive first-time init of the NEW layout (R10, AE5). Main-checkout-only.
///
/// Reuses the patterns picker (`ui::pick_patterns`), seeds `magic.json`'s
/// `files` with `default_magic_files()` + the chosen patterns, writes
/// `magic.sh`, sets `config.json` `setup` to the wrapper entry, and bootstraps
/// `magic.local.json` + gitignore. Stage → prompt → materialize, same as
/// migration.
pub fn run_init(repo_root: &Path, existing: Option<&Config>) -> Result<ExitCode> {
    style::print_section("Initialize .superset (magic layout)");
    println!(
        "{}",
        style::info(format!("Repo root: {}", repo_root.display()))
    );

    // ---- Capture decisions. Nothing written to repo_root yet. ----
    let mut options: Vec<String> = crate::repo_scan::OPTIONS.iter().map(|s| s.to_string()).collect();
    // Preselect filesystem hits among the preconfigured patterns.
    let pattern_strs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let fs_match = crate::repo_scan::matches_for_patterns(repo_root, &pattern_strs)?;
    let preselected: Vec<usize> = (0..options.len()).filter(|&i| fs_match[i]).collect();
    let chosen = ui::pick_patterns(&options, &preselected, repo_root)?;
    options.clear(); // not needed past this point

    // magic.json files = default_magic_files() + chosen (deduped, defaults first).
    let files = init_magic_files(&chosen);

    println!();
    println!("This will:");
    ui::print_pattern_list(&[
        "Write .superset/magic.json (synced patterns)".to_string(),
        "Write .superset/magic.sh (executable wrapper)".to_string(),
        format!("Write .superset/config.json setup → [{MAGIC_WRAPPER_ENTRY}]"),
        "Bootstrap .superset/magic.local.json + add it to .gitignore".to_string(),
    ]);
    println!();

    let action = ui::pick_final_action()?;

    // ---- Stage. ----
    let staging = tempfile::Builder::new()
        .prefix("ss-magic-init-")
        .tempdir()
        .context("creating init staging tempdir")?;
    let stage_root = staging.path();
    superset_files::write_magic_json(stage_root, &files)?;
    superset_files::write_magic_sh(stage_root)?;
    let merged =
        superset_files::merge_setup_into_config(existing, vec![MAGIC_WRAPPER_ENTRY.to_string()]);
    superset_files::write_config_json(stage_root, &merged)?;
    superset_files::bootstrap_magic_local_json(stage_root)?;

    // ---- Materialize. ----
    superset_files::copy_into_repo(stage_root, repo_root, &[])?;
    gitignore::ensure_entry(repo_root, MAGIC_LOCAL_REL)?;

    println!();
    println!("{}", style::ok("Wrote .superset/magic.json"));
    println!("{}", style::ok("Wrote .superset/magic.sh"));
    println!("{}", style::ok("Wrote .superset/config.json"));
    println!("{}", style::ok("Bootstrapped .superset/magic.local.json"));

    execute_final_action(repo_root, action)
}

/// Run the finishing action chosen at the prompt. Mirrors
/// `main::execute_final_action` (Done = changes on disk, not committed;
/// Commit/Branch stage + commit + push/PR). Duplicated here rather than
/// shared to keep U9 self-contained; U10 may unify them.
fn execute_final_action(repo_root: &Path, action: FinalAction) -> Result<ExitCode> {
    match action {
        FinalAction::Done => {
            println!(
                "{}",
                style::info("Done. Changes are on disk; run `git status` to review.")
            );
            Ok(ExitCode::SUCCESS)
        }
        FinalAction::CommitPushMain => {
            git::stage_paths(repo_root, &[".superset", ".gitignore"])?;
            if git::nothing_to_commit(repo_root)? {
                println!(
                    "{}",
                    style::info("Nothing to commit — files already match what is tracked.")
                );
                return Ok(ExitCode::SUCCESS);
            }
            git::commit(repo_root, MIGRATE_COMMIT_MESSAGE)?;
            let main_branch = git::main_branch_name(repo_root)?;
            git::push(repo_root, "origin", &main_branch)?;
            println!("{}", style::ok(format!("Pushed to origin/{main_branch}")));
            Ok(ExitCode::SUCCESS)
        }
        FinalAction::FeatureBranchPR => {
            git::stage_paths(repo_root, &[".superset", ".gitignore"])?;
            if git::nothing_to_commit(repo_root)? {
                println!(
                    "{}",
                    style::info(
                        "Nothing to commit — files already match what is tracked. No branch created."
                    )
                );
                return Ok(ExitCode::SUCCESS);
            }
            let suffix = git::timestamp_branch_suffix()?;
            let branch = format!("chore/ss-magic-migrate-{suffix}");
            git::create_branch(repo_root, &branch)?;
            git::commit(repo_root, MIGRATE_COMMIT_MESSAGE)?;
            git::push_upstream(repo_root, "origin", &branch)?;
            if !git::gh_available() {
                println!(
                    "{}",
                    style::warn(format!(
                        "`gh` not found in PATH; branch `{branch}` pushed. Open the PR manually."
                    ))
                );
                return Ok(ExitCode::SUCCESS);
            }
            let main_branch = git::main_branch_name(repo_root)?;
            match git::pr_create(repo_root, &main_branch) {
                Ok(url) => println!("{}", style::ok(format!("PR opened: {url}"))),
                Err(err) => eprintln!(
                    "{}",
                    style::warn(format!(
                        "{err:#}\nBranch `{branch}` is pushed; open the PR manually."
                    ))
                ),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

const MIGRATE_COMMIT_MESSAGE: &str = "chore(superset): migrate to ss-magic layout";

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fresh() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn cfg(setup: Vec<&str>, teardown: Vec<&str>, run: Vec<&str>) -> Config {
        Config {
            setup: setup.into_iter().map(String::from).collect(),
            teardown: teardown.into_iter().map(String::from).collect(),
            run: run.into_iter().map(String::from).collect(),
        }
    }

    // ── detect_branch truth table ───────────────────────────────────────────

    /// AE5: setup references neither marker → Init.
    #[test]
    fn ae5_detect_neither_marker_is_init() {
        let c = cfg(vec!["uv sync", "pnpm install"], vec![], vec![]);
        assert_eq!(detect_branch(Some(&c)), Branch::Init);
    }

    /// config.json absent → Init.
    #[test]
    fn detect_absent_config_is_init() {
        assert_eq!(detect_branch(None), Branch::Init);
    }

    /// Old setup.sh reference → Migrate.
    #[test]
    fn detect_setup_sh_is_migrate() {
        let c = cfg(vec!["./.superset/setup.sh"], vec![], vec![]);
        assert_eq!(detect_branch(Some(&c)), Branch::Migrate);
    }

    /// magic.sh marker only → Normal.
    #[test]
    fn detect_magic_marker_only_is_normal() {
        let c = cfg(vec![MAGIC_WRAPPER_ENTRY], vec![], vec![]);
        assert_eq!(detect_branch(Some(&c)), Branch::Normal);
    }

    /// `ss-magic sync` style marker (no magic.sh) → Normal.
    #[test]
    fn detect_ss_magic_sync_marker_is_normal() {
        let c = cfg(vec!["ss-magic sync"], vec![], vec![]);
        assert_eq!(detect_branch(Some(&c)), Branch::Normal);
    }

    /// Both markers present → Migrate wins.
    #[test]
    fn detect_both_markers_is_migrate() {
        let c = cfg(
            vec!["./.superset/setup.sh", MAGIC_WRAPPER_ENTRY],
            vec![],
            vec![],
        );
        assert_eq!(detect_branch(Some(&c)), Branch::Migrate);
    }

    /// Empty setup → Init (neither marker).
    #[test]
    fn detect_empty_setup_is_init() {
        let c = cfg(vec![], vec![], vec![]);
        assert_eq!(detect_branch(Some(&c)), Branch::Init);
    }

    /// Malformed config.json is a HARD ERROR at the load seam — never silently
    /// classified as Init. `detect_branch` only ever sees a successfully
    /// parsed `Option<&Config>`; the caller (U10) surfaces the parse error
    /// from `load_config`, which names the path. This pins that contract so a
    /// malformed file can never reach `detect_branch` as `None`.
    #[test]
    fn malformed_config_is_hard_error_not_init() {
        let repo = fresh();
        fs::create_dir_all(repo.path().join(".superset")).unwrap();
        fs::write(repo.path().join(".superset/config.json"), "{not json").unwrap();

        let err = superset_files::load_config(repo.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("config.json"), "error must name the path: {msg}");
        assert!(msg.contains("malformed JSON"), "msg: {msg}");
    }

    // ── migrated_setup: marker-replace-in-place + preservation ──────────────

    /// setup.sh is replaced in place by the wrapper; other entries keep order.
    #[test]
    fn migrated_setup_replaces_in_place_preserving_order() {
        let old = vec![
            "echo before".to_string(),
            "./.superset/setup.sh".to_string(),
            "uv sync".to_string(),
        ];
        let new = migrated_setup(&old);
        assert_eq!(
            new,
            vec![
                "echo before".to_string(),
                MAGIC_WRAPPER_ENTRY.to_string(),
                "uv sync".to_string(),
            ]
        );
    }

    /// Both markers already present → setup.sh stripped, wrapper kept once,
    /// no duplicate wrapper.
    #[test]
    fn migrated_setup_both_markers_strips_setup_sh_keeps_wrapper_once() {
        let old = vec![
            "./.superset/setup.sh".to_string(),
            MAGIC_WRAPPER_ENTRY.to_string(),
            "pnpm i".to_string(),
        ];
        let new = migrated_setup(&old);
        assert_eq!(
            new,
            vec![MAGIC_WRAPPER_ENTRY.to_string(), "pnpm i".to_string()],
            "setup.sh dropped, wrapper not duplicated"
        );
        assert_eq!(
            new.iter().filter(|e| *e == MAGIC_WRAPPER_ENTRY).count(),
            1
        );
    }

    /// A lone setup.sh entry becomes a lone wrapper entry.
    #[test]
    fn migrated_setup_lone_setup_sh_becomes_wrapper() {
        let old = vec!["./.superset/setup.sh".to_string()];
        assert_eq!(migrated_setup(&old), vec![MAGIC_WRAPPER_ENTRY.to_string()]);
    }

    // ── stage_migration: file transforms (no UI) ────────────────────────────

    /// Seed the repo with the OLD layout: setup.sh + setup_config.json +
    /// config.json referencing setup.sh, plus teardown/run to preserve.
    fn seed_old_layout(root: &Path) {
        let dot = root.join(".superset");
        fs::create_dir_all(&dot).unwrap();
        fs::write(dot.join("setup.sh"), "#!/bin/bash\necho old\n").unwrap();
        fs::write(
            dot.join("setup_config.json"),
            r#"{"files":["**/.env","apps/*/.dev.vars"]}"#,
        )
        .unwrap();
        fs::write(
            dot.join("config.json"),
            r#"{"setup":["./.superset/setup.sh","uv sync"],"teardown":["./drop.sh"],"run":["pnpm dev"]}"#,
        )
        .unwrap();
    }

    /// Old setup.sh reference → staged magic.json carries files, config.json
    /// gets the wrapper, teardown/run preserved, magic.sh + magic.local.json
    /// staged. Then materialize and assert setup.sh + setup_config.json gone.
    #[test]
    fn migration_transforms_old_layout_into_new() {
        let repo = fresh();
        seed_old_layout(repo.path());
        let existing = superset_files::load_config(repo.path())
            .unwrap()
            .unwrap();

        let stage = fresh();
        stage_migration(repo.path(), stage.path(), &existing).unwrap();

        // Staged magic.json carries setup_config.json's files verbatim.
        let staged_magic = superset_files::load_overlaid(stage.path())
            .unwrap()
            .unwrap();
        assert_eq!(staged_magic.files, vec!["**/.env", "apps/*/.dev.vars"]);

        // Staged config.json: setup rewritten in place, teardown/run preserved.
        let staged_cfg = superset_files::load_config(stage.path())
            .unwrap()
            .unwrap();
        assert_eq!(
            staged_cfg.setup,
            vec![MAGIC_WRAPPER_ENTRY.to_string(), "uv sync".to_string()]
        );
        assert_eq!(staged_cfg.teardown, vec!["./drop.sh".to_string()]);
        assert_eq!(staged_cfg.run, vec!["pnpm dev".to_string()]);

        // Staged magic.sh + magic.local.json present.
        assert!(stage.path().join(".superset/magic.sh").is_file());
        assert!(stage.path().join(".superset/magic.local.json").is_file());

        // Now materialize the way run_migrate does, and assert the repo's
        // legacy files are gone and the new ones are present.
        superset_files::copy_into_repo(stage.path(), repo.path(), &[SETUP_SH_REL]).unwrap();
        rename_setup_config(repo.path()).unwrap();
        gitignore::ensure_entry(repo.path(), MAGIC_LOCAL_REL).unwrap();

        let dot = repo.path().join(".superset");
        assert!(!dot.join("setup.sh").exists(), "setup.sh must be deleted");
        assert!(
            !dot.join("setup_config.json").exists(),
            "setup_config.json must be renamed away"
        );
        assert!(dot.join("magic.json").is_file());
        assert!(dot.join("magic.sh").is_file());
        assert!(dot.join("magic.local.json").is_file());

        // magic.sh is executable (0755) on unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dot.join("magic.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "magic.sh must be 0755");
        }

        // .gitignore now ignores magic.local.json.
        let gi = fs::read_to_string(repo.path().join(".gitignore")).unwrap();
        assert!(gi.lines().any(|l| l == MAGIC_LOCAL_REL));
    }

    /// Both-markers-present old config → migration still strips setup.sh and
    /// keeps the wrapper exactly once in the staged config.json.
    #[test]
    fn migration_both_markers_strips_setup_sh_keeps_wrapper() {
        let repo = fresh();
        let dot = repo.path().join(".superset");
        fs::create_dir_all(&dot).unwrap();
        fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();
        fs::write(dot.join("setup_config.json"), r#"{"files":[]}"#).unwrap();
        fs::write(
            dot.join("config.json"),
            format!(
                r#"{{"setup":["./.superset/setup.sh","{MAGIC_WRAPPER_ENTRY}"],"teardown":[],"run":[]}}"#
            ),
        )
        .unwrap();
        let existing = superset_files::load_config(repo.path())
            .unwrap()
            .unwrap();

        let stage = fresh();
        stage_migration(repo.path(), stage.path(), &existing).unwrap();

        let staged_cfg = superset_files::load_config(stage.path())
            .unwrap()
            .unwrap();
        assert_eq!(staged_cfg.setup, vec![MAGIC_WRAPPER_ENTRY.to_string()]);
        assert_eq!(
            staged_cfg
                .setup
                .iter()
                .filter(|e| *e == MAGIC_WRAPPER_ENTRY)
                .count(),
            1,
            "wrapper must not be duplicated"
        );
    }

    /// AE6: an already-migrated repo (Normal branch) is the idempotent case.
    /// `detect_branch` returns Normal so neither run_migrate nor any
    /// rename/delete fires; we assert the branch decision and that
    /// `migrated_setup` on an already-migrated array is a no-op (no duplicate
    /// wrapper, nothing stripped).
    #[test]
    fn ae6_already_migrated_is_normal_and_idempotent() {
        let migrated = cfg(vec![MAGIC_WRAPPER_ENTRY, "uv sync"], vec!["./drop.sh"], vec![]);
        assert_eq!(detect_branch(Some(&migrated)), Branch::Normal);
        // migrated_setup is only called on the Migrate branch, but prove it's
        // a structural no-op should it ever run on already-migrated input.
        assert_eq!(
            migrated_setup(&migrated.setup),
            vec![MAGIC_WRAPPER_ENTRY.to_string(), "uv sync".to_string()]
        );
    }

    /// rename_setup_config is a no-op when the file is already absent.
    #[test]
    fn rename_setup_config_noop_when_absent() {
        let repo = fresh();
        fs::create_dir_all(repo.path().join(".superset")).unwrap();
        rename_setup_config(repo.path()).unwrap(); // must not error
        assert!(!repo.path().join(".superset/setup_config.json").exists());
    }

    /// Esc/abort safety (logic seam): `stage_migration` writes ONLY into the
    /// tempdir. `run_migrate` calls it strictly AFTER `ui::pick_final_action()?`
    /// returns Ok, so an Esc/Ctrl-C aborts via `?` before this runs — leaving
    /// the on-disk old layout untouched. Here we prove the staging step itself
    /// never mutates the repo: after staging, the repo still has the legacy
    /// files and none of the new ones.
    #[test]
    fn staging_does_not_mutate_repo_until_materialized() {
        let repo = fresh();
        seed_old_layout(repo.path());

        // Snapshot the legacy on-disk files.
        let dot = repo.path().join(".superset");
        let setup_sh_before = fs::read_to_string(dot.join("setup.sh")).unwrap();
        let setup_cfg_before = fs::read_to_string(dot.join("setup_config.json")).unwrap();
        let config_before = fs::read_to_string(dot.join("config.json")).unwrap();

        let existing = superset_files::load_config(repo.path())
            .unwrap()
            .unwrap();
        let stage = fresh();
        stage_migration(repo.path(), stage.path(), &existing).unwrap();

        // Repo is byte-identical: nothing was written, renamed, or deleted.
        assert_eq!(
            fs::read_to_string(dot.join("setup.sh")).unwrap(),
            setup_sh_before
        );
        assert_eq!(
            fs::read_to_string(dot.join("setup_config.json")).unwrap(),
            setup_cfg_before
        );
        assert_eq!(
            fs::read_to_string(dot.join("config.json")).unwrap(),
            config_before
        );
        assert!(
            !dot.join("magic.json").exists(),
            "magic.json must not appear in the repo before materialize"
        );
        assert!(!dot.join("magic.sh").exists());
        assert!(!dot.join("magic.local.json").exists());
        assert!(
            !repo.path().join(".gitignore").exists(),
            ".gitignore must not be created before materialize"
        );
    }

    /// `copy_into_repo` materializes magic.sh (0755) + magic.json (not the
    /// legacy filenames) and deletes setup.sh from the repo via the delete set.
    #[test]
    fn copy_into_repo_materializes_magic_layout_and_deletes_setup_sh() {
        let repo = fresh();
        // Repo already has a legacy setup.sh that must be deleted.
        let dot = repo.path().join(".superset");
        fs::create_dir_all(&dot).unwrap();
        fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();

        // Stage the new layout.
        let stage = fresh();
        superset_files::write_magic_json(stage.path(), &["**/.env".to_string()]).unwrap();
        superset_files::write_magic_sh(stage.path()).unwrap();
        superset_files::write_config_json(
            stage.path(),
            &cfg(vec![MAGIC_WRAPPER_ENTRY], vec![], vec![]),
        )
        .unwrap();

        superset_files::copy_into_repo(stage.path(), repo.path(), &[SETUP_SH_REL]).unwrap();

        assert!(dot.join("magic.json").is_file(), "magic.json materialized");
        assert!(dot.join("magic.sh").is_file(), "magic.sh materialized");
        assert!(!dot.join("setup.sh").exists(), "setup.sh deleted");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dot.join("magic.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "magic.sh must be 0755");
        }
    }

    /// Init (AE5) seeds magic.json with default_magic_files() FIRST, then the
    /// chosen patterns, deduped. magic.local.json is always present.
    #[test]
    fn init_magic_files_seeds_defaults_then_chosen_deduped() {
        let chosen = vec!["**/.env".to_string(), "**/.dev.vars".to_string()];
        let files = init_magic_files(&chosen);
        // Defaults first.
        assert_eq!(files[0], ".superset/magic.local.json");
        // Chosen appended, in order.
        assert_eq!(
            files,
            vec![
                ".superset/magic.local.json".to_string(),
                "**/.env".to_string(),
                "**/.dev.vars".to_string(),
            ]
        );
    }

    /// A chosen pattern that duplicates a default appears only once.
    #[test]
    fn init_magic_files_dedupes_chosen_against_defaults() {
        let chosen = vec![".superset/magic.local.json".to_string(), ".env".to_string()];
        let files = init_magic_files(&chosen);
        assert_eq!(
            files,
            vec![".superset/magic.local.json".to_string(), ".env".to_string()],
            "magic.local.json must not be duplicated"
        );
    }

    /// stage_migration with no setup_config.json on disk → magic.json has an
    /// empty files array (no crash).
    #[test]
    fn stage_migration_without_setup_config_yields_empty_files() {
        let repo = fresh();
        let dot = repo.path().join(".superset");
        fs::create_dir_all(&dot).unwrap();
        fs::write(dot.join("setup.sh"), "#!/bin/bash\n").unwrap();
        fs::write(
            dot.join("config.json"),
            r#"{"setup":["./.superset/setup.sh"],"teardown":[],"run":[]}"#,
        )
        .unwrap();
        let existing = superset_files::load_config(repo.path())
            .unwrap()
            .unwrap();

        let stage = fresh();
        stage_migration(repo.path(), stage.path(), &existing).unwrap();
        let staged_magic = superset_files::load_overlaid(stage.path())
            .unwrap()
            .unwrap();
        assert!(staged_magic.files.is_empty());
    }
}
