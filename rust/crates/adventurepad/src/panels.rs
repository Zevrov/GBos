//! The extra screen: the panel packs the app ships with, and the host that runs a game's panels as tabs
//! (refreshing them as memory copies arrive, opening the one that asks for focus, and switching off one
//! that panics). See `adventurepad_sdk` for writing a panel.

use adventurepad_sdk::style::{self, ACCENT, BAD};
use adventurepad_sdk::{Action, GameInfo, Memory, Panel, PanelCtx, PanelPack};
use eframe::egui::{self, RichText};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// Every pack, in tab order. A new pack goes here.
pub fn packs() -> Vec<Box<dyn PanelPack>> {
    vec![Box::new(panels_pokemon::PokemonPack)]
}

/// Packs for people writing panels (SETTINGS: developer panels).
pub fn dev_packs() -> Vec<Box<dyn PanelPack>> {
    vec![Box::new(panels_tools::ToolsPack)]
}

/// The names of the packs with panels for `info` (the library's "companion ready" line).
pub fn packs_for(info: &GameInfo) -> Vec<String> {
    packs()
        .iter()
        .filter(|p| !p.panels_for(info).is_empty())
        .map(|p| p.name().to_owned())
        .collect()
}

/// A panel, and why it was switched off if it panicked.
struct Slot {
    panel: Box<dyn Panel>,
    failed: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Panel(usize),
    /// The app's own STATES tab.
    States,
}

pub struct PanelHost {
    slots: Vec<Slot>,
    pub tab: Tab,
    /// The panel that asked for focus and got it, and the tab to go back to after (None: the player picked
    /// another tab meanwhile, so stay where they are).
    focus: Option<(usize, Option<Tab>)>,
    memory: Option<Arc<Memory>>,
    actions: Vec<Action>,
}

fn panic_text(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| e.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "it crashed".into())
}

