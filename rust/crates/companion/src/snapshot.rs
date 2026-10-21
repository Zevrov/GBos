//! One read of the running game: party, battle, bag, money and place - 3ds/core/pd_snapshot.c in Rust (the
//! app's readNativeTelemetry, NativeReader.kt, with Gen3Mon.kt's decoding).

use crate::game::{Config, Game, GameKind};
use crate::tables::{self, SPECIES_EGG};
use crate::{text, Bus};

const MON_SIZE: usize = 100;
const BATTLE_MON_SIZE: usize = 0x58;
const PARTY_SIZE: usize = 6;
const POCKET_SLOTS: usize = 64;
const MAP_HEADER_MAPSEC_OFF: u32 = 0x14;
const SAVEBLOCK2_GENDER_OFF: u32 = 8;
const BATTLE_TYPE_DOUBLE: u32 = 0x0001;
const BATTLE_TYPE_TRAINER: u32 = 0x0008;
const MAX_MONEY: u32 = 999_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gender {
    None,
    Male,
    Female,
}

impl Gender {
    pub fn symbol(self) -> &'static str {
        match self {
            Gender::None => "",
            Gender::Male => "♂",
            Gender::Female => "♀",
        }
    }
}

/// The bag's pockets, in a fixed order of our own (the app's POCKET_* ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pocket {
    Items = 0,
    Balls = 1,
    TmHm = 2,
    Berries = 3,
    Key = 4,
}

impl Pocket {
    pub const ALL: [Pocket; 5] = [
        Pocket::Items,
        Pocket::Balls,
        Pocket::TmHm,
        Pocket::Berries,
        Pocket::Key,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pocket::Items => "ITEMS",
            Pocket::Balls => "POKé BALLS",
            Pocket::TmHm => "TMs & HMs",
            Pocket::Berries => "BERRIES",
            Pocket::Key => "KEY ITEMS",
        }
    }
}

/// HP, ATTACK, DEFENSE, SPEED, SP. ATK, SP. DEF - the order the game stores IVs / EVs in.
pub type Stats = [u16; 6];

#[derive(Debug, Clone, PartialEq)]
pub struct Mon {
    /// Internal species id; [`SPECIES_EGG`] for an egg.
    pub species: u16,
    pub nickname: String,
    /// The party menu's mark (none for eggs, genderless species, a "NIDORAN♀" still named so).
    pub gender: Gender,
    pub level: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub status: u32,
    pub moves: [u16; 4],
    pub pp: [u8; 4],
    pub personality: u32,
    pub held_item: u16,
    pub experience: u32,
    /// The party struct's battle stats: ATTACK, DEFENSE, SPEED, SP. ATK, SP. DEF (HP is [`Mon::max_hp`]).
    pub stats: [u16; 5],
    /// HP, ATTACK, DEFENSE, SPEED, SP. ATK, SP. DEF.
    pub ivs: [u8; 6],
    pub evs: [u8; 6],
}

impl Mon {
    pub fn is_egg(&self) -> bool {
        self.species == SPECIES_EGG
    }

    pub fn nature(&self) -> &'static str {
        NATURES[(self.personality % 25) as usize]
    }

    /// The status condition's label, or None for a healthy one (or fainted: see [`Mon::hp`]).
    pub fn status_label(&self) -> Option<&'static str> {
        status_label(self.status)
    }
}

pub fn status_label(status: u32) -> Option<&'static str> {
    if status & 0x7 != 0 {
        Some("SLP")
    } else if status & 0x8 != 0 || status & 0x80 != 0 {
        Some("PSN")
    } else if status & 0x10 != 0 {
        Some("BRN")
    } else if status & 0x20 != 0 {
        Some("FRZ")
    } else if status & 0x40 != 0 {
        Some("PAR")
    } else {
        None
    }
}

pub const NATURES: [&str; 25] = [
    "HARDY", "LONELY", "BRAVE", "ADAMANT", "NAUGHTY", "BOLD", "DOCILE", "RELAXED", "IMPISH", "LAX",
    "TIMID", "HASTY", "SERIOUS", "JOLLY", "NAIVE", "MODEST", "MILD", "QUIET", "BASHFUL", "RASH",
    "CALM", "GENTLE", "SASSY", "CAREFUL", "QUIRKY",
];

