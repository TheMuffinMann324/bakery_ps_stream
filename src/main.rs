// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod dpapi;
mod ffi;
mod input;
mod logo;
mod remote;
mod theme;
mod video;

fn main() -> eframe::Result {
    if let Err(e) = remote::init() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("PS Stream")
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([900.0, 560.0])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "PS Stream",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)?))),
    )
}
