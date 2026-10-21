//! libmgba for Adventure Pad: the flat C API in `csrc/pd_core.h` and [`Core`], a safe owner of one loaded game.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint};
use std::path::Path;

#[repr(C)]
struct PdCore {
    _private: [u8; 0],
}

extern "C" {
    fn pd_core_open(rom: *const c_char, save: *const c_char) -> *mut PdCore;
    fn pd_core_close(c: *mut PdCore);
    fn pd_core_platform(c: *const PdCore) -> c_int;
    fn pd_core_width(c: *const PdCore) -> c_uint;
    fn pd_core_height(c: *const PdCore) -> c_uint;
    fn pd_core_pixels(c: *const PdCore) -> *const u8;
    fn pd_core_set_keys(c: *mut PdCore, mask: u32);
    fn pd_core_run_frame(c: *mut PdCore);
    fn pd_core_read_audio(c: *mut PdCore, out: *mut i16, cap: usize) -> usize;
    fn pd_core_finish_save(c: *mut PdCore, max_frames: c_int) -> c_int;
    fn pd_core_save_state(c: *mut PdCore, path: *const c_char) -> bool;
    fn pd_core_load_state(c: *mut PdCore, path: *const c_char) -> bool;
    fn pd_core_state_matches_save(c: *mut PdCore, path: *const c_char) -> c_int;
    fn pd_core_read(c: *mut PdCore, addr: u32, dst: *mut u8, len: usize) -> bool;
    fn pd_core_call(c: *mut PdCore, fun: u32, args: *const u32, r0: *mut u32) -> bool;
    fn pd_core_park(c: *mut PdCore, spin: u32) -> bool;
    fn pd_core_write(c: *mut PdCore, addr: u32, src: *const u8, len: usize) -> bool;
    fn pd_core_enable_audio_channel(c: *mut PdCore, id: c_int, enable: bool);
    fn pd_core_state_to_memory(c: *mut PdCore, out: *mut *mut u8, len: *mut usize) -> bool;
    fn pd_core_state_from_memory(c: *mut PdCore, data: *const u8, len: usize) -> bool;
    fn pd_core_free(p: *mut std::os::raw::c_void);
    fn pd_core_wide_knows(crc: u32) -> bool;
    fn pd_core_wide_configure(c: *mut PdCore, crc: u32) -> bool;
    fn pd_core_wide_set_view(c: *mut PdCore, x: c_int, y: c_int, w: c_int, h: c_int);
    fn pd_core_wide_set_draw(c: *mut PdCore, draw: bool);
    fn pd_core_wide_pixels(c: *const PdCore, w: *mut c_int, h: *mut c_int) -> *const u8;
    fn pd_core_wide_scene(c: *const PdCore) -> c_int;
    fn pd_core_wide_fault(c: *const PdCore) -> std::os::raw::c_ulong;
    fn pd_core_set_guarded(c: *mut PdCore, guarded: bool);
    fn pd_core_dead(c: *const PdCore) -> std::os::raw::c_ulong;
    fn pd_core_test_fault(c: *mut PdCore);
    fn pd_core_wide_test_fault(c: *mut PdCore);
    fn pd_core_guard_selftest() -> std::os::raw::c_ulong;
    fn pd_crash_log_install(log: *const c_char, flag: *const c_char);
}

/// A native crash nothing caught goes to the log at `log` (one line: the exception, and where), and leaves the
/// file `flag` for the next launch to find. Windows only (elsewhere it does nothing).
pub fn install_crash_log(log: &Path, flag: &Path) {
    if let (Some(l), Some(f)) = (c_path(log), c_path(flag)) {
        unsafe { pd_crash_log_install(l.as_ptr(), f.as_ptr()) }
    }
}

/// The rate [`Core::read_audio`]'s samples play at.
pub const SAMPLE_RATE: u32 = 48_000;

/// The GBA's (and Game Boy's) frame rate: 16777216 Hz / 280896 cycles per frame.
pub const FPS: f64 = 16_777_216.0 / 280_896.0;

/// GBA key bits, as the core takes them ([`Core::set_keys`]).
pub mod key {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const SELECT: u32 = 1 << 2;
    pub const START: u32 = 1 << 3;
    pub const RIGHT: u32 = 1 << 4;
    pub const LEFT: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
    pub const R: u32 = 1 << 8;
    pub const L: u32 = 1 << 9;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Gba,
    Gb,
    Other,
}

#[derive(Debug)]
pub enum OpenError {
    BadPath,
    /// mGBA couldn't load it: not a ROM it knows, or unreadable.
    NotLoaded,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::BadPath => write!(f, "the path can't be passed to mGBA"),
            OpenError::NotLoaded => write!(f, "mGBA could not load this game"),
        }
    }
}

