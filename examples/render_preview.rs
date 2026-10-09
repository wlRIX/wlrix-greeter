// SPDX-License-Identifier: GPL-3.0-or-later
//! Writes one dialog frame to a PNM, with no compositor and no login.
//!
//! `paint_preview` puts the dialog on screen, which needs a Wayland connection and a pair of
//! eyes. This writes the same frame to a file, so a change to the drawing can be diffed
//! against the frame before it -- which is how the move to `wlrix-ui` was shown to have
//! changed no pixel.
//!
//! ```sh
//! cargo run --example render_preview -- out.pnm            # classic
//! cargo run --example render_preview -- out.pnm gotham     # a scheme
//! cargo run --example render_preview -- out.pnm gotham password
//! ```
//! Not part of the greeter; a dev tool only.

use wlrix_greeter::preview_api::{Focus, Model, User, render};

const W: i32 = 560;
const H: i32 = 470;

fn user(name: &str, full: Option<&str>, uid: u32) -> User {
    User {
        name: name.to_string(),
        full_name: full.map(str::to_string),
        uid,
        home: None,
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dialog.pnm".into());
    let scheme = std::env::args().nth(2);
    let state = std::env::args().nth(3);

    let mut model = Model::new(vec![
        user("root", Some("Super-User"), 0),
        user("vic485", Some("Victor Fugazzotto"), 1000),
        user("ada", Some("Ada Lovelace"), 1001),
        user("guest", None, 1002),
        user("demos", Some("Demo Account"), 1003),
        user("oracle", Some("Oracle"), 1004),
    ]);
    // A selected user with the password row revealed: the state that exercises the most
    // surfaces at once -- the list, the scrollbar, both fields, the wordmark and the buttons.
    // A second user under the pointer, so all three picture tints are in one frame.
    model.selected = Some(1);
    model.hovered = Some(2);
    model.username = "vic485".into();
    model.full_name = Some("Victor Fugazzotto".into());
    model.focus = Focus::Username;
    model.caret_on = true;
    if state.as_deref() == Some("password") {
        model.password = Some("hunter2".into());
        model.focus = Focus::Password;
    }

    let (palette, unknown) = wlrix_ui::palette::resolve(scheme.as_deref());
    if let Some(why) = unknown {
        eprintln!("{why}");
    }
    let pixels = render(&model, palette, W, H).expect("render");

    // The dialog is opaque, so no compositing is needed -- just Argb8888 to P6's RGB.
    let mut pnm = format!("P6\n{W} {H}\n255\n").into_bytes();
    for px in pixels.as_chunks::<4>().0 {
        pnm.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    std::fs::write(&out, pnm).expect("write");
    println!("wrote {out} ({})", palette.id);
}