impl PanelHost {
    pub fn new(info: &GameInfo, dev: bool) -> PanelHost {
        let mut all = packs();
        if dev {
            all.extend(dev_packs());
        }
        let slots: Vec<Slot> = all
            .iter()
            .flat_map(|p| p.panels_for(info))
            .map(|panel| Slot {
                panel,
                failed: None,
            })
            .collect();
        let tab = if slots.is_empty() {
            Tab::States
        } else {
            Tab::Panel(0)
        };
        PanelHost {
            slots,
            tab,
            focus: None,
            memory: None,
            actions: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// What the panels asked the app to do since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    fn visible(&self, i: usize) -> bool {
        let s = &self.slots[i];
        s.failed.is_some() || catch_unwind(AssertUnwindSafe(|| s.panel.visible())).unwrap_or(true)
    }

    /// The newest memory copy: refreshes the panels when it's a new one, then opens the panel that wants focus.
    pub fn feed(&mut self, info: &GameInfo, memory: Option<Arc<Memory>>) {
        let new = match (&memory, &self.memory) {
            (Some(m), Some(old)) => !Arc::ptr_eq(m, old),
            (Some(_), None) => true,
            _ => false,
        };
        if new {
            self.memory = memory;
            let m = self.memory.clone().unwrap();
            for s in self.slots.iter_mut().filter(|s| s.failed.is_none()) {
                if let Err(e) = catch_unwind(AssertUnwindSafe(|| s.panel.refresh(info, &m))) {
                    s.failed = Some(panic_text(e));
                }
            }
        }
        self.update_focus();
    }

    fn update_focus(&mut self) {
        let wants: Option<usize> = (0..self.slots.len()).find(|&i| {
            let s = &self.slots[i];
            s.failed.is_none()
                && catch_unwind(AssertUnwindSafe(|| s.panel.wants_focus())).unwrap_or(false)
        });
        match (wants, self.focus) {
            (Some(i), None) => {
                if self.tab != Tab::Panel(i) {
                    self.focus = Some((i, Some(self.tab)));
                    self.tab = Tab::Panel(i);
                } else {
                    self.focus = Some((i, None));
                }
            }
            (None, Some((_, back))) => {
                if let Some(t) = back {
                    self.tab = t;
                }
                self.focus = None;
            }
            _ => {}
        }
        // A tab that went away (BATTLE after a battle) falls back to the first one showing.
        if let Tab::Panel(i) = self.tab {
            if i >= self.slots.len() || !self.visible(i) {
                self.tab = (0..self.slots.len())
                    .find(|&j| self.visible(j))
                    .map_or(Tab::States, Tab::Panel);
            }
        }
    }

    /// The tab bar.
    pub fn tabs_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for i in 0..self.slots.len() {
                if !self.visible(i) {
                    continue;
                }
                let title = catch_unwind(AssertUnwindSafe(|| self.slots[i].panel.title()))
                    .unwrap_or_else(|_| "?".into());
                if ui
                    .selectable_label(self.tab == Tab::Panel(i), RichText::new(title).strong())
                    .clicked()
                {
                    self.pick(Tab::Panel(i));
                }
            }
            if ui
                .selectable_label(self.tab == Tab::States, RichText::new("STATES").strong())
                .clicked()
            {
                self.pick(Tab::States);
            }
        });
    }

    /// The player's own pick: it wins over a panel's focus (no going back to the tab before).
    fn pick(&mut self, tab: Tab) {
        self.tab = tab;
        if let Some((i, _)) = self.focus {
            self.focus = Some((i, None));
        }
    }

    /// The open panel's page (the caller draws STATES itself).
    pub fn panel_ui(&mut self, ui: &mut egui::Ui, info: &GameInfo) {
        let Tab::Panel(i) = self.tab else { return };
        let memory = self.memory.clone();
        let slot = &mut self.slots[i];
        if let Some(why) = &slot.failed {
            ui.label(
                RichText::new("This panel stopped working and was switched off for this game.")
                    .color(BAD),
            );
            style::hint(ui, why);
            return;
        }
        let mut ctx = PanelCtx::new(info, memory.as_deref(), &mut self.actions);
        let r = catch_unwind(AssertUnwindSafe(|| slot.panel.ui(ui, &mut ctx)));
        if let Err(e) = r {
            slot.failed = Some(panic_text(e));
        }
        if self.memory.is_none() {
            ui.label(RichText::new("Starting...").color(ACCENT));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panel that asks for focus while `on`, and panics in refresh when told to.
    struct Toy {
        on: Arc<std::sync::atomic::AtomicBool>,
        boom: bool,
    }

    impl Panel for Toy {
        fn title(&self) -> String {
            "TOY".into()
        }
        fn refresh(&mut self, _: &GameInfo, _: &Memory) {
            if self.boom {
                panic!("toy panel broke");
            }
        }
        fn ui(&mut self, _: &mut egui::Ui, _: &mut PanelCtx) {}
        fn wants_focus(&self) -> bool {
            self.on.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    fn host(boom: bool) -> (PanelHost, Arc<std::sync::atomic::AtomicBool>, GameInfo) {
        let on = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let info = GameInfo::from_rom("t", Arc::from(vec![0u8; 0x200]));
        let slots = vec![
            Slot {
                panel: Box::new(Toy {
                    on: Arc::new(Default::default()),
                    boom: false,
                }),
                failed: None,
            },
            Slot {
                panel: Box::new(Toy {
                    on: on.clone(),
                    boom,
                }),
                failed: None,
            },
        ];
        (
            PanelHost {
                slots,
                tab: Tab::Panel(0),
                focus: None,
                memory: None,
                actions: Vec::new(),
            },
            on,
            info,
        )
    }

    #[test]
    fn focus_opens_and_goes_back() {
        let (mut h, on, info) = host(false);
        h.tab = Tab::States;
        on.store(true, std::sync::atomic::Ordering::Relaxed);
        h.feed(&info, None);
        assert_eq!(h.tab, Tab::Panel(1));
        on.store(false, std::sync::atomic::Ordering::Relaxed);
        h.feed(&info, None);
        assert_eq!(h.tab, Tab::States);
    }

    #[test]
    fn the_players_pick_wins_over_focus() {
        let (mut h, on, info) = host(false);
        on.store(true, std::sync::atomic::Ordering::Relaxed);
        h.feed(&info, None);
        h.pick(Tab::States);
        h.feed(&info, None);
        assert_eq!(
            h.tab,
            Tab::States,
            "not taken back while it still wants focus"
        );
        on.store(false, std::sync::atomic::Ordering::Relaxed);
        h.feed(&info, None);
        assert_eq!(h.tab, Tab::States);
    }

    #[test]
    fn a_panicking_panel_is_switched_off_alone() {
        let (mut h, _, info) = host(true);
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        h.feed(&info, Some(Arc::new(Memory::default())));
        std::panic::set_hook(prev);
        assert!(h.slots[0].failed.is_none());
        assert_eq!(h.slots[1].failed.as_deref(), Some("toy panel broke"));
    }
}
