use super::*;
use std::fs;

/// KTD3's whole point: pin `fnv1a_64` to a literal output for a literal
/// input. If anyone ever changes the constants or the algorithm, this test
/// goes red — the alternative is a cache key that silently changes on a
/// rebuild, which looks exactly like the cache just not working.
#[test]
fn fnv1a_64_matches_known_vector() {
    assert_eq!(fnv1a_64(b"hello"), 0xa430d84680aabd0b);
}

/// The empty input still has a defined hash: the FNV-1a offset basis
/// itself, since the loop never runs.
#[test]
fn fnv1a_64_of_empty_input_is_offset_basis() {
    assert_eq!(fnv1a_64(b""), FNV_OFFSET_BASIS);
}

/// Same bytes, different length-adjacent inputs, must not collide — a
/// sanity check that the hash actually mixes in every byte rather than only
/// the last one written.
#[test]
fn fnv1a_64_distinguishes_similar_inputs() {
    assert_ne!(fnv1a_64(b"hello"), fnv1a_64(b"hellp"));
    assert_ne!(fnv1a_64(b"hello"), fnv1a_64(b"hello "));
}

#[test]
fn hash_file_matches_fnv1a_64_of_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("content.txt");
    fs::write(&path, b"hello").unwrap();
    assert_eq!(hash_file(&path).unwrap(), fnv1a_64(b"hello"));
}

/// An empty file hashes the same as an empty byte slice — the read-then-hash
/// path has no special case that would make a zero-length file behave
/// differently from any other input.
#[test]
fn hash_file_of_empty_file_is_offset_basis() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.txt");
    fs::write(&path, b"").unwrap();
    assert_eq!(hash_file(&path).unwrap(), FNV_OFFSET_BASIS);
}

/// A large file (bigger than any single read buffer) still hashes correctly
/// — the read is a single `fs::read` of the whole file, so there is no
/// buffering boundary that could drop or duplicate bytes.
#[test]
fn hash_file_over_large_content_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.bin");
    let content = vec![0xABu8; 5 * 1024 * 1024]; // 5 MiB, uniform bytes.
    fs::write(&path, &content).unwrap();
    let first = hash_file(&path).unwrap();
    let second = hash_file(&path).unwrap();
    assert_eq!(first, second, "hashing the same large file twice must agree");
    assert_eq!(first, fnv1a_64(&content));
}

#[test]
fn hash_file_missing_path_errors() {
    let dir = tempfile::tempdir().unwrap();
    let err = hash_file(&dir.path().join("nope.txt")).unwrap_err();
    assert!(
        format!("{err:#}").contains("nope.txt"),
        "error should name the unreadable path"
    );
}
