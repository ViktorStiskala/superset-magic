//! The sync engine: glob expansion, forward copy, reverse sync, and the
//! shared pattern/working-tree helpers behind both directions.

pub(crate) mod apply;
pub(crate) mod pattern;
pub(crate) mod repo_scan;
pub(crate) mod reverse_sync;
