use iced::widget::{button, column, container, row, text};
use iced::{Element, Length};

use crate::models::Notification;
use crate::ui::components::call::call_notification_card;
use crate::ui::state::{Message, NotificationUIState};

pub fn view(state: &NotificationUIState) -> Element<'_, Message> {
    let dnd_label = if state.do_not_disturb {
        "Do not Disturb: On"
    } else {
        "Do not Disturb: Off"
    };

    let toolbar = row![button(text(dnd_label)).on_press(Message::ToggleDoNotDisturb)];

    let mut list = column![].spacing(8);
    for notification in &state.notifications {
        list = list.push(notification_card(notification));
    }

    container(column![toolbar, list].spacing(12).padding(12))
        .width(Length::Fill)
        .into()
}

fn notification_card(notification: &Notification) -> Element<'_, Message> {
    let body: Element<'_, Message> = match notification {
        Notification::General(n) => column![
            text(&n.app_name).size(14),
            text(&n.summary).size(16),
            text(&n.body).size(13),
        ]
        .spacing(2)
        .into(),
        Notification::Call(notification) => call_notification_card(notification).into(),
        Notification::Music(n) => column![
            text("Now playing").size(14),
            text(&n.song_name).size(16),
            text(format!("{}%", n.percentage)).size(13),
        ]
        .spacing(2)
        .into(),
        Notification::Chat(n) => column![
            row![text(&n.app_name).size(14), text(&n.timestamp).size(12)].spacing(8),
            text(&n.title).size(16),
            text(&n.body).size(13),
        ]
        .spacing(2)
        .into(),
        Notification::Audio(_) => text("Audio notification").into(),
        Notification::Display(_) => text("Display notification").into(),
    };

    container(body).padding(10).width(Length::Fill).into()
}
