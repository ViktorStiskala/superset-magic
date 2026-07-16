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

/// backup_rel_path joins the timestamp dir onto the repo-relative path.
#[test]
fn backup_rel_path_joins_ts_and_rel() {
    assert_eq!(
        backup_rel_path("20260716-153000", Path::new("apps/api/.env")),
        Path::new("20260716-153000/apps/api/.env")
    );
}
