//! One game being played: the core on its own thread, paced to the GBA's frame rate, with save states,
//! the synced-save marker and the memory copies the panels read. The UI talks to it through [`Session`].

use crate::audio::{FastSound, Ring};
use crate::music::{self, MusicMute, SteadyMusic};
use crate::saves::{self, PlayMarker};
use adventurepad_sdk::{GameInfo, Memory, Platform, Region};
use mgba_sys::Core;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const SLOTS: u8 = 10;
/// The panels get a fresh copy of memory this often (frames): 4 times a second.
const SAMPLE_FRAMES: u64 = 15;
/// While fast-forwarding, the screen shows every Nth frame.
const FF_DRAW_EVERY: u64 = 2;

/// The last finished frame, as RGBA.
#[derive(Default, Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// EXPAND drew it: where the window goes on the view (else None: the plain frame, fitted).
    pub window: Option<crate::wide::Window>,
    /// New for every frame published.
    pub seq: u64,
}

/// What the UI reads, written by the emulator thread.
#[derive(Default)]
pub struct Shared {
    pub frame: Mutex<Frame>,
    /// The newest copy of the game's memory, for the panels.
    pub memory: Mutex<Option<Arc<Memory>>>,
    pub notices: Mutex<Vec<String>>,
    /// The GBA keys held (mgba_sys::key bits).
    pub keys: AtomicU32,
    pub ff_held: AtomicBool,
    pub ff_on: AtomicBool,
    /// Fast-forward's speed as f32 bits (2.0 = twice as fast; 0 = as fast as it goes). Live: SETTINGS sets it.
    pub ff_speed: AtomicU32,
    /// Fast-forward plays the game's sound at its normal pitch ([`FastSound`]); off = silent.
    pub ff_sound: AtomicBool,
    /// Fast-forward's effects at their own pitch (Wsola) rather than sped up (squeeze).
    pub ff_effects_pitch: AtomicBool,
    /// The game stands still (SETTINGS is open): no frames, no sound.
    pub paused: AtomicBool,
    /// The music core is up: fast-forward plays the song at its normal speed (else in slices, FastSound).
    pub steady_music: AtomicBool,
    /// SETTINGS > SCREEN: EXPAND on, and ZOOM's close-ups for text boxes and menus.
    pub expand: AtomicBool,
    pub zoom_on_text: AtomicBool,
    /// SETTINGS > SCREEN: the area-name banner in the view's top-left corner, not the game frame's.
    pub banner_corner: AtomicBool,
    /// Screen pixels at the view's top that the bar of buttons covers (the UI sets it): the banner goes under it.
    pub top_clear: AtomicU32,
    /// EXPAND has tables for this game and draws it (set by the thread).
    pub expanding: AtomicBool,
    /// The HUD's cue (crate::hud::FIELD_*): the overworld, settled, as EXPAND's scene says; unknown without EXPAND.
    pub field: AtomicU8,
    /// The battle buttons (battle_input.rs): this game has them; the battle's menu (battle_input::NONE...); a pick
    /// on its way; whether a switch can be driven here.
    pub battle_known: AtomicBool,
    pub battle_state: AtomicU8,
    pub battle_busy: AtomicBool,
    pub battle_can_switch: AtomicBool,
    /// The game view's size in screen pixels (the UI sets it each frame), for ZOOM's steps.
    pub view_w: AtomicU32,
    pub view_h: AtomicU32,
    /// The look-around, -1..1 per axis as f32 bits (right / down positive).
    pub look_x: AtomicU32,
    pub look_y: AtomicU32,
    /// Set by the thread when it's done (closed, or the game didn't load).
    pub ended: AtomicBool,
    pub error: Mutex<Option<String>>,
}

impl Shared {
    pub fn notice(&self, s: impl Into<String>) {
        self.notices.lock().unwrap().push(s.into());
    }
}

enum Cmd {
    SaveState(u8),
    LoadState(u8),
    /// Buttons held on top of the player's for this many frames (a panel's Action::Press).
    Press {
        buttons: u32,
        frames: u32,
    },
    Reset,
    /// ZOOM: the next step out (back to 1x after the last).
    Zoom,
    /// A battle button: pressed into the game's battle menu (battle_input.rs).
    Battle(crate::battle_input::Pick),
    /// Tests: a panic in the emulator thread.
    #[cfg(test)]
    TestPanic,
    Quit,
}

/// A state slot on disk.
pub struct SlotInfo {
    pub slot: u8,
    pub thumb: PathBuf,
    pub saved: Option<std::time::SystemTime>,
}

pub struct Session {
    pub info: GameInfo,
    pub shared: Arc<Shared>,
    states_dir: PathBuf,
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

/// The game in `rom`, read from the file.
pub fn game_info(rom: &Path) -> std::io::Result<GameInfo> {
    let bytes: Arc<[u8]> = std::fs::read(rom)?.into();
    let name = rom.file_stem().unwrap_or_default().to_string_lossy();
    Ok(GameInfo::from_rom(&name, bytes))
}

impl Session {
    /// Starts `rom` with its save at `save`, resuming where it was left off here (unless the save changed).
    /// `wake` runs (on the emulator thread) whenever there's a new frame or notice to show.
    pub fn start(
        rom: PathBuf,
        info: GameInfo,
        save: PathBuf,
        marker: PlayMarker,
        ring: Ring,
        wake: impl Fn() + Send + 'static,
    ) -> Session {
        // Save states by the ROM's CRC32, like the app's SaveStates: a renamed ROM keeps its states.
        let states_dir = crate::settings::home()
            .join("states")
            .join(format!("{:08x}", info.crc32));
        let _ = std::fs::create_dir_all(&states_dir);
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();
        let thread = {
            let job = Job {
                rom,
                rom_bytes: info.rom.clone(),
                platform: info.platform,
                crc: info.crc32,
                battle: crate::battle_input::Addrs::for_game(&info.code, info.revision),
                save,
                states_dir: states_dir.clone(),
                shared: shared.clone(),
                marker,
                ring,
                wake: Box::new(wake),
            };
            // A panic in the emulator thread ends the game with a message (in crash.log, with where it was) instead
            // of leaving the window waiting on a thread that's gone. The in-game saves are already in the file.
            let on_panic = shared.clone();
            std::thread::Builder::new()
                .name("adventurepad-emu".into())
                .spawn(move || {
                    if let Err(e) = crate::crash::caught(std::panic::AssertUnwindSafe(|| job.run(rx))) {
                        let msg = crate::crash::message(e.as_ref());
                        *on_panic.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(format!(
                            "The game stopped because of a problem ({msg}). Your in-game saves are safe; the \
                             details are in crash.log in the app's folder."
                        ));
                        on_panic.ended.store(true, Ordering::SeqCst);
                    }
                })
                .expect("emulator thread")
        };
        Session {
            info,
            shared,
            states_dir,
            tx,
            thread: Some(thread),
        }
    }

    pub fn save_state(&self, slot: u8) {
        let _ = self.tx.send(Cmd::SaveState(slot));
    }

