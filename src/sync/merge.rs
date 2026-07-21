//! Pure per-hunk merge model for the reverse-sync merge cockpit.
//!
//! These files (`.env`, `.dev.vars`, `magic.local.json`) have no common
//! ancestor, so reconciliation is a **base-less 2-way** walk: the local
//! (worktree) copy against the main copy. This module owns three pure
//! concerns and nothing else:
//!
//! - [`default_decision`] picks a file's starting [`Decision`] from its
//!   [`FileState`] — conservative: only worktree-only files auto-push,
//!   everything that differs starts [`Decision::Undecided`].
//! - [`merge_segments`] turns two texts into an ordered list of
//!   [`MergeSegment`]s (equal runs pass through; each differing region is one
//!   choice point), and [`assemble`] walks that list with a per-hunk
//!   [`MergeChoice`] list to produce the reconciled text.
//! - [`backup_rel_path`] is the timestamped backup naming primitive shared by
//!   the apply seam.
//!
//! It is deliberately free of any TUI / `ratatui` dependency so the logic
//! stays unit-testable in isolation. [`default_decision`] and
//! [`backup_rel_path`] are wired into the cockpit apply path; the per-hunk
//! merge machinery ([`merge_segments`], [`assemble`], [`diff_count`],
//! [`MergeSegment`], [`MergeChoice`], and [`Decision::Merge`]) drives the
//! cockpit's interactive-merge overlay ([`crate::tui::cockpit`]).

use std::path::{Path, PathBuf};

use similar::{ChangeTag, TextDiff};

/// A single file's reconcile direction in the cockpit.
///
/// [`Decision::Merge`] carries the fully assembled reconciled text (the output
/// of [`assemble`]) so the apply seam writes exact bytes with no re-diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No direction chosen yet — nothing is written for this file.
    Undecided,
    /// Push the worktree copy to main.
    Push,
    /// Pull main's copy into the worktree.
    Pull,
    /// Write this assembled reconciled text to BOTH sides.
    Merge(String),
    /// Delete the file from BOTH sides (whichever exist) — reconcile by
    /// removing. Every existing side is backed up before its unlink.
    Delete,
}

/// Whether a candidate exists on both sides (a real reconcile) or only in the
/// worktree (a plain new-file push).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Present in both the worktree and main (bytes differ — it is a candidate).
    ExistsBoth,
    /// Present only in the worktree; absent in main.
    WorktreeOnly,
    /// Present only in main; absent in the worktree. A pull creates it locally;
    /// a delete removes main's copy. NEW in the unified sync (Task 5).
    MainOnly,
}

/// The starting decision for a file in the unified Sync cockpit: NOTHING is
/// pre-selected (KD4). The user explicitly picks push / pull / merge / delete
/// for every file, so nothing destructive — and nothing at all — is auto-chosen.
/// This replaces the old worktree-only auto-push default: the unified set now
/// includes TRACKED worktree-only files, which must not be pushed to main on a
/// bare keystroke. `FileState` is retained for the cockpit's exhaustive
/// `file_state` mapping and future use.
pub fn default_decision(_state: FileState) -> Decision {
    Decision::Undecided
}

/// The per-hunk choice for one differing region during an interactive merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeChoice {
    /// Keep the local (worktree) side.
    Local,
    /// Keep the main side.
    Main,
    /// Keep both, local first then main.
    Both,
}

/// One segment of a base-less 2-way diff between the local and main texts.
///
/// [`MergeSegment::Equal`] regions are shared verbatim; each
/// [`MergeSegment::Diff`] is one differing region (a choice point) carrying the
/// two candidate texts. Consecutive equal lines coalesce into a single `Equal`
/// and each differing region (delete / insert / replace) coalesces into a
/// single `Diff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeSegment {
    /// A run of lines identical on both sides, kept verbatim.
    Equal(String),
    /// A differing region: the `local` candidate text vs the `main` candidate
    /// text (either may be empty for a pure insert / delete).
    Diff {
        /// The local (worktree) side of this region.
        local: String,
        /// The main side of this region.
        main: String,
    },
}

/// Pending accumulator while coalescing the flat change stream into segments.
enum Pending {
    Equal(String),
    Diff { local: String, main: String },
}

impl Pending {
    fn into_segment(self) -> MergeSegment {
        match self {
            Pending::Equal(s) => MergeSegment::Equal(s),
            Pending::Diff { local, main } => MergeSegment::Diff { local, main },
        }
    }
}

