use std::{io::Write, os::unix::net::UnixStream, process::exit, str::FromStr};

use ashpd::zbus::block_on;
use global_hotkey::{hotkey::HotKey, wayland::using_wayland};
use iced::{
    Element, Font, Length, Subscription, Task, Theme,
    alignment::{Horizontal, Vertical},
    font::{Style, Weight},
    futures::StreamExt,
    keyboard::{self, Key, Modifiers},
    widget::{
        button, checkbox, column, container, pick_list, rich_text, row, rule, space, span, text,
        tooltip,
    },
    window::{Id, Settings, UserAttention, close_requests, settings::PlatformSpecific},
};
use iced_fonts::lucide;
use ksni::{Handle, TrayMethods};
use notify_rust::Notification;
use signal_hook_tokio::Signals;
use tokio::{io::AsyncReadExt, net::UnixListener, sync::mpsc::Sender};
use tokio_stream::wrappers::UnixListenerStream;

use crate::{
    APP_ID, PADDING, SPACING,
    hotkey::{HotKeyConfig, hotkeys},
    pulse::{InputDevice, PulseAudioState, VIRTUALMIC_DESCRIPTION},
    tray::Tray,
};

#[derive(Debug, Clone)]
pub enum Msg {
    GlobalShortcutsFail,
    ChooseMicrophone(String),
    SetActive(bool),
    ToggleActive,
    SetMuted(bool),
    UpdateHotKeyDescriptions(HotKeyConfig<String>),
    ShowWindow,
    Close,
    Exit,
    SetTheme(Option<Theme>),
    InitChangeHotKeyTX(Sender<HotKeyConfig<HotKey>>),
    StartHotKeyRecording(HotKeyAction),
    FinishHotKeyRecording(String),
    None,
}

#[derive(Debug, Clone)]
pub enum HotKeyAction {
    Trigger,
    ToggleActive,
}

#[derive(Clone)]
struct Backend {
    pa_state: PulseAudioState,
    tray: Option<Handle<Tray>>,
}

#[derive(Clone)]
enum BackendState {
    Loaded(Backend),
    Error(String),
}

#[derive(Clone)]
pub struct App {
    active: bool,
    muted: bool,
    hk_descriptions: HotKeyConfig<String>,
    backend: BackendState,
    theme: Option<Theme>,
    change_hotkey_tx: Option<Sender<HotKeyConfig<HotKey>>>,
    recording_hotkey: Option<HotKeyAction>,
}

