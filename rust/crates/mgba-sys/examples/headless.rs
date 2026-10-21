//! Boots a ROM without a window: runs it, mashes START / A through the title screens, and prints what the
//! companion reads. `cargo run -p mgba-sys --example headless -- <rom> [save] [frames]`.

use companion::{tables::species_name, Bus, Game, Snapshot};
use mgba_sys::{key, Core};
use std::path::PathBuf;

struct CoreBus<'a>(&'a mut Core);

impl Bus for CoreBus<'_> {
    fn read(&mut self, addr: u32, out: &mut [u8]) -> bool {
        self.0.read(addr, out)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = PathBuf::from(&args[1]);
    let save = args.get(2).map(PathBuf::from);
    let frames: u32 = args.get(3).and_then(|f| f.parse().ok()).unwrap_or(3000);
    let bytes = std::fs::read(&rom).expect("rom");
    let game = Game::detect(&bytes[..0xC0], bytes.len());
    println!(
        "game: {} {} rev {} (companion: {})",
        game.title,
        game.code,
        game.revision,
        game.config.is_some()
    );
    let mut core = Core::open(&rom, save.as_deref()).expect("core");
    println!(
        "screen {}x{} {:?}",
        core.width(),
        core.height(),
        core.platform()
    );
    let mut audio = vec![0i16; 8192];
    let mut samples = 0usize;
    let start = std::time::Instant::now();
    for f in 0..frames {
        // START, then A, a few frames each, every second: through the intro and CONTINUE.
        let phase = f % 60;
        core.set_keys(if f > 600 && phase < 4 {
            if (f / 60) % 2 == 0 {
                key::START
            } else {
                key::A
            }
        } else {
            0
        });
        core.run_frame();
        samples += core.read_audio(&mut audio);
    }
    let el = start.elapsed().as_secs_f64();
    println!(
        "{frames} frames in {el:.2}s = {:.0} fps uncapped; {} audio samples",
        frames as f64 / el,
        samples
    );
    let lit = core
        .pixels()
        .chunks(4)
        .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 30)
        .count();
    println!("non-dark pixels: {lit}");
    match Snapshot::read(&game, &mut CoreBus(&mut core)) {
        Some(s) => {
            println!(
                "money {:?}, mapsec {}, in battle {}",
                s.money, s.mapsec, s.in_battle
            );
            for m in &s.party {
                println!(
                    "  {}{} ({}) Lv{} {}/{}",
                    m.nickname,
                    m.gender.symbol(),
                    species_name(m.species),
                    m.level,
                    m.hp,
                    m.max_hp
                );
            }
        }
        None => println!("no companion for this game"),
    }
    let state = std::env::temp_dir().join("adventurepad-headless.ss");
    println!(
        "finish save: {} frames; state saved: {}",
        core.finish_save(120),
        core.save_state(&state)
    );
    drop(core);
    println!("core closed");
}
