//! The presentation layer: two crownui windows over the daemon's snapshots.
//!
//! Nothing here reads a socket or decides policy. A window is handed
//! [`Snapshot`](crate::daemon::Snapshot)s and draws them; what it produces from
//! a click is a [`Request`](crate::daemon::Request) naming an intent, which the
//! core turns into a store mutation or a phone command.

pub mod action;
pub mod board;
pub mod card;
pub mod center;
pub mod deck;
pub mod env;
pub mod feed;
pub mod head;
pub mod motion;
pub mod popup;
pub mod stack;
pub mod theme;
pub mod tile;
pub mod widget;
pub mod window;

pub use action::{CardAction, MediaAction};
pub use board::{BoardHandle, Frame, Side};
pub use card::relative_time;
pub use deck::{Deck, Pose};
pub use motion::{CardMotion, Phase};
pub use stack::{Placement, Stack, card_left, card_width};
pub use theme::{Appearance, Metrics, Palette, Theme, Typography};
pub use window::Window;