    pub fn load_state(&self, slot: u8) {
        let _ = self.tx.send(Cmd::LoadState(slot));
    }

    pub fn press(&self, buttons: u32, frames: u32) {
        let _ = self.tx.send(Cmd::Press { buttons, frames });
    }

    pub fn reset(&self) {
        let _ = self.tx.send(Cmd::Reset);
    }

    #[cfg(test)]
    fn test_panic(&self) {
        let _ = self.tx.send(Cmd::TestPanic);
    }

    /// ZOOM: the next step out.
    pub fn zoom(&self) {
        let _ = self.tx.send(Cmd::Zoom);
    }

    pub fn battle(&self, pick: crate::battle_input::Pick) {
        let _ = self.tx.send(Cmd::Battle(pick));
    }

    /// The settings the running game follows: fast-forward's speed and sound.
    pub fn apply(&self, settings: &crate::settings::Settings) {
        self.shared
            .ff_speed
            .store(settings.ff_speed.to_bits(), Ordering::Relaxed);
        self.shared
            .ff_sound
            .store(settings.ff_sound, Ordering::Relaxed);
        self.shared
            .ff_effects_pitch
            .store(settings.ff_effects_pitch, Ordering::Relaxed);
        self.shared.expand.store(settings.expand, Ordering::Relaxed);
        self.shared
            .zoom_on_text
            .store(settings.expand_zoom_on_text, Ordering::Relaxed);
        self.shared
            .banner_corner
            .store(settings.expand_banner_corner, Ordering::Relaxed);
    }

    /// Stops the game where it is (SETTINGS open), or lets it go on.
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn slots(&self) -> Vec<SlotInfo> {
        (0..SLOTS)
            .map(|slot| {
                let state = self.states_dir.join(format!("ss{slot}"));
                let saved = std::fs::metadata(&state).and_then(|m| m.modified()).ok();
                SlotInfo {
                    slot,
                    thumb: self.states_dir.join(format!("ss{slot}.png")),
                    saved,
                }
            })
            .collect()
    }

    /// Ends the game: lets a save in progress finish, keeps a resume state, and waits for the thread.
    pub fn stop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

fn write_thumb(core: &Core, path: &Path) {
    let (w, h) = (core.width(), core.height());
    let mut rgba = core.pixels().to_vec();
    for px in rgba.chunks_mut(4) {
        px[3] = 255;
    }
    let Ok(file) = std::fs::File::create(path) else {
        return;
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    if let Ok(mut writer) = enc.write_header() {
        let _ = writer.write_image_data(&rgba);
    }
}

/// The panels' copy of memory: the RAM read off the bus now, the ROM shared (never copied).
fn copy_memory(core: &mut Core, platform: Platform, rom: &Arc<[u8]>, frame: u64) -> Memory {
    let mut ram = |name: &'static str, base: u32, len: usize| {
        let mut bytes = vec![0u8; len];
        core.read(base, &mut bytes);
        Region {
            name,
            base,
            bytes: bytes.into(),
        }
    };
    let regions = match platform {
        Platform::Gba => vec![
            ram("EWRAM", 0x0200_0000, 0x4_0000),
            ram("IWRAM", 0x0300_0000, 0x8000),
            Region {
                name: "ROM",
                base: 0x0800_0000,
                bytes: rom.clone(),
            },
        ],
        Platform::Gb => vec![
            ram("WRAM", 0xC000, 0x2000),
            ram("HRAM", 0xFF80, 0x7F),
            Region {
                name: "ROM",
                base: 0,
                bytes: rom[..rom.len().min(0x8000)].into(),
            },
        ],
    };
    Memory::new(regions, frame)
}

/// Fast-forward's sound effects without touching the player's game: a throwaway copy of it, started from a
/// snapshot of the game (read only) and run in step with it, the same buttons pressed, with its music muted
/// ([`MusicMute`]). Muting changes how long the sound engine takes each frame, which moves the copy's timing
/// (its random numbers drift), so it's re-synced from a fresh snapshot every [`SHADOW_RESYNC`] frames: what it
/// plays stays what the game does.
struct Shadow {
    core: Core,
    mute: MusicMute,
    /// Frames since the last snapshot.
    since_sync: u32,
    /// Re-sync on the next frame (fast-forward just started, a state was loaded, the game was reset).
    stale: bool,
    /// Taking part right now (fast-forward with the effects on).
    active: bool,
    /// The engine's layout checked out on the first snapshot; false = this game's effects aren't played.
    usable: bool,
    checked: bool,
    /// Frames of sound to drop: right after a snapshot, its sound buffer still holds mixed-ahead music.
    quiet: u32,
}

/// Frames between the shadow's re-syncs: 5 s of game time.
const SHADOW_RESYNC: u32 = 300;

impl Shadow {
    fn new(mut core: Core) -> Shadow {
        // A throwaway copy: a fault in it (the fault guard, pd_core.c) only ends fast-forward's effects.
        core.set_guarded(true);
        Shadow {
            core,
            mute: MusicMute::default(),
            since_sync: 0,
            stale: true,
            active: false,
            usable: true,
            checked: false,
            quiet: 0,
        }
    }

    /// One frame, before the game runs its own: re-synced first if due. Its sound (effects and cries), or
    /// None while there's none to use yet.
    fn frame<'a>(&mut self, game: &mut Core, keys: u32, audio: &'a mut [i16]) -> Option<&'a [i16]> {
        if !self.active {
            self.active = true;
            self.stale = true;
        }
        if self.stale || self.since_sync >= SHADOW_RESYNC {
            let state = game.state_bytes()?;
            if !self.core.load_state_bytes(&state) {
                return None;
            }
            self.mute.forget();
            self.since_sync = 0;
            self.stale = false;
            self.quiet = 1;
        }
        // Nothing is written until the engine's layout has been seen to be the one the mute knows.
        if !self.checked {
            match MusicMute::layout_ok(&mut self.core) {
                Some(ok) => {
                    self.checked = true;
                    self.usable = ok;
                    if !ok {
                        eprintln!("fast-forward: this game's sound engine isn't the one the mute knows; music only");
                        return None;
                    }
                }
                None => {
                    // No music to judge by yet: this frame runs unmuted and unheard.
                    self.since_sync += 1;
                    self.core.set_keys(keys);
                    self.core.run_frame();
                    self.core.read_audio(audio);
                    return None;
                }
            }
        }
        self.since_sync += 1;
        self.mute.hold(&mut self.core);
        self.core.set_keys(keys);
        self.core.run_frame();
        let n = self.core.read_audio(audio);
        if self.quiet > 0 {
            self.quiet -= 1;
            return None;
        }
        Some(&audio[..n])
    }
}

struct Job {
    rom: PathBuf,
    rom_bytes: Arc<[u8]>,
    platform: Platform,
    crc: u32,
    /// Where the battle buttons read the game, if it's one they know.
    battle: Option<crate::battle_input::Addrs>,
    save: PathBuf,
    states_dir: PathBuf,
    shared: Arc<Shared>,
    marker: PlayMarker,
    ring: Ring,
    wake: Box<dyn Fn() + Send>,
}

