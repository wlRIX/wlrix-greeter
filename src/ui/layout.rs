// SPDX-License-Identifier: GPL-3.0-or-later
//! Where everything in the dialog goes.
//!
//! Computed once from the dialog's size into a [`Geometry`] that both drawing and
//! hit-testing read, so a click always lands on what was painted -- there is no second
//! copy of the layout to fall out of step.
//!
//! The password row is given a place from the start, whether or not it is shown, so
//! revealing it never resizes the dialog.

use crate::theme::palette::metrics;
use crate::ui::canvas::Rect;
use crate::ui::icons::ICON_SIZE;

/// Padding between the dialog edge and its contents.
const MARGIN: i32 = 12;
/// Gap between stacked rows.
const GAP: i32 = 10;
/// Height of the text fields and buttons.
const FIELD_H: i32 = 28;
/// The scrollbar's width, from the palette metrics.
const SCROLLBAR_W: i32 = metrics::SCROLL_BAR_SIZE;
/// One icon plus its label, the grid's cell.
const CELL_W: i32 = ICON_SIZE + 24;
const CELL_H: i32 = ICON_SIZE + 22;

/// Every rectangle the dialog is made of.
pub struct Geometry {
    /// The whole dialog.
    pub dialog: Rect,
    /// The sunken panel holding the icon grid.
    pub list_panel: Rect,
    /// The area icons are drawn in, inside the panel and left of the scrollbar.
    pub grid_viewport: Rect,
    /// The scrollbar trough.
    pub scrollbar: Rect,
    pub scroll_up: Rect,
    pub scroll_down: Rect,
    /// The `Login name:` label.
    pub name_label: Rect,
    /// The username field.
    pub name_field: Rect,
    /// Where the full name sits, right of the field.
    pub full_name: Rect,
    /// The password field's place, drawn only when active.
    pub password_field: Rect,
    /// The `wlRIX` wordmark, bottom-left.
    pub wordmark: Rect,
    pub login_button: Rect,
    pub help_button: Rect,

    /// The modal error dialog and its parts, drawn only when there is an error.
    pub error_box: Rect,
    pub error_icon: Rect,
    pub error_text: Rect,
    pub error_ok: Rect,
}

/// A thing in the dialog a pointer can land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Widget {
    Icon(usize),
    Username,
    Password,
    ScrollUp,
    ScrollDown,
    Thumb,
    /// The trough above or below the thumb; a click there pages the list.
    Track,
    LoginButton,
    HelpButton,
    /// The OK button of the modal error dialog.
    ErrorOk,
    None,
}

