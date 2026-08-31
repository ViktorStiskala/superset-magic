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
    ("compact-window", HumanVerb::CompactWindow),
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
                    HumanVerb::Enable | HumanVerb::Disable | HumanVerb::Config
                ),
                "unexpected config writer {verb:?}"
            );
        }
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
