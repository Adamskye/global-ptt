use std::sync::Arc;

use iced::{
    futures::{
        channel::mpsc::{channel, Receiver, Sender},
        executor::block_on,
        lock::Mutex,
        FutureExt, SinkExt, Stream, StreamExt,
    },
    stream,
};
use ksni::{
    menu::{CheckmarkItem, StandardItem},
    Category, MenuItem, Status, ToolTip,
};

use crate::Msg;

pub enum TrayMsg {
    Show,
    TogglePTT,
}

#[derive(Debug)]
pub struct Tray {
    msg_sender: Arc<Mutex<Sender<Msg>>>,
    ptt_enabled: bool,
}

impl Tray {
    pub fn new() -> (Self, impl Stream<Item = Msg>) {
        let (msg_sender, mut msg_receiver) = channel(10);
        let stream = stream::channel(10, async move |mut tx| {
            while let Some(msg) = msg_receiver.next().await {
                let _ = tx.send(msg).now_or_never();
            }
        });

        (
            Self {
                msg_sender: Arc::new(Mutex::new(msg_sender)),
                ptt_enabled: false,
            },
            stream,
        )
    }

    pub fn set_ptt_enabled(&mut self, enabled: bool) {
        self.ptt_enabled = enabled;
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn icon_name(&self) -> String {
        "microphone".into()
    }

    fn title(&self) -> String {
        "Global Push-to-Talk".into()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn activate(&mut self, _: i32, _: i32) {
        let _ = block_on(self.msg_sender.lock())
            .send(Msg::ShowWindow)
            .now_or_never();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let sender = self.msg_sender.clone();
        let tx = sender.clone();
        let toggle_ptt = MenuItem::Checkmark(CheckmarkItem {
            label: "Enable Push-to-Talk".into(),
            checked: self.ptt_enabled,
            activate: Box::new(move |_| {
                let _ = block_on(tx.lock()).send(Msg::ToggleActive).now_or_never();
            }),
            ..Default::default()
        });
        let tx = sender.clone();
        let exit = MenuItem::Standard(StandardItem {
            label: "Exit".into(),
            activate: Box::new(move |_| {
                let _ = block_on(tx.lock()).send(Msg::Exit).now_or_never();
            }),
            ..Default::default()
        });
        vec![toggle_ptt, exit]
    }

    fn tool_tip(&self) -> ToolTip {
        if self.ptt_enabled {
            ToolTip {
                title: "Global Push-to-Talk".into(),
                description: "Running".into(),
                ..Default::default()
            }
        } else {
            ToolTip {
                title: "Global Push-to-Talk".into(),
                description: "Not Running".into(),
                ..Default::default()
            }
        }
    }
}
