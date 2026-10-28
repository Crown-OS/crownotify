//! Which notifications get through, and what becomes of the ones that do not.
//!
//! Policy only: nothing here touches a store, a surface or a transport, so the
//! whole decision table is exercised by handing it a level and an app key. What
//! a blocked notification is *held in* is the store's business — see
//! [`Store`](crate::store::Store) — because holding one means owning it, and a
//! module that owned notifications could not stay this cheap to test.

pub mod mode;
pub mod policy;

pub use mode::{AppPolicy, Blocked, FocusMode, FocusModes};
pub use policy::{FocusPolicy, Outcome};
