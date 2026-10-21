//! The window: the game library (and where saves go), then a game with its panels beside it, or in a
//! window of their own (a second monitor makes the extra screen).

use crate::audio::{Output, Ring};
use crate::controls::{Control, Keymap};
use crate::emu::{self, Session};
use crate::panels::{self, PanelHost, Tab};
use crate::saves::{self, CloudFolder, Holder, PlayMarker};
use crate::settings::{self, Settings};
use adventurepad_sdk::style::{ACCENT, BAD, GOOD, MUTED, WARN};
use adventurepad_sdk::Action;
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ROM_EXTS: [&str; 3] = ["gba", "gbc", "gb"];
const PANEL_WIDTH: f32 = 460.0;

/// Checks before a game starts: another device has it open, or the sync client left copies.
enum Pending {
    OtherDevice {
        rom: PathBuf,
        save: PathBuf,
        holder: Holder,
    },
    Conflict {
        rom: PathBuf,
        save: PathBuf,
        copies: Vec<PathBuf>,
    },
}

/// SETTINGS' pages.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Controls,
    Screen,
    FastForward,
    Panels,
    Saves,
    About,
}

impl Page {
    const ALL: [Page; 6] = [
        Page::Controls,
        Page::Screen,
        Page::FastForward,
        Page::Panels,
        Page::Saves,
        Page::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Controls => "CONTROLS",
            Page::Screen => "SCREEN",
            Page::FastForward => "FAST-FORWARD",
            Page::Panels => "PANELS",
            Page::Saves => "SAVES",
            Page::About => "ABOUT",
        }
    }
}

/// The game keys and hotkeys one window's input holds this frame.
#[derive(Default)]
struct KeyRead {
    mask: u32,
    ff_held: bool,
    /// Hotkeys pressed this frame (FAST-FORWARD on / off, states, slots).
    pressed: Vec<Control>,
    /// EXPAND's look-around keys held: -1..1 per axis (right / down positive).
    look: (f32, f32),
}

fn read_keys(ctx: &egui::Context, keys: &Keymap) -> KeyRead {
    let mut k = KeyRead::default();
    // A text field (a panel's search box) has the keyboard: the game doesn't.
    if ctx.wants_keyboard_input() {
        return k;
    }
    ctx.input(|i| {
        for c in Control::ALL {
            let Some(key) = keys.key(c) else { continue };
            match (c.button(), c) {
                (Some(bit), _) if i.key_down(key) => k.mask |= bit,
                (None, Control::FastForwardHold) => k.ff_held |= i.key_down(key),
                (None, Control::LookUp) if i.key_down(key) => k.look.1 -= 1.0,
                (None, Control::LookDown) if i.key_down(key) => k.look.1 += 1.0,
                (None, Control::LookLeft) if i.key_down(key) => k.look.0 -= 1.0,
                (None, Control::LookRight) if i.key_down(key) => k.look.0 += 1.0,
                (
                    None,
                    Control::LookUp | Control::LookDown | Control::LookLeft | Control::LookRight,
                ) => {}
                (None, _) if i.key_pressed(key) => k.pressed.push(c),
                _ => {}
            }
        }
    });
    k
}

pub struct AdventurePadApp {
    settings: Settings,
    clouds: Vec<CloudFolder>,
    session: Option<Session>,
    host: Option<PanelHost>,
    ring: Ring,
    _audio: Option<Output>,
    screen: Option<TextureHandle>,
    screen_seq: u64,
    /// Where EXPAND's window goes on the view (None: the plain frame, fitted).
    screen_window: Option<crate::wide::Window>,
    slot: u8,
    thumbs: Vec<(PathBuf, std::time::SystemTime, TextureHandle)>,
    hud: Option<(String, Instant)>,
    pending: Option<Pending>,
    error: Option<String>,
    /// SETTINGS is showing (the game, if any, stands still meanwhile).
    settings_open: bool,
    settings_page: Page,
    /// The control waiting for its new key (SETTINGS > CONTROLS: "Press a key...").
    binding: Option<Control>,
    /// What the last rebind took from another control ("X was B's").
    bind_note: Option<String>,
    /// The library's line per game ("Pokémon (Gen 3) panels"), worked out off the UI thread.
    labels: Arc<Mutex<HashMap<PathBuf, String>>>,
    /// Keys read in the panels' own window, added to the main window's.
    panel_keys: KeyRead,
}

