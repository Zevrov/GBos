//! Lookups over the generated tables (3ds/core/pd_tables.c's, in Rust).

use crate::tables_gen as t;
use crate::GameKind;

/// The egg's species id in FireRed / Emerald's numbering.
pub const SPECIES_EGG: u16 = 412;
/// No type ([`species_types`] of an unknown species).
pub const TYPE_NONE: u8 = 255;

/// Names in the game's own casing: capitals, with a small é (the app's gameCase()).
fn game_case(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect()
}

pub fn species_name(species: u16) -> String {
    match species {
        0 => "-".into(),
        SPECIES_EGG => "EGG".into(),
        s => t::SPECIES_NAMES
            .get(s as usize)
            .copied()
            .flatten()
            .map(game_case)
            .unwrap_or_else(|| format!("#{s}")),
    }
}

pub fn move_name(id: u16) -> String {
    if id == 0 {
        return "-".into();
    }
    t::MOVES
        .get(id as usize)
        .and_then(|m| m.0)
        .map(game_case)
        .unwrap_or_else(|| format!("MOVE#{id}"))
}

/// A move's type, [`TYPE_NONE`] if unknown.
pub fn move_type(id: u16) -> u8 {
    t::MOVES
        .get(id as usize)
        .filter(|m| m.0.is_some())
        .map_or(TYPE_NONE, |m| m.1)
}

/// A move's power, 0 for a status move (or unknown).
pub fn move_power(id: u16) -> u16 {
    t::MOVES
        .get(id as usize)
        .filter(|m| m.0.is_some())
        .map_or(0, |m| m.2)
}

pub fn type_name(ty: u8) -> &'static str {
    t::TYPE_NAMES
        .get(ty as usize)
        .copied()
        .flatten()
        .unwrap_or("")
}

/// A species' two types (the same type twice for a single-typed one); [`TYPE_NONE`] both if unknown.
pub fn species_types(species: u16) -> (u8, u8) {
    if species == 0 {
        return (TYPE_NONE, TYPE_NONE);
    }
    t::SPECIES_TYPES
        .get(species as usize)
        .copied()
        .unwrap_or((TYPE_NONE, TYPE_NONE))
}

pub fn gender_ratio(species: u16) -> Option<u8> {
    t::GENDER_RATIOS.get(species as usize).copied()
}

fn single_pct(atk: u8, def: u8) -> u32 {
    t::TYPE_CHART
        .iter()
        .find(|p| p.0 == atk && p.1 == def)
        .map_or(100, |p| p.2 as u32)
}

/// One attacking type against a (possibly dual-typed) defender, in percent (100 = 1x): the app's
/// typeMultiplierPct().
pub fn type_multiplier_pct(atk: u8, def1: u8, def2: u8) -> u32 {
    let mut pct = single_pct(atk, def1);
    if def2 != TYPE_NONE && def2 != def1 {
        pct = pct * single_pct(atk, def2) / 100;
    }
    pct
}

pub fn item_name(game: GameKind, id: u16) -> String {
    let names: &[Option<&str>] = match game {
        GameKind::Emerald => &t::ITEM_NAMES_EMERALD,
        _ => &t::ITEM_NAMES_FIRERED,
    };
    names
        .get(id as usize)
        .copied()
        .flatten()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("ITEM#{id}"))
}

pub fn item_description(game: GameKind, id: u16) -> Option<&'static str> {
    let descs: &[Option<&'static str>] = match game {
        GameKind::Emerald => &t::ITEM_DESC_EMERALD,
        _ => &t::ITEM_DESC_FIRERED,
    };
    descs.get(id as usize).copied().flatten()
}

/// The map section's name (gMapHeader.regionMapSectionId), as the game's region map names it.
pub fn place_name(game: GameKind, mapsec: u8) -> Option<&'static str> {
    match game {
        // FireRed's sections start at 0x58 (the Hoenn ones before them are Ruby / Sapphire's).
        GameKind::FireRed => t::MAPSEC_FIRERED.get(mapsec as usize).copied().flatten(),
        GameKind::Emerald => t::MAPSEC_EMERALD.get(mapsec as usize).copied().flatten(),
    }
}
