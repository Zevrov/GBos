//! The HUD over the game: your party (levels, HP, status), your Poké Balls and a BAG button on the game view while
//! you walk around, all clickable. A party member opens its summary, the balls or BAG a compact bag; the game keeps
//! running under them. It steps aside for battles, menus and conversations. After the Android app's FieldHud.kt
//! (its MINIMAP, WILD and FLY need the map and encounter tables, not ported yet).
//!
//! It only reads the game: the panels' memory copy, through the companion crate's readers.

use adventurepad_sdk::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use adventurepad_sdk::style::{ACCENT, BAD, CARD, GOOD, MUTED, WARN};
use adventurepad_sdk::{GameInfo, Memory};
use companion::tables::{item_description, item_name, species_name};
use companion::{Game, GameKind, Mon, Pocket, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What the emulator thread says about the screen, for the HUD (`Shared::field`): EXPAND's scene when it draws
/// the game, else unknown (the HUD then goes by the game's main callback, [`Hud::on_field`]).
pub const FIELD_UNKNOWN: u8 = 0;
pub const FIELD_NO: u8 = 1;
pub const FIELD_YES: u8 = 2;

/// Frames of plain overworld before the HUD comes back (~1/3 s at 1x): a conversation's lines don't make it blink.
pub const FIELD_SETTLE_FRAMES: u32 = 20;

/// Where the HUD's pieces sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HudLayout {
    /// The party bottom left, the balls over BAG bottom right (the games slide an area's name in top left).
    Corners,
    /// CORNERS with the right side up: BAG over the balls, top right.
    Corners2,
    /// Two columns beside the GBA's own 3:2 (the black bars, or EXPAND's margins), over the game if there's no room.
    Rails,
    /// One bar along the bottom: the party, the balls, BAG.
    Dock,
}

/// How the HUD's windows are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HudLook {
    /// See-through dark panels, light text: as little of the game hidden as can be.
    Glass,
    /// The same in white, dark text.
    WhiteGlass,
    /// The app's own windows.
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HudSize {
    Small,
    Normal,
    Large,
}

impl HudLayout {
    pub const ALL: [HudLayout; 4] = [
        HudLayout::Corners,
        HudLayout::Corners2,
        HudLayout::Rails,
        HudLayout::Dock,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HudLayout::Corners => "CORNERS",
            HudLayout::Corners2 => "CORNERS 2",
            HudLayout::Rails => "RAILS",
            HudLayout::Dock => "DOCK",
        }
    }
}

impl HudLook {
    pub const ALL: [HudLook; 3] = [HudLook::Glass, HudLook::WhiteGlass, HudLook::Window];

    pub fn label(self) -> &'static str {
        match self {
            HudLook::Glass => "GLASS",
            HudLook::WhiteGlass => "WHITE GLASS",
            HudLook::Window => "WINDOW",
        }
    }
}

impl HudSize {
    pub const ALL: [HudSize; 3] = [HudSize::Small, HudSize::Normal, HudSize::Large];

    pub fn label(self) -> &'static str {
        match self {
            HudSize::Small => "SMALL",
            HudSize::Normal => "NORMAL",
            HudSize::Large => "LARGE",
        }
    }

    fn scale(self) -> f32 {
        match self {
            HudSize::Small => 0.8,
            HudSize::Normal => 1.0,
            HudSize::Large => 1.3,
        }
    }
}

/// SETTINGS > HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudStyle {
    pub on: bool,
    pub layout: HudLayout,
    pub look: HudLook,
    pub size: HudSize,
    /// The parts that show.
    pub party: bool,
    pub balls: bool,
    pub bag: bool,
    /// The battle buttons (FIGHT / POKéMON / RUN) beside a battle, and their lists' hints (SUPER, RESISTED...).
    pub battle_buttons: bool,
    pub battle_hints: bool,
}

impl Default for HudStyle {
    fn default() -> HudStyle {
        HudStyle {
            on: true,
            layout: HudLayout::Corners,
            look: HudLook::Glass,
            size: HudSize::Normal,
            party: true,
            balls: true,
            bag: true,
            battle_buttons: true,
            battle_hints: true,
        }
    }
}

/// What the HUD has open over the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    /// A party member's summary (its slot).
    Summary(usize),
    Bag(Pocket),
}

/// Where the game is in the window, for the HUD to keep to.
#[derive(Debug, Clone, Copy)]
pub struct Place {
    /// The whole game view.
    pub view: Rect,
    /// The game's picture in it (EXPAND: the whole view).
    pub picture: Rect,
    /// EXPAND draws the game: the GBA's own 3:2 is the view's middle.
    pub expanding: bool,
    /// Room kept clear at the view's top (the bar of buttons over the game).
    pub top_clear: f32,
}

/// The HUD's colours in one look.
struct Ink {
    fill: Color32,
    stroke: Stroke,
    text: Color32,
    muted: Color32,
    hover: Color32,
    /// The summary's and the bag's windows: nearly solid, so their text reads over anything.
    panel: Color32,
}

fn ink(look: HudLook) -> Ink {
    match look {
        HudLook::Glass => Ink {
            fill: Color32::from_black_alpha(150),
            stroke: Stroke::new(1.0_f32, Color32::from_white_alpha(40)),
            text: Color32::WHITE,
            muted: Color32::from_gray(190),
            hover: Color32::from_white_alpha(30),
            panel: Color32::from_rgba_unmultiplied(12, 10, 18, 236),
        },
        HudLook::WhiteGlass => Ink {
            fill: Color32::from_white_alpha(180),
            stroke: Stroke::new(1.0_f32, Color32::from_black_alpha(50)),
            text: Color32::from_gray(24),
            muted: Color32::from_gray(80),
            hover: Color32::from_black_alpha(25),
            panel: Color32::from_rgba_unmultiplied(246, 246, 250, 240),
        },
        HudLook::Window => Ink {
            fill: CARD,
            stroke: Stroke::new(1.0_f32, Color32::from_gray(70)),
            text: Color32::WHITE,
            muted: MUTED,
            hover: Color32::from_white_alpha(18),
            panel: CARD,
        },
    }
}

/// The ball items, as the bag's ids (the same in FireRed, LeafGreen, Emerald and Ruby / Sapphire): the colour of
/// each one's top half.
fn ball_color(item: u16) -> Color32 {
    match item {
        1 => Color32::from_rgb(0x90, 0x50, 0xC8),  // MASTER
        2 => Color32::from_rgb(0x38, 0x38, 0x38),  // ULTRA
        3 => Color32::from_rgb(0x30, 0x70, 0xD8),  // GREAT
        5 => Color32::from_rgb(0x58, 0xA8, 0x48),  // SAFARI
        6 => Color32::from_rgb(0x40, 0xB8, 0xB8),  // NET
        7 => Color32::from_rgb(0x28, 0x58, 0xA8),  // DIVE
        8 => Color32::from_rgb(0x98, 0xC0, 0x40),  // NEST
        9 => Color32::from_rgb(0xE8, 0x78, 0x30),  // REPEAT
        10 => Color32::from_rgb(0xD8, 0xD8, 0xD8), // TIMER
        11 => Color32::from_rgb(0x28, 0x28, 0x28), // LUXURY
        12 => Color32::from_rgb(0xF4, 0xF4, 0xF4), // PREMIER
        _ => Color32::from_rgb(0xE0, 0x38, 0x38),  // POKé (and any other)
    }
}

