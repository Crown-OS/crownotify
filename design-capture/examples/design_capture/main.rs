//! Captures every crownotify surface state for the CrownOS-Design Figma file. Run both appearances
//! with `../../crownui/capture/scripts/capture-app.sh . design_capture <output-dir>`.

mod fixtures;

use std::sync::Arc;

use anyhow::Result;
use calloop::channel::Channel;
use capture::{CaptureApp, Carrier};
use crownotify::{
    daemon::{Request, Snapshot},
    model::{InterruptionLevel, Notification},
    profile::table::Profiles,
    ui::{Appearance, Theme, Window, center, popup},
};
use crowntest::{Role, role};
use crownui::prelude::{AppConfig, View};
use tokio::sync::watch;

const STRIP: (f32, f32) = (400.0, 1080.0);
const OUTPUT: (f32, f32) = (1920.0, 1080.0);
const BANNERS: &str = "Banners";
const RICH: &str = "Rich cards";
const STACKS: &str = "Stacks";
const CENTER: &str = "Notification Center";

/// A mounted window and the daemon ends it needs kept alive.
struct Surface {
    app: CaptureApp,
    _requests: Channel<Request>,
    _publish: watch::Sender<Arc<Snapshot>>,
}

fn mount<V: View>(
    snapshot: Snapshot,
    view: impl FnOnce(Window) -> V,
    config: &AppConfig,
    size: (f32, f32),
) -> Result<Surface> {
    let (requests, received) = calloop::channel::channel();
    let (publish, snapshots) = watch::channel(Arc::new(snapshot));
    let theme = Theme::for_appearance(Appearance::from_prefers_dark(
        capture::Theme::current() == capture::Theme::Dark,
    ));
    let window = Window {
        theme,
        profiles: Profiles::default(),
        requests,
        snapshots,
    };
    let app = capture::mount_app(view(window), config, Some(size))?;
    Ok(Surface {
        app,
        _requests: received,
        _publish: publish,
    })
}

fn strip(toasts: Vec<Notification>) -> Result<Surface> {
    let snapshot = Snapshot {
        toasts,
        ..Snapshot::default()
    };
    mount(snapshot, popup::popup, &popup::config(), STRIP)
}

fn panel(history: Vec<Notification>, do_not_disturb: bool) -> Result<Surface> {
    let snapshot = Snapshot {
        history,
        center_open: true,
        do_not_disturb,
        ..Snapshot::default()
    };
    mount(snapshot, center::center, &center::config(), OUTPUT)
}

fn text(content: &str) -> crowntest::Query {
    role(Role::Text).name(content)
}

fn button(label: &str) -> crowntest::Query {
    role(Role::Button).name(label)
}

fn banners(carrier: &mut Carrier) -> Result<()> {
    {
        let mut surface = strip(vec![fixtures::mail(
            1,
            "Grace Hopper",
            "Can you review the compiler notes before tomorrow's sync?",
            0,
        )])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "Banner",
            "A single toast: app icon, app name, summary, body and timestamp",
        )?;
        surface.app.hover(text("Grace Hopper"));
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "Banner hovered",
            "Pointer over the card: the timestamp gives way to the Dismiss button",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::software_update(2)])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "With actions",
            "Action buttons along the bottom, aligned with the text column",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::calendar(3, InterruptionLevel::Active)])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "More actions",
            "Actions that do not fit go behind the chevron",
        )?;
        surface.app.click(button("More actions"));
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "Actions menu",
            "The chevron opened: the surplus actions as a menu",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::screenshot(4)])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "With image",
            "The notification's own image in place of the app icon",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::calendar(
            5,
            InterruptionLevel::TimeSensitive,
        )])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "Time sensitive",
            "A time-sensitive notification wears its eyebrow",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::battery(6)])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "Critical",
            "A critical notification breaks through everything",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::copying(7)])?;
        carrier.capture_state(
            &mut surface.app,
            BANNERS,
            "Progress",
            "A long-running operation drawn as a labelled bar",
        )?;
    }
    Ok(())
}

fn rich_cards(carrier: &mut Carrier) -> Result<()> {
    {
        let mut surface = strip(vec![fixtures::message(11)])?;
        carrier.capture_state(
            &mut surface.app,
            RICH,
            "Message",
            "A chat message relayed from the phone, with avatar and device badge",
        )?;
        surface.app.click(button("Reply")).settle();
        surface.app.type_text("On my way, ten minutes");
        carrier.capture_state(
            &mut surface.app,
            RICH,
            "Inline reply",
            "The reply field open with a draft",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::call(12)])?;
        carrier.capture_state(
            &mut surface.app,
            RICH,
            "Incoming call",
            "A relayed call with Decline and Answer",
        )?;
    }
    {
        let mut surface = strip(vec![fixtures::now_playing(13)])?;
        carrier.capture_state(
            &mut surface.app,
            RICH,
            "Now playing",
            "A music remote: album art, scrubber and transport",
        )?;
    }
    Ok(())
}

fn stacks(carrier: &mut Carrier) -> Result<()> {
    {
        let mut surface = strip(fixtures::mail_thread())?;
        carrier.capture_state(
            &mut surface.app,
            STACKS,
            "Stack",
            "Three notifications from one app collapse into a deck",
        )?;
        surface.app.click(text("Grace Hopper"));
        carrier.capture_state(
            &mut surface.app,
            STACKS,
            "Stack expanded",
            "The deck unfurled under its app header and Clear All",
        )?;
    }
    {
        let mut surface = strip(vec![
            fixtures::message(41),
            fixtures::copying(42),
            fixtures::software_update(43),
            fixtures::calendar(44, InterruptionLevel::TimeSensitive),
        ])?;
        carrier.capture_state(
            &mut surface.app,
            STACKS,
            "Toast strip",
            "Several apps at once, stacked in arrival order",
        )?;
    }
    Ok(())
}

fn notification_center(carrier: &mut Carrier) -> Result<()> {
    {
        let mut surface = panel(fixtures::history(), false)?;
        carrier.capture_state(
            &mut surface.app,
            CENTER,
            "Center",
            "The panel open over the desktop with the history list",
        )?;
        surface.app.click(text("Grace Hopper"));
        carrier.capture_state(
            &mut surface.app,
            CENTER,
            "Center group expanded",
            "A collapsed history deck unfurled under its app header and Clear All",
        )?;
    }
    {
        let mut surface = panel(fixtures::history(), true)?;
        carrier.capture_state(
            &mut surface.app,
            CENTER,
            "Do Not Disturb",
            "Do Not Disturb switched on",
        )?;
    }
    {
        let mut surface = panel(Vec::new(), false)?;
        carrier.capture_state(
            &mut surface.app,
            CENTER,
            "Empty center",
            "Nothing in the history",
        )?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut carrier = Carrier::new(
        "Notifications",
        "crownotify: the toast strip down the right edge and the notification center panel, captured headlessly from the current source.",
    );
    banners(&mut carrier)?;
    rich_cards(&mut carrier)?;
    stacks(&mut carrier)?;
    notification_center(&mut carrier)?;
    let path = carrier.write(None)?;
    println!("{}", path.display());
    capture::finish()
}
