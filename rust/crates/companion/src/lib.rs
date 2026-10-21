//! Adventure Pad's Pokémon companion, in Rust: reads the running game's RAM (party, battle, bag, money, place) and works
//! out what the companion shows. A port of the app's Kotlin readers (`companion/data`: NativeReader, Gen3Mon)
//! by way of the 3DS port's C (`3ds/core`): same addresses, same decoding. Retail FireRed / LeafGreen /
//! Emerald so far.

pub mod battle;
pub mod game;
pub mod snapshot;
pub mod tables;
#[rustfmt::skip]
mod tables_gen;
pub mod text;

pub use game::{Game, GameKind};
pub use snapshot::{BattleMon, Gender, Mon, Pocket, Snapshot};

/// Reads the GBA bus: RAM (0x02xxxxxx / 0x03xxxxxx) and the ROM (0x08xxxxxx).
pub trait Bus {
    /// Fills `out` from `addr`; false if it can't (the bytes are then unspecified).
    fn read(&mut self, addr: u32, out: &mut [u8]) -> bool;

    fn read_u8(&mut self, addr: u32) -> Option<u8> {
        let mut b = [0u8; 1];
        self.read(addr, &mut b).then_some(b[0])
    }

    fn read_u32(&mut self, addr: u32) -> Option<u32> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b).then(|| u32::from_le_bytes(b))
    }
}

/// A RAM dump as a [`Bus`]: the repo's fixtures (`ewram.bin` at 0x02000000, `iwram.bin` at 0x03000000) and,
/// optionally, the ROM's bytes at 0x08000000.
pub struct RamImage {
    pub ewram: Vec<u8>,
    pub iwram: Vec<u8>,
    pub rom: Vec<u8>,
}

impl RamImage {
    pub fn load(dir: &std::path::Path) -> std::io::Result<RamImage> {
        Ok(RamImage {
            ewram: std::fs::read(dir.join("ewram.bin"))?,
            iwram: std::fs::read(dir.join("iwram.bin"))?,
            rom: Vec::new(),
        })
    }
}

impl Bus for RamImage {
    fn read(&mut self, addr: u32, out: &mut [u8]) -> bool {
        let (mem, base) = match addr >> 24 {
            0x02 => (&self.ewram, 0x0200_0000),
            0x03 => (&self.iwram, 0x0300_0000),
            0x08 | 0x09 => (&self.rom, 0x0800_0000),
            _ => return false,
        };
        let off = (addr - base) as usize;
        match mem.get(off..off + out.len()) {
            Some(src) => {
                out.copy_from_slice(src);
                true
            }
            None => false,
        }
    }
}
