//! What a click on a card means.

/// What a click on a card means, in the UI's own vocabulary.
///
/// Deliberately names no transport type: the daemon translates
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

#[cfg(test)]
mod tests {
    use super::*;

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
