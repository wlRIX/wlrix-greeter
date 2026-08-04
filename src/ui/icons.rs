// SPDX-License-Identifier: GPL-3.0-or-later
//! The user icons in the list.
//!
//! IRIX drew a distinct icon per user -- a keyring for root, a workstation for everyone
//! else. There is no artwork in the tree yet, so these are drawn in code: a beveled tile
//! with a simple emblem, enough to tell root apart from an ordinary user and to give the
//! grid its shape. Real icons drop in at [`for_user`] without the rest of the UI
//! noticing.

use crate::theme::{Rgb, palette};
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::motif::{self, Bevel};
use crate::users::User;

/// The drawn size of a user icon, in logical pixels. Roughly IRIX's.
pub const ICON_SIZE: i32 = 64;

/// Which placeholder to draw for a user.
///
/// A real loader would return an owned bitmap here instead; the call sites only ask for
/// "the icon for this user", so swapping the source changes nothing above.
pub enum Icon {
    /// The super-user: a keyring.
    Root,
    /// Everyone else: a workstation.
    Workstation,
}

/// The icon a user should show.
pub fn for_user(user: &User) -> Icon {
    if user.uid == 0 {
        Icon::Root
    } else {
        Icon::Workstation
    }
}

/// Draw `icon` filling `rect` (expected to be [`ICON_SIZE`] square).
pub fn draw(canvas: &mut Canvas, rect: Rect, icon: &Icon) {
    // A raised tile, like a small button, so the emblem sits on something.
    motif::panel(
        canvas,
        rect,
        palette::FACE,
        Bevel::raised(palette::FACE_TOP_SHADOW, palette::FACE_BOTTOM_SHADOW, 2),
    );
    let inner = rect.inset(2);
    match icon {
        Icon::Root => draw_keyring(canvas, inner),
        Icon::Workstation => draw_workstation(canvas, inner),
    }
}

/// A ring with a couple of keys, in the bright IRIX palette colors.
fn draw_keyring(canvas: &mut Canvas, r: Rect) {
    // Colors picked to read as keys rather than to match any one real icon.
    const RING: Rgb = Rgb(0xff_c0c0c0);
    const KEY_A: Rgb = Rgb(0xff_e04040);
    const KEY_B: Rgb = Rgb(0xff_40a0e0);
    const KEY_C: Rgb = Rgb(0xff_e0c040);

    let cx = r.x + r.w / 2;
    let cy = r.y + r.h / 3;
    ring(canvas, cx, cy, r.w / 6, RING);

    // Three key blades fanning down from the ring.
    let blade_top = cy + r.h / 8;
    let blade_h = r.h / 2;
    for (i, color) in [KEY_A, KEY_B, KEY_C].into_iter().enumerate() {
        let x = cx - r.w / 8 + (i as i32) * (r.w / 8);
        canvas.fill_rect(Rect::new(x, blade_top, r.w / 12, blade_h), color);
    }
}

/// A little monitor on a stand, the generic-user look.
fn draw_workstation(canvas: &mut Canvas, r: Rect) {
    const CASE: Rgb = Rgb(0xff_d0d0d0);
    const SCREEN: Rgb = Rgb(0xff_384858);
    const OUTLINE: Rgb = palette::OUTER_LINE;

    let mon = Rect::new(r.x + r.w / 6, r.y + r.h / 6, r.w * 2 / 3, r.h / 2);
    canvas.fill_rect(mon, CASE);
    canvas.stroke_rect(mon, OUTLINE);
    canvas.fill_rect(mon.inset(4), SCREEN);

    // A neck and base under it.
    let neck = Rect::new(r.x + r.w / 2 - 2, mon.bottom(), 4, r.h / 8);
    canvas.fill_rect(neck, CASE);
    let base = Rect::new(r.x + r.w / 3, neck.bottom(), r.w / 3, 4);
    canvas.fill_rect(base, CASE);
    canvas.stroke_rect(base, OUTLINE);
}

/// A rough filled ring, centered on `(cx, cy)` with the given outer `radius`.
fn ring(canvas: &mut Canvas, cx: i32, cy: i32, radius: i32, color: Rgb) {
    let outer = radius * radius;
    let inner = ((radius - 2).max(0)) * ((radius - 2).max(0));
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let d = dx * dx + dy * dy;
            if d <= outer && d >= inner {
                canvas.put(cx + dx, cy + dy, color);
            }
        }
    }
}
