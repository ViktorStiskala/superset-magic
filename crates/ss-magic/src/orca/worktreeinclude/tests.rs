use super::*;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn section(entries: &[&str]) -> String {
    let mut out = format!("{BEGIN_LINE}\n");
    for entry in entries {
        out.push_str(entry);
        out.push('\n');
    }
    out.push_str(END_LINE);
    out.push('\n');
    out
}

// --- section I/O ----------------------------------------------------------

#[test]
fn an_absent_file_renders_as_the_section_alone() {
    let rendered = Document::empty().render(&strings(&[".env", "apps/web/.env"]));
    assert_eq!(rendered, section(&[".env", "apps/web/.env"]));
}

#[test]
fn hand_lines_above_and_below_the_section_are_untouched() {
    let above = "# my own notes\nsecrets/local.pem\n\n";
    let below = "\n# added later\nconfig/dev.json\n";
    let text = format!("{above}{}{below}", section(&["old/.env", "stale/.env"]));

    let rendered = Document::parse(&text).render(&strings(&[".env"]));

    assert_eq!(rendered, format!("{above}{}{below}", section(&[".env"])));
}

#[test]
fn a_file_without_a_section_gains_one_at_the_end() {
    let text = "# hand-written\nsecrets/local.pem\n";

    let rendered = Document::parse(text).render(&strings(&[".env"]));

    assert_eq!(rendered, format!("{text}{}", section(&[".env"])));
    assert!(rendered.starts_with(text), "hand lines are byte-identical");
}

#[test]
fn a_last_line_without_a_line_break_is_not_glued_to_the_marker() {
    let rendered = Document::parse("secrets/local.pem").render(&strings(&[".env"]));
    assert_eq!(rendered, format!("secrets/local.pem\n{}", section(&[".env"])));
}

#[test]
fn rendering_twice_with_the_same_entries_gives_identical_bytes() {
    for text in [
        "",
        "# notes\nsecrets/local.pem",
        "a\n\n# >>> ss-magic old wording >>>\nx\n# <<< ss-magic <<<\nb\n",
    ] {
        let managed = strings(&[".env", "apps/web/.env"]);
        let once = Document::parse(text).render(&managed);
        let twice = Document::parse(&once).render(&managed);
        assert_eq!(once, twice, "from {text:?}");
    }
}

#[test]
fn a_section_written_with_other_marker_wording_is_still_replaced() {
    let text = "keep-me\n# >>> ss-magic (an older release's wording) >>>\nold/.env\n# <<< ss-magic end <<<\n";

    let rendered = Document::parse(text).render(&strings(&[".env"]));

    assert_eq!(rendered, format!("keep-me\n{}", section(&[".env"])));
}

/// A begin marker whose end marker is gone does not swallow the rest of the
/// file: the lines after it may be the person's, so they are kept as theirs.
#[test]
fn an_unterminated_section_keeps_the_lines_after_it() {
    let text = "above\n# >>> ss-magic: generated >>>\nmaybe/theirs.env\nbelow\n";

    let doc = Document::parse(text);

    assert_eq!(doc.hand_entries(), strings(&["above", "maybe/theirs.env", "below"]));
    let rendered = doc.render(&strings(&[".env"]));
    assert_eq!(
        rendered,
        format!("above\n{}maybe/theirs.env\nbelow\n", section(&[".env"]))
    );
}

#[test]
fn a_crlf_file_gets_a_crlf_section() {
    let text = "# notes\r\nsecrets/local.pem\r\n";

    let rendered = Document::parse(text).render(&strings(&[".env"]));

    assert_eq!(
        rendered,
        format!("{text}{BEGIN_LINE}\r\n.env\r\n{END_LINE}\r\n")
    );
    assert_eq!(entries(&rendered), strings(&["secrets/local.pem", ".env"]));
}

#[test]
fn both_marker_lines_are_comments_to_orca() {
    assert!(BEGIN_LINE.starts_with('#') && END_LINE.starts_with('#'));
    assert!(BEGIN_LINE.is_ascii() && END_LINE.is_ascii());
    assert!(BEGIN_LINE.starts_with(BEGIN_PREFIX) && END_LINE.starts_with(END_PREFIX));
    assert!(entries(&section(&[])).is_empty(), "an empty section lists nothing");
}

