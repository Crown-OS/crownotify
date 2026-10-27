//! crownotify — the CrownOS notification server.
//!
//! The daemon is split into three layers that never reach across each other:
//!
//! * [`model`] and [`store`] are the pure core. They own what a notification
//!   *is* and which ones are currently on screen, and they depend on nothing
//!   but `chrono` and the standard library, so every rule the daemon enforces
//!   is testable without a compositor, a GPU or a bus.
//! * [`dbus`] and [`ipc`] are the transports. They translate
//!   `org.freedesktop.Notifications` calls and crownos-ipc requests into the
//!   core's vocabulary and publish the results back out.
//! * [`ui`], [`surface`], [`icons`] and [`profile`] are the presentation. They
//!   read the store and paint it; they never decide policy.
//!
//! The store lives on the compositor's event-loop thread and is never locked.
//! The only state shared with the D-Bus thread is the notification id counter
//! (see [`Store::id_counter`]), because ids must be handed back to the caller
//! of `Notify` synchronously, before the UI thread has seen the request.

pub mod proto;

// Everything below is the daemon. A client that only speaks the protocol
// builds none of it — see the `daemon` feature in Cargo.toml.
#[cfg(feature = "daemon")]
pub mod config;
#[cfg(feature = "daemon")]
pub mod dbus;
#[cfg(feature = "daemon")]
pub mod error;
#[cfg(feature = "daemon")]
pub mod focus;
#[cfg(feature = "daemon")]
pub mod icons;
#[cfg(feature = "daemon")]
pub mod ipc;
#[cfg(feature = "daemon")]
pub mod launch;
#[cfg(feature = "daemon")]
pub mod model;
#[cfg(feature = "daemon")]
pub mod profile;
#[cfg(feature = "daemon")]
pub mod store;
#[cfg(feature = "daemon")]
pub mod surface;
#[cfg(feature = "daemon")]
pub mod ui;
#[cfg(feature = "daemon")]
pub mod util;

#[cfg(feature = "daemon")]
pub use config::{AppConfig, AppearancePreference, Config};
#[cfg(feature = "daemon")]
pub use error::{Error, Result};
#[cfg(feature = "daemon")]
pub use focus::{Blocked, FocusMode, FocusModes, Outcome};
#[cfg(feature = "daemon")]
pub use model::{
    Action, ActionKind, AppIdentity, Call, Chat, CloseReason, Content, DeviceKind, Icon, ImageData,
    InterruptionLevel, Music, Notification, NotificationBuilder, NotificationId, Progress,
    RelayDevice, Timeout, Urgency,
};
#[cfg(feature = "daemon")]
pub use store::{Entry, Group, Store, StoreConfig, allocate_id};
