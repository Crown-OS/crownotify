use zbus::{connection, interface, proxy, Connection};

struct TestInterface;

#[interface(name = "io.crownos.crownotify")]
impl TestInterface {
    fn open_notification_center(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    fn close_notification_center(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }
}

#[proxy(
    interface = "io.crownos.crownotify",
    default_service = "io.crownos.crownotify",
    default_path = "/io/crownos/crownotify"
)]
trait CustomNotify {
    fn open_notification_center(&self) -> zbus::Result<()>;
    fn close_notification_center(&self) -> zbus::Result<()>;
}

async fn start_service() -> zbus::Connection {
    connection::Builder::session()
        .expect("session bus connection failed")
        .name("io.crownos.crownotify")
        .expect("name registration failed")
        .serve_at("/io/crownos/crownotify", TestInterface)
        .expect("serve_at failed")
        .build()
        .await
        .expect("connection build failed")
}

#[test]
fn test_open_notification_center() {
    smol::block_on(async {
        let _service_conn = start_service().await;
        let conn = Connection::session().await.expect("session bus connection failed");
        let proxy = CustomNotifyProxy::new(&conn).await.expect("proxy creation failed");
        proxy.open_notification_center().await.expect("open_notification_center call failed");
    });
}

#[test]
fn test_close_notification_center() {
    smol::block_on(async {
        let _service_conn = start_service().await;
        let conn = Connection::session().await.expect("session bus connection failed");
        let proxy = CustomNotifyProxy::new(&conn).await.expect("proxy creation failed");
        proxy.close_notification_center().await.expect("close_notification_center call failed");
    });
}
