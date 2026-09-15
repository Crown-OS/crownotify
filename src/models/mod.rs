pub mod call;
pub mod chat;
pub mod general;
pub mod music;

use call::CallNotification;
use chat::ChatNotification;
use general::GeneralNotification;
use music::MusicNotification;

#[derive(Debug, Clone)]
pub enum Notification {
    General(GeneralNotification),
    Call(CallNotification),
    Music(MusicNotification),
    Chat(ChatNotification),
}