// --- reading entries --------------------------------------------------------

#[test]
fn entries_cover_both_the_managed_section_and_the_hand_lines() {
    let text = format!("hand/a.env\n{}hand/b.env\n", section(&[".env", "apps/web/.env"]));
    assert_eq!(
        entries(&text),
        strings(&["hand/a.env", ".env", "apps/web/.env", "hand/b.env"])
    );
    assert_eq!(
        Document::parse(&text).hand_entries(),
        strings(&["hand/a.env", "hand/b.env"])
    );
}

/// The reader mirrors Orca's: same trimming, same normalization, same
/// rejections, same de-duplication.
#[test]
fn entries_are_read_the_way_orca_reads_them() {
    let text = "\u{feff}# a comment\n\n   \n  .env  \r\n./apps/web/.env\nconfig/local/\n\
                win\\style\\path.env\n.env\n   # indented comment\nname # not a comment\n\
                *.env\nsecrets/?.pem\n!negated\n/absolute/path\n../escape\na/../b\na//b\n\
                .git/config\n.gitignore-like\n";

    assert_eq!(
        entries(text),
        strings(&[
            ".env",
            "apps/web/.env",
            "config/local",
            "win/style/path.env",
            "name # not a comment",
            ".gitignore-like",
        ])
    );
}

#[test]
fn listable_paths_are_exactly_those_orca_reads_back_unchanged() {
    for ok in [".env", "apps/web/.env", "dir with space/file name.txt", "a#b", "ünï/cödé.env"] {
        assert!(is_listable(ok), "{ok:?} should be listable");
    }
    for bad in [
        " leading",
        "trailing ",
        "#hash-first",
        "!bang-first",
        "star*.env",
        "what?.env",
        "back\\slash",
        "./dot-slash",
        "trailing/",
        "two\nlines",
        "carriage\rreturn",
        "a//b",
        "../up",
        ".git/config",
        "/abs",
        "",
    ] {
        assert!(!is_listable(bad), "{bad:?} must not be listable");
    }
}

// --- budget -----------------------------------------------------------------

#[test]
fn the_budget_is_what_orca_allows_minus_the_hand_lines() {
    let text = "# five of mine\na\nb\nc\nd\ne\n";
    let doc = Document::parse(text);

    let budget = doc.budget();

    assert_eq!(budget.max_entries, MAX_ENTRIES - 5);
    assert_eq!(budget.per_entry_overhead, 1);
    assert_eq!(budget.max_bytes, MAX_BYTES - doc.render(&[]).len());
    // The same file with a section already in it leaves the same room.
    let with_section = doc.render(&strings(&["x", "y"]));
    assert_eq!(Document::parse(&with_section).budget(), budget);
}

#[test]
fn hand_lines_that_fill_orcas_limits_leave_no_budget() {
    let many: String = (0..1200).map(|i| format!("hand/{i}\n")).collect();
    assert_eq!(Document::parse(&many).budget().max_entries, 0);

    let huge = format!("{}\n", "x".repeat(MAX_BYTES + 10));
    assert_eq!(Document::parse(&huge).budget().max_bytes, 0);
}

// --- reading the file ---------------------------------------------------------

#[test]
fn read_tells_absent_text_and_unreadable_apart() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read(dir.path()), Existing::Absent);

    fs::write(dir.path().join(FILE_NAME), "secrets/local.pem\n").unwrap();
    assert_eq!(read(dir.path()), Existing::Text("secrets/local.pem\n".to_string()));

    // Latin-1 bytes are not UTF-8.
    fs::write(dir.path().join(FILE_NAME), b"caf\xe9/.env\n").unwrap();
    assert!(matches!(read(dir.path()), Existing::Unreadable(why) if why.contains("UTF-8")));

    fs::remove_file(dir.path().join(FILE_NAME)).unwrap();
    fs::create_dir(dir.path().join(FILE_NAME)).unwrap();
    assert!(matches!(read(dir.path()), Existing::Unreadable(_)));
}