/// A Poké Ball drawn at `c`: its top half in the ball's colour.
fn paint_ball(p: &egui::Painter, c: Pos2, r: f32, item: u16) {
    let edge = Stroke::new((r / 6.0).max(1.0), Color32::from_gray(20));
    p.circle_filled(c, r, Color32::WHITE);
    let top: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::convex_polygon(
        top,
        ball_color(item),
        Stroke::NONE,
    ));
    p.line_segment([c - Vec2::new(r, 0.0), c + Vec2::new(r, 0.0)], edge);
    p.circle_stroke(c, r, edge);
    p.circle_filled(c, r * 0.32, Color32::WHITE);
    p.circle_stroke(c, r * 0.32, edge);
}

fn hp_color(hp: u16, max: u16) -> Color32 {
    let f = if max == 0 {
        0.0
    } else {
        hp as f32 / max as f32
    };
    if f > 0.5 {
        GOOD
    } else if f > 0.2 {
        WARN
    } else {
        BAD
    }
}

fn paint_hp(p: &egui::Painter, r: Rect, hp: u16, max: u16) {
    let f = if max == 0 {
        0.0
    } else {
        (hp as f32 / max as f32).min(1.0)
    };
    p.rect_filled(r, r.height() / 2.0, Color32::from_gray(45));
    if f > 0.0 {
        p.rect_filled(
            Rect::from_min_size(r.min, Vec2::new(r.width() * f, r.height())),
            r.height() / 2.0,
            hp_color(hp, max),
        );
    }
}

/// The HUD for one game. None for a game the companion can't read (the HUD needs the party).
pub struct Hud {
    game: Game,
    kind: GameKind,
    snap: Option<Snapshot>,
    /// The memory copy last read (its frame).
    read_frame: Option<u64>,
    /// Without EXPAND's scene: how often each `gMain.callback2` was seen with a party, out of battle. The overworld's
    /// is far the most common, so the most seen one is taken as the field ([`Hud::on_field`]).
    callbacks: HashMap<u32, u32>,
    callback: Option<u32>,
    pub open: Option<Open>,
    /// The battle buttons' list open, and whether the battle's menu has been read this battle.
    battle_open: Option<BattleList>,
    battle_read: bool,
}

impl Hud {
    pub fn for_game(info: &GameInfo) -> Option<Hud> {
        let game = panels_pokemon::detect(info);
        let kind = game.kind.filter(|_| game.config.is_some())?;
        Some(Hud {
            game,
            kind,
            snap: None,
            read_frame: None,
            callbacks: HashMap::new(),
            callback: None,
            open: None,
            battle_open: None,
            battle_read: false,
        })
    }

    /// Reads the game from a new memory copy (4 a second).
    pub fn feed(&mut self, memory: &Memory) {
        if self.read_frame == Some(memory.frame) {
            return;
        }
        self.read_frame = Some(memory.frame);
        self.snap = panels_pokemon::read(&self.game, memory);
        let Some(config) = self.game.config else {
            return;
        };
        self.callback = memory.u32(config.g_main + 4);
        if let (Some(s), Some(cb)) = (&self.snap, self.callback) {
            if !s.in_battle && !s.party.is_empty() && cb != 0 {
                *self.callbacks.entry(cb).or_default() += 1;
            }
        }
    }

    /// The overworld is on screen: EXPAND's scene (`field`, [`FIELD_YES`]...) when it draws the game, else the
    /// game's main callback being the one seen most with a party out of battle (after a couple of seconds of it).
    fn on_field(&self, field: u8) -> bool {
        match field {
            FIELD_YES => true,
            FIELD_NO => false,
            _ => {
                let most = self.callbacks.iter().max_by_key(|(_, n)| **n);
                matches!((most, self.callback), (Some((cb, n)), Some(now)) if *cb == now && *n >= 8)
            }
        }
    }

    /// Whether the HUD shows now.
    fn shown(&self, style: &HudStyle, field: u8) -> bool {
        style.on
            && self.on_field(field)
            && self
                .snap
                .as_ref()
                .is_some_and(|s| !s.in_battle && !s.party.is_empty())
    }

    #[cfg(test)]
    pub fn shown_for_test(&self, style: &HudStyle, field: u8) -> bool {
        self.shown(style, field)
    }

