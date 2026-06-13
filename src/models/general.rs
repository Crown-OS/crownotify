use crate::ui::components::icon::Icon;

#[derive(Debug, Clone)]
pub struct GeneralNotification {
    pub app_icon: Icon,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub expire_timeout: u32,
    pub action: Vec<String>,
}