impl AdventurePadApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> AdventurePadApp {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let ring = Ring::default();
        let mut app = AdventurePadApp {
            settings: Settings::load(),
            clouds: saves::cloud_folders(),
            session: None,
            host: None,
            _audio: Output::open(ring.clone()),
            ring,
            screen: None,
            screen_seq: 0,
            screen_window: None,
            slot: 0,
            thumbs: Vec::new(),
            hud: None,
            pending: None,
            error: None,
            labels: Default::default(),
            panel_keys: KeyRead::default(),
            settings_open: false,
            settings_page: Page::Controls,
            binding: None,
            bind_note: None,
        };
        app.load_labels(&cc.egui_ctx);
        if let Some(log) = crate::crash::crashed_last_time() {
            app.error = Some(format!(
                "Adventure Pad closed unexpectedly last time. What happened is in {}",
                log.display()
            ));
        }
        // A game passed on the command line ("Open with Adventure Pad", a shortcut) starts right away.
        if let Some(rom) = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .filter(|p| p.is_file())
        {
            app.try_play(&cc.egui_ctx, rom);
        }
        app
    }

    /// Reads the recent games' ROMs in the background for their library line.
    fn load_labels(&self, ctx: &egui::Context) {
        let todo: Vec<PathBuf> = {
            let have = self.labels.lock().unwrap();
            self.settings
                .recent
                .iter()
                .filter(|r| !have.contains_key(*r))
                .cloned()
                .collect()
        };
        if todo.is_empty() {
            return;
        }
        let (labels, ctx) = (self.labels.clone(), ctx.clone());
        std::thread::spawn(move || {
            for rom in todo {
                let label = match emu::game_info(&rom) {
                    Ok(info) => {
                        let packs = panels::packs_for(&info);
                        let title = if info.header_title.is_empty() {
                            "Unknown game".into()
                        } else {
                            info.header_title
                        };
                        if packs.is_empty() {
                            format!("{title}: plays, no panels for it yet")
                        } else {
                            format!("{title}: {} panels", packs.join(", "))
                        }
                    }
                    Err(e) => format!("Can't read it: {e}"),
                };
                labels.lock().unwrap().insert(rom, label);
                ctx.request_repaint();
            }
        });
    }

    // --- starting and leaving a game -----------------------------------------------------------------

    fn marker(&self, save: &Path) -> PlayMarker {
        PlayMarker::new(save, &settings::device_name(), &self.settings.device_id)
    }

    /// Opens `rom` once the synced-save checks pass (or asks about them first).
    fn try_play(&mut self, ctx: &egui::Context, rom: PathBuf) {
        self.settings.add_recent(&rom);
        self.load_labels(ctx);
        let save = saves::save_for(&self.settings.saves_dir(), &rom);
        if let Some(holder) = self.marker(&save).other_device() {
            self.pending = Some(Pending::OtherDevice { rom, save, holder });
            return;
        }
        let copies = saves::conflict_copies(&save);
        if !copies.is_empty() {
            self.pending = Some(Pending::Conflict { rom, save, copies });
            return;
        }
        self.play(ctx, rom, save);
    }

    fn play(&mut self, ctx: &egui::Context, rom: PathBuf, save: PathBuf) {
        let info = match emu::game_info(&rom) {
            Ok(i) => i,
            Err(e) => {
                self.error = Some(format!("Can't read {}: {e}", rom.display()));
                return;
            }
        };
        let marker = self.marker(&save);
        self.ring.clear();
        self.host = Some(PanelHost::new(&info, self.settings.dev_panels));
        let ctx = ctx.clone();
        let wake = move || ctx.request_repaint();
        self.session = Some(Session::start(
            rom,
            info,
            save,
            marker,
            self.ring.clone(),
            wake,
        ));
        if let Some(s) = &self.session {
            s.apply(&self.settings);
        }
        self.screen_seq = 0;
        self.thumbs.clear();
    }

    fn leave(&mut self) {
        if let Some(mut s) = self.session.take() {
            s.stop();
            if let Some(e) = s
                .shared
                .error
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
            {
                self.error = Some(e);
            }
        }
        self.host = None;
        self.ring.clear();
        self.screen = None;
    }

    // --- input --------------------------------------------------------------------------------------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.session else { return };
        // SETTINGS takes the keys while it's open (a rebind is listening for one).
        if self.settings_open {
            s.shared.keys.store(0, Ordering::Relaxed);
            s.shared.ff_held.store(false, Ordering::Relaxed);
            self.panel_keys = KeyRead::default();
            return;
        }
        let main = read_keys(ctx, &self.settings.keys);
        let extra = std::mem::take(&mut self.panel_keys);
        s.shared
            .keys
            .store(main.mask | extra.mask, Ordering::Relaxed);
        s.shared
            .ff_held
            .store(main.ff_held || extra.ff_held, Ordering::Relaxed);
        let look = (
            (main.look.0 + extra.look.0).clamp(-1.0, 1.0),
            (main.look.1 + extra.look.1).clamp(-1.0, 1.0),
        );
        s.shared.look_x.store(look.0.to_bits(), Ordering::Relaxed);
        s.shared.look_y.store(look.1.to_bits(), Ordering::Relaxed);
        let mut hud = None;
        for k in main.pressed.into_iter().chain(extra.pressed) {
            match k {
                Control::FastForwardToggle => {
                    let on = !s.shared.ff_on.load(Ordering::Relaxed);
                    s.shared.ff_on.store(on, Ordering::Relaxed);
                    let music =
                        s.shared.steady_music.load(Ordering::Relaxed) && self.settings.ff_sound;
                    hud = Some(
                        match (on, music) {
                            (true, true) => "Fast-forward on (music at normal speed)",
                            (true, false) => "Fast-forward on",
                            (false, _) => "Fast-forward off",
                        }
                        .to_owned(),
                    );
                }
                Control::Zoom => s.zoom(),
                Control::SaveState => s.save_state(self.slot),
                Control::LoadState => s.load_state(self.slot),
                Control::PrevSlot | Control::NextSlot => {
                    self.slot = if k == Control::NextSlot {
                        (self.slot + 1) % emu::SLOTS
                    } else {
                        (self.slot + emu::SLOTS - 1) % emu::SLOTS
                    };
                    let used = s.slots()[self.slot as usize].saved.is_some();
                    hud = Some(format!(
                        "Slot {} ({})",
                        self.slot,
                        if used { "used" } else { "empty" }
                    ));
                }
                _ => {}
            }
        }
        if let Some(h) = hud {
            self.show_hud(&h);
        }
    }

    fn show_hud(&mut self, text: &str) {
        self.hud = Some((text.to_owned(), Instant::now()));
    }

    /// What the panels asked for, carried out.
    fn run_actions(&mut self) {
        let (Some(host), Some(s)) = (&mut self.host, &self.session) else {
            return;
        };
        let mut notices = Vec::new();
        for a in host.take_actions() {
            match a {
                Action::SaveState(slot) => s.save_state(slot),
                Action::LoadState(slot) => s.load_state(slot),
                Action::Press { buttons, frames } => s.press(buttons, frames),
                Action::Notice(text) => notices.push(text),
            }
        }
        for n in notices {
            self.show_hud(&n);
        }
    }

    // --- the library ----------------------------------------------------------------------------------

    fn library(&mut self, ctx: &egui::Context) {
        let mut play: Option<PathBuf> = None;
        egui::SidePanel::right("saves")
            .exact_width(440.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                egui::ScrollArea::vertical().show(ui, |ui| self.saves_panel(ui));
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("Adventure Pad")
                    .size(34.0)
                    .strong()
                    .color(ACCENT),
            );
            ui.label(
                "Game Boy, Game Boy Color and GBA, with live panels about your game beside it.",
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Open a game...").size(16.0))
                    .clicked()
                {
                    if let Some(f) = rfd::FileDialog::new()
                        .add_filter("Game Boy / GBA games", &ROM_EXTS)
                        .pick_file()
                    {
                        play = Some(f);
                    }
                }
                if ui.button(RichText::new("Settings").size(16.0)).clicked() {
                    self.open_settings(Page::Controls);
                }
            });
            if let Some(e) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(BAD, e);
            }
            ui.add_space(16.0);
            ui.label(RichText::new("RECENT").strong().color(MUTED));
            if self.settings.recent.is_empty() {
                ui.label(RichText::new("Games you open show up here.").color(MUTED));
            }
            let labels = self.labels.lock().unwrap().clone();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for rom in &self.settings.recent {
                    if !rom.is_file() {
                        continue;
                    }
                    let title = rom
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let sub = labels.get(rom).map_or("...", String::as_str);
                    let r = ui.add(
                        egui::Button::new(RichText::new(format!("{title}\n{sub}")).size(14.0))
                            .min_size(egui::vec2(ui.available_width(), 44.0)),
                    );
                    if r.clicked() {
                        play = Some(rom.clone());
                    }
                }
            });
        });
        if let Some(rom) = play {
            self.error = None;
            self.try_play(ctx, rom);
        }
    }

    fn saves_panel(&mut self, ui: &mut egui::Ui) {
        let dir = self.settings.saves_dir();
        ui.label(RichText::new("SAVES").strong().color(MUTED));
        match saves::cloud_for(&dir, &self.clouds) {
            Some(c) => ui.label(
                RichText::new(format!("Synced with {}", c.name))
                    .strong()
                    .color(GOOD),
            ),
            None => ui.label(RichText::new("On this PC only").strong().color(WARN)),
        };
        ui.label(
            RichText::new(dir.display().to_string())
                .small()
                .color(MUTED),
        );
        ui.add_space(8.0);
        ui.label(
            "Put your saves in a synced folder to carry them between devices: this PC, your phone or handheld \
             (point the Android app's saves folder at the same folder). The game's own saves sync; save states \
             stay on each device.",
        );
        ui.add_space(8.0);
        let mut pick: Option<Option<PathBuf>> = None;
        for c in &self.clouds {
            if ui
                .button(format!("Use {}  ({})", c.name, c.saves_dir().display()))
                .clicked()
            {
                pick = Some(Some(c.saves_dir()));
            }
        }
        if self.clouds.is_empty() {
            ui.label(
                RichText::new(
                    "No OneDrive, Dropbox, Google Drive or iCloud Drive folder found on this PC.",
                )
                .color(MUTED),
            );
        }
        if ui.button("Choose a folder...").clicked() {
            if let Some(d) = rfd::FileDialog::new().set_directory(&dir).pick_folder() {
                pick = Some(Some(d));
            }
        }
        if ui.button("Keep saves on this PC only").clicked() {
            pick = Some(None);
        }
        if let Some(choice) = pick {
            if let Some(d) = &choice {
                let _ = std::fs::create_dir_all(d);
            }
            self.settings.saves_dir = choice;
            self.settings.save();
        }
    }

    fn pending_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pending else { return };
        let mut action: Option<Option<(PathBuf, PathBuf)>> = None;
        let mut use_copy: Option<PathBuf> = None;
        match p {
            Pending::OtherDevice { rom, save, holder } => {
                let mins = (chrono::Utc::now().timestamp_millis() as u64)
                    .saturating_sub(holder.heartbeat)
                    / 60_000;
                egui::Window::new(format!("Still open on {}", holder.device)).collapsible(false).resizable(false).show(ctx, |ui| {
                    ui.label(format!(
                        "This game was open on {} {}. If both play, whichever saves last wins and the other's progress \
                         is lost. Close it there first, and give it a minute to sync.",
                        holder.device,
                        if mins == 0 { "just now".to_owned() } else { format!("{mins} min ago") }
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Play here anyway").clicked() {
                            action = Some(Some((rom.clone(), save.clone())));
                        }
                        if ui.button("Cancel").clicked() {
                            action = Some(None);
                        }
                    });
                });
            }
            Pending::Conflict { rom, save, copies } => {
                egui::Window::new("Two versions of this save").collapsible(false).resizable(false).show(ctx, |ui| {
                    ui.label(
                        "Your sync folder kept more than one copy of this save, from devices that both saved. Pick the \
                         one to play; the current one is kept as a backup.",
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("Current: {}, {}", file_name(save), modified(save))).strong());
                    for c in copies {
                        if ui.button(format!("Use {}, {}", file_name(c), modified(c))).clicked() {
                            use_copy = Some(c.clone());
                            action = Some(Some((rom.clone(), save.clone())));
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Keep current").clicked() {
                            action = Some(Some((rom.clone(), save.clone())));
                        }
                        if ui.button("Cancel").clicked() {
                            action = Some(None);
                        }
                    });
                });
            }
        }
        if let Some(a) = action {
            if let (Some(copy), Some((_, save))) = (&use_copy, &a) {
                if let Err(e) = saves::use_copy(save, copy) {
                    self.error = Some(format!("Couldn't use that copy: {e}"));
                }
            }
            self.pending = None;
            if let Some((rom, save)) = a {
                self.play(ctx, rom, save);
            }
        }
    }

    // --- a game -------------------------------------------------------------------------------------

    fn game(&mut self, ctx: &egui::Context) {
        let shared = self.session.as_ref().unwrap().shared.clone();
        if shared.ended.load(Ordering::SeqCst) {
            self.leave();
            return;
        }
        for n in shared
            .notices
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            self.hud = Some((n, Instant::now()));
        }
        // The newest frame onto the screen texture.
        {
            let f = shared.frame.lock().unwrap_or_else(|p| p.into_inner());
            if f.seq != self.screen_seq && f.width > 0 {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.width, f.height], &f.rgba);
                match &mut self.screen {
                    Some(t) => t.set(img, TextureOptions::NEAREST),
                    None => {
                        self.screen = Some(ctx.load_texture("screen", img, TextureOptions::NEAREST))
                    }
                }
                self.screen_seq = f.seq;
                self.screen_window = f.window;
            }
        }
        let memory = shared
            .memory
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let (Some(host), Some(s)) = (&mut self.host, &self.session) {
            host.feed(&s.info, memory);
        }

        if self.settings.panel_window {
            // The whole app closing closes the panels' window too: that isn't the player docking them.
            let app_closing = ctx.input(|i| i.viewport().close_requested());
            let mut closed = false;
            let mut keys = KeyRead::default();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("adventurepad-panels"),
                egui::ViewportBuilder::default()
                    .with_title("Adventure Pad - Panels")
                    .with_inner_size([PANEL_WIDTH + 20.0, 760.0]),
                |ctx, _class| {
                    keys = read_keys(ctx, &self.settings.keys);
                    egui::CentralPanel::default().show(ctx, |ui| self.panels_ui(ui, ctx));
                    closed = ctx.input(|i| i.viewport().close_requested());
                },
            );
            self.panel_keys = keys;
            if closed && !app_closing {
                // Closing the panels' window docks them again.
                self.settings.panel_window = false;
                self.settings.save();
            }
        } else {
            egui::SidePanel::right("panels")
                .exact_width(PANEL_WIDTH)
                .resizable(false)
                .show(ctx, |ui| {
                    self.panels_ui(ui, ctx);
                });
        }

        let mut leave = false;
        let mut open_settings = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::BLACK))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                // EXPAND's camera works in screen pixels: the view's size, for its ZOOM steps.
                let ppp = ctx.pixels_per_point();
                shared
                    .view_w
                    .store((rect.width() * ppp).round() as u32, Ordering::Relaxed);
                shared
                    .view_h
                    .store((rect.height() * ppp).round() as u32, Ordering::Relaxed);
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                if let (Some(t), Some(win)) = (&self.screen, self.screen_window) {
                    // EXPAND: the window as the camera placed it (whole screen pixels per GBA pixel), clipped to
                    // the view.
                    let min = rect.min + egui::vec2(win.off_x, win.off_y) / ppp;
                    let size = egui::vec2(win.w as f32, win.h as f32) * win.scale / ppp;
                    ui.painter().with_clip_rect(rect).image(
                        t.id(),
                        egui::Rect::from_min_size(min, size),
                        uv,
                        Color32::WHITE,
                    );
                } else if let Some(t) = &self.screen {
                    let [w, h] = t.size().map(|v| v as f32);
                    let fit = (rect.width() / w).min(rect.height() / h);
                    let scale = if fit >= 1.0 { fit.floor() } else { fit };
                    let r = egui::Rect::from_center_size(
                        rect.center(),
                        egui::vec2(w * scale, h * scale),
                    );
                    ui.painter().image(t.id(), r, uv, Color32::WHITE);
                } else if let Some(e) = shared
                    .error
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        e,
                        egui::FontId::proportional(18.0),
                        BAD,
                    );
                }
                let bar = egui::Rect::from_min_size(
                    rect.min + egui::vec2(8.0, 8.0),
                    egui::vec2(rect.width() - 16.0, 28.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Library").clicked() {
                            leave = true;
                        }
                        let ff = shared.ff_on.load(Ordering::Relaxed);
                        if ui.selectable_label(ff, "Fast-forward").clicked() {
                            shared.ff_on.store(!ff, Ordering::Relaxed);
                        }
                        if ui.button("Reset").clicked() {
                            if let Some(s) = &self.session {
                                s.reset();
                            }
                        }
                        let label = if self.settings.panel_window {
                            "Dock panels"
                        } else {
                            "Pop out panels"
                        };
                        if ui.button(label).clicked() {
                            self.settings.panel_window = !self.settings.panel_window;
                            self.settings.save();
                        }
                        if ui.button("Settings").clicked() {
                            open_settings = true;
                        }
                    });
                });
                if let Some((text, at)) = &self.hud {
                    if at.elapsed() < Duration::from_millis(1800) {
                        let p = egui::pos2(rect.right() - 12.0, rect.top() + 12.0);
                        let galley = ui.painter().layout_no_wrap(
                            text.clone(),
                            egui::FontId::proportional(16.0),
                            Color32::WHITE,
                        );
                        let bg = egui::Rect::from_min_size(
                            p - egui::vec2(galley.size().x + 16.0, 0.0),
                            galley.size() + egui::vec2(16.0, 10.0),
                        );
                        ui.painter()
                            .rect_filled(bg, 4.0, Color32::from_black_alpha(200));
                        ui.painter()
                            .galley(bg.min + egui::vec2(8.0, 5.0), galley, Color32::WHITE);
                    }
                }
            });
        self.run_actions();
        if open_settings {
            self.open_settings(Page::Controls);
        }
        if leave {
            self.leave();
        }
    }

    /// The tab bar and the open tab: a panel, or STATES.
    fn panels_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(6.0);
        let Some(host) = &mut self.host else { return };
        host.tabs_ui(ui);
        if host.is_empty() {
            ui.label(
                RichText::new(
                    "No panels for this game yet: the game plays, with save states here.",
                )
                .color(MUTED),
            );
        }
        ui.separator();
        let tab = host.tab;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match tab {
                Tab::States => self.states_tab(ui, ctx),
                Tab::Panel(_) => {
                    if let (Some(host), Some(s)) = (&mut self.host, &self.session) {
                        host.panel_ui(ui, &s.info);
                    }
                }
            });
    }

    fn states_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(slots) = self.session.as_ref().map(|s| s.slots()) else {
            return;
        };
        let mut save = None;
        let mut load = None;
        ui.label(
            RichText::new("Save states stay on this PC. F5 / F7 save / load the selected slot.")
                .small()
                .color(MUTED),
        );
        egui::Grid::new("slots")
            .num_columns(2)
            .spacing([10.0, 10.0])
            .show(ui, |ui| {
                for info in &slots {
                    let thumb = info.saved.and_then(|_| self.thumb(ctx, &info.thumb));
                    ui.vertical(|ui| {
                        let selected = self.slot == info.slot;
                        let label = RichText::new(format!("SLOT {}", info.slot))
                            .strong()
                            .color(if selected { ACCENT } else { Color32::WHITE });
                        if ui.selectable_label(selected, label).clicked() {
                            self.slot = info.slot;
                        }
                        match &thumb {
                            Some(t) => {
                                ui.image((t.id(), egui::vec2(160.0, 106.0)));
                            }
                            None => {
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(160.0, 106.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(r, 4.0, Color32::from_gray(30));
                                let text = if info.saved.is_some() {
                                    "SAVED"
                                } else {
                                    "EMPTY"
                                };
                                ui.painter().text(
                                    r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    text,
                                    egui::FontId::proportional(13.0),
                                    MUTED,
                                );
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("SAVE").clicked() {
                                save = Some(info.slot);
                            }
                            if info.saved.is_some() && ui.small_button("LOAD").clicked() {
                                load = Some(info.slot);
                            }
                        });
                    });
                    if info.slot % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        let Some(session) = &self.session else { return };
        if let Some(slot) = save {
            session.save_state(slot);
            self.slot = slot;
        }
        if let Some(slot) = load {
            session.load_state(slot);
            self.slot = slot;
        }
    }

    /// A state's thumbnail as a texture, decoded once per file version.
    fn thumb(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        if let Some((_, _, t)) = self
            .thumbs
            .iter()
            .find(|(p, at, _)| p == path && *at == stamp)
        {
            return Some(t.clone());
        }
        let decoder = png::Decoder::new(std::fs::File::open(path).ok()?);
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).ok()?;
        if info.color_type != png::ColorType::Rgba {
            return None;
        }
        let img = egui::ColorImage::from_rgba_unmultiplied(
            [info.width as usize, info.height as usize],
            &buf[..info.buffer_size()],
        );
        let tex = ctx.load_texture(path.display().to_string(), img, TextureOptions::NEAREST);
        self.thumbs.retain(|(p, _, _)| p != path);
        self.thumbs.push((path.to_path_buf(), stamp, tex.clone()));
        Some(tex)
    }
}

impl AdventurePadApp {
    // --- SETTINGS -----------------------------------------------------------------------------------

    fn open_settings(&mut self, page: Page) {
        self.settings_open = true;
        self.settings_page = page;
        self.binding = None;
        self.bind_note = None;
        if let Some(s) = &self.session {
            s.set_paused(true);
        }
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        self.binding = None;
        self.settings.save();
        if let Some(s) = &self.session {
            s.apply(&self.settings);
            s.set_paused(false);
        }
    }

    /// The whole window while it's open: pages down the left, the page on the right.
    fn settings_screen(&mut self, ctx: &egui::Context) {
        // A rebind takes the next key pressed; ESC lets it be. Without one, ESC leaves SETTINGS.
        let pressed: Option<egui::Key> = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } => Some(*key),
                _ => None,
            })
        });
        match (self.binding, pressed) {
            (Some(_), Some(egui::Key::Escape)) => self.binding = None,
            (Some(c), Some(k)) => {
                self.bind_note = self.settings.keys.bind(c, k).map(|o| {
                    format!(
                        "{} was {}'s, so {} has no key now",
                        k.name(),
                        o.label(),
                        o.label()
                    )
                });
                self.binding = None;
                self.settings.save();
            }
            (None, Some(egui::Key::Escape)) => {
                self.close_settings();
                return;
            }
            _ => {}
        }

        let mut done = false;
        egui::SidePanel::left("settings-pages")
            .exact_width(200.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("SETTINGS").size(22.0).strong().color(ACCENT));
                if self.session.is_some() {
                    ui.label(RichText::new("The game is paused.").small().color(MUTED));
                }
                ui.add_space(12.0);
                for p in Page::ALL {
                    let label = RichText::new(p.label()).strong().size(15.0);
                    if ui
                        .selectable_label(self.settings_page == p, label)
                        .clicked()
                    {
                        self.settings_page = p;
                        self.binding = None;
                    }
                }
                ui.add_space(20.0);
                let back = if self.session.is_some() {
                    "Back to the game"
                } else {
                    "Done"
                };
                if ui.button(RichText::new(back).size(15.0)).clicked() {
                    done = true;
                }
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.settings_page {
                    Page::Controls => self.controls_page(ui),
                    Page::FastForward => self.fast_forward_page(ui),
                    Page::Panels => self.panels_page(ui),
                    Page::Saves => self.saves_panel(ui),
                    Page::Screen => self.screen_page(ui),
                    Page::About => about_page(ui),
                });
        });
        if done {
            self.close_settings();
        }
    }

    fn controls_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("CONTROLS").strong().color(MUTED));
        ui.label("Click a control, then press the key you want for it. ESC cancels.");
        ui.add_space(8.0);
        let mut unbind = None;
        egui::Grid::new("controls")
            .num_columns(3)
            .spacing([24.0, 8.0])
            .striped(true)
            .show(ui, |ui| {
                for c in Control::ALL {
                    ui.label(RichText::new(c.label()).strong());
                    let listening = self.binding == Some(c);
                    let text = if listening {
                        "Press a key...".to_owned()
                    } else {
                        self.settings.keys.key_label(c)
                    };
                    let b = egui::Button::new(RichText::new(text).monospace())
                        .min_size(egui::vec2(140.0, 0.0))
                        .selected(listening);
                    if ui.add(b).clicked() {
                        self.binding = if listening { None } else { Some(c) };
                        self.bind_note = None;
                    }
                    if ui.small_button("clear").clicked() {
                        unbind = Some(c);
                    }
                    ui.end_row();
                }
            });
        if let Some(c) = unbind {
            self.settings.keys.unbind(c);
            self.settings.save();
        }
        if let Some(n) = &self.bind_note {
            ui.add_space(6.0);
            ui.label(RichText::new(n).color(WARN));
        }
        ui.add_space(12.0);
        if ui.button("Back to the default keys").clicked() {
            self.settings.keys.reset();
            self.bind_note = None;
            self.settings.save();
        }
    }

    fn fast_forward_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("FAST-FORWARD").strong().color(MUTED));
        ui.label(format!(
            "Hold {} or press {} to turn it on and off.",
            self.settings.keys.key_label(Control::FastForwardHold),
            self.settings.keys.key_label(Control::FastForwardToggle)
        ));
        ui.add_space(10.0);
        ui.label(RichText::new("SPEED").strong());
        ui.horizontal(|ui| {
            for (v, label) in [
                (2.0, "2x"),
                (3.0, "3x"),
                (4.0, "4x"),
                (8.0, "8x"),
                (0.0, "As fast as it goes"),
            ] {
                if ui
                    .selectable_label(self.settings.ff_speed == v, label)
                    .clicked()
                {
                    self.settings.ff_speed = v;
                }
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new("SOUND").strong());
        ui.radio_value(
            &mut self.settings.ff_sound,
            true,
            "Music at normal speed: the song keeps playing at its own tempo while the game races ahead",
        );
        ui.label(
            RichText::new(
                "Pokémon GBA games and their hacks: the song at normal speed, with the game's sound effects and \
                 cries over it, in step with the game. Other games: their sound at its normal pitch, skipping \
                 ahead.",
            )
            .small()
            .color(MUTED),
        );
        ui.radio_value(
            &mut self.settings.ff_sound,
            false,
            "Off: silent while fast-forwarding",
        );
        ui.add_space(10.0);
        ui.add_enabled_ui(self.settings.ff_sound, |ui| {
            ui.label(RichText::new("SOUND EFFECTS AND CRIES").strong());
            ui.radio_value(
                &mut self.settings.ff_effects_pitch,
                true,
                "Their own pitch: shorter, so they keep up with the game",
            );
            ui.radio_value(
                &mut self.settings.ff_effects_pitch,
                false,
                "Sped up: higher, the way the game makes them at speed",
            );
        });
    }

    fn screen_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("SCREEN").strong().color(MUTED));
        ui.checkbox(&mut self.settings.expand, RichText::new("EXPAND").strong());
        ui.label(
            "The overworld drawn past the GBA's edges, so a wide window shows more of the map with the pixels kept              square. The strips beside the game are rebuilt every frame from the game's own map, tiles and sprites              (it only reads the game, never changes it). Battles, menus and title screens get a glow from the              game's edges instead.",
        );
        ui.label(
            RichText::new(
                "For FireRed, LeafGreen, Emerald, Ruby and Sapphire in every language and revision, and the ROM                  hacks PokeDaisy's EXPAND knows (Unbound, Radical Red, Lazarus, SoulGold, Emerald Rogue and more).                  Other games show as they are.",
            )
            .small()
            .color(MUTED),
        );
        if let Some(s) = &self.session {
            let on = s.shared.expanding.load(Ordering::Relaxed);
            ui.label(
                RichText::new(if on {
                    "This game: EXPAND is on."
                } else {
                    "This game: EXPAND is off."
                })
                .small()
                .color(if on { GOOD } else { MUTED }),
            );
        }
        ui.add_space(10.0);
        ui.add_enabled_ui(self.settings.expand, |ui| {
            ui.label(RichText::new("ZOOM").strong());
            ui.label(format!(
                "{} steps out to see more of the map; again steps back in. {} / {} / {} / {} look around: the view                  stays where you leave it, and glides back to you when you walk.",
                self.settings.keys.key_label(Control::Zoom),
                self.settings.keys.key_label(Control::LookUp),
                self.settings.keys.key_label(Control::LookLeft),
                self.settings.keys.key_label(Control::LookDown),
                self.settings.keys.key_label(Control::LookRight),
            ));
            ui.checkbox(
                &mut self.settings.expand_zoom_on_text,
                "Zoom back in for text boxes and menus (battles always do)",
            );
        });
    }

    fn panels_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("PANELS").strong().color(MUTED));
        ui.label(
            "The extra screen beside the game. Packs of panels add tabs for the games they know.",
        );
        ui.add_space(8.0);
        ui.checkbox(
            &mut self.settings.panel_window,
            "Panels in their own window (drag it to a second screen)",
        );
        ui.checkbox(
            &mut self.settings.dev_panels,
            "Developer panels: MEMORY, to view and watch the game's RAM (from the next game you open)",
        );
        ui.add_space(10.0);
        ui.label(RichText::new("INSTALLED").strong());
        for p in panels::packs() {
            ui.label(format!("  {}", p.name()));
        }
        ui.label(RichText::new("  Developer tools (when turned on above)").color(MUTED));
        ui.add_space(8.0);
        ui.label(
            RichText::new("Writing a panel: rust/PANELS.md")
                .small()
                .color(MUTED),
        );
    }
}

