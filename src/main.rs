#![deny(clippy::unwrap_used)]
#![warn(clippy::pedantic)]

mod app;
mod config;
mod hotkey;
mod pulse;
mod tray;

use iced_fonts::LUCIDE_FONT_BYTES;

use crate::app::App;

const APP_ID: &str = "com.github.Adamskye.GlobalPushToTalk";

const PADDING: f32 = 12.0;
const SPACING: f32 = 8.0;

fn main() -> iced::Result {
    iced::daemon(App::new, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .title("Global Push-to-Talk")
        .font(LUCIDE_FONT_BYTES)
        .run()
}
