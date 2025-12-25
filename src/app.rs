use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    process::exit,
    str::FromStr,
    sync::Arc,
};

use ashpd::zbus::block_on;
use fs4::fs_std::FileExt;
use global_hotkey::{hotkey::HotKey, wayland::using_wayland};
use iced::{
    Element, Font, Length, Subscription, Task, Theme,
    alignment::{Horizontal, Vertical},
    font::{Style, Weight},
    futures::StreamExt,
    keyboard,
    widget::{button, checkbox, column, pick_list, row, rule, space, text, tooltip},
    window::{Id, Settings, close_requests},
};
use iced_fonts::{LUCIDE_FONT_BYTES, lucide};
use ksni::{Handle, TrayMethods};
use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};
use signal_hook_tokio::Signals;
use tokio::sync::mpsc::Sender;

use crate::{
    APP_ID, PADDING, SPACING,
    hotkey::hotkeys,
    pulse::{PulseAudioState, VIRTUALMIC_DESCRIPTION},
    tray::Tray,
};

#[derive(Debug, Clone)]
pub enum Msg {
    GlobalShortcutsFail,
    ChooseMicrophone(String),
    SetActive(bool),
    ToggleActive,
    SetMuted(bool),
    SetHotKeyDescription(String),
    ShowWindow,
    Close,
    Exit,
    SetTheme(Option<Theme>),
    InitChangeHotKeySender(Sender<HotKey>),
    StartHotKeyRecording,
    StopHotKeyRecording(String),
    None,
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
    hotkey_description: String,
    backend: BackendState,
    theme: Option<Theme>,
    change_hotkey_tx: Option<Sender<HotKey>>,
    recording_hotkey: bool,
    _flock: Option<Arc<File>>,
}

impl App {
    pub fn new() -> (Self, Task<Msg>) {
        // there must only be one running instance of this application
        let lock_path = format!("/tmp/{APP_ID}.{}.lock", nix::unistd::Uid::current());
        let _flock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&lock_path)
            .map(|mut file| {
                if !matches!(file.try_lock_exclusive(), Ok(true)) {
                    // there is another running instance
                    eprintln!("There is another running instance!");

                    let mut c = String::new();
                    let _ = file.read_to_string(&mut c);

                    let sig = Some(Signal::SIGUSR1);
                    if let Ok(pid) = c.parse::<i32>()
                        && signal::kill(Pid::from_raw(pid), sig).is_ok()
                    {
                        exit(0);
                    }

                    // open a new instance, if in doubt
                    Arc::new(file)
                } else {
                    // write PID into it
                    let pid = nix::unistd::getpid();
                    let _ = file.write(pid.to_string().as_bytes());

                    Arc::new(file)
                }
            })
            .ok();

        let pa_state = PulseAudioState::init();
        let (tray_builder, stream) = Tray::new();
        let tray = block_on(tray_builder.spawn());

        let backend = match (pa_state, tray.ok()) {
            (Ok(pa_state), tray) => BackendState::Loaded(Backend { pa_state, tray }),
            (Err(e), _) => BackendState::Error(e.to_string()),
        };

        let (_, window_open_task) = iced::window::open(Settings {
            exit_on_close_request: false,
            size: match backend {
                BackendState::Loaded(_) => (600, 300),
                BackendState::Error(_) => (280, 180),
            }
            .into(),
            ..Default::default()
        });

        let this = Self {
            muted: false,
            active: false,
            hotkey_description: "".into(),
            theme: None,
            backend,
            _flock,
            change_hotkey_tx: None,
            recording_hotkey: false,
        };

        // handling signals
        let signal_handler = match Signals::new([signal_hook::consts::SIGUSR1]) {
            Ok(signals) => Task::stream(signals).map(|_| Msg::ShowWindow),
            Err(_) => Task::none(),
        };

        let tasks = Task::batch([
            Task::stream(stream),
            window_open_task.discard(),
            iced::font::load(LUCIDE_FONT_BYTES).discard(),
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
            Msg::ChooseMicrophone(mic) => {
                if let BackendState::Loaded(b) = &mut self.backend {
                    b.pa_state.set_virtual_mic(&mic);
                }
            }
            Msg::SetActive(a) => {
                if let BackendState::Loaded(b) = &mut self.backend {
                    self.active = a;
                    if let Some(tray) = &b.tray {
                        block_on(tray.update(|tray| tray.set_ptt_enabled(a)));
                    }
                    return Task::done(Msg::SetMuted(a));
                }
            }
            Msg::ToggleActive => {
                return Task::done(Msg::SetActive(!self.active));
            }
            Msg::SetMuted(m) => {
                if let BackendState::Loaded(b) = &mut self.backend {
                    let res = b.pa_state.set_mute(self.active && m);
                    if let Err(e) = res {
                        eprintln!("Failed to set mute: {}", e);
                    }
                    self.muted = self.active && m;
                }
            }
            Msg::GlobalShortcutsFail => {
                let msg = "Failed to load global shortcuts. Push-to-talk will not work. Make sure you are using a Wayland compositor with a portal implementation that supports global shortcuts.";
                self.backend = BackendState::Error(msg.into());
            }
            Msg::SetHotKeyDescription(desc) => self.hotkey_description = desc,
            Msg::ShowWindow => {
                let size = match self.backend {
                    BackendState::Loaded(_) => (680, 420),
                    BackendState::Error(_) => (280, 180),
                };

                let task = iced::window::latest().then(move |res| {
                    if res.is_some() {
                        Task::none()
                    } else {
                        iced::window::open(Settings {
                            size: size.into(),
                            ..Default::default()
                        })
                        .1
                        .discard()
                    }
                });

                return task;
            }
            Msg::Close => return iced::window::latest().and_then(iced::window::close),
            Msg::Exit => {
                if let BackendState::Loaded(b) = &mut self.backend {
                    b.pa_state.remove_virtual_mic();
                }
                exit(0);
            }
            Msg::SetTheme(theme) => self.theme = theme,
            Msg::InitChangeHotKeySender(change_hotkey) => {
                self.change_hotkey_tx = Some(change_hotkey)
            }
            Msg::StartHotKeyRecording => {
                self.recording_hotkey = true;
            }
            Msg::StopHotKeyRecording(hk_string) => {
                self.recording_hotkey = false;
                if let (Some(tx), Ok(hk)) =
                    (self.change_hotkey_tx.clone(), HotKey::from_str(&hk_string))
                {
                    return Task::future(async move { tx.send(hk).await }).discard();
                }
            }
            Msg::None => {}
        };
        Task::none()
    }

