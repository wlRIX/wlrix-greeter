// SPDX-License-Identifier: GPL-3.0-or-later
//! Turning clicks and keystrokes into changes to the dialog.
//!
//! The rules of the login screen live here, apart from the Wayland plumbing that
//! delivers the events and the timers that drive the caret and scroll repeat. Everything
//! reads and writes the [`Model`] through a [`Geometry`], the same layout drawing uses,
//! so a click always acts on what was painted.
//!
//! What "advance" does -- reveal a password field, start a session -- is not decided
//! here. These functions report [`Action::Advance`] and leave it to the caller, which is
//! what lets the same interaction serve the greeter and, until greetd is wired in, a
//! local stand-in.

use crate::ui::dialog::{Button, Focus, Model};
use crate::ui::layout::{Geometry, Widget};

/// How close together two clicks on one icon count as a double-click.
const DOUBLE_CLICK_MS: u32 = 400;
/// How far one arrow press or wheel notch scrolls.
const SCROLL_STEP: i32 = 40;

/// Interaction state that is not part of what gets drawn.
#[derive(Default)]
pub struct Input {
    /// The last pointer position, in dialog pixels.
    pub pointer: (i32, i32),
    /// The last icon click, for spotting a double-click: `(time_ms, index)`.
    last_click: Option<(u32, usize)>,
    /// While dragging the scrollbar thumb: the pointer's offset within it.
    thumb_grab: Option<i32>,
}

/// Something the event loop must act on, beyond the model change already made.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing more to do.
    None,
    /// A login should be attempted with the current username.
    Advance,
    /// A scroll arrow is held; begin repeating it. `true` is up.
    HoldArrow(bool),
}

/// A pointer button went down at `(x, y)` at time `time`.
pub fn press(
    model: &mut Model,
    input: &mut Input,
    g: &Geometry,
    x: i32,
    y: i32,
    time: u32,
) -> Action {
    input.pointer = (x, y);
    let count = model.users.len();
    match g.hit(
        x,
        y,
        count,
        model.scroll_y,
        model.password.is_some(),
        model.error.is_some(),
    ) {
        Widget::ErrorOk => {
            model.armed = Some(Button::ErrorOk);
            Action::None
        }
        Widget::Icon(index) => {
            let double = matches!(input.last_click, Some((t, i)) if i == index && time - t <= DOUBLE_CLICK_MS);
            select(model, index);
            input.last_click = Some((time, index));
            if double {
                Action::Advance
            } else {
                Action::None
            }
        }
        Widget::Username => {
            model.focus = Focus::Username;
            Action::None
        }
        Widget::Password => {
            model.focus = Focus::Password;
            Action::None
        }
        Widget::ScrollUp => {
            scroll_by(model, g, -SCROLL_STEP);
            Action::HoldArrow(true)
        }
        Widget::ScrollDown => {
            scroll_by(model, g, SCROLL_STEP);
            Action::HoldArrow(false)
        }
        Widget::Track => {
            // Page towards the click.
            let thumb = g.thumb(count, model.scroll_y);
            let dir = if y < thumb.top() { -1 } else { 1 };
            scroll_by(model, g, dir * g.scroll_track().h.max(SCROLL_STEP));
            Action::None
        }
        Widget::Thumb => {
            let thumb = g.thumb(count, model.scroll_y);
            input.thumb_grab = Some(y - thumb.top());
            Action::None
        }
        Widget::LoginButton => {
            model.armed = Some(Button::Login);
            Action::None
        }
        Widget::HelpButton => {
            model.armed = Some(Button::Help);
            Action::None
        }
        Widget::None => Action::None,
    }
}

