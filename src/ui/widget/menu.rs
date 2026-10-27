//! The menu an overflowing action list spills into.
//!
//! A sender may attach more actions than a card is wide, and the card used to
//! stop drawing at the edge — so an app's "Snooze" silently did not exist. The
//! ones that do not fit go here instead, behind a chevron.
//!
//! The menu draws into the surface the card is already on rather than a popup
//! of its own: the toast strip is a fixed full-height layer surface with the
//! cards animating inside it, so there is nowhere a second surface would need
//! to go, and a new one would cost a round trip to the compositor per open.

use crownshell::{
    Scene, Text, TextContext,
    kurbo::{Point, Rect},
};

use crate::ui::{
    theme::Theme,
    widget::{button::ButtonState, surface::fill_rounded, sync_style},
};

/// Height of one row.
const ROW_HEIGHT: f64 = 30.0;
/// Inset of a row's label from the menu's left edge.
const ROW_PAD_X: f64 = 12.0;
/// Padding above the first row and below the last.
const PAD_Y: f64 = 4.0;
/// Corner radius of the menu panel, and of a highlighted row.
const RADIUS: f64 = 10.0;
const ROW_RADIUS: f64 = 6.0;

/// The actions that did not fit on the card, and their retained labels.
#[derive(Default)]
pub struct Menu {
    items: Vec<(String, Text)>,
    open: bool,
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open && !self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Replaces the items, reporting whether the set actually changed.
    ///
    /// Compared before it is written so an unchanged menu keeps its shaped
    /// labels, for the same reason every other retained text in a card does.
    pub fn sync<'a>(&mut self, items: impl Iterator<Item = (&'a str, &'a str)>) -> bool {
        let wanted: Vec<_> = items.collect();
        let unchanged = wanted.len() == self.items.len()
            && wanted
                .iter()
                .zip(&self.items)
                .all(|((key, label), (have, text))| have == key && text.text() == *label);
        if unchanged {
            return false;
        }
        self.items = wanted
            .into_iter()
            .map(|(key, label)| (key.to_string(), Text::new(label)))
            .collect();
        if self.items.is_empty() {
            self.open = false;
        }
        true
    }

    /// How tall the menu is when open, and zero when it is not.
    pub fn height(&self) -> f64 {
        if !self.is_open() {
            return 0.0;
        }
        self.items.len() as f64 * ROW_HEIGHT + 2.0 * PAD_Y
    }

    /// The action key at `point`, if the open menu has a row there.
    pub fn key_at(&self, bounds: Rect, point: Point) -> Option<&str> {
        if !self.is_open() || !bounds.contains(point) {
            return None;
        }
        let row = ((point.y - bounds.y0 - PAD_Y) / ROW_HEIGHT).floor();
        let index = usize::try_from(row as i64).ok()?;
        self.items.get(index).map(|(key, _)| key.as_str())
    }

    /// Draws the panel and returns each row's rectangle with the key it invokes.
    ///
    /// The caller pushes those into the [`HitMap`](crate::ui::hit::HitMap)
    /// itself, after the card, so a row wins the click over the body beneath it.
    pub fn render(
        &mut self,
        scene: &mut Scene,
        bounds: Rect,
        theme: &Theme,
        hovered: Option<&str>,
        alpha: f32,
        tcx: &mut TextContext,
    ) -> Vec<(Rect, String)> {
        if !self.is_open() {
            return Vec::new();
        }
        fill_rounded(scene, bounds, RADIUS, theme.palette.field, alpha);

        let mut rows = Vec::with_capacity(self.items.len());
        let mut y = bounds.y0 + PAD_Y;
        for (key, label) in &mut self.items {
            let row = Rect::new(bounds.x0, y, bounds.x1, y + ROW_HEIGHT);
            if hovered == Some(key.as_str()) {
                fill_rounded(
                    scene,
                    row.inset(-2.0),
                    ROW_RADIUS,
                    theme.palette.separator,
                    alpha,
                );
            }
            sync_style(label, &theme.text.body);
            let size = label.size(tcx);
            label.draw(
                tcx,
                scene,
                (
                    (row.x0 + ROW_PAD_X).round(),
                    (row.center().y - size.height / 2.0).round(),
                ),
            );
            rows.push((row, key.clone()));
            y = row.y1;
        }
        rows
    }

    /// The width the widest label needs, so the card can size the panel.
    pub fn width(&mut self, tcx: &mut TextContext, theme: &Theme) -> f64 {
        self.items
            .iter_mut()
            .map(|(_, label)| {
                sync_style(label, &theme.text.body);
                label.size(tcx).width + 2.0 * ROW_PAD_X
            })
            .fold(0.0, f64::max)
    }
}

/// Whether the chevron should look pressed, given the menu's state.
pub fn chevron_state(open: bool, hovered: bool) -> ButtonState {
    match (open, hovered) {
        (true, _) => ButtonState::Pressed,
        (false, true) => ButtonState::Hover,
        (false, false) => ButtonState::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(items: &[(&str, &str)]) -> Menu {
        let mut menu = Menu::new();
        menu.sync(items.iter().copied());
        menu
    }

    #[test]
    fn an_empty_menu_is_never_open_and_takes_no_room() {
        let mut menu = Menu::new();
        menu.toggle();
        assert!(!menu.is_open());
        assert_eq!(menu.height(), 0.0);
    }

    #[test]
    fn a_closed_menu_takes_no_room_but_a_open_one_grows_with_its_rows() {
        let mut menu = menu(&[("snooze", "Snooze"), ("later", "Remind me later")]);
        assert_eq!(menu.height(), 0.0, "closed");
        menu.toggle();
        assert!(menu.height() > 2.0 * ROW_HEIGHT, "{}", menu.height());
    }

    #[test]
    fn syncing_the_same_items_keeps_the_shaped_labels() {
        let mut menu = menu(&[("snooze", "Snooze")]);
        assert!(
            !menu.sync([("snooze", "Snooze")].into_iter()),
            "an unchanged menu re-shaped its text"
        );
        assert!(menu.sync([("snooze", "Snooze later")].into_iter()));
    }

    #[test]
    fn losing_every_item_closes_the_menu() {
        let mut menu = menu(&[("snooze", "Snooze")]);
        menu.toggle();
        assert!(menu.is_open());
        menu.sync(std::iter::empty());
        assert!(!menu.is_open(), "a menu with nothing in it cannot be open");
    }

    #[test]
    fn a_click_resolves_to_the_row_it_landed_on() {
        let mut menu = menu(&[("snooze", "Snooze"), ("later", "Later")]);
        menu.toggle();
        let bounds = Rect::new(0.0, 0.0, 200.0, menu.height());

        assert_eq!(
            menu.key_at(bounds, Point::new(10.0, PAD_Y + 1.0)),
            Some("snooze")
        );
        assert_eq!(
            menu.key_at(bounds, Point::new(10.0, PAD_Y + ROW_HEIGHT + 1.0)),
            Some("later")
        );
        assert_eq!(menu.key_at(bounds, Point::new(400.0, 10.0)), None);
    }

    #[test]
    fn a_closed_menu_resolves_no_click_at_all() {
        let menu = menu(&[("snooze", "Snooze")]);
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(menu.key_at(bounds, Point::new(10.0, 10.0)), None);
    }
}
