//! How loudly a notification asks to interrupt, as distinct from how urgent its
//! sender thinks it is.
//!
//! [`Urgency`] is what the freedesktop spec gives us and what goes back out on
//! the wire. A level is what the daemon presents and files on. They are kept
//! apart because the spec has three values chosen for a 2004 mail client, and a
//! focus mode needs to tell "you asked to be told about this kind of thing"
//! from "this is loud".

use std::{borrow::Cow, fmt, str::FromStr};

use serde::{Deserialize, Deserializer};

use crate::{error::Error, model::Urgency};

/// Ascending, and load-bearing: the whole breakthrough test a focus mode
/// applies is `level >= mode.minimum_level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub enum InterruptionLevel {
    /// Filed in the center and never toasted, whatever is or is not in force.
    Passive,
    #[default]
    Active,
    /// Breaks through a focus mode that asks for it by name.
    TimeSensitive,
    /// Breaks through everything. No configuration can suppress it.
    Critical,
}

impl InterruptionLevel {
    /// The level a notification that only speaks freedesktop is judged at.
    pub const fn from_urgency(urgency: Urgency) -> Self {
        match urgency {
            Urgency::Low => Self::Passive,
            Urgency::Normal => Self::Active,
            Urgency::Critical => Self::Critical,
        }
    }

    /// The uppercase label the card wears, if this level earns one.
    ///
    /// `Active` earns nothing because badging it would badge nearly every
    /// notification on the system, which is the same as badging none; `Passive`
    /// earns nothing because it never reaches a card that could show it.
    pub const fn eyebrow(self) -> Option<&'static str> {
        match self {
            Self::Critical => Some("CRITICAL"),
            Self::TimeSensitive => Some("TIME SENSITIVE"),
            Self::Active | Self::Passive => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passive => "passive",
            Self::Active => "active",
            Self::TimeSensitive => "time-sensitive",
            Self::Critical => "critical",
        }
    }
}

impl fmt::Display for InterruptionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for InterruptionLevel {
    type Err = Error;

    /// Tolerant about separators and case, because the same spelling has to
    /// survive a TOML file, a D-Bus hint and whatever a sender's own constant
    /// happened to be called.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let folded: String = raw
            .chars()
            .filter(|character| !matches!(character, '-' | '_' | ' '))
            .flat_map(char::to_lowercase)
            .collect();
        match folded.as_str() {
            "passive" => Ok(Self::Passive),
            "active" => Ok(Self::Active),
            "timesensitive" => Ok(Self::TimeSensitive),
            "critical" => Ok(Self::Critical),
            _ => Err(Error::UnknownInterruptionLevel(raw.to_string())),
        }
    }
}

impl<'de> Deserialize<'de> for InterruptionLevel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // `Cow` rather than `&str`: TOML cannot always hand back a borrowed
        // string, and a level that fails to deserialize costs the whole file.
        let raw = Cow::<'de, str>::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_order_passive_below_critical() {
        assert!(InterruptionLevel::Passive < InterruptionLevel::Active);
        assert!(InterruptionLevel::Active < InterruptionLevel::TimeSensitive);
        assert!(InterruptionLevel::TimeSensitive < InterruptionLevel::Critical);
    }

    #[test]
    fn every_urgency_maps_onto_a_level() {
        assert_eq!(
            InterruptionLevel::from_urgency(Urgency::Low),
            InterruptionLevel::Passive
        );
        assert_eq!(
            InterruptionLevel::from_urgency(Urgency::Normal),
            InterruptionLevel::Active
        );
        assert_eq!(
            InterruptionLevel::from_urgency(Urgency::Critical),
            InterruptionLevel::Critical
        );
    }

    #[test]
    fn a_level_is_parsed_whatever_separator_the_sender_used() {
        for spelling in [
            "time-sensitive",
            "time_sensitive",
            "TimeSensitive",
            "Time Sensitive",
        ] {
            assert_eq!(
                spelling.parse::<InterruptionLevel>().ok(),
                Some(InterruptionLevel::TimeSensitive),
                "{spelling:?}"
            );
        }
    }

    #[test]
    fn an_unrecognised_level_is_an_error_rather_than_a_silent_default() {
        assert!("urgent".parse::<InterruptionLevel>().is_err());
        assert!("".parse::<InterruptionLevel>().is_err());
    }

    #[test]
    fn only_the_two_loud_levels_carry_an_eyebrow() {
        assert_eq!(InterruptionLevel::Critical.eyebrow(), Some("CRITICAL"));
        assert_eq!(
            InterruptionLevel::TimeSensitive.eyebrow(),
            Some("TIME SENSITIVE")
        );
        assert_eq!(InterruptionLevel::Active.eyebrow(), None);
        assert_eq!(InterruptionLevel::Passive.eyebrow(), None);
    }

    #[test]
    fn a_level_round_trips_through_its_own_spelling() {
        for level in [
            InterruptionLevel::Passive,
            InterruptionLevel::Active,
            InterruptionLevel::TimeSensitive,
            InterruptionLevel::Critical,
        ] {
            assert_eq!(
                level.as_str().parse::<InterruptionLevel>().ok(),
                Some(level)
            );
        }
    }
}
