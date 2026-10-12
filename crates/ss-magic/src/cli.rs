//! Hand-rolled argument parsing for the `ss-magic` entry points.
//!
//! A handful of entry points don't justify pulling in `clap`, so this is a tiny
//! parser over `std::env::args`: the first non-flag token selects `sync`,
//! `reverse-sync`, `pack`, `orca`, `update`, or `init`; its absence falls
//! through to the interactive (bare) mode. `--version`/`-V` and `--help`/`-h` short-circuit to
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
        /// `--no-orca-skip`: in an Orca worktree, copy the paths listed in
        /// the main checkout's `.worktreeinclude` too, instead of leaving the
        /// ones Orca already copied alone. No effect outside an Orca worktree.
        no_orca_skip: bool,
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
    /// Explicit Orca refresh: regenerate `orca.yaml`'s shared directories and
    /// `.worktreeinclude` in the main checkout from `magic.json`.
    Orca,
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
  orca           Regenerate the Orca files in the main checkout from
                 magic.json: orca.yaml's shared directories and
                 .worktreeinclude (runs from any directory of the repository)
  update         Force a self-update to the latest release
  init           Initialize .superset (magic.json layout) and the Orca files
                 non-interactively; optional file-pattern args become
                 magic.json `files`

Options:
  -n, --no-backup   Skip the pre-overwrite backup on `sync`/`reverse-sync`.
                    WARNING: overwriting or deleting an untracked secret then
                    leaves NO recovery path (no git history, no backup).
      --no-orca-skip
                    On `sync` in an Orca worktree: also copy the paths Orca
                    already copied from `.worktreeinclude` (default: skip them).
  -h, --help        Print this help (recognized before the subcommand, and
                    after `orca`)
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
                no_orca_skip: has_no_orca_skip(args),
            }),
            "reverse-sync" => Parsed::Command(Command::ReverseSync {
                no_backup: has_no_backup(args),
            }),
            "pack" => Parsed::Command(Command::Pack),
            // `orca` rewrites files in the shared main checkout. A `--help`
            // typed AFTER it is someone asking what the command does, and
            // answering by running it would be the wrong surprise – so here,
            // unlike for the other subcommands, a trailing help flag wins.
            "orca" if args[i + 1..].iter().any(|a| a == "-h" || a == "--help") => Parsed::Help,
            "orca" => Parsed::Command(Command::Orca),
            "update" => Parsed::Command(Command::Update),
            // Positional args after `init` become magic.json file patterns.
            "init" => Parsed::Init(
                args[i + 1..]
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect(),
            ),
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

/// True when `--no-orca-skip` appears ANYWHERE in argv, for the same reason
/// [`has_no_backup`] scans the whole slice: the flag is as valid after the
/// `sync` token as before it. It has no short form – it is the rarely needed
/// opt-out of a default, not something typed often.
fn has_no_orca_skip(args: &[String]) -> bool {
    args.iter().any(|a| a == "--no-orca-skip")
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