    #[cfg(test)]
    pub fn has_party_for_test(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| !s.party.is_empty())
    }

    #[cfg(test)]
    pub fn in_battle_for_test(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.in_battle)
    }

    /// The bag's balls, most first (three at most).
    fn balls(&self) -> Vec<(u16, u16)> {
        let mut b: Vec<(u16, u16)> = self
            .snap
            .as_ref()
            .and_then(|s| s.bag.as_ref())
            .map(|bag| {
                bag[Pocket::Balls as usize]
                    .iter()
                    .filter(|i| i.quantity > 0)
                    .map(|i| (i.id, i.quantity))
                    .collect()
            })
            .unwrap_or_default();
        b.sort_by_key(|i| std::cmp::Reverse(i.1));
        b.truncate(3);
        b
    }

    /// Draws the HUD (and what it has open) over the game view.
    pub fn ui(&mut self, ui: &mut egui::Ui, place: Place, style: &HudStyle, field: u8) {
        let shown = self.shown(style, field);
        if !shown {
            // A battle, a menu or a conversation: whatever was open goes with the HUD.
            self.open = None;
        }
        let fade = ui
            .ctx()
            .animate_bool_with_time(egui::Id::new("hud-shown"), shown, 0.18);
        if fade <= 0.0 {
            return;
        }
        let Some(snap) = self.snap.clone() else {
            return;
        };
        let ink = ink(style.look);
        let u = style.size.scale();
        let mut painter = ui.painter().with_clip_rect(place.view);
        painter.set_opacity(fade);
        let d = Draw {
            ui,
            painter,
            ink: &ink,
            u,
            kind: self.kind,
            interact: shown,
        };
        let mut clicked = None;
        let balls = self.balls();
        let layout = layout(&snap, style, place, u, balls.len());
        d.draw(&snap, &layout, &balls, self.open, &mut clicked);
        if let Some(c) = clicked {
            self.open = if self.open == Some(c) { None } else { Some(c) };
        }
        if shown {
            self.panels(ui, &snap, place, &ink, layout.panel_anchor);
        }
    }

    /// The summary or the bag, over the game.
    fn panels(
        &mut self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        place: Place,
        ink: &Ink,
        anchor: Anchor,
    ) {
        let Some(open) = self.open else { return };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = None;
            return;
        }
        let top = place.view.top() + place.top_clear;
        let room = (place.view.bottom() - top - 12.0).max(120.0);
        let (pos, pivot, width) = match (open, anchor) {
            (Open::Summary(_), _) => (
                Pos2::new(place.view.left() + 12.0, top),
                egui::Align2::LEFT_TOP,
                380.0,
            ),
            (Open::Bag(_), Anchor::BottomRight(p)) => (p, egui::Align2::RIGHT_BOTTOM, 300.0),
            (Open::Bag(_), Anchor::TopRight(p)) => (p, egui::Align2::RIGHT_TOP, 300.0),
        };
        let height = match open {
            Open::Summary(_) => room,
            Open::Bag(_) => room.min(420.0),
        };
        let mut next = Some(open);
        egui::Area::new(egui::Id::new("hud-panel"))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(pos)
            .constrain_to(place.view)
            .show(ui.ctx(), |ui| {
                egui::Frame::none()
                    .fill(ink.panel)
                    .stroke(ink.stroke)
                    .rounding(6.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.visuals_mut().override_text_color = Some(ink.text);
                        match open {
                            Open::Summary(i) => {
                                next = self.summary_panel(ui, snap, i, ink, height);
                            }
                            Open::Bag(p) => {
                                next = self.bag_panel(ui, snap, p, ink, height);
                            }
                        }
                    });
            });
        self.open = next;
    }

    fn summary_panel(
        &self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        i: usize,
        ink: &Ink,
        height: f32,
    ) -> Option<Open> {
        let mons: Vec<usize> = (0..snap.party.len())
            .filter(|&k| !snap.party[k].is_egg())
            .collect();
        let at = mons.iter().position(|&k| k == i)?;
        let mut next = Some(Open::Summary(i));
        ui.horizontal(|ui| {
            ui.label(RichText::new("SUMMARY").strong().color(ink.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("CLOSE").clicked() {
                    next = None;
                }
                if mons.len() > 1 {
                    if ui.button("▼").on_hover_text("Next").clicked() {
                        next = Some(Open::Summary(mons[(at + 1) % mons.len()]));
                    }
                    if ui.button("▲").on_hover_text("Previous").clicked() {
                        next = Some(Open::Summary(mons[(at + mons.len() - 1) % mons.len()]));
                    }
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(height - 60.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                panels_pokemon::summary(ui, self.kind, &snap.party[i])
            });
        next
    }

    fn bag_panel(
        &self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        pocket: Pocket,
        ink: &Ink,
        height: f32,
    ) -> Option<Open> {
        let mut next = Some(Open::Bag(pocket));
        ui.horizontal(|ui| {
            ui.label(RichText::new("BAG").strong().color(ink.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("CLOSE").clicked() {
                    next = None;
                }
            });
        });
        let Some(bag) = snap.bag.as_ref() else {
            ui.label(RichText::new("The bag can't be read yet.").color(ink.muted));
            return next;
        };
        ui.horizontal_wrapped(|ui| {
            for p in Pocket::ALL {
                if ui.selectable_label(p == pocket, p.label()).clicked() {
                    next = Some(Open::Bag(p));
                }
            }
        });
        ui.separator();
        let items = &bag[pocket as usize];
        egui::ScrollArea::vertical()
            .max_height(height - 90.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if items.is_empty() {
                    ui.label(RichText::new("Nothing in this pocket.").color(ink.muted));
                }
                for item in items {
                    ui.horizontal(|ui| {
                        if pocket == Pocket::Balls {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                            paint_ball(ui.painter(), r.center(), 6.5, item.id);
                        }
                        ui.label(RichText::new(item_name(self.kind, item.id)).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if pocket != Pocket::Key {
                                ui.label(format!("x{}", item.quantity));
                            }
                        });
                    });
                    if let Some(desc) = item_description(self.kind, item.id) {
                        ui.label(RichText::new(desc).small().color(ink.muted));
                    }
                    ui.add_space(2.0);
                }
            });
        next
    }
}

/// Where a panel opens from: BAG's corner of the HUD.
#[derive(Debug, Clone, Copy)]
enum Anchor {
    BottomRight(Pos2),
    TopRight(Pos2),
}

/// How the party is drawn: tiles in a row, or rows in a column (RAILS).
#[derive(Debug, Clone, Copy, PartialEq)]
enum PartyShape {
    Tiles,
    Rows,
}

/// Where each part goes this frame.
#[derive(Debug, Clone)]
struct Layout {
    /// The party's window and how it's drawn.
    party: Option<(Rect, PartyShape)>,
    /// The balls' window (rows of ball + count), and BAG's button.
    balls: Option<Rect>,
    bag: Option<Rect>,
    /// DOCK: one window around everything.
    bar: Option<Rect>,
    panel_anchor: Anchor,
}

// Sizes at NORMAL, in points.
const PAD: f32 = 5.0;
const GAP: f32 = 4.0;
const TILE: Vec2 = Vec2::new(66.0, 50.0);
const ROW: Vec2 = Vec2::new(132.0, 32.0);
const BALL_ROW: Vec2 = Vec2::new(66.0, 18.0);
const BAG: Vec2 = Vec2::new(66.0, 28.0);
const MARGIN: f32 = 8.0;

fn layout(snap: &Snapshot, style: &HudStyle, place: Place, u: f32, balls: usize) -> Layout {
    let n = snap.party.len().min(6) as f32;
    let party_on = style.party && n > 0.0;
    let balls_on = style.balls && balls > 0;
    let bag_on = style.bag;
    let tiles = Vec2::new(
        n * TILE.x * u + (n - 1.0).max(0.0) * GAP * u + 2.0 * PAD * u,
        TILE.y * u + 2.0 * PAD * u,
    );
    let balls_size = Vec2::new(
        BALL_ROW.x * u + 2.0 * PAD * u,
        balls as f32 * BALL_ROW.y * u + 2.0 * PAD * u,
    );
    let bag_size = BAG * u;
    let m = MARGIN * u;
    let pic = place.picture.intersect(place.view);
    let top = (place.view.top() + place.top_clear).max(pic.top());
    let mut out = Layout {
        party: None,
        balls: None,
        bag: None,
        bar: None,
        panel_anchor: Anchor::BottomRight(Pos2::new(pic.right() - m, pic.bottom() - m)),
    };
    match style.layout {
        HudLayout::Corners | HudLayout::Corners2 => {
            if party_on {
                out.party = Some((
                    Rect::from_min_size(
                        Pos2::new(pic.left() + m, pic.bottom() - m - tiles.y),
                        tiles,
                    ),
                    PartyShape::Tiles,
                ));
            }
            let right = pic.right() - m;
            if style.layout == HudLayout::Corners {
                // The balls over BAG, in the bottom-right corner; panels open above them.
                let mut y = pic.bottom() - m;
                if bag_on {
                    out.bag = Some(Rect::from_min_size(
                        Pos2::new(right - bag_size.x, y - bag_size.y),
                        bag_size,
                    ));
                    y -= bag_size.y + GAP * u;
                }
                if balls_on {
                    out.balls = Some(Rect::from_min_size(
                        Pos2::new(right - balls_size.x, y - balls_size.y),
                        balls_size,
                    ));
                    y -= balls_size.y + GAP * u;
                }
                out.panel_anchor = Anchor::BottomRight(Pos2::new(right, y));
            } else {
                // BAG toward the screen's edge, the balls under it; panels open below them.
                let mut y = top + m;
                if bag_on {
                    out.bag = Some(Rect::from_min_size(
                        Pos2::new(right - bag_size.x, y),
                        bag_size,
                    ));
                    y += bag_size.y + GAP * u;
                }
                if balls_on {
                    out.balls = Some(Rect::from_min_size(
                        Pos2::new(right - balls_size.x, y),
                        balls_size,
                    ));
                    y += balls_size.y + GAP * u;
                }
                out.panel_anchor = Anchor::TopRight(Pos2::new(right, y));
            }
        }
        HudLayout::Rails => {
            // The GBA's own 3:2: the picture, or the middle of EXPAND's wider view.
            let core = if place.expanding {
                let w = (place.view.height() * 1.5).min(place.view.width());
                Rect::from_center_size(place.view.center(), Vec2::new(w, place.view.height()))
            } else {
                pic
            };
            let rows = Vec2::new(
                ROW.x * u + 2.0 * PAD * u,
                n * ROW.y * u + (n - 1.0).max(0.0) * GAP * u + 2.0 * PAD * u,
            );
            // In the margin beside the 3:2 when it's wide enough, else over the game's edge.
            let left_room = core.left() - place.view.left();
            let lx = if left_room >= rows.x + 2.0 * m {
                place.view.left() + (left_room - rows.x) / 2.0
            } else {
                place.view.left() + m
            };
            if party_on {
                out.party = Some((
                    Rect::from_min_size(Pos2::new(lx, place.view.center().y - rows.y / 2.0), rows),
                    PartyShape::Rows,
                ));
            }
            let col_w = balls_size.x.max(bag_size.x);
            let right_room = place.view.right() - core.right();
            let rx = if right_room >= col_w + 2.0 * m {
                core.right() + (right_room - col_w) / 2.0
            } else {
                place.view.right() - m - col_w
            };
            let h = if balls_on {
                balls_size.y + GAP * u
            } else {
                0.0
            } + if bag_on { bag_size.y } else { 0.0 };
            let mut y = place.view.center().y - h / 2.0;
            if balls_on {
                out.balls = Some(Rect::from_min_size(
                    Pos2::new(rx, y),
                    Vec2::new(col_w, balls_size.y),
                ));
                y += balls_size.y + GAP * u;
            }
            if bag_on {
                out.bag = Some(Rect::from_min_size(
                    Pos2::new(rx, y),
                    Vec2::new(col_w, bag_size.y),
                ));
            }
            out.panel_anchor = Anchor::BottomRight(Pos2::new(
                rx + col_w,
                place.view.center().y - h / 2.0 - GAP * u,
            ));
        }
        HudLayout::Dock => {
            // One bar: the party's tiles, the balls side by side, BAG.
            let ball_w = BALL_ROW.x * u;
            let parts_w = if party_on {
                tiles.x - 2.0 * PAD * u
            } else {
                0.0
            } + if balls_on { balls as f32 * ball_w } else { 0.0 }
                + if bag_on { bag_size.x } else { 0.0 };
            let count = party_on as u32 + balls_on as u32 + bag_on as u32;
            if count == 0 {
                return out;
            }
            let w = parts_w + (count - 1) as f32 * 3.0 * GAP * u + 2.0 * PAD * u;
            let h = TILE.y * u + 2.0 * PAD * u;
            let bar = Rect::from_min_size(
                Pos2::new(pic.center().x - w / 2.0, pic.bottom() - m - h),
                Vec2::new(w, h),
            );
            out.bar = Some(bar);
            let mut x = bar.left() + PAD * u;
            let inner_top = bar.top() + PAD * u;
            if party_on {
                let pw = tiles.x - 2.0 * PAD * u;
                out.party = Some((
                    Rect::from_min_size(Pos2::new(x, inner_top), Vec2::new(pw, TILE.y * u)),
                    PartyShape::Tiles,
                ));
                x += pw + 3.0 * GAP * u;
            }
            if balls_on {
                let bw = balls as f32 * ball_w;
                out.balls = Some(Rect::from_min_size(
                    Pos2::new(x, inner_top),
                    Vec2::new(bw, TILE.y * u),
                ));
                x += bw + 3.0 * GAP * u;
            }
            if bag_on {
                out.bag = Some(Rect::from_min_size(
                    Pos2::new(x, bar.center().y - bag_size.y / 2.0),
                    bag_size,
                ));
            }
            out.panel_anchor = Anchor::BottomRight(Pos2::new(bar.right(), bar.top() - GAP * u));
        }
    }
    out
}

/// One frame's drawing.
struct Draw<'a> {
    ui: &'a mut egui::Ui,
    painter: egui::Painter,
    ink: &'a Ink,
    u: f32,
    kind: GameKind,
    /// Clicks count (not while it's fading out).
    interact: bool,
}

