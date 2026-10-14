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
//!
//! ## Orca files
//!
//! Every operation here also writes Orca's `orca.yaml` and `.worktreeinclude`
//! at the repository root, derived from the same `magic.json` (see
//! `crate::orca`). The Orca output is COMPUTED before the finishing prompt, so
//! the "This will:" list names it, and WRITTEN after the `.superset` tree has
//! materialized ([`materialize_init`], [`materialize_migration`]). A file that
//! cannot be edited safely is reported and left alone; it never fails the
//! operation.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::git;
use crate::git::gitignore;
use crate::orca::shared_dirs;
use crate::orca::write::{self as orca_write, Notice, SetupPolicy};
use crate::sync::reverse_sync;
use crate::workspace::superset_files::{self, Config, MagicConfig};
use crate::tui::style;
use crate::tui::ui::{self, FinalAction};
use ss_magic_core::state_tree;

/// Marker substring identifying the retired `setup.sh` reference in a
/// `config.json` `setup` entry (e.g. `./.superset/setup.sh`).
const SETUP_SH_MARKER: &str = "setup.sh";

/// The wrapper invocation written into `config.json` `setup` by migration
/// and init. Superset reads this array verbatim during workspace creation.
///
/// NOTE the coupling: `magic.sh` is a pure pass-through (`exec ss-magic "$@"`),
/// so the `sync` subcommand is injected by THIS entry, not by the wrapper. A
/// future change that made `magic.sh` inject `sync` itself would have to drop
/// `sync` from this constant in lockstep, or the binary would receive
/// `sync sync`.
pub const MAGIC_WRAPPER_ENTRY: &str = "./.superset/magic.sh sync";

/// Relative path of the retired `setup.sh`, deleted on migration.
const SETUP_SH_REL: &str = ".superset/setup.sh";

/// Relative path of `magic.local.json` as it appears inside the repo. Gitignored
/// via `gitignore::ensure_path_ignored` during migration and init (the closest
/// existing `.gitignore`, or the git-root file; a no-op if git already ignores it).
const MAGIC_LOCAL_REL: &str = ".superset/magic.local.json";

/// Ensure the workspace's bootstrap gitignore rules exist under `repo_root` —
/// the step shared verbatim by `run_migrate`, `run_init`, and
/// `run_init_noninteractive`. Three rules, each idempotent (a no-op when git
/// already ignores it):
///
/// - `.superset/magic.local.json` — the per-machine overlay, unrecoverable if
///   committed;
/// - `.superset/backups/` — the tool's pre-overwrite backup tree, which can hold
///   recovered secret bytes. Ignored eagerly (via
///   [`reverse_sync::ensure_backups_ignored`]) so a fresh `ss-magic init` gets
///   the same rule the FIRST sync would otherwise add lazily — the backups tree
///   is protected before any secret bytes are ever backed up; and
/// - `.superset/.magic/` — the Claude plugin's per-worktree state tree (via
///   core's [`state_tree::ensure_state_ignored`], the rule's single owner, which
///   the plugin's `scratchpad` re-exports for its lazy callers – so the CLI no
///   longer reaches into plugin code for it). Eager for the same reason, and with
///   more riding on it: the plugin's hooks refuse to write ANY state while git
///   does not report that tree ignored, so a repository that never gets this
///   rule gets no plugin state at all. `ss-magic sync` deliberately runs no
///   plugin step, so `init`/`migrate` here and the explicit
///   `ss-magic-plugin enable` are the only two paths that ever write it — never
///   a hook.
fn ensure_bootstrap_gitignores(repo_root: &Path) -> Result<()> {
    gitignore::ensure_path_ignored(
        repo_root,
        repo_root,
        Path::new(MAGIC_LOCAL_REL),
        gitignore::PathKind::File,
    )?;
    reverse_sync::ensure_backups_ignored(repo_root)?;
    state_tree::ensure_state_ignored(repo_root)?;
    Ok(())
}

