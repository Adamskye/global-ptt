use global_hotkey::{
    hotkey::{Code, HotKey},
    wayland::{using_wayland, WlHotKeysChangedEvent, WlNewHotKeyAction},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use iced::{
    futures::{FutureExt, SinkExt, Stream},
    stream,
};

use crate::{app::Msg, config::Config, APP_ID, WL_HOTKEY_ID};

pub fn hotkeys_wl_change() -> impl Stream<Item = Msg> {
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

pub fn hotkeys() -> impl Stream<Item = Msg> {
    stream::channel(100, async |mut tx| {
        let Ok(gh) = GlobalHotKeyManager::new() else {
            let _ = tx.send(Msg::GlobalShortcutsFail).await;
            return;
        };

        let default_hotkey = HotKey::new(None, Code::Insert);
        let hk_id = if using_wayland() {
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

            WL_HOTKEY_ID
        } else {
            // if not using Wayland
            let config = Config::load().unwrap_or_default();
            let hk = config.get_hotkey();

            let _ = gh.register(hk);
            let _ = tx.send(Msg::SetHotKeyDescription(hk.into_string())).await;

            config.save();
            hk.id()
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
