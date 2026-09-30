//! U6 second-level parse tests: the split between the stdin-driven hook entry
//! point and the argv-driven human verbs.
//!
//! Everything here exercises `parse` only, which touches no stdin, no
//! filesystem and no process — the whole verb tree resolves in memory.

use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Every event name the manifest can carry, paired with the channel it selects.
const EVENTS: &[(&str, HookEvent)] = &[
    ("session-start", HookEvent::SessionStart),
    ("pre-tool-use", HookEvent::PreToolUse),
    ("pre-compact", HookEvent::PreCompact),
    ("subagent-stop", HookEvent::SubagentStop),
    ("session-end", HookEvent::SessionEnd),
    ("file-changed", HookEvent::FileChanged),
];

/// Every human verb token, paired with the verb it selects.
const VERBS: &[(&str, HumanVerb)] = &[
    ("status", HumanVerb::Status),
    ("cost", HumanVerb::Cost),
    ("spill-index", HumanVerb::SpillIndex),
    ("scratchpad", HumanVerb::Scratchpad),
    ("conclude", HumanVerb::Conclude),
    ("conclusions", HumanVerb::Conclusions),
    ("gc", HumanVerb::Gc),
    ("bypass", HumanVerb::Bypass),
    ("expect-artifact", HumanVerb::ExpectArtifact),
    ("enable", HumanVerb::Enable),
    ("disable", HumanVerb::Disable),
    ("config", HumanVerb::Config),
    ("seed-config", HumanVerb::SeedConfig),
    ("compact-window", HumanVerb::CompactWindow),
    ("release-check", HumanVerb::ReleaseCheck),
    ("setup-github-ci", HumanVerb::SetupGithubCi),
    ("checklist", HumanVerb::Checklist),
];

// ── Hook entry point ──────────────────────────────────────────────────────────

#[test]
fn hook_pre_tool_use_parses_to_the_pre_tool_use_channel() {
    assert_eq!(
        parse(&argv(&["hook", "pre-tool-use"])),
        Parsed::Invocation(Invocation::Hook {
            event: HookEvent::PreToolUse,
            args: vec![],
        })
    );
}

#[test]
fn every_hook_event_parses_to_its_channel() {
    for (token, expected) in EVENTS {
        assert_eq!(
            parse(&argv(&["hook", token])),
            Parsed::Invocation(Invocation::Hook {
                event: expected.clone(),
                args: vec![],
            }),
            "`hook {token}` should select {expected:?}"
        );
    }
}

#[test]
fn hook_event_token_round_trips() {
    for (token, _) in EVENTS {
        assert_eq!(HookEvent::from_token(token).as_str(), *token);
    }
}

#[test]
fn hook_carries_trailing_args_to_the_event() {
    assert_eq!(
        parse(&argv(&["hook", "session-start", "--extra", "value"])),
        Parsed::Invocation(Invocation::Hook {
            event: HookEvent::SessionStart,
            args: argv(&["--extra", "value"]),
        })
    );
}

/// A manifest from a newer build can name an event this binary does not route.
/// The parse layer must hand that name onward as a value; turning it into an
/// error here would make the exit-0-with-empty-stdout contract impossible.
#[test]
fn unknown_hook_event_is_a_value_not_an_error() {
    assert_eq!(
        parse(&argv(&["hook", "notification"])),
        Parsed::Invocation(Invocation::Hook {
            event: HookEvent::Unknown("notification".to_string()),
            args: vec![],
        })
    );
}

#[test]
fn hook_with_no_event_is_a_value_not_an_error() {
    assert_eq!(
        parse(&argv(&["hook"])),
        Parsed::Invocation(Invocation::Hook {
            event: HookEvent::Missing,
            args: vec![],
        })
    );
}

#[test]
fn missing_hook_event_has_no_name_to_report() {
    assert_eq!(HookEvent::Missing.as_str(), "");
}

// ── Human verbs ───────────────────────────────────────────────────────────────

#[test]
fn every_human_verb_parses_to_its_verb() {
    for (token, expected) in VERBS {
        assert_eq!(
            parse(&argv(&[token])),
            Parsed::Invocation(Invocation::Human {
                verb: *expected,
                args: vec![],
            }),
            "`{token}` should select {expected:?}"
        );
    }
}

