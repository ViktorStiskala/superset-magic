//! Hand-rolled argument parsing for the three `ss-magic` entry points.
//!
//! Three entry points don't justify pulling in `clap`, so this is a tiny
//! parser over `std::env::args`: the first non-flag token selects `sync` or
//! `update`; its absence falls through to the interactive (bare) mode.
//! `--help`/`-h` short-circuits to a help request, and any unrecognized
//! subcommand is an error carrying the same usage text the help path prints.
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
    /// Force a self-update.
    Update,
}

/// Outcome of parsing argv. `Help` and `Error` are terminal signals the
/// caller turns into a usage print + exit code; `Command` proceeds to work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Run this command.
    Command(Command),
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
  update    Force a self-update to the latest release

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
    for arg in args {
        if arg == "-h" || arg == "--help" {
            return Parsed::Help;
        }
        if arg.starts_with('-') {
            // Unknown flag before any subcommand — skip it and keep scanning.
            continue;
        }
        return match arg.as_str() {
            "sync" => Parsed::Command(Command::Sync),
            "update" => Parsed::Command(Command::Update),
            other => Parsed::Error(other.to_string()),
        };
    }
    Parsed::Command(Command::Bare)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sync_token_dispatches_to_sync() {
        assert_eq!(parse(&argv(&["sync"])), Parsed::Command(Command::Sync));
    }

    #[test]
    fn update_token_dispatches_to_update() {
        assert_eq!(parse(&argv(&["update"])), Parsed::Command(Command::Update));
    }

    #[test]
    fn no_args_routes_to_bare() {
        assert_eq!(parse(&argv(&[])), Parsed::Command(Command::Bare));
    }

    #[test]
    fn unknown_subcommand_is_error_naming_the_token() {
        assert_eq!(parse(&argv(&["bogus"])), Parsed::Error("bogus".to_string()));
    }

    #[test]
    fn help_long_and_short_request_help() {
        assert_eq!(parse(&argv(&["--help"])), Parsed::Help);
        assert_eq!(parse(&argv(&["-h"])), Parsed::Help);
    }

    #[test]
    fn help_lists_the_three_modes() {
        let text = usage();
        assert!(text.contains("sync"), "usage should mention sync: {text:?}");
        assert!(
            text.contains("update"),
            "usage should mention update: {text:?}"
        );
        assert!(
            text.contains("interactive"),
            "usage should mention the interactive (bare) mode: {text:?}"
        );
    }

    #[test]
    fn help_wins_over_a_following_subcommand() {
        // A help flag short-circuits even when a subcommand follows it.
        assert_eq!(parse(&argv(&["--help", "sync"])), Parsed::Help);
    }

    #[test]
    fn unknown_flag_before_subcommand_is_skipped() {
        // An unrecognized leading flag must not be mistaken for the
        // subcommand token.
        assert_eq!(
            parse(&argv(&["--verbose", "sync"])),
            Parsed::Command(Command::Sync)
        );
    }

    #[test]
    fn flags_only_with_no_subcommand_routes_to_bare() {
        assert_eq!(parse(&argv(&["--verbose"])), Parsed::Command(Command::Bare));
    }

    #[test]
    fn extra_args_after_subcommand_are_ignored() {
        assert_eq!(
            parse(&argv(&["sync", "extra"])),
            Parsed::Command(Command::Sync)
        );
    }
}
