use std::str::FromStr;

use confy::ConfyError;
use global_hotkey::hotkey::HotKey;
use serde::{Deserialize, Serialize};

use crate::hotkey::HotKeyConfig;

const APP_NAME: &str = "global-push-to-talk";

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    trigger_hotkey: Option<String>,
    toggle_active_hotkey: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self, ConfyError> {
        let config: Self = confy::load(APP_NAME, Some("config"))?;
        Ok(config)
    }

    pub fn hotkeys(&self) -> HotKeyConfig<HotKey> {
        let default = HotKeyConfig::default();
        let trigger = self
            .trigger_hotkey
            .as_deref()
            .and_then(|t| HotKey::from_str(t).ok())
            .unwrap_or(default.trigger);
        let toggle_active = self
            .toggle_active_hotkey
            .as_deref()
            .and_then(|t| HotKey::from_str(t).ok())
            .unwrap_or(default.toggle_active);

        HotKeyConfig {
            trigger,
            toggle_active,
        }
    }

    pub fn store_hotkeys(&mut self, hotkeys: &HotKeyConfig<HotKey>) {
        self.trigger_hotkey = Some(hotkeys.trigger.into_string());
        self.toggle_active_hotkey = Some(hotkeys.toggle_active.into_string());
        let _ = confy::store(APP_NAME, Some("config"), self);
    }
}
