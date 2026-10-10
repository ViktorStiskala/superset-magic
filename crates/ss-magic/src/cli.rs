//! Hand-rolled argument parsing for the `ss-magic` entry points.
//!
//! A handful of entry points don't justify pulling in `clap`, so this is a tiny
//! parser over `std::env::args`: the first non-flag token selects `sync`,
//! `pack`, `update`, or `init` (with `--local` anywhere selecting the local
//! install); its absence falls through to the
//! interactive (bare) mode. `--version`/`-V` and `--help`/`-h` short-circuit to
//! terminal signals, and any unrecognized subcommand is an error carrying the
//! same usage text the help path prints.
//!
//! The parser is split from `main.rs` so it's unit-testable without spawning
//! the process: `parse(&[String]) -> Parsed` takes argv (sans program name)
//! and never touches global state.

/// Which operation the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// No subcommand — open the interactive operation menu.
    Bare,
    /// Non-interactive forward file copy, main → current worktree. Backs up
    /// every overwritten worktree file first unless `no_backup`.
    Sync {
        /// `--no-backup`/`-n`: skip the pre-overwrite backup pass.
        no_backup: bool,
    },
    /// Non-interactive reverse copy, current worktree → main, for git-untracked
    /// files matching the configured patterns. Backs up every overwritten main
    /// file first unless `no_backup`.
    ReverseSync {
        /// `--no-backup`/`-n`: skip the pre-overwrite backup pass.
        no_backup: bool,
    },
    /// Non-interactive pack: archive the configured files into a tar.bz2 at the
    /// git root.
    Pack,
    /// Force a self-update.
    Update,
}

/// Outcome of parsing argv. `Help` and `Error` are terminal signals the
/// caller turns into a usage print + exit code; `Command` proceeds to work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Run this command.
    Command(Command),
    /// Non-interactive `init [PATTERN...]`: seed the magic.json layout from the
    /// given file patterns without the TUI. Carried separately from `Command`
    /// (which stays `Copy`) and handled before the update gate.
    Init(Vec<String>),
    /// Non-interactive `init --local [PATTERN...]`: a local (uncommitted)
    /// install in the main checkout – the patterns go into the gitignored
    /// `magic.local.json` and `ss-magic sync` is registered in Superset's
    /// gitignored `config.local.json`, so no tracked file changes. A separate
    /// variant rather than a flag on [`Parsed::Init`] so the committed `init`
    /// keeps its exact shape; `--local` anywhere in argv selects it (see
    /// [`has_local`]).
    InitLocal(Vec<String>),
    /// `--version`/`-V` was requested; print the version and exit 0. Terminal:
    /// it is decided before any subcommand is selected, so a `--version` from a
    /// script can never fall through to the bare menu.
    Version,
    /// `--help`/`-h` was requested; print usage and exit 0.
    Help,
    /// An unrecognized subcommand; the string is the offending token. The
    /// caller prints usage to stderr and exits non-zero.
    Error(String),
}

/// One-line program usage banner.
pub const USAGE: &str = "\
Usage: ss-magic [COMMAND] [OPTIONS]

Commands:
  (none)         Open the interactive sync / operation menu
  sync           Non-interactive forward file copy (main → current worktree)
  reverse-sync   Non-interactive reverse copy (current worktree → main) for
                 git-untracked files matching the configured patterns
  pack           Archive the configured files into ss-magic-<repo>.tar.bz2 at
                 the git root (name derived from the origin remote)
  update         Force a self-update to the latest release
  init           Initialize .superset (magic.json layout) non-interactively;
                 optional file-pattern args become magic.json `files`
  init --local [PATTERN...]
                 Local (uncommitted) install in the main checkout: patterns go
                 to the gitignored magic.local.json, `ss-magic sync` is
                 registered in .superset/config.local.json, ignore rules go to
                 .git/info/exclude; no tracked file changes

Options:
  -n, --no-backup   Skip the pre-overwrite backup on `sync`/`reverse-sync`.
                    WARNING: overwriting or deleting an untracked secret then
                    leaves NO recovery path (no git history, no backup).
  -h, --help        Print this help (recognized before the subcommand)
  -V, --version     Print the version and exit (recognized anywhere in argv)";

