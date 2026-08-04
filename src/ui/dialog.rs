// SPDX-License-Identifier: GPL-3.0-or-later
//! Painting the login dialog.
//!
//! One function, [`paint`], draws the whole thing from a [`Model`] and a [`Geometry`].
//! It keeps no state of its own: the layout is re-derived each frame and drawing is a
//! pure function of the model, which is what lets the same code serve the live greeter
//! and the `paint_preview` example.
//!
//! Interaction is a later stage. This draws whatever the model says, including a
//! selection and a shown password field, so the preview can exercise those states.

use crate::theme::Rgb;
use crate::theme::font::{Face, Fonts, Run};
use crate::theme::palette::{self, metrics};
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::icons;
use crate::ui::layout::Geometry;
use crate::ui::motif::{self, Bevel};
use crate::users::User;

/// Which field has the keyboard, and where the caret is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    None,
    Username,
    Password,
}

/// A pushable button, tracked while held so it can be drawn pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Login,
    Help,
    /// The OK button of the error dialog.
    ErrorOk,
}

/// Everything drawing needs to know. Interaction fills this in; the preview hard-codes
/// it.
pub struct Model {
    pub users: Vec<User>,
    /// The highlighted user, if one is chosen.
    pub selected: Option<usize>,
    /// The username field's contents.
    pub username: String,
    /// The full name shown beside the field.
    pub full_name: Option<String>,
    /// The answer field's contents, `Some` once PAM has asked a question.
    pub password: Option<String>,
    /// Whether that answer is hidden as it is typed. A password is; a one-time code,
    /// which PAM asks as a visible question, is not.
    pub password_secret: bool,
    /// How far the grid is scrolled, in pixels.
    pub scroll_y: i32,
    /// A message shown under the field, e.g. after a wrong password.
    pub message: Option<String>,
    /// Which field has the keyboard.
    pub focus: Focus,
    /// Whether the caret is in its visible half of the blink.
    pub caret_on: bool,
    /// A button currently held down, drawn pressed.
    pub armed: Option<Button>,
    /// A modal error, shown as a dialog over everything until dismissed.
    pub error: Option<String>,
}

impl Model {
    /// A blank model at the initial screen.
    pub fn new(users: Vec<User>) -> Self {
        Self {
            users,
            selected: None,
            username: String::new(),
            full_name: None,
            password: None,
            password_secret: true,
            scroll_y: 0,
            message: None,
            focus: Focus::None,
            caret_on: false,
            armed: None,
            error: None,
        }
    }
}

/// Text sizes, in pixels.
const TEXT_PX: f32 = 15.0;
const LABEL_PX: f32 = 15.0;
const NAME_PX: f32 = 13.0;
const WORDMARK_PX: f32 = 22.0;

/// Draw the dialog into `canvas`, laid out to its size.
pub fn paint(canvas: &mut Canvas, fonts: &mut Fonts, model: &Model) {
    let g = Geometry::compute(canvas.width(), canvas.height());

    // The dialog itself: a raised panel with the black keyline around it.
    motif::panel(
        canvas,
        g.dialog,
        palette::PANEL,
        Bevel::raised(
            palette::PANEL_TOP_SHADOW,
            palette::PANEL_BOTTOM_SHADOW,
            metrics::SHADOW_DEFAULT,
        ),
    );
    motif::outline(canvas, g.dialog, palette::OUTER_LINE);

    paint_list(canvas, fonts, &g, model);
    paint_scrollbar(canvas, &g, model);
    paint_fields(canvas, fonts, &g, model);
    paint_footer(canvas, fonts, &g, model);

    // The error dialog goes on top of everything, modal.
    if let Some(message) = &model.error {
        paint_error(
            canvas,
            fonts,
            &g,
            message,
            model.armed == Some(Button::ErrorOk),
        );
    }
}