impl App {
    pub fn new() -> (Self, Task<Msg>) {
        // there must only be one running instance of this application

        // try to open existing instance
        let socket_path = format!("/tmp/{APP_ID}.{}", nix::unistd::Uid::current());
        if let Ok(mut stream) = UnixStream::connect(socket_path.clone())
            && stream.write_all(b"open").is_ok()
        {
            // existing instance successfully opened
            exit(0);
        }

        // create new unix listener
        let _ = std::fs::remove_file(&socket_path);
        let ipc_stream: Task<Msg> = Task::future(async move { UnixListener::bind(socket_path) })
            .then(|res| {
                let Ok(listener) = res else {
                    return Task::none();
                };

                let stream = UnixListenerStream::new(listener);
                Task::stream(stream).then(|incoming| {
                    Task::future(async {
                        let Ok(mut incoming) = incoming else {
                            return Msg::None;
                        };

                        let mut buffer = String::new();
                        let _ = incoming.read_to_string(&mut buffer).await;

                        if buffer == "open" {
                            Msg::ShowWindow
                        } else {
                            Msg::None
                        }
                    })
                })
            });

        let pa_state = PulseAudioState::init();
        let (tray_builder, tray_stream) = Tray::new();
        let tray = block_on(tray_builder.spawn());

        let backend = match (pa_state, tray.ok()) {
            (Ok(pa_state), tray) => BackendState::Loaded(Backend { pa_state, tray }),
            (Err(e), _) => BackendState::Error(e.to_string()),
        };

        let this = Self {
            muted: false,
            active: false,
            hk_descriptions: HotKeyConfig::default(),
            theme: None,
            backend,
            change_hotkey_tx: None,
            recording_hotkey: None,
        };

        // handling signals
        let signal_handler = match Signals::new([signal_hook::consts::SIGUSR1]) {
            Ok(signals) => Task::stream(signals).map(|_| Msg::ShowWindow),
            Err(_) => Task::none(),
        };

        let tasks = Task::batch([
            Task::done(Msg::ShowWindow),
            Task::stream(tray_stream),
            ipc_stream,
            Task::stream(
                mundy::Preferences::stream(mundy::Interest::ColorScheme).map(|c| {
                    Msg::SetTheme(match c.color_scheme {
                        mundy::ColorScheme::NoPreference => None,
                        mundy::ColorScheme::Light => Some(Theme::Light),
                        mundy::ColorScheme::Dark => Some(Theme::KanagawaDragon),
                    })
                }),
            ),
            signal_handler,
        ]);
        (this, tasks)
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::None => {}
            Msg::ChooseMicrophone(mic) => return self.choose_microphone(&mic),
            Msg::SetActive(a) => return self.set_active(a),
            Msg::ToggleActive => return Task::done(Msg::SetActive(!self.active)),
            Msg::SetMuted(m) => self.set_muted(m),
            Msg::GlobalShortcutsFail => self.global_shortcuts_fail(),
            Msg::UpdateHotKeyDescriptions(descriptions) => self.hk_descriptions = descriptions,
            Msg::ShowWindow => return self.show_window(),
            Msg::Close => return Self::close_window(),
            Msg::Exit => self.exit(),
            Msg::SetTheme(theme) => self.theme = theme,
            Msg::InitChangeHotKeyTX(change_hotkey) => self.change_hotkey_tx = Some(change_hotkey),
            Msg::StartHotKeyRecording(recording) => self.recording_hotkey = Some(recording),
            Msg::FinishHotKeyRecording(hk_string) => {
                println!("hk_string: {hk_string}");
                // return Task::none();
                return self.finish_hotkey_recording(hk_string);
            }
        }
        Task::none()
    }

    fn global_shortcuts_fail(&mut self) {
        let msg = "Failed to load global shortcuts. Push-to-talk will not work. Make sure you are using a Wayland compositor with a portal implementation that supports global shortcuts.";
        self.backend = BackendState::Error(msg.into());
    }

    fn set_muted(&mut self, muted: bool) {
        let BackendState::Loaded(b) = &mut self.backend else {
            return;
        };

        let res = b.pa_state.set_mute(self.active && muted);
        if let Err(e) = res {
            eprintln!("Failed to set mute: {e}");
        }
        self.muted = self.active && muted;
    }

    fn set_active(&mut self, active: bool) -> Task<Msg> {
        let BackendState::Loaded(b) = &mut self.backend else {
            return Task::none();
        };

        self.active = active;
        if let Some(tray) = &b.tray {
            block_on(tray.update(|tray| tray.set_ptt_enabled(active)));
        }

        Task::done(Msg::SetMuted(active))
    }

    fn choose_microphone(&mut self, mic: &str) -> Task<Msg> {
        let BackendState::Loaded(b) = &mut self.backend else {
            return Task::none();
        };

        let is_first_time = b.pa_state.get_active_source_name().is_none();
        b.pa_state.set_virtual_mic(mic);

        // enable ptt automatically after choosing microphone for the first time
        if is_first_time {
            Task::done(Msg::SetActive(true))
        } else {
            Task::none()
        }
    }

    fn finish_hotkey_recording(&mut self, hk_string: String) -> Task<Msg> {
        let Some(recording_hotkey) = self.recording_hotkey.take() else {
            return Task::none();
        };

        let Ok(new_hk) = HotKey::from_str(&hk_string) else {
            return Task::none();
        };

        // get current hotkeys
        let def = HotKeyConfig::default();
        let mut hotkeys = HotKeyConfig {
            trigger: HotKey::from_str(&self.hk_descriptions.trigger).unwrap_or(def.trigger),
            toggle_active: HotKey::from_str(&self.hk_descriptions.toggle_active)
                .unwrap_or(def.toggle_active),
        };

        match recording_hotkey {
            HotKeyAction::Trigger => hotkeys.trigger = new_hk,
            HotKeyAction::ToggleActive => hotkeys.toggle_active = new_hk,
        }

        if let Some(tx) = self.change_hotkey_tx.clone() {
            Task::future(async move { tx.send(hotkeys).await }).discard()
        } else {
            Task::none()
        }
    }

    fn show_window(&mut self) -> Task<Msg> {
        let size = match self.backend {
            BackendState::Loaded(_) => (600, 300),
            BackendState::Error(_) => (280, 180),
        };
        iced::window::latest().then(move |res| {
            if let Some(id) = res {
                Task::batch([
                    iced::window::request_user_attention(id, Some(UserAttention::Informational)),
                    iced::window::gain_focus(id),
                ])
            } else {
                iced::window::open(Settings {
                    exit_on_close_request: false,
                    size: size.into(),
                    resizable: true,
                    decorations: true,
                    platform_specific: PlatformSpecific {
                        application_id: APP_ID.to_string(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .1
                .discard()
            }
        })
    }

    fn close_window() -> Task<Msg> {
        let _ = Notification::new()
            .appname("Global Push-to-Talk")
            .summary("Global Push-to-Talk is running in the background")
            .show();
        iced::window::latest().and_then(iced::window::close)
    }

    fn exit(&mut self) {
        if let BackendState::Loaded(b) = &mut self.backend {
            b.pa_state.remove_virtual_mic();
        }
        exit(0);
    }

    pub fn theme(&self, _: Id) -> Option<Theme> {
        self.theme.clone()
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            close_requests().map(|_| Msg::Close),
            Subscription::run(hotkeys),
            if self.recording_hotkey.is_some() {
                Self::record_hotkey()
            } else {
                Subscription::none()
            },
        ])
    }

    fn record_hotkey() -> Subscription<Msg> {
        fn key_to_str(key: Key) -> String {
            match key {
                keyboard::Key::Named(named) => format!("{named:?}"),
                keyboard::Key::Character(c) => c.into(),
                keyboard::Key::Unidentified => "".into(),
            }
        }

        fn mod_to_str(modifiers: Modifiers) -> String {
            // otherwise, finish recording
            let mut s = Vec::with_capacity(4);
            if modifiers.control() {
                s.push("CTRL");
            }
            if modifiers.shift() {
                s.push("SHIFT");
            }
            if modifiers.alt() {
                s.push("ALT");
            }
            if modifiers.logo() {
                s.push("SUPER");
            }
            return s.join("+");
        }

        keyboard::listen().map(|k_ev| match k_ev {
            keyboard::Event::KeyReleased { key, modifiers, .. } => {
                // rule: if the key released is a modifier key, then finish
                use iced::keyboard::key::Named as N;
                use keyboard::Key::Named;
                match key {
                    Named(N::Control) | Named(N::Alt) | Named(N::AltGraph) | Named(N::Shift)
                    | Named(N::Super) => Msg::FinishHotKeyRecording(mod_to_str(modifiers)),
                    _ => Msg::None,
                }
            }
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
                // rule: if the key pressed is not a modifier key, then finish
                use iced::keyboard::key::Named as N;
                use keyboard::Key::Named;

                match key {
                    Named(N::Control) | Named(N::Alt) | Named(N::AltGraph) | Named(N::Shift)
                    | Named(N::Super) => Msg::None,
                    _ => {
                        if modifiers.is_empty() {
                            Msg::FinishHotKeyRecording(key_to_str(key))
                        } else {
                            Msg::FinishHotKeyRecording(
                                [mod_to_str(modifiers), key_to_str(key)].join("+"),
                            )
                        }
                    }
                }
            }
            _ => Msg::None,
        })
    }

    pub fn view(&self, _window: Id) -> Element<'_, Msg> {
        let backend = match &self.backend {
            BackendState::Loaded(backend) => backend,
            BackendState::Error(e) => return show_error(e.clone()),
        };

        if self.recording_hotkey.is_some() {
            return recording_hotkey();
        }

        let title = title("Global Push-to-Talk");
        let sep = rule::horizontal(1.0);

        let main = container(
            column![self.toggle_controls(backend), select_mic(backend),].spacing(SPACING),
        )
        .padding(PADDING);

        let footer = row![
            self.hotkey_indicator(),
            space().width(Length::Fill),
            button("Exit").on_press(Msg::Exit)
        ]
        .align_y(Vertical::Bottom);

        column![title, sep, main, space().height(Length::Fill), footer]
            .padding(PADDING)
            .spacing(SPACING)
            .into()
    }

    fn toggle_controls(&self, backend: &Backend) -> Element<'_, Msg> {
        if get_selected_mic(backend).is_none() {
            return row![
                text("Select a microphone to enable push-to-talk")
                    .font(Font {
                        style: Style::Italic,
                        ..Default::default()
                    })
                    .style(weak_text_style)
            ]
            .spacing(SPACING)
            .align_y(Vertical::Center)
            .into();
        }

        let label = text("Enable");
        let checkbox = checkbox(self.active).on_toggle(Msg::SetActive);

        let info = text(format!(
            "Select \"{VIRTUALMIC_DESCRIPTION}\" in any application to use push-to-talk"
        ))
        .font(Font {
            style: Style::Italic,
            ..Default::default()
        })
        .style(weak_text_style);

        column![
            row![label, checkbox, self.mute_indicator()]
                .spacing(SPACING)
                .align_y(Vertical::Center),
            info
        ]
        .spacing(SPACING)
        .into()
    }

    fn mute_indicator(&self) -> Element<'_, Msg> {
        let icon = if self.muted {
            lucide::mic_off()
        } else {
            lucide::mic()
        }
        .align_y(Vertical::Bottom);

        icon.color(if self.muted {
            [0.8, 0.0, 0.0]
        } else {
            [0.0, 0.8, 0.0]
        })
        .into()
    }

    fn hotkey_indicator(&self) -> Element<'_, Msg> {
        if using_wayland() {
            let trigger_label = hk_label("Trigger", &self.hk_descriptions.trigger, None);
            let toggle_active_label =
                hk_label("Enable/Disable", &self.hk_descriptions.toggle_active, None);

            let all = row![trigger_label, toggle_active_label]
                .spacing(SPACING)
                .align_y(Vertical::Center);

            tooltip(
                all,
                "Configure these hotkeys in your system's settings",
                tooltip::Position::Top,
            )
            .into()
        } else {
            let d = &self.hk_descriptions;
            use HotKeyAction as HKR;

            let trigger_label = hk_label("Trigger", &d.trigger, Some(HKR::Trigger));
            let toggle_active_label =
                hk_label("Enable/Disable", &d.toggle_active, Some(HKR::ToggleActive));

            let all = row![trigger_label, toggle_active_label]
                .spacing(SPACING)
                .align_y(Vertical::Center);

            tooltip(
                all,
                "Click on any hotkey to change it...",
                tooltip::Position::Top,
            )
            .into()
        }
    }
}

