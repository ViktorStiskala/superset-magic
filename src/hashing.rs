//! A single non-cryptographic content fingerprint (KTD3), shared by every
//! caller that needs "did these bytes change" rather than a security
//! guarantee: reverse sync's mtime-less TOCTOU fallback today, and (from the
//! plugin units onward) the conclusion cache and ledger, whose cache key
//! fingerprints `(realpath, size, mtime)` per R24.
//!
//! Deliberately NOT `std::collections::hash_map::DefaultHasher`: std
//! documents its output as unstable across releases (and even across
//! processes of the same build), so a cache key built from it can change out
//! from under a long-lived cache for no reason a user could ever see —
//! indistinguishable from the cache silently not working. FNV-1a's constants
//! are fixed forever, so [`fnv1a_64`] for a given input is stable across
//! every Rust version, every platform, every rebuild. The threat here is
//! accidental collision from ordinary file edits, not an adversary
//! constructing one, so a non-cryptographic hash is the right tool — but it
//! must be a hash whose output is a promise, not an implementation detail.

use std::path::Path;

use anyhow::{Context, Result};

/// FNV-1a's fixed starting value. Never changes between inputs or runs.
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
/// FNV-1a's fixed prime multiplier.
const FNV_PRIME: u64 = 0x100000001b3;

/// FNV-1a over `bytes`: XOR each byte into the running hash, then multiply
/// by the fixed prime. Pinned by a test asserting one literal input hashes
/// to one literal `u64` — changing this function is a failing test, not a
/// silent cache flush for every user with a warm cache.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Content fingerprint of the whole file at `path`. Callers use this only
/// where a full read is cheap (small files) and the alternative — trusting
/// the filesystem's mtime — does not hold.
pub fn hash_file(path: &Path) -> Result<u64> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading {} to hash its contents", path.display()))?;
    Ok(fnv1a_64(&bytes))
}

#[cfg(test)]
mod tests;