/// Render the usage text. Kept as a function (not just the `const`) so the
/// help path and the error path share one source of truth and a trailing
/// newline is easy to attach at the print site.
pub fn usage() -> &'static str {
    USAGE
}

/// Parse argv with the program name already stripped (i.e. pass
/// `std::env::args().skip(1)` collected into a slice).
///
/// `--version`/`-V` wins over everything (see [`version_requested`]). After
/// that the first non-flag token decides the command, and a `--help`/`-h`
/// anywhere before that token short-circuits to [`Parsed::Help`]. Other flags
/// are skipped while scanning for the subcommand (none are defined today, but
/// this keeps `ss-magic --foo sync` from mis-selecting `--foo`).
pub fn parse(args: &[String]) -> Parsed {
    if version_requested(args) {
        return Parsed::Version;
    }
    for (i, arg) in args.iter().enumerate() {
        if arg == "-h" || arg == "--help" {
            return Parsed::Help;
        }
        if arg.starts_with('-') {
            // Unknown flag before any subcommand — skip it and keep scanning.
            continue;
        }
        return match arg.as_str() {
            "sync" => Parsed::Command(Command::Sync {
                no_backup: has_no_backup(args),
            }),
            "reverse-sync" => Parsed::Command(Command::ReverseSync {
                no_backup: has_no_backup(args),
            }),
            "pack" => Parsed::Command(Command::Pack),
            "update" => Parsed::Command(Command::Update),
            // Positional args after `init` become file patterns: magic.json's
            // for a committed init, magic.local.json's for `--local`.
            "init" => {
                let patterns: Vec<String> = args[i + 1..]
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                if has_local(args) {
                    Parsed::InitLocal(patterns)
                } else {
                    Parsed::Init(patterns)
                }
            }
            other => Parsed::Error(other.to_string()),
        };
    }
    Parsed::Command(Command::Bare)
}

/// True when `--no-backup`/`-n` appears ANYWHERE in argv — before OR after the
/// subcommand token. Position-independent because [`parse`]'s loop returns as
/// soon as it matches a subcommand, so a whole-slice scan is the only way to
/// also catch a trailing flag. Deliberately asymmetric with `-h`/`--help`, a
/// terminal short-circuit recognized only BEFORE the subcommand (see USAGE): a
/// maintainer should NOT "fix" one to match the other.
fn has_no_backup(args: &[String]) -> bool {
    args.iter().any(|a| a == "--no-backup" || a == "-n")
}

/// True when `--local` appears ANYWHERE in argv, before or after `init`.
///
/// A whole-slice scan, like [`has_no_backup`], and for a safety reason beyond
/// convenience: a positional scan that only looked after `init` would read
/// `ss-magic --local init` as a COMMITTED init, which writes tracked files –
/// the one outcome a user asking for a local install must never get.
fn has_local(args: &[String]) -> bool {
    args.iter().any(|a| a == "--local")
}

/// True when `--version`/`-V` appears anywhere in argv.
///
/// Deliberately asymmetric with `-h`/`--help`, which is only recognized before
/// the subcommand: the scan runs PAST a subcommand token, so
/// `ss-magic sync --version` still prints the version. Without that, an
/// unrecognized `--version` would be skipped as an unknown flag and fall
/// through to `Command::Bare`, which is gated for auto-update and opens the
/// interactive menu — exactly the wrong thing when a script shells out to check
/// which binary it got.
///
/// The scan used to stop at a `plugin` token, whose trailing argv belonged to
/// the plugin verb tree and could carry a `-V` of its own. That token is gone:
/// the plugin is its own binary (`ss-magic-plugin`) with its own release line,
/// so `ss-magic plugin …` now takes the ordinary unknown-subcommand path and
/// there is no sub-argv left to protect.
fn version_requested(args: &[String]) -> bool {
    args.iter().any(|a| a == "--version" || a == "-V")
}

#[cfg(test)]
mod tests;
