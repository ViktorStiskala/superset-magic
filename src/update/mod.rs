//! Self-update subsystem.
//!
//! Split into the cheap gate and the heavy apply (per the plan's Phase C):
//! - [`check`] (this unit, U6) answers "is a newer release available?"
//!   cheaply and offline-safely — a 24h-cached check that never errors,
//!   never logs, and never blocks an offline or rate-limited run.
//! - The lock/download/verify/swap/re-exec apply path is U7 (`apply.rs`),
//!   gated on a `Newer` verdict from here.
//! - Wiring the gate into every entrypoint is U8 (`main.rs`).
//!
//! Public items here are consumed by U7/U8; they're re-exported from `check`
//! so the rest of the crate refers to `update::*` rather than reaching into
//! the submodule.

pub mod check;

// Re-exports for U7/U8 consumers. Allowed-unused until those units wire them
// into the startup path and the forced-update command.
#[allow(unused_imports)] // consumed by U7/U8
pub use check::{check, UpdateCheck};