impl eframe::App for AdventurePadApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keys(ctx);
        if self.settings_open {
            self.settings_screen(ctx);
        } else if self.session.is_some() {
            self.game(ctx);
            // The emulator thread wakes the UI for each frame; this keeps the HUD and keys moving meanwhile.
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            self.library(ctx);
        }
        self.pending_dialog(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.leave();
    }
}

/// Credits and licenses. PokeDaisy's additional term (NOTICE): its attribution line shows where credits do.
fn about_page(ui: &mut egui::Ui) {
    ui.label(RichText::new("ADVENTURE PAD").strong().color(MUTED));
    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
    ui.add_space(8.0);
    ui.label(
        RichText::new("Based on PokeDaisy by Lidor Itzhari - https://github.com/lidor30/pokedaisy")
            .strong(),
    );
    ui.hyperlink_to(
        "github.com/lidor30/pokedaisy",
        "https://github.com/lidor30/pokedaisy",
    );
    ui.add_space(8.0);
    ui.label(
        "Free software under the GNU General Public License v3, with PokeDaisy's additional term: the line above          stays in the documentation and the credits of anything built from it. The source comes with the app.",
    );
    ui.add_space(12.0);
    ui.label(RichText::new("BUILT WITH").strong().color(MUTED));
    for line in [
        "mGBA by Vicki Pfau (endrift) and contributors, which runs the games (MPL 2.0), with its blip_buf (LGPL 2.1) and inih (BSD)",
        "egui and eframe by Emil Ernerfeldt and contributors (MIT / Apache 2.0)",
        "cpal (Apache 2.0), rfd (MIT), serde and serde_json (MIT / Apache 2.0), png (MIT / Apache 2.0)",
        "The pret decompilation projects, which the game data was checked against",
    ] {
        ui.label(format!("- {line}"));
    }
    ui.add_space(12.0);
    ui.label(
        RichText::new(
            "A fan project, not affiliated with or endorsed by Nintendo, Game Freak, Creatures or The Pokémon              Company. Pokémon and all related names are trademarks of their owners; Game Boy, Game Boy Color and              Game Boy Advance are trademarks of Nintendo. No games, BIOS or game art are included: use your own              legally dumped ROMs.",
        )
        .small()
        .color(MUTED),
    );
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn modified(p: &Path) -> String {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%b %-d, %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
