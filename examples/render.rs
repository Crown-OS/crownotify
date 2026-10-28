//! Renders a window offscreen, with one notification of every kind, to
//! `toasts.png` or `center.png` in the directory given (default `.`).
//!
//! `cargo run --example render -- /tmp/crownotify [toasts|center]`

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use chrono::{Duration, Local};
use crownotify::{
    daemon::Snapshot,
    model::{
        Action, ActionKind, Call, Chat, Content, DeviceKind, InterruptionLevel, Music,
        Notification, Progress, RelayDevice,
    },
    profile::table::Profiles,
    ui::{Theme, Window, center, popup},
};
use crownui::{
    ext::TextSystem,
    impeller::{self, ImpellerBackend, Offscreen},
    kit,
    prelude::{Size, View},
    runtime::InlineApp,
    taffy_layout::TaffyEngine,
};
use tokio::sync::watch;

const SCALE: f32 = 2.0;

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments.next().unwrap_or_else(|| ".".into());
    let directory = Path::new(&directory);
    let notifications = samples();
    match arguments.next().as_deref() {
        Some("center") => render(
            center::center,
            Snapshot {
                history: notifications.into_iter().rev().collect(),
                center_open: true,
                do_not_disturb: true,
                ..Snapshot::default()
            },
            Size::new(900.0, 1100.0),
            &directory.join("center.png"),
        ),
        _ => render(
            popup::popup,
            Snapshot {
                toasts: notifications,
                ..Snapshot::default()
            },
            Size::new(400.0, 1100.0),
            &directory.join("toasts.png"),
        ),
    }
}

fn render<V: View>(
    view: impl FnOnce(Window) -> V,
    snapshot: Snapshot,
    size: Size,
    path: &Path,
) -> Result<()> {
    let (requests, _received) = calloop::channel::channel();
    let (_publish, snapshots) = watch::channel(Arc::new(snapshot));
    let window = Window {
        theme: Theme::dark(),
        profiles: Profiles::default(),
        requests,
        snapshots,
    };
    let surface = Offscreen {
        width: (size.width * SCALE) as u32,
        height: (size.height * SCALE) as u32,
        scale: SCALE,
    };
    let mut app = InlineApp::<ImpellerBackend, TaffyEngine>::with_surface(
        view(window),
        TaffyEngine::new(),
        size,
        SCALE,
        &surface,
    )
    .context("creating the offscreen renderer")?;
    app.text_cache()
        .system_mut()
        .register_font(kit::inter_font().to_vec(), Some(kit::INTER_FAMILY))?;
    app.ui().set_emulate_materials(true);
    app.settle()?;
    let pixels = app.renderer_mut().read_pixels()?;
    std::fs::write(path, impeller::encode_png(&pixels)?)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

fn samples() -> Vec<Notification> {
    let now = Local::now();
    vec![
        Notification::builder("Calendar")
            .id(1)
            .summary("Design review in 10 minutes")
            .body("Room 4B, with the whole shell team. Bring the motion studies.")
            .level(InterruptionLevel::TimeSensitive)
            .action(button("snooze", "Snooze"))
            .action(button("join", "Join"))
            .action(button("decline", "Decline"))
            .action(button("later", "Remind me later"))
            .build(),
        Notification::builder("Messages")
            .id(2)
            .summary("Ada Lovelace")
            .body("Are we still on for five? I found the notes on the analytical engine.")
            .content(Content::Chat(Chat {
                conversation_id: "thread-7".into(),
                sender: "Ada Lovelace".into(),
                avatar: None,
                sent_at: now - Duration::minutes(3),
            }))
            .action(Action {
                key: "inline-reply".into(),
                label: "Reply".into(),
                kind: ActionKind::InlineReply {
                    placeholder: "Reply to Ada".into(),
                },
            })
            .relayed_from(RelayDevice::new(DeviceKind::Phone, "Pixel"))
            .build(),
        Notification::builder("Phone")
            .id(3)
            .content(Content::Call(Call {
                call_id: "call-1".into(),
                contact: "Grace Hopper".into(),
                number: "+1 555 0100".into(),
                avatar: None,
            }))
            .relayed_from(RelayDevice::new(DeviceKind::Phone, "Pixel"))
            .build(),
        Notification::builder("Player")
            .id(4)
            .content(Content::Music(Music {
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                album_art: None,
                playing: true,
                percent: 42,
            }))
            .build(),
        Notification::builder("Files")
            .id(5)
            .summary("Copying photos")
            .content(Content::Progress(Progress {
                label: "312 of 840 items".into(),
                percent: 37,
            }))
            .build(),
        Notification::builder("Mail")
            .id(6)
            .summary("Quarterly figures")
            .body("The numbers are in and they look good.")
            .build(),
        Notification::builder("Mail")
            .id(7)
            .summary("Lunch on Friday?")
            .body("The usual place, half twelve.")
            .build(),
    ]
}

fn button(key: &str, label: &str) -> Action {
    Action {
        key: key.into(),
        label: label.into(),
        kind: ActionKind::Button,
    }
}
