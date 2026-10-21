//! The battle buttons' driver: picks FIGHT and a move, a Pokémon to switch to, or RUN in the game's own battle menu
//! by pressing its buttons, as a player would - the Gen 3 part of the Android app's BattleInputController.kt, which it
//! follows step for step (keep in step).
//!
//! - **What's up**: the battle's menu as the game runs it - which of its input handlers (choose an action, a move, a
//!   target; the bag or the party menu waiting) `gBattlerControllerFuncs` holds for a battler ([read_state]).
//! - **A pick**: where the game's menu cursors are known (gActionSelectionCursor / gMoveSelectionCursor), it reads the
//!   cursor and presses only what moves it to the cell, then A; elsewhere it walks there blind (LEFT, UP, then RIGHT /
//!   DOWN, a settling gap after each). While a sequence runs, the player's own keys are shut out so they can't throw
//!   it off - bounded, so nothing here can lock them out for good.
//! - **A switch**: POKéMON, then the party menu's cursor steered to the Pokémon (found by its personality: the battle
//!   party menu shows the party in battle order) and SHIFT. The screen keeps the battle's last frame meanwhile
//!   ([BattleDriver::hold_frame]), instead of a fade to black and a party menu nobody asked to see.
//!
//! Unlike the Android app it never writes the game's RAM (Android can set the cursor byte for a one-press pick): only
//! buttons, so the game sees nothing a player couldn't do.

use mgba_sys::{key, Core};
use std::collections::VecDeque;

/// The battle's menu (the Android app's BATTLE_INPUT_*).
pub const NONE: u8 = 0;
pub const BUSY: u8 = 1;
pub const ACTION_SELECT: u8 = 2;
pub const MOVE_SELECT: u8 = 3;
pub const BAG_OPEN: u8 = 4;
pub const PARTY_OPEN: u8 = 5;
pub const TARGET_SELECT: u8 = 6;

/// FIGHT, BAG, POKéMON, RUN: the action menu's cells (gActionSelectionCursor's values).
pub const ACTION_FIGHT: u8 = 0;
pub const ACTION_POKEMON: u8 = 2;
pub const ACTION_RUN: u8 = 3;

/// What a battle button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// An action menu cell (FIGHT 0, BAG 1, POKéMON 2, RUN 3).
    Action(u8),
    /// A move, 0..4 (opening FIGHT first if the action menu is up).
    Move(u8),
    /// The party Pokémon with this personality.
    Switch(u32),
    /// B: out of the move menu.
    Back,
}

/// Where a game keeps what the driver reads (NativeReader.kt's NativeConfig; nm's even addresses, the Thumb bit
/// masked off when compared).
#[derive(Debug, Clone, Copy)]
pub struct Addrs {
    /// gBattlerControllerFuncs[4] (IWRAM).
    funcs: u32,
    /// gMain.inBattle's byte (bit 1).
    in_battle: u32,
    choose_action: u32,
    choose_move: u32,
    /// CompleteWhenChoseItem / WaitForMonSelection: the bag / the party menu up.
    chose_item: u32,
    mon_selection: u32,
    choose_target: u32,
    /// gActionSelectionCursor / gMoveSelectionCursor (a byte per battler), where checked; else the blind walk.
    cursors: Option<(u32, u32)>,
    /// gPartyMenu and gPlayerParty, for a switch, where checked.
    switch: Option<(u32, u32)>,
}

