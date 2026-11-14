// No console window behind the app in a release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod battle_input;
mod controls;
mod crash;
mod emu;
mod hud;
mod music;
mod panels;
mod saves;
mod settings;
mod wide;
mod wsola;

/// The window's icon (the exe's is linked in by build.rs): assets/icon.png, from rust/tools/make_icon.py.
fn icon() -> eframe::egui::IconData {
    let decoder = png::Decoder::new(&include_bytes!("../assets/icon.png")[..]);
    let mut reader = decoder.read_info().expect("the icon is a PNG");
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).expect("the icon decodes");
    rgba.truncate(info.buffer_size());
    eframe::egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}

#[cfg(test)]
#[test]
fn the_icon_decodes() {
    let i = icon();
    assert_eq!((i.width, i.height), (256, 256));
    assert_eq!(i.rgba.len(), 256 * 256 * 4);
}

fn main() -> eframe::Result<()> {
    crash::install();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Adventure Pad")
            .with_inner_size([1400.0, 760.0])
            .with_min_inner_size([900.0, 520.0])
            .with_icon(icon()),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "AdventurePad",
        options,
        Box::new(|cc| Ok(Box::new(app::AdventurePadApp::new(cc)))),
    )
}
