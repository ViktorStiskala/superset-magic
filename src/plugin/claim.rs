//! Taking a one-shot file claim so that exactly one caller wins it.
//!
//! Several parts of the plugin keep "this may happen once" as a file in the
//! state tree: the Read gate's per-file bypass token, and the pending
//! subagent-output declaration a `SubagentStop` consumes. Both have the same
//! hard requirement — the file must be taken **exactly once** even when
//! several processes reach for it at the same instant, because a hook is
//! spawned per event and the harness happily spawns duplicates.
//!
//! ## Why the obvious implementation is wrong
//!
//! "Delete the file, and treat a successful delete as the claim" does not
//! work. It was measured on macOS: with eight threads unlinking one path
//! behind a barrier, **five to eight of them get `Ok`** and only the
//! stragglers see `ENOENT`. Run the same two calls sequentially and the second
//! `unlink` correctly reports `ENOENT`, so the bug is invisible to any test
//! that does not actually race — which is exactly how a one-shot claim would
//! have shipped letting a whole burst through.
//!
//! `rename` measured clean on the same machine, in the same shape: exactly one
//! winner, every trial. So the claim is a rename. Each caller creates a
//! private landing file in the claim's own directory and renames the claim
//! onto it; `rename` needs its source to exist, so exactly one caller's rename
//! finds it there. The landing file unlinks itself when it drops, which is
//! what takes the claim out of circulation. No lock is involved — this is one
//! atomic syscall, not a read-modify-write, which is what satisfies R48 here.
//!
//! The landing file lives in the same directory as the claim so the rename
//! never crosses a filesystem (a cross-device `rename` fails outright), and it
//! is named with a leading dot and a `.tmp` suffix so no directory listing
//! mistakes it for a claim of its own.

use std::fs;
use std::path::Path;

use tempfile::NamedTempFile;

/// A claim this caller won.
///
/// Holding one means the claim file has already been moved out of the way, so
/// no other caller can win it. Dropping this value deletes it for good; read
/// whatever the claim recorded through [`Claimed::text`] first.
pub struct Claimed {
    /// The claim, now living under a private name of ours. Its `Drop` unlinks
    /// the path, which is what retires the claim.
    landing: NamedTempFile,
}

impl Claimed {
    /// What the claim file held, or `None` when it cannot be read at all.
    ///
    /// Read only after winning, so nothing can change under a caller between
    /// deciding it has the claim and looking at what the claim said.
    pub fn text(&self) -> Option<String> {
        fs::read_to_string(self.landing.path()).ok()
    }
}

/// Try to take the claim at `path`, whose directory is `dir`.
///
/// `Some` means this caller won it and may act; `None` means there was no
/// claim there, or another caller took it first, or the directory does not
/// exist — all of which mean the same thing to a caller, namely that this is
/// not the invocation that gets to act.
pub fn take(dir: &Path, path: &Path) -> Option<Claimed> {
    // The landing spot has a name no claim can ever have, so a listing of the
    // directory never confuses the two, and dropping it unlinks whatever ended
    // up at that path — the claim, once the rename below has moved it there.
    let landing = tempfile::Builder::new()
        .prefix(".taken-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .ok()?;

    fs::rename(path, landing.path()).ok()?;

    Some(Claimed { landing })
}

#[cfg(test)]
mod tests;
