//! Which device a notification was mirrored from.
//!
//! crownotify shows the notifications of the machine it runs on *and* the ones
//! crownconnect relays from a paired device, and a card that does not say which
//! is which is quietly misleading: a message that has already been answered on
//! the phone looks exactly like one that has not. The badge the icon wears is
//! the whole answer, so all the model records is what to draw on it and what
//! the device calls itself.

/// The kind of device a notification was relayed from, which selects the glyph
/// drawn on the icon badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum DeviceKind {
    #[default]
    Phone,
    Tablet,
    Watch,
    Computer,
}

impl DeviceKind {
    /// Reads the kind another process sent, in whatever case it sent it.
    ///
    /// Anything unrecognised — an empty string, a spelling nobody agreed on, a
    /// device type invented after this was written — is a [`Phone`], because a
    /// phone is what relays in practice and because the alternative is dropping
    /// the badge: learning "this came from another device" from a slightly
    /// wrong glyph beats not being told at all.
    ///
    /// [`Phone`]: DeviceKind::Phone
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "tablet" | "pad" => Self::Tablet,
            "watch" => Self::Watch,
            "computer" | "desktop" | "laptop" => Self::Computer,
            _ => Self::Phone,
        }
    }

    /// The spelling [`from_name`](Self::from_name) round-trips, which is what
    /// the IPC method and the `x-crownos.device-kind` hint carry.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Watch => "watch",
            Self::Computer => "computer",
        }
    }
}

/// The device a notification was mirrored from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelayDevice {
    pub kind: DeviceKind,
    /// What the device calls itself ("Ada's phone"). May be empty: the badge is
    /// drawn from the kind alone, and a peer that knows only that much is still
    /// worth believing.
    pub name: String,
}

impl RelayDevice {
    pub fn new(kind: DeviceKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
        }
    }

    /// Builds a device from the two `x-crownos.device-*` hints, either of which
    /// the sender may have left out.
    ///
    /// `None` only when neither was sent, which is the ordinary local
    /// notification. A name on its own is enough to badge the card — the sender
    /// said it was relayed and simply did not say by what — and so is a kind on
    /// its own.
    pub fn from_parts(kind: Option<&str>, name: Option<&str>) -> Option<Self> {
        if kind.is_none() && name.is_none() {
            return None;
        }
        Some(Self::new(
            kind.map(DeviceKind::from_name).unwrap_or_default(),
            name.unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_kinds_are_read_in_whatever_case_they_arrive() {
        assert_eq!(DeviceKind::from_name("phone"), DeviceKind::Phone);
        assert_eq!(DeviceKind::from_name("Tablet"), DeviceKind::Tablet);
        assert_eq!(DeviceKind::from_name(" WATCH "), DeviceKind::Watch);
        assert_eq!(DeviceKind::from_name("laptop"), DeviceKind::Computer);
    }

    #[test]
    fn an_unknown_device_kind_still_badges_the_card_as_a_phone() {
        for name in ["", "toaster", "iPhone 17", "🙂"] {
            assert_eq!(
                DeviceKind::from_name(name),
                DeviceKind::Phone,
                "{name:?} should degrade to a phone rather than losing the badge"
            );
        }
        assert_eq!(DeviceKind::default(), DeviceKind::Phone);
    }

    #[test]
    fn every_kinds_name_parses_back_to_itself() {
        for kind in [
            DeviceKind::Phone,
            DeviceKind::Tablet,
            DeviceKind::Watch,
            DeviceKind::Computer,
        ] {
            assert_eq!(
                DeviceKind::from_name(kind.name()),
                kind,
                "{kind:?} does not survive a round trip through {:?}",
                kind.name()
            );
        }
    }

    #[test]
    fn a_notification_with_neither_hint_was_not_relayed() {
        assert_eq!(RelayDevice::from_parts(None, None), None);
    }

    #[test]
    fn either_hint_on_its_own_is_enough_to_badge_the_card() {
        let named = RelayDevice::from_parts(None, Some("Ada's phone")).expect("a name is enough");
        assert_eq!(named.kind, DeviceKind::Phone, "the kind falls back");
        assert_eq!(named.name, "Ada's phone");

        let kinded = RelayDevice::from_parts(Some("watch"), None).expect("a kind is enough");
        assert_eq!(kinded.kind, DeviceKind::Watch);
        assert!(kinded.name.is_empty());

        assert_eq!(
            RelayDevice::from_parts(Some("tablet"), Some("Ada's pad")),
            Some(RelayDevice::new(DeviceKind::Tablet, "Ada's pad"))
        );
    }
}
