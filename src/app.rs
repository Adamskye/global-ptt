use std::process::exit;

use ashpd::zbus::block_on;
use global_hotkey::{
    hotkey::{Code, HotKey},
    wayland::{using_wayland, WlHotKeysChangedEvent, WlNewHotKeyAction},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use iced::{
    alignment::Vertical,
    font::Weight,
    futures::{FutureExt, SinkExt, Stream, StreamExt},
    stream,
    widget::{checkbox, column, pick_list, row, rule, text},
    window::{close_requests, Id, Settings},
    Element, Font, Subscription, Task, Theme,
};
use ksni::{Handle, TrayMethods};

use crate::{pulse::PulseAudioState, tray::Tray, Error, APP_ID, PADDING, SPACING, WL_HOTKEY_ID};

#[derive(Debug, Clone)]
pub enum Msg {
    GlobalShortcutsFail(String),
    ChooseMicrophone(String),
    SetActive(bool),
    ToggleActive,
    SetMuted(bool),
    SetHotKeyDescription(String),
    ShowWindow,
    Close,
    Exit,
    SetTheme(Option<Theme>),
}

#[derive(Clone)]
pub struct App {
    active: bool,
    muted: bool,
    hotkey_description: String,
    pa_state: PulseAudioState,
    tray: Handle<Tray>,
    window_id: Id,
    theme: Option<Theme>,
    fatal_error: Option<String>,
}

impl App {
    pub fn new() -> Result<(Self, Task<Msg>), Error> {
        // TODO: remove expect statements later
        let pa_state = PulseAudioState::init()?;
        let (tray_builder, stream) = Tray::new();
        let tray = block_on(tray_builder.spawn())?;

        let (window_id, window_open_task) = iced::window::open(Settings {
            exit_on_close_request: false,
            ..Default::default()
        });

        let this = Self {
            muted: false,
            active: false,
            hotkey_description: "".into(),
            pa_state,
            tray,
            window_id,
            theme: None,
            fatal_error: None,
        };

        let tasks = Task::batch([
            Task::stream(stream),
            window_open_task.discard(),
            Task::stream(
                mundy::Preferences::stream(mundy::Interest::ColorScheme).map(|c| {
                    Msg::SetTheme(match c.color_scheme {
                        mundy::ColorScheme::NoPreference => None,
                        mundy::ColorScheme::Light => Some(Theme::Light),
                        mundy::ColorScheme::Dark => Some(Theme::Dark),
                    })
                }),
            ),
        ]);
        Ok((this, tasks))
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::ChooseMicrophone(mic) => self.pa_state.set_virtual_mic(&mic),
            Msg::SetActive(a) => {
                self.active = a;
                block_on(self.tray.update(|tray| tray.set_ptt_enabled(a)));
                return Task::done(Msg::SetMuted(a));
            }
            Msg::ToggleActive => {
                return Task::done(Msg::SetActive(!self.active));
            }
            Msg::SetMuted(m) => {
                let res = self.pa_state.set_mute(self.active && m);
                if let Err(e) = res {
                    eprintln!("Failed to set mute: {}", e);
                }
                self.muted = self.active && m;
            }
            Msg::GlobalShortcutsFail(_) => return iced::exit(),
            Msg::SetHotKeyDescription(desc) => self.hotkey_description = desc,
            Msg::ShowWindow => {
                return iced::window::open(Settings::default()).1.discard();
            }
            Msg::Close => {
                return iced::window::latest().and_then(|w| iced::window::close(w));
            }
            Msg::Exit => {
                self.pa_state.remove_virtual_mic();
                exit(0);
            }
            Msg::SetTheme(theme) => self.theme = theme,
        };
        Task::none()
    }

    pub fn theme(&self, _: Id) -> Option<Theme> {
        self.theme.clone()
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            close_requests().map(|_| Msg::Close),
            Subscription::run(global_shortcuts),
            Subscription::run(global_shortcuts_wl_change),
        ])
    }

    pub fn view(&self, _window: Id) -> Element<'_, Msg> {
        let title = title("Global Push-to-Talk");
        let spacer = rule::horizontal(1.0);

        column![
            title,
            spacer,
            self.toggle_active(),
            self.select_mic(),
            self.mute_indicator(),
            self.hotkey_indicator()
        ]
        .padding(PADDING)
        .spacing(SPACING)
        .into()
    }

    fn toggle_active(&self) -> Element<'_, Msg> {
        if self.pa_state.get_active_source().is_none() {
            return row![text("Select a microphone to enable push-to-talk")]
                .spacing(SPACING)
                .padding(PADDING)
                .align_y(Vertical::Center)
                .into();
        }

        let text = text("Enable");
        let checkbox = checkbox(self.active)
            .on_toggle_maybe(self.pa_state.get_active_source().map(|_| Msg::SetActive));

        row![text, checkbox]
            .spacing(SPACING)
            .padding(PADDING)
            .align_y(Vertical::Center)
            .into()
    }

    fn mute_indicator(&self) -> Element<'_, Msg> {
        text(if self.muted { "Muted" } else { "Not Muted" }).into()
    }

    fn hotkey_indicator(&self) -> Element<'_, Msg> {
        text(format!("Hotkey: {}", self.hotkey_description)).into()
    }

    fn select_mic(&self) -> Element<'_, Msg> {
        let label = text("Microphone");
        let pick_list = pick_list(
            self.pa_state.get_input_devices(),
            self.pa_state.get_active_source().map(|s| s.to_string()),
            Msg::ChooseMicrophone,
        )
        .placeholder("Choose Microphone...");

        row![label, pick_list]
            .spacing(SPACING)
            .padding(PADDING)
            .align_y(Vertical::Center)
            .into()
    }
}

