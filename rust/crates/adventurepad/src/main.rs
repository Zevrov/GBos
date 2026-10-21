// No console window behind the app in a release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod controls;
mod crash;
mod emu;
mod music;
mod panels;
mod saves;
mod settings;
mod wide;
mod wsola;

fn main() -> eframe::Result<()> {
    crash::install();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Adventure Pad")
            .with_inner_size([1400.0, 760.0])
            .with_min_inner_size([900.0, 520.0]),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "AdventurePad",
        options,
        Box::new(|cc| Ok(Box::new(app::AdventurePadApp::new(cc)))),
    )
}