/// The pointer button came back up.
///
/// A button fires now, not on press, and only if the pointer is still over the one that
/// was armed -- so a press begun and dragged off is canceled, the Motif way.
pub fn release(model: &mut Model, input: &mut Input, g: &Geometry, x: i32, y: i32) -> Action {
    input.thumb_grab = None;
    let armed = model.armed.take();
    match armed {
        Some(Button::Login) if g.login_button.contains(x, y) => advance_if_ready(model),
        Some(Button::ErrorOk) if g.error_ok.contains(x, y) => {
            dismiss_error(model);
            Action::None
        }
        // Help has no behavior yet; a released Login off the button, or any Help, does
        // nothing beyond un-arming.
        _ => Action::None,
    }
}

/// The pointer moved to `(x, y)`.
pub fn motion(model: &mut Model, input: &mut Input, g: &Geometry, x: i32, y: i32) {
    input.pointer = (x, y);
    hover(model, g, x, y);
    // Dragging the thumb maps the pointer's position in the track onto the scroll range.
    if let Some(grab) = input.thumb_grab {
        let track = g.scroll_track();
        let thumb = g.thumb(model.users.len(), model.scroll_y);
        let travel = (track.h - thumb.h).max(1);
        let offset = (y - grab - track.top()).clamp(0, travel);
        let max = g.max_scroll(model.users.len());
        model.scroll_y = offset * max / travel;
    }
}

/// The pointer left the dialog: nothing is under it any more.
pub fn leave(model: &mut Model) {
    model.hovered = None;
}

/// Which user is under the pointer at `(x, y)`.
///
/// Through the same hit-test a click uses, so the user drawn white is the one a click there
/// would choose -- and nothing is, while the error dialog covers the list.
fn hover(model: &mut Model, g: &Geometry, x: i32, y: i32) {
    model.hovered = match g.hit(
        x,
        y,
        model.users.len(),
        model.scroll_y,
        model.password.is_some(),
        model.error.is_some(),
    ) {
        Widget::Icon(index) => Some(index),
        _ => None,
    };
}

/// A scroll wheel notch: positive `delta` scrolls down.
///
/// The list moves under a pointer that stays put, so what it is over is worked out again.
pub fn wheel(model: &mut Model, input: &Input, g: &Geometry, delta: i32) {
    scroll_by(model, g, delta.signum() * SCROLL_STEP);
    hover(model, g, input.pointer.0, input.pointer.1);
}

/// A held scroll arrow repeated. Returns whether it can still scroll further, so the
/// caller can stop the repeat at the end of the list.
pub fn repeat_arrow(model: &mut Model, g: &Geometry, up: bool) -> bool {
    let before = model.scroll_y;
    scroll_by(model, g, if up { -SCROLL_STEP } else { SCROLL_STEP });
    model.scroll_y != before
}

/// A key producing `text` (its UTF-8, if any) with keysym name `enter`/`backspace`
/// flagged by the caller was pressed. Returns any resulting action.
pub fn key(model: &mut Model, key: Key) -> Action {
    // A modal error swallows the keyboard: Enter or Space dismisses it, nothing else
    // reaches the fields behind it.
    if model.error.is_some() {
        if matches!(key, Key::Enter | Key::Text(_)) {
            dismiss_error(model);
        }
        return Action::None;
    }
    match key {
        Key::Enter => advance_if_ready(model),
        Key::Backspace => {
            field_mut(model).map(String::pop);
            // Editing the username by hand drops the icon selection.
            if model.focus == Focus::Username {
                model.selected = None;
                model.full_name = None;
            }
            Action::None
        }
        Key::Text(text) => {
            // Ignore control characters; take only what a person typed.
            let printable: String = text.chars().filter(|c| !c.is_control()).collect();
            if printable.is_empty() {
                return Action::None;
            }
            if model.focus == Focus::None {
                // Typing with nothing focused starts editing the username.
                model.focus = Focus::Username;
            }
            if model.focus == Focus::Username {
                model.selected = None;
                model.full_name = None;
            }
            if let Some(field) = field_mut(model) {
                field.push_str(&printable);
            }
            Action::None
        }
        Key::Tab => {
            // Move between the fields when both are present.
            model.focus = match (model.focus, model.password.is_some()) {
                (Focus::Username, true) => Focus::Password,
                (Focus::Password, _) => Focus::Username,
                (other, _) => other,
            };
            Action::None
        }
        Key::Other => Action::None,
    }
}

