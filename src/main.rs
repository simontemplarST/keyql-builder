#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod query;

fn main() -> eframe::Result {
    let example = std::env::args().any(|a| a == "--example");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("eDiscovery Search Request")
            .with_app_id("keyql-builder")
            .with_inner_size([1200.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "eDiscovery Search Request",
        options,
        Box::new(|cc| Ok(Box::new(app::KeyqlApp::new(cc, example)))),
    )
}
