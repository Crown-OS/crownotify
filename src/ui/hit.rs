//! Where a click lands, and what it means.
//!
//! Cards are drawn immediately rather than retained as a widget tree, so the
//! only record of what is where is the [`HitMap`] a paint fills in. It is
//! rebuilt every frame and doubles as the source of the surface's input region,
//! which is what keeps the parts of the toast strip that hold no card
//! click-through instead of swallowing clicks meant for the window below.

use crownshell::kurbo::{Point, Rect};

use crate::model::NotificationId;

/// What a click on a card means, in the UI's own vocabulary.
///
/// Deliberately names no transport type: the surface layer translates
/// `Answer`/`Decline`/`Media` into `ipc::PhoneCommand`. That keeps `ui` free of
/// any dependency on `ipc` and makes these cases trivial to assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAction {
    /// The default action, or a bare click on the card body.
    Activate,
    Dismiss,
    /// A named action from the notification's own action list.
    Invoke(String),
    FocusReply,
    SubmitReply,
    /// A bare click on a collapsed deck: pull its cards apart, or push them
    /// back together. Carries the group key because a deck's index does not
    /// survive the frame that drew it.
    ToggleStack(String),
    /// Clear every notification the deck holds.
    ClearStack(String),
    /// Reveal, or hide, the actions that did not fit on the card.
    ToggleMenu,
    Answer,
    Decline,
    Media(MediaAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    Next,
    Previous,
    /// Position as a percentage of the track, matching the crownconnect protocol.
    Seek(u8),
}

/// One clickable rectangle, in surface-local logical pixels.
#[derive(Debug, Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub id: NotificationId,
    pub action: CardAction,
}

/// Every clickable rectangle in the frame that was last painted.
///
/// The backing `Vec` is reused across frames — [`clear`](Self::clear) keeps the
/// allocation — because this is rebuilt at the compositor's frame rate and a
/// per-frame allocation is exactly the kind of cost that shows up as jitter in
/// an animation.
#[derive(Debug, Default)]
pub struct HitMap {
    regions: Vec<HitRegion>,
}

impl HitMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops every region while keeping the memory they were stored in. Call it
    /// at the top of a paint, not at the end of one: the map has to survive
    /// until the next frame so pointer events that arrive in between can still
    /// be resolved.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn push(&mut self, rect: Rect, id: NotificationId, action: CardAction) {
        self.regions.push(HitRegion { rect, id, action });
    }

    /// Topmost region containing `point`; later pushes win.
    ///
    /// Cards paint back to front, so a button pushed after the card body it
    /// sits on is the one that gets the click — the same rule the painter's
    /// algorithm that drew them followed.
    pub fn hit(&self, point: Point) -> Option<&HitRegion> {
        self.regions
            .iter()
            .rev()
            .find(|region| region.rect.contains(point))
    }

    /// Every region's rectangle, for `SurfaceCtx::set_input_region`.
    pub fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.regions.iter().map(|region| region.rect)
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::new(x0, y0, x1, y1)
    }

    #[test]
    fn a_point_outside_every_region_hits_nothing() {
        let mut map = HitMap::new();
        assert!(map.is_empty());
        assert!(map.hit(Point::new(5.0, 5.0)).is_none());

        map.push(rect(0.0, 0.0, 10.0, 10.0), 1, CardAction::Activate);
        assert!(map.hit(Point::new(20.0, 5.0)).is_none());
        assert!(map.hit(Point::new(5.0, 20.0)).is_none());
    }

    #[test]
    fn the_last_region_pushed_over_a_point_is_the_one_that_is_hit() {
        let mut map = HitMap::new();
        map.push(rect(0.0, 0.0, 100.0, 40.0), 7, CardAction::Activate);
        map.push(rect(70.0, 8.0, 94.0, 32.0), 7, CardAction::Dismiss);
        map.push(
            rect(74.0, 12.0, 90.0, 28.0),
            7,
            CardAction::Media(MediaAction::Play),
        );

        let hit = map.hit(Point::new(80.0, 20.0)).expect("nothing was hit");
        assert_eq!(
            hit.action,
            CardAction::Media(MediaAction::Play),
            "an earlier push won over a later one"
        );
        assert_eq!(hit.id, 7);

        // Just outside the topmost two, still inside the card body.
        assert_eq!(
            map.hit(Point::new(96.0, 20.0)).map(|hit| &hit.action),
            Some(&CardAction::Activate)
        );
        assert_eq!(
            map.hit(Point::new(72.0, 20.0)).map(|hit| &hit.action),
            Some(&CardAction::Dismiss)
        );
    }

    #[test]
    fn regions_carry_the_notification_they_belong_to() {
        let mut map = HitMap::new();
        map.push(rect(0.0, 0.0, 100.0, 40.0), 1, CardAction::Activate);
        map.push(rect(0.0, 50.0, 100.0, 90.0), 2, CardAction::Activate);

        assert_eq!(map.hit(Point::new(50.0, 20.0)).map(|hit| hit.id), Some(1));
        assert_eq!(map.hit(Point::new(50.0, 70.0)).map(|hit| hit.id), Some(2));
        assert_eq!(map.len(), 2);
        assert_eq!(map.rects().count(), 2);
        assert_eq!(map.rects().next(), Some(rect(0.0, 0.0, 100.0, 40.0)));
    }

    /// A paint runs at the compositor's frame rate, so the map must not be a
    /// per-frame allocation.
    #[test]
    fn clearing_the_map_keeps_its_allocation_for_the_next_frame() {
        let mut map = HitMap::new();
        for id in 0..8 {
            map.push(rect(0.0, 0.0, 10.0, 10.0), id, CardAction::Activate);
        }
        let capacity = map.regions.capacity();
        let buffer = map.regions.as_ptr();

        map.clear();
        assert!(map.is_empty());
        assert_eq!(
            map.regions.capacity(),
            capacity,
            "clear() gave the buffer back"
        );

        for id in 0..8 {
            map.push(rect(0.0, 0.0, 10.0, 10.0), id, CardAction::Activate);
        }
        assert_eq!(
            map.regions.capacity(),
            capacity,
            "refilling the map re-allocated"
        );
        assert_eq!(
            map.regions.as_ptr(),
            buffer,
            "the regions moved to a new buffer"
        );
    }

    #[test]
    fn actions_compare_by_value_so_a_dispatcher_can_match_on_them() {
        assert_eq!(
            CardAction::Invoke("reply".to_string()),
            CardAction::Invoke("reply".to_string())
        );
        assert_ne!(
            CardAction::Invoke("reply".to_string()),
            CardAction::Invoke("archive".to_string())
        );
        assert_ne!(
            CardAction::Media(MediaAction::Seek(0)),
            CardAction::Media(MediaAction::Seek(1))
        );
        assert_ne!(CardAction::Answer, CardAction::Decline);
    }
}