/// A battler as the battle has it (gBattleMons): its types and stat stages are the battle's.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleMon {
    pub species: u16,
    pub level: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub types: (u8, u8),
    pub status: u32,
    pub moves: [u16; 4],
    pub pp: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: u16,
    pub quantity: u16,
}

/// Battler positions: the player's left, the foe's left, then the doubles partners.
pub const POS_PLAYER_LEFT: usize = 0;
pub const POS_OPPONENT_LEFT: usize = 1;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub party: Vec<Mon>,
    pub in_battle: bool,
    pub is_double: bool,
    pub is_trainer: bool,
    /// By battler position ([`POS_PLAYER_LEFT`], ...); None = no one there.
    pub battlers: [Option<BattleMon>; 4],
    /// A trainer's team (gEnemyParty), the one out, and the one about to come in (once the one out has
    /// fainted and that pick is alive).
    pub foes: Vec<Mon>,
    pub foe_active: Option<usize>,
    pub foe_next: Option<usize>,
    pub mapsec: u8,
    pub money: Option<u32>,
    /// By [`Pocket`] index; None when the bag couldn't be read.
    pub bag: Option<[Vec<Item>; 5]>,
    /// SaveBlock2.playerGender: 0 boy, 1 girl.
    pub player_gender: Option<u8>,
    /// gMain's vblank counter: moves on while the game runs.
    pub frame: u32,
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn in_ram(a: u32) -> bool {
    (0x0200_0000..0x0400_0000).contains(&a)
}

/// gen3SubstructOrder[personality % 24][slot] = logical substruct (0 Growth, 1 Attacks, 2 EVs, 3 Misc).
const SUBSTRUCT_ORDER: [[u8; 4]; 24] = [
    [0, 1, 2, 3],
    [0, 1, 3, 2],
    [0, 2, 1, 3],
    [0, 2, 3, 1],
    [0, 3, 1, 2],
    [0, 3, 2, 1],
    [1, 0, 2, 3],
    [1, 0, 3, 2],
    [1, 2, 0, 3],
    [1, 2, 3, 0],
    [1, 3, 0, 2],
    [1, 3, 2, 0],
    [2, 0, 1, 3],
    [2, 0, 3, 1],
    [2, 1, 0, 3],
    [2, 1, 3, 0],
    [2, 3, 0, 1],
    [2, 3, 1, 0],
    [3, 0, 1, 2],
    [3, 0, 2, 1],
    [3, 1, 0, 2],
    [3, 1, 2, 0],
    [3, 2, 0, 1],
    [3, 2, 1, 0],
];

/// GetMonGender, and DisplayPartyPokemonGender's rule that a Nidoran still named "NIDORAN♀" / "NIDORAN♂"
/// shows no mark (its name has one): the app's withNativeGender.
fn gender_of(species: u16, nickname: &str, personality: u32) -> Gender {
    if species == SPECIES_EGG {
        return Gender::None;
    }
    if (species == 29 && nickname == "NIDORAN♀") || (species == 32 && nickname == "NIDORAN♂") {
        return Gender::None;
    }
    match tables::gender_ratio(species) {
        None | Some(255) => Gender::None,
        Some(0) => Gender::Male,
        Some(254) => Gender::Female,
        Some(r) => {
            if ((personality & 0xFF) as u8) < r {
                Gender::Female
            } else {
                Gender::Male
            }
        }
    }
}

