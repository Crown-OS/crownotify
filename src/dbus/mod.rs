//! The session-bus transport.
//!
//! Only `org.freedesktop.Notifications` lives here now. The custom
//! `io.crownos.crownotify` interface this crate used to also export is gone:
//! everything inside CrownOS talks over crownos-ipc instead, which is typed,
//! brokerless and an order of magnitude cheaper per message. What remains is
//! the interface crownotify does not get to choose — the one every third-party
//! application on the system already speaks.

pub mod freedesktop;

pub use freedesktop::{
    BUS_NAME, Notifications, OBJECT_PATH, emit_activation_token, emit_signals, serve,
};