/// The keys the dialog cares about; the Wayland layer maps keysyms onto these.
pub enum Key {
    Enter,
    Backspace,
    Tab,
    Text(String),
    Other,
}

/// Clear the error dialog and reset to the initial screen: fields empty, no user
/// selected. Matches IRIX, which starts over after a failed login.
fn dismiss_error(model: &mut Model) {
    model.error = None;
    model.username.clear();
    model.password = None;
    model.full_name = None;
    model.selected = None;
    model.focus = Focus::None;
    model.message = None;
}

/// Choose user `index`: fill the field from it and highlight it.
fn select(model: &mut Model, index: usize) {
    if let Some(user) = model.users.get(index) {
        model.selected = Some(index);
        model.username = user.name.clone();
        model.full_name = user.full_name.clone();
        model.focus = Focus::Username;
        model.message = None;
    }
}

/// Advance only when there is a username to advance with; the Log In button and Enter
/// do nothing on an empty field.
fn advance_if_ready(model: &Model) -> Action {
    if model.username.trim().is_empty() {
        Action::None
    } else {
        Action::Advance
    }
}

/// The field the keyboard is editing.
fn field_mut(model: &mut Model) -> Option<&mut String> {
    match model.focus {
        Focus::Username => Some(&mut model.username),
        Focus::Password => model.password.as_mut(),
        Focus::None => None,
    }
}

