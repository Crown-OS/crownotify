use crate::ui::components::icon::Icon;

#[derive(Debug, Clone)]
pub struct MusicNotification {
    pub app_icon: Icon,
    pub song_name: String,
    pub percentage: u8,
}