impl Draw<'_> {
    fn window(&self, r: Rect) {
        self.painter
            .rect(r, 6.0 * self.u, self.ink.fill, self.ink.stroke);
    }

    fn click(&mut self, r: Rect, id: &str) -> bool {
        if !self.interact {
            return false;
        }
        let resp = self
            .ui
            .interact(r, egui::Id::new(("hud", id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.hovered() {
            self.painter.rect_filled(r, 4.0 * self.u, self.ink.hover);
        }
        resp.clicked()
    }

    fn text(&self, pos: Pos2, align: egui::Align2, text: &str, size: f32, color: Color32) -> Rect {
        label(&self.painter, pos, align, text, size * self.u, color)
    }

    fn draw(
        mut self,
        snap: &Snapshot,
        l: &Layout,
        balls: &[(u16, u16)],
        open: Option<Open>,
        clicked: &mut Option<Open>,
    ) {
        let u = self.u;
        let docked = l.bar.is_some();
        if let Some(bar) = l.bar {
            self.window(bar);
        }
        // The lead: the first that can battle.
        let lead = snap.party.iter().position(|m| !m.is_egg() && m.hp > 0);
        if let Some((r, shape)) = l.party {
            if !docked {
                self.window(r);
            }
            let inner = if docked { r } else { r.shrink(PAD * u) };
            for (i, m) in snap.party.iter().take(6).enumerate() {
                let cell = match shape {
                    PartyShape::Tiles => Rect::from_min_size(
                        inner.min + Vec2::new(i as f32 * (TILE.x + GAP) * u, 0.0),
                        TILE * u,
                    ),
                    PartyShape::Rows => Rect::from_min_size(
                        inner.min + Vec2::new(0.0, i as f32 * (ROW.y + GAP) * u),
                        Vec2::new(inner.width(), ROW.y * u),
                    ),
                };
                if open == Some(Open::Summary(i)) || lead == Some(i) {
                    let stroke = Stroke::new(
                        1.5_f32,
                        if open == Some(Open::Summary(i)) {
                            ACCENT
                        } else {
                            ACCENT.gamma_multiply(0.5)
                        },
                    );
                    self.painter.rect_stroke(cell, 4.0 * u, stroke);
                }
                if !m.is_egg() && self.click(cell, &format!("mon{i}")) {
                    *clicked = Some(Open::Summary(i));
                }
                self.mon(cell, m, shape);
            }
        }
        if let Some(r) = l.balls {
            if !docked {
                self.window(r);
            }
            if self.click(r, "balls") {
                *clicked = Some(Open::Bag(Pocket::Balls));
            }
            let inner = if docked { r } else { r.shrink(PAD * u) };
            for (k, (item, qty)) in balls.iter().enumerate() {
                let cell = if docked {
                    Rect::from_min_size(
                        inner.min + Vec2::new(k as f32 * BALL_ROW.x * u, 0.0),
                        Vec2::new(BALL_ROW.x * u, inner.height()),
                    )
                } else {
                    Rect::from_min_size(
                        inner.min + Vec2::new(0.0, k as f32 * BALL_ROW.y * u),
                        Vec2::new(inner.width(), BALL_ROW.y * u),
                    )
                };
                let c = Pos2::new(cell.left() + 9.0 * u, cell.center().y);
                paint_ball(&self.painter, c, 6.5 * u, *item);
                self.text(
                    Pos2::new(cell.left() + 20.0 * u, cell.center().y),
                    egui::Align2::LEFT_CENTER,
                    &format!("x{qty}"),
                    12.5,
                    self.ink.text,
                );
                if self.interact {
                    let name = item_name(self.kind, *item);
                    self.ui
                        .interact(cell, egui::Id::new(("hud-ball", k)), Sense::hover())
                        .on_hover_text(name);
                }
            }
        }
        if let Some(r) = l.bag {
            self.painter.rect(
                r,
                6.0 * u,
                if docked {
                    self.ink.hover
                } else {
                    self.ink.fill
                },
                self.ink.stroke,
            );
            if self.click(r, "bag") {
                *clicked = Some(match open {
                    Some(Open::Bag(p)) => Open::Bag(p),
                    _ => Open::Bag(Pocket::Items),
                });
            }
            let lit = matches!(open, Some(Open::Bag(_)));
            self.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "BAG",
                13.0,
                if lit { ACCENT } else { self.ink.text },
            );
        }
    }

    /// One party member: a tile (name, Lv and status, the HP bar) or a row (RAILS).
    fn mon(&self, cell: Rect, m: &Mon, shape: PartyShape) {
        let u = self.u;
        let p = self
            .painter
            .with_clip_rect(cell.intersect(self.painter.clip_rect()));
        let fainted = m.hp == 0 && !m.is_egg();
        let text = if fainted {
            self.ink.muted
        } else {
            self.ink.text
        };
        if m.is_egg() {
            label(
                &p,
                cell.center(),
                egui::Align2::CENTER_CENTER,
                "EGG",
                12.0 * u,
                self.ink.muted,
            );
            return;
        }
        let chip = if fainted {
            Some("FNT")
        } else {
            m.status_label()
        };
        let chip_color = if fainted { BAD } else { WARN };
        match shape {
            PartyShape::Tiles => {
                let x = cell.left() + 4.0 * u;
                label(
                    &p,
                    Pos2::new(x, cell.top() + 3.0 * u),
                    egui::Align2::LEFT_TOP,
                    &m.nickname,
                    11.5 * u,
                    text,
                );
                label(
                    &p,
                    Pos2::new(x, cell.top() + 19.0 * u),
                    egui::Align2::LEFT_TOP,
                    &format!("Lv{}", m.level),
                    11.0 * u,
                    self.ink.muted,
                );
                if let Some(c) = chip {
                    label(
                        &p,
                        Pos2::new(cell.right() - 4.0 * u, cell.top() + 19.0 * u),
                        egui::Align2::RIGHT_TOP,
                        c,
                        10.0 * u,
                        chip_color,
                    );
                }
                let bar = Rect::from_min_size(
                    Pos2::new(x, cell.bottom() - 10.0 * u),
                    Vec2::new(cell.width() - 8.0 * u, 5.0 * u),
                );
                paint_hp(&p, bar, m.hp, m.max_hp);
            }
            PartyShape::Rows => {
                let x = cell.left() + 5.0 * u;
                label(
                    &p,
                    Pos2::new(x, cell.top() + 3.0 * u),
                    egui::Align2::LEFT_TOP,
                    &m.nickname,
                    12.0 * u,
                    text,
                );
                let right = match chip {
                    Some(c) => format!("{c}  Lv{}", m.level),
                    None => format!("Lv{}", m.level),
                };
                label(
                    &p,
                    Pos2::new(cell.right() - 5.0 * u, cell.top() + 3.0 * u),
                    egui::Align2::RIGHT_TOP,
                    &right,
                    11.0 * u,
                    if chip.is_some() {
                        chip_color
                    } else {
                        self.ink.muted
                    },
                );
                let bar = Rect::from_min_size(
                    Pos2::new(x, cell.bottom() - 9.0 * u),
                    Vec2::new(cell.width() - 10.0 * u, 5.0 * u),
                );
                paint_hp(&p, bar, m.hp, m.max_hp);
            }
        }
    }
}

/// Text at `size` points.
fn label_at(
    p: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    size: f32,
    color: Color32,
) -> Rect {
    label(p, pos, align, text, size, color)
}

/// Text at `size` points.
fn label(
    p: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    size: f32,
    color: Color32,
) -> Rect {
    p.text(pos, align, text, FontId::proportional(size), color)
}

// --- The battle buttons ------------------------------------------------------------------------------------------
//
// FIGHT / POKéMON / RUN beside the battle (the Android app's BattleButtons.kt), clickable: FIGHT and POKéMON open a list
// of the moves (with what each does to the foe) or the party, a click on one presses it into the game's own battle
// menu (battle_input.rs: buttons only, as a player would). Single battles in the games battle_input knows.

/// What the emulator thread says about the battle's menu, for the battle buttons.
#[derive(Debug, Clone, Copy)]
pub struct BattleView {
    /// This game has battle buttons (battle_input::Addrs).
    pub known: bool,
    /// battle_input::NONE, ACTION_SELECT...
    pub state: u8,
    /// A pick is on its way: the buttons wait.
    pub busy: bool,
    pub can_switch: bool,
}

/// The list a battle button has open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleList {
    Moves,
    Party,
}

