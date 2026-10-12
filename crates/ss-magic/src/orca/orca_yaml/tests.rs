use super::*;

fn dirs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Parse `text` and return `scripts.setup` as a string, if it is one.
fn setup_of(text: &str) -> Option<String> {
    let docs = MarkedYaml::load_from_str(text.trim_start_matches('\u{feff}')).unwrap();
    let root = entries(docs.first()?)?;
    let (_, scripts) = find(&root, SCRIPTS)?;
    let (_, setup) = find(&entries(scripts)?, SETUP)?;
    string_value(setup).map(str::to_string)
}

/// Parse `text` and return `worktree.sharedDirectories` as strings.
fn shared_of(text: &str) -> Option<Vec<String>> {
    let docs = MarkedYaml::load_from_str(text.trim_start_matches('\u{feff}')).unwrap();
    let root = entries(docs.first()?)?;
    let (_, worktree) = find(&root, WORKTREE)?;
    let (_, shared) = find(&entries(worktree)?, SHARED_DIRECTORIES)?;
    match &shared.data {
        YamlData::Sequence(items) => items
            .iter()
            .map(|item| string_value(item).map(str::to_string))
            .collect(),
        _ => None,
    }
}

/// Every line of `kept` appears in `edited`, in the same order – the way to say
/// "these comments, blank lines and unmanaged keys survived" without pinning
/// the exact position of the lines ss-magic inserted between them.
fn assert_lines_survive(edited: &str, kept: &[&str]) {
    let mut lines = edited.lines();
    for want in kept {
        assert!(
            lines.any(|line| line == *want),
            "line {want:?} is missing or out of order in:\n{edited}"
        );
    }
}

fn refusal(current: &str, shared: &[&str], ensure_setup: bool) -> Refusal {
    match edit(Some(current), &dirs(shared), ensure_setup) {
        Err(refusal) => refusal,
        Ok(text) => panic!("expected a refusal, got:\n{text}"),
    }
}

// --- creating the file ------------------------------------------------------

#[test]
fn no_file_yields_the_setup_block_and_the_directory_list() {
    let out = edit(None, &dirs(&["node_modules", "apps/web/node_modules"]), true).unwrap();

    assert_eq!(
        out,
        "scripts:\n  setup: |\n    ./.superset/magic.sh sync\nworktree:\n  sharedDirectories:\n    \
         - node_modules\n    - apps/web/node_modules\n"
    );
    assert_eq!(setup_of(&out).as_deref(), Some("./.superset/magic.sh sync\n"));
    assert_eq!(
        shared_of(&out),
        Some(dirs(&["node_modules", "apps/web/node_modules"]))
    );
}

#[test]
fn no_file_with_no_directories_writes_an_empty_list() {
    let out = edit(None, &[], true).unwrap();

    assert_eq!(
        out,
        "scripts:\n  setup: |\n    ./.superset/magic.sh sync\nworktree:\n  sharedDirectories: []\n"
    );
    assert_eq!(shared_of(&out), Some(Vec::new()));
}

#[test]
fn an_empty_or_comment_only_file_gains_both_blocks_and_keeps_its_comments() {
    let out = edit(Some(""), &dirs(&["node_modules"]), true).unwrap();
    assert_eq!(out, edit(None, &dirs(&["node_modules"]), true).unwrap());

    let commented = "# Orca configuration for this repository\n\n";
    let out = edit(Some(commented), &dirs(&["node_modules"]), true).unwrap();
    assert!(out.starts_with(commented), "the comment stays first:\n{out}");
    assert_eq!(shared_of(&out), Some(dirs(&["node_modules"])));
}

#[test]
fn the_managed_block_is_what_a_new_file_consists_of() {
    let shared = dirs(&["node_modules"]);
    assert_eq!(managed_block(&shared, true), edit(None, &shared, true).unwrap());
    assert_eq!(
        managed_block(&shared, false),
        "worktree:\n  sharedDirectories:\n    - node_modules\n"
    );
}

// --- scripts.setup ----------------------------------------------------------

