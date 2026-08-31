use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn sync_token_dispatches_to_sync() {
    assert_eq!(parse(&argv(&["sync"])), Parsed::Command(Command::Sync { no_backup: false }));
}

#[test]
fn update_token_dispatches_to_update() {
    assert_eq!(parse(&argv(&["update"])), Parsed::Command(Command::Update));
}

#[test]
fn pack_token_dispatches_to_pack() {
    assert_eq!(parse(&argv(&["pack"])), Parsed::Command(Command::Pack));
}

#[test]
fn help_mentions_pack() {
    assert!(usage().contains("pack"), "usage should mention pack");
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
    // Same precedence for the `pack` token (plan U3).
    assert_eq!(parse(&argv(&["--help", "pack"])), Parsed::Help);
}

#[test]
fn unknown_flag_before_subcommand_is_skipped() {
    // An unrecognized leading flag must not be mistaken for the
    // subcommand token.
    assert_eq!(
        parse(&argv(&["--verbose", "sync"])),
        Parsed::Command(Command::Sync { no_backup: false })
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
        Parsed::Command(Command::Sync { no_backup: false })
    );
}

#[test]
fn init_with_no_patterns_yields_empty_init() {
    assert_eq!(parse(&argv(&["init"])), Parsed::Init(vec![]));
}

#[test]
fn init_collects_positional_patterns() {
    assert_eq!(
        parse(&argv(&["init", "**/.env", "apps/*/.dev.vars"])),
        Parsed::Init(vec![
            "**/.env".to_string(),
            "apps/*/.dev.vars".to_string()
        ])
    );
}

#[test]
fn reverse_sync_token_dispatches_to_reverse_sync() {
    assert_eq!(
        parse(&argv(&["reverse-sync"])),
        Parsed::Command(Command::ReverseSync { no_backup: false })
    );
}

#[test]
fn sync_no_backup_long_flag() {
    assert_eq!(
        parse(&argv(&["sync", "--no-backup"])),
        Parsed::Command(Command::Sync { no_backup: true })
    );
}

#[test]
fn sync_no_backup_short_flag() {
    assert_eq!(
        parse(&argv(&["sync", "-n"])),
        Parsed::Command(Command::Sync { no_backup: true })
    );
}

#[test]
fn sync_no_backup_before_subcommand() {
    // has_no_backup scans the whole argv, so a leading flag counts too, not
    // just one trailing after the subcommand token.
    assert_eq!(
        parse(&argv(&["--no-backup", "sync"])),
        Parsed::Command(Command::Sync { no_backup: true })
    );
}

#[test]
fn reverse_sync_no_backup_flag() {
    assert_eq!(
        parse(&argv(&["reverse-sync", "-n"])),
        Parsed::Command(Command::ReverseSync { no_backup: true })
    );
}

#[test]
fn no_backup_ignored_for_pack() {
    // Command::Pack has no no_backup field to set — the flag is simply inert.
    assert_eq!(
        parse(&argv(&["pack", "--no-backup"])),
        Parsed::Command(Command::Pack)
    );
}

#[test]
fn help_mentions_reverse_sync() {
    assert!(
        usage().contains("reverse-sync"),
        "usage should mention reverse-sync"
    );
}

#[test]
fn help_mentions_no_backup() {
    assert!(
        usage().contains("--no-backup"),
        "usage should mention --no-backup"
    );
}

// ── `plugin` verb tree (U6) ───────────────────────────────────────────────────

#[test]
fn plugin_token_carries_the_rest_of_argv() {
    assert_eq!(
        parse(&argv(&["plugin", "hook", "pre-tool-use"])),
        Parsed::Plugin(vec!["hook".to_string(), "pre-tool-use".to_string()])
    );
}

#[test]
fn plugin_with_no_further_args_is_still_a_plugin_invocation() {
    // The "no verb" error belongs to `plugin::parse`, not here.
    assert_eq!(parse(&argv(&["plugin"])), Parsed::Plugin(vec![]));
}

#[test]
fn plugin_keeps_flags_in_its_tail() {
    // Unlike `init`, plugin verbs take their own flags, so nothing is filtered.
    assert_eq!(
        parse(&argv(&["plugin", "status", "--json"])),
        Parsed::Plugin(vec!["status".to_string(), "--json".to_string()])
    );
}

#[test]
fn plugin_never_falls_through_to_bare() {
    // Bare is the one command that opens the TUI and is gated for auto-update;
    // no plugin argv may reach it.
    for tail in [
        vec!["plugin"],
        vec!["plugin", "hook", "session-start"],
        vec!["plugin", "bogus"],
        vec!["plugin", "--anything"],
    ] {
        let parsed = parse(&argv(&tail));
        assert!(
            matches!(parsed, Parsed::Plugin(_)),
            "{tail:?} should parse to Plugin, got {parsed:?}"
        );
    }
}

#[test]
fn help_mentions_plugin() {
    assert!(usage().contains("plugin"), "usage should mention plugin");
}

// ── `--version` / `-V` short-circuit (U6, AE56) ───────────────────────────────

#[test]
fn version_long_and_short_request_version() {
    assert_eq!(parse(&argv(&["--version"])), Parsed::Version);
    assert_eq!(parse(&argv(&["-V"])), Parsed::Version);
}

#[test]
fn version_wins_from_any_position_before_the_plugin_token() {
    // Before a subcommand, after a subcommand, and among other flags: all the
    // same answer. Without this, an unrecognized `--version` would be skipped
    // as an unknown flag and land on Command::Bare — a network update check
    // plus a menu, inside a hook that has no terminal.
    assert_eq!(parse(&argv(&["--version", "sync"])), Parsed::Version);
    assert_eq!(parse(&argv(&["sync", "--version"])), Parsed::Version);
    assert_eq!(parse(&argv(&["pack", "-V"])), Parsed::Version);
    assert_eq!(parse(&argv(&["--verbose", "-V", "update"])), Parsed::Version);
}

#[test]
fn version_after_the_plugin_token_belongs_to_the_plugin_verb() {
    // Past `plugin` the argv is the verb tree's, and a `-V` there may be a
    // verb's own flag or a value it was given.
    assert_eq!(
        parse(&argv(&["plugin", "conclude", "--version"])),
        Parsed::Plugin(vec![
            "conclude".to_string(),
            "--version".to_string()
        ])
    );
    assert_eq!(
        parse(&argv(&["plugin", "-V"])),
        Parsed::Plugin(vec!["-V".to_string()])
    );
}

#[test]
fn version_before_the_plugin_token_still_short_circuits() {
    assert_eq!(parse(&argv(&["--version", "plugin", "status"])), Parsed::Version);
}

#[test]
fn help_mentions_version() {
    assert!(
        usage().contains("--version"),
        "usage should mention --version"
    );
}
