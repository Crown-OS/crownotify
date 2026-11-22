use std::{path::PathBuf, sync::Arc};

use chrono::{Duration, Local};
use crownotify::model::{
    Action, ActionKind, Call, Chat, Content, DeviceKind, Icon, ImageData, InterruptionLevel, Music,
    Notification, NotificationBuilder, Progress, RelayDevice,
};

const ICON_DIR: &str = "/usr/share/icons/Colloid-Light/apps/scalable";
const PICTURE_SIZE: u32 = 256;

fn app_icon(name: &str) -> Icon {
    Icon::Path(PathBuf::from(format!("{ICON_DIR}/{name}.svg")))
}

fn gradient(from: [u8; 3], to: [u8; 3]) -> Icon {
    let span = (2 * (PICTURE_SIZE - 1)) as f32;
    let rgba = (0..PICTURE_SIZE)
        .flat_map(|y| (0..PICTURE_SIZE).map(move |x| (x + y) as f32 / span))
        .flat_map(|t| {
            let mix = |channel: usize| {
                (from[channel] as f32 + (to[channel] as f32 - from[channel] as f32) * t) as u8
            };
            [mix(0), mix(1), mix(2), u8::MAX]
        })
        .collect();
    Icon::Data(Arc::new(ImageData {
        width: PICTURE_SIZE,
        height: PICTURE_SIZE,
        rgba,
    }))
}

fn button(key: &str, label: &str) -> Action {
    Action {
        key: key.into(),
        label: label.into(),
        kind: ActionKind::Button,
    }
}

fn minutes_ago(builder: NotificationBuilder, minutes: i64) -> Notification {
    let mut notification = builder.build();
    notification.posted = Local::now() - Duration::minutes(minutes);
    notification
}

pub fn mail(id: u32, summary: &str, body: &str, minutes: i64) -> Notification {
    minutes_ago(
        Notification::builder("Mail")
            .id(id)
            .icon(app_icon("thunderbird"))
            .summary(summary)
            .body(body),
        minutes,
    )
}

pub fn software_update(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Software")
            .id(id)
            .icon(app_icon("org.gnome.Software"))
            .summary("Updates ready to install")
            .body("CrownOS 1.4 and 12 app updates are downloaded. Restart to finish installing.")
            .action(button("restart", "Restart"))
            .action(button("later", "Later")),
        0,
    )
}

pub fn calendar(id: u32, level: InterruptionLevel) -> Notification {
    minutes_ago(
        Notification::builder("Calendar")
            .id(id)
            .icon(app_icon("org.gnome.Calendar"))
            .summary("Design review in 10 minutes")
            .body("Room 4B with the whole shell team. Bring the motion studies.")
            .level(level)
            .action(button("snooze", "Snooze"))
            .action(button("join", "Join"))
            .action(button("decline", "Decline"))
            .action(button("later", "Remind me later")),
        0,
    )
}

pub fn screenshot(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Screenshot")
            .id(id)
            .icon(app_icon("org.gnome.Screenshot"))
            .image(gradient([88, 86, 214], [255, 159, 10]))
            .summary("Screenshot captured")
            .body("Saved to Pictures › Screenshots. Click to open it in the editor."),
        0,
    )
}

pub fn battery(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Power")
            .id(id)
            .icon(app_icon("org.gnome.Settings"))
            .summary("Battery critically low")
            .body("4% remaining. Plug in now or CrownOS will suspend in 2 minutes.")
            .level(InterruptionLevel::Critical),
        0,
    )
}

pub fn copying(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Files")
            .id(id)
            .icon(app_icon("org.gnome.Nautilus"))
            .summary("Copying photos to Backup")
            .content(Content::Progress(Progress {
                label: "312 of 840 items · about 2 minutes left".into(),
                percent: 37,
            })),
        1,
    )
}

pub fn message(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Messages")
            .id(id)
            .icon(app_icon("signal-desktop"))
            .summary("Ada Lovelace")
            .body("Are we still on for five? I found the notes on the analytical engine.")
            .content(Content::Chat(Chat {
                conversation_id: "thread-7".into(),
                sender: "Ada Lovelace".into(),
                avatar: Some(gradient([255, 105, 97], [191, 90, 242])),
                sent_at: Local::now() - Duration::minutes(3),
            }))
            .action(Action {
                key: "inline-reply".into(),
                label: "Reply".into(),
                kind: ActionKind::InlineReply {
                    placeholder: "Reply to Ada".into(),
                },
            })
            .relayed_from(RelayDevice::new(DeviceKind::Phone, "Pixel")),
        3,
    )
}

pub fn call(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Phone")
            .id(id)
            .content(Content::Call(Call {
                call_id: "call-1".into(),
                contact: "Grace Hopper".into(),
                number: "+1 555 0100".into(),
                avatar: Some(gradient([48, 209, 88], [10, 132, 255])),
            }))
            .relayed_from(RelayDevice::new(DeviceKind::Phone, "Pixel")),
        0,
    )
}

pub fn now_playing(id: u32) -> Notification {
    minutes_ago(
        Notification::builder("Music")
            .id(id)
            .icon(app_icon("org.gnome.Music"))
            .content(Content::Music(Music {
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                album_art: Some(gradient([10, 132, 255], [94, 92, 230])),
                playing: true,
                percent: 42,
            })),
        0,
    )
}

pub fn mail_thread() -> Vec<Notification> {
    vec![
        mail(
            21,
            "Lunch on Friday?",
            "The usual place, half twelve. I booked a table for six.",
            9,
        ),
        mail(
            22,
            "Quarterly figures",
            "The numbers are in and they look good. Slides attached.",
            4,
        ),
        mail(
            23,
            "Grace Hopper",
            "Can you review the compiler notes before tomorrow's sync?",
            0,
        ),
    ]
}

pub fn history() -> Vec<Notification> {
    let mut history = vec![
        message(31),
        software_update(32),
        copying(33),
        screenshot(34),
        calendar(35, InterruptionLevel::TimeSensitive),
    ];
    history.extend(mail_thread().into_iter().rev());
    let now = Local::now();
    for (index, notification) in (0_i64..).zip(&mut history) {
        notification.posted = now - Duration::minutes(2 + index * 6);
    }
    history
}
