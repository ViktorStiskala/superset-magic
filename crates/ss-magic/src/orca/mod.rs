//! Orca compatibility: generate Orca's `orca.yaml` and `.worktreeinclude` from
//! the `magic.json` a repository already maintains, and let forward sync leave
//! alone what Orca copied itself.
//!
//! Everything Orca-specific lives in this module of the CLI crate (KTD1). Core
//! gains only generic plumbing – a path filter on the copy engine, a bulk
//! ignored-path probe, a public atomic writer and the `sharedDirectories` field
//! – because the plugin binary links core and must never gain a YAML parser.

pub(crate) mod detect;
pub(crate) mod orca_yaml;
pub(crate) mod shared_dirs;
pub(crate) mod snapshot;
pub(crate) mod worktreeinclude;
pub(crate) mod write;
