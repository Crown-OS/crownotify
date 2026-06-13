use crate::ui::components::icon::Icon;

#[derive(Debug, Clone)]
pub struct CallNotification {
    pub app_icon: Icon,
    pub contact_name: String,
}
