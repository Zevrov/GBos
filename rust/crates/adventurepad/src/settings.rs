//! The app's own folder (%APPDATA%\AdventurePad) and its settings.json.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// %APPDATA%\AdventurePad (or ~/.adventurepad elsewhere): settings, save states, the default saves folder.
pub fn home() -> PathBuf {
    let dir = std::env::var_os("ADVENTUREPAD_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("AdventurePad")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".adventurepad")))
        .unwrap_or_else(|| PathBuf::from("AdventurePad"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// The saves folder the player picked (a synced one for cloud saves); None = [`Settings::default_saves_dir`].
    pub saves_dir: Option<PathBuf>,
    /// Games opened, newest first.
    pub recent: Vec<PathBuf>,
    /// This install's id, for the "open on another device" marker.
    pub device_id: String,
    /// Fast-forward's speed (2 = twice as fast; 0 = as fast as it goes).
    pub ff_speed: f32,
    /// The panels in a window of their own (for a second screen) instead of beside the game.
    pub panel_window: bool,
    /// The developer panels (MEMORY) too.
    pub dev_panels: bool,
    /// Fast-forward plays the game's sound at its normal pitch; false = silent.
    pub ff_sound: bool,
    /// Fast-forward's sound effects and cries at their own pitch (shortened, in step with the game); false =
    /// sped up (higher, as the game makes them).
    pub ff_effects_pitch: bool,
    /// SETTINGS > SCREEN: EXPAND, the overworld drawn past the GBA's edges (for the games it has tables for).
    pub expand: bool,
    /// ZOOM pulls back in for text boxes and menus too (battles always do).
    pub expand_zoom_on_text: bool,
    /// EXPAND draws the area-name banner in the view's top-left corner, not the game frame's (towards the middle).
    pub expand_banner_corner: bool,
    /// The keyboard controls (SETTINGS > CONTROLS).
    pub keys: crate::controls::Keymap,
    /// SETTINGS > HUD: the party, balls and BAG over the game.
    pub hud: crate::hud::HudStyle,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            saves_dir: None,
            recent: Vec::new(),
            device_id: String::new(),
            ff_speed: 3.0,
            panel_window: false,
            dev_panels: false,
            ff_sound: true,
            ff_effects_pitch: true,
            expand: true,
            expand_zoom_on_text: true,
            expand_banner_corner: true,
            keys: Default::default(),
            hud: Default::default(),
        }
    }
}

impl Settings {
    fn file() -> PathBuf {
        home().join("settings.json")
    }

    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read(Self::file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if s.device_id.is_empty() {
            s.device_id = uuid::Uuid::new_v4().to_string();
            s.save();
        }
        s.ff_speed = s.ff_speed.max(0.0);
        s
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = crate::saves::write_atomically(&Self::file(), &json);
        }
    }

    pub fn default_saves_dir() -> PathBuf {
        home().join("saves")
    }

    pub fn saves_dir(&self) -> PathBuf {
        let dir = self
            .saves_dir
            .clone()
            .unwrap_or_else(Self::default_saves_dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub fn add_recent(&mut self, rom: &Path) {
        self.recent.retain(|r| r != rom);
        self.recent.insert(0, rom.to_path_buf());
        self.recent.truncate(20);
        self.save();
    }
}

/// This computer's name, as other devices' warnings say it ("Still open on DESKTOP-4F2K9QX").
pub fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "a PC".into())
}
