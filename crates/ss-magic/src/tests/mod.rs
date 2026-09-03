//! Crate-root test modules extracted from main.rs.
//!
//! The shared helpers (git fixtures, the run-in-a-child-process seam) live in
//! `ss_magic_core::testutil`, enabled through this crate's `[dev-dependencies]`
//! `testutil` feature; per-module test files reference them by that path.

mod sync;
mod reverse_sync_flow;
mod update_gate;
