use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::{connection, interface, proxy, Connection};

use crownotify::models::call::{
    CallNotification, CROWNCRATE_INTERFACE, CROWNCRATE_PATH, CROWNCRATE_SERVICE,
};

#[derive(Default, Clone)]
struct CrowncrateCaptured {
    pickups: Vec<String>,
    declines: Vec<String>,
}

struct MockCrowncrate {
    captured: Arc<Mutex<CrowncrateCaptured>>,
}

#[interface(name = "io.crownos.crowncrate")]
impl MockCrowncrate {
    fn pickup_call(&self, call_id: String) -> zbus::fdo::Result<()> {
        self.captured.lock().unwrap().pickups.push(call_id);
        Ok(())
    }

    fn decline_call(&self, call_id: String) -> zbus::fdo::Result<()> {
        self.captured.lock().unwrap().declines.push(call_id);
        Ok(())
    }
}

async fn start_mock_crowncrate(captured: Arc<Mutex<CrowncrateCaptured>>) -> Connection {
    connection::Builder::session()
        .expect("session bus connection failed")
        .name(CROWNCRATE_SERVICE)
        .expect("crowncrate name registration failed")
        .serve_at(CROWNCRATE_PATH, MockCrowncrate { captured })
        .expect("serve_at failed")
        .build()
        .await
        .expect("connection build failed")
}

#[derive(Default, Clone)]
struct CallNotificationCaptured {
    call_id: String,
    app_name: String,
    contact_avatar: String,
    contact_name: String,
    phone_number: String,
    received: bool,
}

struct MockCrownotify {
    captured: Arc<Mutex<CallNotificationCaptured>>,
}

#[interface(name = "io.crownos.crownotify")]
impl MockCrownotify {
    fn send_call_notification(
        &self,
        call_id: String,
        app_name: String,
        contact_avatar: String,
        contact_name: String,
        phone_number: String,
    ) -> zbus::fdo::Result<()> {
        let mut state = self.captured.lock().unwrap();
        state.call_id = call_id;
        state.app_name = app_name;
        state.contact_avatar = contact_avatar;
        state.contact_name = contact_name;
        state.phone_number = phone_number;
        state.received = true;
        Ok(())
    }
}

#[proxy(
    interface = "io.crownos.crownotify",
    default_service = "io.crownos.crownotify",
    default_path = "/io/crownos/crownotify"
)]
trait CrownotifyCall {
    fn send_call_notification(
        &self,
        call_id: &str,
        app_name: &str,
        contact_avatar: &str,
        contact_name: &str,
        phone_number: &str,
    ) -> zbus::Result<()>;
}

async fn start_mock_crownotify(captured: Arc<Mutex<CallNotificationCaptured>>) -> Connection {
    connection::Builder::session()
        .expect("session bus connection failed")
        .name("io.crownos.crownotify")
        .expect("crownotify name registration failed")
        .serve_at("/io/crownos/crownotify", MockCrownotify { captured })
        .expect("serve_at failed")
        .build()
        .await
        .expect("connection build failed")
}

#[test]
fn test_send_call_notification_delivers_all_fields() {
    smol::block_on(async {
        let captured: Arc<Mutex<CallNotificationCaptured>> = Arc::new(Mutex::new(Default::default()));
        let _service_conn = start_mock_crownotify(captured.clone()).await;

        let conn = Connection::session()
            .await
            .expect("session bus connection failed");
        let proxy = CrownotifyCallProxy::new(&conn)
            .await
            .expect("proxy creation failed");

        proxy
            .send_call_notification(
                "call_42",
                "Phone",
                "/tmp/avatar.png",
                "Name Surname",
                "+1-555-0100",
            )
            .await
            .expect("send_call_notification call failed");

        let state = captured.lock().unwrap().clone();
        assert!(state.received, "mock did not receive the notification");
        assert_eq!(state.call_id, "call_42");
        assert_eq!(state.app_name, "Phone");
        assert_eq!(state.contact_avatar, "/tmp/avatar.png");
        assert_eq!(state.contact_name, "Name Surname");
        assert_eq!(state.phone_number, "+1-555-0100");
    });
}

#[test]
fn test_pickup_notifies_crowncrate() {
    smol::block_on(async {
        let captured: Arc<Mutex<CrowncrateCaptured>> = Arc::new(Mutex::new(Default::default()));
        let _service_conn = start_mock_crowncrate(captured.clone()).await;

        CallNotification::pickup("call_pickup_1")
            .await
            .expect("pickup dbus call failed");

        let state = captured.lock().unwrap().clone();
        assert_eq!(state.pickups, vec!["call_pickup_1".to_string()]);
        assert!(state.declines.is_empty());
    });
}

#[test]
fn test_decline_notifies_crowncrate() {
    smol::block_on(async {
        let captured: Arc<Mutex<CrowncrateCaptured>> = Arc::new(Mutex::new(Default::default()));
        let _service_conn = start_mock_crowncrate(captured.clone()).await;

        CallNotification::decline("call_decline_1")
            .await
            .expect("decline dbus call failed");

        let state = captured.lock().unwrap().clone();
        assert_eq!(state.declines, vec!["call_decline_1".to_string()]);
        assert!(state.pickups.is_empty());
    });
}

#[test]
fn test_pickup_and_decline_use_correct_dbus_endpoint() {
    // Sanity: the constants the model uses match what the test mock advertises.
    assert_eq!(CROWNCRATE_SERVICE, "io.crownos.crowncrate");
    assert_eq!(CROWNCRATE_PATH, "/io/crownos/crowncrate");
    assert_eq!(CROWNCRATE_INTERFACE, "io.crownos.crowncrate");
}

#[allow(dead_code)]
async fn settle() {
    smol::Timer::after(Duration::from_millis(50)).await;
}
