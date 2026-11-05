//! What a progress notification draws under its text: a bar while it knows
//! how far it has got, a spinner while it does not.

use crownui::prelude::{Signal, Styled, View, hstack, keyed};

use crate::{
    model::{Content, Notification},
    ui::{
        theme::Theme,
        widget::{bar, percent_fraction, spinner},
    },
};

/// Gap above the indicator.
const PROGRESS_GAP: f32 = 8.0;
/// Height of the band a progress bar is centred in.
const PROGRESS_BAND: f32 = 10.0;

/// Which indicator a progress notification shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    Spinner,
    Bar,
}

impl Indicator {
    /// `None` for anything that is not a progress notification.
    pub fn of(notification: &Notification) -> Option<Self> {
        match &notification.content {
            Content::Progress(progress) => Some(match progress.percent {
                Some(_) => Self::Bar,
                None => Self::Spinner,
            }),
            _ => None,
        }
    }
}

/// How much of the work is done, nothing when nobody said.
fn completion(notification: &Notification) -> u8 {
    match &notification.content {
        Content::Progress(progress) => progress.percent.unwrap_or_default(),
        _ => 0,
    }
}

/// The indicator under a card's text.
///
/// Keyed on the kind of indicator alone: a new percentage only moves the bar,
/// and a switch between spinner and bar rebuilds this one row, never the card
/// around it.
pub fn indicator(theme: Theme, notification: Signal<Notification>) -> impl View {
    let palette = theme.palette;
    keyed(
        move |runtime| notification.with(runtime, Indicator::of),
        move |_, shown| match shown {
            Some(Indicator::Bar) => bar(
                move |runtime| notification.with(runtime, |n| percent_fraction(completion(n))),
                palette.separator,
                palette.accent,
            )
            .h(PROGRESS_BAND)
            .mt(PROGRESS_GAP)
            .boxed(),
            Some(Indicator::Spinner) => hstack((spinner(theme),)).mt(PROGRESS_GAP).boxed(),
            None => hstack(()).boxed(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Progress;

    fn progress(percent: Option<u8>) -> Notification {
        Notification::builder("Android apps")
            .content(Content::Progress(Progress {
                label: "Downloading".into(),
                percent,
            }))
            .build()
    }

    #[test]
    fn an_unknown_completion_spins_and_a_known_one_fills_a_bar() {
        assert_eq!(Indicator::of(&progress(None)), Some(Indicator::Spinner));
        assert_eq!(Indicator::of(&progress(Some(0))), Some(Indicator::Bar));
        assert_eq!(Indicator::of(&progress(Some(100))), Some(Indicator::Bar));
    }

    #[test]
    fn a_new_percentage_keeps_the_indicator_it_already_has() {
        assert_eq!(
            Indicator::of(&progress(Some(10))),
            Indicator::of(&progress(Some(90))),
            "the bar would be rebuilt on every update otherwise"
        );
    }

    #[test]
    fn a_notification_that_is_not_progress_shows_no_indicator() {
        assert_eq!(Indicator::of(&Notification::builder("Mail").build()), None);
    }

    #[test]
    fn the_bar_fills_to_the_reported_completion_and_a_spinner_to_none() {
        assert_eq!(completion(&progress(Some(64))), 64);
        assert_eq!(completion(&progress(None)), 0);
    }
}
