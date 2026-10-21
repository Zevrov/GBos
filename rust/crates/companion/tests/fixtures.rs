//! The repo's RAM fixtures (app/src/test/resources/fixtures), read the way the app's tests check them
//! (FireRedVanillaDecodeTest, MoneyTest, ...): the same answers from the Rust readers.

use companion::battle::{rank_suggestions, Verdict};
use companion::game::Game;
use companion::tables::{species_name, SPECIES_EGG};
use companion::{Gender, Pocket, RamImage, Snapshot};
use std::path::PathBuf;

fn fixture(name: &str) -> RamImage {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../app/src/test/resources/fixtures")
        .join(name);
    RamImage::load(&dir).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// A header for `code` / `revision`, as a retail ROM of 16 MB has it.
fn game(code: &str, revision: u8) -> Game {
    let mut header = [0u8; 0xC0];
    header[0xAC..0xB0].copy_from_slice(code.as_bytes());
    header[0xBC] = revision;
    Game::detect(&header, 0x100_0000)
}

fn read(name: &str, code: &str, revision: u8) -> Snapshot {
    Snapshot::read(&game(code, revision), &mut fixture(name)).expect("a supported game")
}

#[test]
fn firered_party_matches_the_games_party_screen() {
    // Read off the real game's party menu for this save (FireRedVanillaDecodeTest).
    let s = read("firered_vanilla", "BPRE", 1);
    let names: Vec<String> = s.party.iter().map(|m| species_name(m.species)).collect();
    assert_eq!(
        names,
        ["LAPRAS", "KADABRA", "JOLTEON", "CHARIZARD", "EGG", "MEOWTH"]
    );
    let genders: Vec<Gender> = s.party.iter().map(|m| m.gender).collect();
    assert_eq!(
        genders,
        [
            Gender::Female,
            Gender::Male,
            Gender::Female,
            Gender::Male,
            Gender::None,
            Gender::Male
        ]
    );
    assert_eq!(s.party[4].species, SPECIES_EGG);
    for m in s.party.iter().filter(|m| !m.is_egg()) {
        assert!(m.level >= 1 && m.level <= 100, "{m:?}");
        assert!(m.hp <= m.max_hp && m.max_hp > 0, "{m:?}");
        assert!(m.moves[0] != 0, "{m:?}");
    }
}

#[test]
fn money_matches_the_apps_reads() {
    assert_eq!(read("firered_vanilla", "BPRE", 1).money, Some(224_300));
    assert_eq!(read("leafgreen_rev1", "BPGE", 1).money, Some(6_098));
    assert_eq!(read("emerald_vanilla", "BPEE", 0).money, Some(185_092));
}

#[test]
fn bags_have_items_in_more_than_one_pocket() {
    for (name, code, rev) in [
        ("firered_vanilla", "BPRE", 1),
        ("emerald_vanilla", "BPEE", 0),
    ] {
        let s = read(name, code, rev);
        let bag = s.bag.expect("bag read");
        let used = Pocket::ALL
            .iter()
            .filter(|p| !bag[**p as usize].is_empty())
            .count();
        assert!(used > 1, "{name}: {used} pockets");
        for item in bag.iter().flatten() {
            assert!(
                item.quantity >= 1 && item.quantity <= 999,
                "{name}: {item:?}"
            );
        }
    }
}

#[test]
fn emerald_party_decodes() {
    let s = read("emerald_vanilla", "BPEE", 0);
    assert!(!s.party.is_empty());
    for m in &s.party {
        assert!(m.species > 0 && (m.species as usize) <= 412, "{m:?}");
    }
}

#[test]
fn a_battle_has_a_foe_and_suggestions() {
    let s = read("emerald_de_battle", "BPEE", 0);
    assert!(s.in_battle);
    let foe = s.foe().expect("a foe");
    let picks = rank_suggestions(&s, foe.types);
    assert!(!picks.is_empty());
    assert!(picks.windows(2).all(|w| w[0].score >= w[1].score));
    let _ = Verdict::from_pct(picks[0].pct);
}

#[test]
fn hacks_and_unknown_games_have_no_companion_yet() {
    let mut header = [0u8; 0xC0];
    header[0xAC..0xB0].copy_from_slice(b"BPRE");
    assert!(Game::detect(&header, 0x200_0000).config.is_none());
    header[0xAC..0xB0].copy_from_slice(b"AGBJ");
    assert!(Game::detect(&header, 0x40_0000).config.is_none());
}
