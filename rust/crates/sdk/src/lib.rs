//! # Adventure Pad panel SDK
//!
//! The extra screen beside the game is made of **panels**: each one is a tab that reads the running game's
//! memory and draws whatever it likes about it. A **pack** groups panels and says which games they're for.
//!
//! ```no_run
//! use adventurepad_sdk::{egui, GameInfo, Memory, Panel, PanelCtx, PanelPack};
//!
//! /// Shows a counter the game keeps at 0x02000000.
//! struct Counter { value: Option<u32> }
//!
//! impl Panel for Counter {
//!     fn title(&self) -> String { "COUNTER".into() }
//!     fn refresh(&mut self, _game: &GameInfo, memory: &Memory) {
//!         self.value = memory.u32(0x0200_0000);
//!     }
//!     fn ui(&mut self, ui: &mut egui::Ui, _ctx: &mut PanelCtx) {
//!         ui.heading(format!("{:?}", self.value));
//!     }
//! }
//!
//! struct MyPack;
//!
//! impl PanelPack for MyPack {
//!     fn name(&self) -> &str { "My pack" }
//!     fn panels_for(&self, game: &GameInfo) -> Vec<Box<dyn Panel>> {
//!         if game.code == "BPRE" { vec![Box::new(Counter { value: None })] } else { vec![] }
//!     }
//! }
//! ```
//!
//! Register a pack in the app's `panels::packs()` and it shows up for its games. The rules a panel lives by:
//!
//! - [`Panel::refresh`] runs whenever a new copy of the game's memory arrives (4 times a second): read and
//!   decode there, keep what you need in `self`.
//! - [`Panel::ui`] runs every time the screen is drawn: draw from what `refresh` kept. Keep it cheap.
//! - [`Memory`] is a copy, never the live game: reading it can't disturb the game, and a slow panel can't
//!   slow the game down.
//! - To change something in the game, ask through [`PanelCtx::act`] with an [`Action`]: the app carries it
//!   out between frames.
//!
//! [`Memory::from_dumps`] loads the RAM dumps the repo's tests use, so a panel's reads can be unit-tested
//! without a ROM or a window.

pub use egui;

use std::path::Path;
use std::sync::Arc;

/// The app's look, for panels that want to match the built-in ones.
pub mod style {
    use egui::{Color32, RichText, Ui};

    pub const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xD8, 0x4A);
    pub const MUTED: Color32 = Color32::from_rgb(0x8A, 0x86, 0x98);
    pub const GOOD: Color32 = Color32::from_rgb(0x7F, 0xD6, 0x7F);
    pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xB4, 0x5A);
    pub const BAD: Color32 = Color32::from_rgb(0xF0, 0x70, 0x70);
    pub const CARD: Color32 = Color32::from_rgb(0x22, 0x1F, 0x2B);

    /// A section's heading ("MOVES").
    pub fn heading(ui: &mut Ui, text: &str) {
        ui.label(RichText::new(text).strong().color(MUTED));
    }

    /// Grey explanatory text ("Nothing in this pocket.").
    pub fn hint(ui: &mut Ui, text: &str) {
        ui.label(RichText::new(text).color(MUTED));
    }

    /// A bar `width` wide, filled `value / max`: green over half, amber over a fifth, red below (an HP bar).
    pub fn meter(ui: &mut Ui, value: u32, max: u32, width: f32) {
        let frac = if max == 0 {
            0.0
        } else {
            (value as f32 / max as f32).min(1.0)
        };
        let color = if frac > 0.5 {
            GOOD
        } else if frac > 0.2 {
            WARN
        } else {
            BAD
        };
        let (r, _) = ui.allocate_exact_size(egui::vec2(width, 8.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 3.0, Color32::from_gray(45));
        ui.painter().rect_filled(
            egui::Rect::from_min_size(r.min, egui::vec2(width * frac, 8.0)),
            3.0,
            color,
        );
    }

    /// A card that can be clicked (a party slot): `add` fills it.
    pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> egui::Response {
        egui::Frame::group(ui.style())
            .fill(CARD)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui)
            })
            .response
            .interact(egui::Sense::click())
    }
}