/// The sync patterns as they will be once `files` is written to `magic.json`:
/// `files` first, then every pattern of the existing `magic.local.json` overlay
/// that is not already among them – the same union-and-dedupe
/// `superset_files::load_overlaid` applies, computed from the list in hand
/// because `magic.json` is not on disk yet when the Orca output is planned.
/// A repository with no local overlay yet contributes nothing, which is right:
/// the one a first init bootstraps is empty.
///
/// An overlay that cannot be read or parsed (a merge conflict, a hand edit
/// gone wrong) contributes no patterns, and the second element of the result is
/// a warning for the caller to print. Init, edit config and migrate never
/// parsed that file before the Orca output existed, so a broken per-machine
/// file must not stop them now; leaving its patterns out of the list handed to
/// Orca is always safe, because forward sync still copies those files. The
/// function prints nothing itself so the warning text can be tested.
fn overlaid_patterns(repo_root: &Path, files: &[String]) -> (Vec<String>, Option<String>) {
    let mut patterns = files.to_vec();
    match superset_files::load_magic_local_json(repo_root) {
        Ok(Some(local)) => {
            for pattern in local.files {
                if !patterns.contains(&pattern) {
                    patterns.push(pattern);
                }
            }
            (patterns, None)
        }
        Ok(None) => (patterns, None),
        Err(e) => {
            let warning = format!(
                ".superset/magic.local.json could not be read ({e:#}); its patterns are \
                 left out of the files listed for Orca until it is fixed."
            );
            (patterns, Some(warning))
        }
    }
}

/// Prints the warning `overlaid_patterns` produced, if any, to stderr.
fn warn_overlay(warning: Option<String>) {
    if let Some(warning) = warning {
        eprintln!("{}", style::warn(warning));
    }
}

/// Path of the pre-marketplace skills-directory copy of the plugin, relative to
/// the user's home directory.
const LEGACY_SKILLS_REL: &str = ".claude/skills/ss-magic";

/// Remove a pre-marketplace `~/.claude/skills/ss-magic/` left by an earlier
/// revision of this tool, if one is there.
///
/// The plugin now reaches a machine only through the Claude Code marketplace,
/// and a marketplace install outranks a skills-directory copy of the same
/// name. Left in place, the shadowed copy does no work but is permanently
/// reported as a conflict in the harness's plugin-errors view, so `init` and
/// `migrate` clear it out. Nothing outside that exact path is touched, and a
/// symlink at that path has the LINK removed rather than whatever it points
/// at. Returns whether anything was removed.
fn remove_legacy_skills_dir(home: &Path) -> Result<bool> {
    let path = home.join(LEGACY_SKILLS_REL);
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Ok(false);
    };
    if meta.is_symlink() || meta.is_file() {
        fs::remove_file(&path)
            .with_context(|| format!("removing {}", path.display()))?;
    } else {
        fs::remove_dir_all(&path)
            .with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(true)
}

/// [`remove_legacy_skills_dir`] against the real home directory, best-effort:
/// a failure warns and lets init/migrate finish, since a stale skills copy is
/// a cosmetic conflict in the plugin-errors view rather than a reason to
/// abandon a workspace bootstrap. A machine with no discoverable home
/// directory has nowhere for the legacy copy to be, so it is a no-op.
#[cfg(not(test))]
fn clear_legacy_skills_install() {
    let Some(dirs) = directories::BaseDirs::new() else {
        return;
    };
    match remove_legacy_skills_dir(dirs.home_dir()) {
        Ok(true) => println!(
            "{}",
            style::ok(format!("Removed legacy ~/{LEGACY_SKILLS_REL}/"))
        ),
        Ok(false) => {}
        Err(e) => eprintln!(
            "{}",
            style::warn(format!("Could not remove ~/{LEGACY_SKILLS_REL}/: {e:#}"))
        ),
    }
}

/// The real home directory is deliberately unreachable from the test binary:
/// the unit suite runs on a developer's own machine, and `run_init` /
/// `run_init_noninteractive` are called directly by several tests. A test that
/// can delete something under someone's `~` is not one anyone should have to
/// trust, however narrow the path. The removal logic itself is covered by
/// driving [`remove_legacy_skills_dir`] against a temporary home instead.
#[cfg(test)]
fn clear_legacy_skills_install() {}

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