const FIGHT_RED: Color32 = Color32::from_rgb(0xD0, 0x48, 0x40);
const MON_GREEN: Color32 = Color32::from_rgb(0x3C, 0xA0, 0x54);
const RUN_BLUE: Color32 = Color32::from_rgb(0x40, 0x70, 0xC8);

impl Hud {
    /// The battle buttons over the game, in a battle; the pick clicked, if one was.
    pub fn battle_ui(
        &mut self,
        ui: &mut egui::Ui,
        place: Place,
        style: &HudStyle,
        view: BattleView,
    ) -> Option<crate::battle_input::Pick> {
        use crate::battle_input::{self as bi, Pick};
        let snap = self.snap.clone()?;
        let in_battle = snap.in_battle && view.state != bi::NONE;
        // Read once this battle before they show (the menu's state comes a moment after the battle does).
        if !in_battle {
            self.battle_read = false;
            self.battle_open = None;
        } else if view.state != bi::BUSY {
            self.battle_read = true;
        }
        let shown = style.on
            && style.battle_buttons
            && view.known
            && in_battle
            && !snap.is_double
            && self.battle_read;
        // A list closes with the action menu (a pick ran, the turn moved on).
        let keep = match self.battle_open {
            Some(BattleList::Moves) => matches!(view.state, bi::ACTION_SELECT | bi::MOVE_SELECT),
            Some(BattleList::Party) => view.state == bi::ACTION_SELECT,
            None => true,
        };
        if !shown || !keep {
            self.battle_open = None;
        }
        // Out of the way while a list is open, or the game's own bag or party is up.
        let game_menu = matches!(view.state, bi::PARTY_OPEN | bi::BAG_OPEN);
        let fade = ui.ctx().animate_bool_with_time(
            egui::Id::new("battle-buttons"),
            shown && self.battle_open.is_none() && !game_menu,
            0.18,
        );
        if !shown && fade <= 0.0 {
            return None;
        }
        let u = style.size.scale();
        let m = MARGIN * u;
        let core = if place.expanding {
            let w = (place.view.height() * 1.5).min(place.view.width());
            Rect::from_center_size(place.view.center(), Vec2::new(w, place.view.height()))
        } else {
            place.picture.intersect(place.view)
        };
        // In the room right of the game when there's enough, else over its top-right corner (a battle has only its
        // background there).
        let room = place.view.right() - core.right();
        let col_w = (room - 2.0 * m).clamp(130.0 * u, 210.0 * u);
        let x = if room >= col_w + 2.0 * m {
            core.right() + (room - col_w) / 2.0
        } else {
            place.view.right() - m - col_w
        };
        let top = place.view.top() + place.top_clear;
        let gap = 8.0 * u;
        let bh = ((place.view.bottom() - m - top - 2.0 * gap) / 3.0).clamp(36.0 * u, 64.0 * u);
        let actions = view.state == bi::ACTION_SELECT;
        let ready = shown && !view.busy;
        let me = snap.player_battler().cloned();
        let moves: Vec<(usize, u16, u8)> = me
            .as_ref()
            .map(|b| {
                (0..4)
                    .filter(|&k| b.moves[k] != 0)
                    .map(|k| (k, b.moves[k], b.pp[k]))
                    .collect()
            })
            .unwrap_or_default();
        let active = me.as_ref().and_then(|b| {
            snap.party.iter().position(|p| {
                !p.is_egg() && p.species == b.species && p.level == b.level && p.hp == b.hp
            })
        });
        let can_go = |i: usize, p: &Mon| !p.is_egg() && p.hp > 0 && Some(i) != active;
        let others = snap
            .party
            .iter()
            .enumerate()
            .filter(|(i, p)| can_go(*i, p))
            .count();
        let mut pick = None;
        let mut painter = ui.painter().with_clip_rect(place.view);
        painter.set_opacity(fade);
        let buttons = [
            (
                "FIGHT",
                me.as_ref()
                    .map(|b| species_name(b.species))
                    .unwrap_or_default(),
                FIGHT_RED,
                ready && !moves.is_empty() && (actions || view.state == bi::MOVE_SELECT),
            ),
            (
                "POKéMON",
                String::new(),
                MON_GREEN,
                ready && actions && view.can_switch && others > 0,
            ),
            (
                "RUN",
                String::new(),
                RUN_BLUE,
                ready && actions && !snap.is_trainer,
            ),
        ];
        for (k, (label, sub, color, enabled)) in buttons.into_iter().enumerate() {
            let r = Rect::from_min_size(
                Pos2::new(x, top + k as f32 * (bh + gap)),
                Vec2::new(col_w, bh),
            );
            let live = enabled && fade >= 1.0;
            let resp = ui.interact(
                r,
                egui::Id::new(("battle-button", k)),
                if live { Sense::click() } else { Sense::hover() },
            );
            let fill = if !enabled {
                color.gamma_multiply(0.35)
            } else if resp.hovered() {
                color.gamma_multiply(1.15)
            } else {
                color
            };
            painter.rect(
                r,
                8.0 * u,
                fill,
                Stroke::new(2.0_f32 * u, Color32::from_black_alpha(90)),
            );
            let text = if enabled {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(120)
            };
            let mid = if sub.is_empty() {
                r.center()
            } else {
                r.center() - Vec2::new(0.0, 7.0 * u)
            };
            label_at(
                &painter,
                mid,
                egui::Align2::CENTER_CENTER,
                label,
                19.0 * u,
                text,
            );
            if !sub.is_empty() {
                label_at(
                    &painter,
                    r.center() + Vec2::new(0.0, 12.0 * u),
                    egui::Align2::CENTER_CENTER,
                    &sub,
                    11.0 * u,
                    text,
                );
            }
            if live
                && resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
            {
                match k {
                    0 => self.battle_open = Some(BattleList::Moves),
                    1 => self.battle_open = Some(BattleList::Party),
                    _ => pick = Some(Pick::Action(bi::ACTION_RUN)),
                }
            }
        }
        if !shown {
            return pick;
        }
        let Some(list) = self.battle_open else {
            return pick;
        };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.battle_open = None;
            return pick;
        }
        let ink = ink(style.look);
        let foe = snap.foe().cloned();
        let width = 300.0 * u;
        let mut close = false;
        egui::Area::new(egui::Id::new("battle-list"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(Pos2::new(place.view.right() - m, top))
            .constrain_to(place.view)
            .show(ui.ctx(), |ui| {
                egui::Frame::none()
                    .fill(ink.panel)
                    .stroke(ink.stroke)
                    .rounding(8.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.visuals_mut().override_text_color = Some(ink.text);
                        ui.label(
                            RichText::new(if list == BattleList::Moves {
                                "FIGHT"
                            } else {
                                "POKéMON"
                            })
                            .strong()
                            .color(ink.muted),
                        );
                        let enabled = !view.busy;
                        match list {
                            BattleList::Moves => {
                                for &(k, mv, pp) in &moves {
                                    let verdict = foe
                                        .as_ref()
                                        .filter(|_| style.battle_hints)
                                        .map(|f| companion::battle::move_verdict(mv, f.types));
                                    if move_row(ui, &ink, u, mv, pp, verdict, enabled && pp > 0) {
                                        pick = Some(Pick::Move(k as u8));
                                        close = true;
                                    }
                                }
                            }
                            BattleList::Party => {
                                for (i, p) in snap.party.iter().enumerate() {
                                    let why = if p.is_egg() {
                                        Some("EGG")
                                    } else if Some(i) == active {
                                        Some("OUT")
                                    } else if p.hp == 0 {
                                        Some("FNT")
                                    } else {
                                        None
                                    };
                                    if party_row(ui, &ink, u, p, why, enabled && why.is_none()) {
                                        pick = Some(Pick::Switch(p.personality));
                                        close = true;
                                    }
                                }
                            }
                        }
                        ui.add_space(4.0);
                        if ui
                            .add_sized(
                                [width, 26.0 * u],
                                egui::Button::new(RichText::new("CANCEL").strong()),
                            )
                            .clicked()
                        {
                            close = true;
                            // The game's own move menu is up (a pick opened it): back out of it too.
                            if list == BattleList::Moves
                                && view.state == bi::MOVE_SELECT
                                && pick.is_none()
                            {
                                pick = Some(Pick::Back);
                            }
                        }
                    });
            });
        if close {
            self.battle_open = None;
        }
        pick
    }
}

