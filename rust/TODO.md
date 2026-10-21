# To do: PokeDaisy's features in Adventure Pad

What's left to bring over from PokeDaisy's Android app (its README's feature list), adapted to a PC. Tick items off
as they land. [PARITY.md](PARITY.md) has the same ground as a status table, with where each piece's Android code
is.

Already settled, so not listed: **EXPAND** (with ZOOM and the look-around) and **fast-forward's sound** (the song at
normal speed, effects over it).

Effort: **S** a session or less · **M** a few sessions · **L** a big port.

## Asked for first

- [ ] **Controller support** (M): Xbox, 8BitDo, GameSir and other pads (a gamepad crate such as `gilrs`), plugged in
      or connected while a game runs. Every game button and hotkey rebindable to a pad button in CONTROLS. The right
      stick drives EXPAND's look-around and its click is ZOOM, as on Android. X / Y as a second START / SELECT.
      HOME / guide works like BACK for the panels.
- [ ] **HUD over the game, clickable with the mouse** (L): your party (levels, HP, status), a minimap, your Poké
      Balls and BAG / WILD buttons on the game view while you walk around. A click opens a Pokémon's summary, the
      whole map (with FLY), a compact bag with USE, or the area's wild Pokémon; the game keeps running under them.
      Three layouts (corners, side rails, a bottom dock), two looks (see-through glass, the app's windows), three
      sizes, each part can be hidden. Steps aside for battles, menus and (with EXPAND) conversations.
      (`FieldHud.kt`)

## The panels (the companion)

- [ ] **PARTY** (M)
  - [x] Your team with HP, levels and status; a summary with stats, moves, EXP, IVs / EVs and nature.
  - [ ] Drawn in your game's own party screen (its art rebuilt from the ROM: `RomArt`, `3ds/core/pd_romart.c`).
  - [ ] The summary's weaknesses / resistances, and Hidden Power.
  - [ ] LEARN: the moves it learns next and the TMs in your bag it can use, each rated LEARN / FORGET a move / SKIP,
        with the reasons. (`MoveLearnUi.kt`)
- [ ] **BAG** (M)
  - [x] Every pocket, with descriptions.
  - [ ] Item icons.
  - [ ] A TM: who can learn it, and who it suits best.
- [ ] **BATTLE** (M)
  - [x] Opens by itself in a battle; INFO's move verdicts; SUGGESTIONS; FOE TEAM.
  - [ ] INFO: the foe's weaknesses; scrolling down, its ability, held item, stats (with boosts) and moves; a corner
        button gives the foe the whole page.
  - [ ] STATS: the foe's IVs, EVs and nature next to yours (off by default: FOE IVS in SETTINGS).
  - [ ] FIGHT / switch / RUN buttons, clickable (FireRed and Emerald: `BattleInputController.kt`; `pd_core_call`
        is ported).
- [ ] **MAP** (M): the game's region map with you on it; click a place to name it, or search every town and route.
      (`3ds/core/pd_map.c`, `MapScreen.kt`)
- [ ] **DEX** (L): seen / caught, and full entries: sprite, types, stats, abilities, dex text, how it evolves (the
      whole family, method by method), where it's caught (route, method, levels, odds), and its moves, all from the
      game's own data. (`Pokedex.kt`, `3ds/core/pd_dex.c`)
- [ ] **GUIDE** (L): hints first, answers on a second click. HERE (the area's wild encounters, items, gifts and
      trades, what you have ticked off), NEXT BOSS, EVOLUTIONS, TIPS, WHERE IS, STUCK?. (`Guide*.kt`,
      `3ds/core/pd_guide.c`)
- [ ] **CARD** (M): your trainer card, drawn like the game's, front and back. (`3ds/core/pd_card.c`)
- [x] **STATES**: 10 save-state slots with screenshots.
- [ ] Pick which tabs show (S).

## Games the panels read

- [x] FireRed, LeafGreen, Emerald (English, retail).
- [ ] Ruby and Sapphire (S).
- [ ] The other languages: ES / DE / FR / IT / JP (M).
- [ ] The ROM hacks PokeDaisy reads (Unbound, Radical Red, Lazarus, Emerald Rogue, Seaglass, SoulGold and about 30
      more) (L: their tables are most of the app's Kotlin; much of it is generated, so a generator carries it).
- [ ] Game Boy: Yellow, Crystal, Crystal Clear, Prism (L).

## The player

- [ ] **Fast-forward speeds** (S): 1.5x to 10x, or no cap (now 2 / 3 / 4 / 8 / no cap).
- [ ] **Slow motion** (S).
- [ ] **Smart fast-forward** (M): normal speed in menus and on the map, fast battles. (`FfMenuWatch.kt`)
- [ ] **Game speed per game** (S): 0.25x, 0.5x, 1x, 1.5x, 2x, 3x or 4x, kept for each game.
- [ ] **Real-time clock** (S): check day and night, berries, tides and daily events follow the PC's clock, and
      that saves keep it (mGBA's clock, and the dead-battery fix, are in).
- [ ] **Save-state undo** (S): undo a save, undo a load.
- [ ] **Rewind** (S): hold a key to play the last ~20 seconds backwards, never touching the save.
      (`pk_rewind.c`, C as is)
- [ ] **Status bar** (S): game, place, money, clock (and a laptop's battery) above the game.
- [ ] **Aspect** (S): the game's own shape (3:2 / 10:9) or stretched to fill the window (EXPAND is in).
- [ ] **Shaders** (M): an LCD grid (plain or on paper), scanlines, a CRT look, the GBA screen's own colours; edge
      smoothing (xBRZ, xBR, ScaleFX), alone or under any of them; on the panels too if you like.
      (`ScreenShaders.kt`)
- [ ] **Overlays** (M): a frame around the game, PokeDaisy's own two or your RetroArch overlays (`.cfg` + PNG, a
      `.zip` of them, any PNG with a see-through screen).
- [ ] **Controls** (S)
  - [x] Every game button and hotkey rebindable (keyboard).
  - [ ] Turn hotkeys off.
  - [ ] TURBO A / B on any key or button, pressing in game time (keeps up with fast-forward).
- [ ] **Settings search** (S): type "key bindings", "rebind", "turbo", "box art" and the matching settings show.
- [ ] **First-time setup** (S): the saves folder, the ROMs folder, walking through the game buttons.
- [ ] **Themes** (S): PokeDaisy's own, or eight game-coloured ones, for the library and settings.

## The library

- [ ] **List or grid**, recently played (S; the recent list is in).
- [ ] **Cover art** (M): SteamGridDB or RetroAchievements box art, or your own image. (`CoverArtSync.kt`)
- [ ] **A ROMs folder** (S): its games listed, rescanned on REFRESH (saying what it found); add more with +.
- [ ] **Zipped ROMs** (S): `.zip` and `.7z`.
- [ ] **A game's menu** (right-click) (S): rename or hide it; load a save into it (the current one kept as a dated
      backup); INFO (ROM, save file, save states); its own save folder (another emulator's), and ALSO LOOK IN folders.
- [ ] **NOT SUPPORTED** tag and **ASK FOR SUPPORT** (S): a GitHub issue with the ROM's file name and SHA-1, never
      the ROM.
- [ ] **Best effort** (M): an unknown build or hack read as each game the app knows, keeping the one that fits
      (PARTIALLY SUPPORTED: party, bag, map and battles); WHICH GAME IS THIS? when none clearly does, remembered per
      ROM (FORGET MATCH undoes it); unsupported hacks of the five games listed so you can try them. (`BestEffort.kt`)

## Not for a PC as they are

- On-screen touch controls: a keyboard and pads instead (the HUD and battle buttons take the mouse).
- The dual-screen launcher: the panels' own window on a second monitor covers the second screen.
- Launching from ES-DE / Cocoon / iiSU: a ROM path on the command line opens it (`AdventurePad.exe game.gba`).