#[test]
fn human_verb_token_round_trips() {
    for (token, verb) in VERBS {
        assert_eq!(verb.as_str(), *token);
    }
}

#[test]
fn human_verb_carries_trailing_args_including_flags() {
    // Unlike `init`, the plugin verbs need their own flags, so nothing is
    // filtered out of the tail.
    assert_eq!(
        parse(&argv(&["config", "set", "plugin.enabled", "false", "--local"])),
        Parsed::Invocation(Invocation::Human {
            verb: HumanVerb::Config,
            args: argv(&["set", "plugin.enabled", "false", "--local"]),
        })
    );
}

#[test]
fn unknown_verb_is_a_loud_error_naming_the_token() {
    assert_eq!(
        parse(&argv(&["bogus"])),
        Parsed::UnknownVerb("bogus".to_string())
    );
}

#[test]
fn no_verb_at_all_is_an_error() {
    assert_eq!(parse(&argv(&[])), Parsed::MissingVerb);
}

#[test]
fn plugin_help_flags_request_usage() {
    assert_eq!(parse(&argv(&["--help"])), Parsed::Help);
    assert_eq!(parse(&argv(&["-h"])), Parsed::Help);
}

#[test]
fn usage_lists_the_hook_entry_point_and_every_verb() {
    let text = usage();
    assert!(text.contains("hook <event>"), "usage should show the hook form");
    for (token, _) in EVENTS {
        assert!(text.contains(token), "usage should mention event {token}");
    }
    for (token, _) in VERBS {
        assert!(text.contains(token), "usage should mention verb {token}");
    }
}

// ── `-V` / `--version` (KTD10) ────────────────────────────────────────────────
//
// This flag is not a convenience. `hooks/bootstrap.sh` refuses to install a
// freshly downloaded binary unless it runs HERE and reports the pinned version,
// which is the check that catches a wrong-architecture download — one that
// passes the SHA-256 verification and then cannot execute. `status` probes the
// same flag to report drift. Both take the LAST whitespace-separated field of
// the FIRST line.

#[test]
fn version_flags_are_answered_ahead_of_verb_parsing() {
    assert_eq!(parse(&argv(&["--version"])), Parsed::Version);
    assert_eq!(parse(&argv(&["-V"])), Parsed::Version);
}

#[test]
fn the_version_line_is_the_shape_the_bootstrap_parses() {
    let line = version_line();
    // What bootstrap.sh does: `| head -1 | awk '{print $NF}'`.
    let first = line.lines().next().expect("at least one line");
    let last_field = first.split_whitespace().last().expect("a last field");
    assert_eq!(
        last_field,
        env!("CARGO_PKG_VERSION"),
        "the bootstrap reads the last field of the first line as the version; \
         {line:?} would make it discard every download"
    );
    assert!(
        first.starts_with("ss-magic-plugin "),
        "the line should name this binary, not the sync CLI: {first:?}"
    );
    assert_eq!(line.lines().count(), 1, "one line only: {line:?}");
}

#[test]
fn a_version_flag_after_a_verb_belongs_to_the_verb() {
    // Deliberately unlike the `ss-magic` CLI, which scans the whole argv. The
    // verbs here parse their own arguments, so a whole-argv scan would swallow
    // a flag that is not ours. Neither caller of `--version` ever passes it
    // after a verb.
    assert_eq!(
        parse(&argv(&["conclude", "--version"])),
        Parsed::Invocation(Invocation::Human {
            verb: HumanVerb::Conclude,
            args: vec!["--version".to_string()],
        })
    );
    assert!(matches!(
        parse(&argv(&["hook", "session-start", "-V"])),
        Parsed::Invocation(Invocation::Hook { .. })
    ));
}

// ── The hook / human boundary (AE44, R57) ─────────────────────────────────────

/// There is no install verb, in any spelling — the marketplace is the only
/// delivery path, so no argv can reach an install.
#[test]
fn there_is_no_install_verb() {
    for token in ["install", "uninstall", "plugin-install"] {
        assert!(
            HumanVerb::from_token(token).is_none(),
            "`{token}` must not be a verb"
        );
        assert_eq!(
            parse(&argv(&[token])),
            Parsed::UnknownVerb(token.to_string())
        );
    }
}

