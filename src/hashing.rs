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
//!
//! [`sha256`] lives here too even though it serves a different need: R80
//! pins the plugin's private temporary-root identifier to the first 16 hex
//! characters of the SHA-256 of `$HOME`, and that literal choice of algorithm
//! is load-bearing because TWO independent implementations must derive the
//! same string — this crate's `tmproot.rs` in Rust, and the plugin's
//! `bootstrap.sh` in shell (`shasum -a 256`). FNV-1a is not a substitute:
//! nothing requires the shell side and this crate to agree on a
//! home-grown, non-standard hash, whereas SHA-256 is a name every
//! `shasum`/`sha256sum` on every platform already implements identically.
//! Adding a crate for one hash felt like a bigger footprint than ~80 lines of
//! the reference algorithm, so this is a plain from-scratch implementation of
//! FIPS 180-4, kept honest by the pinned test vectors in `tests.rs`.

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

/// SHA-256's eight initial hash values — the fractional parts of the square
/// roots of the first eight primes, fixed by the standard.
const SHA256_H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256's 64 round constants — the fractional parts of the cube roots of
/// the first 64 primes, fixed by the standard.
#[rustfmt::skip]
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 (FIPS 180-4) over `data`, returned as the raw 32-byte digest. A
/// plain from-scratch implementation of the padding + 64-round compression
/// loop the standard defines — see the module docs for why this exists
/// alongside FNV-1a instead of reusing it, and `tests.rs` for the pinned
/// vectors that keep it honest.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    // Padding: a single `1` bit (byte 0x80, since everything here is
    // byte-aligned), then zeros up to 8 bytes short of a 64-byte boundary,
    // then the original bit length as a big-endian u64 — exactly filling the
    // final block.
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h = SHA256_H0;
    // The padding above always lands `msg.len()` on a 64-byte boundary, so
    // `as_chunks` never has a remainder to worry about.
    let (blocks, _) = msg.as_chunks::<64>();
    for block in blocks {
        // Message schedule: the block's 16 big-endian words, expanded to 64
        // via the standard's two sigma functions.
        let mut w = [0u32; 64];
        let (words, _) = block.as_chunks::<4>();
        for (i, word) in words.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    let (chunks, _) = out.as_chunks_mut::<4>();
    for (word, chunk) in h.iter().zip(chunks.iter_mut()) {
        *chunk = word.to_be_bytes();
    }
    out
}

/// `sha256`'s digest, rendered as lowercase hex. A thin formatting helper so
/// callers that want a string (R80's temp-root identifier, a diagnostic)
/// don't each hand-roll the same `format!("{:02x}", byte)` loop.
pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests;