/// A move as a clickable row: its name and PP, its type and power, and (BATTLE HINTS) what it does to the foe.
fn move_row(
    ui: &mut egui::Ui,
    ink: &Ink,
    u: f32,
    mv: u16,
    pp: u8,
    verdict: Option<companion::battle::Verdict>,
    enabled: bool,
) -> bool {
    let (r, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 44.0 * u),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter();
    let ty = companion::tables::move_type(mv);
    let frame = type_color(ty);
    let fill = if enabled && resp.hovered() {
        ink.hover
    } else {
        Color32::TRANSPARENT
    };
    p.rect(
        r,
        6.0 * u,
        fill,
        Stroke::new(
            2.0_f32 * u,
            if enabled {
                frame
            } else {
                frame.gamma_multiply(0.4)
            },
        ),
    );
    let text = if enabled { ink.text } else { ink.muted };
    let x = r.left() + 10.0 * u;
    label_at(
        p,
        Pos2::new(x, r.top() + 5.0 * u),
        egui::Align2::LEFT_TOP,
        &companion::tables::move_name(mv),
        15.0 * u,
        text,
    );
    label_at(
        p,
        Pos2::new(r.right() - 10.0 * u, r.top() + 6.0 * u),
        egui::Align2::RIGHT_TOP,
        &format!("PP {pp}"),
        12.0 * u,
        text,
    );
    let pill = label_pill(
        p,
        Pos2::new(x, r.bottom() - 6.0 * u),
        companion::tables::type_name(ty),
        10.5 * u,
        frame,
    );
    let power = companion::tables::move_power(mv);
    label_at(
        p,
        Pos2::new(pill.right() + 8.0 * u, r.bottom() - 6.0 * u),
        egui::Align2::LEFT_BOTTOM,
        &if power > 0 {
            format!("POW {power}")
        } else {
            "STATUS".into()
        },
        11.0 * u,
        ink.muted,
    );
    if let Some(v) = verdict.filter(|_| power > 0) {
        let color = if v.is_good() {
            GOOD
        } else if v.is_bad() {
            BAD
        } else {
            MUTED
        };
        let g = p.layout_no_wrap(
            v.label().to_owned(),
            FontId::proportional(10.5 * u),
            Color32::WHITE,
        );
        let at = Pos2::new(
            r.right() - 10.0 * u - g.size().x - 8.0 * u,
            r.bottom() - 6.0 * u - g.size().y - 2.0 * u,
        );
        label_pill(
            p,
            Pos2::new(at.x, r.bottom() - 6.0 * u),
            v.label(),
            10.5 * u,
            color,
        );
    }
    enabled
        && resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
}

