use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ChatNotification {
    pub icon: Option<PathBuf>,
    pub title: String,
    pub app_name: String,
    pub body: String,
    pub timestamp: String,
}
