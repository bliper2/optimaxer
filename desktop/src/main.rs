#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod net;
mod opti;
mod opti_ui;
mod shots;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Orion Store + Optimaxer")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("Orion Store", options, Box::new(|cc| Ok(Box::new(app::OrionApp::new(cc)))))
}
