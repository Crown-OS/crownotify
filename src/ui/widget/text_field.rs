//! The inline-reply field.
//!
//! A single-line editable field, retained by the chat card that owns it. It
//! keeps the string, a byte cursor, a selection anchor and a blink instant; the
//! shaping and hit-testing come from [`crownshell::Text`], which caches its
//! layout across frames.
//!
//! **Every cursor position is a `char` boundary.** The cursor is a byte index
//! because that is what `Text::index_at` and `Text::caret` speak, but it is only
//! ever moved by whole characters: slicing a `String` in the middle of a
//! multi-byte character panics, and "é" and "🎉" are not exotic input for a
//! reply field, they are Tuesday.
//!
//! The field never sets a line clamp. `Text` documents that caret indices are
//! into the text it *laid out*, which a `max_lines` clamp replaces with a
//! prefix plus an ellipsis — so a clamp here would silently start editing a
//! different string from the one the user typed.

use std::time::Instant;

use crownshell::{
    KeyPress, Keysym, Scene, Text, TextContext,
    kurbo::{Affine, Point, Rect},
    peniko::{Fill, Mix},
};

use crate::ui::{
    theme::Theme,
    widget::{
        surface::{HAIRLINE, fill_rounded, multiply_alpha, stroke_rounded},
        sync_style,
    },
};

/// Caret blink half-period. macOS blinks at 530 ms and the eye notices when it
/// does not.
const BLINK_MS: u128 = 530;
/// Horizontal padding inside the field.
const PAD_X: f64 = 10.0;
/// How much text is kept visible past the caret when scrolling it into view, so
/// the next character typed is not already off the edge.
const CARET_MARGIN: f64 = 2.0;
/// Opacity of the selection wash, over the field's own fill.
const SELECTION_ALPHA: f32 = 0.28;

/// What a key did to the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldEvent {
    /// The field did not want the key; the card should try it next.
    Ignored,
    /// The text or the cursor moved, so the surface has to repaint.
    Changed,
    /// Return: the caller should [`take`](TextField::take) the text and send it.
    Submit,
    /// Escape: the caller should unfocus the field and leave the text alone.
    Cancel,
}

/// A single-line editable field.
pub struct TextField {
    /// The authoritative content. [`text`](Self::text) is synced from this
    /// rather than the other way round, because editing a `Text` in place would
    /// mean re-laying it out on every keystroke *and* on every frame.
    value: String,
    text: Text,
    placeholder: Text,
    /// Byte index of the caret. Always a `char` boundary of `value`.
    cursor: usize,
    /// The other end of the selection; equal to the cursor when nothing is
    /// selected. Always a `char` boundary of `value`.
    anchor: usize,
    focused: bool,
    /// When the caret last became visible. Reset by anything the user does, so
    /// the caret is solid while they type instead of blinking under their hands.
    blink: Instant,
    /// How far the text is scrolled left to keep the caret in view.
    scroll: f64,
}

