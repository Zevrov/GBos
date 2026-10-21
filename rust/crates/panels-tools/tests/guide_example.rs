//! PANELS.md's "five minutes to a panel" example, kept compiling and checked against the FireRed fixture.

use adventurepad_sdk::{egui, style, Action, GameInfo, Memory, Panel, PanelCtx, PanelPack};
use std::path::PathBuf;
use std::sync::Arc;

/// FireRed's money, live.
pub struct Money {
    value: Option<u32>,
}

impl Panel for Money {
    fn title(&self) -> String {
        "MONEY".into()
    }

    fn refresh(&mut self, _game: &GameInfo, m: &Memory) {
        let sb1 = m.u32(0x0300_5008); // gSaveBlock1Ptr
        let sb2 = m.u32(0x0300_500C); // gSaveBlock2Ptr
        let key = sb2.and_then(|p| m.u32(p + 0xF20)); // its encryption key
        let money = sb1.and_then(|p| m.u32(p + 0x290));
        self.value = money.zip(key).map(|(v, k)| v ^ k);
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut PanelCtx) {
        match self.value {
            Some(v) => {
                ui.heading(format!("₽{v}"));
            }
            None => style::hint(ui, "Not in a game yet."),
        }
        if ui.button("Save a state").clicked() {
            ctx.act(Action::SaveState(9));
        }
    }
}

pub struct MyPack;

impl PanelPack for MyPack {
    fn name(&self) -> &str {
        "My pack"
    }
    fn panels_for(&self, game: &GameInfo) -> Vec<Box<dyn Panel>> {
        if game.code == "BPRE" {
            vec![Box::new(Money { value: None })]
        } else {
            vec![]
        }
    }
}

#[test]
fn the_guides_panel_reads_firereds_money() {
    let mut rom = vec![0u8; 0x200];
    rom[0xAC..0xB0].copy_from_slice(b"BPRE");
    rom[0xB2] = 0x96;
    let game = GameInfo::from_rom("FireRed", Arc::from(rom));
    assert_eq!(MyPack.panels_for(&game).len(), 1);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../app/src/test/resources/fixtures/firered_vanilla");
    let m = Memory::from_dumps(&dir, None).unwrap();
    let mut panel = Money { value: None };
    panel.refresh(&game, &m);
    assert_eq!(panel.value, Some(224_300));
}
