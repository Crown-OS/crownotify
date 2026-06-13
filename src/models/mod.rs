pub mod audio;
pub mod call;
pub mod chat;
pub mod display;
pub mod general;
pub mod music;

use audio::AudioNotification;
use call::CallNotification;
use chat::ChatNotification;
use display::DisplayNotification;
use general::GeneralNotification;
use music::MusicNotification;

#[derive(Debug, Clone)]
pub enum Notification {
    General(GeneralNotification),
    Call(CallNotification),
    Music(MusicNotification),
    Chat(ChatNotification),
    Audio(AudioNotification),
    Display(DisplayNotification),
}
