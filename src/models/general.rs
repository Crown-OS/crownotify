use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct GeneralNotification {
    pub app_icon: Option<PathBuf>,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub expire_timeout: u32,
    pub action: Vec<String>,
}
