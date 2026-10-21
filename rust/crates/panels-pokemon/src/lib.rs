//! The Pokémon pack: PARTY, BATTLE and BAG for retail FireRed / LeafGreen / Emerald, on the `companion`
//! crate's readers. Each panel reads the game for itself on [`Panel::refresh`].

use adventurepad_sdk::egui::{self, Color32, RichText};
use adventurepad_sdk::style::{self, ACCENT, BAD, GOOD, MUTED, WARN};
use adventurepad_sdk::{GameInfo, Memory, Panel, PanelCtx, PanelPack};
use companion::battle::{move_verdict, rank_suggestions, Verdict};
use companion::snapshot::status_label;
use companion::tables::{self, item_description, item_name, move_name, species_name, type_name};
use companion::{Bus, Game, GameKind, Mon, Pocket, Snapshot};

pub struct PokemonPack;

impl PanelPack for PokemonPack {
    fn name(&self) -> &str {
        "Pokémon (Gen 3)"
    }

    fn panels_for(&self, info: &GameInfo) -> Vec<Box<dyn Panel>> {
        let game = detect(info);
        let Some(kind) = game.kind.filter(|_| game.config.is_some()) else {
            return Vec::new();
        };
        vec![
            Box::new(PartyPanel {
                game: detect(info),
                kind,
                snap: None,
                open: None,
            }),
            Box::new(BattlePanel {
                game: detect(info),
                snap: None,
            }),
            Box::new(BagPanel {
                game,
                kind,
                snap: None,
                pocket: Pocket::Items,
            }),
        ]
    }
}

/// Which Gen 3 game this is, if any (the companion crate's detection, from the ROM's header and size).
pub fn detect(info: &GameInfo) -> Game {
    let header = &info.rom[..info.rom.len().min(0xC0)];
    Game::detect(header, info.rom.len())
}

/// The companion crate reads through its own [`Bus`]; the SDK hands panels a [`Memory`] copy.
struct MemoryBus<'a>(&'a Memory);

impl Bus for MemoryBus<'_> {
    fn read(&mut self, addr: u32, out: &mut [u8]) -> bool {
        self.0.read(addr, out)
    }
}

/// One read of the game from a memory copy.
pub fn read(game: &Game, memory: &Memory) -> Option<Snapshot> {
    Snapshot::read(game, &mut MemoryBus(memory))
}

fn types_text(types: (u8, u8)) -> String {
    match types {
        (a, _) if a == tables::TYPE_NONE => String::new(),
        (a, b) if a == b || b == tables::TYPE_NONE => type_name(a).to_owned(),
        (a, b) => format!("{} / {}", type_name(a), type_name(b)),
    }
}

fn verdict_color(v: Verdict) -> Color32 {
    if v.is_good() {
        GOOD
    } else if v.is_bad() {
        BAD
    } else {
        MUTED
    }
}

// --- PARTY -----------------------------------------------------------------------------------------------

pub struct PartyPanel {
    game: Game,
    kind: GameKind,
    snap: Option<Snapshot>,
    /// The party slot whose summary is open.
    open: Option<usize>,
}

impl Panel for PartyPanel {
    fn title(&self) -> String {
        "PARTY".into()
    }

    fn refresh(&mut self, _info: &GameInfo, memory: &Memory) {
        self.snap = read(&self.game, memory);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &mut PanelCtx) {
        let Some(s) = &self.snap else {
            style::hint(ui, "Reading the game...");
            return;
        };
        ui.horizontal(|ui| {
            if let Some(place) = s.place_name(self.kind) {
                ui.label(RichText::new(place).color(MUTED));
            }
            if let Some(m) = s.money {
                ui.label(RichText::new(format!("₽{m}")).color(ACCENT));
            }
        });
        if let Some(m) = self.open.and_then(|i| s.party.get(i)) {
            if ui.button("< PARTY").clicked() {
                self.open = None;
            }
            summary(ui, self.kind, m);
            return;
        }
        if s.party.is_empty() {
            style::hint(ui, "No one in your party yet.");
        }
        for (i, m) in s.party.iter().enumerate() {
            if party_row(ui, m)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
                && !m.is_egg()
            {
                self.open = Some(i);
            }
        }
    }
}

fn party_row(ui: &mut egui::Ui, m: &Mon) -> egui::Response {
    style::card(ui, |ui| {
        if m.is_egg() {
            ui.label(RichText::new("EGG").strong().size(16.0));
            return;
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{}{}", m.nickname, m.gender.symbol()))
                    .strong()
                    .size(16.0),
            );
            ui.label(RichText::new(format!("Lv{}", m.level)).color(MUTED));
            if m.hp == 0 {
                ui.label(RichText::new("FNT").color(BAD).strong());
            } else if let Some(st) = m.status_label() {
                ui.label(RichText::new(st).color(WARN).strong());
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(species_name(m.species)).small().color(MUTED));
            ui.label(
                RichText::new(types_text(tables::species_types(m.species)))
                    .small()
                    .color(MUTED),
            );
        });
        ui.horizontal(|ui| {
            style::meter(ui, m.hp as u32, m.max_hp as u32, 220.0);
            ui.label(format!("{}/{}", m.hp, m.max_hp));
        });
    })
}