/// AE44: a repository can get a hook to fire, so nothing reachable through
/// `hook` may be a configuration write. Every event — routable, unknown, or
/// absent — lands on the hook side of the split.
#[test]
fn no_hook_event_reaches_a_config_writing_verb() {
    let mut event_argvs: Vec<Vec<String>> = EVENTS
        .iter()
        .map(|(token, _)| argv(&["hook", token]))
        .collect();
    event_argvs.push(argv(&["hook", "notification"]));
    event_argvs.push(argv(&["hook"]));
    // A verb name smuggled into the event slot is still just an event name.
    event_argvs.push(argv(&["hook", "enable"]));
    event_argvs.push(argv(&["hook", "config", "set", "plugin.enabled", "true"]));

    for args in event_argvs {
        match parse(&args) {
            Parsed::Invocation(Invocation::Hook { .. }) => {}
            other => panic!("{args:?} must stay on the hook side, got {other:?}"),
        }
    }
}

/// The mirror of the above: the config-writing verbs exist, and they are only
/// reachable as human verbs.
#[test]
fn config_writing_verbs_are_reachable_only_as_human_verbs() {
    for verb in [HumanVerb::Enable, HumanVerb::Disable, HumanVerb::Config] {
        assert!(verb.writes_config(), "{verb:?} should be a config writer");
        assert_eq!(
            parse(&argv(&[verb.as_str()])),
            Parsed::Invocation(Invocation::Human {
                verb,
                args: vec![],
            })
        );
    }
    // And nothing else claims to write config, so the assertion above stays
    // meaningful as verbs are added.
    for (_, verb) in VERBS {
        if verb.writes_config() {
            assert!(
                matches!(
                    verb,
                    HumanVerb::Enable
                        | HumanVerb::Disable
                        | HumanVerb::Config
                        | HumanVerb::SeedConfig
                ),
                "unexpected config writer {verb:?}"
            );
        }
    }
}

/// The invariant the hook/human split actually protects: a repository cannot
/// arrange its own ENABLEMENT by getting a hook to fire.
///
/// `writes_config` used to carry this by standing in for "no hook invokes one
/// of these". It cannot any more: `seed-config` writes configuration AND runs
/// from `hooks/bootstrap.sh`, a `SessionStart` hook. So the property is pinned
/// here directly, against the verbs a hook invokes.
///
/// `seed-config` is safe not because it is trusted to behave but because
/// `config::seed_block` has no code path that emits an `enabled` key at all —
/// asserted exhaustively in `config::tests`. `release-check` writes only the
/// plugin release cache in the OS cache directory and reads no configuration
/// at all.
#[test]
fn no_hook_invoked_verb_can_set_enabled() {
    // The complete list of verbs any shipped hook or hook-adjacent script
    // invokes. `hooks/bootstrap.sh` calls `seed-config`; the `SessionStart`
    // handler itself spawns `release-check --refresh --quiet` detached when
    // the release cache is stale. Every other hook entry runs `hook <event>`,
    // which never reaches a human verb at all.
    const INVOKED_BY_A_HOOK: &[HumanVerb] = &[HumanVerb::SeedConfig, HumanVerb::ReleaseCheck];

    for verb in INVOKED_BY_A_HOOK {
        assert!(
            !verb.can_set_enabled(),
            "{verb:?} is invoked from a hook and can set `plugin.enabled`; a \
             repository could then turn the plugin on by getting a hook to fire"
        );
    }
    // The three that CAN set it are the three no hook invokes.
    for (_, verb) in VERBS {
        if verb.can_set_enabled() {
            assert!(
                !INVOKED_BY_A_HOOK.contains(verb),
                "{verb:?} both sets `enabled` and is invoked by a hook"
            );
        }
    }
    // And the spawned argv is exactly the verb this list names.
    assert_eq!(
        HumanVerb::from_token(crate::release_check::REFRESH_ARGV[0]),
        Some(HumanVerb::ReleaseCheck)
    );
    assert!(!HumanVerb::ReleaseCheck.writes_config());
}