/// The modal "cannot log in" box: a raised panel with a red prohibition icon, the
/// message, and an OK button.
fn paint_error(
    canvas: &mut Canvas,
    fonts: &mut Fonts,
    g: &Geometry,
    message: &str,
    ok_pressed: bool,
) {
    motif::panel(
        canvas,
        g.error_box,
        palette::PANEL,
        Bevel::raised(
            palette::PANEL_TOP_SHADOW,
            palette::PANEL_BOTTOM_SHADOW,
            metrics::SHADOW_DEFAULT,
        ),
    );
    motif::outline(canvas, g.error_box, palette::OUTER_LINE);

    prohibition_icon(canvas, g.error_icon);

    // The message in a sunken read-only field, one line per `\n`.
    motif::panel(
        canvas,
        g.error_text,
        palette::PANEL,
        Bevel::sunken(
            palette::PANEL_TOP_SHADOW,
            palette::PANEL_BOTTOM_SHADOW,
            metrics::SHADOW_DEFAULT,
        ),
    );
    let line_h = fonts.line_height(Face::Regular, NAME_PX);
    let mut baseline = g.error_text.top() + fonts.ascent(Face::Regular, NAME_PX) + 4;
    for line in message.lines() {
        fonts.draw(
            canvas,
            Run {
                face: Face::Regular,
                px: NAME_PX,
                x: g.error_text.left() + metrics::TEXT_MARGIN_WIDTH,
                baseline,
                color: palette::FOREGROUND,
            },
            line,
        );
        baseline += line_h;
    }

    button(canvas, fonts, g.error_ok, "OK", ok_pressed);
}

/// A red square with a white prohibition symbol -- the circle and bar of "no entry".
fn prohibition_icon(canvas: &mut Canvas, rect: Rect) {
    const RED: Rgb = Rgb(0xff_e00000);
    const WHITE: Rgb = Rgb(0xff_ffffff);
    canvas.fill_rect(rect, RED);
    canvas.stroke_rect(rect, palette::OUTER_LINE);

    let cx = rect.x + rect.w / 2;
    let cy = rect.y + rect.h / 2;
    let r = rect.w / 2 - 8;

    // A thick white ring: the band between two radii.
    let outer = r * r;
    let inner = (r - 4).max(0) * (r - 4).max(0);
    for dy in -r..=r {
        for dx in -r..=r {
            let d = dx * dx + dy * dy;
            if d <= outer && d >= inner {
                canvas.put(cx + dx, cy + dy, WHITE);
            }
        }
    }
    // The diagonal bar across it (top-left to bottom-right), a few pixels thick.
    for t in -r..=r {
        for w in -1..=1 {
            canvas.put(cx + t, cy + t + w, WHITE);
        }
    }
}

/// The sunken panel and the icon grid inside it.
fn paint_list(canvas: &mut Canvas, fonts: &mut Fonts, g: &Geometry, model: &Model) {
    motif::panel(
        canvas,
        g.list_panel,
        palette::LIST_BACKGROUND,
        Bevel::sunken(
            palette::PANEL_TOP_SHADOW,
            palette::PANEL_BOTTOM_SHADOW,
            metrics::SHADOW_SCROLLED_WINDOW,
        ),
    );

    for (index, user) in model.users.iter().enumerate() {
        let cell = g.cell(index, model.scroll_y);
        // Skip rows scrolled entirely out of view.
        if cell.bottom() <= g.grid_viewport.top() || cell.top() >= g.grid_viewport.bottom() {
            continue;
        }

        let icon_rect = Rect::new(
            cell.x + (cell.w - icons::ICON_SIZE) / 2,
            cell.y + 4,
            icons::ICON_SIZE,
            icons::ICON_SIZE,
        );
        // The yellow selection ring goes behind the icon.
        if model.selected == Some(index) {
            let ring = icon_rect.inset(-3);
            canvas.fill_rect(ring, palette::SELECT_FILL);
            canvas.stroke_rect(ring, palette::OUTER_LINE);
        }
        icons::draw(canvas, icon_rect, &icons::for_user(user));

        // The login name centered under the icon.
        let label_w = fonts.width(Face::Regular, TEXT_PX, &user.name);
        let label_x = cell.x + (cell.w - label_w) / 2;
        let baseline = icon_rect.bottom() + fonts.ascent(Face::Regular, TEXT_PX);
        fonts.draw(
            canvas,
            Run {
                face: Face::Regular,
                px: TEXT_PX,
                x: label_x,
                baseline,
                color: palette::FOREGROUND,
            },
            &user.name,
        );
    }
}