/// AE4. An existing script keeps running, with the sync line ahead of it, and
/// an unmanaged key's text is byte-identical.
#[test]
fn ae4_existing_setup_gains_the_line_and_other_keys_are_untouched() {
    let current = "scripts:\n  setup: pnpm install\ndefaultTabs:\n  - title: dev\n    command: pnpm dev\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\npnpm install")
    );
    assert!(
        out.contains("defaultTabs:\n  - title: dev\n    command: pnpm dev\n"),
        "defaultTabs text must be byte-identical:\n{out}"
    );
    assert_eq!(
        out,
        "scripts:\n  setup: |-\n    ./.superset/magic.sh sync\n    pnpm install\ndefaultTabs:\n  \
         - title: dev\n    command: pnpm dev\nworktree:\n  sharedDirectories: []\n"
    );
}

#[test]
fn a_multi_line_block_script_keeps_its_lines_and_its_trailing_newline() {
    let current = "scripts:\n  setup: |\n    pnpm install\n    pnpm build\n  archive: rm -rf dist\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\npnpm install\npnpm build\n")
    );
    assert_lines_survive(&out, &["scripts:", "    pnpm install", "    pnpm build", "  archive: rm -rf dist"]);
}

#[test]
fn a_setup_already_holding_the_line_is_left_byte_identical() {
    for current in [
        "scripts:\n  setup: ./.superset/magic.sh sync\nworktree:\n  sharedDirectories: []\n",
        // Surrounding white space on the line does not make it "absent".
        "scripts:\n  setup: |\n    pnpm install\n      ./.superset/magic.sh sync   \n    pnpm build\nworktree:\n  sharedDirectories: []\n",
    ] {
        let out = edit(Some(current), &[], true).unwrap();
        assert_eq!(out, current, "no edit is needed, so not one byte moves");
        assert_eq!(
            out.matches("magic.sh sync").count(),
            1,
            "the line must never be duplicated"
        );
    }
}

#[test]
fn editing_twice_changes_nothing_the_second_time() {
    let current = "scripts:\n  setup: pnpm install\n";
    let once = edit(Some(current), &dirs(&["node_modules"]), true).unwrap();
    let twice = edit(Some(&once), &dirs(&["node_modules"]), true).unwrap();
    assert_eq!(once, twice);
}

#[test]
fn scripts_without_setup_gains_the_entry_ahead_of_its_siblings() {
    let current = "scripts:\n  # cleans the build output\n  archive: rm -rf dist\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(setup_of(&out).as_deref(), Some("./.superset/magic.sh sync\n"));
    assert_eq!(
        out,
        "scripts:\n  setup: |\n    ./.superset/magic.sh sync\n  # cleans the build output\n  \
         archive: rm -rf dist\nworktree:\n  sharedDirectories: []\n",
        "the comment stays attached to the key it was written above"
    );
}

#[test]
fn an_empty_setup_value_is_filled_with_the_line() {
    for current in [
        "scripts:\n  setup:\n  archive: x\n",
        "scripts:\n  setup: ~\n  archive: x\n",
        "scripts:\n  setup: \"\"\n  archive: x\n",
    ] {
        let out = edit(Some(current), &[], true).unwrap();
        assert_eq!(
            setup_of(&out).as_deref(),
            Some("./.superset/magic.sh sync\n"),
            "from {current:?} got:\n{out}"
        );
        assert_lines_survive(&out, &["scripts:", "  archive: x"]);
    }
}

/// R25. With the switch off, `scripts` is not looked at: a line someone removed
/// stays removed, and even a layout ss-magic could not edit is no reason to
/// refuse.
#[test]
fn setup_switch_off_leaves_scripts_untouched() {
    let current = "scripts:\n  setup: pnpm install\n";
    let out = edit(Some(current), &dirs(&["node_modules"]), false).unwrap();
    assert!(out.starts_with(current), "scripts text is untouched:\n{out}");
    assert_eq!(setup_of(&out).as_deref(), Some("pnpm install"));
    assert_eq!(shared_of(&out), Some(dirs(&["node_modules"])));

    let flow = "scripts: {setup: make}\n";
    let out = edit(Some(flow), &dirs(&["node_modules"]), false).unwrap();
    assert!(out.starts_with(flow));

    let none = edit(Some("defaultTabs: []\n"), &[], false).unwrap();
    assert!(!none.contains("scripts"), "no scripts block is created:\n{none}");
}

