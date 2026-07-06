use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MusicNotification {
    pub app_icon: Option<PathBuf>,
    pub song_name: String,
    pub percentage: u8,
}
