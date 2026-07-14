//! Hand-rolled argument parsing for the `ss-magic` entry points.
//!
//! A handful of entry points don't justify pulling in `clap`, so this is a tiny
//! parser over `std::env::args`: the first non-flag token selects `sync`,
//! `pack`, `update`, or `init`; its absence falls through to the interactive
//! (bare) mode. `--help`/`-h` short-circuits to a help request, and any
//! unrecognized subcommand is an error carrying the same usage text the help
//! path prints.
//!
//! The parser is split from `main.rs` so it's unit-testable without spawning
//! the process: `parse(&[String]) -> Parsed` takes argv (sans program name)
//! and never touches global state.

/// Which operation the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// No subcommand — open the interactive operation menu.
    Bare,
    /// Non-interactive forward file copy, main → current worktree.
    Sync,
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
    /// `--help`/`-h` was requested; print usage and exit 0.
    Help,
    /// An unrecognized subcommand; the string is the offending token. The
    /// caller prints usage to stderr and exits non-zero.
    Error(String),
}

/// One-line program usage banner.
pub const USAGE: &str = "\
Usage: ss-magic [COMMAND]

Commands:
  (none)    Open the interactive operation menu
  sync      Non-interactive forward file copy (main → current worktree)
  pack      Archive the configured files into ss-magic-<repo>.tar.bz2 at the
            git root (name derived from the origin remote)
  update    Force a self-update to the latest release
  init      Initialize .superset (magic.json layout) non-interactively;
            optional file-pattern args become magic.json `files`

Options:
  -h, --help    Print this help";

/// Render the usage text. Kept as a function (not just the `const`) so the
/// help path and the error path share one source of truth and a trailing
/// newline is easy to attach at the print site.
pub fn usage() -> &'static str {
    USAGE
}

/// Parse argv with the program name already stripped (i.e. pass
/// `std::env::args().skip(1)` collected into a slice).
///
/// The first non-flag token decides the command. A leading `--help`/`-h`
/// anywhere before a subcommand short-circuits to [`Parsed::Help`]. Other
/// flags are skipped while scanning for the subcommand (none are defined
/// today, but this keeps `ss-magic --foo sync` from mis-selecting `--foo`).
pub fn parse(args: &[String]) -> Parsed {
    for (i, arg) in args.iter().enumerate() {
        if arg == "-h" || arg == "--help" {
            return Parsed::Help;
        }
        if arg.starts_with('-') {
            // Unknown flag before any subcommand — skip it and keep scanning.
            continue;
        }
        return match arg.as_str() {
            "sync" => Parsed::Command(Command::Sync),
            "pack" => Parsed::Command(Command::Pack),
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

#[cfg(test)]
mod tests;