impl Job {
    fn run(self, rx: Receiver<Cmd>) {
        let Job {
            rom,
            rom_bytes,
            platform,
            crc,
            battle: battle_addrs,
            save,
            states_dir,
            shared,
            marker,
            ring,
            wake,
        } = self;
        // The save as it was before this session could touch it.
        saves::backup(&save);
        let mut core = match Core::open(&rom, Some(&save)) {
            Ok(c) => c,
            Err(e) => {
                *shared.error.lock().unwrap() = Some(e.to_string());
                shared.ended.store(true, Ordering::SeqCst);
                wake();
                return;
            }
        };
        marker.touch();
        let mut last_beat = Instant::now();

        // Fast-forward's music at its normal speed (music.rs): a second core, booted off this thread.
        // And the shadow core: a throwaway copy of the game for fast-forward's sound effects (Shadow).
        let (music_tx, music_rx) = mpsc::channel::<Option<(SteadyMusic, Option<Core>)>>();
        if platform == Platform::Gba {
            let (rom, bytes) = (rom.clone(), rom_bytes.clone());
            let _ = std::thread::Builder::new()
                .name("adventurepad-music".into())
                .spawn(move || {
                    let music = SteadyMusic::boot(&rom, &bytes);
                    let shadow = music.as_ref().and_then(|_| Core::open(&rom, None).ok());
                    let _ = music_tx.send(music.map(|m| (m, shadow)));
                });
        }
        let mut steady: Option<SteadyMusic> = None;
        let mut music_next = Instant::now();
        // While fast-forwarding with the music core, the sound effects and cries come from the shadow core,
        // gathered here until the next 1x music frame mixes them in.
        let mut shadow: Option<Shadow> = None;
        // EXPAND (pk_wide.c through the bridge): the games it has tables for; the camera (ZOOM, the look-around).
        let wide_known = platform == Platform::Gba && Core::wide_knows(crc);
        let mut wide_on = false;
        let mut camera = crate::wide::Camera::new();
        let mut camera_at = Instant::now();
        let mut window: Option<crate::wide::Window> = None;
        let mut wide_fault_told = false;
        // Frames of overworld in a row (EXPAND's scene), for the HUD.
        let mut field_frames = 0u32;
        let mut effects: Vec<i16> = Vec::new();
        let mut wsola = crate::wsola::Wsola::default();
        // The battle buttons' presses (battle_input.rs).
        let mut battle = battle_addrs.map(crate::battle_input::BattleDriver::new);
        if let Some(b) = &battle {
            shared.battle_known.store(true, Ordering::Relaxed);
            shared
                .battle_can_switch
                .store(b.addrs().can_switch(), Ordering::Relaxed);
        }

        // Resume, unless the save changed since the state was made (another device wrote it: the save wins).
        let resume = states_dir.join("resume");
        if resume.is_file() {
            match core.state_matches_save(&resume) {
                Some(false) => {
                    let _ = std::fs::rename(&resume, states_dir.join("resume.replaced"));
                    shared.notice("Your save changed since you last played here, so the game started from the save");
                }
                _ => {
                    if !core.load_state(&resume) {
                        shared.notice("Couldn't resume; started from the save");
                    }
                    let _ = std::fs::remove_file(&resume);
                }
            }
        }

        let frame_time = Duration::from_secs_f64(1.0 / mgba_sys::FPS);
        let mut next = Instant::now();
        let mut audio = vec![0i16; 8192];
        let mut frames: u64 = 0;
        let mut seq: u64 = 0;
        // A panel's held buttons and the frames left to hold them.
        let mut pressed: u32 = 0;
        let mut press_left: u32 = 0;
        let mut fast = FastSound::default();
        let mut was_ff = false;
        // ADVENTUREPAD_DEBUG=1: the frame rate and the sound's buffer on stderr every 300 frames.
        let debug = std::env::var_os("ADVENTUREPAD_DEBUG").is_some();
        let mut fps_since = Instant::now();
        loop {
            match rx.try_recv() {
                Ok(Cmd::Quit) | Err(mpsc::TryRecvError::Disconnected) => break,
                Ok(Cmd::SaveState(slot)) => {
                    let ok = slot < SLOTS && core.save_state(&states_dir.join(format!("ss{slot}")));
                    if ok {
                        write_thumb(&core, &states_dir.join(format!("ss{slot}.png")));
                    }
                    shared.notice(if ok {
                        format!("Saved slot {slot}")
                    } else {
                        format!("Slot {slot}: save failed")
                    });
                }
                Ok(Cmd::LoadState(slot)) => {
                    camera.reset();
                    if let Some(sh) = &mut shadow {
                        sh.stale = true;
                    }
                    let ok = slot < SLOTS && core.load_state(&states_dir.join(format!("ss{slot}")));
                    ring.clear();
                    shared.notice(if ok {
                        format!("Loaded slot {slot}")
                    } else {
                        format!("Slot {slot}: nothing to load")
                    });
                }
                Ok(Cmd::Press { buttons, frames }) => {
                    pressed = buttons & 0x3FF;
                    press_left = frames.min(600);
                }
                #[cfg(test)]
                Ok(Cmd::TestPanic) => panic!("a test panic in the emulator thread"),
                Ok(Cmd::Battle(pick)) => {
                    if let Some(b) = &mut battle {
                        b.pick(pick);
                    }
                }
                Ok(Cmd::Zoom) => {
                    if wide_on {
                        let (vw, vh) = (
                            shared.view_w.load(Ordering::Relaxed),
                            shared.view_h.load(Ordering::Relaxed),
                        );
                        let more = camera.cycle(vw as f32, vh as f32);
                        shared.notice(if more > 1.01 {
                            format!("ZOOM {more:.2}x")
                        } else {
                            "ZOOM 1x".to_owned()
                        });
                    } else {
                        shared
                            .notice("ZOOM needs EXPAND on (SETTINGS > SCREEN), in a game it knows");
                    }
                }
                Ok(Cmd::Reset) => {
                    // A soft reset the way the games take it: A + B + START + SELECT.
                    core.set_keys(
                        mgba_sys::key::A
                            | mgba_sys::key::B
                            | mgba_sys::key::START
                            | mgba_sys::key::SELECT,
                    );
                    for _ in 0..10 {
                        core.run_frame();
                    }
                    if let Some(sh) = &mut shadow {
                        sh.stale = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            if shared.paused.load(Ordering::Relaxed) {
                // SETTINGS is open: stand still (commands above still run, Quit included).
                ring.clear();
                fast.reset();
                std::thread::sleep(Duration::from_millis(15));
                next = Instant::now();
                continue;
            }

            let ff = shared.ff_held.load(Ordering::Relaxed) || shared.ff_on.load(Ordering::Relaxed);
            let ff_speed = f32::from_bits(shared.ff_speed.load(Ordering::Relaxed));
            let mut keys = shared.keys.load(Ordering::Relaxed);
            if press_left > 0 {
                keys |= pressed;
                press_left -= 1;
            }
            // A battle button's presses: in place of the player's while a pick steers.
            if let Some(b) = &mut battle {
                let (bits, exclusive) = b.tick(&mut core);
                keys = if exclusive { bits } else { keys | bits };
                shared.battle_state.store(b.state(), Ordering::Relaxed);
                shared.battle_busy.store(b.busy(), Ordering::Relaxed);
            }
            core.set_keys(keys);
            // Fast-forward's effects: the shadow runs this frame from where the game is, with the same
            // buttons, its music muted. The game itself is only read (a snapshot when the shadow re-syncs).
            let want_effects = ff && steady.is_some() && shared.ff_sound.load(Ordering::Relaxed);
            match &mut shadow {
                Some(sh) if want_effects && sh.usable => {
                    if let Some(fx) = sh.frame(&mut core, keys, &mut audio) {
                        effects.extend_from_slice(fx);
                    }
                }
                Some(sh) if !want_effects && sh.active => {
                    sh.active = false;
                    effects.clear();
                    wsola.reset();
                }
                _ => {}
            }
            // EXPAND: on or off as SETTINGS has it, the camera's window for this frame, drawn only when it's shown.
            let want_wide = wide_known && shared.expand.load(Ordering::Relaxed);
            if want_wide != wide_on {
                wide_on = core.wide_configure(if want_wide { crc } else { 0 });
                shared.expanding.store(wide_on, Ordering::Relaxed);
                camera.reset();
                window = None;
            }
            if wide_on {
                use mgba_sys::key::{DOWN, LEFT, RIGHT, UP};
                camera.walking = keys & (UP | DOWN | LEFT | RIGHT) != 0;
                camera.look_x = f32::from_bits(shared.look_x.load(Ordering::Relaxed));
                camera.look_y = f32::from_bits(shared.look_y.load(Ordering::Relaxed));
                camera.zoom_on_text = shared.zoom_on_text.load(Ordering::Relaxed);
                let now = Instant::now();
                let dt = now.duration_since(camera_at).as_secs_f32();
                camera_at = now;
                let (vw, vh) = (
                    shared.view_w.load(Ordering::Relaxed),
                    shared.view_h.load(Ordering::Relaxed),
                );
                window = camera.frame(vw as f32, vh as f32, core.wide_scene(), dt);
                match window {
                    Some(w) => core.wide_set_view(w.x, w.y, w.w, w.h),
                    None => core.wide_set_view(0, 0, 0, 0),
                }
                let banner = window
                    .filter(|_| shared.banner_corner.load(Ordering::Relaxed))
                    .map(|w| w.banner_at(shared.top_clear.load(Ordering::Relaxed) as f32));
                core.wide_set_banner(banner);
                core.wide_set_draw(!ff || (frames + 1).is_multiple_of(FF_DRAW_EVERY));
            }
            core.run_frame();
            frames += 1;
            // The HUD shows on the overworld once it has settled (hud.rs); EXPAND's scene says what's on screen.
            let field = if wide_on && window.is_some() {
                field_frames = if core.wide_scene() == crate::wide::SCENE_FIELD {
                    field_frames.saturating_add(1)
                } else {
                    0
                };
                if field_frames >= crate::hud::FIELD_SETTLE_FRAMES {
                    crate::hud::FIELD_YES
                } else {
                    crate::hud::FIELD_NO
                }
            } else {
                field_frames = 0;
                crate::hud::FIELD_UNKNOWN
            };
            shared.field.store(field, Ordering::Relaxed);
            if wide_on && !wide_fault_told && core.wide_fault() != 0 {
                // The fault guard caught it (pd_core.c): EXPAND is off for this game, which plays on plain.
                wide_fault_told = true;
                eprintln!(
                    "EXPAND faulted (0x{:08x}) and switched itself off",
                    core.wide_fault()
                );
                shared.notice("EXPAND hit a problem and is off for this game");
            }

            let n = core.read_audio(&mut audio);
            if steady.is_none() {
                if let Ok(m) = music_rx.try_recv() {
                    shared.steady_music.store(m.is_some(), Ordering::Relaxed);
                    if let Some((music, sh)) = m {
                        steady = Some(music);
                        shadow = sh.map(Shadow::new);
                    }
                }
            }
            // A helper core the fault guard caught (pd_core.c): its feature goes, the game plays on.
            if let Some(code) = steady.as_ref().and_then(|m| m.dead()) {
                eprintln!("the music core faulted (0x{code:08x}): fast-forward plays the game's own sound");
                steady = None;
                shadow = None;
                shared.steady_music.store(false, Ordering::Relaxed);
                shared.notice(
                    "Fast-forward's music hit a problem: it plays the game's own sound now",
                );
                effects.clear();
            }
            if let Some(code) = shadow
                .as_ref()
                .filter(|s| s.usable)
                .and_then(|s| s.core.dead())
            {
                eprintln!(
                    "the shadow core faulted (0x{code:08x}): fast-forward plays the music alone"
                );
                if let Some(s) = &mut shadow {
                    s.usable = false;
                }
                shared.notice(
                    "Fast-forward's sound effects hit a problem: the music plays alone now",
                );
                effects.clear();
            }
            let sound = shared.ff_sound.load(Ordering::Relaxed);
            let live_song = if steady.is_some() {
                music::current_song(&mut core)
            } else {
                None
            };
            match (&mut steady, ff) {
                // Playing: the live game's own sound; the music core keeps in step at 1x, unheard.
                (Some(m), false) => {
                    m.follow(live_song);
                    m.run_frame();
                    ring.push(&audio[..n]);
                    music_next = Instant::now() + frame_time;
                }
                // Fast-forward: the music core's song at its normal speed, on a real-time clock of its own.
                (Some(m), true) => {
                    let now = Instant::now();
                    if now.saturating_duration_since(music_next) > Duration::from_millis(100) {
                        music_next = now;
                    }
                    if sound {
                        // The shadow's effects (above); at most a second of them waits (uncapped
                        // fast-forward, a stall).
                        let over = effects
                            .len()
                            .saturating_sub(mgba_sys::SAMPLE_RATE as usize * 2);
                        effects.drain(..over - over % 2);
                    }
                    while music_next <= now {
                        m.follow(live_song);
                        let out = m.run_frame();
                        if sound {
                            // Everything the game played since the last music frame, into its length: at its
                            // own pitch (Wsola), or sped up.
                            let fx = if shared.ff_effects_pitch.load(Ordering::Relaxed) {
                                wsola.process(&effects, out.len() / 2)
                            } else {
                                crate::audio::squeeze(&effects, out.len() / 2)
                            };
                            effects.clear();
                            ring.push(&crate::audio::mix(out, &fx));
                        }
                        // The queued sound nudges it, like the game's own pacing below.
                        let queued = ring.queued();
                        let target = mgba_sys::SAMPLE_RATE as usize / 15;
                        music_next += if queued > target * 2 {
                            frame_time + Duration::from_micros(500)
                        } else if queued < target / 2 {
                            frame_time.saturating_sub(Duration::from_micros(500))
                        } else {
                            frame_time
                        };
                    }
                }
                // No music core (a Game Boy game, an unknown sound engine, still booting): the game's own
                // sound, in normal-pitch slices while fast-forwarding (FastSound).
                (None, false) => {
                    if was_ff {
                        fast.reset();
                    }
                    ring.push(&audio[..n]);
                }
                (None, true) => {
                    if sound {
                        fast.feed(&audio[..n], &ring);
                    }
                }
            }
            was_ff = ff;

            // A battle button's switch keeps the battle's last frame up while it drives the game's party menu.
            let hold = battle.as_ref().is_some_and(|b| b.hold_frame());
            if !hold && (!ff || frames.is_multiple_of(FF_DRAW_EVERY)) {
                let mut f = shared.frame.lock().unwrap();
                // EXPAND's window when it drew one, else the game's own frame.
                let wide = if wide_on {
                    core.wide_pixels().zip(window)
                } else {
                    None
                };
                let (px, w, h) = match wide {
                    Some(((px, w, h), _)) => (px, w, h),
                    None => (core.pixels(), core.width() as usize, core.height() as usize),
                };
                f.window = wide.map(|(_, win)| win);
                f.width = w;
                f.height = h;
                f.rgba.resize(px.len(), 0);
                for (o, i) in f.rgba.chunks_mut(4).zip(px.chunks(4)) {
                    o[..3].copy_from_slice(&i[..3]);
                    o[3] = 255;
                }
                seq += 1;
                f.seq = seq;
                drop(f);
                wake();
            }

            if frames.is_multiple_of(SAMPLE_FRAMES) {
                let m = copy_memory(&mut core, platform, &rom_bytes, frames);
                *shared.memory.lock().unwrap() = Some(Arc::new(m));
            }

            if debug && frames.is_multiple_of(300) {
                let el = fps_since.elapsed().as_secs_f64();
                eprintln!(
                    "fps {:.1}  ff {ff}  audio queued {} frames",
                    300.0 / el,
                    ring.queued()
                );
                fps_since = Instant::now();
            }

            if last_beat.elapsed() >= saves::HEARTBEAT {
                marker.touch();
                last_beat = Instant::now();
            }

            // Pacing: the frame rate, sped up while fast-forwarding; the queued sound nudges it so the
            // device's clock and ours don't drift apart (more queued: a little slower).
            let mut dt = frame_time;
            if ff {
                dt = if ff_speed > 0.0 {
                    frame_time.div_f32(ff_speed)
                } else {
                    Duration::ZERO
                };
            } else {
                let queued = ring.queued();
                let target = mgba_sys::SAMPLE_RATE as usize / 15; // ~66 ms
                if queued > target * 2 {
                    dt += Duration::from_micros(500);
                } else if queued < target / 2 {
                    dt = dt.saturating_sub(Duration::from_micros(500));
                }
            }
            next += dt;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > Duration::from_millis(100) {
                next = now; // fell behind (a hitch): don't race to catch up
            }
        }

        // Let a save the game is writing finish, then keep where the player is for next time.
        core.finish_save(120);
        core.save_state(&resume);
        drop(core);
        marker.clear();
        shared.ended.store(true, Ordering::SeqCst);
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adventurepad_sdk::PanelPack;

    /// EXPAND is one game's at a time (pk_wide.c's state is global, and pd_core.c hands it to one core): the
    /// tests whose games use it take turns.
    static ONE_GAME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn one_game() -> std::sync::MutexGuard<'static, ()> {
        ONE_GAME.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A second core asking for EXPAND while one has it is refused, and gets it once the first lets go.
    #[test]
    #[ignore]
    fn expand_is_one_games_at_a_time() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let info = game_info(&rom).unwrap();
        if !Core::wide_knows(info.crc32) {
            return;
        }
        let mut a = Core::open(&rom, None).unwrap();
        let mut b = Core::open(&rom, None).unwrap();
        assert!(a.wide_configure(info.crc32));
        assert!(
            !b.wide_configure(info.crc32),
            "refused while the first has it"
        );
        drop(a);
        assert!(b.wide_configure(info.crc32), "free once the first is gone");
        assert!(!b.wide_configure(0));
    }

    /// A fault in EXPAND (caught by the guard) switches it off for this game, and the game plays on, plain.
    /// `ADVENTUREPAD_TEST_ROM=<a ROM EXPAND knows> cargo test -p adventurepad -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn a_fault_in_expand_switches_it_off_and_the_game_goes_on() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let info = game_info(&rom).unwrap();
        if !Core::wide_knows(info.crc32) {
            return;
        }
        let mut core = Core::open(&rom, None).unwrap();
        assert!(core.wide_configure(info.crc32));
        core.wide_set_view(-60, 0, 360, 160);
        for _ in 0..60 {
            core.run_frame();
        }
        assert!(core.wide_pixels().is_some() && core.wide_fault() == 0);
        core.wide_test_fault();
        core.run_frame();
        assert_eq!(core.wide_fault(), 0xC000_0005, "the guard caught it");
        assert!(
            core.wide_pixels().is_none(),
            "EXPAND is off: the plain frame"
        );
        let before = core.pixels().to_vec();
        for _ in 0..120 {
            core.run_frame();
        }
        assert_ne!(core.pixels(), &before[..], "and the game goes on");
        assert!(core.wide_pixels().is_none(), "EXPAND stays off");
    }

    /// A panic in the emulator thread ends the game with a message (and the window can go back to the library)
    /// instead of leaving it waiting on a thread that's gone. `ADVENTUREPAD_TEST_ROM=<rom> ... -- --ignored`.
    #[test]
    #[ignore]
    fn a_panic_in_the_emulator_ends_the_game_cleanly() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("adventurepad-panic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let info = game_info(&rom).unwrap();
        let save = dir.join("test.sav");
        let marker = PlayMarker::new(&save, "test", "test-id");
        let s = Session::start(rom, info, save, marker, Ring::default(), || {});
        std::thread::sleep(Duration::from_millis(500));
        assert!(!s.shared.ended.load(Ordering::SeqCst));
        s.test_panic();
        std::thread::sleep(Duration::from_millis(300));
        assert!(s.shared.ended.load(Ordering::SeqCst), "the game ended");
        let err = s
            .shared
            .error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .unwrap_or_default();
        assert!(
            err.contains("The game stopped") && err.contains("a test panic"),
            "{err}"
        );
        drop(s); // the thread is gone: stopping it mustn't hang or panic
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// EXPAND only reads the game: two copies from the same start, the same buttons, one drawing EXPAND's window
    /// every frame (the camera moving it about). Its window is wider than the game, with the game's own frame
    /// in the middle; and afterwards all of their RAM is identical. `ADVENTUREPAD_TEST_SAVE=<sav>` (copied) gets
    /// to the overworld, where it draws the map. `ADVENTUREPAD_TEST_ROM=<rom> ... -- --ignored`.
    #[test]
    #[ignore]
    fn expand_draws_wide_and_leaves_the_game_alone() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let info = game_info(&rom).unwrap();
        if !Core::wide_knows(info.crc32) {
            eprintln!("EXPAND has no tables for this ROM: nothing to check");
            return;
        }
        let dir = std::env::temp_dir().join(format!("adventurepad-expand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saves: Vec<Option<PathBuf>> = (0..2)
            .map(|i| {
                std::env::var_os("ADVENTUREPAD_TEST_SAVE")
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let to = dir.join(format!("copy{i}.sav"));
                        std::fs::copy(s, &to).unwrap();
                        to
                    })
            })
            .collect();
        let mut plain = Core::open(&rom, saves[0].as_deref()).unwrap();
        let mut wide = Core::open(&rom, saves[1].as_deref()).unwrap();
        assert!(wide.wide_configure(info.crc32));
        let mut camera = crate::wide::Camera::new();
        let mut audio = vec![0i16; 8192];
        let (mut drawn, mut field) = (0usize, 0usize);
        for f in 0u32..6000 {
            let keys = match (f / 30) % 8 {
                _ if f % 30 >= 4 => 0,
                0 | 4 => mgba_sys::key::A,
                1 => mgba_sys::key::START,
                2 => mgba_sys::key::DOWN,
                3 => mgba_sys::key::LEFT,
                5 => mgba_sys::key::UP,
                6 => mgba_sys::key::RIGHT,
                _ => mgba_sys::key::B,
            };
            // The camera on a 1080p view, zooming out now and then, looking about.
            if f % 900 == 450 {
                camera.cycle(1920.0, 1080.0);
            }
            camera.look_x = if (f / 200) % 3 == 1 { 1.0 } else { 0.0 };
            camera.walking = keys & 0xF0 != 0;
            let win = camera
                .frame(1920.0, 1080.0, wide.wide_scene(), 1.0 / 60.0)
                .unwrap();
            wide.wide_set_view(win.x, win.y, win.w, win.h);
            wide.wide_set_draw(f % 3 != 0); // fast-forward's unshown frames too
            for c in [&mut plain, &mut wide] {
                c.set_keys(keys);
                c.run_frame();
                c.read_audio(&mut audio);
            }
            if wide.wide_scene() == crate::wide::SCENE_FIELD {
                field += 1;
            }
            if let Some((px, w, h)) = wide.wide_pixels() {
                drawn += 1;
                assert!(
                    w > 240 && w as i32 == win.w && h as i32 == win.h,
                    "{w}x{h} for {win:?}"
                );
                // The game's own frame sits where the window says, pixel for pixel (when it covers it all).
                if win.x <= 0 && win.y <= 0 && win.x + win.w >= 240 && win.y + win.h >= 160 {
                    let (ox, oy) = ((-win.x) as usize, (-win.y) as usize);
                    let frame = wide.pixels();
                    let same = (0..160)
                        .flat_map(|y| (0..240).map(move |x| (x, y)))
                        .filter(|&(x, y)| {
                            px[((oy + y) * w + ox + x) * 4..][..3]
                                == frame[(y * 240 + x) * 4..][..3]
                        })
                        .count();
                    assert_eq!(same, 240 * 160, "the middle is the game's frame");
                }
            }
        }
        eprintln!("{drawn} frames drawn wide, {field} of them on the overworld");
        assert!(drawn > 3000, "EXPAND drew");
        assert_eq!(wide.wide_fault(), 0);
        let mut diffs = Vec::new();
        for (base, len) in [(0x0200_0000u32, 0x4_0000usize), (0x0300_0000, 0x8000)] {
            let (mut a, mut b) = (vec![0u8; len], vec![0u8; len]);
            plain.read(base, &mut a);
            wide.read(base, &mut b);
            diffs.extend((0..len).filter(|&i| a[i] != b[i]).map(|i| base + i as u32));
        }
        assert!(
            diffs.is_empty(),
            "EXPAND changed the game at {:08x?}",
            &diffs[..diffs.len().min(32)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// EXPAND moves the area-name banner from the game frame's corner (towards the middle of a wide view) to the
    /// window's: a game played about from a save or state on the overworld; whenever the banner is up on the frame,
    /// none of its pixels are left where the frame has it and all of them are at the corner. The game is only read.
    /// `ADVENTUREPAD_TEST_SHOTS=<dir>` saves a frame of it. `ADVENTUREPAD_TEST_ROM=<rom>`, `_SAVE` / `_STATE`
    /// (copied) `... -- --ignored`.
    #[test]
    #[ignore]
    fn expand_moves_the_area_banner_to_the_corner() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let info = game_info(&rom).unwrap();
        if !Core::wide_knows(info.crc32) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("adventurepad-banner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = |var: &str, name: &str| {
            std::env::var_os(var).filter(|s| !s.is_empty()).map(|s| {
                let to = dir.join(name);
                std::fs::copy(s, &to).unwrap();
                to
            })
        };
        let save = copy("ADVENTUREPAD_TEST_SAVE", "copy.sav");
        let mut core = Core::open(&rom, save.as_deref()).unwrap();
        if let Some(state) = copy("ADVENTUREPAD_TEST_STATE", "copy.state") {
            assert!(core.load_state(&state));
        }
        assert!(core.wide_configure(info.crc32));
        let shots = std::env::var_os("ADVENTUREPAD_TEST_SHOTS").map(PathBuf::from);
        let mut camera = crate::wide::Camera::new();
        let mut audio = vec![0i16; 8192];
        let (mut up, mut shot) = (0, false);
        for f in 0u32..12000 {
            // Through the title (Continue), then walking about: new areas bring the banner.
            let keys = match (f / 20) % 12 {
                _ if f < 1500 => {
                    [0, mgba_sys::key::A, 0, mgba_sys::key::START][(f / 20 % 4) as usize]
                }
                // A now and then: through a battle the state may start in, and text boxes.
                0..=2 => mgba_sys::key::DOWN,
                3..=5 => mgba_sys::key::LEFT,
                6 | 11 => mgba_sys::key::A,
                7 => mgba_sys::key::B,
                _ => mgba_sys::key::UP,
            };
            let win = camera
                .frame(1920.0, 1080.0, core.wide_scene(), 1.0 / 60.0)
                .unwrap();
            core.wide_set_view(win.x, win.y, win.w, win.h);
            let (bx, by) = win.banner_at(44.0);
            core.wide_set_banner(Some((bx, by)));
            core.set_keys(keys);
            core.run_frame();
            core.read_audio(&mut audio);
            let scene = core.wide_scene();
            if scene != crate::wide::SCENE_FIELD && scene != crate::wide::SCENE_TEXT {
                continue;
            }
            let frame = core.pixels();
            let Some(mask) = core.wide_banner() else {
                continue;
            };
            let banner: Vec<(usize, usize)> = (0..mgba_sys::BANNER_H)
                .flat_map(|y| (0..mgba_sys::BANNER_W).map(move |x| (x, y)))
                .filter(|&(x, y)| mask[y * mgba_sys::BANNER_W + x] != 0)
                .collect();
            assert!(banner.len() >= 64, "a banner is most of a window");
            let Some((px, w, h)) = core.wide_pixels() else {
                continue;
            };
            up += 1;
            let at = |i: i32, j: i32| -> Option<&[u8]> {
                (i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h)
                    .then(|| &px[(j as usize * w + i as usize) * 4..][..3])
            };
            for &(x, y) in &banner {
                let p = &frame[(y * 240 + x) * 4..][..3];
                if let Some(o) = at(bx + x as i32, by + y as i32) {
                    assert_eq!(o, p, "the banner's pixel ({x}, {y}) at the corner");
                }
            }
            // Where the frame has it, the map now (skipping any the corner copy lands on).
            let left = banner
                .iter()
                .filter(|&&(x, y)| {
                    let (i, j) = (x as i32 - win.x, y as i32 - win.y);
                    let moved_here = (0..112).contains(&(i - bx)) && (0..40).contains(&(j - by));
                    !moved_here && at(i, j) == Some(&frame[(y * 240 + x) * 4..][..3])
                })
                .count();
            assert!(
                left * 10 < banner.len(),
                "{left} of {} banner pixels left on the frame",
                banner.len()
            );
            if let (Some(dir), false) = (&shots, shot) {
                if up >= 30 {
                    shot = true;
                    let mut rgba = px[..w * h * 4].to_vec();
                    rgba.chunks_mut(4).for_each(|p| p[3] = 255);
                    let file = std::fs::File::create(dir.join("banner.png")).unwrap();
                    let mut enc =
                        png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
                    enc.set_color(png::ColorType::Rgba);
                    enc.set_depth(png::BitDepth::Eight);
                    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
                }
            }
        }
        eprintln!("the banner was up on {up} frames, moved each time");
        assert_eq!(core.wide_fault(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The HUD shows on the overworld and steps aside for menus, text and battles: a game played about with
    /// EXPAND's camera on, the HUD fed the panels' memory copies, its cue worked out as the emulator thread does.
    /// Without EXPAND (the callback it learns) it mostly agrees, and never shows in a battle. A save that gets to
    /// the overworld: `ADVENTUREPAD_TEST_SAVE=<sav>`, or a state there: `ADVENTUREPAD_TEST_STATE=<state>` (both
    /// copied). `ADVENTUREPAD_TEST_ROM=<rom> ... -- --ignored`.
    #[test]
    #[ignore]
    fn the_hud_shows_on_the_overworld_only() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let info = game_info(&rom).unwrap();
        let Some(mut hud) = crate::hud::Hud::for_game(&info) else {
            eprintln!("the companion can't read this ROM: no HUD to check");
            return;
        };
        assert!(Core::wide_knows(info.crc32), "the HUD's games are EXPAND's");
        let dir = std::env::temp_dir().join(format!("adventurepad-hud-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = std::env::var_os("ADVENTUREPAD_TEST_SAVE")
            .filter(|s| !s.is_empty())
            .map(|s| {
                let to = dir.join("copy.sav");
                std::fs::copy(s, &to).unwrap();
                to
            });
        let mut core = Core::open(&rom, save.as_deref()).unwrap();
        // A save state to start from (copied): somewhere further in than a save gets by pressing buttons.
        if let Some(state) = std::env::var_os("ADVENTUREPAD_TEST_STATE").filter(|s| !s.is_empty()) {
            let to = dir.join("copy.state");
            std::fs::copy(state, &to).unwrap();
            assert!(core.load_state(&to), "the state loads");
        }
        assert!(core.wide_configure(info.crc32));
        let mut camera = crate::wide::Camera::new();
        let mut audio = vec![0i16; 8192];
        let mut field_frames = 0u32;
        let mut scenes = [0u32; 8];
        // (EXPAND's cue, the HUD shown with it, shown without it, in a battle) per memory copy.
        let mut seen = Vec::new();
        for f in 0u32..9000 {
            let keys = match (f / 40) % 10 {
                _ if f % 40 >= 6 && (f / 40) % 10 < 5 => 0,
                0 | 5 => mgba_sys::key::A,
                1 => mgba_sys::key::START,
                2 | 6 => mgba_sys::key::DOWN,
                3 | 7 => mgba_sys::key::LEFT,
                4 | 8 => mgba_sys::key::B,
                _ => mgba_sys::key::RIGHT,
            };
            let win = camera.frame(1920.0, 1080.0, core.wide_scene(), 1.0 / 60.0);
            if let Some(w) = win {
                core.wide_set_view(w.x, w.y, w.w, w.h);
            }
            core.set_keys(keys);
            core.run_frame();
            core.read_audio(&mut audio);
            scenes[core.wide_scene().clamp(0, 7) as usize] += 1;
            field_frames = if core.wide_scene() == crate::wide::SCENE_FIELD {
                field_frames + 1
            } else {
                0
            };
            if f % 15 == 0 {
                let cue = if field_frames >= crate::hud::FIELD_SETTLE_FRAMES {
                    crate::hud::FIELD_YES
                } else {
                    crate::hud::FIELD_NO
                };
                let m = copy_memory(&mut core, info.platform, &info.rom, f as u64);
                hud.feed(&m);
                let style = crate::hud::HudStyle::default();
                seen.push((
                    cue,
                    hud.shown_for_test(&style, cue),
                    hud.shown_for_test(&style, crate::hud::FIELD_UNKNOWN),
                    hud.in_battle_for_test(),
                ));
            }
        }
        eprintln!("scenes (field, text, overlay, battle start, off, fade, menu): {scenes:?}");
        let with = seen.iter().filter(|s| s.1).count();
        let field = seen.iter().filter(|s| s.0 == crate::hud::FIELD_YES).count();
        let battles = seen.iter().filter(|s| s.3).count();
        let agree = seen.iter().filter(|s| s.1 && s.2).count();
        let fallback_extra = seen.iter().filter(|s| !s.1 && s.2).count();
        eprintln!(
            "{} copies: {field} on the overworld, HUD shown in {with}; without EXPAND shown in {agree} of those (+{fallback_extra} \
             elsewhere); {battles} in a battle",
            seen.len()
        );
        if field == 0 || !hud.has_party_for_test() {
            eprintln!("never reached the overworld with a party (a save or a state there?): nothing to check");
            return;
        }
        assert!(with > 50, "the HUD came up on the overworld");
        assert!(
            seen.iter().all(|s| !s.1 || s.0 == crate::hud::FIELD_YES),
            "only on the overworld"
        );
        assert!(
            seen.iter().all(|s| !s.3 || (!s.1 && !s.2)),
            "never in a battle"
        );
        assert!(
            agree * 10 >= with * 8,
            "without EXPAND it mostly agrees ({agree} of {with})"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fast-forward's effects never touch the player's game: two copies from the same start, the same buttons,
    /// one with a shadow core taking snapshots of it and playing its effects muted, every frame, re-syncs
    /// and all. Afterwards all of their RAM is identical, the sound engine's included. A save to boot from:
    /// `ADVENTUREPAD_TEST_SAVE=<sav>` (copied: never written). `ADVENTUREPAD_TEST_ROM=<rom> ... -- --ignored`.
    #[test]
    #[ignore]
    fn the_shadow_leaves_the_game_alone() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("adventurepad-shadow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saves: Vec<Option<PathBuf>> = (0..2)
            .map(|i| {
                std::env::var_os("ADVENTUREPAD_TEST_SAVE")
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let to = dir.join(format!("copy{i}.sav"));
                        std::fs::copy(s, &to).unwrap();
                        to
                    })
            })
            .collect();
        let mut plain = Core::open(&rom, saves[0].as_deref()).unwrap();
        let mut live = Core::open(&rom, saves[1].as_deref()).unwrap();
        let mut shadow = Shadow::new(Core::open(&rom, None).unwrap());
        let mut audio = vec![0i16; 8192];
        let mut heard = 0usize;
        for f in 0u32..6000 {
            let keys = match (f / 30) % 8 {
                _ if f % 30 >= 4 => 0,
                0 | 4 => mgba_sys::key::A,
                1 => mgba_sys::key::START,
                2 => mgba_sys::key::DOWN,
                3 => mgba_sys::key::LEFT,
                5 => mgba_sys::key::UP,
                6 => mgba_sys::key::RIGHT,
                _ => mgba_sys::key::B,
            };
            // "Fast-forward" from frame 600 on, toggled off and on again now and then.
            if f > 600 && (f / 500) % 4 != 3 {
                if let Some(fx) = shadow.frame(&mut live, keys, &mut audio) {
                    heard += fx.len();
                }
            } else {
                shadow.active = false;
            }
            for c in [&mut plain, &mut live] {
                c.set_keys(keys);
                c.run_frame();
                c.read_audio(&mut audio);
            }
        }
        assert!(shadow.usable, "the engine layout checks out");
        assert!(heard > 0, "the shadow played");
        let mut diffs = Vec::new();
        for (base, len) in [(0x0200_0000u32, 0x4_0000usize), (0x0300_0000, 0x8000)] {
            let (mut a, mut b) = (vec![0u8; len], vec![0u8; len]);
            plain.read(base, &mut a);
            live.read(base, &mut b);
            diffs.extend((0..len).filter(|&i| a[i] != b[i]).map(|i| base + i as u32));
        }
        assert!(
            diffs.is_empty(),
            "the game differs at {:08x?}",
            &diffs[..diffs.len().min(32)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole path with a real game: the core runs, memory copies arrive, the Pokémon panels read them
    /// and a panel's button press reaches the game. Needs a retail FireRed / Emerald ROM:
    /// `ADVENTUREPAD_TEST_ROM=<rom> cargo test -p adventurepad -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_game_feeds_the_panels() {
        let _turn = one_game();
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(PathBuf::from) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("adventurepad-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("ADVENTUREPAD_HOME", &dir);
        let info = game_info(&rom).unwrap();
        let save = dir.join("test.sav");
        let marker = PlayMarker::new(&save, "test", "test-id");
        let ring = Ring::default();
        let mut s = Session::start(rom, info.clone(), save.clone(), marker, ring.clone(), || {});
        std::thread::sleep(Duration::from_millis(1500));
        let m = s
            .shared
            .memory
            .lock()
            .unwrap()
            .clone()
            .expect("memory copies arrive");
        assert!(m.frame >= 15);
        assert_eq!(m.regions().len(), 3);
        let ewram = &m.regions()[0];
        assert!(
            ewram.bytes.iter().any(|&b| b != 0),
            "EWRAM has the game in it"
        );
        let mut panels = panels_pokemon::PokemonPack.panels_for(&info);
        // Retail FireRed / Emerald get the Pokémon panels (a hack plays without them, so far).
        if info.rom.len() <= 0x100_0000 {
            assert!(!panels.is_empty(), "a Pokémon game gets its panels");
        }
        for p in &mut panels {
            p.refresh(&info, &m);
        }
        s.press(mgba_sys::key::START, 5);
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            s.shared.frame.lock().unwrap().seq > 30,
            "frames keep coming"
        );

        // Fast-forward at 4x with its sound on: about 4x the frames, and the sound kept at real-time level
        // (nothing drains this ring, so FastSound stops adding once it holds its target).
        let frame_now = || {
            s.shared
                .memory
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |m| m.frame)
        };
        s.apply(&crate::settings::Settings {
            ff_speed: 4.0,
            ff_sound: true,
            ..Default::default()
        });
        ring.clear();
        let f0 = frame_now();
        s.shared.ff_on.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(1000));
        let ran = frame_now() - f0;
        assert!(
            s.shared.steady_music.load(Ordering::Relaxed),
            "the music core is up for a Gen 3 game"
        );
        assert!(ran > 180, "fast-forward ran {ran} frames in a second");
        // The music core plays at 1x: ~4800 stereo frames in 100 ms, where the game's own sound at 4x would
        // be ~19000 (the ring caps at 9600).
        ring.clear();
        std::thread::sleep(Duration::from_millis(100));
        let q = ring.queued();
        assert!(
            (2500..8000).contains(&q),
            "fast-forward's music: {q} frames in 100 ms"
        );
        // While it runs, the live game's own music is untouched (the shadow core is the one muted)...
        let volumes = |m: &Memory| -> Vec<u8> {
            let Some(info) = m.u32(0x0300_7FF0) else {
                return vec![];
            };
            let Some(mut p) = m.u32(info + 0x24) else {
                return vec![];
            };
            let mut bgm = 0;
            while p != 0 && m.u32(p + 0x3C).is_some() {
                bgm = p;
                p = m.u32(p + 0x3C).unwrap();
            }
            let (Some(n), Some(tracks)) = (m.u8(bgm + 0x08), m.u32(bgm + 0x2C)) else {
                return vec![];
            };
            (0..n as u32)
                .filter(|i| m.u8(tracks + i * 0x50).unwrap_or(0) & 0x80 != 0)
                .map(|i| m.u8(tracks + i * 0x50 + 0x13).unwrap_or(0))
                .collect()
        };
        let mem = || s.shared.memory.lock().unwrap().clone().unwrap();
        let during = volumes(&mem());
        assert!(
            during.is_empty() || during.iter().any(|&v| v > 0),
            "the game's own music isn't muted: {during:?}"
        );
        s.shared.ff_on.store(false, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(300));
        let after = volumes(&mem());
        assert!(
            after.is_empty() || after.iter().any(|&v| v > 0),
            "and after: {after:?}"
        );

        // Paused (SETTINGS open): the game stands still.
        s.set_paused(true);
        std::thread::sleep(Duration::from_millis(200));
        let a = frame_now();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(frame_now(), a, "paused");
        s.set_paused(false);
        std::thread::sleep(Duration::from_millis(400));
        assert!(frame_now() > a, "and goes on after");
        s.stop();
        assert!(s.shared.ended.load(Ordering::SeqCst));
        assert!(
            !save.with_file_name("test.sav.playing").exists(),
            "the marker is cleared on close"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