/// Every verb-shaped token the crate-root doc comment names in backticks must
/// be a real verb. The crate doc is the file a maintainer reads first, so a
/// command named there that does not exist is worse than no documentation.
///
/// Prompted by the doc describing the seed as `config seed` for a while when
/// the token is `seed-config`. **This test would not have caught that**, and
/// the limit is worth stating rather than leaving a reader to assume otherwise:
/// `config seed` is a two-word chunk whose first word IS a real verb, so
/// validating it would mean knowing each verb's subverb grammar — which lives
/// in the verb's own module and is not worth duplicating here. What this does
/// catch is the larger and more likely class: a single token that is no verb at
/// all, whether invented, misspelled, or left behind by a rename.
///
/// Nothing else catches either class. A doc comment is prose, and `cargo doc`
/// does not resolve a backticked command.
#[test]
fn the_crate_doc_names_only_real_verbs() {
    // Backticked tokens that are bare kebab-case words — the shape a verb has.
    // Anything else in backticks (a path, a type, a flag, a multi-word phrase)
    // carries a `/`, `.`, `:`, `_`, a capital or a space, and is skipped.
    //
    // These are the only kebab-shaped tokens in the doc that are NOT verbs.
    // The list is short and each entry is a real word the prose needs, not a
    // way to let a wrong verb through: three are crate or binary names, and two
    // are configuration keys.
    const NOT_VERBS: &[&str] = &[
        "ss-magic",
        "ss-magic-core",
        "ss-magic-plugin",
        "plugin",
        "enabled",
    ];

    let doc: String = include_str!("main.rs")
        .lines()
        .take_while(|l| l.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut checked = 0;
    for chunk in doc.split('`').skip(1).step_by(2) {
        let token = chunk.trim();
        let verb_shaped = !token.is_empty()
            && token.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            && !token.starts_with('-');
        if !verb_shaped || NOT_VERBS.contains(&token) {
            continue;
        }
        checked += 1;
        assert!(
            HumanVerb::from_token(token).is_some() || token == "hook",
            "the crate doc names `{token}`, which is not a verb this binary has"
        );
    }
    // Without this the whole test passes when the scan matches nothing at all —
    // a reformatted doc comment would silently retire the guard.
    assert!(
        checked >= 5,
        "the scan found only {checked} verb tokens; it is no longer reading the doc"
    );
}

/// R3/KTD14: nothing this binary prints tells a person to type `ss-magic
/// plugin …`. That subcommand no longer exists — the CLI drops the token
/// entirely and takes the ordinary unknown-subcommand path — so a usage string
/// still carrying it would document a command that cannot run.
#[test]
fn no_usage_text_names_a_plugin_subcommand_of_the_cli() {
    let sources: &[(&str, &str)] = &[
        ("plugin usage", usage()),
        ("enable", crate::config::ENABLE_USAGE),
        ("disable", crate::config::DISABLE_USAGE),
        ("config", crate::config::CONFIG_USAGE),
        ("seed-config", crate::config::SEED_CONFIG_USAGE),
    ];
    for (label, text) in sources {
        assert!(
            !text.contains("ss-magic plugin"),
            "{label} usage still names the removed `ss-magic plugin` subcommand:\n{text}"
        );
    }
}

// ── R47: the color posture is decided by which caller is being served ─────────

/// A hook verb answers the harness with JSON on stdout and plain text on
/// stderr, so it forces color off — for every event, including one this binary
/// cannot route and one with no event token at all.
#[test]
fn every_hook_invocation_forces_color_off() {
    for (token, _) in EVENTS {
        assert!(forces_no_color(&parse(&argv(&["hook", token]))), "{token}");
    }
    assert!(forces_no_color(&parse(&argv(&["hook", "notification"]))));
    assert!(forces_no_color(&parse(&argv(&["hook"]))));
}

/// A human verb is a person at a terminal, so it keeps the ordinary detection.
/// So do the help and error paths, which print styled text of their own.
#[test]
fn human_verbs_and_the_error_paths_keep_normal_color_detection() {
    for (token, _) in VERBS {
        assert!(!forces_no_color(&parse(&argv(&[token]))), "{token}");
    }
    for args in [vec!["--help"], vec![], vec!["nonsense"]] {
        assert!(!forces_no_color(&parse(&argv(&args))), "{args:?}");
    }
}
