//! `ss-magic-core`: what the `ss-magic` sync CLI and the Claude Code plugin
//! binary share. Nothing here opens a terminal UI, spawns an updater, or
//! parses argv – those are the binaries' jobs, and keeping them out of core
//! is what lets the plugin be unable to self-update or open a TUI by
//! construction rather than by convention.
//!
//! - [`git`] – the subprocess probes and mutating primitives, the `.gitignore`
//!   helpers, and `discover`, the one filesystem-only reduction of two probes.
//! - [`hashing`] – FNV-1a for cache keys and a hand-rolled SHA-256 the shell
//!   bootstrap reproduces with `shasum`.
//! - [`style`] – the palette and the process-wide color decision (no `inquire`).
//! - [`sync`] – the excluded-trees rule, pattern syntax, the working-tree scan
//!   and the glob/exclude/copy engine.
//! - [`superset_files`] – `.superset/{config.json, magic.sh, magic.json,
//!   magic.local.json}` I/O.
//! - [`reponame`] – the `<repo>` stem the pack archive name and the plugin's
//!   session identity are both derived from.
//! - [`state_tree`] – the `.superset/.magic` constant and the one writer of its
//!   gitignore rule.
//! - [`release`] – the per-line, daily-cached GitHub release check.
//! - [`testutil`] – shared test fixtures, compiled only for tests.

pub mod git;
pub mod hashing;
pub mod release;
pub mod reponame;
pub mod state_tree;
pub mod style;
pub mod superset_files;
pub mod sync;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