    pub fn theme(&self, _: Id) -> Option<Theme> {
        //self.theme.clone()
        Some(Theme::KanagawaDragon)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            close_requests().map(|_| Msg::Close),
            Subscription::run(hotkeys),
            if self.recording_hotkey {
                keyboard::listen().map(|k_ev| match k_ev {
                    keyboard::Event::KeyPressed { key, .. } => {
                        let key_str = match key {
                            keyboard::Key::Named(named) => format!("{named:?}"),
                            keyboard::Key::Character(c) => c.into(),
                            keyboard::Key::Unidentified => return Msg::None,
                        };

                        Msg::StopHotKeyRecording(key_str)
                    }
                    _ => Msg::None,
                })
            } else {
                Subscription::none()
            },
        ])
    }

    pub fn view(&self, _window: Id) -> Element<'_, Msg> {
        let backend = match &self.backend {
            BackendState::Loaded(backend) => backend,
            BackendState::Error(e) => return Self::show_error(e.to_string()),
        };

        if self.recording_hotkey {
            return self.recording_hotkey();
        }

        let title = title("Global Push-to-Talk");
        let sep = rule::horizontal(1.0);

        column![
            title,
            sep,
            self.toggle_active(backend),
            self.select_mic(backend),
            space().height(Length::Fill),
            self.hotkey_indicator()
        ]
        .padding(PADDING)
        .spacing(SPACING)
        .into()
    }

    fn recording_hotkey(&self) -> Element<'_, Msg> {
        let txt = text("Enter a key combination...");
        let space1 = space().width(Length::Fill).height(Length::Fill);
        let space2 = space().width(Length::Fill).height(Length::Fill);
        column![space1, txt, space2]
            .align_x(Horizontal::Center)
            .width(Length::Fill)
            .height(Length::Fill)
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

    fn toggle_active(&self, backend: &Backend) -> Element<'_, Msg> {
        if backend.pa_state.get_active_source_name().is_none() {
            return row![
                text("Select a microphone to enable push-to-talk")
                    .font(Font {
                        style: Style::Italic,
                        ..Default::default()
                    })
                    .style(weak_text_style)
            ]
            .spacing(SPACING)
            .padding(PADDING)
            .align_y(Vertical::Center)
            .into();
        }

        let label = text("Enable");
        let checkbox = checkbox(self.active).on_toggle_maybe(
            backend
                .pa_state
                .get_active_source_name()
                .map(|_| Msg::SetActive),
        );

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
        .padding(PADDING)
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
        let italic = Font {
            style: Style::Italic,
            ..Default::default()
        };
        if using_wayland() {
            tooltip(
                text(format!("Hotkey(s): {}", self.hotkey_description))
                    .style(weak_text_style)
                    .font(italic),
                text("Configure these hotkeys in your system's settings")
                    .style(weak_text_style)
                    .font(italic),
                tooltip::Position::Top,
            )
            .into()
        } else {
            let record_btn = button("Change").on_press(Msg::StartHotKeyRecording);
            let txt = text(format!("Hotkey: {}", self.hotkey_description));
            row![txt, record_btn]
                .spacing(SPACING)
                .align_y(Vertical::Center)
                .into()
        }
    }

    fn select_mic(&self, backend: &Backend) -> Element<'_, Msg> {
        let label = text("Microphone");
        let input_devs = backend.pa_state.get_input_devices();
        let selected = input_devs
            .iter()
            .find(|dev| Some(dev.name.as_str()) == backend.pa_state.get_active_source_name())
            .cloned();
        let pick_list = pick_list(input_devs, selected, |dev| Msg::ChooseMicrophone(dev.name))
            .width(Length::Fill)
            .placeholder("Choose Microphone...");

        let refresh_btn = button("⟳").on_press(Msg::None);

        row![label, pick_list, refresh_btn]
            .spacing(SPACING)
            .padding(PADDING)
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .into()
    }
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