/// Compute the ordered [`MergeSegment`] list for `local` vs `main`.
///
/// Uses `similar`'s FULL line-diff ops (not grouped/context-folded) so `Equal`
/// regions are complete — the assembled output must reproduce every unchanged
/// line, not just folded context. Each op's changes are flattened into a single
/// stream and coalesced: consecutive equal lines become one [`MergeSegment::Equal`]
/// and each differing region (delete / insert / replace, and any adjacent runs)
/// becomes one [`MergeSegment::Diff`]. Line texts retain their trailing newline,
/// so concatenating segments round-trips the inputs.
pub fn merge_segments(local: &str, main: &str) -> Vec<MergeSegment> {
    let diff = TextDiff::from_lines(local, main);
    let mut out: Vec<MergeSegment> = Vec::new();
    let mut pending: Option<Pending> = None;

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let line = change.value(); // &str, includes the trailing '\n'
            pending = Some(match change.tag() {
                ChangeTag::Equal => match pending.take() {
                    Some(Pending::Equal(mut s)) => {
                        s.push_str(line);
                        Pending::Equal(s)
                    }
                    Some(other) => {
                        out.push(other.into_segment());
                        Pending::Equal(line.to_string())
                    }
                    None => Pending::Equal(line.to_string()),
                },
                ChangeTag::Delete => match pending.take() {
                    Some(Pending::Diff { mut local, main }) => {
                        local.push_str(line);
                        Pending::Diff { local, main }
                    }
                    Some(other) => {
                        out.push(other.into_segment());
                        Pending::Diff {
                            local: line.to_string(),
                            main: String::new(),
                        }
                    }
                    None => Pending::Diff {
                        local: line.to_string(),
                        main: String::new(),
                    },
                },
                ChangeTag::Insert => match pending.take() {
                    Some(Pending::Diff { local, mut main }) => {
                        main.push_str(line);
                        Pending::Diff { local, main }
                    }
                    Some(other) => {
                        out.push(other.into_segment());
                        Pending::Diff {
                            local: String::new(),
                            main: line.to_string(),
                        }
                    }
                    None => Pending::Diff {
                        local: String::new(),
                        main: line.to_string(),
                    },
                },
            });
        }
    }

    if let Some(p) = pending.take() {
        out.push(p.into_segment());
    }
    out
}

/// The number of choice points in `segments` — i.e. how many
/// [`MergeSegment::Diff`]s there are, which is the required length of the
/// `choices` slice passed to [`assemble`].
pub fn diff_count(segments: &[MergeSegment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, MergeSegment::Diff { .. }))
        .count()
}

/// Assemble the reconciled text from `segments` and the per-hunk `choices`.
///
/// Walks the segments in order: an [`MergeSegment::Equal`] is emitted verbatim;
/// the i-th [`MergeSegment::Diff`] consults `choices[i]` — [`MergeChoice::Local`]
/// emits the local side, [`MergeChoice::Main`] the main side, [`MergeChoice::Both`]
/// the local side then the main side. If `choices` is shorter than
/// [`diff_count`], any missing choice is treated as [`MergeChoice::Local`].
///
/// For [`MergeChoice::Both`], a missing line terminator between the two sides is
/// repaired: when a non-empty `local` side does not end in `\n` (the final line
/// of a file with no trailing newline), a `\n` is inserted before `main` so the
/// last local line and the first main line stay distinct rather than fusing into
/// one corrupted line.
pub fn assemble(segments: &[MergeSegment], choices: &[MergeChoice]) -> String {
    let mut out = String::new();
    let mut diff_idx = 0usize;
    for seg in segments {
        match seg {
            MergeSegment::Equal(text) => out.push_str(text),
            MergeSegment::Diff { local, main } => {
                // Missing choice defaults to Local (see doc comment).
                match choices.get(diff_idx).copied().unwrap_or(MergeChoice::Local) {
                    MergeChoice::Local => out.push_str(local),
                    MergeChoice::Main => out.push_str(main),
                    MergeChoice::Both => {
                        out.push_str(local);
                        // Keep the two sides on distinct lines: a local side
                        // without a trailing newline would otherwise fuse its
                        // last line onto main's first line. (An empty local
                        // side — a pure insert — needs no separator.)
                        if !local.is_empty() && !local.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str(main);
                    }
                }
                diff_idx += 1;
            }
        }
    }
    out
}

/// Which side of the reconcile a backup's losing bytes came from. Every backup
/// lives under a per-side namespace inside the batch's timestamp directory so
/// the SAME `rel` backed up from both sides (a merge or a delete) never
/// collides into one file, and so a whole batch is exactly one prunable
/// `<ts>/` directory (retention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupSide {
    /// The worktree copy's bytes.
    Worktree,
    /// The main-checkout copy's bytes.
    Main,
}

impl BackupSide {
    /// The namespace directory under the timestamp dir.
    fn dir_name(self) -> &'static str {
        match self {
            BackupSide::Worktree => "worktree",
            BackupSide::Main => "main",
        }
    }
}

/// The repo-relative backup path for `rel` under a timestamp + side directory,
/// e.g. `backup_rel_path("20260716-153000", BackupSide::Main, "apps/api/.env")`
/// → `20260716-153000/main/apps/api/.env`. Pure — no filesystem access; the
/// apply seam joins this onto its chosen backups root.
pub fn backup_rel_path(ts: &str, side: BackupSide, rel: &Path) -> PathBuf {
    Path::new(ts).join(side.dir_name()).join(rel)
}

#[cfg(test)]
mod tests;