/// A party member as a clickable row: name and level, the HP bar; `why` it can't go in (OUT, FNT, EGG).
fn party_row(
    ui: &mut egui::Ui,
    ink: &Ink,
    u: f32,
    m: &Mon,
    why: Option<&str>,
    enabled: bool,
) -> bool {
    let (r, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 40.0 * u),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter();
    let fill = if enabled && resp.hovered() {
        ink.hover
    } else {
        Color32::TRANSPARENT
    };
    p.rect(
        r,
        6.0 * u,
        fill,
        Stroke::new(
            1.5_f32 * u,
            if enabled { MON_GREEN } else { ink.stroke.color },
        ),
    );
    let text = if enabled { ink.text } else { ink.muted };
    let x = r.left() + 10.0 * u;
    let name = if m.is_egg() {
        "EGG".to_owned()
    } else {
        format!("{}  Lv{}", m.nickname, m.level)
    };
    label_at(
        p,
        Pos2::new(x, r.top() + 5.0 * u),
        egui::Align2::LEFT_TOP,
        &name,
        14.0 * u,
        text,
    );
    if let Some(w) = why.or_else(|| m.status_label()) {
        label_at(
            p,
            Pos2::new(r.right() - 10.0 * u, r.top() + 6.0 * u),
            egui::Align2::RIGHT_TOP,
            w,
            11.0 * u,
            if why.is_some() { ink.muted } else { WARN },
        );
    }
    if !m.is_egg() {
        let bar = Rect::from_min_size(
            Pos2::new(x, r.bottom() - 11.0 * u),
            Vec2::new(r.width() - 90.0 * u, 5.0 * u),
        );
        paint_hp(p, bar, m.hp, m.max_hp);
        label_at(
            p,
            Pos2::new(r.right() - 10.0 * u, r.bottom() - 5.0 * u),
            egui::Align2::RIGHT_BOTTOM,
            &format!("{}/{}", m.hp, m.max_hp),
            11.0 * u,
            ink.muted,
        );
    }
    enabled
        && resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
}

