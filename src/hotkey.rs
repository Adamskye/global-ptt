use std::sync::Arc;

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey},
    wayland::{WlHotKeysChangedEvent, WlNewHotKeyAction, using_wayland},
};
use iced::{
    futures::{FutureExt, SinkExt, Stream},
    stream,
};
use tokio::sync::{Mutex, mpsc};

use crate::{APP_ID, app::Msg, config::Config};

const WL_HOTKEY_ID: u32 = 0;

pub fn hotkeys() -> impl Stream<Item = Msg> {
    stream::channel(100, async |mut tx| {
        let Ok(gh) = GlobalHotKeyManager::new() else {
            let _ = tx.send(Msg::GlobalShortcutsFail).await;
            return;
        };

        let default_hotkey = HotKey::new(None, Code::Insert);
        if using_wayland() {
            if gh
                .wl_register_all(
                    APP_ID,
                    &[WlNewHotKeyAction::new(
                        WL_HOTKEY_ID,
                        "Activate push-to-talk",
                        Some(default_hotkey),
                    )],
                )
                .is_err()
            {
                let _ = tx.send(Msg::GlobalShortcutsFail).await;
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

            let mut hk_change_tx = tx.clone();
            tokio::spawn(async move {
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
                                let _ = hk_change_tx
                                    .send(Msg::SetHotKeyDescription(change.hotkey_description))
                                    .await;
                                break;
                            }
                        }
                        _ => return,
                    }
                }
            });

            let hk_event_rx = GlobalHotKeyEvent::receiver();
            while let Ok(Ok(msg)) = tokio::task::spawn_blocking(|| hk_event_rx.recv()).await {
                if msg.id() != WL_HOTKEY_ID {
                    continue;
                }

                let _ = tx
                    .send(Msg::SetMuted(msg.state() == HotKeyState::Released))
                    .now_or_never();
            }
        } else {
            // if not using Wayland
            let (hk_change_tx, mut hk_change_rx) = mpsc::channel::<HotKey>(100);
            let _ = tx.send(Msg::InitChangeHotKeySender(hk_change_tx)).await;

            let mut config = Config::load().unwrap_or_default();
            let hk = Arc::new(Mutex::new(config.get_hotkey()));

            let _ = gh.register(*hk.lock().await);
            let _ = tx
                .send(Msg::SetHotKeyDescription(hk.lock().await.into_string()))
                .await;

            let hk_event_rx = GlobalHotKeyEvent::receiver();

            // handling hotkey changes
            let current_hk = hk.clone();
            let mut hk_event_tx = tx.clone();
            tokio::spawn(async move {
                while let Some(new_hk) = hk_change_rx.recv().await {
                    let mut current_hk = current_hk.lock().await;
                    let _ = gh.unregister(*current_hk);
                    let _ = gh.register(new_hk);
                    config.set_hotkey(new_hk);
                    *current_hk = new_hk;
                    let _ = hk_event_tx
                        .send(Msg::SetHotKeyDescription(current_hk.into_string()))
                        .await;
                }
            });

            // polling for hotkey events
            while let Ok(Ok(msg)) = tokio::task::spawn_blocking(|| hk_event_rx.recv()).await {
                if msg.id() != hk.lock().await.id() {
                    continue;
                }

                let _ = tx
                    .send(Msg::SetMuted(msg.state() == HotKeyState::Released))
                    .now_or_never();
            }
        };
    })
}
