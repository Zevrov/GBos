//! Fast-forward with the music at its normal speed: a second, silent core of the same ROM plays whatever
//! song the real game is playing, at 1x, and while fast-forward is on that's what you hear. The song is
//! started with the ROM's own `m4aSongNumStart` (the Gen 3 games' and their hacks' sound engine), on a core
//! parked in an idle loop so nothing but its sound engine runs. Ported from the Android app's STEADY FF
//! music (FfMusicRenderer.kt, M4aSongs.kt, FfMusicKey.kt), which records each song to a file instead.

use mgba_sys::Core;
use std::collections::HashMap;
use std::path::Path;

const ROM: u32 = 0x0800_0000;

/// Where a ROM's m4a sound engine keeps its songs: `m4aSongNumStart(n, alt)`, found by its code (the same
/// library code in every decomp-built game and binary hack), and the song table its literal pool points at.
#[derive(Debug)]
pub struct M4aSongs {
    /// m4aSongNumStart's address (Thumb: odd).
    pub song_num_start: u32,
    /// A Thumb `b .` in the ROM, to park the music core's main loop on.
    pub idle_loop: Option<u32>,
    /// Song header (ROM offset) -> song id ([`ALT`] set for Heart and Soul's alternate soundtrack).
    ids: HashMap<u32, u32>,
}

/// Set on an alternate song's id: m4aSongNumStart's `alt` argument.
pub const ALT: u32 = 1 << 16;
/// MUS_DUMMY: silence.
const MUS_DUMMY: u32 = 0;

/// m4aSongNumStart's code ("." = any nibble), where its `ldr rX, [pc, #imm]` of the song table sits, and of the
/// alternate table (Heart and Soul). agbcc; modern gcc (Emerald Rogue); newer expansion (Lazarus, SoulGold);
/// Heart and Soul. M4aSongs.kt's SIGNATURES: keep in step.
const SIGNATURES: [(&str, usize, Option<usize>); 4] = [
    ("00b50004074a0849400b401883885900c918890089180a680168101c00f0a2fb", 6, None),
    ("00b5..4b0004400bc018828853009b18..4a9b000168d058........01bc0047", 2, None),
    ("00b5..4a03045b0b9b1899884a005218..4992008858..4a416b914202d11968........01bc0047", 2, None),
    (
        concat!(
            "000430b5000c002916d0..4d2b685a1c12d02a0000210c3206e00c3213000c3b1b6801315c1c07d08342f6d1",
            "4b005b189b000433eb1802e0..4bc000c31899884a005218..4992008858..4a416b914202d11968........30bc01bc0047"
        ),
        0x38,
        Some(0x0A),
    ),
];
const MAX_SONGS: usize = 4096;

