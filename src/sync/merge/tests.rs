use super::*;
use std::path::Path;

/// A one-line change yields exactly one Diff, and each MergeChoice reproduces
/// the expected assembled text (Local ⇒ local, Main ⇒ main, Both ⇒ both).
#[test]
fn single_diff_segments_and_assemble() {
    let local = "a\nb\nc\n";
    let main = "a\nB\nc\n";
    let segs = merge_segments(local, main);

    assert_eq!(diff_count(&segs), 1, "exactly one differing region; got {segs:?}");
    // Equal("a\n"), Diff{local:"b\n", main:"B\n"}, Equal("c\n")
    assert_eq!(
        segs,
        vec![
            MergeSegment::Equal("a\n".to_string()),
            MergeSegment::Diff {
                local: "b\n".to_string(),
                main: "B\n".to_string(),
            },
            MergeSegment::Equal("c\n".to_string()),
        ]
    );

    assert_eq!(assemble(&segs, &[MergeChoice::Local]), local);
    assert_eq!(assemble(&segs, &[MergeChoice::Main]), main);
    assert_eq!(assemble(&segs, &[MergeChoice::Both]), "a\nb\nB\nc\n");
}

/// An all-equal pair has zero diffs and assembles back to the identical input.
#[test]
fn all_equal_has_no_diffs_and_roundtrips() {
    let text = "a\nb\nc\n";
    let segs = merge_segments(text, text);
    assert_eq!(diff_count(&segs), 0);
    assert_eq!(segs, vec![MergeSegment::Equal(text.to_string())]);
    assert_eq!(assemble(&segs, &[]), text);
}

/// A missing per-hunk choice is treated as Local (documented default).
#[test]
fn assemble_missing_choice_defaults_to_local() {
    let segs = merge_segments("a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(assemble(&segs, &[]), "a\nb\nc\n");
}

/// keep-both on a Diff whose local side lacks a trailing newline (the final
/// line of a newline-less file) must NOT fuse the last local line onto the
/// first main line — a `\n` separator is inserted so the two stay distinct.
#[test]
fn assemble_both_inserts_separator_when_local_lacks_trailing_newline() {
    let segs = vec![MergeSegment::Diff {
        local: "LOCAL_LAST".to_string(), // no trailing newline
        main: "MAIN_FIRST\n".to_string(),
    }];
    let out = assemble(&segs, &[MergeChoice::Both]);
    assert_eq!(
        out, "LOCAL_LAST\nMAIN_FIRST\n",
        "the two sides must stay on distinct lines, not fuse into `LOCAL_LASTMAIN_FIRST`"
    );
    assert!(
        !out.contains("LOCAL_LASTMAIN_FIRST"),
        "no fusion of the local and main lines: {out:?}"
    );
}

/// keep-both on a pure-insert Diff (empty local side) inserts NO spurious
/// separator — the main side is emitted as-is.
#[test]
fn assemble_both_pure_insert_adds_no_separator() {
    let segs = vec![MergeSegment::Diff {
        local: String::new(),
        main: "MAIN_ONLY\n".to_string(),
    }];
    assert_eq!(assemble(&segs, &[MergeChoice::Both]), "MAIN_ONLY\n");
}

/// default_decision: worktree-only ⇒ Push; exists-both ⇒ Undecided.
#[test]
fn default_decision_is_conservative() {
    assert!(matches!(
        default_decision(FileState::WorktreeOnly),
        Decision::Push
    ));
    assert!(matches!(
        default_decision(FileState::ExistsBoth),
        Decision::Undecided
    ));
}

/// backup_rel_path joins the timestamp dir + side namespace onto the
/// repo-relative path, so the same rel backed up from both sides never
/// collides into one file.
#[test]
fn backup_rel_path_joins_ts_side_and_rel() {
    assert_eq!(
        backup_rel_path("20260716-153000", BackupSide::Main, Path::new("apps/api/.env")),
        Path::new("20260716-153000/main/apps/api/.env")
    );
    assert_eq!(
        backup_rel_path("20260716-153000", BackupSide::Worktree, Path::new("apps/api/.env")),
        Path::new("20260716-153000/worktree/apps/api/.env")
    );
}
