# Adventure Pad vs PokeDaisy (Android): feature parity

Where the Windows app stands against the Android app's features (its README and `docs/ROM_FEATURES.md`), and
what in this repo each one can be built from, which mostly decides the effort.

✅ done · ◐ partly · — missing · n/a doesn't apply on a PC

Effort: **S** a session or less · **M** a few sessions · **L** a big port (many sessions).

## The player

| Feature | Windows | Built from | Effort |
| --- | :-: | --- | :-: |
| Plays GB / GBC / GBA | ✅ | mGBA | |
| Save states, 10 slots with screenshots | ✅ | | |
| Save-state **undo** (undo save / undo load) | — | `EmulatorEngine.kt` (bak files) | S |
| Resume where you left off | ✅ | | |
| Fast-forward, speed cap | ◐ 2/3/4/8x/uncapped | Android: 1.5x to 10x | S |
| Fast-forward music at normal speed | ✅ (plus sped-up effects over it) | | |
| **Smart** fast-forward (normal speed in menus / map, fast battles) | — | `FfMenuWatch.kt`, `RegionMapWatch.kt` | M |
| **Slow motion**, **game speed** per game (0.25x to 4x) | — | engine pacing | S |
| **Rewind** (~20 s) | — | `app/src/main/cpp/pk_rewind.c` (C, reusable as is) | S |
| **Cheats** | — | `pk_cheats.c` (C) + `cheats/Cheats.kt` | M |
| Real-time clock (RTC games) | ◐ mGBA's clock; the dead-battery fix is ported | | S (check) |
| Remap every button and hotkey | ✅ keyboard | | |
| **Gamepads** (Xbox, 8BitDo, ...), hot-plug | — | a gamepad crate (gilrs) | S |
| **Turbo A / B** | — | `GbaInput.kt` | S |
| Hotkeys off / on | — | | S |
| **Status bar** (game, place, money, clock) | — | `GameStatusBar.kt` | S |
| **HUD** over the game (party, minimap, balls, BAG / WILD) | — | `FieldHud.kt` (Compose: rewrite in egui) | L |
| **Battle buttons** (FIGHT / POKéMON / RUN) | — | `BattleButtons.kt`, `BattleInputController.kt` | M |
| **EXPAND** (overworld past the GBA's edges), ZOOM, look-around | ✅ (look-around on keys until gamepads; no fault guard on Windows yet) | `pk_wide.c` as is + `wide.rs` (WideCamera.kt) | |
| Aspect: native / stretch | ◐ native (integer scaled) | | S |
| **Shaders** (LCD, scanlines, CRT, GBA colours, xBRZ...) | — | `ScreenShaders.kt` (GLSL: port to egui's glow) | M |
| **Overlays** (RetroArch .cfg / PNG frames) | — | `overlay/` | M |
| Field toggles: AUTO-RUN, RUN INDOORS, WILD ENCOUNTERS, SCENE SPEEDS | — | `pk_wide.c` field reads + `EmulatorEngine.kt` | M |
| QUICK MENU (bike, rod, repel...), item USE / GIVE / TAKE, FLY | — | `QuickMenu.kt`, `FieldItems`, `Fly.kt` (call the game's own functions: `pd_core_call` is ported) | M |
| Touch controls | n/a | | |

## The companion (panels)

| Feature | Windows | Built from | Effort |
| --- | :-: | --- | :-: |
| PARTY: slots, HP, status, summary (moves, stats, IVs / EVs, nature) | ✅ | | |
| PARTY: **LEARN** (next moves, TMs in the bag, LEARN / FORGET / SKIP) | — | `MoveLearnUi.kt` + learnset tables | M |
| PARTY / BAG in the **game's own look** (its party screen, icons from the ROM) | — | `RomArt`, `3ds/core/pd_icon.c`, `pd_romart.c` | M |
| BAG: pockets, descriptions | ✅ | | |
| BAG: item icons, **TM: who can learn it** | — | `ItemsScreen.kt` | M |
| BATTLE: INFO verdicts, SUGGESTIONS, FOE TEAM | ✅ | | |
| BATTLE: foe's ability, item, stats with boosts; **STATS** (foe IVs / EVs) | — | `BattleInfoScreen.kt` | S |
| **MAP** (region map, you on it, search places) | — | `3ds/core/pd_map.c` + `MapScreen.kt`, maps rebuilt from the ROM | M |
| **DEX** (seen / caught, entries, evolutions, where to catch, moves) | — | `Pokedex.kt`, `3ds/core/pd_dex.c` | L |
| **GUIDE** (HERE, NEXT BOSS, EVOLUTIONS, TIPS, WHERE IS, STUCK?) | — | `Guide*.kt` (generated), `3ds/core/pd_guide.c` | L |
| **CARD** (trainer card, front and back) | — | `3ds/core/pd_card.c` (C) | M |
| STATES | ✅ (a host tab) | | |
| **PC BOXES**, **TRADE**, **save editor** | — | `PcBoxesScreen.kt`, `TradeScreen.kt`, `SaveEditorActivity.kt`, `data/save/` | L |
| **ACHIEVEMENTS** (RetroAchievements) | — | `pokedaisy_ra.c` + `rcheevos` (both in the repo) | M |
| Pick which tabs show | — | | S |
| HELP (the manual in the app) | — | `DocsApi.kt` / `DocsScreen.kt` | S |

## Games the companion knows

| Games | Windows | Built from | Effort |
| --- | :-: | --- | :-: |
| FireRed, LeafGreen, Emerald (English, retail) | ✅ party / bag / battle | | |
| Ruby, Sapphire | — | `NativeReader.kt` configs | S |
| FireRed / LeafGreen / Emerald / Ruby / Sapphire in ES / DE / FR / IT / JP | — | `NativeReader.kt`, per-language text tables | M |
| ~35 ROM hacks (Unbound, Radical Red, Lazarus, Emerald Rogue, Seaglass, SoulGold, ...) | — (they play, no panels) | `NativeReader.kt`, per-hack species / move / type tables (most of the app's 227k lines of Kotlin are this data, much of it generated) | L |
| Game Boy: Yellow, Crystal, Crystal Clear, Prism | — | `Gen1*.kt`, `Gen2*.kt` | L |
| **Best effort** (an unknown build or hack read as the game it matches; WHICH GAME IS THIS?) | — | `BestEffort.kt` | M |

## Library and settings

| Feature | Windows | Built from | Effort |
| --- | :-: | --- | :-: |
| Recent games, open any ROM | ✅ | | |
| A **ROMs folder** scanned into a library, refresh | — | `RomFolder.kt` | S |
| List / **grid**, **cover art** (SteamGridDB, RetroAchievements, your own) | — | `CoverArtSync.kt`, `SteamGridDbClient.kt` | M |
| **Zipped ROMs** (.zip / .7z) | — | `RomArchive.kt` (needs a zip / 7z crate) | S |
| Rename / hide a game, game INFO | — | `LibraryActivity.kt` | S |
| **Load a save** into a game (old one kept as a backup) | — | `GameSaves.kt` | S |
| Per-game save folder, **ALSO LOOK IN** folders (RetroArch's) | — | `SavesLocation.kt` | S |
| NOT SUPPORTED tag, ASK FOR SUPPORT | — | `RomSupportRequest.kt` | S |
| Save backups before every launch | ✅ | | |
| **Cloud saves** (synced folder, open-elsewhere warning, conflict copies) | ✅ Windows only (not in the Android app yet) | | |
| Panel SDK, pop-out panels window | ✅ Windows only | | |
| **Languages** (JA FR DE IT ES PT-BR PT) | — | `companion/i18n/` | M |
| **Themes** | — | `theme/` | S |
| **Settings search** | — | `SETTING_KEYWORDS` | S |
| NEW tags on new features | — | `NewFeatures.kt` | S |
| Update check | — | `AppUpdater.kt` (GitHub releases) | S |
| An installer | — | (cargo-wix / an MSI) | S |
| Dual-screen launcher, frontends (ES-DE, iiSU), crash reports | n/a (a ROM path on the command line works) | | |

## Suggested order

1. **Quick player wins (S):** gamepads, rewind, slow-mo and game speed, turbo, more fast-forward caps, save-state
   undo, status bar, zipped ROMs, load save, per-game / ALSO LOOK IN folders, a ROMs folder library. Mostly
   reusable C (`pk_rewind.c`) or small Kotlin to port.
2. **Retail depth (M / L):** MAP, CARD, DEX and GUIDE for FireRed / LeafGreen / Emerald, then Ruby / Sapphire and
   the other languages. `3ds/core` already ports MAP, DEX, GUIDE and CARD to portable C for these games, which
   makes this much smaller than the Kotlin.
3. **EXPAND, cheats and RetroAchievements (M):** the C is in the repo (`pk_wide.c`, `pk_cheats.c`,
   `pokedaisy_ra.c` + `rcheevos`).
4. **ROM hacks and Game Boy games (L):** the long tail. The readers are a pattern; the per-game tables are the
   bulk, and much of it is generated, so a generator from the Kotlin (as `rust/tools/gen_tables.py` does for the
   retail tables) carries most of it over.
5. **HUD, battle buttons, QUICK MENU / field items, PC BOXES / TRADE / save editor, shaders, overlays,
   languages, themes, cover art.**
