//! The fault guard (Windows): a deliberate crash inside it is caught, a guarded core it happens in is left dead
//! (and harmless), and the guard keeps working after.

use mgba_sys::Core;

#[cfg(windows)]
const ACCESS_VIOLATION: u32 = 0xC000_0005;

#[test]
#[cfg(windows)]
fn a_fault_is_caught_again_and_again_on_any_thread() {
    for _ in 0..3 {
        assert_eq!(Core::guard_selftest(), ACCESS_VIOLATION);
    }
    let on_other_threads: Vec<u32> = (0..4)
        .map(|_| std::thread::spawn(Core::guard_selftest))
        .map(|t| t.join().unwrap())
        .collect();
    assert!(
        on_other_threads.iter().all(|&c| c == ACCESS_VIOLATION),
        "{on_other_threads:x?}"
    );
}

/// A guarded core that faults is dead: its frames do nothing, its calls fail, and dropping it is safe.
/// Needs a ROM: `ADVENTUREPAD_TEST_ROM=<rom> cargo test -p mgba-sys -- --ignored`.
#[test]
#[ignore]
#[cfg(windows)]
fn a_dead_core_stays_harmless() {
    let Some(rom) = std::env::var_os("ADVENTUREPAD_TEST_ROM").map(std::path::PathBuf::from) else {
        return;
    };
    let mut core = Core::open(&rom, None).unwrap();
    core.set_guarded(true);
    for _ in 0..30 {
        core.run_frame();
    }
    assert_eq!(core.dead(), None, "a working core isn't dead");
    core.test_fault();
    assert_eq!(core.dead(), Some(ACCESS_VIOLATION));
    for _ in 0..30 {
        core.run_frame();
    }
    assert_eq!(
        core.call(0x0800_0000, [0; 4]),
        None,
        "a dead core's calls fail"
    );
    let state = Core::open(&rom, None).unwrap().state_bytes().unwrap();
    assert!(!core.load_state_bytes(&state), "nor does it load states");
    drop(core);
    // The process is fine, and a new core runs.
    let mut fresh = Core::open(&rom, None).unwrap();
    fresh.run_frame();
    assert_eq!(fresh.dead(), None);
}
