//! Pointer input that comes and goes with the state a view is in.

use crownui::{
    ext::{Modifier, StyleProp},
    prelude::{PointerEvents, Runtime},
};

/// Takes pointer events only while `when` holds. The window's input region is
/// built from the views that take them, so a view that stops taking them also
/// stops covering whatever is underneath.
pub fn takes_input(mut when: impl FnMut(&mut Runtime) -> bool + 'static) -> Modifier {
    Modifier::Bound(Box::new(move |runtime| {
        StyleProp::PointerEvents(if when(runtime) {
            PointerEvents::Auto
        } else {
            PointerEvents::None
        })
    }))
}
