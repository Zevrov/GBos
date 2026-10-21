//! The keyboard controls: which key does what, rebindable in SETTINGS > CONTROLS and kept in settings.json
//! by key name ("Z", "ArrowUp").

use eframe::egui::Key;
use mgba_sys::key;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Everything a key can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Control {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    L,
    R,
    Start,
    Select,
    FastForwardHold,
    FastForwardToggle,
    SaveState,
    LoadState,
    PrevSlot,
    NextSlot,
    /// EXPAND's ZOOM: the next step out.
    Zoom,
    /// EXPAND's look-around (the Android app's right stick).
    LookUp,
    LookDown,
    LookLeft,
    LookRight,
}

impl Control {
    /// In the order SETTINGS lists them.
    pub const ALL: [Control; 21] = [
        Control::Up,
        Control::Down,
        Control::Left,
        Control::Right,
        Control::A,
        Control::B,
        Control::L,
        Control::R,
        Control::Start,
        Control::Select,
        Control::FastForwardHold,
        Control::FastForwardToggle,
        Control::SaveState,
        Control::LoadState,
        Control::PrevSlot,
        Control::NextSlot,
        Control::Zoom,
        Control::LookUp,
        Control::LookDown,
        Control::LookLeft,
        Control::LookRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Control::Up => "UP",
            Control::Down => "DOWN",
            Control::Left => "LEFT",
            Control::Right => "RIGHT",
            Control::A => "A",
            Control::B => "B",
            Control::L => "L",
            Control::R => "R",
            Control::Start => "START",
            Control::Select => "SELECT",
            Control::FastForwardHold => "FAST-FORWARD (hold)",
            Control::FastForwardToggle => "FAST-FORWARD (on / off)",
            Control::SaveState => "SAVE STATE",
            Control::LoadState => "LOAD STATE",
            Control::PrevSlot => "PREVIOUS SLOT",
            Control::NextSlot => "NEXT SLOT",
            Control::Zoom => "ZOOM (EXPAND)",
            Control::LookUp => "LOOK UP (EXPAND)",
            Control::LookDown => "LOOK DOWN (EXPAND)",
            Control::LookLeft => "LOOK LEFT (EXPAND)",
            Control::LookRight => "LOOK RIGHT (EXPAND)",
        }
    }

    /// The game button it holds, for the ones that are game buttons.
    pub fn button(self) -> Option<u32> {
        Some(match self {
            Control::Up => key::UP,
            Control::Down => key::DOWN,
            Control::Left => key::LEFT,
            Control::Right => key::RIGHT,
            Control::A => key::A,
            Control::B => key::B,
            Control::L => key::L,
            Control::R => key::R,
            Control::Start => key::START,
            Control::Select => key::SELECT,
            _ => return None,
        })
    }

    fn default_key(self) -> Key {
        match self {
            Control::Up => Key::ArrowUp,
            Control::Down => Key::ArrowDown,
            Control::Left => Key::ArrowLeft,
            Control::Right => Key::ArrowRight,
            Control::A => Key::Z,
            Control::B => Key::X,
            Control::L => Key::A,
            Control::R => Key::S,
            Control::Start => Key::Enter,
            Control::Select => Key::Backspace,
            Control::FastForwardHold => Key::Tab,
            Control::FastForwardToggle => Key::Space,
            Control::SaveState => Key::F5,
            Control::LoadState => Key::F7,
            Control::PrevSlot => Key::F6,
            Control::NextSlot => Key::F8,
            Control::Zoom => Key::Q,
            Control::LookUp => Key::I,
            Control::LookDown => Key::K,
            Control::LookLeft => Key::J,
            Control::LookRight => Key::L,
        }
    }
}

/// Control -> key name. A control missing from it has its default; one mapped to "" has no key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keymap(BTreeMap<Control, String>);

impl Keymap {
    pub fn key(&self, c: Control) -> Option<Key> {
        match self.0.get(&c) {
            Some(name) => Key::from_name(name),
            None => Some(c.default_key()),
        }
    }

    /// The key's name for SETTINGS ("none" when unbound).
    pub fn key_label(&self, c: Control) -> String {
        self.key(c)
            .map_or_else(|| "none".to_owned(), |k| k.name().to_owned())
    }

    /// Binds `k` to `c`. Whatever else had `k` loses it (one key, one job), and is returned.
    pub fn bind(&mut self, c: Control, k: Key) -> Option<Control> {
        let taken = Control::ALL
            .into_iter()
            .find(|&o| o != c && self.key(o) == Some(k));
        if let Some(o) = taken {
            self.0.insert(o, String::new());
        }
        self.0.insert(c, k.name().to_owned());
        taken
    }

    pub fn unbind(&mut self, c: Control) {
        self.0.insert(c, String::new());
    }

    pub fn reset(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_until_rebound() {
        let mut m = Keymap::default();
        assert_eq!(m.key(Control::A), Some(Key::Z));
        assert_eq!(m.bind(Control::A, Key::G), None);
        assert_eq!(m.key(Control::A), Some(Key::G));
        m.reset();
        assert_eq!(m.key(Control::A), Some(Key::Z));
    }

    #[test]
    fn a_taken_key_moves() {
        let mut m = Keymap::default();
        // X is B's: binding it to A leaves B with no key.
        assert_eq!(m.bind(Control::A, Key::X), Some(Control::B));
        assert_eq!(m.key(Control::A), Some(Key::X));
        assert_eq!(m.key(Control::B), None);
        assert_eq!(m.key_label(Control::B), "none");
        let on_x: Vec<Control> = Control::ALL
            .into_iter()
            .filter(|&c| m.key(c) == Some(Key::X))
            .collect();
        assert_eq!(on_x, [Control::A], "one key, one job");
    }

    #[test]
    fn survives_settings_json() {
        let mut m = Keymap::default();
        m.bind(Control::Start, Key::P);
        m.unbind(Control::Select);
        let json = serde_json::to_string(&m).unwrap();
        let back: Keymap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.key(Control::Start), Some(Key::P));
        assert_eq!(back.key(Control::Select), None);
        assert_eq!(back.key(Control::Up), Some(Key::ArrowUp));
    }
}