/// A comment after the old value would become script text once the value is a
/// block scalar. It moves to the block header line instead of being lost.
#[test]
fn a_comment_after_the_old_script_moves_to_the_block_header() {
    let current = "scripts:\n  setup: pnpm install # installs deps\n  archive: x\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\npnpm install")
    );
    assert!(out.contains("# installs deps"), "the comment survives:\n{out}");
    assert_lines_survive(&out, &["scripts:", "  archive: x"]);
}

#[test]
fn quoted_and_folded_scripts_are_rewritten_to_the_same_commands() {
    let cases = [
        ("scripts:\n  setup: \"pnpm install\\npnpm build\"\n", "pnpm install\npnpm build"),
        ("scripts:\n  setup: 'echo ''hi'''   # greet\n", "echo 'hi'"),
        ("scripts:\n  setup: >-\n    pnpm install\n    --frozen-lockfile\nnext: 1\n", "pnpm install --frozen-lockfile"),
        ("scripts:\n  setup:\n    pnpm install\n  other: 1\n", "pnpm install"),
    ];
    for (current, old_script) in cases {
        let out = edit(Some(current), &[], true).unwrap();
        assert_eq!(
            setup_of(&out),
            Some(format!("./.superset/magic.sh sync\n{old_script}")),
            "from {current:?} got:\n{out}"
        );
    }
}

/// A block scalar has no escapes, so a script it cannot carry is written
/// double-quoted. Either way the script reads back as the same string.
#[test]
fn a_script_a_block_scalar_cannot_carry_still_round_trips() {
    let cases = [
        // First line of the old script starts with a tab.
        "\tmake deps\nmake build\n",
        // A control character (bell) in the middle.
        "printf '\u{7}'\nmake\n",
        // Several trailing newlines.
        "make\n\n\n",
    ];
    for old_script in cases {
        let current = format!("scripts:\n  setup: {}\nnext: 1\n", double_quoted(old_script));
        assert_eq!(setup_of(&current).as_deref(), Some(old_script), "fixture sanity");

        let out = edit(Some(&current), &[], true).unwrap();

        assert_eq!(
            setup_of(&out),
            Some(format!("./.superset/magic.sh sync\n{old_script}")),
            "from {old_script:?} got:\n{out}"
        );
        assert_lines_survive(&out, &["next: 1"]);
    }
}

#[test]
fn a_script_that_is_not_a_string_is_refused() {
    let r = refusal("scripts:\n  setup: [a, b]\n", &[], true);
    assert!(r.reason.contains("scripts.setup"), "{}", r.reason);
    let r = refusal("scripts:\n  setup: 5\n", &[], true);
    assert!(r.reason.contains("not a string"), "{}", r.reason);
}

/// An old block header's comment is carried over to the rewritten header.
#[test]
fn a_comment_on_the_old_block_header_is_carried_over() {
    let current = "scripts:\n  setup: | # runs once per worktree\n    pnpm install\n  archive: x\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(
        out,
        "scripts:\n  setup: | # runs once per worktree\n    ./.superset/magic.sh sync\n    \
         pnpm install\n  archive: x\nworktree:\n  sharedDirectories: []\n"
    );
}

/// A keep-chomped block (`|+`) stays a block: the blank lines after it are
/// still in the file and already give the script its ending, so none are added.
#[test]
fn a_keep_chomped_block_script_stays_a_block_with_the_same_ending() {
    let current = "scripts:\n  setup: |+\n    make\n\n\nnext: 1\n";
    assert_eq!(setup_of(current).as_deref(), Some("make\n\n\n"), "fixture sanity");

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(
        out,
        "scripts:\n  setup: |+\n    ./.superset/magic.sh sync\n    make\n\n\nnext: 1\nworktree:\n  \
         sharedDirectories: []\n"
    );
    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\nmake\n\n\n")
    );
}