/// Scroll by `delta`, clamped to the list.
fn scroll_by(model: &mut Model, g: &Geometry, delta: i32) {
    let max = g.max_scroll(model.users.len());
    model.scroll_y = (model.scroll_y + delta).clamp(0, max);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::User;

    fn model() -> Model {
        let users = (0..12)
            .map(|i| User {
                name: format!("user{i}"),
                full_name: Some(format!("User {i}")),
                uid: 1000 + i,
                home: None,
            })
            .collect();
        Model::new(users)
    }

    fn geometry() -> Geometry {
        Geometry::compute(wlrix_ui::palette::DEFAULT, 560, 470)
    }

    #[test]
    fn clicking_an_icon_selects_and_fills_the_field() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        let cell = g.cell(2, 0);
        let action = press(
            &mut m,
            &mut input,
            &g,
            cell.x + cell.w / 2,
            cell.y + 10,
            100,
        );
        assert_eq!(action, Action::None);
        assert_eq!(m.selected, Some(2));
        assert_eq!(m.username, "user2");
        assert_eq!(m.full_name.as_deref(), Some("User 2"));
    }

    #[test]
    fn the_pointer_hovers_the_user_under_it_and_nothing_else() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        let cell = g.cell(3, 0);
        motion(&mut m, &mut input, &g, cell.x + cell.w / 2, cell.y + 10);
        assert_eq!(m.hovered, Some(3));
        // Hovering is not choosing.
        assert_eq!(m.selected, None);

        // Off the list, onto the field: nothing is hovered.
        let field = g.name_field;
        motion(&mut m, &mut input, &g, field.x + 4, field.y + 4);
        assert_eq!(m.hovered, None);

        motion(&mut m, &mut input, &g, cell.x + cell.w / 2, cell.y + 10);
        leave(&mut m);
        assert_eq!(m.hovered, None, "leaving the dialog clears it");
    }

    #[test]
    fn scrolling_moves_the_hover_to_whatever_is_now_under_the_pointer() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        let first = g.cell(0, 0);
        motion(&mut m, &mut input, &g, first.x + first.w / 2, first.y + 10);
        assert_eq!(m.hovered, Some(0));
        wheel(&mut m, &input, &g, 1);
        assert_ne!(m.scroll_y, 0, "the list is long enough to scroll");
        assert_eq!(
            m.hovered,
            g.icon_at(
                first.x + first.w / 2,
                first.y + 10,
                m.users.len(),
                m.scroll_y
            )
        );
    }

    #[test]
    fn nothing_is_hovered_under_the_error_dialog() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        m.error = Some("no".into());
        let cell = g.cell(0, 0);
        motion(&mut m, &mut input, &g, cell.x + cell.w / 2, cell.y + 10);
        assert_eq!(m.hovered, None);
    }

    #[test]
    fn a_second_click_on_the_same_icon_advances() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        let cell = g.cell(1, 0);
        let (cx, cy) = (cell.x + cell.w / 2, cell.y + 10);
        assert_eq!(press(&mut m, &mut input, &g, cx, cy, 100), Action::None);
        // Within the window: a double-click.
        assert_eq!(press(&mut m, &mut input, &g, cx, cy, 300), Action::Advance);
    }

    #[test]
    fn two_slow_clicks_are_not_a_double_click() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        let cell = g.cell(1, 0);
        let (cx, cy) = (cell.x + cell.w / 2, cell.y + 10);
        press(&mut m, &mut input, &g, cx, cy, 100);
        assert_eq!(
            press(&mut m, &mut input, &g, cx, cy, 100 + 500),
            Action::None
        );
    }

    #[test]
    fn typing_a_name_clears_the_selection() {
        let mut m = model();
        select(&mut m, 3);
        assert!(m.selected.is_some());
        key(&mut m, Key::Text("x".to_string()));
        assert_eq!(m.selected, None);
        assert_eq!(m.username, "user3x");
        assert_eq!(m.full_name, None);
    }

    #[test]
    fn enter_advances_only_with_a_username() {
        let mut m = model();
        m.focus = Focus::Username;
        assert_eq!(key(&mut m, Key::Enter), Action::None); // empty
        m.username = "root".to_string();
        assert_eq!(key(&mut m, Key::Enter), Action::Advance);
    }

    #[test]
    fn the_login_button_does_nothing_without_a_username() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        let b = g.login_button;
        let action = press(&mut m, &mut input, &g, b.x + 2, b.y + 2, 10);
        assert_eq!(action, Action::None);
    }

    #[test]
    fn backspace_edits_the_focused_field() {
        let mut m = model();
        m.focus = Focus::Username;
        m.username = "abc".to_string();
        key(&mut m, Key::Backspace);
        assert_eq!(m.username, "ab");
    }

    #[test]
    fn control_characters_are_not_typed() {
        let mut m = model();
        m.focus = Focus::Username;
        key(&mut m, Key::Text("\u{7f}".to_string()));
        assert_eq!(m.username, "");
    }

    #[test]
    fn password_is_edited_when_focused() {
        let mut m = model();
        m.password = Some(String::new());
        m.focus = Focus::Password;
        key(&mut m, Key::Text("pw".to_string()));
        assert_eq!(m.password.as_deref(), Some("pw"));
        // Editing the password must not disturb the username's selection.
        select(&mut m, 0);
        assert_eq!(m.selected, Some(0));
    }

    #[test]
    fn scrolling_is_clamped_to_the_list() {
        let (mut m, g) = (model(), geometry());
        scroll_by(&mut m, &g, -100);
        assert_eq!(m.scroll_y, 0, "cannot scroll above the top");
        scroll_by(&mut m, &g, 100_000);
        assert_eq!(m.scroll_y, g.max_scroll(m.users.len()));
    }

    #[test]
    fn the_login_button_arms_on_press_and_fires_on_release() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        m.username = "root".to_string();
        let b = g.login_button;
        // Press arms it but does not fire.
        assert_eq!(
            press(&mut m, &mut input, &g, b.x + 2, b.y + 2, 10),
            Action::None
        );
        assert_eq!(m.armed, Some(Button::Login));
        // Release over it fires.
        assert_eq!(
            release(&mut m, &mut input, &g, b.x + 2, b.y + 2),
            Action::Advance
        );
        assert_eq!(m.armed, None);
    }

    #[test]
    fn a_press_released_off_the_button_does_not_fire() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        m.username = "root".to_string();
        let b = g.login_button;
        press(&mut m, &mut input, &g, b.x + 2, b.y + 2, 10);
        // Released away from the button: canceled, and it un-arms.
        assert_eq!(release(&mut m, &mut input, &g, 0, 0), Action::None);
        assert_eq!(m.armed, None);
    }

    #[test]
    fn the_error_dialog_is_modal() {
        let (mut m, g, mut input) = (model(), geometry(), Input::default());
        m.error = Some("Cannot log in to this account.".to_string());
        m.username = "root".to_string();
        m.selected = Some(0);
        // A click on an icon behind the dialog does nothing.
        let cell = g.cell(0, 0);
        press(&mut m, &mut input, &g, cell.x + 5, cell.y + 5, 10);
        assert!(m.error.is_some(), "input behind a modal error is ignored");
        assert_eq!(m.selected, Some(0));

        // OK dismisses it and resets to the initial screen.
        let ok = g.error_ok;
        press(&mut m, &mut input, &g, ok.x + 2, ok.y + 2, 20);
        release(&mut m, &mut input, &g, ok.x + 2, ok.y + 2);
        assert!(m.error.is_none());
        assert_eq!(m.username, "");
        assert_eq!(m.selected, None);
        assert_eq!(m.focus, Focus::None);
    }

    #[test]
    fn enter_dismisses_the_error() {
        let mut m = model();
        m.error = Some("Cannot log in to this account.".to_string());
        m.username = "root".to_string();
        key(&mut m, Key::Enter);
        assert!(m.error.is_none());
        assert_eq!(m.username, "");
    }
}