fn u16_at(b: &[u8], i: usize) -> u32 {
    u16::from_le_bytes([b[i], b[i + 1]]) as u32
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The first `step`-aligned offset where hex `pattern` ("." nibbles match anything) is.
fn find_pattern(hay: &[u8], pattern: &str, step: usize) -> Option<usize> {
    let p = pattern.as_bytes();
    let n = p.len() / 2;
    let nib = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
    let (value, mask): (Vec<u8>, Vec<u8>) = (0..n)
        .map(|k| {
            let (h, l) = (p[k * 2], p[k * 2 + 1]);
            let v = (if h == b'.' { 0 } else { nib(h) << 4 }) | if l == b'.' { 0 } else { nib(l) };
            let m = (if h == b'.' { 0 } else { 0xF0 }) | if l == b'.' { 0 } else { 0x0F };
            (v, m)
        })
        .unzip();
    (0..hay.len().saturating_sub(n) + 1)
        .step_by(step)
        .find(|&i| (0..n).all(|j| hay[i + j] & mask[j] == value[j]))
}

/// The word a Thumb `ldr rX, [pc, #imm]` at `at` loads.
fn literal(rom: &[u8], at: usize) -> Option<u32> {
    let ins = u16_at(rom, at);
    if ins >> 11 != 0b01001 {
        return None;
    }
    let addr = ((at + 4) & !3) + (ins as usize & 0xFF) * 4;
    (addr + 4 <= rom.len()).then(|| u32_at(rom, addr))
}

impl M4aSongs {
    pub fn locate(rom: &[u8]) -> Option<M4aSongs> {
        let (at, table_ldr, alt_ldr) = SIGNATURES
            .iter()
            .find_map(|&(pat, t, a)| find_pattern(rom, pat, 2).map(|at| (at, t, a)))?;
        let table = literal(rom, at + table_ldr)?;
        let t = table.checked_sub(ROM)? as usize;
        if t >= rom.len() {
            return None;
        }
        let in_rom = |h: u32| h >= ROM && ((h - ROM) as usize) < rom.len();
        let mut ids = HashMap::new();
        let mut n = 0;
        while n < MAX_SONGS && t + n * 8 + 8 <= rom.len() {
            let header = u32_at(rom, t + n * 8);
            if !in_rom(header) {
                break;
            }
            ids.entry(header - ROM).or_insert(n as u32);
            n += 1;
        }
        if n == 0 {
            return None;
        }
        // Heart and Soul's alternate table: {u32 n, SongHeader*, u16 ms, u16 me}, n = -1 ends it.
        if let Some(alt) = alt_ldr
            .and_then(|a| literal(rom, at + a))
            .and_then(|a| a.checked_sub(ROM))
        {
            let mut e = alt as usize;
            for _ in 0..MAX_SONGS {
                if e + 12 > rom.len() {
                    break;
                }
                let num = u32_at(rom, e);
                let header = u32_at(rom, e + 4);
                if num > 0xFFFF || !in_rom(header) {
                    break;
                }
                ids.entry(header - ROM).or_insert(num | ALT);
                e += 12;
            }
        }
        let idle_loop = (0..rom.len().saturating_sub(1))
            .step_by(2)
            .find(|&i| rom[i] == 0xFE && rom[i + 1] == 0xE7);
        Some(M4aSongs {
            song_num_start: ROM + at as u32 + 1,
            idle_loop: idle_loop.map(|i| ROM + i as u32),
            ids,
        })
    }

    /// The song id that plays `header` (a ROM address), if it's in the tables.
    pub fn song_id(&self, header: u32) -> Option<u32> {
        self.ids.get(&header.checked_sub(ROM)?).copied()
    }
}

fn in_ram(a: u32) -> bool {
    (0x0200_0000..0x0204_0000).contains(&a) || (0x0300_0000..0x0300_8000).contains(&a)
}

fn read_u32(core: &mut Core, addr: u32) -> Option<u32> {
    let mut b = [0u8; 4];
    core.read(addr, &mut b).then(|| u32::from_le_bytes(b))
}

/// The background-music player's MusicPlayerInfo: SOUND_INFO_PTR (0x03007FF0) -> SoundInfo.musicPlayerHead,
/// a chain of players through musicPlayerNext, newest first; the BGM player (opened first) is its tail.
/// FfMusicKey.kt.
pub fn bgm_player(core: &mut Core) -> Option<u32> {
    let info = read_u32(core, 0x0300_7FF0).filter(|&a| in_ram(a))?;
    let mut p = read_u32(core, info + 0x24)?;
    let mut bgm = 0;
    let mut n = 0;
    while p != 0 {
        n += 1;
        if !in_ram(p) || n > 12 {
            return None;
        }
        bgm = p;
        p = read_u32(core, p + 0x3C)?;
    }
    (bgm != 0).then_some(bgm)
}

/// The song the background-music player is playing (its SongHeader's ROM address), or None for silence.
pub fn current_song(core: &mut Core) -> Option<u32> {
    let bgm = bgm_player(core)?;
    if read_u32(core, bgm + 0x04)? & 0xFFFF == 0 {
        return None; // no tracks playing
    }
    read_u32(core, bgm).filter(|&h| (ROM..0x0A00_0000).contains(&h))
}

// MusicPlayerInfo / MusicPlayerTrack (the pret decomps' m4a.h): trackCount, the tracks array, and in each
// 0x50-byte track its flags and volX - what m4aMPlayVolumeControl sets (volume >> 2, 64 = full) with the
// MPT_FLG_VOLCHG flag, so the engine recomputes the track's channels.
const PLAYER_TRACK_COUNT: u32 = 0x08;
const PLAYER_TRACKS: u32 = 0x2C;
const TRACK_SIZE: u32 = 0x50;
const TRACK_FLAGS: u32 = 0x00;
const TRACK_VOLX: u32 = 0x13;
const MPT_FLG_EXIST: u8 = 0x80;
const MPT_FLG_VOLCHG: u8 = 0x03;
/// MusicPlayerTrack.cmdPtr: where in the song (the ROM) the track reads next - what the layout check reads.
const TRACK_CMD_PTR: u32 = 0x40;
const MAX_TRACKS: u32 = 16;
// SoundInfo.cgbChans: the engine's 4 PSG channels (square 1, square 2, wave, noise), 0x40 bytes each, with
// their statusFlags and the track that plays on them.
const SOUND_INFO_CGB_CHANS: u32 = 0x1C;
const CGB_CHANNEL_SIZE: u32 = 0x40;
const CGB_CHANNEL_TRACK: u32 = 0x2C;

/// A game's music turned down to nothing, so what it plays is its sound effects and cries alone (the music
/// core supplies the song at 1x). Only ever on the fast-forward shadow core, a throwaway copy of the game:
/// a muted sound engine does less work each frame, which moves the game's timing (Emerald's random numbers
/// drifted in a test), so the player's own game is never written to. Only the BGM player's tracks are
/// touched; the game's own volume for each (m4aMPlayVolumeControl: under a cry, a fade) is kept and put back.
/// The engine's PSG notes don't follow a track's volume all the way down, so a PSG channel a music track is
/// playing on is also switched off in mGBA's mixer (output only), and back on as soon as an effect has it.
#[derive(Default)]
pub struct MusicMute {
    /// Per track: the volume the game last asked for (None: the track wasn't playing).
    wanted: Vec<Option<u8>>,
    /// The PSG channels switched off in the mixer.
    psg_off: [bool; 4],
}

impl MusicMute {
    fn tracks(core: &mut Core) -> Option<(u32, u32)> {
        let bgm = bgm_player(core)?;
        let mut count = [0u8; 1];
        core.read(bgm + PLAYER_TRACK_COUNT, &mut count);
        let tracks = read_u32(core, bgm + PLAYER_TRACKS).filter(|&a| in_ram(a))?;
        Some((tracks, (count[0] as u32).min(MAX_TRACKS)))
    }

    fn set(core: &mut Core, track: u32, volx: u8) {
        let mut flags = [0u8; 1];
        core.read(track + TRACK_FLAGS, &mut flags);
        core.write(track + TRACK_VOLX, &[volx]);
        core.write(track + TRACK_FLAGS, &[flags[0] | MPT_FLG_VOLCHG]);
    }

    /// Every frame while fast-forwarding: each playing track's volume to 0, after noting what the game set it
    /// to since (a new song's tracks start at full, a fade lowers them), and the PSG channels music has off.
    pub fn hold(&mut self, core: &mut Core) {
        let Some((tracks, n)) = Self::tracks(core) else {
            return;
        };
        self.gate_psg(core, tracks, n);
        self.wanted.resize(n as usize, None);
        for i in 0..n {
            let t = tracks + i * TRACK_SIZE;
            let mut b = [0u8; 0x14];
            if !core.read(t, &mut b) || b[TRACK_FLAGS as usize] & MPT_FLG_EXIST == 0 {
                continue;
            }
            let volx = b[TRACK_VOLX as usize];
            if volx != 0 {
                self.wanted[i as usize] = Some(volx);
                Self::set(core, t, 0);
            }
        }
    }

    /// Which PSG channels a BGM track plays on now: off in the mixer; the rest on.
    fn gate_psg(&mut self, core: &mut Core, tracks: u32, n: u32) {
        let Some(info) = read_u32(core, 0x0300_7FF0).filter(|&a| in_ram(a)) else {
            return;
        };
        let Some(chans) = read_u32(core, info + SOUND_INFO_CGB_CHANS).filter(|&a| in_ram(a)) else {
            return;
        };
        for i in 0..4u32 {
            let ch = chans + i * CGB_CHANNEL_SIZE;
            let mut flags = [0u8; 1];
            core.read(ch, &mut flags);
            let owner = read_u32(core, ch + CGB_CHANNEL_TRACK).unwrap_or(0);
            let music = flags[0] != 0 && owner >= tracks && owner < tracks + n * TRACK_SIZE;
            if music != self.psg_off[i as usize] {
                core.set_audio_channel(i as i32, !music);
                self.psg_off[i as usize] = music;
            }
        }
    }

    /// The game's own volumes back (the tests' check that the mute is undoable; the shadow core is re-synced
    /// from the game instead).
    #[cfg(test)]
    pub fn release(&mut self, core: &mut Core) {
        for i in 0..4 {
            if self.psg_off[i] {
                core.set_audio_channel(i as i32, true);
                self.psg_off[i] = false;
            }
        }
        if let Some((tracks, n)) = Self::tracks(core) {
            for i in 0..n.min(self.wanted.len() as u32) {
                if let Some(v) = self.wanted[i as usize] {
                    Self::set(core, tracks + i * TRACK_SIZE, v);
                }
            }
        }
        self.wanted.clear();
    }

    /// A state was loaded: its tracks are its own, nothing of ours to put back.
    pub fn forget(&mut self) {
        self.wanted.clear();
    }

    /// Whether `core`'s BGM player looks the way the mute expects (the pret decomps' m4a.h): a sane track
    /// count, the tracks in RAM, and every playing track reading its song from the ROM. A hack whose engine
    /// was changed fails this and is never written to.
    /// None: nothing to tell by yet (no music player set up, no track playing): ask again later.
    pub fn layout_ok(core: &mut Core) -> Option<bool> {
        let (tracks, n) = Self::tracks(core)?;
        if n > MAX_TRACKS || tracks % 4 != 0 {
            return Some(false);
        }
        let mut playing = 0;
        for i in 0..n {
            let t = tracks + i * TRACK_SIZE;
            let mut flags = [0u8; 1];
            core.read(t + TRACK_FLAGS, &mut flags);
            if flags[0] & MPT_FLG_EXIST == 0 {
                continue;
            }
            match read_u32(core, t + TRACK_CMD_PTR) {
                // A track that has finished, or not started: nothing to tell by.
                Some(0) => {}
                Some(p) if (ROM..0x0A00_0000).contains(&p) => playing += 1,
                _ => return Some(false),
            }
        }
        (playing > 0).then_some(true)
    }
}

/// The second core playing the live game's song at 1x.
pub struct SteadyMusic {
    core: Core,
    songs: M4aSongs,
    /// The song this core was told to play (None: silence).
    playing: Option<u32>,
    /// What the live game has played for how many frames in a row: a song must hold a moment before this
    /// core follows (the player's status flickers between songs).
    seen: (Option<u32>, u32),
    scratch: Vec<i16>,
}

/// Frames of boot before parking (logos, the title's sound engine set up): ~5 s of game time.
const BOOT_FRAMES: u32 = 300;
/// At most this long waiting for the title's own music (some games keep the engine off through splash
/// screens: Emerald Rogue ~15 s).
const MAX_BOOT_FRAMES: u32 = 1800;
/// A change of song holds this many live frames before the music core follows.
const FOLLOW_FRAMES: u32 = 3;

impl SteadyMusic {
    /// Boots `rom` silently (no save: it never touches the player's), parks it and quiets its own music.
    /// None for a ROM without the m4a engine (a Game Boy game, an unrecognised GBA one). A few hundred
    /// milliseconds of work: run it off the emulator thread.
    pub fn boot(rom: &Path, rom_bytes: &[u8]) -> Option<SteadyMusic> {
        let songs = M4aSongs::locate(rom_bytes)?;
        let mut core = Core::open(rom, None).ok()?;
        if core.platform() != mgba_sys::Platform::Gba {
            return None;
        }
        let mut scratch = vec![0i16; 8192];
        let mut frames = 0;
        while frames < MAX_BOOT_FRAMES
            && (frames < BOOT_FRAMES || current_song(&mut core).is_none())
        {
            core.run_frame();
            core.read_audio(&mut scratch);
            frames += 1;
        }
        if !songs.idle_loop.is_some_and(|spin| core.park(spin)) {
            eprintln!("steady music: couldn't park the music core");
        }
        let mut m = SteadyMusic {
            core,
            songs,
            playing: Some(u32::MAX),
            seen: (None, 0),
            scratch,
        };
        m.play(None);
        Some(m)
    }

    fn play(&mut self, header: Option<u32>) {
        let id = header
            .and_then(|h| self.songs.song_id(h))
            .unwrap_or(MUS_DUMMY);
        let fun = self.songs.song_num_start & !1;
        self.core
            .call(fun, [id & 0xFFFF, (id & ALT != 0) as u32, 0, 0]);
        self.playing = header;
    }

    /// The live game is playing `live` (from [`current_song`] on its core): follow it once it holds.
    pub fn follow(&mut self, live: Option<u32>) {
        if live == self.seen.0 {
            self.seen.1 = self.seen.1.saturating_add(1);
        } else {
            self.seen = (live, 1);
        }
        if self.seen.1 >= FOLLOW_FRAMES && live != self.playing {
            self.play(live);
        }
    }

    /// One frame of the music at 1x: its sound, interleaved stereo at [`mgba_sys::SAMPLE_RATE`].
    pub fn run_frame(&mut self) -> &[i16] {
        self.core.run_frame();
        let n = self.core.read_audio(&mut self.scratch);
        &self.scratch[..n]
    }

    /// The song this core plays now.
    #[cfg(test)]
    pub fn playing(&self) -> Option<u32> {
        self.playing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_hex_pattern_with_wildcards() {
        let hay = [0x00, 0x11, 0x00, 0xb5, 0x12, 0x4b, 0x99];
        assert_eq!(find_pattern(&hay, "00b5..4b", 2), Some(2));
        assert_eq!(find_pattern(&hay, "00b5..4c", 2), None);
        assert_eq!(find_pattern(&hay, "11", 2), None, "only on even offsets");
    }

    /// A real ROM's engine is found and the music core plays the song the live game plays.
    /// `ADVENTUREPAD_TEST_ROM=<rom> cargo test -p adventurepad -- --ignored`.
    /// Muting the live game's BGM tracks silences its music, and releasing brings it back - checked on the
    /// title screen, where the music is all there is. `ADVENTUREPAD_TEST_ROM=<rom> cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn muting_the_bgm_silences_the_music() {
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(std::path::PathBuf::from)
        else {
            return;
        };
        let mut live = Core::open(&rom, None).unwrap();
        let mut audio = vec![0i16; 8192];
        let mut frames = 0;
        while frames < MAX_BOOT_FRAMES
            && (frames < BOOT_FRAMES || current_song(&mut live).is_none())
        {
            live.run_frame();
            live.read_audio(&mut audio);
            frames += 1;
        }
        let mut energy = |live: &mut Core, mute: Option<&mut MusicMute>, frames: usize| -> f64 {
            let mut mute = mute;
            let mut sum = 0f64;
            let mut count = 0usize;
            for _ in 0..frames {
                if let Some(m) = mute.as_deref_mut() {
                    m.hold(live);
                }
                live.run_frame();
                let n = live.read_audio(&mut audio);
                sum += audio[..n].iter().map(|&s| (s as f64).powi(2)).sum::<f64>();
                count += n;
            }
            (sum / count.max(1) as f64).sqrt()
        };
        let before = energy(&mut live, None, 120);
        let mut mute = MusicMute::default();
        energy(&mut live, Some(&mut mute), 10); // notes already sounding die away
        let muted = energy(&mut live, Some(&mut mute), 120);
        mute.release(&mut live);
        energy(&mut live, None, 10);
        let after = energy(&mut live, None, 120);
        eprintln!("title music RMS: {before:.0} playing, {muted:.0} muted, {after:.0} released");
        assert!(before > 300.0, "the title has music");
        assert!(muted < before * 0.03, "muted: {muted:.0} of {before:.0}");
        assert!(
            after > before * 0.5,
            "back after release: {after:.0} of {before:.0}"
        );
    }

    #[test]
    #[ignore]
    fn the_music_core_follows_the_live_game() {
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(std::path::PathBuf::from)
        else {
            return;
        };
        let bytes = std::fs::read(&rom).unwrap();
        let songs = M4aSongs::locate(&bytes).expect("an m4a game");
        assert!(songs.idle_loop.is_some());
        let mut steady = SteadyMusic::boot(&rom, &bytes).expect("the music core boots");
        // The live game: boot to its title music.
        let mut live = Core::open(&rom, None).unwrap();
        let mut song = None;
        for _ in 0..MAX_BOOT_FRAMES {
            live.run_frame();
            song = current_song(&mut live);
            if song.is_some() {
                break;
            }
        }
        let song = song.expect("the live game plays its title music");
        assert!(songs.song_id(song).is_some(), "the song is in the table");
        let mut loud = 0usize;
        for _ in 0..120 {
            steady.follow(Some(song));
            loud += steady
                .run_frame()
                .iter()
                .filter(|s| s.unsigned_abs() > 200)
                .count();
        }
        assert_eq!(steady.playing(), Some(song));
        assert_eq!(
            current_song(&mut steady.core),
            Some(song),
            "the music core really plays it"
        );
        assert!(
            loud > 10_000,
            "and it's audible: {loud} loud samples in 2 s"
        );
        steady.follow(None);
        for _ in 0..10 {
            steady.follow(None);
            steady.run_frame();
        }
        assert_eq!(steady.playing(), None, "silence follows silence");
    }
}