/// A party struct (100 bytes): decrypts and checks the box data; None for an empty or corrupt slot.
pub fn decode_party_mon(raw: &[u8]) -> Option<Mon> {
    if raw.len() < MON_SIZE {
        return None;
    }
    let personality = u32_at(raw, 0);
    let key = personality ^ u32_at(raw, 4);
    let stored = u16_at(raw, 28);
    let mut block = [0u8; 48];
    let mut sum: u16 = 0;
    for i in 0..12 {
        let w = u32_at(raw, 0x20 + i * 4) ^ key;
        block[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        sum = sum
            .wrapping_add((w & 0xFFFF) as u16)
            .wrapping_add((w >> 16) as u16);
    }
    // Retail boxes are encrypted and checksummed; an empty slot fails here.
    if sum != stored || stored == 0 {
        return None;
    }
    let mut off = [0usize; 4];
    for (slot, &logical) in SUBSTRUCT_ORDER[(personality % 24) as usize]
        .iter()
        .enumerate()
    {
        off[logical as usize] = slot * 12;
    }
    let (growth, attacks, evs, misc) = (off[0], off[1], off[2], off[3]);
    let mut species = u16_at(&block, growth);
    if species == 0 {
        return None;
    }
    let mut moves = [0u16; 4];
    let mut pp = [0u8; 4];
    for i in 0..4 {
        moves[i] = u16_at(&block, attacks + i * 2);
        pp[i] = block[attacks + 8 + i];
    }
    let iv_word = u32_at(&block, misc + 4);
    let ivs = std::array::from_fn(|i| ((iv_word >> (i * 5)) & 0x1F) as u8);
    let evs = std::array::from_fn(|i| block[evs + i]);
    // BoxPokemon +0x13's flags (isBadEgg, hasSpecies, isEgg): an egg shows as one, like the party menu.
    if raw[0x13] & 0x04 != 0 {
        species = SPECIES_EGG;
    }
    let nickname = text::decode(&raw[0x08..0x12]);
    Some(Mon {
        species,
        gender: gender_of(species, &nickname, personality),
        nickname,
        level: raw[0x54],
        hp: u16_at(raw, 0x56),
        max_hp: u16_at(raw, 0x58),
        status: u32_at(raw, 0x50),
        moves,
        pp,
        personality,
        held_item: u16_at(&block, growth + 2),
        experience: u32_at(&block, growth + 4),
        stats: std::array::from_fn(|i| u16_at(raw, 0x5A + i * 2)),
        ivs,
        evs,
    })
}

fn decode_battle_mon(raw: &[u8]) -> Option<BattleMon> {
    let species = u16_at(raw, 0);
    if species == 0 {
        return None;
    }
    Some(BattleMon {
        species,
        moves: std::array::from_fn(|i| u16_at(raw, 0x0C + i * 2)),
        pp: std::array::from_fn(|i| raw[0x24 + i]),
        types: (raw[0x21], raw[0x22]),
        hp: u16_at(raw, 0x28),
        level: raw[0x2A],
        max_hp: u16_at(raw, 0x2C),
        status: u32_at(raw, 0x4C),
    })
}

fn read_mons(bus: &mut dyn Bus, addr: u32, count: usize) -> Vec<Mon> {
    let mut raw = vec![0u8; count * MON_SIZE];
    if count == 0 || !bus.read(addr, &mut raw) {
        return Vec::new();
    }
    raw.chunks(MON_SIZE).filter_map(decode_party_mon).collect()
}

impl Snapshot {
    /// Reads `game` off `bus`. None for a game with no [`Config`].
    pub fn read(game: &Game, bus: &mut dyn Bus) -> Option<Snapshot> {
        let cfg = game.config?;
        let mut s = Snapshot {
            frame: bus.read_u32(cfg.g_main + 0x24).unwrap_or(0),
            ..Default::default()
        };

        let count = (bus.read_u8(cfg.player_party_count).unwrap_or(0) as usize).min(PARTY_SIZE);
        s.party = read_mons(bus, cfg.player_party, count);

        // gMain.inBattle is bit 1 of the byte at +0x439.
        s.in_battle = bus.read_u8(cfg.g_main + 0x439).unwrap_or(0) & 0x02 != 0;
        if s.in_battle {
            read_battle(&mut s, cfg, bus);
        }

        s.mapsec = bus
            .read_u8(cfg.map_header + MAP_HEADER_MAPSEC_OFF)
            .unwrap_or(0);

        // Money: SaveBlock1.money XOR SaveBlock2.encryptionKey; the bag's quantities XOR its low half.
        let sb2 = bus.read_u32(cfg.save_block2_ptr).filter(|&a| in_ram(a));
        let key = sb2.and_then(|sb2| bus.read_u32(sb2 + cfg.encryption_key_off));
        let sb1 = bus.read_u32(cfg.save_block1_ptr).filter(|&a| in_ram(a));
        if let (Some(key), Some(sb1)) = (key, sb1) {
            s.money = bus
                .read_u32(sb1 + cfg.money_off)
                .map(|m| m ^ key)
                .filter(|&m| m <= MAX_MONEY);
        }
        if let Some(key) = key {
            s.bag = read_bag(cfg, bus, key);
        }
        if let (Some(sb2), Some(_)) = (sb2, sb1) {
            s.player_gender = bus.read_u8(sb2 + SAVEBLOCK2_GENDER_OFF).filter(|&g| g <= 1);
        }
        Some(s)
    }

    pub fn place_name(&self, game: GameKind) -> Option<&'static str> {
        tables::place_name(game, self.mapsec)
    }

    /// The battler facing the player's lead (the foe's left).
    pub fn foe(&self) -> Option<&BattleMon> {
        self.battlers[POS_OPPONENT_LEFT].as_ref()
    }

    /// The player's battler out front.
    pub fn player_battler(&self) -> Option<&BattleMon> {
        self.battlers[POS_PLAYER_LEFT].as_ref()
    }
}

