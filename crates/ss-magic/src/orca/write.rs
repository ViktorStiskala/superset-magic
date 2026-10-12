//! Writing the Orca files into the main checkout.
//!
//! Two writers that share one rule about where a write may land:
//!
//! - [`plan`] and [`Plan::apply`] are the shared Orca write path behind init,
//!   migrate, edit-config and the explicit `ss-magic orca` refresh (KTD13). The
//!   Orca output is COMPUTED before the finishing prompt and WRITTEN after the
//!   Superset tree has materialized. `plan` reads the current `orca.yaml` and
//!   edits it in memory, computes the snapshot and renders `.worktreeinclude`,
//!   so the "This will:" list can say whether `orca.yaml` is created, updated
//!   or cannot be edited and how many files Orca will copy. `apply` then writes
//!   exactly what was planned, and gitignores `.worktreeinclude`.
//! - [`refresh_snapshot`] is the AUTOMATIC refresh that ends a forward sync in
//!   an Orca worktree (KTD11). It rewrites only the managed section of main's
//!   `.worktreeinclude`, only when git positively reports that file as ignored
//!   and untracked, and it reports rather than fails: a background setup script
//!   must never leave the shared main checkout with a modified tracked file, and
//!   must never turn a successful sync into a failed one.
//!
//! Neither writer ever fails the operation it belongs to. A file that cannot be
//! edited safely is left byte-for-byte untouched and reported, and everything
//! else is still written (R22).
//!
//! Every write goes through [`write_contained`], which refuses a target that
//! resolves outside the repository through a symlink (KTD13). The hazard is a
//! repository that commits `orca.yaml` or `.worktreeinclude` as a link to a file
//! the person owns elsewhere; a command run in that checkout would otherwise
//! follow the link and rewrite THAT file.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::git;
use crate::git::gitignore::{self, Ignored, PathKind};
use crate::orca::shared_dirs::{self, Expansion};
use crate::orca::worktreeinclude::{self, Document, Existing};
use crate::orca::{orca_yaml, snapshot};
use crate::tui::style;
use crate::workspace::superset_files;

/// The name of Orca's configuration file, at the root of the main checkout.
pub const ORCA_YAML: &str = "orca.yaml";

/// Where a write to `<root>/<rel>` would actually land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// Inside the repository: the file does not exist yet, or it exists and
    /// resolves inside – possibly through a symlink that stays in the tree,
    /// which the writer follows onto its target.
    Inside,
    /// The file resolves OUTSIDE the repository through a symlink, to this
    /// path. No writer may proceed.
    Outside(PathBuf),
}

/// Decide where `<root>/<rel>` really is before a writer follows the path.
///
/// Both sides are canonicalized, which resolves every symlink in one step and
/// keeps the comparison honest when the repository itself sits behind a link
/// (macOS's `/tmp`). `rel` names a file directly under `root`, so a file that
/// does not exist yet is created in `root` itself and nothing on the path can
/// redirect it. A dangling link reads as "does not exist"; the atomic writer
/// then replaces the link with a regular file, inside the repository. A root
/// that cannot be resolved is an error rather than a pass – the unknown answer
/// must be the refusing one.
pub fn landing(root: &Path, rel: &str) -> Result<Landing> {
    let real_root = root
        .canonicalize()
        .with_context(|| format!("resolving the repository root {}", root.display()))?;
    match root.join(rel).canonicalize() {
        Ok(real_file) if !real_file.starts_with(&real_root) => Ok(Landing::Outside(real_file)),
        _ => Ok(Landing::Inside),
    }
}

/// Replace `<root>/<rel>` with `text` atomically, unless that would write
/// outside the repository. Returns where the write landed, so the caller can
/// report an `Outside` refusal instead of treating it as a failure.
pub fn write_contained(root: &Path, rel: &str, text: &str) -> Result<Landing> {
    let landing = landing(root, rel)?;
    if landing == Landing::Inside {
        superset_files::write_atomically(&root.join(rel), text)?;
    }
    Ok(landing)
}

/// Whether an operation may touch `scripts.setup` in `orca.yaml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupPolicy {
    /// Make sure the script contains the `magic.sh sync` line. Init, migrate
    /// and edit-config: the person is setting the repository up, so a line that
    /// went missing comes back.
    Ensure,
    /// Write the line only into an `orca.yaml` this operation creates; leave
    /// the script of an existing file exactly as it is (R25). The explicit
    /// refresh: it must not undo someone's deliberate removal of the line, yet
    /// it is also how a repository set up before Orca support gets a working
    /// file in one step.
    OnlyWhenCreating,
}