/// A party member's summary: HP, held item, nature, moves, stats with IVs / EVs (PARTY's, and the HUD's).
pub fn summary(ui: &mut egui::Ui, kind: GameKind, m: &Mon) {
    ui.label(
        RichText::new(format!(
            "{}{}  Lv{}",
            m.nickname,
            m.gender.symbol(),
            m.level
        ))
        .strong()
        .size(20.0),
    );
    ui.label(
        RichText::new(format!(
            "{}   {}",
            species_name(m.species),
            types_text(tables::species_types(m.species))
        ))
        .color(MUTED),
    );
    ui.horizontal(|ui| {
        style::meter(ui, m.hp as u32, m.max_hp as u32, 240.0);
        ui.label(format!("HP {}/{}", m.hp, m.max_hp));
    });
    if m.held_item != 0 {
        ui.label(format!("Holding {}", item_name(kind, m.held_item)));
    }
    ui.label(format!("Nature {}", m.nature()));
    ui.add_space(8.0);
    style::heading(ui, "MOVES");
    for k in 0..4 {
        if m.moves[k] == 0 {
            continue;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(move_name(m.moves[k])).strong());
            ui.label(
                RichText::new(type_name(tables::move_type(m.moves[k])))
                    .small()
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("PP {}", m.pp[k]));
                let p = tables::move_power(m.moves[k]);
                ui.label(
                    RichText::new(if p > 0 {
                        format!("POW {p}")
                    } else {
                        "STATUS".into()
                    })
                    .color(MUTED),
                );
            });
        });
    }
    ui.add_space(8.0);
    style::heading(ui, "STATS");
    // The party struct: ATTACK, DEFENSE, SPEED, SP. ATK, SP. DEF; IVs / EVs are stored HP, ATK, DEF, SPE, SPA, SPD.
    egui::Grid::new("stats")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("STAT").color(MUTED));
            ui.label(RichText::new("IV").color(MUTED));
            ui.label(RichText::new("EV").color(MUTED));
            ui.end_row();
            let rows = [
                ("HP", m.max_hp, 0),
                ("ATTACK", m.stats[0], 1),
                ("DEFENSE", m.stats[1], 2),
                ("SP. ATK", m.stats[3], 4),
                ("SP. DEF", m.stats[4], 5),
                ("SPEED", m.stats[2], 3),
            ];
            for (name, stat, i) in rows {
                ui.label(name);
                ui.label(stat.to_string());
                let iv = RichText::new(m.ivs[i].to_string());
                ui.label(if m.ivs[i] >= 30 { iv.color(GOOD) } else { iv });
                ui.label(m.evs[i].to_string());
                ui.end_row();
            }
        });
    ui.label(
        RichText::new(format!("EXP {}", m.experience))
            .small()
            .color(MUTED),
    );
}

// --- BATTLE ----------------------------------------------------------------------------------------------

/// Shows itself, and asks to be the open tab, while a battle is on.
pub struct BattlePanel {
    game: Game,
    snap: Option<Snapshot>,
}

impl Panel for BattlePanel {
    fn title(&self) -> String {
        "BATTLE".into()
    }

    fn refresh(&mut self, _info: &GameInfo, memory: &Memory) {
        self.snap = read(&self.game, memory);
    }

