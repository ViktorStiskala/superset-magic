//! Interactive layer: inquire wrappers, the operation menu, the prompt theme,
//! and the merge cockpit. The palette itself (`style`) is core's, re-exported
//! here so `crate::tui::style::…` keeps resolving.

pub(crate) mod cockpit;
pub(crate) mod diffmodel;
pub(crate) mod menu;
pub(crate) mod theme;
pub(crate) mod ui;

pub(crate) use ss_magic_core::style;
