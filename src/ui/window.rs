//! What each window is started with, and the configuration both share.

use std::sync::Arc;

use crownui::{
    kit,
    prelude::{AppConfig, TRANSPARENT},
};
use tokio::sync::watch;

use crate::{daemon::RequestSender, daemon::Snapshot, profile::table::Profiles, ui::theme::Theme};

/// One window's way to the daemon, and the look it is drawn in.
pub struct Window {
    pub theme: Theme,
    pub profiles: Profiles,
    pub requests: RequestSender,
    pub snapshots: watch::Receiver<Arc<Snapshot>>,
}

impl Window {
    /// A transparent, Inter-set window called `title`, asking for
    /// `width` × `height` until the compositor says otherwise.
    pub fn config(title: &str, width: u32, height: u32) -> AppConfig {
        AppConfig::new(title)
            .app_id("crownotify")
            .size(width, height)
            .clear_color(TRANSPARENT)
            .font(kit::inter_font().to_vec(), Some(kit::INTER_FAMILY))
    }
}