/// How loudly a [`Notice`] is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Something was written.
    Ok,
    /// Something worth knowing that needs no action.
    Info,
    /// Something was NOT done, or the person should act.
    Warn,
}

/// One line of what the Orca write path did, for the caller to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

impl Notice {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            text: text.into(),
        }
    }

    fn info(text: impl Into<String>) -> Self {
        Self {
            level: Level::Info,
            text: text.into(),
        }
    }

    fn warn(text: impl Into<String>) -> Self {
        Self {
            level: Level::Warn,
            text: text.into(),
        }
    }
}

/// Print notices in order: warnings to stderr, everything else to stdout.
pub fn print_notices(notices: &[Notice]) {
    for notice in notices {
        match notice.level {
            Level::Ok => println!("{}", style::ok(&notice.text)),
            Level::Info => println!("{}", style::info(&notice.text)),
            Level::Warn => eprintln!("{}", style::warn(&notice.text)),
        }
    }
}

/// What will happen to `orca.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum YamlPlan {
    /// Write this text. `creating` says the file does not exist yet.
    Write { text: String, creating: bool },
    /// The file already holds the intended values.
    Unchanged,
    /// Leave the file byte-for-byte untouched, for `reason`. `manual_block` is
    /// what the person can add by hand to get the same effect.
    Untouched {
        reason: String,
        manual_block: Option<String>,
    },
}

/// What will happen to `.worktreeinclude`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IncludePlan {
    /// Write this text, whose managed section lists `listed` files.
    Write { text: String, listed: usize },
    /// The file already holds exactly this section.
    Unchanged { listed: usize },
    /// Leave the file byte-for-byte untouched, for the reason given.
    Untouched(String),
}

/// The Orca output of one init / migrate / edit-config / refresh, computed
/// before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    yaml: YamlPlan,
    include: IncludePlan,
    /// The directories `orca.yaml` will share once this plan is applied.
    shared: Expansion,
    /// The Orca-limit notice for files the snapshot could not hold, if any.
    snapshot_warning: Option<String>,
}

/// Compute what the Orca files of `main_root` should become. Nothing is
/// written here.
///
/// `patterns` are the overlaid sync patterns as they WILL be once the operation
/// completes – the `files` it is about to write into `magic.json`, plus the
/// local overlay. They are passed in rather than read back because the plan is
/// made before `magic.json` is materialized. `shared_patterns` are the
/// `sharedDirectories` globs as they will be stored; they are expanded against
/// the checkout right now, and the result replaces
/// `worktree.sharedDirectories` outright (R21).
///
/// Because the snapshot is taken here, before the `.superset` tree is written,
/// it describes the main checkout as it stands at that moment. A file the same
/// operation creates afterwards – the `magic.local.json` a first init
/// bootstraps – is not in it yet, and enters with the next refresh.
pub fn plan(
    main_root: &Path,
    patterns: &[String],
    shared_patterns: &[String],
    setup: SetupPolicy,
) -> Plan {
    let shared = shared_dirs::expand(main_root, shared_patterns);
    let (yaml, shared_on_disk) = plan_yaml(main_root, &shared.dirs, setup);
    // The snapshot leaves out files under a directory Orca shares, and "shares"
    // means whatever list `orca.yaml` will hold: the new one when the file is
    // written, the one already on disk when it is left untouched.
    let listed_dirs = match &yaml {
        YamlPlan::Write { .. } | YamlPlan::Unchanged => &shared.dirs,
        YamlPlan::Untouched { .. } => &shared_on_disk,
    };
    let (include, snapshot_warning) = plan_include(main_root, patterns, listed_dirs);
    Plan {
        yaml,
        include,
        shared,
        snapshot_warning,
    }
}

fn outside_reason(target: &Path) -> String {
    format!(
        "it is a symlink to {}, outside the repository",
        target.display()
    )
}

