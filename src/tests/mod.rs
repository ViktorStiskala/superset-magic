//! Crate-root test modules extracted from main.rs, plus shared helpers.
//!
//! `support` is pub(crate): per-module test files (git, gitignore, pack,
//! reverse_sync) live outside `crate::tests` and reference it as
//! `crate::tests::support`.

pub(crate) mod support;

mod sync;
mod reverse_sync_flow;
mod update_gate;
