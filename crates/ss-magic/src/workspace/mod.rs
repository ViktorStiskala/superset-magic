//! The `.superset` workspace contract: config/magic file I/O and the
//! init/migration lifecycle that materializes the layout.

pub(crate) mod migrate;

// The contract I/O is core's (the plugin reads the same files); re-exported
// so `crate::workspace::superset_files::…` keeps resolving.
pub(crate) use ss_magic_core::superset_files;