/// The `.worktreeinclude` half of [`plan`]: the rendered file, or why it is
/// left alone, plus the Orca-limit notice. The containment check comes BEFORE
/// the read, and a file that cannot be read as UTF-8 text is never rebuilt from
/// an empty parse – that would drop the person's own lines along with whatever
/// made it unreadable.
fn plan_include(
    main_root: &Path,
    patterns: &[String],
    shared_dirs: &[String],
) -> (IncludePlan, Option<String>) {
    match landing(main_root, worktreeinclude::FILE_NAME) {
        Ok(Landing::Inside) => {}
        Ok(Landing::Outside(target)) => {
            return (IncludePlan::Untouched(outside_reason(&target)), None);
        }
        Err(err) => return (IncludePlan::Untouched(format!("{err:#}")), None),
    }
    let existing = worktreeinclude::read(main_root);
    let document = match &existing {
        Existing::Absent => Document::empty(),
        Existing::Text(text) => Document::parse(text),
        Existing::Unreadable(why) => return (IncludePlan::Untouched(why.clone()), None),
    };
    let snap = snapshot::compute(main_root, patterns, shared_dirs, document.budget());
    let warning = snapshot::overflow_warning(&snap);
    let text = document.render(&snap.entries);
    let listed = snap.entries.len();
    let plan = if matches!(&existing, Existing::Text(current) if *current == text) {
        IncludePlan::Unchanged { listed }
    } else {
        IncludePlan::Write { text, listed }
    };
    (plan, warning)
}

/// The `orca.yaml` half of [`plan`], plus the directory list the file on disk
/// holds now. The containment check comes BEFORE the read: a link out of the
/// repository is not even opened.
fn plan_yaml(main_root: &Path, dirs: &[String], setup: SetupPolicy) -> (YamlPlan, Vec<String>) {
    let untouched = |reason: String, manual: bool, ensure_setup: bool| YamlPlan::Untouched {
        reason,
        manual_block: manual.then(|| orca_yaml::managed_block(dirs, ensure_setup)),
    };
    match landing(main_root, ORCA_YAML) {
        Ok(Landing::Inside) => {}
        Ok(Landing::Outside(target)) => {
            return (
                untouched(outside_reason(&target), true, setup == SetupPolicy::Ensure),
                Vec::new(),
            );
        }
        Err(err) => return (untouched(format!("{err:#}"), false, false), Vec::new()),
    }
    let current = match fs::read(main_root.join(ORCA_YAML)) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                return (
                    untouched(
                        "it is not valid UTF-8 text".to_string(),
                        true,
                        setup == SetupPolicy::Ensure,
                    ),
                    Vec::new(),
                );
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return (
                untouched(format!("it could not be read ({err})"), false, false),
                Vec::new(),
            );
        }
    };
    let on_disk = current
        .as_deref()
        .map(orca_yaml::shared_directories)
        .unwrap_or_default();
    let ensure_setup = match setup {
        SetupPolicy::Ensure => true,
        SetupPolicy::OnlyWhenCreating => current.is_none(),
    };
    let plan = match orca_yaml::edit(current.as_deref(), dirs, ensure_setup) {
        Ok(text) if current.as_deref() == Some(text.as_str()) => YamlPlan::Unchanged,
        Ok(text) => YamlPlan::Write {
            text,
            creating: current.is_none(),
        },
        Err(refusal) => YamlPlan::Untouched {
            reason: refusal.reason,
            manual_block: Some(refusal.manual_block),
        },
    };
    (plan, on_disk)
}

/// "N file(s) for Orca to copy".
fn file_count(listed: usize) -> String {
    format!(
        "{listed} {} for Orca to copy",
        if listed == 1 { "file" } else { "files" }
    )
}

impl Plan {
    /// The lines this plan adds to an operation's "This will:" list. Pure, so
    /// the wording is testable without a prompt.
    pub fn summary_lines(&self) -> Vec<String> {
        let count = self.shared.dirs.len();
        let shared = format!(
            "{count} shared {}",
            if count == 1 { "directory" } else { "directories" }
        );
        let yaml = match &self.yaml {
            YamlPlan::Write { creating: true, .. } => {
                format!("Create orca.yaml (Orca setup script, {shared})")
            }
            YamlPlan::Write {
                creating: false, ..
            } => format!("Update orca.yaml (Orca setup script, {shared})"),
            YamlPlan::Unchanged => "Keep orca.yaml (already up to date)".to_string(),
            YamlPlan::Untouched { reason, .. } => {
                format!("Cannot edit orca.yaml ({reason}); it is left untouched")
            }
        };
        let name = worktreeinclude::FILE_NAME;
        let include = match &self.include {
            IncludePlan::Write { listed, .. } => {
                format!("Write {name} ({}) + add it to .gitignore", file_count(*listed))
            }
            IncludePlan::Unchanged { listed } => format!(
                "Keep {name} (already up to date, {}) + add it to .gitignore",
                file_count(*listed)
            ),
            IncludePlan::Untouched(why) => format!("Leave {name} untouched ({why})"),
        };
        vec![yaml, include]
    }

