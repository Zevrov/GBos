//! Which game is running, from its ROM header, and where its RAM globals live: 3ds/core/pd_game.c's configs
//! (the app's NativeConfig, NativeReader.kt) - each address comes from a pret decomp build byte-identical to
//! retail. Keep in sync.

use crate::snapshot::Pocket;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameKind {
    /// FireRed and LeafGreen (LeafGreen runs as FireRed, like the app).
    FireRed,
    Emerald,
}

/// RAM addresses (and in-save offsets) the readers use.
#[derive(Debug)]
pub struct Config {
    pub player_party: u32,
    pub player_party_count: u32,
    pub battle_mons: u32,
    pub battler_positions: u32,
    pub battlers_count: u32,
    pub battle_type_flags: u32,
    pub g_main: u32,
    pub save_block1_ptr: u32,
    pub save_block2_ptr: u32,
    pub map_header: u32,
    /// In SaveBlock2.
    pub encryption_key_off: u32,
    /// In SaveBlock1.
    pub money_off: u32,
    /// gEnemyParty, gBattlerPartyIndexes, gBattleStruct (a pointer) and its monToSwitchIntoId; 0 = unknown.
    pub enemy_party: u32,
    pub battler_party_indexes: u32,
    pub battle_struct_ptr: u32,
    pub mon_to_switch_into_off: u32,
    /// gBagPockets: five {slots pointer, capacity} pairs; `bag_order[i]` is the pocket the game keeps at i.
    pub bag_pockets: u32,
    pub bag_order: [Pocket; 5],
}

const fn firered(foes: bool) -> Config {
    Config {
        player_party: 0x0202_4284,
        player_party_count: 0x0202_4029,
        battle_mons: 0x0202_3BE4,
        battler_positions: 0x0202_3BD6,
        battlers_count: 0x0202_3BCC,
        battle_type_flags: 0x0202_2B4C,
        g_main: 0x0300_30F0,
        save_block1_ptr: 0x0300_5008,
        save_block2_ptr: 0x0300_500C,
        map_header: 0x0203_6DFC,
        encryption_key_off: 0xF20,
        money_off: 0x290,
        enemy_party: if foes { 0x0202_402C } else { 0 },
        battler_party_indexes: if foes { 0x0202_3BCE } else { 0 },
        battle_struct_ptr: if foes { 0x0202_3FE8 } else { 0 },
        mon_to_switch_into_off: if foes { 0x5C } else { 0 },
        bag_pockets: 0x0203_988C,
        // FireRed's item.c: Items, Key Items, Poke Balls, TM Case, Berry Pouch.
        bag_order: [
            Pocket::Items,
            Pocket::Key,
            Pocket::Balls,
            Pocket::TmHm,
            Pocket::Berries,
        ],
    }
}

static FIRERED_REV1: Config = firered(true);
static FIRERED_REV0: Config = firered(false);

static EMERALD: Config = Config {
    player_party: 0x0202_44EC,
    player_party_count: 0x0202_44E9,
    battle_mons: 0x0202_4084,
    battler_positions: 0x0202_4076,
    battlers_count: 0x0202_406C,
    battle_type_flags: 0x0202_2FEC,
    g_main: 0x0300_22C0,
    save_block1_ptr: 0x0300_5D8C,
    save_block2_ptr: 0x0300_5D90,
    map_header: 0x0203_7318,
    encryption_key_off: 0xAC,
    money_off: 0x490,
    enemy_party: 0x0202_4744,
    battler_party_indexes: 0x0202_406E,
    battle_struct_ptr: 0x0202_449C,
    mon_to_switch_into_off: 0x5C,
    bag_pockets: 0x0203_9DD8,
    bag_order: [
        Pocket::Items,
        Pocket::Balls,
        Pocket::TmHm,
        Pocket::Berries,
        Pocket::Key,
    ],
};

/// The running game, as its header says.
#[derive(Debug)]
pub struct Game {
    /// The header's game code, e.g. "BPRE".
    pub code: String,
    pub revision: u8,
    pub title: &'static str,
    pub kind: Option<GameKind>,
    pub config: Option<&'static Config>,
    /// Why there's no companion for it, when [`Game::config`] is None.
    pub unsupported: Option<&'static str>,
}

impl Game {
    /// From the ROM's first 0xC0 bytes and its size.
    pub fn detect(header: &[u8], rom_size: usize) -> Game {
        let code: String = header
            .get(0xAC..0xB0)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let revision = header.get(0xBC).copied().unwrap_or(0);
        let mut g = Game {
            code,
            revision,
            title: "Unknown game",
            kind: None,
            config: None,
            unsupported: None,
        };
        match g.code.as_str() {
            "BPRE" | "BPGE" => {
                g.kind = Some(GameKind::FireRed);
                g.title = if g.code == "BPRE" {
                    "Pokémon FireRed"
                } else {
                    "Pokémon LeafGreen"
                };
                // LeafGreen's RAM is FireRed's (the icons in its ROM differ, which the companion doesn't read yet).
                g.config = Some(if revision == 1 {
                    &FIRERED_REV1
                } else {
                    &FIRERED_REV0
                });
            }
            "BPEE" => {
                g.kind = Some(GameKind::Emerald);
                g.title = "Pokémon Emerald";
                g.config = Some(&EMERALD);
            }
            _ => {
                g.unsupported = Some("Not a game the companion knows yet");
                return g;
            }
        }
        if revision > 1 {
            g.config = None;
            g.unsupported = Some("This revision isn't supported yet");
        } else if rom_size > 0x100_0000 {
            // A >16 MB FireRed / Emerald is a ROM hack: the app tells them apart by SHA-1 (not ported yet).
            g.config = None;
            g.title = if g.kind == Some(GameKind::FireRed) {
                "FireRed ROM hack"
            } else {
                "Emerald ROM hack"
            };
            g.unsupported = Some("ROM hacks aren't supported in this version yet");
        }
        g
    }
}
