use iced::{
    alignment::{Horizontal, Vertical},
    border::Radius,
    widget::{button, column, container, row, text, Space},
    Background, Border, Color, Element, Length, Shadow, Theme,
};

use crate::{models::call::CallNotification, ui::state::Message};

const BAR_BG: Color = Color::from_rgb(0.04, 0.04, 0.04);
const GREEN: Color = Color::from_rgb(0.20, 0.78, 0.35);
const RED: Color = Color::from_rgb(0.93, 0.30, 0.27);
const WHITE: Color = Color::WHITE;
const SUBTITLE: Color = Color::from_rgb(0.55, 0.55, 0.55);

pub fn call_notification_card(notification: &CallNotification) -> Element<'_, Message> {
    let avatar = container(text("\u{1F464}").size(28).color(WHITE))
        .width(Length::Fixed(56.0))
        .height(Length::Fixed(56.0))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(|_: &Theme| container::Style {
            background: None,
            border: Border {
                color: WHITE,
                width: 2.0,
                radius: Radius::from(28.0),
            },
            ..Default::default()
        });

    let info = column![
        text(&notification.contact_name).size(22).color(WHITE),
        text(&notification.phone_number).size(14).color(SUBTITLE),
    ]
    .spacing(2);

    let pickup_btn = button(
        container(text("\u{260E}").size(22).color(WHITE))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
    )
    .width(Length::Fixed(48.0))
    .height(Length::Fixed(48.0))
    .padding(0)
    .on_press(Message::PickupCall(notification.call_id.clone()))
    .style(|_: &Theme, _| button::Style {
        background: Some(Background::Color(GREEN)),
        text_color: WHITE,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(24.0),
        },
        shadow: Shadow::default(),
        snap: true,
    });

    let hangup_btn = button(
        container(text("\u{260E}").size(22).color(WHITE))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
    )
    .width(Length::Fixed(48.0))
    .height(Length::Fixed(48.0))
    .padding(0)
    .on_press(Message::DeclineCall(notification.call_id.clone()))
    .style(|_: &Theme, _| button::Style {
        background: Some(Background::Color(RED)),
        text_color: WHITE,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(24.0),
        },
        shadow: Shadow::default(),
        snap: true,
    });

    let content = row![
        avatar,
        Space::new().width(Length::Fixed(16.0)),
        info,
        Space::new().width(Length::Fill),
        pickup_btn,
        Space::new().width(Length::Fixed(12.0)),
        hangup_btn,
    ]
    .align_y(Vertical::Center)
    .padding([12, 20]);

    container(content)
        .width(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BAR_BG)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: Radius::from(40.0),
            },
            ..Default::default()
        })
        .into()
}
