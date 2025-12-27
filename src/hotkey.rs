use std::sync::Arc;

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
    wayland::{WlHotKeysChangedEvent, WlNewHotKeyAction, using_wayland},
};
use iced::{
    futures::{FutureExt, SinkExt, Stream, channel::mpsc::Sender},
    stream,
};
use tokio::sync::{Mutex, mpsc};

use crate::{APP_ID, app::Msg, config::Config};

const WL_TRIGGER_ID: u32 = 0;
const WL_TOGGLE_ACTIVE_ID: u32 = 1;

// used to store any data corresponding to each type of hotkey
#[derive(Debug, Clone)]
pub struct HotKeyConfig<T> {
    pub trigger: T,
    pub toggle_active: T,
}

impl Default for HotKeyConfig<HotKey> {
    fn default() -> Self {
        Self {
            trigger: HotKey::new(None, Code::Insert),
            toggle_active: HotKey::new(Some(Modifiers::CONTROL | Modifiers::SUPER), Code::KeyP),
        }
    }
}

impl Default for HotKeyConfig<String> {
    fn default() -> Self {
        Self {
            trigger: String::default(),
            toggle_active: String::default(),
        }
    }
}

async fn hotkeys_wl(gh: GlobalHotKeyManager, tx: Sender<Msg>) -> anyhow::Result<()> {
    let trigger_hk = WlNewHotKeyAction::new(
        WL_TRIGGER_ID,
        "Push-to-talk trigger/unmute microphone",
        Some(HotKey::new(None, Code::Insert)),
    );

    let toggle_active_hk = WlNewHotKeyAction::new(
        WL_TOGGLE_ACTIVE_ID,
        "Enable/disable push-to-talk",
        Some(HotKey::new(
            Some(Modifiers::CONTROL | Modifiers::SUPER),
            Code::KeyP,
        )),
    );

    gh.wl_register_all(APP_ID, &[trigger_hk, toggle_active_hk])?;

    // react to user changing the hotkeys
    let mut msg_tx = tx.clone();
    tokio::task::spawn(async move {
        loop {
            // set hotkey descriptions
            let mut d = HotKeyConfig::default();
            for hk in gh.wl_get_hotkeys() {
                let hk_desc = hk.hotkey_description().into();
                match hk.id() {
                    WL_TRIGGER_ID => d.trigger = hk_desc,
                    WL_TOGGLE_ACTIVE_ID => d.toggle_active = hk_desc,
                    _ => (),
                }
            }
            let _ = msg_tx.send(Msg::UpdateHotKeyDescriptions(d)).await;

            let Some(rec) = WlHotKeysChangedEvent::receiver() else {
                return;
            };

            // wait for hotkey change event
            if !matches!(
                tokio::task::spawn_blocking(move || rec.recv()).await,
                Ok(Ok(_))
            ) {
                return;
            }
        }
    });

    // handle hotkey events
    let hk_event_rx = GlobalHotKeyEvent::receiver();
    let hotkey_ids = HotKeyConfig {
        trigger: WL_TRIGGER_ID,
        toggle_active: WL_TOGGLE_ACTIVE_ID,
    };
    while let Ok(Ok(event)) = tokio::task::spawn_blocking(|| hk_event_rx.recv()).await {
        handle_hotkey_press(tx.clone(), event, &hotkey_ids);
    }

    Ok(())
}

async fn hotkeys_non_wl(gh: GlobalHotKeyManager, mut tx: Sender<Msg>) -> anyhow::Result<()> {
    // channel for UI to send hotkey updates through
    let (change_hotkey_tx, mut change_hotkey_rx) = mpsc::channel(10);
    let _ = tx.send(Msg::InitChangeHotKeyTX(change_hotkey_tx)).await;

    // load our hotkeys
    let config_outer = Arc::new(Mutex::new(Config::load().unwrap_or_default()));

    // handle hotkey changes from UI
    let mut msg_tx = tx.clone();
    let config = config_outer.clone();
    tokio::spawn(async move {
        loop {
            // set up hotkeys
            {
                let mut config = config.lock().await;
                let hks = config.hotkeys();

                // register the hotkeys
                let _ = gh.register(hks.trigger);
                let _ = gh.register(hks.toggle_active);

                // update description in UI
                let _ = msg_tx
                    .send(Msg::UpdateHotKeyDescriptions(HotKeyConfig {
                        trigger: hks.trigger.into_string(),
                        toggle_active: hks.toggle_active.into_string(),
                    }))
                    .await;

                // save config
                config.store_hotkeys(&hks);
            }

            // update hotkeys whenever one is changed
            if let Some(change) = change_hotkey_rx.recv().await {
                // unregister old hotkeys
                let mut config = config.lock().await;
                let hks = config.hotkeys();
                let _ = gh.unregister(hks.trigger);
                let _ = gh.unregister(hks.toggle_active);

                config.store_hotkeys(&change);
            } else {
                return;
            }
        }
    });

    // handle hotkey events
    let hk_event_rx = GlobalHotKeyEvent::receiver();
    while let Ok(Ok(event)) = tokio::task::spawn_blocking(|| hk_event_rx.recv()).await {
        let config = config_outer.lock().await;
        let hks = config.hotkeys();
        let ids = HotKeyConfig {
            trigger: hks.trigger.id(),
            toggle_active: hks.toggle_active.id(),
        };
        handle_hotkey_press(tx.clone(), event, &ids);
    }
    Ok(())
}

fn handle_hotkey_press(
    mut tx: Sender<Msg>,
    event: GlobalHotKeyEvent,
    hotkey_ids: &HotKeyConfig<u32>,
) {
    let id = event.id();
    let _ = tx
        .send(if id == hotkey_ids.trigger {
            Msg::SetMuted(event.state() == HotKeyState::Released)
        } else if id == hotkey_ids.toggle_active && event.state() == HotKeyState::Pressed {
            Msg::ToggleActive
        } else {
            return;
        })
        .now_or_never();
}

pub fn hotkeys() -> impl Stream<Item = Msg> {
    stream::channel(100, async |mut tx| {
        let Ok(gh) = GlobalHotKeyManager::new() else {
            let _ = tx.send(Msg::GlobalShortcutsFail).await;
            return;
        };

        let res = if using_wayland() {
            hotkeys_wl(gh, tx.clone()).await
        } else {
            hotkeys_non_wl(gh, tx.clone()).await
        };

        if res.is_err() {
            let _ = tx.send(Msg::GlobalShortcutsFail).await;
        }
    })
}