/// White text on a rounded `color` tag, its bottom-left at `at`; its rect.
fn label_pill(p: &egui::Painter, at: Pos2, text: &str, size: f32, color: Color32) -> Rect {
    let g = p.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE);
    let r = Rect::from_min_size(
        Pos2::new(at.x, at.y - g.size().y - 2.0),
        g.size() + Vec2::new(8.0, 2.0),
    );
    p.rect_filled(r, 3.0, color);
    p.galley(r.min + Vec2::new(4.0, 1.0), g, Color32::WHITE);
    r
}

/// The games' type colours (the Android app's AppColors.typeColors), by the companion's type ids.
fn type_color(ty: u8) -> Color32 {
    match companion::tables::type_name(ty) {
        "NORMAL" => Color32::from_rgb(0xA8, 0xA8, 0x78),
        "FIGHTING" => Color32::from_rgb(0xC0, 0x30, 0x28),
        "FLYING" => Color32::from_rgb(0xA8, 0x90, 0xF0),
        "POISON" => Color32::from_rgb(0xA0, 0x40, 0xA0),
        "GROUND" => Color32::from_rgb(0xE0, 0xC0, 0x68),
        "ROCK" => Color32::from_rgb(0xB8, 0xA0, 0x38),
        "BUG" => Color32::from_rgb(0xA8, 0xB8, 0x20),
        "GHOST" => Color32::from_rgb(0x70, 0x58, 0x98),
        "STEEL" => Color32::from_rgb(0xB8, 0xB8, 0xD0),
        "FIRE" => Color32::from_rgb(0xF0, 0x80, 0x30),
        "WATER" => Color32::from_rgb(0x68, 0x90, 0xF0),
        "GRASS" => Color32::from_rgb(0x78, 0xC8, 0x50),
        "ELECTRIC" => Color32::from_rgb(0xF8, 0xD0, 0x30),
        "PSYCHIC" => Color32::from_rgb(0xF8, 0x58, 0x88),
        "ICE" => Color32::from_rgb(0x98, 0xD8, 0xD8),
        "DRAGON" => Color32::from_rgb(0x70, 0x38, 0xF8),
        "DARK" => Color32::from_rgb(0x70, 0x58, 0x48),
        _ => MUTED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(w: f32, h: f32, expanding: bool) -> Place {
        let view = Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h));
        let picture = if expanding {
            view
        } else {
            let s = (w / 240.0).min(h / 160.0).floor();
            Rect::from_center_size(view.center(), Vec2::new(240.0 * s, 160.0 * s))
        };
        Place {
            view,
            picture,
            expanding,
            top_clear: 44.0,
        }
    }

    fn snap(n: usize) -> Snapshot {
        let mut s = Snapshot::default();
        for i in 0..n {
            s.party.push(Mon {
                species: 1 + i as u16,
                nickname: format!("MON{i}"),
                gender: companion::Gender::None,
                level: 10,
                hp: 20,
                max_hp: 30,
                status: 0,
                moves: [33, 0, 0, 0],
                pp: [35, 0, 0, 0],
                personality: 0,
                held_item: 0,
                experience: 1000,
                stats: [20; 5],
                ivs: [15; 6],
                evs: [0; 6],
            });
        }
        s
    }

    fn inside(outer: Rect, inner: Rect) -> bool {
        outer.expand(0.5).contains_rect(inner)
    }

    #[test]
    fn every_layout_keeps_to_the_view() {
        for layout in HudLayout::ALL {
            for size in HudSize::ALL {
                for (w, h, wide) in [
                    (1280.0, 720.0, false),
                    (1280.0, 720.0, true),
                    (820.0, 640.0, false),
                ] {
                    let style = HudStyle {
                        layout,
                        size,
                        ..Default::default()
                    };
                    let p = place(w, h, wide);
                    let l = super::layout(&snap(6), &style, p, 1.0, 3);
                    let parts = [l.party.map(|p| p.0), l.balls, l.bag, l.bar];
                    for r in parts.into_iter().flatten() {
                        assert!(
                            inside(p.view, r),
                            "{layout:?} {size:?} {w}x{h}: {r:?} outside"
                        );
                        assert!(
                            r.top() >= p.view.top() + 8.0,
                            "{layout:?}: under the bar of buttons"
                        );
                    }
                    assert!(l.party.is_some() && l.balls.is_some() && l.bag.is_some());
                }
            }
        }
    }

    #[test]
    fn parts_switched_off_go() {
        let style = HudStyle {
            party: false,
            balls: false,
            ..Default::default()
        };
        let l = super::layout(&snap(6), &style, place(1280.0, 720.0, false), 1.0, 3);
        assert!(l.party.is_none() && l.balls.is_none() && l.bag.is_some());
        // No balls in the bag: no balls' window either.
        let l = super::layout(
            &snap(6),
            &HudStyle::default(),
            place(1280.0, 720.0, false),
            1.0,
            0,
        );
        assert!(l.balls.is_none());
    }

    #[test]
    fn corners_dont_overlap() {
        for layout in [HudLayout::Corners, HudLayout::Corners2, HudLayout::Dock] {
            let style = HudStyle {
                layout,
                ..Default::default()
            };
            let l = super::layout(&snap(6), &style, place(1280.0, 720.0, false), 1.0, 3);
            let (party, _) = l.party.unwrap();
            for r in [l.balls.unwrap(), l.bag.unwrap()] {
                assert!(!party.intersects(r), "{layout:?}: the party under {r:?}");
            }
            assert!(!l.balls.unwrap().intersects(l.bag.unwrap()), "{layout:?}");
        }
    }

    #[test]
    fn rails_go_beside_the_game_when_theres_room() {
        let style = HudStyle {
            layout: HudLayout::Rails,
            ..Default::default()
        };
        // 1920x720 with EXPAND: the 3:2 is 1080 wide, 420 each side: room for both rails beside it.
        let p = place(1920.0, 720.0, true);
        let l = super::layout(&snap(6), &style, p, 1.0, 3);
        let core = Rect::from_center_size(p.view.center(), Vec2::new(1080.0, 720.0));
        assert!(
            l.party.unwrap().0.right() <= core.left(),
            "the party beside the game"
        );
        assert!(l.bag.unwrap().left() >= core.right(), "BAG beside the game");
    }

    #[test]
    fn style_survives_settings_json() {
        let s = HudStyle {
            layout: HudLayout::Dock,
            look: HudLook::WhiteGlass,
            size: HudSize::Large,
            balls: false,
            ..Default::default()
        };
        let back: HudStyle = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // An older settings.json without it: the default.
        let empty: HudStyle = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, HudStyle::default());
    }
}
