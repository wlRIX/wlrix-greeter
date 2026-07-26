// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX greeter, as a library.
//!
//! The binary (`main.rs`) is a thin shell over this. Splitting it out lets
//! `examples/paint_preview` reach the drawing code to show the dialog without a login.

pub mod greetd;
pub mod logging;
pub mod prompt;
pub mod theme;
pub mod ui;
pub mod users;
pub mod worker;

/// The bits `examples/paint_preview` needs, gathered so the example depends on a small,
/// named surface rather than the greeter's innards.
pub mod preview_api {
    pub use crate::ui::dialog::{Button, Focus, Model};
    pub use crate::ui::preview::show;
    pub use crate::users::User;
}