    /// Write the planned Orca files and report what happened. Never fails:
    /// every file that cannot be written is left untouched and reported, and
    /// the rest is still written (R22).
    ///
    /// Called after the Superset tree has materialized. `orca.yaml` goes first.
    /// Then `.worktreeinclude` is gitignored – skipped, with a notice, when the
    /// file is tracked, since a rule cannot ignore a tracked file and would
    /// only mislead – and the planned text is written.
    pub fn apply(&self, main_root: &Path) -> Vec<Notice> {
        let mut notices = Vec::new();
        self.apply_yaml(main_root, &mut notices);
        if let Some(warning) = shared_dirs::overflow_warning(&self.shared) {
            notices.push(Notice::warn(warning));
        }
        self.apply_include(main_root, &mut notices);
        if let Some(warning) = &self.snapshot_warning {
            notices.push(Notice::warn(warning.clone()));
        }
        notices
    }

    /// Write `orca.yaml` per the plan.
    fn apply_yaml(&self, main_root: &Path, notices: &mut Vec<Notice>) {
        let untouched = |notices: &mut Vec<Notice>, reason: &str, manual: Option<&str>| {
            notices.push(Notice::warn(format!("orca.yaml was not changed: {reason}.")));
            if let Some(block) = manual {
                notices.push(Notice::info(format!(
                    "To set it up by hand, add this to orca.yaml:\n{}",
                    indent(block)
                )));
            }
        };
        match &self.yaml {
            YamlPlan::Unchanged => notices.push(Notice::info("orca.yaml is already up to date")),
            YamlPlan::Untouched {
                reason,
                manual_block,
            } => untouched(notices, reason, manual_block.as_deref()),
            YamlPlan::Write { text, creating } => {
                match write_contained(main_root, ORCA_YAML, text) {
                    Ok(Landing::Inside) => notices.push(Notice::ok(format!(
                        "{} orca.yaml (commit it so every Orca worktree gets the setup script)",
                        if *creating { "Created" } else { "Updated" }
                    ))),
                    // The target changed between planning and writing.
                    Ok(Landing::Outside(target)) => {
                        untouched(notices, &outside_reason(&target), None);
                    }
                    Err(err) => untouched(notices, &format!("writing it failed ({err:#})"), None),
                }
            }
        }
    }

    /// Gitignore `.worktreeinclude` and write the planned text.
    fn apply_include(&self, main_root: &Path, notices: &mut Vec<Notice>) {
        let name = worktreeinclude::FILE_NAME;
        let (text, listed) = match &self.include {
            IncludePlan::Untouched(why) => {
                notices.push(Notice::warn(format!(
                    "{name} was left untouched because {why}."
                )));
                return;
            }
            IncludePlan::Write { text, listed } => (Some(text), *listed),
            IncludePlan::Unchanged { listed } => (None, *listed),
        };

        // Positive tracked determination; "git could not be asked" (no
        // repository) reads as untracked, and the rule is then written.
        let tracked = git::tracked_files(main_root, &[name]).is_ok_and(|list| !list.is_empty());
        if tracked {
            notices.push(Notice::info(format!(
                "{name} is tracked by git, so no ignore rule was added. Its contents are a \
                 per-machine snapshot; consider `git rm --cached {name}`."
            )));
        } else {
            match gitignore::ensure_path_ignored(
                main_root,
                main_root,
                Path::new(name),
                PathKind::File,
            ) {
                Ok(Ignored::Appended) => notices.push(Notice::ok(format!("Gitignored {name}"))),
                Ok(Ignored::Already) => {}
                Err(err) => notices.push(Notice::warn(format!(
                    "Could not add {name} to .gitignore: {err:#}"
                ))),
            }
        }

        let count = file_count(listed);
        let Some(text) = text else {
            notices.push(Notice::info(format!("{name} is already up to date ({count})")));
            return;
        };
        match write_contained(main_root, name, text) {
            Ok(Landing::Inside) => notices.push(Notice::ok(format!("Wrote {name} ({count})"))),
            Ok(Landing::Outside(target)) => notices.push(Notice::warn(format!(
                "{name} was left untouched because {}.",
                outside_reason(&target)
            ))),
            Err(err) => notices.push(Notice::warn(format!("{name} was not written: {err:#}."))),
        }
    }
}