impl std::error::Error for OpenError {}

fn c_path(p: &Path) -> Option<CString> {
    CString::new(p.to_str()?).ok()
}

/// One loaded game. Not thread-safe by itself: the app keeps it on its emulator thread (it is [`Send`]).
pub struct Core {
    raw: *mut PdCore,
}

// The C side has no thread affinity; only concurrent use is unsafe, and &mut self rules that out.
unsafe impl Send for Core {}

impl Core {
    /// Loads `rom`, with its battery save at `save` (created if missing).
    pub fn open(rom: &Path, save: Option<&Path>) -> Result<Core, OpenError> {
        let rom = c_path(rom).ok_or(OpenError::BadPath)?;
        let save = match save {
            Some(s) => Some(c_path(s).ok_or(OpenError::BadPath)?),
            None => None,
        };
        let raw = unsafe {
            pd_core_open(
                rom.as_ptr(),
                save.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            )
        };
        if raw.is_null() {
            Err(OpenError::NotLoaded)
        } else {
            Ok(Core { raw })
        }
    }

    pub fn platform(&self) -> Platform {
        match unsafe { pd_core_platform(self.raw) } {
            1 => Platform::Gba,
            2 => Platform::Gb,
            _ => Platform::Other,
        }
    }

    pub fn width(&self) -> u32 {
        unsafe { pd_core_width(self.raw) }
    }

    pub fn height(&self) -> u32 {
        unsafe { pd_core_height(self.raw) }
    }

    /// The frame, `width * height` pixels of R, G, B and a junk 4th byte.
    pub fn pixels(&self) -> &[u8] {
        let n = (self.width() * self.height() * 4) as usize;
        unsafe { std::slice::from_raw_parts(pd_core_pixels(self.raw), n) }
    }

    pub fn set_keys(&mut self, mask: u32) {
        unsafe { pd_core_set_keys(self.raw, mask) }
    }

    pub fn run_frame(&mut self) {
        unsafe { pd_core_run_frame(self.raw) }
    }

    /// The last frame's audio, interleaved stereo at [`SAMPLE_RATE`]; returns the samples written to `out`.
    pub fn read_audio(&mut self, out: &mut [i16]) -> usize {
        unsafe { pd_core_read_audio(self.raw, out.as_mut_ptr(), out.len()) }
    }

    /// Lets a save the game is writing finish (silent frames, at most `max_frames`); returns the frames run.
    pub fn finish_save(&mut self, max_frames: i32) -> i32 {
        unsafe { pd_core_finish_save(self.raw, max_frames) }
    }

    pub fn save_state(&mut self, path: &Path) -> bool {
        c_path(path).is_some_and(|p| unsafe { pd_core_save_state(self.raw, p.as_ptr()) })
    }

    pub fn load_state(&mut self, path: &Path) -> bool {
        c_path(path).is_some_and(|p| unsafe { pd_core_load_state(self.raw, p.as_ptr()) })
    }

    /// Does the save inside the state at `path` match the save file? `None`: it holds no save, or can't be read.
    pub fn state_matches_save(&mut self, path: &Path) -> Option<bool> {
        let p = c_path(path)?;
        match unsafe { pd_core_state_matches_save(self.raw, p.as_ptr()) } {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        }
    }

    /// Reads `dst.len()` bytes off the bus at `addr`.
    pub fn read(&mut self, addr: u32, dst: &mut [u8]) -> bool {
        unsafe { pd_core_read(self.raw, addr, dst.as_mut_ptr(), dst.len()) }
    }
}

impl Core {
    /// Calls the game's own Thumb function at `fun` with r0-r3 = `args`, between frames, and puts the CPU back
    /// as it was. What it returned in r0, or None (a Game Boy game, or it never returned).
    pub fn call(&mut self, fun: u32, args: [u32; 4]) -> Option<u32> {
        let mut r0 = 0u32;
        unsafe { pd_core_call(self.raw, fun, args.as_ptr(), &mut r0) }.then_some(r0)
    }

    /// Writes `bytes` at `addr`, between frames: EWRAM / IWRAM only (anything else is refused). GBA only.
    pub fn write(&mut self, addr: u32, bytes: &[u8]) -> bool {
        unsafe { pd_core_write(self.raw, addr, bytes.as_ptr(), bytes.len()) }
    }

    /// mGBA's own mixer switch for one sound channel (GBA: 0-3 the PSG's square 1, square 2, wave and noise;
    /// 4-5 DirectSound A / B). It only changes what's heard: the game, and any state saved, never see it.
    pub fn set_audio_channel(&mut self, id: i32, enable: bool) {
        unsafe { pd_core_enable_audio_channel(self.raw, id, enable) }
    }