/// A `#` line inside a literal script is script text, while a less-indented
/// `#` line after it is a real comment. Both stay what they were.
#[test]
fn a_hash_line_inside_the_script_is_content_and_one_after_it_is_a_comment() {
    let current = "scripts:\n  setup: |\n    make\n    # still the script\n  # a real comment\n  other: x\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\nmake\n# still the script\n")
    );
    assert_lines_survive(
        &out,
        &["    make", "    # still the script", "  # a real comment", "  other: x"],
    );
}

// --- worktree.sharedDirectories ----------------------------------------------

/// R21. ss-magic owns the list outright: a hand-added entry is replaced.
#[test]
fn an_existing_list_with_a_hand_added_entry_is_replaced() {
    let current = "worktree:\n  sharedDirectories:\n    - node_modules\n    - hand/added\n  other: keep\n";

    let out = edit(Some(current), &dirs(&["node_modules", "apps/web/node_modules"]), false).unwrap();

    assert_eq!(
        out,
        "worktree:\n  sharedDirectories:\n    - node_modules\n    - apps/web/node_modules\n  other: keep\n"
    );
}

#[test]
fn a_list_that_already_matches_is_left_byte_identical_whatever_its_style() {
    for current in [
        "worktree:\n  sharedDirectories: [node_modules, \".venv\"]   # flow, by hand\n",
        "worktree:\n  sharedDirectories:\n  - node_modules\n  - .venv\n",
    ] {
        let out = edit(Some(current), &dirs(&["node_modules", ".venv"]), false).unwrap();
        assert_eq!(out, current);
    }
}

#[test]
fn every_existing_list_form_is_replaced_by_the_given_list() {
    let cases = [
        "worktree:\n  sharedDirectories: [old, \"b ] c\"]   # tail\n  k: v\n",
        "worktree:\n  sharedDirectories: [a,\n    b, # x ]\n    c]\n  k: v\n",
        "worktree:\n  sharedDirectories: []\n  k: v\n",
        "worktree:\n  sharedDirectories:\n  k: v\n",
        "worktree:\n  sharedDirectories: ~\n  k: v\n",
        "worktree:\n  sharedDirectories: oops\n  k: v\n",
        // Compact style: the dashes sit at the key's own column.
        "worktree:\n  sharedDirectories:\n  - old\n  - older # last\n  # about k\n\n  k: v\n",
        "worktree:\n  k: v\n",
    ];
    for current in cases {
        let out = edit(Some(current), &dirs(&["node_modules", ".venv"]), false).unwrap();
        assert_eq!(
            shared_of(&out),
            Some(dirs(&["node_modules", ".venv"])),
            "from {current:?} got:\n{out}"
        );
        assert_lines_survive(&out, &["worktree:", "  k: v"]);
    }
}

#[test]
fn a_compact_list_keeps_its_indentation_style_and_the_comments_after_it() {
    let current = "worktree:\n  sharedDirectories:\n  - old\n  # about k\n\n  k: v\n";

    let out = edit(Some(current), &dirs(&["node_modules", ".venv"]), false).unwrap();

    assert_eq!(
        out,
        "worktree:\n  sharedDirectories:\n  - node_modules\n  - .venv\n  # about k\n\n  k: v\n"
    );
}

/// Only the item lines of a block list are rewritten: a comment on the key
/// line and comment lines above the first item stay where they were.
#[test]
fn comments_around_a_block_list_survive_its_replacement() {
    let current = "worktree:\n  sharedDirectories: # shared with every worktree\n    # the root one\n    - old\n    - older # gone with its item\n  # about k\n  k: v\n";

    let out = edit(Some(current), &dirs(&["node_modules"]), false).unwrap();

    assert_eq!(
        out,
        "worktree:\n  sharedDirectories: # shared with every worktree\n    # the root one\n    \
         - node_modules\n  # about k\n  k: v\n"
    );

    let emptied = edit(Some(current), &[], false).unwrap();
    assert_eq!(
        emptied,
        "worktree:\n  sharedDirectories: [] # shared with every worktree\n    # the root one\n  \
         # about k\n  k: v\n"
    );
    assert_eq!(shared_of(&emptied), Some(Vec::new()));
}