impl Addrs {
    /// The game's, for the retail games the companion reads (by header code and revision): FireRed, LeafGreen and
    /// Emerald (English), as the Android app has them.
    pub fn for_game(code: &str, revision: u8) -> Option<Addrs> {
        const FR_REV0: Addrs = Addrs {
            funcs: 0x0300_4FE0,
            in_battle: 0x0300_30F0 + 0x439,
            choose_action: 0x0802_E438,
            choose_move: 0x0802_EA10,
            chose_item: 0x0803_073C,
            mon_selection: 0x0803_0684,
            choose_target: 0x0802_E674,
            cursors: None,
            switch: None,
        };
        const FR_REV1: Addrs = Addrs {
            choose_action: 0x0802_E44C,
            choose_move: 0x0802_EA24,
            chose_item: 0x0803_0750,
            mon_selection: 0x0803_0698,
            choose_target: 0x0802_E688,
            ..FR_REV0
        };
        match (code, revision) {
            ("BPRE", 0) => Some(FR_REV0),
            ("BPRE", 1) => Some(Addrs {
                cursors: Some((0x0202_3FF8, 0x0202_3FFC)),
                switch: Some((0x0203_B0A0, 0x0202_4284)),
                ..FR_REV1
            }),
            // LeafGreen: FireRed's (the Android app's copy of its config), without the cursors and the switch, which
            // haven't been checked on it.
            ("BPGE", 0) => Some(FR_REV0),
            ("BPGE", 1) => Some(FR_REV1),
            ("BPEE", 0) => Some(Addrs {
                funcs: 0x0300_5D60,
                in_battle: 0x0300_22C0 + 0x439,
                choose_action: 0x0805_7588,
                choose_move: 0x0805_7BFC,
                chose_item: 0x0805_98E0,
                mon_selection: 0x0805_9828,
                choose_target: 0x0805_7824,
                cursors: Some((0x0202_44AC, 0x0202_44B0)),
                switch: Some((0x0203_CEC8, 0x0202_44EC)),
            }),
            _ => None,
        }
    }

    pub fn can_switch(&self) -> bool {
        self.switch.is_some()
    }
}

