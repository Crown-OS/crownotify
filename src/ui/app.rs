use std::sync::{Mutex, OnceLock};

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::futures::{Stream, StreamExt};
use iced::{Color, Subscription};
use iced_layershell::application;
use iced_layershell::reexport::{Anchor, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

use crate::models::Notification;
use crate::ui::state::{update, Message, NotificationUIState};
use crate::ui::view::view;

static NOTIFICATION_RX: OnceLock<Mutex<Option<UnboundedReceiver<Notification>>>> = OnceLock::new();

pub fn create_window(
    rx: UnboundedReceiver<Notification>,
) -> Result<(), iced_layershell::Error> {
    NOTIFICATION_RX
        .set(Mutex::new(Some(rx)))
        .map_err(|_| ())
        .expect("create_window must only be called once");

    let binded_output_name = std::env::args().nth(1);
    let start_mode = match binded_output_name {
        Some(output) => StartMode::TargetScreen(output),
        None => StartMode::Active,
    };

    application(|| NotificationUIState::default(), namespace, update, view)
        .subscription(dbus_subscription)
        .style(style)
        .settings(Settings {
            layer_settings: LayerShellSettings {
                size: Some((400, 0)),
                layer: Layer::Top,
                anchor: Anchor::Right | Anchor::Top | Anchor::Bottom,
                start_mode,
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

fn dbus_subscription(_state: &NotificationUIState) -> Subscription<Message> {
    Subscription::run(notification_stream)
}

fn notification_stream() -> impl Stream<Item = Message> {
    let rx = NOTIFICATION_RX
        .get()
        .and_then(|cell| cell.lock().ok().and_then(|mut guard| guard.take()))
        .expect("notification receiver was not initialized or already taken");
    rx.map(Message::ShowNotification)
}

fn namespace() -> String {
    "Crownotify".to_string()
}

fn style(_state: &NotificationUIState, theme: &iced::Theme) -> iced::theme::Style {
    use iced::theme::Style;
    Style {
        background_color: Color::TRANSPARENT,
        text_color: theme.palette().text,
    }
}
