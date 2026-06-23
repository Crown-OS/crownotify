use crate::ui::components::icon::Icon;

pub const CROWNCRATE_SERVICE: &str = "io.crownos.crowncrate";
pub const CROWNCRATE_PATH: &str = "/io/crownos/crowncrate";
pub const CROWNCRATE_INTERFACE: &str = "io.crownos.crowncrate";

#[derive(Debug, Clone)]
pub struct CallNotification {
    pub call_id: String,
    pub app_icon: Icon,
    pub app_name: String,
    pub contact_avatar: String,
    pub contact_name: String,
    pub phone_number: String,
}

impl CallNotification {
    pub async fn pickup(call_id: &str) -> zbus::Result<()> {
        notify_crowncrate(call_id, "PickupCall").await
    }

    pub async fn decline(call_id: &str) -> zbus::Result<()> {
        notify_crowncrate(call_id, "DeclineCall").await
    }
}

async fn notify_crowncrate(call_id: &str, method: &str) -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    conn.call_method(
        Some(CROWNCRATE_SERVICE),
        CROWNCRATE_PATH,
        Some(CROWNCRATE_INTERFACE),
        method,
        &(call_id,),
    )
    .await?;
    Ok(())
}