/// The GBA's buttons, as [`Action::Press`] takes them (the GB uses the same bits, minus L and R).
pub mod buttons {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const SELECT: u32 = 1 << 2;
    pub const START: u32 = 1 << 3;
    pub const RIGHT: u32 = 1 << 4;
    pub const LEFT: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
    pub const R: u32 = 1 << 8;
    pub const L: u32 = 1 << 9;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Gba,
    /// Game Boy and Game Boy Color.
    Gb,
}

/// The game that's running, from its ROM.
#[derive(Debug, Clone)]
pub struct GameInfo {
    /// The ROM file's name without its extension.
    pub file_name: String,
    /// The header's title (GBA: 12 characters at 0xA0; GB: at 0x134).
    pub header_title: String,
    /// The GBA header's 4-letter game code ("BPRE" is FireRed, US); empty on a Game Boy.
    pub code: String,
    pub revision: u8,
    /// CRC32 of the whole ROM: tells ROM hacks of one game apart.
    pub crc32: u32,
    pub platform: Platform,
    /// The ROM's bytes, as the file has them.
    pub rom: Arc<[u8]>,
}

impl GameInfo {
    /// From a ROM's bytes (and its file name).
    pub fn from_rom(file_name: &str, rom: Arc<[u8]>) -> GameInfo {
        let ascii = |b: &[u8]| -> String {
            b.iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as char)
                .filter(|c| c.is_ascii_graphic() || *c == ' ')
                .collect()
        };
        // A GBA cart has its fixed 0x96 byte at 0xB2; anything else is a Game Boy's.
        let gba = rom.len() >= 0xC0 && rom[0xB2] == 0x96;
        let (header_title, code, revision) = if gba {
            (ascii(&rom[0xA0..0xAC]), ascii(&rom[0xAC..0xB0]), rom[0xBC])
        } else if rom.len() >= 0x150 {
            (ascii(&rom[0x134..0x143]), String::new(), rom[0x14C])
        } else {
            (String::new(), String::new(), 0)
        };
        GameInfo {
            file_name: file_name.to_owned(),
            header_title: header_title.trim().to_owned(),
            code,
            revision,
            crc32: crc32(&rom),
            platform: if gba { Platform::Gba } else { Platform::Gb },
            rom,
        }
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    !bytes.iter().fold(!0u32, |c, &b| {
        table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// A stretch of the game's address space, copied.
#[derive(Debug, Clone)]
pub struct Region {
    pub name: &'static str,
    pub base: u32,
    pub bytes: Arc<[u8]>,
}

/// A copy of the game's memory at one moment: on a GBA, EWRAM (0x02000000), IWRAM (0x03000000) and the
/// ROM (0x08000000); on a Game Boy, WRAM (0xC000), HRAM (0xFF80) and the ROM's first 32 KB (0x0000).
/// All reads are little-endian and bounds-checked: None past the end of what's there.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    regions: Vec<Region>,
    /// Frames the game had run when this was copied.
    pub frame: u64,
}

impl Memory {
    pub fn new(regions: Vec<Region>, frame: u64) -> Memory {
        Memory { regions, frame }
    }

    /// A fixture as the repo's tests keep them: `ewram.bin` and `iwram.bin` in `dir` (a GBA's RAM), with
    /// `rom` mapped at 0x08000000 if given.
    pub fn from_dumps(dir: &Path, rom: Option<Arc<[u8]>>) -> std::io::Result<Memory> {
        let mut regions = vec![
            Region {
                name: "EWRAM",
                base: 0x0200_0000,
                bytes: std::fs::read(dir.join("ewram.bin"))?.into(),
            },
            Region {
                name: "IWRAM",
                base: 0x0300_0000,
                bytes: std::fs::read(dir.join("iwram.bin"))?.into(),
            },
        ];
        if let Some(rom) = rom {
            regions.push(Region {
                name: "ROM",
                base: 0x0800_0000,
                bytes: rom,
            });
        }
        Ok(Memory { regions, frame: 0 })
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// `len` bytes from `addr`, if they're all in one region.
    pub fn slice(&self, addr: u32, len: usize) -> Option<&[u8]> {
        self.regions.iter().find_map(|r| {
            let off = addr.checked_sub(r.base)? as usize;
            r.bytes.get(off..off.checked_add(len)?)
        })
    }

    /// Fills `out` from `addr`; false if it isn't all there.
    pub fn read(&self, addr: u32, out: &mut [u8]) -> bool {
        match self.slice(addr, out.len()) {
            Some(s) => {
                out.copy_from_slice(s);
                true
            }
            None => false,
        }
    }

    pub fn u8(&self, addr: u32) -> Option<u8> {
        self.slice(addr, 1).map(|s| s[0])
    }

    pub fn u16(&self, addr: u32) -> Option<u16> {
        self.slice(addr, 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
    }

    pub fn u32(&self, addr: u32) -> Option<u32> {
        self.slice(addr, 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

/// Something a panel asks the app to do in the game. Carried out between frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Save a state into slot 0..=9 (the STATES tab's).
    SaveState(u8),
    LoadState(u8),
    /// Hold `buttons` ([`buttons`] bits) for `frames` frames, on top of the player's own.
    Press {
        buttons: u32,
        frames: u32,
    },
    /// A short message over the game ("Copied!").
    Notice(String),
}

/// What a panel can use while it draws.
pub struct PanelCtx<'a> {
    pub game: &'a GameInfo,
    /// The newest copy of the game's memory (None before the first arrives).
    pub memory: Option<&'a Memory>,
    actions: &'a mut Vec<Action>,
}

impl<'a> PanelCtx<'a> {
    pub fn new(
        game: &'a GameInfo,
        memory: Option<&'a Memory>,
        actions: &'a mut Vec<Action>,
    ) -> PanelCtx<'a> {
        PanelCtx {
            game,
            memory,
            actions,
        }
    }

    /// Asks the app to do `action` in the game.
    pub fn act(&mut self, action: Action) {
        self.actions.push(action);
    }
}

/// One tab on the extra screen.
pub trait Panel {
    /// The tab's label (short, upper case reads best: "PARTY").
    fn title(&self) -> String;

    /// A new copy of the game's memory: read the game here and keep what [`Panel::ui`] needs.
    fn refresh(&mut self, game: &GameInfo, memory: &Memory) {
        let _ = (game, memory);
    }

    /// Draws the panel (every time the screen is drawn: keep it cheap).
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut PanelCtx);

    /// Whether its tab shows right now (a BATTLE panel: only in a battle).
    fn visible(&self) -> bool {
        true
    }

    /// Asks to be the open tab (a BATTLE panel, as a battle starts). The app opens it while this is true and
    /// goes back to the tab before once it's false again; the player's own pick wins meanwhile.
    fn wants_focus(&self) -> bool {
        false
    }
}

/// A set of panels for the games it knows.
pub trait PanelPack {
    /// Shown in the app's PANELS list.
    fn name(&self) -> &str;

    /// The panels for `game`, in tab order; empty if this pack doesn't know it.
    fn panels_for(&self, game: &GameInfo) -> Vec<Box<dyn Panel>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_inside_regions_only() {
        let m = Memory::new(
            vec![Region {
                name: "EWRAM",
                base: 0x0200_0000,
                bytes: vec![1, 2, 3, 4, 5].into(),
            }],
            0,
        );
        assert_eq!(m.u32(0x0200_0000), Some(0x0403_0201));
        assert_eq!(m.u16(0x0200_0003), Some(0x0504));
        assert_eq!(m.u32(0x0200_0002), None, "runs past the end");
        assert_eq!(m.u8(0x0100_0000), None, "before the region");
    }

    #[test]
    fn tells_gba_from_gb() {
        let mut rom = vec![0u8; 0x200];
        rom[0xA0..0xA8].copy_from_slice(b"POKEMON ");
        rom[0xAC..0xB0].copy_from_slice(b"BPRE");
        rom[0xB2] = 0x96;
        rom[0xBC] = 1;
        let g = GameInfo::from_rom("FireRed", rom.into());
        assert_eq!(
            (
                g.platform,
                g.code.as_str(),
                g.revision,
                g.header_title.as_str()
            ),
            (Platform::Gba, "BPRE", 1, "POKEMON")
        );
        let mut gb = vec![0u8; 0x200];
        gb[0x134..0x13B].copy_from_slice(b"POKEMON");
        let g = GameInfo::from_rom("Red", gb.into());
        assert_eq!(
            (g.platform, g.header_title.as_str()),
            (Platform::Gb, "POKEMON")
        );
    }

    #[test]
    fn crc_matches_the_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
