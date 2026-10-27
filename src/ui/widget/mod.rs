//! The pieces a card is assembled from.
//!
//! These are drawing functions and small retained widgets, not a widget tree:
//! a card lays its own contents out, calls these to paint them and records the
//! rectangles it used in the [`HitMap`]. Anything that owns a shaped
//! [`Text`](crownshell::Text) — a button label, the reply field — is a struct
//! the card keeps alive, because re-shaping text every frame is the one cost
//! that would actually be felt at 144 Hz.
//!
//! [`HitMap`]: crate::ui::hit::HitMap

pub mod button;
pub mod menu;
pub mod progress;
pub mod surface;
pub mod text_field;

use crownshell::{Text, TextStyle};

/// Applies `style` to `text`, but only when it differs from what the text
/// already carries.
///
/// [`Text::set_style`] takes its style by value and a [`TextStyle`] owns its
/// family stack, so handing it one every frame means an allocation per label
/// per frame. Comparing first costs nothing — the styles are already there —
/// and leaves the cached layout alone in the overwhelmingly common case where
/// the theme has not moved.
pub fn sync_style(text: &mut Text, style: &TextStyle) {
    if text.style() != style {
        text.set_style(style.clone());
    }
}

pub use button::{ButtonState, ButtonStyle, IconButton, PillButton, Symbol, draw_symbol};
pub use menu::{Menu, chevron_state};
pub use progress::{
    KNOB_DIAMETER, TRACK_HEIGHT, draw_bar, draw_scrubber, percent_fraction, seek_fraction,
    seek_percent,
};
pub use surface::{
    HAIRLINE, Painted, draw_card, draw_shadow, fill_rounded, multiply_alpha, rounded_bands,
    stroke_rounded,
};
pub use text_field::{FieldEvent, TextField};