/// (The battler whose menu is up, the menu) as the game has it now: [NONE] out of battle, [BUSY] when none of the
/// battlers waits on the player. The whole array is scanned: gActiveBattler is a stale loop variable outside the
/// battle's own dispatch loop.
pub fn read_state(core: &mut Core, a: &Addrs) -> (usize, u8) {
    let mut b = [0u8; 1];
    core.read(a.in_battle, &mut b);
    if b[0] & 0x02 == 0 {
        return (0, NONE);
    }
    let mut raw = [0u8; 16];
    core.read(a.funcs, &mut raw);
    let thumb = |v: u32| v & !1;
    for i in 0..4 {
        let f = thumb(u32::from_le_bytes(
            raw[i * 4..i * 4 + 4].try_into().unwrap(),
        ));
        let state = match f {
            _ if f == thumb(a.choose_action) => ACTION_SELECT,
            _ if f == thumb(a.choose_move) => MOVE_SELECT,
            _ if f == thumb(a.chose_item) => BAG_OPEN,
            _ if f == thumb(a.mon_selection) => PARTY_OPEN,
            _ if f == thumb(a.choose_target) => TARGET_SELECT,
            _ => continue,
        };
        return (i, state);
    }
    (0, BUSY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwitchPhase {
    WaitMenu,
    Navigate,
    Confirm,
    WaitClose,
}

#[derive(Debug, Clone, Copy)]
struct Switch {
    target: u32,
    phase: SwitchPhase,
    frames: u32,
    presses: u32,
    last_slot: i32,
}

// BattleInputController.kt's timings (headless on FireRed and Emerald there).
const PARTY_MENU_SLOT_ID: u32 = 9; // struct PartyMenu.slotId, both games
const PARTY_MENU_CANCEL: i32 = 7; // slotId on CANCEL (after the last mon)
const MON_SIZE: u32 = 100;
const MENU_TIMEOUT_FRAMES: u32 = 240; // the party menu's fade-in drops presses for ~50 frames
const CURSOR_FRAMES: u32 = 3; // after each press, before reading the cursor again
const MAX_PICK_PRESSES: u32 = 8; // a 2x2 grid needs two; the rest re-press dropped ones
const MAX_CURSOR_PRESSES: u32 = 16; // two laps of six slots + CANCEL, then give up
const SUBMENU_FRAMES: u32 = 8; // SHIFT / SUMMARY / CANCEL opening
const CLOSE_TIMEOUT_FRAMES: u32 = 240;
const SETTLE_FRAMES: u32 = 20; // the blind walk's gap: a cursor move starts a palette fade that drops input
const RUN_B_PRESSES: u32 = 6;
const RUN_FIRST_GAP: i32 = 45; // the escape message starts printing
const RUN_B_GAP: i32 = 30;
const HOLD_TAIL_FRAMES: u32 = 12;
const WATCH_TIMEOUT_FRAMES: u32 = 300; // a target screen / FIGHT that hasn't shown by then won't
const EXCLUSIVE_TIMEOUT_FRAMES: u32 = 1200; // a switch's worst case is ~800

/// One game's battle buttons, run on the emulator thread: [BattleDriver::pick] from the buttons, [BattleDriver::tick]
/// once per emulated frame before the keys go in.
pub struct BattleDriver {
    a: Addrs,
    queue: VecDeque<(u32, u32)>,
    current: u32,
    frames_left: u32,
    pending_move: Option<u8>,
    state: u8,
    battler: usize,
    pick_cell: Option<u8>,
    pick_move: bool,
    pick_presses: u32,
    awaiting_target: bool,
    switch: Option<Switch>,
    run_presses: u32,
    run_gap: i32,
    run_left_menu: bool,
    watch_frames: u32,
    exclusive_frames: u32,
    hold_tail: u32,
}

fn col(i: u8) -> u8 {
    i & 1
}

fn row(i: u8) -> u8 {
    (i >> 1) & 1
}

impl BattleDriver {
    pub fn new(a: Addrs) -> BattleDriver {
        BattleDriver {
            a,
            queue: VecDeque::new(),
            current: 0,
            frames_left: 0,
            pending_move: None,
            state: NONE,
            battler: 0,
            pick_cell: None,
            pick_move: false,
            pick_presses: 0,
            awaiting_target: false,
            switch: None,
            run_presses: 0,
            run_gap: 0,
            run_left_menu: false,
            watch_frames: 0,
            exclusive_frames: 0,
            hold_tail: 0,
        }
    }

    pub fn addrs(&self) -> &Addrs {
        &self.a
    }

    /// The battle's menu as last read ([NONE], [ACTION_SELECT]...).
    pub fn state(&self) -> u8 {
        self.state
    }

    /// A pick is on its way: the buttons wait for it.
    pub fn busy(&self) -> bool {
        self.exclusive() || self.pending_move.is_some() || self.awaiting_target
    }

    /// The screen keeps the battle's last frame: a switch drives the game's party menu, faded in from black.
    pub fn hold_frame(&self) -> bool {
        self.switch.is_some() || self.hold_tail > 0
    }

    /// The player's own keys are shut out: presses queued or playing, or a pick / switch / RUN steering. Not during
    /// the passive watches (a target screen that may follow a move): the turn can stop on a message waiting for the
    /// player's A there (a level up, "learn a new move?").
    fn exclusive(&self) -> bool {
        !self.queue.is_empty()
            || self.frames_left > 0
            || self.switch.is_some()
            || self.run_presses > 0
            || self.pick_cell.is_some()
    }

    pub fn pick(&mut self, p: Pick) {
        match p {
            Pick::Action(i) => {
                self.pending_move = None;
                self.pick_cell(i & 3, false);
                if i == ACTION_RUN {
                    // "Got away safely!" waits for a button: B closes it (one while it's printing just finishes it).
                    self.run_presses = RUN_B_PRESSES;
                    self.run_gap = RUN_FIRST_GAP;
                    self.run_left_menu = false;
                }
            }
            Pick::Move(i) => {
                if self.state == ACTION_SELECT {
                    // FIGHT first; the move once its menu is up (on_state).
                    self.pending_move = Some(i & 3);
                    self.pick_cell(ACTION_FIGHT, false);
                } else {
                    self.pending_move = None;
                    self.queue_move(i & 3);
                }
            }
            Pick::Switch(personality) => {
                if self.a.switch.is_none() || personality == 0 {
                    return;
                }
                self.pending_move = None;
                if self.state == ACTION_SELECT {
                    self.pick_cell(ACTION_POKEMON, false);
                }
                self.switch = Some(Switch {
                    target: personality,
                    phase: SwitchPhase::WaitMenu,
                    frames: 0,
                    presses: 0,
                    last_slot: -1,
                });
            }
            Pick::Back => {
                self.pending_move = None;
                self.press(key::B);
            }
        }
    }

    pub fn cancel(&mut self) {
        self.queue.clear();
        self.pick_cell = None;
        self.pending_move = None;
        self.awaiting_target = false;
        self.switch = None;
        self.run_presses = 0;
        self.current = 0;
        self.frames_left = 0;
        self.watch_frames = 0;
        self.exclusive_frames = 0;
        self.hold_tail = 0;
    }

    /// Once per emulated frame, before the keys go in: reads the battle's menu, steps what's on its way. Returns the
    /// buttons to press and whether they replace the player's.
    pub fn tick(&mut self, core: &mut Core) -> (u32, bool) {
        let (battler, state) = read_state(core, &self.a);
        self.on_state(battler, state);
        if self.frames_left == 0 && self.queue.is_empty() {
            self.drive_pick(core);
            if self.queue.is_empty() {
                self.drive_switch(core);
                self.drive_run();
            }
        }
        if self.frames_left == 0 {
            let (bits, frames) = self.queue.pop_front().unwrap_or((0, 0));
            self.current = bits;
            self.frames_left = frames;
        }
        self.frames_left = self.frames_left.saturating_sub(1);
        // A target screen follows a move's A within a few frames, and FIGHT opens as fast: an unanswered watch is stale.
        if self.queue.is_empty()
            && self.frames_left == 0
            && (self.awaiting_target || self.pending_move.is_some())
        {
            self.watch_frames += 1;
            if self.watch_frames > WATCH_TIMEOUT_FRAMES {
                self.awaiting_target = false;
                self.pending_move = None;
                self.watch_frames = 0;
            }
        } else {
            self.watch_frames = 0;
        }
        // Every sequence is bounded on its own; this is the backstop.
        if self.exclusive() {
            self.exclusive_frames += 1;
            if self.exclusive_frames > EXCLUSIVE_TIMEOUT_FRAMES {
                self.cancel();
            }
        } else {
            self.exclusive_frames = 0;
        }
        if self.switch.is_none() {
            self.hold_tail = self.hold_tail.saturating_sub(1);
        }
        (self.current, self.exclusive())
    }

    fn on_state(&mut self, battler: usize, state: u8) {
        self.state = state;
        self.battler = battler.min(3);
        // A pick still steering is still on its menu.
        if self.pick_cell.is_some() {
            return;
        }
        if self.awaiting_target && self.queue.is_empty() && self.frames_left == 0 {
            match state {
                // A move that hits "the user or one chosen" opens a target screen even in a single battle: accept
                // the game's own default target.
                TARGET_SELECT => {
                    self.awaiting_target = false;
                    self.press(key::A);
                }
                BUSY => {}
                _ => self.awaiting_target = false,
            }
        }
        let Some(pending) = self.pending_move else {
            return;
        };
        // Doubles aren't handled: a pending move for battler 0 is dropped if another's menu came up.
        if battler != 0 {
            self.pending_move = None;
            return;
        }
        if state == MOVE_SELECT && self.queue.is_empty() && self.frames_left == 0 {
            self.pending_move = None;
            self.queue_move(pending);
        }
    }

    fn queue_move(&mut self, i: u8) {
        self.pick_cell(i, true);
        self.awaiting_target = true;
    }

    fn pick_cell(&mut self, i: u8, mv: bool) {
        if self.a.cursors.is_none() {
            return self.queue_cell(col(i), row(i));
        }
        self.pick_cell = Some(i);
        self.pick_move = mv;
        self.pick_presses = 0;
    }

    /// The blind walk: LEFT and UP (no-ops if already there), RIGHT / DOWN if the cell needs them, then A, each with
    /// a gap (a cursor move starts a palette fade some games drop input under).
    fn queue_cell(&mut self, c: u8, r: u8) {
        self.press_and_settle(key::LEFT);
        self.press_and_settle(key::UP);
        if c == 1 {
            self.press_and_settle(key::RIGHT);
        }
        if r == 1 {
            self.press_and_settle(key::DOWN);
        }
        self.press(key::A);
    }

    /// One step of a closed-loop pick: on the cell, A; else the press toward it (a dropped one is pressed again).
    fn drive_pick(&mut self, core: &mut Core) {
        let Some(cell) = self.pick_cell else { return };
        let Some((action, mv)) = self.a.cursors else {
            return self.pick_fallback();
        };
        let mut b = [0u8; 1];
        core.read(
            if self.pick_move { mv } else { action } + self.battler as u32,
            &mut b,
        );
        let cur = b[0];
        if cur > 3 {
            return self.pick_fallback();
        }
        if cur == cell {
            self.pick_cell = None;
            self.press(key::A);
            return;
        }
        self.pick_presses += 1;
        if self.pick_presses > MAX_PICK_PRESSES {
            return self.pick_fallback();
        }
        self.press(if col(cur) != col(cell) {
            if col(cell) == 1 {
                key::RIGHT
            } else {
                key::LEFT
            }
        } else if row(cell) == 1 {
            key::DOWN
        } else {
            key::UP
        });
    }

    fn pick_fallback(&mut self) {
        if let Some(cell) = self.pick_cell.take() {
            self.queue_cell(col(cell), row(cell));
        }
    }

    fn drive_switch(&mut self, core: &mut Core) {
        let Some(mut sw) = self.switch else { return };
        let Some((menu, party)) = self.a.switch else {
            return self.end_switch();
        };
        sw.frames += 1;
        match sw.phase {
            // No fixed settle: presses the menu drops while it fades in are pressed again.
            SwitchPhase::WaitMenu => {
                if self.state == PARTY_OPEN {
                    sw.phase = SwitchPhase::Navigate;
                    sw.frames = 0;
                    sw.last_slot = -1;
                } else if sw.frames > MENU_TIMEOUT_FRAMES {
                    return self.end_switch();
                }
            }
            SwitchPhase::Navigate => {
                let mut b = [0u8; 1];
                core.read(menu + PARTY_MENU_SLOT_ID, &mut b);
                let slot = b[0] as i8 as i32;
                let pids = party_pids(core, party);
                let Some(want) = pids.iter().position(|&p| p == sw.target) else {
                    return self.end_switch();
                };
                let want = want as i32;
                // Steps the cursor really took (dropped presses don't count).
                if slot != sw.last_slot {
                    if sw.last_slot >= 0 {
                        sw.presses += 1;
                    }
                    sw.last_slot = slot;
                }
                if slot == want {
                    self.press(key::A); // the mon: its SHIFT / SUMMARY / CANCEL menu
                    self.wait(SUBMENU_FRAMES);
                    sw.phase = SwitchPhase::Confirm;
                } else if sw.presses > MAX_CURSOR_PRESSES || sw.frames > MENU_TIMEOUT_FRAMES {
                    return self.end_switch();
                } else {
                    let count = pids.iter().filter(|&&p| p != 0).count().max(1);
                    self.press(list_key(slot, want, count));
                    self.wait(CURSOR_FRAMES);
                }
            }
            SwitchPhase::Confirm => {
                self.press(key::A); // SHIFT, the menu's first entry
                sw.phase = SwitchPhase::WaitClose;
                sw.frames = 0;
            }
            // The player's keys stay out until the game has taken it.
            SwitchPhase::WaitClose => {
                if self.state != PARTY_OPEN || sw.frames > CLOSE_TIMEOUT_FRAMES {
                    return self.end_switch();
                }
            }
        }
        self.switch = Some(sw);
    }

    fn end_switch(&mut self) {
        self.switch = None;
        self.hold_tail = HOLD_TAIL_FRAMES;
    }

    fn drive_run(&mut self) {
        // RUN's own pick (to the cell, then A) goes first.
        if self.run_presses == 0 || self.pick_cell.is_some() {
            return;
        }
        if self.state == NONE || (self.run_left_menu && self.state == ACTION_SELECT) {
            // Got away (the battle's over), or back on the menu (couldn't escape).
            self.run_presses = 0;
            return;
        }
        if self.state != ACTION_SELECT {
            self.run_left_menu = true;
        }
        self.run_gap -= 1;
        if self.run_gap <= 0 {
            self.press(key::B);
            self.run_presses -= 1;
            self.run_gap = RUN_B_GAP;
        }
    }

    /// One frame pressed, one released: the games act on a button going down, so presses in a row need a release.
    fn press(&mut self, bit: u32) {
        self.queue.push_back((bit, 1));
        self.queue.push_back((0, 1));
    }

    fn press_and_settle(&mut self, bit: u32) {
        self.press(bit);
        self.wait(SETTLE_FRAMES);
    }

    fn wait(&mut self, frames: u32) {
        self.queue.push_back((0, frames));
    }
}

/// gPlayerParty's personalities (the battle party menu's order while it's open), 0 for an empty slot.
fn party_pids(core: &mut Core, party: u32) -> [u32; 6] {
    let mut raw = [0u8; 6 * MON_SIZE as usize];
    core.read(party, &mut raw);
    std::array::from_fn(|i| {
        let o = i * MON_SIZE as usize;
        u32::from_le_bytes(raw[o..o + 4].try_into().unwrap())
    })
}

/// DOWN or UP, the shorter way from `slot` to `want` round the party list (`count` mons, then CANCEL; both wrap).
fn list_key(slot: i32, want: i32, count: usize) -> u32 {
    let ring: Vec<i32> = (0..count as i32).chain([PARTY_MENU_CANCEL]).collect();
    let (i, j) = (
        ring.iter().position(|&s| s == slot),
        ring.iter().position(|&s| s == want),
    );
    match (i, j) {
        (Some(i), Some(j)) => {
            let down = (j + ring.len() - i) % ring.len();
            if down <= ring.len() - down {
                key::DOWN
            } else {
                key::UP
            }
        }
        _ => key::DOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_party_list_goes_the_short_way_round() {
        // Six mons, then CANCEL: 0 -> 5 is shorter up (through CANCEL).
        assert_eq!(list_key(0, 1, 6), key::DOWN);
        assert_eq!(list_key(0, 5, 6), key::UP);
        assert_eq!(list_key(PARTY_MENU_CANCEL, 0, 6), key::DOWN);
        assert_eq!(list_key(3, 1, 4), key::UP);
    }

    /// The battle buttons in a real battle, headless: a switch to the next Pokémon that can fight, a move (its PP goes
    /// down by one), then RUN, each pressed in by the driver alone. A state (or save) in a battle:
    /// `ADVENTUREPAD_TEST_STATE=<state>` (copied), `ADVENTUREPAD_TEST_ROM=<rom>`
    /// `cargo test -p adventurepad battle_buttons -- --ignored`.
    #[test]
    #[ignore]
    fn battle_buttons_drive_a_real_battle() {
        let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(std::path::PathBuf::from)
        else {
            return;
        };
        let bytes = std::fs::read(&rom).unwrap();
        let code = String::from_utf8_lossy(&bytes[0xAC..0xB0]).into_owned();
        let Some(a) = Addrs::for_game(&code, bytes[0xBC]) else {
            eprintln!("no battle buttons for {code}");
            return;
        };
        let dir = std::env::temp_dir().join(format!("adventurepad-battle-{}", std::process::id()));
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
        // gBattleMons (Emerald / FireRed): battler 0's species (+0), moves' PP (+0x24).
        let battle_mons = if code == "BPEE" {
            0x0202_4084
        } else {
            0x0202_3BE4
        };
        let mut d = BattleDriver::new(a);
        let mut audio = vec![0i16; 8192];
        let mut f = 0u32;
        // Runs the game (B now and then through the battle's messages, as a player would) until `done`, or None.
        let mut run = |core: &mut Core,
                       d: &mut BattleDriver,
                       limit: u32,
                       done: &dyn Fn(&mut Core, &BattleDriver) -> bool| {
            for _ in 0..limit {
                let (bits, exclusive) = d.tick(core);
                // Walking about out of battle (grass brings one), B through its messages in one.
                let player = match d.state() {
                    NONE => {
                        [key::LEFT, key::RIGHT, key::UP, key::DOWN][(f / 40 % 4) as usize]
                            | if f % 300 < 2 { key::B } else { 0 }
                    }
                    BUSY if !d.busy() && f % 24 < 2 => key::B,
                    _ => 0,
                };
                core.set_keys(if exclusive { bits } else { bits | player });
                core.run_frame();
                core.read_audio(&mut audio);
                f += 1;
                if done(core, d) {
                    return true;
                }
            }
            false
        };
        let at_menu = |_: &mut Core, d: &BattleDriver| d.state() == ACTION_SELECT && !d.busy();
        if !run(&mut core, &mut d, 20000, &at_menu) {
            eprintln!("no battle menu came up: nothing to check");
            return;
        }
        let read16 = |core: &mut Core, addr: u32| {
            let mut b = [0u8; 2];
            core.read(addr, &mut b);
            u16::from_le_bytes(b)
        };
        // A switch, to the first other party Pokémon that can fight.
        if let Some((_, party)) = a.switch {
            let pids = party_pids(&mut core, party);
            let out = read16(&mut core, battle_mons);
            eprintln!(
                "party: {} Pokémon",
                pids.iter().filter(|&&p| p != 0).count()
            );
            let next = (0..6).find(|&i| {
                let base = party + i as u32 * MON_SIZE;
                let mut hp = [0u8; 2];
                core.read(base + 0x56, &mut hp);
                pids[i] != 0 && u16::from_le_bytes(hp) > 0 && i != 0
            });
            if let Some(i) = next {
                d.pick(Pick::Switch(pids[i]));
                assert!(d.hold_frame(), "the frame holds through the switch");
                assert!(
                    run(&mut core, &mut d, 1500, &|_, d| !d.hold_frame()),
                    "the switch ends"
                );
                assert!(run(&mut core, &mut d, 3000, &at_menu), "back to the menu");
                let now = read16(&mut core, battle_mons);
                eprintln!("switched: species {out} -> {now}");
                assert_ne!(now, out, "another Pokémon is out");
            }
        }
        // A move: the first with PP; its PP goes down by one.
        let mut pp = [0u8; 4];
        core.read(battle_mons + 0x24, &mut pp);
        let mv = (0..4).find(|&k| pp[k] > 0).expect("a move with PP") as u8;
        d.pick(Pick::Move(mv));
        assert!(
            run(&mut core, &mut d, 600, &|_, d| d.state() == BUSY
                && !d.busy()),
            "the move went in"
        );
        let mut after = [0u8; 4];
        core.read(battle_mons + 0x24, &mut after);
        eprintln!(
            "move {mv}: PP {} -> {}",
            pp[mv as usize], after[mv as usize]
        );
        assert_eq!(
            after[mv as usize] + 1,
            pp[mv as usize],
            "move {mv} was used"
        );
        // RUN (a wild battle: it ends; a trainer's: back on the menu).
        if run(&mut core, &mut d, 3000, &at_menu) {
            d.pick(Pick::Action(ACTION_RUN));
            let left = run(&mut core, &mut d, 1500, &|_, d| d.state() == NONE);
            eprintln!("RUN: battle over = {left}");
        }
        assert!(!d.busy(), "nothing left steering");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_games_only() {
        assert!(Addrs::for_game("BPRE", 1).unwrap().can_switch());
        assert!(!Addrs::for_game("BPRE", 0).unwrap().can_switch());
        assert!(Addrs::for_game("BPEE", 0).unwrap().cursors.is_some());
        assert!(Addrs::for_game("BPGE", 1).unwrap().cursors.is_none());
        assert!(Addrs::for_game("AXVE", 0).is_none());
    }
}
