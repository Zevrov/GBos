# Adventure Pad

A native Windows Game Boy, Game Boy Color and GBA player: one `AdventurePad.exe`, no runtime to install.
The game runs on the repo's mGBA submodule (compiled in), with an extra screen of **panels** beside it (or in
a window of its own, for a second monitor) and cloud saves through a synced folder.

**Work in progress.** Windows first, **iOS planned** next (not started). What the Android app does and this
doesn't yet is tracked in [PARITY.md](PARITY.md); there are no release builds yet, so build it as below.

**Based on PokeDaisy by Lidor Itzhari - https://github.com/lidor30/pokedaisy**

Adventure Pad grew out of [PokeDaisy](https://github.com/lidor30/pokedaisy): its emulator setup and its Pokémon
readers are ported from PokeDaisy's Android app, which lives in this same repository. It's free software under
the GNU GPL v3 with PokeDaisy's additional term (keep the line above in the docs and the credits; see
[NOTICE](../NOTICE)). Adventure Pad is a fan project, not affiliated with or endorsed by Nintendo, Game Freak,
Creatures or The Pokémon Company; no games, BIOS or game art are included.

**Writing a panel:** see [PANELS.md](PANELS.md).

## Crates

| Crate | What it is |
| --- | --- |
| `crates/mgba-sys` | libmgba (the `third_party/mgba` submodule) built with CMake, behind a small flat C shim (`csrc/pd_core.c`, the same core setup and save handling as the Android app's JNI bridge) and a safe `Core` |
| `crates/companion` | The Pokémon readers in Rust: game detection, the party / battle / bag / money reads and the battle verdicts and suggestions. Ported from `3ds/core` (itself the Kotlin app's `NativeReader` / `Gen3Mon`); same addresses. Retail FireRed, LeafGreen and Emerald so far |
| `crates/sdk` | `adventurepad-sdk`: the `Panel` / `PanelPack` traits, the `Memory` copy panels read, the `Action`s they can ask for, and the app's style |
| `crates/panels-pokemon` | The Pokémon pack: PARTY, BATTLE, BAG |
| `crates/panels-tools` | The developer pack: MEMORY (hex view and watch list) |
| `crates/adventurepad` | The app: an egui window, cpal sound, the emulator thread, the panel host, save states and the synced-folder saves |

`crates/companion/src/tables_gen.rs` is generated: `python rust/tools/gen_tables.py` (from
`3ds/core/pd_tables_gen.c`, which `make -C 3ds tables` builds from the app's Kotlin tables).

## Build

Needs Rust with the GNU toolchain, CMake, and MSYS2's MinGW64 GCC + Ninja (Rust's `windows-gnu` target
links against msvcrt, so use the `mingw64` packages, not `ucrt64`):

```bash
winget install Rustlang.Rustup Kitware.CMake MSYS2.MSYS2
rustup default stable-x86_64-pc-windows-gnu
C:/msys64/usr/bin/bash.exe -lc "pacman -S --noconfirm mingw-w64-x86_64-gcc mingw-w64-x86_64-ninja"
git submodule update --init third_party/mgba
```

Then, with `C:\msys64\mingw64\bin` and CMake on `PATH`, from `rust/`:

```bash
cargo run --release -p adventurepad
cargo test
```

The app is `rust/target/release/AdventurePad.exe`. Pass a ROM path to open it at once (`AdventurePad.exe game.gba`).
`cargo run --release -p mgba-sys --example headless -- <rom> <save> [frames]` boots a ROM with no window and
prints what the companion reads.

`ADVENTUREPAD_HOME` moves the app's folder (settings, states, the default saves folder) from
`%APPDATA%\AdventurePad`; `ADVENTUREPAD_DEBUG=1` prints the frame rate to stderr.

## Controls and settings

**Settings** (in the library, or the bar over a running game, which pauses while it's open) has:

- **CONTROLS**: every game button and hotkey, rebindable (click it, press the new key; a key that was
  another control's moves over). Defaults below.
- **FAST-FORWARD**: 2x, 3x, 4x, 8x or as fast as it goes; the music at its **normal speed**, or off. In the
  Pokémon GBA games and their hacks a second, silent core of the same ROM plays the song the game is on, at
  1x, with the ROM's own `m4aSongNumStart` (`crates/adventurepad/src/music.rs`, after the Android app's STEADY
  FF music); fast-forward plays that, so the song keeps its tempo while the game races. The game's sound
  effects and cries come from a **shadow core**: a throwaway copy of the game started from a snapshot of it,
  run in step with the same buttons, with its music muted (its m4a tracks' volume, and the PSG channels a music
  track holds switched off in mGBA's mixer). They're mixed over the song at their own pitch, shortened (WSOLA,
  `wsola.rs`), or sped up (a setting). The player's game is only ever read: muting changes how long the sound
  engine takes each frame, which moved Emerald's random numbers in a test, so only the copy is muted, and it's
  re-synced from a fresh snapshot every 300 frames. `the_shadow_leaves_the_game_alone` checks that all of the
  game's RAM comes out byte for byte as without it (FireRed, Emerald, Lazarus, Emerald Rogue), and nothing is
  written to a game whose sound engine doesn't look like the one the mute knows (it gets the music alone).
  Other games get their own sound at its normal pitch, in slices that skip ahead.
- **PANELS**: the panels in their own window (for a second screen); the developer panels.
- **SAVES**: the saves folder (the synced folder for cloud saves).

| Key | Does |
| --- | --- |
| Arrows | D-pad |
| Z / X | A / B |
| A / S | L / R |
| Enter / Backspace | START / SELECT |
| Tab (hold) / Space | Fast-forward held / on-off |
| F5 / F7 | Save / load the selected state slot |
| F6 / F8 | Previous / next slot |
| Esc | Leaves SETTINGS |

## Cloud saves

The library's SAVES section offers the OneDrive, Dropbox, Google Drive and iCloud Drive folders it finds (or
any folder). Put the saves there and the sync client carries them between devices; point the Android
app's saves folder at the same synced folder to share one save. On top of a plain folder:

- every launch backs the save up first (`pokedaisy-backups/`, newest 10: the Android app's folder name, so
  both devices share one set);
- `<save>.playing` marks a game open on a device (refreshed every minute, stale after 3), so opening it on
  another device warns first. It's the same JSON as the Kotlin `SyncedSaves.kt`, so the Android app can read
  it once it's wired in there;
- the copies a sync client makes when two devices saved anyway (`Name-PC.sav`, `Name (conflicted copy).sav`,
  `Name (1).sav`) are found at launch, with a pick of which to play (the other is backed up).

The game's own saves sync. Save states and the resume state stay on each device; if the save changed on
another device since this one's resume state, the game boots from the save instead.
