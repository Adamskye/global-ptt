use std::str::FromStr;

use confy::ConfyError;
use global_hotkey::hotkey::{Code, HotKey};
use serde::{Deserialize, Serialize};

const APP_NAME: &str = "global-push-to-talk";

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    pub hotkey: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self, ConfyError> {
        let config: Self = confy::load(APP_NAME, Some("config"))?;
        config.save();
        Ok(config)
    }

    fn save(&self) {
        let _ = confy::store(APP_NAME, Some("config"), self);
    }

    pub fn get_hotkey(&self) -> HotKey {
        self.hotkey
            .as_deref()
            .and_then(|hk| HotKey::from_str(hk).ok())
            .unwrap_or(HotKey::new(None, Code::Insert))
    }

    pub fn set_hotkey(&mut self, hotkey: HotKey) {
        self.hotkey = Some(hotkey.into_string());
        self.save();
    }
}