/// Build the `setup` array init and edit config write: the existing one, with
/// the wrapper ensured and every other command kept in its place.
///
/// Edit config runs on a repository whose `setup` already holds the wrapper,
/// often beside commands of the person's own (`pnpm install`, `uv sync`), and
/// init may meet a `config.json` with commands but no wrapper. Both used to
/// write `[wrapper]` alone, which silently deleted those commands from every
/// Superset workspace created afterwards. Now a `setup.sh` reference is
/// replaced in place exactly as migration does it ([`migrated_setup`] – only
/// the scripted `ss-magic init` can meet one, the menu routes such a
/// repository to migrate); then, when no entry is a wrapper or sync marker
/// yet, the wrapper is put FIRST, so the configured files are in place before
/// the person's own commands run. An array that already carries a marker is
/// returned as it is. `None` (no `config.json`) is the wrapper alone.
fn init_setup(existing: Option<&Config>) -> Vec<String> {
    let mut setup = existing.map_or_else(Vec::new, |cfg| migrated_setup(&cfg.setup));
    if !setup.iter().any(|entry| entry_is_magic_marker(entry)) {
        setup.insert(0, MAGIC_WRAPPER_ENTRY.to_string());
    }
    setup
}

/// The "This will:" line for the local overlay. Init and edit config only
/// create `magic.local.json` when it is absent (the file is gitignored, so an
/// existing one is unrecoverable and is kept), and `ensure_bootstrap_gitignores`
/// adds its ignore rule only when git does not already apply one. Saying
/// "Bootstrap … + add it to .gitignore" unconditionally described neither on a
/// second run. `ignored` is git's answer, with "could not be asked" passed as
/// `false` – the rule is then promised, and the bootstrap writes it.
fn local_overlay_line(exists: bool, ignored: bool) -> String {
    format!(
        "{} {MAGIC_LOCAL_REL}{}",
        if exists { "Keep" } else { "Bootstrap" },
        if ignored {
            " (already gitignored)"
        } else {
            " + add it to .gitignore"
        }
    )
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
///
/// `shared` becomes `magic.json`'s `sharedDirectories` – the patterns naming
/// the dependency directories Orca shares with a new worktree – and `None`
/// leaves that key out of the file.
fn stage_migration(
    repo_root: &Path,
    stage_root: &Path,
    existing: &Config,
    shared: Option<&[String]>,
) -> Result<()> {
    // setup_config.json `files` carry across to magic.json. Absent → empty.
    let files = superset_files::load_setup_config(repo_root)?
        .map(|c| c.files)
        .unwrap_or_default();
    // Preserve any unknown top-level keys (KTD8) already sitting in
    // magic.json — a repo mid-migration (both the old setup.sh marker and
    // the new magic.sh marker present in `setup`) can already have one.
    // `shared` is the Orca shared-directory pattern list the caller settled on
    // (the stored one, or the detected defaults); it is stored as given, and
    // `None` leaves the key absent.
    let existing_magic = superset_files::load_magic_json(repo_root)?;
    let magic_cfg = superset_files::merge_files_into_magic_config(
        existing_magic.as_ref(),
        files,
        shared.map(<[String]>::to_vec),
    );
    superset_files::write_magic_json(stage_root, &magic_cfg)?;

    // magic.sh wrapper.
    superset_files::write_magic_sh(stage_root)?;

    // config.json: preserve teardown/run, rewrite setup in place.
    let new_setup = migrated_setup(&existing.setup);
    let merged = superset_files::merge_setup_into_config(Some(existing), new_setup);
    superset_files::write_config_json(stage_root, &merged)?;

    // magic.local.json bootstrap (in the stage so it materializes atomically).
    // Guard against clobbering a pre-existing repo magic.local.json (gitignored,
    // therefore unrecoverable) on the rare old-layout repo that already has one.
    if !repo_root.join(".superset/magic.local.json").exists() {
        superset_files::bootstrap_magic_local_json(stage_root)?;
    }

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

    // ---- Orca output, decided before the prompt so the summary can name it.
    // Migration asks nothing about shared directories: it keeps a stored list
    // and otherwise takes every default kind that matches something. ----
    let stored_shared = superset_files::load_magic_json(repo_root)?
        .and_then(|magic| magic.shared_directories);
    let shared = shared_dirs::stored_or_detected(repo_root, stored_shared.as_deref());
    // The patterns migration is about to carry into magic.json, read from the
    // same legacy file `stage_migration` reads them from.
    let migrated_files = superset_files::load_setup_config(repo_root)?
        .map(|c| c.files)
        .unwrap_or_default();
    let (orca_patterns, overlay_warning) = overlaid_patterns(repo_root, &migrated_files);
    warn_overlay(overlay_warning);
    let orca_plan = orca_write::plan(
        repo_root,
        &orca_patterns,
        shared.as_deref().unwrap_or_default(),
        SetupPolicy::Ensure,
    );

    // ---- Change summary (printed BEFORE the prompt). ----
    println!();
    println!("This will:");
    let mut summary = migration_summary(existing);
    summary.extend(orca_plan.summary_lines());
    ui::print_pattern_list(&summary);
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

    let orca_notices =
        materialize_migration(repo_root, existing, shared.as_deref(), &orca_plan)?;

    println!();
    println!("{}", style::ok("Wrote .superset/magic.json"));
    println!("{}", style::ok("Wrote .superset/magic.sh"));
    println!("{}", style::ok("Updated .superset/config.json"));
    println!("{}", style::ok("Removed .superset/setup.sh"));
    println!("{}", style::ok("Bootstrapped .superset/magic.local.json"));
    println!("{}", style::ok("Gitignored .superset/backups/"));
    println!("{}", style::ok("Gitignored .superset/.magic/"));
    orca_write::print_notices(&orca_notices);

    execute_final_action(repo_root, action, MIGRATE_COMMIT_MESSAGE, "chore/ss-magic-migrate-")
}

/// Everything migration does once the finishing action is chosen: stage the
/// new tree in a tempdir, copy it in, delete the retired `setup.sh` and
/// `setup_config.json`, add the bootstrap gitignore rules, and write the Orca
/// files per `orca_plan`. Returns the Orca notices for the caller to print.
/// Split from [`run_migrate`] so the whole write is testable without a prompt.
fn materialize_migration(
    repo_root: &Path,
    existing: &Config,
    shared: Option<&[String]>,
    orca_plan: &orca_write::Plan,
) -> Result<Vec<Notice>> {
    // Stage into a tempdir. Drop on early return cleans up.
    let staging = tempfile::Builder::new()
        .prefix("ss-magic-migrate-")
        .tempdir()
        .context("creating migration staging tempdir")?;
    stage_migration(repo_root, staging.path(), existing, shared)?;

    // Materialize: copy staged files in, delete the retired setup.sh.
    superset_files::copy_into_repo(staging.path(), repo_root, &[SETUP_SH_REL])?;
    rename_setup_config(repo_root)?;
    ensure_bootstrap_gitignores(repo_root)?;
    clear_legacy_skills_install();

    // The Orca files come last, written as they were planned before the
    // prompt. A file that changed on disk since the plan read it is left alone
    // with a warning, since the planned text would discard that change.
    Ok(orca_plan.apply(repo_root))
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

/// Build the picker `(options, preselected_indices)` for `run_init`, factored
/// out so it can be unit-tested without driving the interactive prompt.
///
/// `existing_magic_files` is the `files` array from the base `magic.json`
/// on disk (empty slice for a first-time init).  `fs_match` must be aligned
/// to `repo_scan::OPTIONS` (one `bool` per entry, `true` = filesystem hit).
///
/// Rules:
/// - Options = `repo_scan::OPTIONS` + any CUSTOM patterns in
///   `existing_magic_files` that are not already in `repo_scan::OPTIONS`
///   (computed via `superset_files::existing_unknown_entries`).
/// - Preconfigured `OPTIONS` rows are preselected when there is a filesystem
///   hit **OR** the pattern is already present in `existing_magic_files`.
/// - Custom rows are always preselected (they came from the existing config).
pub fn build_pattern_options(
    existing_magic_files: &[String],
    fs_match: &[bool],
) -> (Vec<String>, Vec<usize>) {
    use crate::sync::repo_scan::OPTIONS;
    use crate::workspace::superset_files::existing_unknown_entries;

    debug_assert_eq!(
        fs_match.len(),
        OPTIONS.len(),
        "fs_match must be aligned to repo_scan::OPTIONS"
    );

    // Custom patterns = those in existing magic.json not in OPTIONS.
    let custom = existing_unknown_entries(existing_magic_files, &OPTIONS);

    let mut options: Vec<String> = OPTIONS.iter().map(|s| s.to_string()).collect();
    options.extend(custom.iter().cloned());

    let mut preselected: Vec<usize> = Vec::new();
    // Preconfigured OPTIONS: preselect on fs hit OR already in existing config.
    for (i, opt) in OPTIONS.iter().enumerate() {
        let in_existing = existing_magic_files.iter().any(|f| f == opt);
        if fs_match[i] || in_existing {
            preselected.push(i);
        }
    }
    // Custom entries start after OPTIONS; always preselected.
    for i in 0..custom.len() {
        preselected.push(OPTIONS.len() + i);
    }

    (options, preselected)
}

/// Interactive first-time init of the NEW layout (R10, AE5). Main-checkout-only.
///
/// Reuses the patterns picker (`ui::pick_patterns`), seeds `magic.json`'s
/// `files` with `default_magic_files()` + the chosen patterns, writes
/// `magic.sh`, ensures the wrapper entry in `config.json`'s `setup` while
/// keeping every other command there ([`init_setup`]), and bootstraps
/// `magic.local.json` + gitignore. Stage → prompt → materialize, same as
/// migration.
///
/// A second picker (`ui::pick_shared_directories`) then asks which dependency
/// directories Orca should share with a new worktree; the chosen patterns are
/// stored as `magic.json`'s `sharedDirectories`, and `orca.yaml` and
/// `.worktreeinclude` are written from them once the `.superset` tree is in
/// place.
///
/// When an existing `magic.json` is present (edit-config path, `Branch::Normal`),
/// custom patterns are preserved in both pickers and preselected so they
/// survive a rewrite.
pub fn run_init(repo_root: &Path, existing: Option<&Config>) -> Result<ExitCode> {
    style::print_section("Initialize .superset (magic layout)");
    println!(
        "{}",
        style::info(format!("Repo root: {}", repo_root.display()))
    );

    // ---- Capture decisions. Nothing written to repo_root yet. ----

    // Read existing base magic.json (absent → empty). Custom patterns are
    // preserved via build_pattern_options so they survive an edit-config rewrite;
    // any unknown top-level keys (KTD8) are carried forward below via
    // `merge_files_into_magic_config` so this same rewrite never drops them.
    let existing_magic = superset_files::load_magic_json(repo_root)?;
    let existing_magic_files: Vec<String> = existing_magic
        .as_ref()
        .map(|m| m.files.clone())
        .unwrap_or_default();

    // Precompute filesystem hits for the preconfigured OPTIONS.
    let options_strs: Vec<&str> = crate::sync::repo_scan::OPTIONS.to_vec();
    let fs_match = crate::sync::repo_scan::matches_for_patterns(repo_root, &options_strs)?;

    let (options, preselected) = build_pattern_options(&existing_magic_files, &fs_match);

    let chosen = ui::pick_patterns(&options, &preselected, repo_root)?;

    // magic.json files = default_magic_files() + chosen (deduped, defaults first).
    let files = init_magic_files(&chosen);

    // ---- Shared directories for Orca: the default kinds plus any stored
    // custom pattern, each shown with the gitignored directories it matches
    // today. A stored choice is what starts checked; with none stored, every
    // kind that matches something does. ----
    let stored_shared = existing_magic
        .as_ref()
        .and_then(|magic| magic.shared_directories.clone());
    let shared_rows = shared_dirs::matches_by_pattern(
        repo_root,
        &shared_dirs::picker_patterns(stored_shared.as_deref()),
    );
    let shared_preselected =
        shared_dirs::picker_preselected(stored_shared.as_deref(), &shared_rows);
    let shared = ui::pick_shared_directories(&shared_rows, &shared_preselected, repo_root)?;

    // ---- Orca output, computed before the prompt so the summary can say
    // whether orca.yaml is created, updated or cannot be edited. ----
    let (orca_patterns, overlay_warning) = overlaid_patterns(repo_root, &files);
    warn_overlay(overlay_warning);
    let orca_plan = orca_write::plan(
        repo_root,
        &orca_patterns,
        &shared,
        SetupPolicy::Ensure,
    );

    println!();
    println!("This will:");
    let mut summary = vec![
        "Write .superset/magic.json (synced patterns)".to_string(),
        "Write .superset/magic.sh (executable wrapper)".to_string(),
        format!(
            "Write .superset/config.json setup → [{}]",
            init_setup(existing).join(", ")
        ),
        local_overlay_line(
            repo_root.join(MAGIC_LOCAL_REL).exists(),
            matches!(git::is_ignored_str(repo_root, MAGIC_LOCAL_REL), Ok(true)),
        ),
    ];
    summary.extend(orca_plan.summary_lines());
    ui::print_pattern_list(&summary);
    println!();

    let action = ui::pick_final_action()?;

    // The picker's answer is always stored, an empty one included: here
    // someone did choose.
    let written = materialize_init(
        repo_root,
        existing,
        existing_magic.as_ref(),
        files,
        Some(shared),
        &orca_plan,
    )?;

    println!();
    println!("{}", style::ok("Wrote .superset/magic.json"));
    println!("{}", style::ok("Wrote .superset/magic.sh"));
    println!("{}", style::ok("Wrote .superset/config.json"));
    if written.had_local {
        println!("{}", style::info("Kept existing .superset/magic.local.json"));
    } else {
        println!("{}", style::ok("Bootstrapped .superset/magic.local.json"));
    }
    println!("{}", style::ok("Gitignored .superset/backups/"));
    println!("{}", style::ok("Gitignored .superset/.magic/"));
    orca_write::print_notices(&written.orca_notices);

    execute_final_action(repo_root, action, INIT_COMMIT_MESSAGE, "chore/ss-magic-init-")
}

/// What [`materialize_init`] did, for the caller's report.
struct InitWritten {
    /// The repository already had a `magic.local.json`, which was kept.
    had_local: bool,
    /// What happened to the Orca files.
    orca_notices: Vec<Notice>,
}

/// Everything init does once its decisions are made – shared by the
/// interactive [`run_init`] (also the edit-config path) and
/// [`run_init_noninteractive`], so the two cannot drift apart.
///
/// Stages `magic.json` (with `files` and `shared` as its two lists – `shared`
/// being `None` leaves `sharedDirectories` out of the file – and every unknown
/// key of `existing_magic` carried over), `magic.sh`, `config.json`
/// (`existing_config`'s `setup` with the wrapper ensured, see [`init_setup`],
/// and its `teardown`/`run` preserved) and, when the repository has none, a
/// fresh `magic.local.json`;
/// copies the stage in; adds the bootstrap gitignore rules; then writes the
/// Orca files per `orca_plan`. Nothing in the repository is touched before the
/// stage is complete.
fn materialize_init(
    repo_root: &Path,
    existing_config: Option<&Config>,
    existing_magic: Option<&MagicConfig>,
    files: Vec<String>,
    shared: Option<Vec<String>>,
    orca_plan: &orca_write::Plan,
) -> Result<InitWritten> {
    let staging = tempfile::Builder::new()
        .prefix("ss-magic-init-")
        .tempdir()
        .context("creating init staging tempdir")?;
    let stage_root = staging.path();
    let magic_cfg = superset_files::merge_files_into_magic_config(existing_magic, files, shared);
    superset_files::write_magic_json(stage_root, &magic_cfg)?;
    superset_files::write_magic_sh(stage_root)?;
    let merged =
        superset_files::merge_setup_into_config(existing_config, init_setup(existing_config));
    superset_files::write_config_json(stage_root, &merged)?;
    // Only seed magic.local.json when the repo doesn't already have one — it's
    // gitignored (unrecoverable), so staging the empty template would clobber a
    // user's existing local overlay when this flow runs as edit-config.
    let had_local = repo_root.join(".superset/magic.local.json").exists();
    if !had_local {
        superset_files::bootstrap_magic_local_json(stage_root)?;
    }

    superset_files::copy_into_repo(stage_root, repo_root, &[])?;
    ensure_bootstrap_gitignores(repo_root)?;
    clear_legacy_skills_install();

    // The Orca files come last, written as they were planned before the
    // prompt. A file that changed on disk since the plan read it is left alone
    // with a warning, since the planned text would discard that change.
    Ok(InitWritten {
        had_local,
        orca_notices: orca_plan.apply(repo_root),
    })
}

/// Non-interactive init for automation (`ss-magic init [PATTERN...]`, AN1):
/// write the magic.json layout from `patterns` without the TUI pickers or the
/// finishing-action prompt. Files land on disk uncommitted (equivalent to the
/// interactive "Done" action); no git operations run. Patterns are seeded into
/// `magic.json` `files` alongside the defaults (`.superset/magic.local.json`),
/// deduped. An existing `magic.local.json` is preserved, not clobbered. Any
/// unknown top-level keys (KTD8) already in `magic.json` survive the rewrite.
///
/// The Orca files are written too. With nobody to ask, the shared-directory
/// patterns are the ones `magic.json` already stores – an empty list included,
/// since that is a choice someone made – and otherwise every default kind that
/// matches a gitignored directory in the checkout. When nothing is stored and
/// nothing is detected the key is left out of `magic.json`, so a later run can
/// still detect directories that did not exist yet.
pub fn run_init_noninteractive(repo_root: &Path, patterns: &[String]) -> Result<ExitCode> {
    let files = init_magic_files(patterns);
    let existing_magic = superset_files::load_magic_json(repo_root)?;
    let existing = superset_files::load_config(repo_root)?;
    let stored_shared = existing_magic
        .as_ref()
        .and_then(|magic| magic.shared_directories.clone());
    let shared = shared_dirs::stored_or_detected(repo_root, stored_shared.as_deref());
    let (orca_patterns, overlay_warning) = overlaid_patterns(repo_root, &files);
    warn_overlay(overlay_warning);
    let orca_plan = orca_write::plan(
        repo_root,
        &orca_patterns,
        shared.as_deref().unwrap_or_default(),
        SetupPolicy::Ensure,
    );

    let written = materialize_init(
        repo_root,
        existing.as_ref(),
        existing_magic.as_ref(),
        files,
        shared,
        &orca_plan,
    )?;

    println!("{}", style::ok("Wrote .superset/magic.json"));
    println!("{}", style::ok("Wrote .superset/magic.sh"));
    println!("{}", style::ok("Wrote .superset/config.json"));
    if !written.had_local {
        println!("{}", style::ok("Bootstrapped .superset/magic.local.json"));
    }
    println!("{}", style::ok("Gitignored .superset/backups/"));
    println!("{}", style::ok("Gitignored .superset/.magic/"));
    orca_write::print_notices(&written.orca_notices);
    println!(
        "{}",
        style::info("Done. Changes are on disk; run `git status` to review.")
    );
    Ok(ExitCode::SUCCESS)
}

/// Run the finishing action chosen at the prompt (Done = changes on disk, not
/// committed; Commit/Branch stage + commit + push/PR). `commit_message` and
/// `branch_prefix` are passed by the caller so migrate and init commit with
/// their own message/branch naming rather than sharing one.
fn execute_final_action(
    repo_root: &Path,
    action: FinalAction,
    commit_message: &str,
    branch_prefix: &str,
) -> Result<ExitCode> {
    match action {
        FinalAction::Done => {
            println!(
                "{}",
                style::info("Done. Changes are on disk; run `git status` to review.")
            );
            Ok(ExitCode::SUCCESS)
        }
        FinalAction::CommitPushMain => {
            git::stage_paths(repo_root, &FINAL_ACTION_PATHS)?;
            if git::nothing_to_commit(repo_root)? {
                println!(
                    "{}",
                    style::info("Nothing to commit — files already match what is tracked.")
                );
                return Ok(ExitCode::SUCCESS);
            }
            git::commit(repo_root, commit_message)?;
            let main_branch = git::main_branch_name(repo_root)?;
            git::push(repo_root, "origin", &main_branch)?;
            println!("{}", style::ok(format!("Pushed to origin/{main_branch}")));
            Ok(ExitCode::SUCCESS)
        }
        FinalAction::FeatureBranchPR => {
            git::stage_paths(repo_root, &FINAL_ACTION_PATHS)?;
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
            let branch = format!("{branch_prefix}{suffix}");
            git::create_branch(repo_root, &branch)?;
            git::commit(repo_root, commit_message)?;
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

/// What a committing finishing action stages: the `.superset` contract, the
/// `.gitignore` rules, and `orca.yaml`, which is committed so the whole team's
/// Orca worktrees run the setup script. `.worktreeinclude` is deliberately NOT
/// here – it is a per-machine snapshot, gitignored by the same operation.
const FINAL_ACTION_PATHS: [&str; 3] = [".superset", ".gitignore", orca_write::ORCA_YAML];

const MIGRATE_COMMIT_MESSAGE: &str = "chore(superset): migrate to ss-magic layout";
const INIT_COMMIT_MESSAGE: &str = "chore(superset): initialize ss-magic layout";

#[cfg(test)]
mod tests;