/// The scrollbar: trough, arrows, and a thumb sized to the visible fraction.
fn paint_scrollbar(canvas: &mut Canvas, g: &Geometry, model: &Model) {
    motif::panel(
        canvas,
        g.scrollbar,
        palette::TROUGH,
        Bevel::sunken(
            palette::TROUGH_TOP_SHADOW,
            palette::TROUGH_BOTTOM_SHADOW,
            metrics::SHADOW_SCROLL_BAR,
        ),
    );

    let control = Bevel::raised(
        palette::FACE_TOP_SHADOW,
        palette::FACE_BOTTOM_SHADOW,
        metrics::SHADOW_SCROLL_BAR,
    );
    motif::panel(canvas, g.scroll_up, palette::SCROLL_BAR_CONTROL, control);
    arrow(canvas, g.scroll_up, true);
    motif::panel(canvas, g.scroll_down, palette::SCROLL_BAR_CONTROL, control);
    arrow(canvas, g.scroll_down, false);

    // The thumb, sized and placed by the shared geometry so it matches hit-testing.
    if g.scroll_track().h > 0 {
        let thumb = g.thumb(model.users.len(), model.scroll_y);
        motif::panel(canvas, thumb, palette::SCROLL_BAR_CONTROL, control);
    }
}

/// A small centered triangle in a scroll button, pointing up or down.
fn arrow(canvas: &mut Canvas, button: Rect, up: bool) {
    let cx = button.x + button.w / 2;
    let cy = button.y + button.h / 2;
    let size: i32 = 4;
    // The apex is one pixel wide and the base `2*size+1`; `up` flips which end is which.
    for step in 0..=size {
        let half = if up { step } else { size - step };
        let row_y = cy - size / 2 + step;
        for dx in -half..=half {
            canvas.put(cx + dx, row_y, palette::FOREGROUND);
        }
    }
}

/// The `Login name:` label, the username field, the full name, and the password row.
fn paint_fields(canvas: &mut Canvas, fonts: &mut Fonts, g: &Geometry, model: &Model) {
    // The bold label, vertically centered on its row.
    let label_baseline =
        g.name_label.top() + (g.name_label.h + fonts.ascent(Face::Bold, LABEL_PX)) / 2 - 2;
    fonts.draw(
        canvas,
        Run {
            face: Face::Bold,
            px: LABEL_PX,
            x: g.name_label.left(),
            baseline: label_baseline,
            color: palette::FOREGROUND,
        },
        "Login name:",
    );

    text_field(
        canvas,
        fonts,
        g.name_field,
        &model.username,
        model.focus == Focus::Username,
        model.caret_on,
    );

    if let Some(full) = &model.full_name {
        let baseline =
            g.full_name.top() + (g.full_name.h + fonts.ascent(Face::Italic, NAME_PX)) / 2 - 2;
        fonts.draw(
            canvas,
            Run {
                face: Face::Italic,
                px: NAME_PX,
                x: g.full_name.left(),
                baseline,
                color: palette::FOREGROUND,
            },
            full,
        );
    }

    // The password field appears only once PAM has asked; its place was reserved.
    if let Some(password) = &model.password {
        let shown = if model.password_secret {
            "*".repeat(password.chars().count())
        } else {
            password.clone()
        };
        text_field(
            canvas,
            fonts,
            g.password_field,
            &shown,
            model.focus == Focus::Password,
            model.caret_on,
        );
    }

    if let Some(message) = &model.message {
        let baseline = g.password_field.bottom() + fonts.ascent(Face::Regular, NAME_PX);
        fonts.draw(
            canvas,
            Run {
                face: Face::Regular,
                px: NAME_PX,
                x: g.name_field.left(),
                baseline: baseline.min(g.wordmark.top()),
                color: palette::OUTER_LINE,
            },
            message,
        );
    }
}