    /// A save state in memory, without the save data: a copy of the game as it is, to start another core
    /// from. Only reads the core.
    pub fn state_bytes(&mut self) -> Option<Vec<u8>> {
        let mut p: *mut u8 = std::ptr::null_mut();
        let mut len = 0usize;
        if !unsafe { pd_core_state_to_memory(self.raw, &mut p, &mut len) } {
            return None;
        }
        let v = unsafe { std::slice::from_raw_parts(p, len) }.to_vec();
        unsafe { pd_core_free(p.cast()) };
        Some(v)
    }

    /// Loads a [`Core::state_bytes`] copy.
    pub fn load_state_bytes(&mut self, state: &[u8]) -> bool {
        unsafe { pd_core_state_from_memory(self.raw, state.as_ptr(), state.len()) }
    }

    /// EXPAND (the Android app's pk_wide.c) has tables for the ROM with this CRC32.
    pub fn wide_knows(crc: u32) -> bool {
        unsafe { pd_core_wide_knows(crc) }
    }

    /// EXPAND draws this game (its ROM's CRC32; 0 = off). False if it can't. One core at a time: the player's.
    pub fn wide_configure(&mut self, crc: u32) -> bool {
        unsafe { pd_core_wide_configure(self.raw, crc) }
    }

    /// The window EXPAND draws from the next frame on, in the game frame's pixels ((0, 0) its top-left, the
    /// frame 240 x 160); w 0 = none (the plain frame).
    pub fn wide_set_view(&mut self, x: i32, y: i32, w: i32, h: i32) {
        unsafe { pd_core_wide_set_view(self.raw, x, y, w, h) }
    }

    /// Whether the next frame is shown: false only keeps EXPAND's tracking up (fast-forward's unseen frames).
    pub fn wide_set_draw(&mut self, draw: bool) {
        unsafe { pd_core_wide_set_draw(self.raw, draw) }
    }

    /// The last frame's EXPAND window, R G B X per pixel, with its size; None when it wasn't drawn wide.
    pub fn wide_pixels(&self) -> Option<(&[u8], usize, usize)> {
        let (mut w, mut h) = (0, 0);
        let p = unsafe { pd_core_wide_pixels(self.raw, &mut w, &mut h) };
        if p.is_null() || w <= 0 || h <= 0 {
            return None;
        }
        let (w, h) = (w as usize, h as usize);
        // The buffer holds the largest window; the last frame used w * h of it, row by row.
        Some((unsafe { std::slice::from_raw_parts(p, w * h * 4) }, w, h))
    }

    /// What the last frame was, for ZOOM (pk_wide.h's PK_WIDE_SCENE_*); 4 (off the map) when EXPAND is off.
    pub fn wide_scene(&self) -> i32 {
        unsafe { pd_core_wide_scene(self.raw) }
    }

    /// The fault that switched EXPAND off for this game (a Windows exception code; 0 = none).
    pub fn wide_fault(&self) -> u32 {
        unsafe { pd_core_wide_fault(self.raw) as u32 }
    }

    /// Runs this core's frames, calls and state loads under the fault guard (Windows): a fault in them leaves the
    /// core dead ([`Core::dead`]) instead of taking the process down. For helper cores, never the player's game.
    pub fn set_guarded(&mut self, guarded: bool) {
        unsafe { pd_core_set_guarded(self.raw, guarded) }
    }

    /// The fault that killed this guarded core (a Windows exception code), or None. A dead core does nothing.
    pub fn dead(&self) -> Option<u32> {
        let f = unsafe { pd_core_dead(self.raw) } as u32;
        (f != 0).then_some(f)
    }

    /// Tests: a fault inside this core's guarded work.
    pub fn test_fault(&mut self) {
        unsafe { pd_core_test_fault(self.raw) }
    }

    /// Tests: EXPAND's next frame faults (inside its guard).
    pub fn wide_test_fault(&mut self) {
        unsafe { pd_core_wide_test_fault(self.raw) }
    }

    /// The guard catching a deliberate fault on its own: its exception code (0 where there's no guard).
    pub fn guard_selftest() -> u32 {
        unsafe { pd_core_guard_selftest() as u32 }
    }

    /// Parks the game's main loop on the Thumb `b .` at `spin`: only its interrupts (the sound engine) run on.
    pub fn park(&mut self, spin: u32) -> bool {
        unsafe { pd_core_park(self.raw, spin) }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        unsafe { pd_core_close(self.raw) }
    }
}
