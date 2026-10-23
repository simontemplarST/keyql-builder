#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod query;

fn main() -> eframe::Result {
    let example = std::env::args().any(|a| a == "--example");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Purview KeyQL Builder")
            .with_app_id("keyql-builder")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Purview KeyQL Builder",
        options,
        Box::new(|cc| Ok(Box::new(app::KeyqlApp::new(cc, example)))),
    )
}
