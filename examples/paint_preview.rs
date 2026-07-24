// SPDX-License-Identifier: GPL-3.0-or-later
//! Draws the login dialog from a fixed user list, with no greetd.
//!
//! The interesting part of the greeter is the drawing, and none of it needs a login to
//! look at. This puts the whole dialog on screen so it can be eyeballed and screenshotted
//! against the reference -- furniture, bevels, the pink field, the icons, the wordmark.
//!
//! With an argument it shows a particular state:
//!   (none)      the initial screen, nothing selected
//!   selected    a user chosen, their full name shown
//!   typed       a half-typed username with the caret showing
//!   password    the password field revealed
//!   denied      a wrong-password message (inline)
//!   error       the modal "cannot log in" dialog
//!   pressed     the Log In button held down
//!
//! Usage: `cargo run --example paint_preview [state]` under a Wayland compositor.
//! Not part of the greeter; a dev tool only.

use wlrix_greeter::preview_api::{Button, Focus, Model, User, show};

fn user(name: &str, full: Option<&str>, uid: u32) -> User {
    User {
        name: name.to_string(),
        full_name: full.map(str::to_string),
        uid,
    }
}

fn main() {
    // Enough users to fill more than one row, so the grid and scrollbar have something
    // to show.
    let users = vec![
        user("root", Some("Super-User"), 0),
        user("vic485", Some("Victor Fugazzotto"), 1000),
        user("ada", Some("Ada Lovelace"), 1001),
        user("guest", None, 1002),
        user("demos", Some("Demo Account"), 1003),
        user("oracle", Some("Oracle"), 1004),
        user("build", Some("Build Bot"), 1005),
        user("kiosk", None, 1006),
    ];

    let mut model = Model::new(users);
    // Show the caret, so its rendering is in the picture too.
    model.caret_on = true;

    match std::env::args().nth(1).as_deref() {
        Some("selected") => {
            model.selected = Some(1);
            model.username = "vic485".to_string();
            model.full_name = Some("Victor Fugazzotto".to_string());
            model.focus = Focus::Username;
        }
        Some("typed") => {
            model.username = "vic4".to_string();
            model.focus = Focus::Username;
        }
        Some("password") => {
            model.selected = Some(1);
            model.username = "vic485".to_string();
            model.full_name = Some("Victor Fugazzotto".to_string());
            model.password = Some("hunter2".to_string());
            model.focus = Focus::Password;
        }
        Some("denied") => {
            model.username = "vic485".to_string();
            model.message = Some("Login incorrect".to_string());
            model.focus = Focus::Username;
        }
        Some("error") => {
            model.selected = Some(1);
            model.username = "vic485".to_string();
            model.error = Some("Cannot log in to this account.".to_string());
        }
        Some("pressed") => {
            model.selected = Some(1);
            model.username = "vic485".to_string();
            model.armed = Some(Button::Login);
        }
        _ => {}
    }

    if let Err(err) = show(model, 560, 470) {
        eprintln!("paint_preview: {err}");
        std::process::exit(1);
    }
}
