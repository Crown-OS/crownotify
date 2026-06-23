use iced_layershell::to_layer_message;

use crate::models::{call::CallNotification, Notification};

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    ToggleDoNotDisturb,
    ShowAllNotifications,
    ShowNotification(Notification),
    PickupCall(String),
    DeclineCall(String),
}

#[derive(Default, Debug, Clone)]
pub struct NotificationUIState {
    pub do_not_disturb: bool,
    pub notifications: Vec<Notification>,
}

pub fn update(state: &mut NotificationUIState, message: Message) {
    match message {
        Message::ToggleDoNotDisturb => {
            state.do_not_disturb = !state.do_not_disturb;
        }
        Message::ShowAllNotifications => {}
        Message::ShowNotification(notification) => {
            if !state.do_not_disturb {
                state.notifications.push(notification);
            }
        }
        Message::PickupCall(call_id) => {
            smol::spawn(async move {
                let _ = CallNotification::pickup(&call_id).await;
            })
            .detach();
        }
        Message::DeclineCall(call_id) => {
            smol::spawn(async move {
                let _ = CallNotification::decline(&call_id).await;
            })
            .detach();
        }
        _ => {}
    }
}