/// The list is the last thing in the file, with and without a final newline.
#[test]
fn a_block_list_at_the_end_of_the_file_is_replaced_cleanly() {
    for (current, expected, emptied) in [
        (
            "worktree:\n  sharedDirectories:\n    - old\n",
            "worktree:\n  sharedDirectories:\n    - node_modules\n",
            "worktree:\n  sharedDirectories: []\n",
        ),
        (
            "worktree:\n  sharedDirectories:\n    - old",
            "worktree:\n  sharedDirectories:\n    - node_modules",
            "worktree:\n  sharedDirectories: []\n",
        ),
    ] {
        assert_eq!(edit(Some(current), &dirs(&["node_modules"]), false).unwrap(), expected);
        assert_eq!(edit(Some(current), &[], false).unwrap(), emptied);
    }
}

#[test]
fn emptying_the_list_writes_an_empty_sequence() {
    let current = "worktree:\n  sharedDirectories:\n    - node_modules\n  k: v\n";

    let out = edit(Some(current), &[], false).unwrap();

    assert_eq!(out, "worktree:\n  sharedDirectories: []\n  k: v\n");
}

/// A directory name YAML would read as something other than the same string is
/// quoted; ordinary paths stay plain.
#[test]
fn unusual_directory_names_are_quoted_so_they_read_back_as_strings() {
    let shared = dirs(&[
        "node_modules",
        "apps/web/.next/cache",
        "123",
        "true",
        "dir with space",
        "weird: name #x",
        "- dash",
        "café/node_modules",
        "@scope/pkg/node_modules",
        ".5e3",
        "cafe",
    ]);

    let out = edit(None, &shared, false).unwrap();

    assert_eq!(shared_of(&out), Some(shared));
    assert!(out.contains("\n    - node_modules\n"), "plain when safe:\n{out}");
    assert!(out.contains("\n    - apps/web/.next/cache\n"), "plain when safe:\n{out}");
    assert!(out.contains("\n    - cafe\n"), "plain when safe:\n{out}");
    assert!(out.contains("\n    - \"123\"\n"), "quoted when numeric:\n{out}");
    assert!(out.contains("\n    - \".5e3\"\n"), "quoted when a leading-dot number:\n{out}");
    assert!(out.contains("\n    - \"true\"\n"), "quoted when a keyword:\n{out}");
}

// --- preservation -------------------------------------------------------------

#[test]
fn rich_comments_four_space_indentation_and_blank_lines_survive() {
    let current = "\
# Orca config – edited by hand
# second header line

scripts:
    # runs once per worktree
    setup: |
        pnpm install   # not a comment: part of the script

    archive: rm -rf dist # tidy up

# tabs the team wants open
defaultTabs:
    - title: dev   # the dev server
      command: pnpm dev

environmentRecipes:
    # nothing yet
    default: {}
issueCommand: gh issue view
";
    let kept: Vec<&str> = current
        .lines()
        .filter(|line| !line.contains("setup: |"))
        .collect();

    let out = edit(Some(current), &dirs(&["node_modules", "apps/web/node_modules"]), true).unwrap();

    assert_lines_survive(&out, &kept);
    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\npnpm install   # not a comment: part of the script\n")
    );
    assert!(
        out.contains("    setup: |\n        ./.superset/magic.sh sync\n        pnpm install"),
        "the script keeps the file's 4-space indentation:\n{out}"
    );
    assert!(
        out.ends_with(
            "issueCommand: gh issue view\nworktree:\n    sharedDirectories:\n        - node_modules\n        - apps/web/node_modules\n"
        ),
        "the appended block uses the file's indentation step:\n{out}"
    );
}