#[cfg(test)]
mod flow {
    //! One realistic session, driven through the same calls the Wayland handlers make,
    //! so the whole interaction path is exercised end to end without a compositor.
    use super::*;
    use crate::ui::dialog::Focus;
    use crate::users::User;

    #[test]
    fn click_type_advance_password_advance() {
        let users = vec![
            User {
                name: "root".into(),
                full_name: Some("Super-User".into()),
                uid: 0,
                home: None,
            },
            User {
                name: "vic485".into(),
                full_name: Some("Victor".into()),
                uid: 1000,
                home: None,
            },
        ];
        let mut model = Model::new(users);
        let mut input = Input::default();
        let g = Geometry::compute(wlrix_ui::palette::DEFAULT, 560, 470);

        // Click vic485's icon: selected, field filled, focused.
        let cell = g.cell(1, 0);
        let (cx, cy) = (cell.x + cell.w / 2, cell.y + 8);
        assert_eq!(press(&mut model, &mut input, &g, cx, cy, 100), Action::None);
        assert_eq!(model.username, "vic485");
        assert_eq!(model.focus, Focus::Username);

        // Correct a typo by hand: selection drops, field edited.
        assert_eq!(key(&mut model, Key::Backspace), Action::None);
        assert_eq!(model.selected, None);
        assert_eq!(model.username, "vic48");
        assert_eq!(key(&mut model, Key::Text("5".into())), Action::None);
        assert_eq!(model.username, "vic485");

        // Enter advances.
        assert_eq!(key(&mut model, Key::Enter), Action::Advance);

        // The caller (stand-in or worker) reveals the password field; simulate that.
        model.password = Some(String::new());
        model.focus = Focus::Password;

        // Type a password: it edits the password field, not the username.
        key(&mut model, Key::Text("hunter2".into()));
        assert_eq!(model.password.as_deref(), Some("hunter2"));
        assert_eq!(model.username, "vic485");

        // Enter advances again.
        assert_eq!(key(&mut model, Key::Enter), Action::Advance);
    }
}
