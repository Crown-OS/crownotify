//! The panel a card's surplus actions open into.
//!
//! A sender can attach more actions than the card is wide. The ones that fit
//! get a pill; the rest go here, because dropping them silently is how an app's
//! "Snooze" stops existing.

use crownui::{
    ext::Kind,
    prelude::{CursorIcon, Element, Listeners, Role, Semantic, Styled, hstack, text, vstack},
};

use crate::{
    daemon::Request,
    model::{Action, NotificationId},
    ui::{action::CardAction, env::Env, widget::tap::tap},
};

const ROW_HEIGHT: f32 = 30.0;
const ROW_PAD_X: f32 = 12.0;
const PAD_Y: f32 = 4.0;
const RADIUS: f32 = 10.0;
const ROW_RADIUS: f32 = 6.0;

/// Every action in `items`, one row each; a click invokes it on `id`.
pub fn menu(env: Env, id: NotificationId, items: Vec<Action>) -> Element<impl Kind> {
    let theme = env.theme;
    let rows: Vec<_> = items
        .into_iter()
        .map(|Action { key, label, .. }| {
            hstack((theme.text.body.apply(text(label.clone()).truncate()),))
                .h(ROW_HEIGHT)
                .px(ROW_PAD_X)
                .items_center()
                .rounded(ROW_RADIUS)
                .cursor(CursorIcon::Pointer)
                .hover(move |variant| variant.bg(theme.palette.separator))
                .role(Role::Button)
                .label(label)
                .on_click(tap(move |cx| {
                    env.send(cx, Request::Perform(id, CardAction::Invoke(key.clone())))
                }))
        })
        .collect();
    vstack(rows)
        .py(PAD_Y)
        .rounded(RADIUS)
        .bg(theme.palette.field)
}