/// A sunken pink text field with its contents, a focus ring, and a caret.
///
/// The caret sits at the end of the text: a login field is only ever appended to or
/// backspaced, so there is no mid-string cursor to place.
fn text_field(
    canvas: &mut Canvas,
    fonts: &mut Fonts,
    rect: Rect,
    text: &str,
    focused: bool,
    caret_on: bool,
) {
    motif::panel(
        canvas,
        rect,
        palette::TEXT_FIELD,
        Bevel::sunken(
            palette::TEXT_TOP_SHADOW,
            palette::TEXT_BOTTOM_SHADOW,
            metrics::SHADOW_TEXT,
        ),
    );
    // A focused field takes a thin keyline just inside the bevel.
    if focused {
        canvas.stroke_rect(rect.inset(metrics::SHADOW_TEXT), palette::OUTER_LINE);
    }

    let x = rect.left() + metrics::TEXT_MARGIN_WIDTH + metrics::SHADOW_TEXT;
    let baseline = rect.top() + (rect.h + fonts.ascent(Face::Regular, 15.0)) / 2 - 2;
    if !text.is_empty() {
        fonts.draw(
            canvas,
            Run {
                face: Face::Regular,
                px: 15.0,
                x,
                baseline,
                color: palette::FOREGROUND,
            },
            text,
        );
    }

    // The caret, in the visible half of its blink, just past the last glyph.
    if focused && caret_on {
        let caret_x = x + fonts.width(Face::Regular, 15.0, text);
        let top = rect.top() + metrics::SHADOW_TEXT + 3;
        let height = rect.h - 2 * metrics::SHADOW_TEXT - 6;
        canvas.fill_rect(
            Rect::new(caret_x, top, 1, height.max(1)),
            palette::FOREGROUND,
        );
    }
}

/// The `wlRIX` wordmark and the two buttons.
fn paint_footer(canvas: &mut Canvas, fonts: &mut Fonts, g: &Geometry, model: &Model) {
    // The gray italic wordmark, baseline-aligned with the buttons.
    let baseline = g.wordmark.top() + (g.wordmark.h + fonts.ascent(Face::Italic, WORDMARK_PX)) / 2;
    fonts.draw(
        canvas,
        Run {
            face: Face::Italic,
            px: WORDMARK_PX,
            x: g.wordmark.left(),
            baseline,
            color: palette::PANEL_BOTTOM_SHADOW,
        },
        "wlRIX",
    );

    button(
        canvas,
        fonts,
        g.login_button,
        "Log In",
        model.armed == Some(Button::Login),
    );
    button(
        canvas,
        fonts,
        g.help_button,
        "Help",
        model.armed == Some(Button::Help),
    );
}

/// A button with a centered label, raised or -- while held -- pressed in.
///
/// A pressed button swaps to a sunken bevel and the darker `armed` face, and its label
/// shifts a pixel down and right, so it reads as physically pushed.
fn button(canvas: &mut Canvas, fonts: &mut Fonts, rect: Rect, label: &str, pressed: bool) {
    let (fill, bevel) = if pressed {
        (
            palette::ARMED,
            Bevel::sunken(
                palette::FACE_TOP_SHADOW,
                palette::FACE_BOTTOM_SHADOW,
                metrics::SHADOW_DEFAULT,
            ),
        )
    } else {
        (
            palette::FACE,
            Bevel::raised(
                palette::FACE_TOP_SHADOW,
                palette::FACE_BOTTOM_SHADOW,
                metrics::SHADOW_DEFAULT,
            ),
        )
    };
    motif::panel(canvas, rect, fill, bevel);

    let nudge = if pressed { 1 } else { 0 };
    let text_w = fonts.width(Face::Regular, 14.0, label);
    let x = rect.left() + (rect.w - text_w) / 2 + nudge;
    let baseline = rect.top() + (rect.h + fonts.ascent(Face::Regular, 14.0)) / 2 - 2 + nudge;
    fonts.draw(
        canvas,
        Run {
            face: Face::Regular,
            px: 14.0,
            x,
            baseline,
            color: palette::FOREGROUND,
        },
        label,
    );
}
