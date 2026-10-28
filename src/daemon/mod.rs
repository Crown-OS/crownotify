//! The daemon's core: the store and everything that may change it, on a calloop
//! loop of its own. The windows follow it through [`Snapshot`]s and talk back
//! with [`Request`]s.

pub mod request;
pub mod snapshot;
pub mod state;
pub mod thread;

pub use request::{Request, RequestSender};
pub use snapshot::Snapshot;
pub use state::{Daemon, DaemonState, phone_command, reply_command};
pub use thread::{Core, spawn};