impl Geometry {
    /// Lay the dialog out at `width` x `height`.
    pub fn compute(width: i32, height: i32) -> Self {
        let dialog = Rect::new(0, 0, width, height);
        let content = dialog.inset(MARGIN);

        // From the bottom up: the button row, the wordmark beside it, then the password
        // and username rows, and the list takes whatever is left at the top.
        let button_w = 70;
        let button_row_y = content.bottom() - FIELD_H;
        let help_button = Rect::new(content.right() - button_w, button_row_y, button_w, FIELD_H);
        let login_button = Rect::new(
            help_button.left() - GAP - button_w,
            button_row_y,
            button_w,
            FIELD_H,
        );
        let wordmark = Rect::new(content.left(), button_row_y, 90, FIELD_H);

        let password_row_y = button_row_y - GAP - FIELD_H;
        let username_row_y = password_row_y - GAP - FIELD_H;

        // The label sits left of the username field; the field runs to a little past
        // center, leaving room for the full name to its right.
        let label_w = 96;
        let name_label = Rect::new(content.left(), username_row_y, label_w, FIELD_H);
        let field_w = 260;
        let name_field = Rect::new(name_label.right(), username_row_y, field_w, FIELD_H);
        let full_name = Rect::new(
            name_field.right() + GAP,
            username_row_y,
            content.right() - name_field.right() - GAP,
            FIELD_H,
        );
        let password_field = Rect::new(name_field.left(), password_row_y, field_w, FIELD_H);

        // The list panel fills from the top down to the username row.
        let list_panel = Rect::new(
            content.left(),
            content.top(),
            content.w,
            username_row_y - GAP - content.top(),
        );
        let inner = list_panel.inset(metrics::SHADOW_SCROLLED_WINDOW);
        let scrollbar = Rect::new(
            inner.right() - SCROLLBAR_W,
            inner.top(),
            SCROLLBAR_W,
            inner.h,
        );
        let scroll_up = Rect::new(scrollbar.left(), scrollbar.top(), SCROLLBAR_W, SCROLLBAR_W);
        let scroll_down = Rect::new(
            scrollbar.left(),
            scrollbar.bottom() - SCROLLBAR_W,
            SCROLLBAR_W,
            SCROLLBAR_W,
        );
        let grid_viewport = Rect::new(inner.left(), inner.top(), inner.w - SCROLLBAR_W, inner.h);

        // The error dialog: a small box centred over the list. Its size is fixed; only
        // its position depends on the dialog's, so drawing and hit-testing agree.
        let (ebw, ebh) = (340, 148);
        let error_box = Rect::new(
            grid_viewport.left() + (grid_viewport.w - ebw) / 2,
            grid_viewport.top() + (grid_viewport.h - ebh) / 2,
            ebw,
            ebh,
        );
        let epad = 16;
        let icon_sz = 44;
        let error_icon = Rect::new(
            error_box.left() + epad,
            error_box.top() + epad,
            icon_sz,
            icon_sz,
        );
        let error_text = Rect::new(
            error_icon.right() + epad,
            error_box.top() + epad,
            error_box.right() - epad - (error_icon.right() + epad),
            icon_sz,
        );
        let (okw, okh) = (54, 28);
        let error_ok = Rect::new(
            error_box.left() + (ebw - okw) / 2,
            error_box.bottom() - epad - okh,
            okw,
            okh,
        );

        Self {
            dialog,
            list_panel,
            grid_viewport,
            scrollbar,
            scroll_up,
            scroll_down,
            name_label,
            name_field,
            full_name,
            password_field,
            wordmark,
            login_button,
            help_button,
            error_box,
            error_icon,
            error_text,
            error_ok,
        }
    }

    /// How many icon columns fit across the grid.
    pub fn columns(&self) -> i32 {
        (self.grid_viewport.w / CELL_W).max(1)
    }

    /// The cell rectangle for icon `index`, before scrolling is applied.
    ///
    /// The same arithmetic hit-testing runs in reverse, so a click resolves to the icon
    /// that was drawn there.
    pub fn cell(&self, index: usize, scroll_y: i32) -> Rect {
        let cols = self.columns();
        let row = index as i32 / cols;
        let col = index as i32 % cols;
        Rect::new(
            self.grid_viewport.left() + col * CELL_W,
            self.grid_viewport.top() + row * CELL_H - scroll_y,
            CELL_W,
            CELL_H,
        )
    }

    /// The icon under a point, if any -- the reverse of [`cell`].
    pub fn icon_at(&self, x: i32, y: i32, count: usize, scroll_y: i32) -> Option<usize> {
        if !self.grid_viewport.contains(x, y) {
            return None;
        }
        let cols = self.columns();
        let col = (x - self.grid_viewport.left()) / CELL_W;
        let row = (y - self.grid_viewport.top() + scroll_y) / CELL_H;
        if col < 0 || col >= cols || row < 0 {
            return None;
        }
        let index = (row * cols + col) as usize;
        (index < count).then_some(index)
    }

    /// The total height the grid's rows occupy, for sizing the scrollbar thumb.
    pub fn content_height(&self, count: usize) -> i32 {
        let cols = self.columns();
        let rows = (count as i32 + cols - 1) / cols;
        rows * CELL_H
    }

    /// The furthest the grid can scroll, so a scroll offset can be clamped.
    pub fn max_scroll(&self, count: usize) -> i32 {
        (self.content_height(count) - self.grid_viewport.h).max(0)
    }