fn get_selected_mic(backend: &Backend) -> Option<InputDevice> {
    backend
        .pa_state
        .get_input_devices()
        .iter()
        .find(|dev| Some(dev.name.as_str()) == backend.pa_state.get_active_source_name())
        .cloned()
}

fn recording_hotkey<'a>() -> Element<'a, Msg> {
    let txt = text("Enter a key combination...");
    let space1 = space().width(Length::Fill).height(Length::Fill);
    let space2 = space().width(Length::Fill).height(Length::Fill);
    column![space1, txt, space2]
        .align_x(Horizontal::Center)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn select_mic(backend: &Backend) -> Element<'_, Msg> {
    let label = text("Microphone");
    let input_devs = backend.pa_state.get_input_devices();
    let selected = get_selected_mic(backend);
    let pick_list = pick_list(input_devs, selected, |dev| Msg::ChooseMicrophone(dev.name))
        .width(Length::Fill)
        .placeholder("Choose Microphone...");

    let refresh_btn = button("⟳").on_press(Msg::None);

    row![label, pick_list, refresh_btn]
        .spacing(SPACING)
        .width(Length::Fill)
        .align_y(Vertical::Center)
        .into()
}

fn show_error<'a>(message: String) -> Element<'a, Msg> {
    let title = title("Error");
    let sep = rule::horizontal(1.0);
    let message = text(message).wrapping(text::Wrapping::Word);

    let close_btn = button("Close").on_press(Msg::Exit);
    let close_btn = column![close_btn]
        .align_x(Horizontal::Right)
        .width(Length::Fill);

    column![title, sep, message, space().height(Length::Fill), close_btn]
        .spacing(SPACING)
        .padding(PADDING)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

fn title<'a>(content: impl text::IntoFragment<'a>) -> Element<'a, Msg> {
    text(content)
        .font(Font {
            weight: Weight::Light,
            ..Default::default()
        })
        .size(24.0)
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .into()
}

fn weak_text_style(theme: &Theme) -> text::Style {
    let color = theme.extended_palette().secondary.strong.color;
    text::Style { color: Some(color) }
}

fn hk_label<'a>(
    name: &'a str,
    description: &'a str,
    recording: Option<HotKeyAction>,
) -> Element<'a, Msg> {
    let italic = Font {
        style: Style::Italic,
        ..Default::default()
    };
    let bold_italic = Font {
        weight: Weight::Bold,
        style: Style::Italic,
        ..Default::default()
    };

    let link = recording.as_ref().map(|_| ());
    rich_text([
        span(name).link_maybe(link),
        span(": "),
        span(description).font(bold_italic),
    ])
    .on_link_click(move |()| match recording.clone() {
        Some(recording) => Msg::StartHotKeyRecording(recording.clone()),
        None => Msg::None,
    })
    .font(italic)
    .style(weak_text_style)
    .into()
}
