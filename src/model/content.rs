//! The payload that decides which card a notification is drawn as.
//!
//! A notification is a summary and a body first; [`Content`] is the extra
//! structure a card needs when it is something richer — an incoming call needs
//! a call id to hang up with, a music card needs a scrub position. Senders opt
//! in through the `x-crownos.*` category or an app profile, so a plain app
//! always lands on [`Content::General`] and never has to know about any of
//! this.

use chrono::{DateTime, Local};

use crate::model::icon::Icon;

/// The shape of the card, and the data only that shape needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Content {
    #[default]
    General,
    Call(Call),
    Chat(Chat),
    Music(Music),
    Progress(Progress),
}

/// An incoming phone call relayed by crownconnect.
///
/// `call_id` is what the pickup and decline buttons send back, so it has to
/// survive a replacement of the notification untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub call_id: String,
    pub contact: String,
    pub number: String,
    pub avatar: Option<Icon>,
}

/// A message in a conversation, which is what makes inline reply meaningful:
/// `conversation_id` is the address the reply is sent to.
#[derive(Debug, Clone, PartialEq)]
pub struct Chat {
    pub conversation_id: String,
    pub sender: String,
    pub avatar: Option<Icon>,
    /// When the message was sent, which is not when we were told about it.
    pub sent_at: DateTime<Local>,
}

/// Now-playing state. The card is a remote control, so it holds the transport
/// state rather than deriving it.
#[derive(Debug, Clone, PartialEq)]
pub struct Music {
    pub title: String,
    pub artist: String,
    pub album_art: Option<Icon>,
    pub playing: bool,
    /// Position in the track, 0-100.
    pub percent: u8,
}

/// A long-running operation, drawn as a labelled bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub label: String,
    /// Completion, 0-100.
    pub percent: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_defaults_to_a_general_card() {
        assert!(matches!(Content::default(), Content::General));
    }

    #[test]
    fn a_call_payload_keeps_the_id_the_buttons_answer_with() {
        let content = Content::Call(Call {
            call_id: "call-42".into(),
            contact: "Ada".into(),
            number: "+44 7700 900000".into(),
            avatar: None,
        });
        let Content::Call(call) = content else {
            panic!("expected a call payload");
        };
        assert_eq!(call.call_id, "call-42");
        assert_eq!(call.contact, "Ada");
    }

    #[test]
    fn a_chat_payload_carries_its_conversation_for_replies() {
        let chat = Chat {
            conversation_id: "thread-7".into(),
            sender: "Ada".into(),
            avatar: Some(Icon::Named("avatar".into())),
            sent_at: Local::now(),
        };
        assert_eq!(chat.conversation_id, "thread-7");
        assert_eq!(chat.avatar, Some(Icon::Named("avatar".into())));
    }

    #[test]
    fn music_and_progress_payloads_survive_a_clone() {
        let music = Music {
            title: "Blue Monday".into(),
            artist: "New Order".into(),
            album_art: None,
            playing: true,
            percent: 33,
        };
        let copy = music.clone();
        assert_eq!(copy.title, music.title);
        assert!(copy.playing);
        assert_eq!(copy.percent, 33);

        let progress = Progress {
            label: "Copying".into(),
            percent: 100,
        };
        assert_eq!(progress.clone().percent, 100);
    }
}
