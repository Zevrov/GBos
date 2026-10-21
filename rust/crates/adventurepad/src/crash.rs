//! When something goes wrong: every panic and every native crash goes to `crash.log` in the app's folder, and a
//! crash that closed the app is mentioned on the next launch. A panic the app catches and carries on from (a
//! panel's, the emulator thread's: [`caught`]) is logged without that.

use std::cell::Cell;
use std::io::Write;
use std::panic::UnwindSafe;
use std::path::PathBuf;

thread_local! {
    /// How deep this thread is in [`caught`] calls: a panic there is handled, not the app closing.
    static CAUGHT: Cell<u32> = const { Cell::new(0) };
}

fn log_path() -> PathBuf {
    crate::settings::home().join("crash.log")
}

/// Left by a crash that closed the app; the next launch finds it.
fn flag_path() -> PathBuf {
    crate::settings::home().join("crash.new")
}

/// [`std::panic::catch_unwind`] for a panic the app recovers from: still logged, but not a crash.
pub fn caught<R>(f: impl FnOnce() -> R + UnwindSafe) -> std::thread::Result<R> {
    CAUGHT.with(|c| c.set(c.get() + 1));
    let r = std::panic::catch_unwind(f);
    CAUGHT.with(|c| c.set(c.get() - 1));
    r
}

/// The text of a caught panic.
pub fn message(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| e.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "it crashed".into())
}

/// Logs panics and native crashes from here on.
pub fn install() {
    let (log, flag) = (log_path(), flag_path());
    mgba_sys::install_crash_log(&log, &flag);
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let caught = CAUGHT.with(|c| c.get()) > 0;
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            let _ = writeln!(
                f,
                "{} Adventure Pad {}: panic in thread '{thread}'{}: {info}\n{}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                env!("CARGO_PKG_VERSION"),
                if caught {
                    " (caught: the app went on)"
                } else {
                    ""
                },
                std::backtrace::Backtrace::force_capture()
            );
        }
        if !caught {
            let _ = std::fs::write(&flag, b"");
        }
        default(info);
    }));
}

/// The log, if the app closed on a crash last time (once: the next launch won't say it again).
pub fn crashed_last_time() -> Option<PathBuf> {
    let flag = flag_path();
    if !flag.exists() {
        return None;
    }
    let _ = std::fs::remove_file(flag);
    Some(log_path())
}