#[test]
fn a_crlf_file_gets_crlf_on_every_inserted_line() {
    let current = "scripts:\r\n  setup: pnpm install\r\ndefaultTabs: []\r\n";

    let out = edit(Some(current), &dirs(&["node_modules"]), true).unwrap();

    assert_eq!(
        out,
        "scripts:\r\n  setup: |-\r\n    ./.superset/magic.sh sync\r\n    pnpm install\r\n\
         defaultTabs: []\r\nworktree:\r\n  sharedDirectories:\r\n    - node_modules\r\n"
    );
    assert!(
        !out.replace("\r\n", "").contains('\n'),
        "no bare line feed may be introduced into a CRLF file"
    );
    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\npnpm install")
    );
}

#[test]
fn a_file_without_a_trailing_newline_gets_the_block_on_a_new_line() {
    let current = "defaultTabs: []";

    let out = edit(Some(current), &dirs(&["node_modules"]), true).unwrap();

    assert!(out.starts_with("defaultTabs: []\nscripts:\n"), "got:\n{out}");
    assert_eq!(shared_of(&out), Some(dirs(&["node_modules"])));
}

#[test]
fn a_byte_order_mark_is_preserved_and_does_not_hide_the_first_key() {
    let current = "\u{feff}scripts:\n  setup: pnpm install\n";

    let out = edit(Some(current), &[], true).unwrap();

    assert!(out.starts_with("\u{feff}scripts:\n"), "the BOM stays first");
    assert_eq!(out.matches('\u{feff}').count(), 1);
    assert_eq!(out.matches("scripts:").count(), 1, "no second scripts block:\n{out}");
    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\npnpm install")
    );
}

#[test]
fn non_ascii_text_before_the_managed_keys_does_not_shift_the_edit() {
    let current = "# Konfigurace – žluťoučký kůň 🐴\nscripts:\n  setup: echo \"příliš\"\nname: žába\n";

    let out = edit(Some(current), &dirs(&["node_modules"]), true).unwrap();

    assert_eq!(
        setup_of(&out).as_deref(),
        Some("./.superset/magic.sh sync\necho \"příliš\"")
    );
    assert_lines_survive(&out, &["# Konfigurace – žluťoučký kůň 🐴", "scripts:", "name: žába"]);
}

#[test]
fn a_leading_document_marker_is_kept() {
    let current = "---\nscripts:\n  setup: make\n";
    let out = edit(Some(current), &[], true).unwrap();
    assert!(out.starts_with("---\nscripts:\n"));
    assert_eq!(setup_of(&out).as_deref(), Some("./.superset/magic.sh sync\nmake"));
}

// --- refusals -----------------------------------------------------------------

/// AE9. A flow-style `scripts` is named as the reason, and the refusal carries
/// the block to add by hand.
#[test]
fn ae9_flow_mapping_scripts_is_refused_with_the_block_to_add_by_hand() {
    let r = refusal("scripts: {setup: make}\n", &["node_modules"], true);

    assert!(r.reason.contains("`scripts` is a flow mapping"), "{}", r.reason);
    assert_eq!(
        r.manual_block,
        "scripts:\n  setup: |\n    ./.superset/magic.sh sync\nworktree:\n  sharedDirectories:\n    - node_modules\n"
    );
}

#[test]
fn each_uneditable_layout_is_refused_with_a_reason_naming_it() {
    let cases: [(&str, &str); 14] = [
        ("scripts:\nworktree:\n  k: v\n", "`scripts` is present but empty"),
        ("scripts: ~\n", "`scripts` is present but empty"),
        ("scripts: just a string\n", "`scripts` is not a mapping"),
        ("scripts: [a, b]\n", "`scripts` is not a mapping"),
        ("base: &b\n  setup: make\nscripts: *b\n", "`scripts` is an alias"),
        ("scripts: &anchored\n  setup: make\n", "anchor or a tag"),
        ("scripts: !!map\n  setup: make\n", "anchor or a tag"),
        ("base: &b\n  x: 1\nscripts:\n  <<: *b\n  setup: make\n", "merge key"),
        ("scripts:\n  setup: &s make\n", "anchor, tag or alias"),
        ("a: 1\n---\nb: 2\n", "more than one YAML document"),
        ("%YAML 1.1\n---\nscripts:\n  setup: make\n", "%YAML 1.1"),
        ("scripts:\n  setup: a\nscripts:\n  setup: b\n", "not valid YAML"),
        ("scripts: [unclosed\nworktree: {", "not valid YAML"),
        ("- a\n- b\n", "top level is not a mapping"),
    ];
    for (current, expected) in cases {
        let r = refusal(current, &["node_modules"], true);
        assert!(
            r.reason.contains(expected),
            "for {current:?} expected a reason containing {expected:?}, got {:?}",
            r.reason
        );
        assert!(r.manual_block.contains(SETUP_LINE));
        assert!(r.manual_block.contains("- node_modules"));
    }
}