    /// The trough the thumb slides in, between the two arrows.
    pub fn scroll_track(&self) -> Rect {
        Rect::new(
            self.scrollbar.left(),
            self.scroll_up.bottom(),
            self.scrollbar.w,
            (self.scroll_down.top() - self.scroll_up.bottom()).max(0),
        )
    }

    /// The thumb's rectangle for a given content count and scroll position. Paint and
    /// hit-testing both call this, so the thumb drawn is the thumb clicked.
    pub fn thumb(&self, count: usize, scroll_y: i32) -> Rect {
        let track = self.scroll_track();
        let content = self.content_height(count).max(self.grid_viewport.h);
        let thumb_h = ((track.h * self.grid_viewport.h) / content).clamp(16, track.h.max(16));
        let max_scroll = self.max_scroll(count).max(1);
        let y = track.top() + (track.h - thumb_h) * scroll_y / max_scroll;
        Rect::new(track.left(), y, track.w, thumb_h)
    }

    /// What a point lands on. `password` is whether that field is currently shown, and
    /// `error` whether the modal error dialog is up -- if so, only its OK button responds.
    pub fn hit(
        &self,
        x: i32,
        y: i32,
        count: usize,
        scroll_y: i32,
        password: bool,
        error: bool,
    ) -> Widget {
        if error {
            return if self.error_ok.contains(x, y) {
                Widget::ErrorOk
            } else {
                Widget::None
            };
        }
        if let Some(index) = self.icon_at(x, y, count, scroll_y) {
            return Widget::Icon(index);
        }
        if self.login_button.contains(x, y) {
            return Widget::LoginButton;
        }
        if self.help_button.contains(x, y) {
            return Widget::HelpButton;
        }
        if self.name_field.contains(x, y) {
            return Widget::Username;
        }
        if password && self.password_field.contains(x, y) {
            return Widget::Password;
        }
        if self.scroll_up.contains(x, y) {
            return Widget::ScrollUp;
        }
        if self.scroll_down.contains(x, y) {
            return Widget::ScrollDown;
        }
        if self.thumb(count, scroll_y).contains(x, y) {
            return Widget::Thumb;
        }
        if self.scroll_track().contains(x, y) {
            return Widget::Track;
        }
        Widget::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_resolves_to_the_icon_drawn_there() {
        let g = Geometry::compute(560, 470);
        // Draw cell 3, then hit-test its center; it must come back as 3.
        let scroll = 0;
        let cell = g.cell(3, scroll);
        let hit = g.icon_at(cell.x + cell.w / 2, cell.y + cell.h / 2, 10, scroll);
        assert_eq!(hit, Some(3));
    }

    #[test]
    fn hit_testing_follows_the_scroll() {
        let g = Geometry::compute(560, 470);
        // A cell scrolled up out of the viewport is no longer where it was.
        let cols = g.columns();
        let index = (cols * 3) as usize; // three rows down
        let unscrolled = g.cell(index, 0);
        // Scroll so that row is at the viewport top.
        let scroll = unscrolled.y - g.grid_viewport.top();
        let hit = g.icon_at(
            g.grid_viewport.left() + 5,
            g.grid_viewport.top() + 5,
            (index + 1).max(40),
            scroll,
        );
        assert_eq!(hit, Some(index));
    }

    #[test]
    fn clicks_outside_the_grid_hit_nothing() {
        let g = Geometry::compute(560, 470);
        assert_eq!(g.icon_at(g.name_field.x, g.name_field.y, 10, 0), None);
    }

    #[test]
    fn the_password_row_exists_before_it_is_shown() {
        // Reserved from the start, so revealing it does not move anything.
        let g = Geometry::compute(560, 470);
        assert!(g.password_field.h > 0);
        assert!(g.password_field.top() > g.name_field.top());
        assert!(g.password_field.bottom() <= g.login_button.top());
    }
}
