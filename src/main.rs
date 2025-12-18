mod app;
mod error_message;
mod pulse;
mod tray;

use crate::app::App;

const APP_ID: &str = "com.github.Adamskye.GlobalPushToTalk";

const PADDING: f32 = 12.0;
const SPACING: f32 = 8.0;

const WL_HOTKEY_ID: u32 = 0;

fn main() -> iced::Result {
    match App::new() {
        Ok((app, task)) => iced::daemon(move || (app.clone(), task), App::update, App::view)
            .subscription(App::subscription)
            .theme(App::theme)
            .run(),
        Err(e) => error_message::run_error_app(e.to_string()),
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Pulse(#[from] pulse::Error),
    #[error(transparent)]
    Ksni(#[from] ksni::Error),
}
