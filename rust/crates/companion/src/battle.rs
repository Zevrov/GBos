//! The BATTLE tab's reasoning: each move's verdict on the foe (BattleInfoScreen) and the party's best picks
//! against it (the app's rankSuggestions) - 3ds/core/pd_ui_battle.c's, in Rust.

use crate::snapshot::{Mon, Snapshot};
use crate::tables::{self, SPECIES_EGG, TYPE_NONE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Immune,
    Super4,
    Super2,
    Resisted4,
    Resisted2,
    Neutral,
    /// A move with no power (it doesn't deal damage by type).
    Status,
}

impl Verdict {
    pub fn from_pct(pct: u32) -> Verdict {
        match pct {
            0 => Verdict::Immune,
            p if p >= 400 => Verdict::Super4,
            p if p >= 200 => Verdict::Super2,
            p if p <= 25 => Verdict::Resisted4,
            p if p < 100 => Verdict::Resisted2,
            _ => Verdict::Neutral,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Verdict::Immune => "IMMUNE 0x",
            Verdict::Super4 => "SUPER 4x",
            Verdict::Super2 => "SUPER 2x",
            Verdict::Resisted4 => "RESISTED 1/4x",
            Verdict::Resisted2 => "RESISTED 1/2x",
            Verdict::Neutral => "NEUTRAL 1x",
            Verdict::Status => "STATUS",
        }
    }

    pub fn is_good(self) -> bool {
        matches!(self, Verdict::Super2 | Verdict::Super4)
    }

    pub fn is_bad(self) -> bool {
        matches!(
            self,
            Verdict::Immune | Verdict::Resisted2 | Verdict::Resisted4
        )
    }
}

/// `mv` used on a defender of `types`.
pub fn move_verdict(mv: u16, types: (u8, u8)) -> Verdict {
    if tables::move_power(mv) == 0 {
        return Verdict::Status;
    }
    Verdict::from_pct(tables::type_multiplier_pct(
        tables::move_type(mv),
        types.0,
        types.1,
    ))
}

/// One SUGGESTIONS row: a party Pokémon, its best move on the foe and that move's multiplier.
#[derive(Debug, Clone)]
pub struct Pick<'a> {
    pub mon: &'a Mon,
    pub party_index: usize,
    pub mv: u16,
    pub pp: u8,
    pub pct: u32,
    pub score: i64,
    /// It's one of the player's battlers right now.
    pub out: bool,
}

/// Whether party Pokémon `m` is out on the field (battlers carry no personality to match on).
fn is_out(m: &Mon, s: &Snapshot) -> bool {
    [0usize, 2]
        .iter()
        .filter_map(|&p| s.battlers[p].as_ref())
        .any(|b| b.species == m.species && b.level == m.level && b.max_hp == m.max_hp)
}

/// Every living party Pokémon with a usable damaging move, best first: its best move's power times the
/// multiplier on the foe, minus the worst the foe's own types do to it. Party order on ties.
pub fn rank_suggestions(s: &Snapshot, foe_types: (u8, u8)) -> Vec<Pick<'_>> {
    let (f1, f2) = foe_types;
    let mut picks: Vec<Pick<'_>> = s
        .party
        .iter()
        .enumerate()
        .filter(|(_, m)| m.hp > 0 && m.species != SPECIES_EGG)
        .filter_map(|(i, m)| {
            let best = (0..4)
                .filter(|&k| m.moves[k] != 0 && m.pp[k] > 0 && tables::move_power(m.moves[k]) > 0)
                .max_by_key(|&k| {
                    // Ties go to the earlier move (max_by_key keeps the last, so reverse the index).
                    let v = tables::move_power(m.moves[k]) as u32
                        * tables::type_multiplier_pct(tables::move_type(m.moves[k]), f1, f2);
                    (v, std::cmp::Reverse(k))
                })?;
            let (t1, t2) = tables::species_types(m.species);
            let mut risk = 100u32;
            if f1 != TYPE_NONE {
                risk = tables::type_multiplier_pct(f1, t1, t2);
                if f2 != TYPE_NONE && f2 != f1 {
                    risk = risk.max(tables::type_multiplier_pct(f2, t1, t2));
                }
            }
            let mv = m.moves[best];
            let pct = tables::type_multiplier_pct(tables::move_type(mv), f1, f2);
            Some(Pick {
                mon: m,
                party_index: i,
                mv,
                pp: m.pp[best],
                pct,
                score: (tables::move_power(mv) as i64 * pct as i64 / 100) - risk as i64,
                out: is_out(m, s),
            })
        })
        .collect();
    picks.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.party_index.cmp(&b.party_index))
    });
    picks
}
