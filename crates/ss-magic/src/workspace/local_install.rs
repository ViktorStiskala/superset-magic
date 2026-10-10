//! Local (uncommitted) install: `ss-magic init --local [PATTERN...]` and the
//! main-checkout menu's "Initialize ss-magic locally" / edit entry.
//!
//! A local install lets one developer use ss-magic on a repository without
//! proposing anything to the team. It writes only gitignored files into the
//! MAIN checkout:
//!
//! - `.superset/magic.local.json` – the pattern list. A local install has no
//!   `magic.json`, so this file alone is what sync, reverse sync and pack read
//!   (core's `superset_files::load_sync_config`).
//! - `.superset/config.local.json` – Superset's per-machine override of the
//!   committed `config.json`. The install registers `ss-magic sync` in its
//!   `setup` key so Superset runs the binary directly for every new workspace.
//!   No `magic.sh` wrapper is written: it would be a committed file.
//!
//! Every ignore rule goes to the repository's shared `<git-common-dir>/info/exclude`
//! (core's `IgnoreSink::LocalExclude`), which git reads for the main checkout
//! and every linked worktree but never tracks, so `git status` stays empty.
//!
//! Three rules shape the flow:
//!
//! - **Always the main checkout** (KTD6 in the plan: the install targets the
//!   main checkout even when run from a linked worktree, because Superset reads
//!   the main checkout's `config.local.json` for a new workspace and sync loads
//!   its patterns from the main root – an install written into a worktree would
//!   never be read).
//! - **Refuse beside a committed install** (R9): a `magic.json`, a
//!   `config.json` `setup` that `migrate::detect_branch` recognizes, either
//!   local file being TRACKED under any capitalization, or `.superset` or
//!   either local file being a symlink (both would let the install dirty the
//!   working tree) refuses with exit 1 before anything is written.
//! - **Validate everything, then ignore, then write** (KTD7 in the plan: there
//!   is no commit prompt because there is nothing to commit; every input is
//!   parsed and merged in memory first, the `info/exclude` rules are written and
//!   verified next, and only then do the two JSON files land – so a failure
//!   part-way leaves at most untracked exclude lines, never an unignored local
//!   file that `git status` would show).
//!
//! The interactive entry REPLACES the pattern list with the picker selection
//! (it doubles as the edit entry, so deselecting removes a pattern); the
//! non-interactive entry APPENDS its patterns to the existing list (KTD8 in the
//! plan). Unlike committed init, neither runs the legacy `~/.claude/skills`
//! cleanup: that belongs to the committed install's lifecycle.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::git;
use crate::git::gitignore::{self, IgnoreSink, PathKind};
use crate::sync::reverse_sync::BACKUPS_REL;
use crate::tui::style;
use crate::tui::ui;
use crate::workspace::migrate::{self, push_missing, Branch};
use crate::workspace::superset_files::{
    self, InstallMode, LocalConfigMerge, MagicConfig, CONFIG_LOCAL_PATTERN, MAGIC_LOCAL_PATTERN,
};
use ss_magic_core::state_tree::STATE_REL;

/// The `.superset` directory both local files live in, relative to the root.
const SUPERSET_DIR_REL: &str = ".superset";

/// The binary Superset must be able to resolve when it runs the registered
/// `ss-magic sync` setup step.
const BIN_NAME: &str = "ss-magic";

/// Everything a local install will write, computed in memory before any write
/// so a malformed input aborts with nothing on disk.
struct Prepared {
    /// The existing `magic.local.json`, if any: its `files` seed the new list
    /// and its unknown keys (including `_comment`) are carried forward.
    existing_local: Option<MagicConfig>,
    /// The merged `config.local.json` and whether it differs from the file on
    /// disk (an unchanged file is never rewritten).
    config_local: LocalConfigMerge,
}

