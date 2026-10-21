//! Developer panels, for every game: MEMORY, a hex view and a watch list of the game's RAM. What you use to
//! find the addresses a new panel reads (watch a value, play, see what moves).

use adventurepad_sdk::egui::{self, RichText, TextEdit};
use adventurepad_sdk::style::{self, ACCENT, MUTED};
use adventurepad_sdk::{Action, GameInfo, Memory, Panel, PanelCtx, PanelPack, Platform};

pub struct ToolsPack;

impl PanelPack for ToolsPack {
    fn name(&self) -> &str {
        "Developer tools"
    }

    fn panels_for(&self, info: &GameInfo) -> Vec<Box<dyn Panel>> {
        let start = match info.platform {
            Platform::Gba => 0x0200_0000,
            Platform::Gb => 0xC000,
        };
        vec![Box::new(MemoryPanel::new(start))]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Width {
    U8,
    U16,
    U32,
}

impl Width {
    fn bytes(self) -> usize {
        match self {
            Width::U8 => 1,
            Width::U16 => 2,
            Width::U32 => 4,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Width::U8 => "u8",
            Width::U16 => "u16",
            Width::U32 => "u32",
        }
    }
}

struct Watch {
    addr: u32,
    width: Width,
    value: Option<u32>,
    /// The value before it last changed, and the memory frame it changed on.
    changed: Option<(Option<u32>, u64)>,
}

pub struct MemoryPanel {
    view_addr: u32,
    addr_text: String,
    view: Vec<Option<u8>>,
    watches: Vec<Watch>,
    new_watch: String,
    new_width: Width,
    frame: u64,
}

const ROWS: u32 = 16;

fn parse_addr(s: &str) -> Option<u32> {
    let s = s
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .replace('_', "");
    u32::from_str_radix(&s, 16).ok()
}

impl MemoryPanel {
    pub fn new(start: u32) -> MemoryPanel {
        MemoryPanel {
            view_addr: start,
            addr_text: format!("{start:08X}"),
            view: Vec::new(),
            watches: Vec::new(),
            new_watch: String::new(),
            new_width: Width::U16,
            frame: 0,
        }
    }

    fn read_watch(memory: &Memory, w: &Watch) -> Option<u32> {
        match w.width {
            Width::U8 => memory.u8(w.addr).map(u32::from),
            Width::U16 => memory.u16(w.addr).map(u32::from),
            Width::U32 => memory.u32(w.addr),
        }
    }
}

impl Panel for MemoryPanel {
    fn title(&self) -> String {
        "MEMORY".into()
    }

    fn refresh(&mut self, _game: &GameInfo, memory: &Memory) {
        self.frame = memory.frame;
        self.view = (0..ROWS * 16)
            .map(|i| memory.u8(self.view_addr.wrapping_add(i)))
            .collect();
        for w in &mut self.watches {
            let v = Self::read_watch(memory, w);
            if v != w.value {
                if w.value.is_some() {
                    w.changed = Some((w.value, memory.frame));
                }
                w.value = v;
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut PanelCtx) {
        style::heading(ui, "WATCH");
        let mut remove = None;
        egui::Grid::new("watches")
            .num_columns(5)
            .striped(true)
            .show(ui, |ui| {
                for (i, w) in self.watches.iter().enumerate() {
                    ui.monospace(format!("{:08X}", w.addr));
                    ui.label(RichText::new(w.width.label()).color(MUTED));
                    match w.value {
                        Some(v) => ui.monospace(format!(
                            "{v} (0x{v:0width$X})",
                            width = w.width.bytes() * 2
                        )),
                        None => ui.label(RichText::new("not in memory").color(MUTED)),
                    };
                    // Changed in the last 120 frames (2 seconds): highlighted.
                    let fresh = w
                        .changed
                        .is_some_and(|(_, at)| self.frame.saturating_sub(at) < 120);
                    match w.changed {
                        Some((old, _)) => ui.label(
                            RichText::new(format!(
                                "was {}",
                                old.map_or("-".into(), |o| o.to_string())
                            ))
                            .color(if fresh { ACCENT } else { MUTED }),
                        ),
                        None => ui.label(""),
                    };
                    if ui.small_button("x").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.watches.remove(i);
        }
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.new_watch)
                    .hint_text("address (hex)")
                    .desired_width(110.0),
            );
            for w in [Width::U8, Width::U16, Width::U32] {
                ui.selectable_value(&mut self.new_width, w, w.label());
            }
            if ui.button("Watch").clicked() {
                match parse_addr(&self.new_watch) {
                    Some(addr) => {
                        self.watches.push(Watch {
                            addr,
                            width: self.new_width,
                            value: None,
                            changed: None,
                        });
                        self.new_watch.clear();
                    }
                    None => ctx.act(Action::Notice("Not a hex address".into())),
                }
            }
        });

        ui.add_space(10.0);
        style::heading(ui, "VIEW");
        ui.horizontal(|ui| {
            let r = ui.add(
                TextEdit::singleline(&mut self.addr_text)
                    .desired_width(110.0)
                    .font(egui::TextStyle::Monospace),
            );
            if r.lost_focus() || ui.button("Go").clicked() {
                if let Some(a) = parse_addr(&self.addr_text) {
                    self.view_addr = a & !0xF;
                }
                self.addr_text = format!("{:08X}", self.view_addr);
            }
            if ui.button("-").clicked() {
                self.view_addr = self.view_addr.wrapping_sub(ROWS * 16);
                self.addr_text = format!("{:08X}", self.view_addr);
            }
            if ui.button("+").clicked() {
                self.view_addr = self.view_addr.wrapping_add(ROWS * 16);
                self.addr_text = format!("{:08X}", self.view_addr);
            }
        });
        if let Some(regions) = ctx.memory.map(|m| m.regions()) {
            let names: Vec<String> = regions
                .iter()
                .map(|r| format!("{} {:08X}+{:X}", r.name, r.base, r.bytes.len()))
                .collect();
            ui.label(RichText::new(names.join("   ")).small().color(MUTED));
        }
        for row in 0..ROWS as usize {
            let mut line = format!("{:08X} ", self.view_addr.wrapping_add(row as u32 * 16));
            for col in 0..16 {
                match self.view.get(row * 16 + col).copied().flatten() {
                    Some(b) => line.push_str(&format!(" {b:02X}")),
                    None => line.push_str(" --"),
                }
            }
            ui.monospace(line);
        }
        ui.label(
            RichText::new("The latest copy of the game's memory, 4 a second.")
                .small()
                .color(MUTED),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adventurepad_sdk::Region;
    use std::sync::Arc;

    #[test]
    fn parses_addresses() {
        assert_eq!(parse_addr("0x02024284"), Some(0x0202_4284));
        assert_eq!(parse_addr(" 0202_4284 "), Some(0x0202_4284));
        assert_eq!(parse_addr("nope"), None);
    }

    #[test]
    fn a_watch_notes_its_old_value() {
        let info = GameInfo::from_rom("t", Arc::from(vec![0u8; 0x200]));
        let mut p = MemoryPanel::new(0x0200_0000);
        p.watches.push(Watch {
            addr: 0x0200_0000,
            width: Width::U16,
            value: None,
            changed: None,
        });
        let mem = |v: u8, frame| {
            Memory::new(
                vec![Region {
                    name: "EWRAM",
                    base: 0x0200_0000,
                    bytes: vec![v, 0, 0, 0].into(),
                }],
                frame,
            )
        };
        p.refresh(&info, &mem(5, 10));
        assert_eq!(p.watches[0].value, Some(5));
        assert!(
            p.watches[0].changed.is_none(),
            "the first read isn't a change"
        );
        p.refresh(&info, &mem(7, 20));
        assert_eq!(p.watches[0].value, Some(7));
        assert_eq!(p.watches[0].changed, Some((Some(5), 20)));
    }
}
