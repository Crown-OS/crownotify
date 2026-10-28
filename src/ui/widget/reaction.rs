//! Runs a callback with the full context whenever a reactive value changes.
//!
//! Effects only get the reactive runtime, but scheduling an animation frame or
//! changing the window's keyboard focus needs [`Cx`]. `keyed` is the one place
//! crownui hands a `Cx` to code that re-runs on a signal, so a reaction is a
//! `keyed` whose branch is an empty, unlaid box.

use crownui::{
    ext::Kind,
    prelude::{Cx, Element, Runtime, Styled, hstack, keyed},
};

/// Calls `act` with every new value `read` produces, the first one included.
pub fn reaction<T, R, A>(read: R, mut act: A) -> Element<impl Kind>
where
    T: Clone + PartialEq + 'static,
    R: FnMut(&mut Runtime) -> T + 'static,
    A: FnMut(&mut Cx, T) + 'static,
{
    keyed(read, move |cx, value| {
        act(cx, value);
        hstack(())
    })
    .absolute()
    .size(0.0)
    .pointer_events_none()
}