/// Indent every line of `block` by four spaces, for showing it under a notice.
fn indent(block: &str) -> String {
    block
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the automatic snapshot refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotRefresh {
    /// The managed section was rewritten with `listed` files. `warning` is the
    /// Orca-limit notice when some files did not fit.
    Written {
        listed: usize,
        warning: Option<String>,
    },
    /// The file already says exactly this; nothing was written.
    Unchanged { warning: Option<String> },
    /// Nothing was written, for the reason given.
    Skipped(String),
}

/// Regenerate the managed section of `<main_root>/.worktreeinclude` from the
/// overlaid sync `patterns`, and touch nothing else – in particular never
/// `orca.yaml` (R12, AE6).
///
/// Best-effort by contract (R26): every reason not to write is a
/// [`SnapshotRefresh::Skipped`], never an error, so the forward sync that calls
/// this keeps its exit status. The write happens only when ALL of these hold:
///
/// - git positively reports `.worktreeinclude` as untracked (an empty
///   `ls-files` answer for it) AND as ignored. A tracked file would show up as
///   modified in the shared main checkout; an unignored one would show up as
///   untracked. Either would be a surprise left behind by a background script,
///   and "git could not be asked" counts as a no.
/// - the existing file, if any, can be read as UTF-8 text. Rebuilding an
///   unreadable file from an empty parse would drop the person's lines.
/// - the file does not resolve outside the repository through a symlink.
///
/// The files under a directory Orca already shares are left out of the
/// snapshot. "Already shares" is read from `orca.yaml` as it is on disk, since
/// that – not what `magic.json` would expand to today – is the list Orca uses
/// until someone runs the explicit refresh.
pub fn refresh_snapshot(main_root: &Path, patterns: &[String]) -> SnapshotRefresh {
    let name = worktreeinclude::FILE_NAME;
    match git::tracked_files(main_root, &[name]) {
        Ok(tracked) if tracked.is_empty() => {}
        Ok(_) => {
            return SnapshotRefresh::Skipped(format!(
                "the main checkout's `{name}` is tracked by git, and the automatic refresh never \
                 rewrites a tracked file. Run `ss-magic orca` there to update it."
            ));
        }
        Err(err) => {
            return SnapshotRefresh::Skipped(format!(
                "git could not say whether the main checkout's `{name}` is tracked ({err:#})"
            ));
        }
    }
    match git::is_ignored_str(main_root, name) {
        Ok(true) => {}
        Ok(false) => {
            return SnapshotRefresh::Skipped(format!(
                "the main checkout does not gitignore `{name}`. Run `ss-magic orca` there once to \
                 set it up."
            ));
        }
        Err(err) => {
            return SnapshotRefresh::Skipped(format!(
                "git could not say whether the main checkout ignores `{name}` ({err:#})"
            ));
        }
    }

    let existing = worktreeinclude::read(main_root);
    let document = match &existing {
        Existing::Absent => Document::empty(),
        Existing::Text(text) => Document::parse(text),
        Existing::Unreadable(why) => {
            return SnapshotRefresh::Skipped(format!(
                "the main checkout's `{name}` was left untouched because {why}"
            ));
        }
    };

    let shared_dirs = fs::read_to_string(main_root.join(ORCA_YAML))
        .map(|text| orca_yaml::shared_directories(&text))
        .unwrap_or_default();
    let snap = snapshot::compute(main_root, patterns, &shared_dirs, document.budget());
    let warning = snapshot::overflow_warning(&snap);
    let rendered = document.render(&snap.entries);
    if matches!(&existing, Existing::Text(text) if *text == rendered) {
        return SnapshotRefresh::Unchanged { warning };
    }

    match write_contained(main_root, name, &rendered) {
        Ok(Landing::Inside) => SnapshotRefresh::Written {
            listed: snap.entries.len(),
            warning,
        },
        Ok(Landing::Outside(target)) => SnapshotRefresh::Skipped(format!(
            "the main checkout's `{name}` is a symlink to {}, outside the repository",
            target.display()
        )),
        Err(err) => SnapshotRefresh::Skipped(format!("writing `{name}` failed ({err:#})")),
    }
}

#[cfg(test)]
mod tests;