fn read_battle(s: &mut Snapshot, cfg: &Config, bus: &mut dyn Bus) {
    let ty = bus.read_u32(cfg.battle_type_flags).unwrap_or(0);
    s.is_double = ty & BATTLE_TYPE_DOUBLE != 0;
    s.is_trainer = ty & BATTLE_TYPE_TRAINER != 0;
    let mut n = bus.read_u8(cfg.battlers_count).unwrap_or(4) as usize;
    if !(1..=4).contains(&n) {
        n = 4;
    }
    let mut pos = [0u8, 1, 2, 3];
    bus.read(cfg.battler_positions, &mut pos);
    let mut mons = [0u8; 4 * BATTLE_MON_SIZE];
    let mut foe_battler = None;
    if bus.read(cfg.battle_mons, &mut mons) {
        for b in 0..n {
            let p = if pos[b] < 4 { pos[b] as usize } else { b };
            s.battlers[p] = decode_battle_mon(&mons[b * BATTLE_MON_SIZE..]);
            if p == POS_OPPONENT_LEFT {
                foe_battler = Some(b as u32);
            }
        }
    }
    if s.is_trainer && cfg.enemy_party != 0 {
        s.foes = read_mons(bus, cfg.enemy_party, PARTY_SIZE);
        let Some(fb) = foe_battler else { return };
        if s.foes.is_empty() || cfg.battler_party_indexes == 0 {
            return;
        }
        let mut idx = [0u8; 2];
        if bus.read(cfg.battler_party_indexes + fb * 2, &mut idx) {
            let active = u16::from_le_bytes(idx) as usize;
            if active < s.foes.len() {
                s.foe_active = Some(active);
            }
        }
        if let Some(active) = s.foe_active {
            let next = bus
                .read_u32(cfg.battle_struct_ptr)
                .filter(|&bs| in_ram(bs))
                .and_then(|bs| bus.read_u8(bs + cfg.mon_to_switch_into_off + fb))
                .map(|n| n as usize);
            if let Some(next) = next {
                if next < s.foes.len()
                    && next != active
                    && s.foes[next].hp > 0
                    && s.foes[active].hp == 0
                {
                    s.foe_next = Some(next);
                }
            }
        }
    }
}

fn read_bag(cfg: &Config, bus: &mut dyn Bus, key: u32) -> Option<[Vec<Item>; 5]> {
    let mut pockets = [0u8; 5 * 8];
    if !bus.read(cfg.bag_pockets, &mut pockets) {
        return None;
    }
    let mut bag: [Vec<Item>; 5] = Default::default();
    for p in 0..5 {
        let slots = u32_at(&pockets, p * 8);
        let capacity = (pockets[p * 8 + 4] as usize).min(POCKET_SLOTS);
        if !in_ram(slots) || capacity == 0 {
            continue;
        }
        let mut items = vec![0u8; capacity * 4];
        if !bus.read(slots, &mut items) {
            continue;
        }
        let out = &mut bag[cfg.bag_order[p] as usize];
        for i in 0..capacity {
            let id = u16_at(&items, i * 4);
            if id != 0 {
                out.push(Item {
                    id,
                    quantity: u16_at(&items, i * 4 + 2) ^ (key & 0xFFFF) as u16,
                });
            }
        }
    }
    Some(bag)
}
