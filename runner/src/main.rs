#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! eDiscovery Search Runner: the administrator's side of the request builder. Opens a `.req`
//! request file, signs the admin in to Microsoft 365, and creates and estimates the search in
//! Microsoft Purview, recording every step in a tamper-evident audit log.

mod app;
mod audit;
mod auth;
mod graph;
mod mirror;
mod plan;
mod purview;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("eDiscovery Search Runner")
            .with_app_id("keyql-runner")
            .with_inner_size([1200.0, 880.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "eDiscovery Search Runner",
        options,
        Box::new(|cc| Ok(Box::new(app::RunnerApp::new(cc)))),
    )
}
