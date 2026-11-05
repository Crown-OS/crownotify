//! The spinner an indeterminate progress notification shows.

use crownui::{
    kit,
    prelude::{Styled, View, component},
};

use crate::ui::theme::Theme;

/// Diameter of the spinner, about the height of a line of body copy.
pub const SPINNER_DIAMETER: f32 = 16.0;

/// crownui's spinner, painted with the accent and track the progress bar uses
/// rather than the kit's own palette.
pub fn spinner(theme: Theme) -> impl View {
    component(move |cx| {
        let palette = theme.palette;
        let base = kit::Theme::DEFAULT;
        kit::provide_theme(
            cx,
            kit::Theme {
                accent: kit::GradientStops::new(palette.accent, palette.accent),
                control: kit::ControlColors {
                    track: palette.separator,
                    ..base.control
                },
                ..base
            },
        );
        kit::spinner().size(SPINNER_DIAMETER)
    })
}