impl TextField {
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            value: String::new(),
            text: Text::new(""),
            placeholder: Text::new(placeholder),
            cursor: 0,
            anchor: 0,
            focused: false,
            blink: Instant::now(),
            scroll: 0.0,
        }
    }

    pub fn text(&self) -> &str {
        &self.value
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn placeholder(&self) -> &str {
        self.placeholder.text()
    }

    /// Replaces the content and puts the caret at the end.
    pub fn set_text(&mut self, text: &str) {
        self.value.clear();
        self.value.push_str(text);
        self.cursor = self.value.len();
        self.anchor = self.cursor;
        self.scroll = 0.0;
        self.wake();
    }

    /// Drains the field, resetting the cursor. Keeps the buffer, since the next
    /// reply will need one.
    pub fn take(&mut self) -> String {
        let taken = self.value.drain(..).collect();
        self.cursor = 0;
        self.anchor = 0;
        self.scroll = 0.0;
        self.wake();
        taken
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            self.wake();
        }
    }

    /// Byte index of the caret.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected byte range, low end first. Empty when there is no selection.
    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    /// Whether the caret is in the visible half of its blink cycle.
    ///
    /// [`render`](Self::render) reads this itself; it is public so the surface
    /// can decide whether its tick actually needs to repaint anything.
    pub fn caret_visible(&self) -> bool {
        self.focused && (self.blink.elapsed().as_millis() / BLINK_MS).is_multiple_of(2)
    }

    /// Handles a key. Returns [`FieldEvent::Ignored`] for anything the field
    /// does not use, including every key while it is not focused, so the card
    /// can fall through to its own shortcuts.
    pub fn on_key(&mut self, key: &KeyPress<'_>) -> FieldEvent {
        if !self.focused {
            return FieldEvent::Ignored;
        }
        let ctrl = key.modifiers.ctrl;
        let shift = key.modifiers.shift;

        match key.keysym {
            Keysym::Return | Keysym::KP_Enter => FieldEvent::Submit,
            Keysym::Escape => FieldEvent::Cancel,
            Keysym::BackSpace => {
                if self.delete_selection() {
                    return self.changed();
                }
                if self.cursor == 0 {
                    return FieldEvent::Changed;
                }
                let from = if ctrl {
                    self.word_start(self.cursor)
                } else {
                    self.prev_boundary(self.cursor)
                };
                self.value.replace_range(from..self.cursor, "");
                self.cursor = from;
                self.anchor = from;
                self.changed()
            }
            Keysym::Delete => {
                if self.delete_selection() {
                    return self.changed();
                }
                if self.cursor == self.value.len() {
                    return FieldEvent::Changed;
                }
                let to = if ctrl {
                    self.word_end(self.cursor)
                } else {
                    self.next_boundary(self.cursor)
                };
                self.value.replace_range(self.cursor..to, "");
                self.anchor = self.cursor;
                self.changed()
            }
            Keysym::Left => {
                let to = if ctrl {
                    self.word_start(self.cursor)
                } else if !shift && self.cursor != self.anchor {
                    // Collapsing a selection moves to its near edge rather than
                    // one character further, which is what every other field on
                    // the desktop does.
                    self.selection().0
                } else {
                    self.prev_boundary(self.cursor)
                };
                self.move_cursor(to, shift)
            }
            Keysym::Right => {
                let to = if ctrl {
                    self.word_end(self.cursor)
                } else if !shift && self.cursor != self.anchor {
                    self.selection().1
                } else {
                    self.next_boundary(self.cursor)
                };
                self.move_cursor(to, shift)
            }
            Keysym::Home => self.move_cursor(0, shift),
            Keysym::End => self.move_cursor(self.value.len(), shift),
            Keysym::a | Keysym::A if ctrl => {
                self.anchor = 0;
                self.cursor = self.value.len();
                self.changed()
            }
            _ => self.insert(key),
        }
    }

    /// Places the caret from a click, where `local` is relative to the field's
    /// own top-left corner — the same rectangle [`render`](Self::render) was
    /// given.
    ///
    /// Focus is deliberately not changed here: raising keyboard interactivity
    /// on the layer surface is the surface's decision, and it has to happen
    /// even when the field is focused by a keyboard shortcut rather than a
    /// click.
    pub fn on_click(&mut self, local: Point, tcx: &mut TextContext) {
        self.text.set_text(&self.value);
        let y = self.text.height(tcx) / 2.0;
        let index = self
            .text
            .index_at(tcx, (local.x - PAD_X + self.scroll, y))
            .min(self.value.len());
        // `index_at` reports a cluster boundary, which is always a char
        // boundary — but it indexes the laid-out text, and a stale layout would
        // make that a boundary of the *previous* string.
        self.cursor = self.clamp_boundary(index);
        self.anchor = self.cursor;
        self.wake();
    }

    /// Draws the field filling `bounds`.
    pub fn render(
        &mut self,
        scene: &mut Scene,
        bounds: Rect,
        theme: &Theme,
        tcx: &mut TextContext,
    ) {
        let radius = bounds.height() / 2.0;
        fill_rounded(scene, bounds, radius, theme.palette.field, 1.0);
        stroke_rounded(
            scene,
            bounds,
            radius,
            if self.focused {
                theme.palette.accent
            } else {
                theme.palette.field_rim
            },
            1.0,
            HAIRLINE,
        );

        let inner = Rect::new(bounds.x0 + PAD_X, bounds.y0, bounds.x1 - PAD_X, bounds.y1);
        if inner.width() <= 0.0 {
            return;
        }

        if self.value.is_empty() && !self.focused {
            sync_style(&mut self.placeholder, &theme.text.meta);
            let size = self.placeholder.size(tcx);
            let y = (bounds.y0 + (bounds.height() - size.height) / 2.0).round();
            self.placeholder.draw(tcx, scene, (inner.x0, y));
            return;
        }

        self.text.set_text(&self.value);
        sync_style(&mut self.text, &theme.text.field);
        let size = self.text.size(tcx);
        let y = (bounds.y0 + (bounds.height() - size.height) / 2.0).round();
        let caret = self.text.caret(tcx, self.cursor);
        self.update_scroll(caret.x0, size.width, inner.width());
        let origin_x = inner.x0 - self.scroll;

        // Clipped to the field, not the surface: a long reply has to be cut off
        // at the field's edge rather than run across the card.
        scene.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &inner);

        let (start, end) = self.selection();
        if start != end {
            let from = self.text.caret(tcx, start).x0;
            let to = self.text.caret(tcx, end).x0;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                multiply_alpha(theme.palette.accent, SELECTION_ALPHA),
                None,
                &Rect::new(origin_x + from, y, origin_x + to, y + size.height),
            );
        }

        self.text.draw(tcx, scene, (origin_x, y));

        if self.caret_visible() {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                theme.palette.accent,
                None,
                &Rect::new(
                    origin_x + caret.x0,
                    y + caret.y0,
                    origin_x + caret.x1,
                    y + caret.y1,
                ),
            );
        }

        scene.pop_layer();
    }

    /// Keeps the caret inside the visible window, scrolling by as little as it
    /// takes. Also pulls the text back when it shrinks, so deleting the tail of
    /// a long reply does not leave the field showing empty space.
    fn update_scroll(&mut self, caret_x: f64, text_width: f64, view_width: f64) {
        if caret_x - self.scroll > view_width - CARET_MARGIN {
            self.scroll = caret_x - view_width + CARET_MARGIN;
        }
        if caret_x - self.scroll < 0.0 {
            self.scroll = caret_x;
        }
        self.scroll = self.scroll.clamp(0.0, (text_width - view_width).max(0.0));
    }

    /// Inserts the text a printable key carries, replacing any selection.
    fn insert(&mut self, key: &KeyPress<'_>) -> FieldEvent {
        // Ctrl and Alt chords still carry text on some keymaps ("\u{1}" for
        // Ctrl+A), and every named key above carries its control character.
        // Either would end up in the reply if this only checked for `Some`.
        let Some(text) = key.text.filter(|text| {
            !text.is_empty()
                && !key.modifiers.ctrl
                && !key.modifiers.alt
                && !text.chars().any(char::is_control)
        }) else {
            return FieldEvent::Ignored;
        };

        self.delete_selection();
        self.value.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.anchor = self.cursor;
        self.changed()
    }

    /// Moves the caret, extending the selection when `shift` is held and
    /// collapsing it otherwise.
    fn move_cursor(&mut self, to: usize, shift: bool) -> FieldEvent {
        self.cursor = self.clamp_boundary(to);
        if !shift {
            self.anchor = self.cursor;
        }
        self.changed()
    }

    /// Removes the selection if there is one, reporting whether it did.
    fn delete_selection(&mut self) -> bool {
        let (start, end) = self.selection();
        if start == end {
            return false;
        }
        self.value.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        true
    }

    fn changed(&mut self) -> FieldEvent {
        self.wake();
        FieldEvent::Changed
    }

    /// Restarts the blink cycle so the caret is solid right after an edit.
    fn wake(&mut self) {
        self.blink = Instant::now();
    }

    /// The start of the character before `index`, or 0.
    fn prev_boundary(&self, index: usize) -> usize {
        self.value[..index]
            .chars()
            .next_back()
            .map_or(0, |c| index - c.len_utf8())
    }

    /// The start of the character after `index`, or the end of the text.
    fn next_boundary(&self, index: usize) -> usize {
        self.value[index..]
            .chars()
            .next()
            .map_or(index, |c| index + c.len_utf8())
    }

    /// Start of the word at or before `index`: back over any whitespace, then
    /// back over the word itself.
    fn word_start(&self, index: usize) -> usize {
        let mut at = index;
        while at > 0 {
            let previous = self.prev_boundary(at);
            if !self.value[previous..at].starts_with(char::is_whitespace) {
                break;
            }
            at = previous;
        }
        while at > 0 {
            let previous = self.prev_boundary(at);
            if self.value[previous..at].starts_with(char::is_whitespace) {
                break;
            }
            at = previous;
        }
        at
    }

    /// End of the word at or after `index`, mirroring [`word_start`].
    fn word_end(&self, index: usize) -> usize {
        let mut at = index;
        let end = self.value.len();
        while at < end {
            let next = self.next_boundary(at);
            if !self.value[at..next].starts_with(char::is_whitespace) {
                break;
            }
            at = next;
        }
        while at < end {
            let next = self.next_boundary(at);
            if self.value[at..next].starts_with(char::is_whitespace) {
                break;
            }
            at = next;
        }
        at
    }

    /// Snaps an index onto the nearest `char` boundary at or below it, and into
    /// the string. The rest of the field maintains this invariant itself; this
    /// is the guard for indices that came from outside, where being wrong means
    /// a panic rather than a misplaced caret.
    fn clamp_boundary(&self, index: usize) -> usize {
        let mut index = index.min(self.value.len());
        while index > 0 && !self.value.is_char_boundary(index) {
            index -= 1;
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crownshell::Modifiers;

    fn field() -> TextField {
        let mut field = TextField::new("Reply");
        field.set_focused(true);
        field
    }

    fn press(field: &mut TextField, keysym: Keysym) -> FieldEvent {
        key(field, keysym, None, Modifiers::default())
    }

    fn ctrl(field: &mut TextField, keysym: Keysym) -> FieldEvent {
        key(
            field,
            keysym,
            None,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        )
    }

    fn shift(field: &mut TextField, keysym: Keysym) -> FieldEvent {
        key(
            field,
            keysym,
            None,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        )
    }

    fn key(
        field: &mut TextField,
        keysym: Keysym,
        text: Option<&str>,
        modifiers: Modifiers,
    ) -> FieldEvent {
        field.on_key(&KeyPress {
            keysym,
            raw_code: 0,
            text,
            modifiers,
            repeat: false,
        })
    }

    /// Types `text` one character at a time, the way a keyboard would.
    fn type_text(field: &mut TextField, text: &str) {
        for character in text.chars() {
            let mut buffer = [0_u8; 4];
            let encoded = character.encode_utf8(&mut buffer);
            assert_eq!(
                key(field, Keysym::from(0), Some(encoded), Modifiers::default()),
                FieldEvent::Changed,
                "typing {character:?} was ignored"
            );
        }
    }

    #[test]
    fn an_unfocused_field_ignores_every_key() {
        let mut field = TextField::new("Reply");
        assert!(!field.is_focused());
        assert_eq!(press(&mut field, Keysym::Return), FieldEvent::Ignored);
        type_ignored(&mut field, "a");
        assert_eq!(field.text(), "");
    }

    fn type_ignored(field: &mut TextField, text: &str) {
        assert_eq!(
            key(field, Keysym::from(0), Some(text), Modifiers::default()),
            FieldEvent::Ignored
        );
    }

    #[test]
    fn typing_inserts_at_the_caret_and_moves_it_along() {
        let mut field = field();
        type_text(&mut field, "hello");
        assert_eq!(field.text(), "hello");
        assert_eq!(field.cursor(), 5);

        press(&mut field, Keysym::Home);
        type_text(&mut field, "oh ");
        assert_eq!(field.text(), "oh hello");
        assert_eq!(field.cursor(), 3);
    }

    #[test]
    fn control_characters_and_chords_never_reach_the_text() {
        let mut field = field();
        // What a keymap reports for Ctrl+A alongside the keysym.
        assert_eq!(
            key(
                &mut field,
                Keysym::a,
                Some("\u{1}"),
                Modifiers {
                    ctrl: true,
                    ..Modifiers::default()
                }
            ),
            FieldEvent::Changed,
            "Ctrl+A should have been read as select-all"
        );
        assert_eq!(field.text(), "", "a chord inserted its control character");

        assert_eq!(
            key(
                &mut field,
                Keysym::from(0),
                Some("\t"),
                Modifiers::default()
            ),
            FieldEvent::Ignored
        );
        assert_eq!(field.text(), "");
    }

    #[test]
    fn backspace_and_delete_remove_whole_characters_not_bytes() {
        let mut field = field();
        type_text(&mut field, "héllo wörld");
        assert_eq!(field.text(), "héllo wörld");
        assert_eq!(field.cursor(), field.text().len());

        press(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "héllo wörl");

        press(&mut field, Keysym::Home);
        press(&mut field, Keysym::Right);
        // The caret is now on 'é', which is two bytes wide.
        assert_eq!(field.cursor(), 1);
        press(&mut field, Keysym::Delete);
        assert_eq!(field.text(), "hllo wörl", "Delete split a character");
        assert_eq!(field.cursor(), 1);

        press(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "llo wörl");
        assert_eq!(field.cursor(), 0);
    }

    #[test]
    fn emoji_are_edited_as_single_characters() {
        let mut field = field();
        type_text(&mut field, "party 🎉🎈");
        assert_eq!(field.cursor(), field.text().len());

        press(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "party 🎉", "backspace ate part of an emoji");

        press(&mut field, Keysym::Left);
        assert_eq!(field.cursor(), 6, "Left landed inside the emoji");
        press(&mut field, Keysym::Right);
        assert_eq!(field.cursor(), 10);

        press(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "party ");
    }

    #[test]
    fn the_caret_stops_at_both_ends_instead_of_running_off_them() {
        let mut field = field();
        type_text(&mut field, "hé🎉");

        for _ in 0..10 {
            press(&mut field, Keysym::Left);
        }
        assert_eq!(field.cursor(), 0);
        assert_eq!(
            press(&mut field, Keysym::BackSpace),
            FieldEvent::Changed,
            "backspace at the start must be harmless"
        );
        assert_eq!(field.text(), "hé🎉");

        for _ in 0..10 {
            press(&mut field, Keysym::Right);
        }
        assert_eq!(field.cursor(), field.text().len());
        press(&mut field, Keysym::Delete);
        assert_eq!(field.text(), "hé🎉");
    }

    #[test]
    fn ctrl_arrows_jump_over_whole_words() {
        let mut field = field();
        type_text(&mut field, "héllo wörld again");

        ctrl(&mut field, Keysym::Left);
        assert_eq!(&field.text()[field.cursor()..], "again");
        ctrl(&mut field, Keysym::Left);
        assert_eq!(&field.text()[field.cursor()..], "wörld again");
        ctrl(&mut field, Keysym::Left);
        assert_eq!(field.cursor(), 0);
        ctrl(&mut field, Keysym::Left);
        assert_eq!(field.cursor(), 0, "a word jump ran past the start");

        ctrl(&mut field, Keysym::Right);
        assert_eq!(&field.text()[..field.cursor()], "héllo");
        ctrl(&mut field, Keysym::Right);
        assert_eq!(&field.text()[..field.cursor()], "héllo wörld");
    }

    #[test]
    fn ctrl_backspace_deletes_the_word_before_the_caret() {
        let mut field = field();
        type_text(&mut field, "héllo wörld");
        ctrl(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "héllo ");

        press(&mut field, Keysym::Home);
        ctrl(&mut field, Keysym::Delete);
        assert_eq!(field.text(), " ");
    }

    #[test]
    fn select_all_then_typing_replaces_the_whole_reply() {
        let mut field = field();
        type_text(&mut field, "héllo");
        ctrl(&mut field, Keysym::a);
        assert_eq!(field.selection(), (0, field.text().len()));

        type_text(&mut field, "x");
        assert_eq!(field.text(), "x");
        assert_eq!(field.selection(), (1, 1));
    }

    #[test]
    fn select_all_then_backspace_empties_the_field() {
        let mut field = field();
        type_text(&mut field, "héllo 🎉");
        ctrl(&mut field, Keysym::A);
        press(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "");
        assert_eq!(field.cursor(), 0);
        assert!(field.is_empty());
    }

    #[test]
    fn shift_arrows_extend_a_selection_and_a_plain_arrow_collapses_it() {
        let mut field = field();
        type_text(&mut field, "héllo");
        press(&mut field, Keysym::Home);
        shift(&mut field, Keysym::Right);
        shift(&mut field, Keysym::Right);
        assert_eq!(field.selection(), (0, 3), "'é' is two bytes wide");

        press(&mut field, Keysym::Right);
        assert_eq!(
            field.selection(),
            (3, 3),
            "a plain Right must collapse to the far edge"
        );
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_text() {
        let mut field = field();
        type_text(&mut field, "héllo wörld");
        press(&mut field, Keysym::Home);
        assert_eq!(field.cursor(), 0);
        press(&mut field, Keysym::End);
        assert_eq!(field.cursor(), field.text().len());
    }

    #[test]
    fn return_submits_and_escape_cancels_without_touching_the_text() {
        let mut field = field();
        type_text(&mut field, "on my way");

        assert_eq!(press(&mut field, Keysym::Return), FieldEvent::Submit);
        assert_eq!(press(&mut field, Keysym::KP_Enter), FieldEvent::Submit);
        assert_eq!(
            field.text(),
            "on my way",
            "submitting must not drain the field; only take() does"
        );
        assert_eq!(press(&mut field, Keysym::Escape), FieldEvent::Cancel);
        assert_eq!(field.text(), "on my way");
    }

    #[test]
    fn take_drains_the_field_and_resets_the_caret() {
        let mut field = field();
        type_text(&mut field, "héllo 🎉");
        assert_eq!(field.take(), "héllo 🎉");
        assert_eq!(field.text(), "");
        assert_eq!(field.cursor(), 0);
        assert_eq!(field.selection(), (0, 0));
        assert_eq!(field.take(), "", "taking an empty field is not an error");

        // And the drained field is still usable.
        type_text(&mut field, "again");
        assert_eq!(field.text(), "again");
    }

    #[test]
    fn setting_the_text_puts_the_caret_at_the_end() {
        let mut field = field();
        field.set_text("héllo");
        assert_eq!(field.cursor(), 6);
        assert_eq!(field.selection(), (6, 6));
        press(&mut field, Keysym::BackSpace);
        assert_eq!(field.text(), "héll");
    }

    #[test]
    fn unknown_keys_fall_through_to_the_card() {
        let mut field = field();
        assert_eq!(press(&mut field, Keysym::F5), FieldEvent::Ignored);
        assert_eq!(press(&mut field, Keysym::Up), FieldEvent::Ignored);
    }

    #[test]
    fn the_caret_blinks_only_while_the_field_is_focused() {
        let mut field = TextField::new("Reply");
        assert!(!field.caret_visible());
        field.set_focused(true);
        assert!(
            field.caret_visible(),
            "the caret must be solid right after focus, not mid-blink"
        );
    }

    #[test]
    fn clicking_places_the_caret_on_a_character_boundary() {
        let mut tcx = TextContext::new();
        let theme = Theme::light();
        let mut field = field();
        type_text(&mut field, "héllo wörld");

        // Render once so the field has a laid-out line to hit-test against.
        let bounds = Rect::new(0.0, 0.0, 240.0, theme.metrics.field_height);
        let mut scene = Scene::new();
        field.render(&mut scene, bounds, &theme, &mut tcx);

        for x in [0.0, 4.0, 12.0, 30.0, 60.0, 120.0, 400.0] {
            field.on_click(Point::new(x, bounds.center().y), &mut tcx);
            let cursor = field.cursor();
            assert!(
                field.text().is_char_boundary(cursor),
                "clicking at {x} put the caret at byte {cursor}, inside a character"
            );
            assert_eq!(field.selection(), (cursor, cursor));
        }

        field.on_click(Point::new(0.0, 0.0), &mut tcx);
        assert_eq!(field.cursor(), 0);
        field.on_click(Point::new(1000.0, 0.0), &mut tcx);
        assert_eq!(field.cursor(), field.text().len());
    }

    #[test]
    fn a_field_draws_its_frame_the_placeholder_and_then_the_typed_text() {
        let mut tcx = TextContext::new();
        let theme = Theme::light();
        let bounds = Rect::new(0.0, 0.0, 240.0, theme.metrics.field_height);

        let mut empty = TextField::new("Reply");
        let mut scene = Scene::new();
        empty.render(&mut scene, bounds, &theme, &mut tcx);
        assert_eq!(
            scene.encoding().resources.glyphs.len(),
            5,
            "expected the placeholder to be drawn"
        );

        let mut typed = field();
        type_text(&mut typed, "hi");
        let mut scene = Scene::new();
        typed.render(&mut scene, bounds, &theme, &mut tcx);
        assert_eq!(
            scene.encoding().resources.glyphs.len(),
            2,
            "a focused field must show its text, not the placeholder"
        );
    }

    /// The field scrolls its own text; a `max_lines` clamp would rewrite the
    /// laid-out string and desynchronise every caret index from `value`.
    #[test]
    fn the_field_never_clamps_or_wraps_the_text_it_edits() {
        let mut tcx = TextContext::new();
        let theme = Theme::light();
        let mut field = field();
        type_text(
            &mut field,
            "a reply long enough to run well past the end of the field",
        );

        let bounds = Rect::new(0.0, 0.0, 120.0, theme.metrics.field_height);
        let mut scene = Scene::new();
        field.render(&mut scene, bounds, &theme, &mut tcx);

        assert_eq!(field.text.max_lines(), None);
        assert_eq!(field.text.max_width(), None);
        assert_eq!(field.text.line_count(&mut tcx), 1);
        assert!(
            field.scroll > 0.0,
            "the caret ran off the end without scrolling the text"
        );

        press(&mut field, Keysym::Home);
        field.render(&mut scene, bounds, &theme, &mut tcx);
        assert_eq!(
            field.scroll, 0.0,
            "going back to the start did not scroll the text back"
        );
    }
}
