//! The sync engine's interactive half – reverse sync and the merge model –
//! beside a re-export of the pure half from `ss-magic-core`, so a caller here
//! still spells `crate::sync::apply` and `crate::sync::under_excluded_tree`.
//!
//! `reverse_sync` and `merge` stay in this crate because they drive the
//! `tui::cockpit`; everything else (`apply`, `pattern`, `repo_scan`, the
//! `EXCLUDED_TREES` rule) is core's, shared with the plugin binary.

pub(crate) mod merge;
pub(crate) mod reverse_sync;

pub(crate) use ss_magic_core::sync::{apply, pattern, repo_scan, under_excluded_tree};