/// Interactive local install, behind the main-checkout menu's "Initialize
/// ss-magic locally" and "Edit synced files" entries on a local install.
///
/// Resolves the main checkout from `cwd_root`, refuses beside a committed
/// install, validates `config.local.json`, then opens the pattern picker seeded
/// from the existing `magic.local.json`. The picker is the confirmation: Esc
/// there returns before anything is written. The selection REPLACES the list
/// (the local defaults are always kept first).
pub fn run_local_init(cwd_root: &Path) -> Result<ExitCode> {
    style::print_section("Initialize ss-magic locally (uncommitted)");
    let Some((main_root, prepared)) = begin(cwd_root)? else {
        return Ok(ExitCode::from(1));
    };

    // The local defaults are always written, so they are not offered as
    // picker rows; every other existing pattern is (custom ones preselected).
    let existing_files: Vec<String> = prepared
        .existing_local
        .as_ref()
        .map(|cfg| without_local_defaults(&cfg.files))
        .unwrap_or_default();
    let options_strs: Vec<&str> = crate::sync::repo_scan::OPTIONS.to_vec();
    let fs_match = crate::sync::repo_scan::matches_for_patterns(&main_root, &options_strs)?;
    let (options, preselected) = migrate::build_pattern_options(&existing_files, &fs_match);
    let chosen = ui::pick_patterns(&options, &preselected, &main_root)?;

    let files = selection_files(&chosen);
    println!();
    println!("This will:");
    ui::print_pattern_list(&summary_lines(&prepared));
    println!();

    commit(&main_root, prepared, files)?;
    finish();
    Ok(ExitCode::SUCCESS)
}

/// Non-interactive `ss-magic init --local [PATTERN...]`: the same install
/// without the picker. `patterns` are APPENDED to the existing
/// `magic.local.json` list (a re-run never drops a pattern), after the local
/// defaults.
pub fn run_local_init_noninteractive(cwd_root: &Path, patterns: &[String]) -> Result<ExitCode> {
    let Some((main_root, prepared)) = begin(cwd_root)? else {
        return Ok(ExitCode::from(1));
    };
    let existing_files: &[String] = prepared
        .existing_local
        .as_ref()
        .map(|cfg| cfg.files.as_slice())
        .unwrap_or_default();
    let files = appended_files(existing_files, patterns);

    commit(&main_root, prepared, files)?;
    finish();
    Ok(ExitCode::SUCCESS)
}

/// The preamble both entries share, in order: resolve the main checkout, name
/// it on screen, refuse (printing why) when a local install must not run there,
/// then [`prepare`] both inputs in memory. `None` means a refusal was already
/// printed and the caller exits 1; nothing has been written by then.
fn begin(cwd_root: &Path) -> Result<Option<(PathBuf, Prepared)>> {
    let main_root = git::main_checkout_root(cwd_root)?;
    println!(
        "{}",
        style::info(format!("Main checkout: {}", main_root.display()))
    );

    if let Some(reason) = refusal(&main_root)? {
        eprintln!("{}", style::err(format!("error: {reason}")));
        return Ok(None);
    }
    let prepared = prepare(&main_root)?;
    Ok(Some((main_root, prepared)))
}

/// Why a local install must not run in `main_root`, or `None` when it may.
///
/// Refuses (R9) when the main checkout already carries a committed install –
/// a `magic.json`, or a `config.json` `setup` that the committed flow's
/// [`migrate::detect_branch`] classifies as `Migrate` (the retired `setup.sh`)
/// or `Normal` (the `magic.sh` / `ss-magic sync` marker) – or when the install
/// could end up writing a tracked file, which would show up in `git status`
/// when a local install must change no tracked file:
///
/// - `.superset` or either local file is a symlink ([`symlink_refusal`]): the
///   JSON writers rename onto a link's resolved target, so an untracked link to
///   `config.json` would rewrite that tracked file. An `lstat` failure refuses
///   too.
/// - either local file is tracked under any capitalization
///   ([`tracked_refusal`]): on a case-insensitive filesystem the canonical
///   path would rewrite a differently-cased tracked entry.
///
/// A malformed `config.json` is an error naming the path, never a guess.
fn refusal(main_root: &Path) -> Result<Option<String>> {
    let mode = superset_files::install_mode(main_root);
    if mode == InstallMode::Committed {
        return Ok(Some(
            "this repository already has a committed ss-magic install (.superset/magic.json). \
             Run `ss-magic` in the main checkout to edit it instead of adding a local install."
                .to_string(),
        ));
    }
    let config = superset_files::load_config(main_root)?;
    match migrate::detect_branch(config.as_ref(), mode) {
        Branch::Migrate => {
            return Ok(Some(
                ".superset/config.json still runs the retired setup.sh: this repository has an \
                 old committed install. Run `ss-magic` in the main checkout to migrate it \
                 instead of adding a local install."
                    .to_string(),
            ))
        }
        Branch::Normal => {
            return Ok(Some(
                ".superset/config.json's setup already runs ss-magic (a committed install). \
                 Run `ss-magic` in the main checkout to edit it instead of adding a local install."
                    .to_string(),
            ))
        }
        // `Local` is this module's own install being re-run or edited.
        Branch::Init | Branch::Local => {}
    }
    if let Some(reason) = symlink_refusal(main_root)? {
        return Ok(Some(reason));
    }
    tracked_refusal(main_root)
}

