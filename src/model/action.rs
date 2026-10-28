//! Actions, and the decoder for the way `Notify` carries them.

/// One thing the user can do about a notification.
///
/// `key` is what goes back over the wire in `ActionInvoked`; `label` is what
/// the button says. They are separate because the sender chooses both and only
/// the key is stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub key: String,
    pub label: String,
    pub kind: ActionKind,
}

/// How an action is presented, which the key decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    /// Invoked by clicking the card body. Key is `"default"` per the spec.
    Default,
    Button,
    /// Renders a text field; submitting emits a reply instead of an action.
    InlineReply {
        placeholder: String,
    },
}

/// The key the spec reserves for "clicking the notification itself".
const DEFAULT_KEY: &str = "default";
/// The de-facto key for inline reply, as used by Fedora's and KDE's servers.
const INLINE_REPLY_KEY: &str = "inline-reply";

impl Action {
    /// Decodes the flat `[key, label, key, label, …]` array the freedesktop
    /// `Notify` call carries, recognising `default` and `inline-reply`.
    ///
    /// The array comes from another process, so it may well be malformed: a
    /// trailing key with no label is dropped rather than panicking, and so is
    /// any pair with an empty key, since an action nobody can name is an
    /// action nobody can invoke.
    pub fn from_spec_pairs(pairs: &[String]) -> Vec<Action> {
        pairs
            .chunks_exact(2)
            .filter(|pair| !pair[0].is_empty())
            .map(|pair| {
                let key = &pair[0];
                let label = &pair[1];
                let kind = match key.as_str() {
                    DEFAULT_KEY => ActionKind::Default,
                    INLINE_REPLY_KEY => ActionKind::InlineReply {
                        placeholder: label.clone(),
                    },
                    _ => ActionKind::Button,
                };
                Action {
                    key: key.clone(),
                    label: label.clone(),
                    kind,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn ordinary_pairs_become_buttons_in_order() {
        let actions = Action::from_spec_pairs(&pairs(&["archive", "Archive", "trash", "Delete"]));

        assert_eq!(actions.len(), 2, "got {actions:?}");
        assert_eq!(actions[0].key, "archive");
        assert_eq!(actions[0].label, "Archive");
        assert_eq!(actions[0].kind, ActionKind::Button);
        assert_eq!(actions[1].key, "trash");
        assert_eq!(actions[1].label, "Delete");
    }

    #[test]
    fn the_default_key_becomes_the_default_action() {
        let actions = Action::from_spec_pairs(&pairs(&["default", "Open"]));
        assert_eq!(actions[0].kind, ActionKind::Default, "got {actions:?}");
        assert_eq!(actions[0].label, "Open");
    }

    #[test]
    fn the_inline_reply_key_carries_its_label_as_the_placeholder() {
        let actions = Action::from_spec_pairs(&pairs(&["inline-reply", "Reply to Ada"]));
        assert_eq!(
            actions[0].kind,
            ActionKind::InlineReply {
                placeholder: "Reply to Ada".to_string()
            },
            "got {actions:?}"
        );
    }

    #[test]
    fn a_trailing_unpaired_key_is_dropped_rather_than_panicking() {
        let actions = Action::from_spec_pairs(&pairs(&["default", "Open", "orphan"]));
        assert_eq!(actions.len(), 1, "got {actions:?}");
        assert_eq!(actions[0].key, "default");
    }

    #[test]
    fn a_lone_key_produces_no_actions() {
        assert!(Action::from_spec_pairs(&pairs(&["orphan"])).is_empty());
    }

    #[test]
    fn an_empty_array_produces_no_actions() {
        assert!(Action::from_spec_pairs(&[]).is_empty());
    }

    #[test]
    fn pairs_with_an_empty_key_are_skipped() {
        let actions = Action::from_spec_pairs(&pairs(&["", "Nameless", "ok", "OK"]));
        assert_eq!(actions.len(), 1, "got {actions:?}");
        assert_eq!(actions[0].key, "ok");
    }

    #[test]
    fn an_empty_label_is_kept_because_only_the_key_has_to_be_usable() {
        let actions = Action::from_spec_pairs(&pairs(&["ok", ""]));
        assert_eq!(actions.len(), 1, "got {actions:?}");
        assert!(actions[0].label.is_empty());
    }
}
