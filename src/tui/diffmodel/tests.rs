use super::*;

/// Concatenate a row-cell's segment texts back into a single string.
fn joined(segs: &[Seg]) -> String {
    segs.iter().map(|s| s.text.as_str()).collect()
}

/// True if any segment in the cell is emphasized.
fn any_emph(segs: &[Seg]) -> bool {
    segs.iter().any(|s| s.emphasized)
}

#[test]
fn classify_plain_text_is_text() {
    match classify_content(b"hello\nworld\n") {
        ContentKind::Text(s) => assert_eq!(s, "hello\nworld\n"),
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn classify_nul_byte_is_binary() {
    assert_eq!(classify_content(b"ab\0cd"), ContentKind::Binary);
}

#[test]
fn classify_invalid_utf8_is_binary() {
    // 0xFF is never valid UTF-8 and is not a NUL byte.
    assert_eq!(classify_content(&[0x66, 0x6f, 0xff, 0x6f]), ContentKind::Binary);
}

#[test]
fn classify_oversized_is_too_large() {
    let big = vec![b'a'; (MAX_DIFF_BYTES + 1) as usize];
    assert_eq!(classify_content(&big), ContentKind::TooLarge(MAX_DIFF_BYTES + 1));
}

#[test]
fn classify_at_cap_is_still_text() {
    // Exactly MAX_DIFF_BYTES is allowed; the cap is a strict `>`.
    let at_cap = vec![b'a'; MAX_DIFF_BYTES as usize];
    assert!(matches!(classify_content(&at_cap), ContentKind::Text(_)));
}

#[test]
fn side_by_side_equal_and_replace() {
    let rows = side_by_side("a\nb\nc\n", "a\nB\nc\n", 3);
    assert_eq!(rows.len(), 3, "no folds expected on a 3-line pair: {rows:?}");

    // Row 0: "a" equal on both sides, matching line numbers, no emphasis.
    assert_eq!(rows[0].tag, RowTag::Equal);
    assert_eq!(rows[0].left_no, Some(1));
    assert_eq!(rows[0].right_no, Some(1));
    assert_eq!(joined(&rows[0].left), "a");
    assert_eq!(joined(&rows[0].right), "a");
    assert!(!any_emph(&rows[0].left) && !any_emph(&rows[0].right));

    // Row 1: the changed line is a Replace, left "b" / right "B", emphasized.
    assert_eq!(rows[1].tag, RowTag::Replace);
    assert_eq!(rows[1].left_no, Some(2));
    assert_eq!(rows[1].right_no, Some(2));
    assert_eq!(joined(&rows[1].left), "b");
    assert_eq!(joined(&rows[1].right), "B");
    assert!(any_emph(&rows[1].left), "changed portion should be emphasized");
    assert!(any_emph(&rows[1].right), "changed portion should be emphasized");

    // Row 2: "c" equal, matching numbers.
    assert_eq!(rows[2].tag, RowTag::Equal);
    assert_eq!(rows[2].left_no, Some(3));
    assert_eq!(rows[2].right_no, Some(3));
}

#[test]
fn side_by_side_pure_insertion() {
    // main gains a line "b" between "a" and "c".
    let rows = side_by_side("a\nc\n", "a\nb\nc\n", 3);
    let ins: Vec<&DiffRow> = rows.iter().filter(|r| r.tag == RowTag::Insert).collect();
    assert_eq!(ins.len(), 1, "exactly one inserted row: {rows:?}");
    let r = ins[0];
    assert_eq!(r.left_no, None, "insert has no left line number");
    assert!(r.left.is_empty(), "insert leaves the left cell empty");
    assert_eq!(r.right_no, Some(2));
    assert_eq!(joined(&r.right), "b");
}

#[test]
fn side_by_side_pure_deletion() {
    // local has an extra line "b" that main lacks.
    let rows = side_by_side("a\nb\nc\n", "a\nc\n", 3);
    let del: Vec<&DiffRow> = rows.iter().filter(|r| r.tag == RowTag::Delete).collect();
    assert_eq!(del.len(), 1, "exactly one deleted row: {rows:?}");
    let r = del[0];
    assert_eq!(r.right_no, None, "delete has no right line number");
    assert!(r.right.is_empty(), "delete leaves the right cell empty");
    assert_eq!(r.left_no, Some(2));
    assert_eq!(joined(&r.left), "b");
}

#[test]
fn side_by_side_folds_long_unchanged_run() {
    // 21 identical lines (index 0..=20) with a single change at index 10.
    let local: String = (0..21).map(|i| format!("l{i}\n")).collect();
    let main: String = (0..21)
        .map(|i| if i == 10 { "CHANGED\n".to_string() } else { format!("l{i}\n") })
        .collect();

    let rows = side_by_side(&local, &main, 3);

    let folds: Vec<&DiffRow> = rows
        .iter()
        .filter(|r| matches!(r.tag, RowTag::Fold(_)))
        .collect();
    assert_eq!(folds.len(), 1, "one leading fold expected: {rows:?}");
    // Change at index 10, context 3 → group starts at index 7 → 7 lines hidden.
    assert_eq!(folds[0].tag, RowTag::Fold(7));

    let shown = rows.len() - folds.len();
    // 3 context before + 1 replace + 3 context after = 7 visible rows.
    assert_eq!(shown, 7, "only ~context lines around the change are shown: {rows:?}");
}

#[test]
fn unified_known_pair() {
    let rows = unified("a\nb\nc\n", "a\nB\nc\n", 3);
    assert_eq!(rows.len(), 4, "a, -b, +B, c: {rows:?}");

    assert_eq!(rows[0].tag, UnifiedTag::Context);
    assert_eq!(rows[0].old_no, Some(1));
    assert_eq!(rows[0].new_no, Some(1));

    assert_eq!(rows[1].tag, UnifiedTag::Delete);
    assert_eq!(rows[1].old_no, Some(2));
    assert_eq!(rows[1].new_no, None);
    assert_eq!(joined(&rows[1].segs), "b");

    assert_eq!(rows[2].tag, UnifiedTag::Insert);
    assert_eq!(rows[2].old_no, None);
    assert_eq!(rows[2].new_no, Some(2));
    assert_eq!(joined(&rows[2].segs), "B");

    assert_eq!(rows[3].tag, UnifiedTag::Context);
    assert_eq!(rows[3].old_no, Some(3));
    assert_eq!(rows[3].new_no, Some(3));
}

#[test]
fn unified_folds_hidden_context() {
    let local: String = (0..21).map(|i| format!("l{i}\n")).collect();
    let main: String = (0..21)
        .map(|i| if i == 10 { "CHANGED\n".to_string() } else { format!("l{i}\n") })
        .collect();

    let rows = unified(&local, &main, 3);
    let folds: Vec<&UnifiedRow> = rows
        .iter()
        .filter(|r| matches!(r.tag, UnifiedTag::Fold(_)))
        .collect();
    assert_eq!(folds.len(), 1);
    assert_eq!(folds[0].tag, UnifiedTag::Fold(7));
}

#[test]
fn should_split_thresholds() {
    assert!(!should_split(80));
    assert!(should_split(120));
    // Boundary: exactly the minimum splits.
    assert!(should_split(100));
    assert!(!should_split(99));
}

/// Two CRLF-identical buffers produce only equal/context rows and leave NO
/// stray `\r` in any rendered segment (the full terminator is trimmed, so CRLF
/// text neither shows a control char nor registers as changed).
#[test]
fn crlf_identical_buffers_have_no_stray_cr_and_no_changes() {
    let local = "a\r\nb\r\nc\r\n";
    let main = "a\r\nb\r\nc\r\n";

    let rows = side_by_side(local, main, 3);
    assert!(
        rows.iter().all(|r| r.tag == RowTag::Equal),
        "identical CRLF must yield only equal rows: {rows:?}"
    );
    for r in &rows {
        for s in r.left.iter().chain(r.right.iter()) {
            assert!(!s.text.contains('\r'), "stray CR in side-by-side segment: {:?}", s.text);
        }
    }

    let urows = unified(local, main, 3);
    assert!(
        urows.iter().all(|r| r.tag == UnifiedTag::Context),
        "identical CRLF must be all context: {urows:?}"
    );
    for r in &urows {
        for s in &r.segs {
            assert!(!s.text.contains('\r'), "stray CR in unified segment: {:?}", s.text);
        }
    }
}
