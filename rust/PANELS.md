# Writing panels for Adventure Pad

The extra screen beside the game (or in its own window, on a second monitor) is built from **panels**.
Each panel is a tab that reads the running game's memory and shows whatever it likes about it: a party,
a map, a speedrun timer, a randomizer tracker, a damage calculator. Panels come in **packs**, and a pack
decides which games its panels are for.

The built-in ones are written exactly the way yours will be:

| Pack | Crate | Panels |
| --- | --- | --- |
| Pokémon (Gen 3) | `crates/panels-pokemon` | PARTY, BATTLE (opens by itself in a battle), BAG |
| Developer tools | `crates/panels-tools` | MEMORY: a hex view and a watch list, for any game |

## Five minutes to a panel

1. Make a crate next to the others and add it to the workspace (`rust/Cargo.toml`, `members`):

   ```toml
   # crates/panels-mine/Cargo.toml
   [package]
   name = "panels-mine"
   version.workspace = true
   edition.workspace = true
   license.workspace = true

   [dependencies]
   adventurepad-sdk = { path = "../sdk" }
   ```

2. Write the panel and its pack:

   ```rust
   // crates/panels-mine/src/lib.rs
   use adventurepad_sdk::{egui, style, Action, GameInfo, Memory, Panel, PanelCtx, PanelPack};

   /// FireRed's money, live.
   pub struct Money { value: Option<u32> }

   impl Panel for Money {
       fn title(&self) -> String { "MONEY".into() }

       // A new copy of memory, 4 times a second: read here, keep what ui() needs.
       fn refresh(&mut self, _game: &GameInfo, m: &Memory) {
           let sb1 = m.u32(0x0300_5008);                       // gSaveBlock1Ptr
           let sb2 = m.u32(0x0300_500C);                       // gSaveBlock2Ptr
           let key = sb2.and_then(|p| m.u32(p + 0xF20));       // its encryption key
           let money = sb1.and_then(|p| m.u32(p + 0x290));
           self.value = money.zip(key).map(|(v, k)| v ^ k);
       }

       // Every redraw: draw from what refresh() kept.
       fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut PanelCtx) {
           match self.value {
               Some(v) => { ui.heading(format!("₽{v}")); }
               None => style::hint(ui, "Not in a game yet."),
           }
           if ui.button("Save a state").clicked() {
               ctx.act(Action::SaveState(9));
           }
       }
   }

   pub struct MyPack;

   impl PanelPack for MyPack {
       fn name(&self) -> &str { "My pack" }
       fn panels_for(&self, game: &GameInfo) -> Vec<Box<dyn Panel>> {
           if game.code == "BPRE" { vec![Box::new(Money { value: None })] } else { vec![] }
       }
   }
   ```

3. Register the pack in `crates/adventurepad/src/panels.rs` (`packs()`), and add the crate to
   `crates/adventurepad/Cargo.toml`. Run `cargo run --release -p adventurepad`, open FireRed, and the
   MONEY tab is there.

## What a panel gets

- **`GameInfo`**: the ROM's header title, game code (`BPRE`, `BPEE`...), revision, CRC32 (tells ROM hacks
  apart) and its bytes. Use it in `panels_for` to say which games you support.
- **`Memory`**: a copy of the game's memory, 4 times a second. On a GBA: EWRAM at `0x02000000`, IWRAM at
  `0x03000000` and the ROM at `0x08000000`; on a Game Boy: WRAM at `0xC000`, HRAM at `0xFF80` and the ROM's
  first 32 KB. `u8` / `u16` / `u32` / `read` / `slice` are little-endian and bounds-checked (`None` past
  what's there). It's a copy: reading it can't disturb the game, and a slow panel can't slow the game down.
- **`PanelCtx::act`** with an **`Action`**: save or load a state, hold buttons for some frames, or show a
  short notice. The app carries them out between frames.
- **`adventurepad_sdk::egui`**: the app's egui, to draw with. **`adventurepad_sdk::style`** has the app's
  colours, an HP-style `meter`, a clickable `card`, `heading` and `hint`, so a panel looks at home.

Optional hooks:

- `visible()`: hide the tab when it has nothing to show (BATTLE outside a battle).
- `wants_focus()`: ask to be the open tab (BATTLE as a battle starts). The app opens it, and goes back to the
  tab before when it stops asking. If the player picks another tab meanwhile, their pick wins.

## Rules the host keeps

- A panel that panics is switched off for that game, with a note on its tab. The game and the other panels
  carry on.
- Keyboard focus in a panel's text field keeps the keys from the game (so typing an address doesn't walk
  the player around).
- Panels never run on the emulator's thread.

## Finding addresses

Turn on **Developer panels** (library, PANELS) and open MEMORY: watch an address, play, and see what moves.
For Pokémon games the pret decomps' `.map` files name every RAM global (`gPlayerParty`, `gSaveBlock1Ptr`...);
`crates/companion/src/game.rs` has the ones this repo already uses.

## Testing a panel

`Memory::from_dumps(dir, rom)` loads RAM dumps (`ewram.bin` + `iwram.bin`), like the repo's fixtures in
`app/src/test/resources/fixtures`, so a panel's reads can be tested without a ROM or a window:

```rust
let m = Memory::from_dumps(Path::new("../../../app/src/test/resources/fixtures/firered_vanilla"), None)?;
let mut panel = Money { value: None };
panel.refresh(&game, &m);
assert_eq!(panel.value, Some(224_300));
```

`crates/panels-pokemon/src/lib.rs`'s tests do this. For the whole path with a real game,
`ADVENTUREPAD_TEST_ROM=<rom> cargo test -p adventurepad -- --ignored` runs the core, the memory copies and the
Pokémon panels together.

## Later

Panels are compiled in for now: a new pack means a rebuild. Loading packs at runtime (so anyone can share
one as a file, in any language) is the next step for the SDK; the `Panel` / `PanelPack` / `Memory` / `Action`
surface is kept small so it can carry over to that unchanged.