/// Refuse when `.superset`, `.superset/magic.local.json` or
/// `.superset/config.local.json` in `main_root` is a symlink.
///
/// The JSON writers commit through a staged sibling plus `rename` onto the
/// path's RESOLVED target (core's `write_atomically`, which follows links on
/// purpose for the plugin's config writer). The tracked-file check only sees
/// the link's own name, so an untracked `config.local.json -> config.json`
/// link would pass it and the install would rewrite the tracked
/// `config.json`. Refusing every link is stricter than resolving and
/// re-checking the target, and also covers a link that leaves the repository.
/// An `lstat` failure other than "not found" refuses too: an unknown answer
/// must never let the write through.
fn symlink_refusal(main_root: &Path) -> Result<Option<String>> {
    for rel in [SUPERSET_DIR_REL, MAGIC_LOCAL_PATTERN, CONFIG_LOCAL_PATTERN] {
        match std::fs::symlink_metadata(main_root.join(rel)) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Ok(Some(format!(
                    "{rel} is a symlink. A local install writes its files in place and never \
                     through a link, which could rewrite a tracked file. Replace it with a \
                     regular file or directory, then re-run."
                )))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Ok(Some(format!(
                    "could not check whether {rel} is a symlink ({e}), so a local install \
                     will not write it"
                )))
            }
        }
    }
    Ok(None)
}

/// Refuse when the index tracks either local file under ANY capitalization.
///
/// A literal `git ls-files -- .superset/config.local.json` misses an index
/// entry spelled `.superset/CONFIG.LOCAL.JSON`, yet on a case-insensitive
/// filesystem (macOS by default) writing the lowercase path rewrites that
/// tracked file. So the whole index is listed and each entry compared with
/// core's case-insensitive [`superset_files::rel_eq_ignore_ascii_case`] (the
/// same rule reverse sync's forward-only filter uses). `:(icase)` pathspec
/// magic is deliberately not used: `GIT_LITERAL_PATHSPECS=1` in the
/// environment turns magic off, the pathspec then matches nothing, and the
/// check would pass when it must refuse. Listing the index once is cheap for
/// a one-shot command.
fn tracked_refusal(main_root: &Path) -> Result<Option<String>> {
    let tracked = git::tracked_files(main_root, &[])?;
    let hit = tracked.iter().find(|path| {
        [MAGIC_LOCAL_PATTERN, CONFIG_LOCAL_PATTERN]
            .iter()
            .any(|rel| superset_files::rel_eq_ignore_ascii_case(path, Path::new(rel)))
    });
    Ok(hit.map(|path| {
        format!(
            "{} is tracked by git, and a local install must not change a tracked file. \
             Untrack it first (`git rm --cached {}`), then re-run. If it is a symlink, \
             also replace it with a regular file.",
            path.display(),
            path.display()
        )
    }))
}

/// Load and validate both inputs in memory: the existing `magic.local.json`
/// (for its patterns and unknown keys) and the `config.local.json` merge. Any
/// parse or merge failure returns here, before a single byte is written.
fn prepare(main_root: &Path) -> Result<Prepared> {
    let existing_local = superset_files::load_magic_local_json(main_root)?;
    let existing_config = superset_files::load_config_local_json(main_root)?;
    let config_local = superset_files::merge_sync_entry_into_local_config(existing_config.as_ref())?;
    Ok(Prepared {
        existing_local,
        config_local,
    })
}

/// Write the install in KTD7 order: first the four `info/exclude` rules
/// (each verified by git, so a tracked `.gitignore` negation that would expose
/// a local file fails loudly here), then `magic.local.json`, then
/// `config.local.json` only when its merge changed something.
fn commit(main_root: &Path, prepared: Prepared, files: Vec<String>) -> Result<()> {
    let rules: [(&str, PathKind); 4] = [
        (MAGIC_LOCAL_PATTERN, PathKind::File),
        (CONFIG_LOCAL_PATTERN, PathKind::File),
        (BACKUPS_REL, PathKind::Dir),
        (STATE_REL, PathKind::Dir),
    ];
    for (rel, kind) in rules {
        gitignore::ensure_path_ignored_in(
            IgnoreSink::LocalExclude,
            main_root,
            main_root,
            Path::new(rel),
            kind,
        )?;
    }

    let magic_local =
        superset_files::merge_files_into_magic_config(prepared.existing_local.as_ref(), files);
    superset_files::write_magic_local_json(main_root, &magic_local)?;
    let config_changed = prepared.config_local.changed;
    if config_changed {
        superset_files::write_config_local_json(main_root, &prepared.config_local.config)?;
    }

    println!("{}", style::ok("Excluded the local files via .git/info/exclude"));
    println!("{}", style::ok(format!("Wrote {MAGIC_LOCAL_PATTERN}")));
    if config_changed {
        println!(
            "{}",
            style::ok(format!(
                "Registered `{}` in {CONFIG_LOCAL_PATTERN}",
                superset_files::LOCAL_SYNC_ENTRY
            ))
        );
    } else {
        println!(
            "{}",
            style::info(format!(
                "{CONFIG_LOCAL_PATTERN} already runs `{}`",
                superset_files::LOCAL_SYNC_ENTRY
            ))
        );
    }
    Ok(())
}