#[test]
fn an_uneditable_worktree_is_refused_when_its_list_must_change() {
    let cases = [
        ("worktree: {sharedDirectories: [old]}\n", "`worktree` is a flow mapping"),
        ("worktree:\n", "`worktree` is present but empty"),
        ("base: &w\n  sharedDirectories: [old]\nworktree: *w\n", "`worktree` is an alias"),
        ("list: &l [old]\nworktree:\n  sharedDirectories: *l\n", "anchor, tag or alias"),
    ];
    for (current, expected) in cases {
        let r = refusal(current, &["node_modules"], false);
        assert!(
            r.reason.contains(expected),
            "for {current:?} expected {expected:?}, got {:?}",
            r.reason
        );
        assert!(
            !r.manual_block.contains("scripts"),
            "with the setup switch off the block to add holds only the list"
        );
    }
}

/// A layout that needs no edit is no reason to refuse: the file already says
/// what was intended.
#[test]
fn an_unusual_layout_that_already_holds_the_intended_values_is_accepted() {
    let current =
        "scripts: {setup: ./.superset/magic.sh sync}\nworktree: {sharedDirectories: [node_modules]}\n";
    assert_eq!(edit(Some(current), &dirs(&["node_modules"]), true).unwrap(), current);
}

/// An anchor that another key refers to makes an edit reach outside the managed
/// keys. The re-parse check sees the other key change and refuses.
#[test]
fn an_edit_that_would_change_another_key_through_an_alias_is_refused() {
    let current = "worktree:\n  sharedDirectories: &dirs\n    - old\nelsewhere: *dirs\n";
    let r = refusal(current, &["node_modules"], false);
    assert!(!r.reason.is_empty());
}

#[test]
fn the_yaml_1_2_directive_is_accepted() {
    let current = "%YAML 1.2\n---\nscripts:\n  setup: make\n";
    let out = edit(Some(current), &[], true).unwrap();
    assert_eq!(setup_of(&out).as_deref(), Some("./.superset/magic.sh sync\nmake"));
}

// --- the pieces ----------------------------------------------------------------

#[test]
fn has_setup_line_matches_a_whole_trimmed_line_only() {
    assert!(has_setup_line("./.superset/magic.sh sync"));
    assert!(has_setup_line("pnpm i\n   ./.superset/magic.sh sync\t\npnpm build"));
    assert!(has_setup_line("a\r\n./.superset/magic.sh sync\r\nb"));
    assert!(!has_setup_line("./.superset/magic.sh sync --no-backup"));
    assert!(!has_setup_line("# ./.superset/magic.sh sync"));
    assert!(!has_setup_line("bash ./.superset/magic.sh sync"));
    assert!(!has_setup_line(""));
}

#[test]
fn block_scalar_picks_the_chomping_indicator_from_the_trailing_newlines() {
    assert_eq!(block_scalar("a\nb").unwrap(), ("|-".to_string(), vec!["a", "b"]));
    assert_eq!(block_scalar("a\nb\n").unwrap(), ("|".to_string(), vec!["a", "b"]));
    assert_eq!(
        block_scalar("a\n\nb\n\n\n").unwrap(),
        ("|+".to_string(), vec!["a", "", "b", "", ""])
    );
    assert!(block_scalar("").is_none());
    assert!(block_scalar("\n").is_none());
    assert!(block_scalar(" leading space").is_none());
    assert!(block_scalar("ok\n\ttab-led line").is_none());
    assert!(block_scalar("bell\u{7}").is_none());
    assert!(block_scalar("mid\ttab is fine").is_some());
}

