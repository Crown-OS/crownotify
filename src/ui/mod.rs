//! The presentation layer: tokens, animation state, hit testing and the
//! widgets the cards are assembled from.
//!
//! Nothing here reads a socket or decides policy. A card is handed an [`Entry`]
//! and a [`Theme`] and paints it; what it produces from a click is a
//! [`CardAction`], which names an intent in the UI's own vocabulary and not a
//! transport type. The surface layer is what turns that intent back into a
//! store mutation or a phone command, which is what keeps `ui` testable with
//! neither a compositor nor a bus.
//!
//! [`Entry`]: crate::store::Entry

pub mod animation;
pub mod card;
pub mod deck;
pub mod hit;
pub mod layout;
pub mod theme;
pub mod widget;

pub use animation::{CardAnimation, Phase};
pub use card::{Card, MeasureCtx, RenderCtx, relative_time, resolve_hit, write_relative_time};
pub use deck::{Deck, Pose};
pub use hit::{CardAction, HitMap, HitRegion, MediaAction};
pub use layout::{Placement, Stack, card_left, card_width};
pub use theme::{Appearance, Metrics, Palette, Theme, Typography};
pub use widget::{FieldEvent, TextField};