/// The closing lines shared by both entries: the KTD10 `PATH` advisory, then
/// the "nothing to commit" note.
fn finish() {
    if !resolves_on_path(std::env::var_os("PATH").as_deref(), BIN_NAME) {
        println!("{}", style::warn(path_warning()));
    }
    println!(
        "{}",
        style::info("Done. Nothing to commit: every file written is ignored by git.")
    );
}

/// The on-screen summary of what the interactive install will write.
fn summary_lines(prepared: &Prepared) -> Vec<String> {
    let mut lines = vec![
        "Exclude the local files via .git/info/exclude (never a tracked .gitignore)".to_string(),
        format!("Write {MAGIC_LOCAL_PATTERN} (the synced patterns)"),
    ];
    if prepared.config_local.changed {
        lines.push(format!(
            "Register `{}` in {CONFIG_LOCAL_PATTERN} setup",
            superset_files::LOCAL_SYNC_ENTRY
        ));
    }
    lines
}

/// The interactive entry's new list: the local defaults, then the picker
/// selection, deduped. Pure so the replace semantic is testable without the
/// prompt.
fn selection_files(chosen: &[String]) -> Vec<String> {
    let mut files = superset_files::local_install_default_files();
    push_missing(&mut files, chosen);
    files
}

/// The non-interactive entry's new list: the existing list in its existing
/// order, then any local default it lacks, then the new `patterns`, deduped.
/// A fresh install therefore lists the defaults first; a re-run only appends.
fn appended_files(existing: &[String], patterns: &[String]) -> Vec<String> {
    let mut files = Vec::with_capacity(existing.len() + patterns.len() + 2);
    push_missing(&mut files, existing);
    push_missing(&mut files, &superset_files::local_install_default_files());
    push_missing(&mut files, patterns);
    files
}

/// `files` without the local defaults, which the picker does not offer since
/// [`selection_files`] always writes them. Original order is kept.
fn without_local_defaults(files: &[String]) -> Vec<String> {
    let defaults = superset_files::local_install_default_files();
    let defaults: Vec<&str> = defaults.iter().map(String::as_str).collect();
    superset_files::existing_unknown_entries(files, &defaults)
}

/// Whether `name` resolves to an executable file through the `PATH` value
/// `path_var` (`None` = `PATH` unset). Takes the value rather than reading the
/// environment so it is testable without mutating the test process's `PATH`.
///
/// KTD10 in the plan: this is advisory only. It catches an invocation by
/// absolute path (`./target/release/ss-magic init --local`), but it cannot prove
/// that the shell Superset runs `ss-magic sync` in resolves the binary.
///
/// Each directory is probed for the bare name and for the name plus the
/// platform's executable suffix (`std::env::consts::EXE_SUFFIX`): on Windows the
/// installed binary is `ss-magic.exe`, which a shell runs as `ss-magic`. On unix
/// the suffix is empty, so the second probe repeats the first.
fn resolves_on_path(path_var: Option<&OsStr>, name: &str) -> bool {
    let Some(path_var) = path_var else {
        return false;
    };
    let with_suffix = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path_var).any(|dir| {
        is_executable_file(&dir.join(name)) || is_executable_file(&dir.join(&with_suffix))
    })
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn path_warning() -> String {
    format!(
        "`{BIN_NAME}` is not on this shell's PATH. Superset runs `{}` from \
         {CONFIG_LOCAL_PATTERN} for every new workspace, so make sure the environment it \
         uses resolves `{BIN_NAME}`, or workspace setup will fail.",
        superset_files::LOCAL_SYNC_ENTRY
    )
}

#[cfg(test)]
mod tests;