#[test]
fn double_quoted_escapes_everything_that_needs_it() {
    assert_eq!(double_quoted("a\"b\\c\n\td\r\0\u{7}"), "\"a\\\"b\\\\c\\n\\td\\r\\0\\x07\"");
    assert_eq!(double_quoted("žába 🐸"), "\"žába 🐸\"");
}

#[test]
fn source_scanners_find_the_real_end_of_quoted_and_flow_values() {
    let src = Source::new("\"a \\\" b\"   # c");
    assert_eq!(src.scan_double_quoted(0), Some(8));
    let src = Source::new("'it''s' # c");
    assert_eq!(src.scan_single_quoted(0), Some(7));
    let src = Source::new("[a, \"b ] c\", {k: 'v }'}, it's] tail");
    assert_eq!(src.scan_flow(0), Some(30));
    let src = Source::new("[a, # ] not the end\n  b]x");
    assert_eq!(src.scan_flow(0), Some(24));
    assert_eq!(Source::new("[never closed").scan_flow(0), None);
    assert_eq!(Source::new("\"never closed").scan_double_quoted(0), None);
}

/// A sweep over broken and half-written files: every prefix of a rich
/// document, and the document with each single line removed. Whatever the
/// input, the editor either refuses or returns text that really holds the
/// intended values – and it never panics on a position it computed.
#[test]
fn truncated_and_mutilated_documents_are_either_refused_or_edited_correctly() {
    let documents = [
        "\
# header – ž
scripts:
    setup: |
        pnpm install   # in script

    archive: \"rm -rf 'dist'\" # tidy
worktree:
    sharedDirectories: [a, \"b ] c\", # x ]
        c]   # tail
    other: {k: [1, 2]}
defaultTabs:
    - title: dev
      command: 'pnpm ''dev'''
",
        "scripts:\r\n  setup: >-\r\n    one\r\n    two\r\nworktree:\r\n  sharedDirectories:\r\n  - x\r\n  - y # c\r\n  # after\r\n\r\nlast: &a [1]\r\nagain: *a\r\n",
        "worktree:\n  sharedDirectories:\n    - \"quoted # not a comment\"\n    - |\n      block item\n  k: v\nscripts:\n  setup: \"a\\nb\" # c\n",
    ];
    let shared = dirs(&["node_modules", "apps/web/.next/cache", "dir with space"]);

    let check = |text: &str| {
        for ensure_setup in [true, false] {
            let Ok(out) = edit(Some(text), &shared, ensure_setup) else {
                continue;
            };
            assert_eq!(
                shared_of(&out).as_ref(),
                Some(&shared),
                "accepted an edit of {text:?} that does not hold the list:\n{out}"
            );
            if ensure_setup {
                assert!(
                    setup_of(&out).is_some_and(|script| has_setup_line(&script)),
                    "accepted an edit of {text:?} without the setup line:\n{out}"
                );
            } else {
                assert_eq!(setup_of(&out), setup_of_lenient(text), "scripts changed for {text:?}");
            }
            // And the result is stable: a second run has nothing left to do.
            assert_eq!(edit(Some(&out), &shared, ensure_setup).as_ref(), Ok(&out));
        }
    };

    for document in documents {
        let chars: Vec<char> = document.chars().collect();
        for cut in 0..=chars.len() {
            let prefix: String = chars[..cut].iter().collect();
            check(&prefix);
        }
        let lines: Vec<&str> = document.split_inclusive('\n').collect();
        for skip in 0..lines.len() {
            let without: String = lines
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, line)| *line)
                .collect();
            check(&without);
        }
    }
}

/// `setup_of`, for text that may not parse at all.
fn setup_of_lenient(text: &str) -> Option<String> {
    MarkedYaml::load_from_str(text).ok()?;
    setup_of(text)
}
