use std::process::exit;

use iced::{
    alignment::{Horizontal, Vertical},
    font::Weight,
    widget::{button, column, row, space, text},
    Element, Font, Length, Size,
};

use crate::{PADDING, SPACING};

struct ErrorMessage(String);

impl ErrorMessage {
    fn new(message: String) -> Self {
        Self(message)
    }

    fn update(&mut self, _: ()) {
        exit(0);
    }

    fn view(&self) -> Element<'_, ()> {
        let title = text("Error").wrapping(text::Wrapping::Word);
        let message = text(&self.0);

        let close_btn = button("Close").on_press(());
        let close_btn = column![close_btn]
            .align_x(Horizontal::Right)
            .width(Length::Fill);

        column![title, message, space().height(Length::Fill), close_btn]
            .spacing(SPACING)
            .padding(PADDING)
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    }
}

pub fn run_error_app(message: String) -> iced::Result {
    iced::application(
        move || ErrorMessage::new(message.clone()),
        ErrorMessage::update,
        ErrorMessage::view,
    )
    .window_size((200, 128))
    .run()
}

fn title<'a>(content: impl text::IntoFragment<'a>) -> Element<'a, ()> {
    text(content)
        .font(Font {
            weight: Weight::Bold,
            ..Default::default()
        })
        .size(18.0)
        .into()
}
