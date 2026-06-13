use crate::ui::components::icon::Icon;

#[derive(Debug, Clone)]
pub struct ChatNotification {
    pub icon: Icon,
    pub title: String,
    pub app_name: String,
    pub body: String,
    pub timestamp: String,
}