    fn visible(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.in_battle)
    }

    fn wants_focus(&self) -> bool {
        self.visible()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &mut PanelCtx) {
        let Some(s) = &self.snap else { return };
        let Some(foe) = s.foe() else {
            style::hint(ui, "Waiting for the battle...");
            return;
        };
        ui.label(
            RichText::new(format!(
                "FOE  {}  Lv{}",
                species_name(foe.species),
                foe.level
            ))
            .strong()
            .size(18.0),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(types_text(foe.types)).color(MUTED));
            if let Some(st) = status_label(foe.status) {
                ui.label(RichText::new(st).color(WARN));
            }
        });
        ui.horizontal(|ui| {
            style::meter(ui, foe.hp as u32, foe.max_hp as u32, 240.0);
            let pct = if foe.max_hp > 0 {
                foe.hp as u32 * 100 / foe.max_hp as u32
            } else {
                0
            };
            ui.label(format!("{pct}%"));
        });
        ui.add_space(8.0);
        if let Some(me) = s.player_battler() {
            ui.label(
                RichText::new(format!(
                    "YOU  {}  Lv{}   {}/{}",
                    species_name(me.species),
                    me.level,
                    me.hp,
                    me.max_hp
                ))
                .strong(),
            );
            for k in 0..4 {
                if me.moves[k] == 0 {
                    continue;
                }
                let v = move_verdict(me.moves[k], foe.types);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(move_name(me.moves[k])).strong());
                    ui.label(
                        RichText::new(format!("PP {}", me.pp[k]))
                            .small()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(v.label()).strong().color(verdict_color(v)));
                    });
                });
            }
        }
        ui.add_space(10.0);
        style::heading(ui, "SUGGESTIONS");
        let picks = rank_suggestions(s, foe.types);
        if picks.is_empty() {
            style::hint(ui, "No one in your party has a damaging move left.");
        }
        for (n, p) in picks.iter().take(3).enumerate() {
            let v = Verdict::from_pct(p.pct);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}. {}", n + 1, p.mon.nickname)).strong());
                if p.out {
                    ui.label(RichText::new("OUT").color(ACCENT).small());
                }
                ui.label(RichText::new(move_name(p.mv)).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(v.label()).color(verdict_color(v)));
                });
            });
        }
        if s.is_trainer && !s.foes.is_empty() {
            ui.add_space(10.0);
            style::heading(ui, "FOE TEAM");
            for (i, m) in s.foes.iter().enumerate() {
                let tag = if Some(i) == s.foe_active {
                    " (out)"
                } else if Some(i) == s.foe_next {
                    " (next)"
                } else {
                    ""
                };
                let fainted = m.hp == 0;
                let text =
                    RichText::new(format!("{}  Lv{}{}", species_name(m.species), m.level, tag))
                        .color(if fainted { MUTED } else { Color32::WHITE });
                ui.label(if fainted { text.strikethrough() } else { text });
            }
        }
    }
}

// --- BAG -------------------------------------------------------------------------------------------------

pub struct BagPanel {
    game: Game,
    kind: GameKind,
    snap: Option<Snapshot>,
    pocket: Pocket,
}

impl Panel for BagPanel {
    fn title(&self) -> String {
        "BAG".into()
    }

    fn refresh(&mut self, _info: &GameInfo, memory: &Memory) {
        self.snap = read(&self.game, memory);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &mut PanelCtx) {
        let Some(bag) = self.snap.as_ref().and_then(|s| s.bag.as_ref()) else {
            style::hint(ui, "The bag can't be read yet.");
            return;
        };
        ui.horizontal_wrapped(|ui| {
            for p in Pocket::ALL {
                ui.selectable_value(&mut self.pocket, p, p.label());
            }
        });
        ui.separator();
        let items = &bag[self.pocket as usize];
        if items.is_empty() {
            style::hint(ui, "Nothing in this pocket.");
        }
        for item in items {
            let r = ui.horizontal(|ui| {
                ui.label(RichText::new(item_name(self.kind, item.id)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.pocket != Pocket::Key {
                        ui.label(format!("x{}", item.quantity));
                    }
                });
            });
            if let Some(d) = item_description(self.kind, item.id) {
                r.response.on_hover_text(d);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// A FireRed rev 1 header on otherwise empty ROM bytes: enough for detection.
    fn firered() -> GameInfo {
        let mut rom = vec![0u8; 0x100_0000];
        rom[0xAC..0xB0].copy_from_slice(b"BPRE");
        rom[0xB2] = 0x96;
        rom[0xBC] = 1;
        GameInfo::from_rom("FireRed", Arc::from(rom))
    }

    fn fixture(name: &str) -> Memory {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../app/src/test/resources/fixtures")
            .join(name);
        Memory::from_dumps(&dir, None).unwrap()
    }

    #[test]
    fn firered_gets_party_battle_and_bag() {
        let panels = PokemonPack.panels_for(&firered());
        let titles: Vec<String> = panels.iter().map(|p| p.title()).collect();
        assert_eq!(titles, ["PARTY", "BATTLE", "BAG"]);
    }

    #[test]
    fn battle_shows_only_in_a_battle() {
        let info = firered();
        let mut panels = PokemonPack.panels_for(&info);
        panels[1].refresh(&info, &fixture("firered_vanilla"));
        assert!(!panels[1].visible());
        assert!(!panels[1].wants_focus());
    }

    #[test]
    fn other_games_get_nothing() {
        let mut rom = vec![0u8; 0x40_0000];
        rom[0xAC..0xB0].copy_from_slice(b"AGBJ");
        rom[0xB2] = 0x96;
        assert!(PokemonPack
            .panels_for(&GameInfo::from_rom("x", Arc::from(rom)))
            .is_empty());
    }
}