fn global_shortcuts_wl_change() -> impl Stream<Item = Msg> {
    // receiving keypress changes under Wayland
    stream::channel(100, async |mut tx| {
        // TODO: make this cleaner
        loop {
            let Some(rec) = WlHotKeysChangedEvent::receiver() else {
                return;
            };

            match tokio::task::spawn_blocking(move || rec.recv()).await {
                Ok(Ok(ev)) => {
                    for change in ev.changed_hotkeys {
                        if change.id != WL_HOTKEY_ID {
                            continue;
                        }
                        let _ = tx
                            .send(Msg::SetHotKeyDescription(change.hotkey_description))
                            .await;
                        break;
                    }
                }
                _ => return,
            }
        }
    })
}

fn global_shortcuts() -> impl Stream<Item = Msg> {
    stream::channel(100, async |mut tx| {
        let gh = match GlobalHotKeyManager::new() {
            Ok(g) => g,
            Err(e) => {
                let _ = tx.send(Msg::GlobalShortcutsFail(e.to_string())).await;
                return;
            }
        };

        let default_hotkey = HotKey::new(None, Code::Insert);
        let hk_id = if using_wayland() {
            if let Err(e) = gh.wl_register_all(
                APP_ID,
                &[WlNewHotKeyAction::new(
                    WL_HOTKEY_ID,
                    "Activate push-to-talk",
                    Some(default_hotkey),
                )],
            ) {
                let _ = tx.send(Msg::GlobalShortcutsFail(e.to_string())).await;
                return;
            }

            if let Some(hk) = gh
                .wl_get_hotkeys()
                .iter()
                .find(|hk| hk.id() == WL_HOTKEY_ID)
            {
                let _ = tx
                    .send(Msg::SetHotKeyDescription(
                        hk.hotkey_description().to_string(),
                    ))
                    .await;
            }

            WL_HOTKEY_ID
        } else {
            let _ = gh.register(default_hotkey);
            default_hotkey.id()
        };

        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(Ok(msg)) = tokio::task::spawn_blocking(|| receiver.recv()).await {
            if msg.id() != hk_id {
                continue;
            }

            let _ = tx
                .send(Msg::SetMuted(msg.state() == HotKeyState::Released))
                .now_or_never();
        }
    })
}

fn title<'a>(content: impl text::IntoFragment<'a>) -> Element<'a, Msg> {
    text(content)
        .font(Font {
            weight: Weight::Bold,
            ..Default::default()
        })
        .size(18.0)
        .into()
}
