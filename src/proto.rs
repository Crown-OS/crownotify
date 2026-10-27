//! The wire contract of the `crownotify` service.
//!
//! Lives in the library target, outside the `daemon` feature, so that every
//! client — the bar's notification pill, a keybinding helper, crownconnect
//! relaying a call from the phone — is generated from the same declaration as
//! the daemon and cannot drift from it. A client builds this module and
//! nothing else: none of the rendering, decoding or D-Bus stack comes with it.
//!
//! The serving half (`Handler`, `dispatch`) costs a caller nothing, so both
//! halves are exported together.
//!
//! Ids are spelled `u32` rather than `NotificationId` deliberately: the alias
//! lives in [`crate::model`], which is behind `daemon`, and the wire type must
//! not depend on whether the daemon was built.

crownos_ipc::protocol! {
    pub mod protocol: "crownotify" {
        /// Show the notification center.
        notify open_notification_center();
        /// Hide the notification center.
        notify close_notification_center();
        /// Show the notification center, or hide it if it is already up.
        notify toggle_notification_center();
        /// Silence notifications, or stop silencing them. Turning this on is
        /// the built-in `do-not-disturb` focus mode; turning it off ends
        /// whichever mode is in force.
        notify set_do_not_disturb(enabled: bool);
        /// Put a named focus mode in force. An empty name ends the active one.
        notify set_focus_mode(name: String);
        /// Clear everything currently on screen and in the center.
        notify dismiss_all();
        /// Act on a notification as though its button had been clicked.
        notify invoke_action(id: u32, action: String);
        /// Dismiss one notification by the id it was posted under.
        notify close_notification(id: u32);

        /// Post a standard notification card.
        fn send_general_notification(
            app_name: String,
            summary: String,
            body: String,
            expire_timeout: u32,
            actions: Vec<String>,
        ) -> u32;
        /// Post an incoming-call card with answer/decline actions.
        fn send_call_notification(
            call_id: String,
            app_name: String,
            contact_avatar: String,
            contact_name: String,
            phone_number: String,
        ) -> u32;
        /// Post or update a now-playing card.
        fn send_music_notification(song_name: String, percentage: u8) -> u32;
        /// Post a chat message card, which grows an inline reply field.
        fn send_chat_notification(
            app_name: String,
            title: String,
            body: String,
            timestamp: String,
        ) -> u32;
        /// Post a card an app profile supplies the payload for.
        fn send_app_notification(app_id: String, kind: String, payload: Vec<u8>) -> u32;
        /// Post a card for a notification that arrived on another device.
        fn send_relayed_notification(
            device_kind: String,
            device_name: String,
            app_name: String,
            summary: String,
            body: String,
            expire_timeout: u32,
            actions: Vec<String>,
        ) -> u32;

        /// A notification left the screen (reason: 1 = expired, 2 = dismissed
        /// by the user, 3 = closed by `close_notification`).
        event NotificationClosed { id: u32, reason: u8 };
        /// The user invoked an action on a notification.
        event ActionInvoked { id: u32, action: String };
        /// The user sent an inline reply.
        event ReplySubmitted { id: u32, text: String };
        /// The notification center opened or closed.
        event CenterVisibility { open: bool };
        /// Do Not Disturb was turned on or off.
        event DoNotDisturbChanged { enabled: bool };
        /// The active focus mode changed. An empty name means none is in force.
        event FocusModeChanged { name: String };
    }
}

pub use protocol::*;
