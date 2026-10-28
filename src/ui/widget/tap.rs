//! A click that belongs to the control it landed on.

use crownui::{
    ext::ClickEvent,
    prelude::{Cx, Event},
};

/// A click handler that runs `handler` and keeps the click from reaching the
/// card underneath, whose own click would otherwise activate the notification.
pub fn tap(
    mut handler: impl FnMut(&mut Cx) + 'static,
) -> impl FnMut(&mut Cx, &mut Event<ClickEvent>) {
    move |cx, event| {
        event.stop_propagation();
        handler(cx);
    }
}
