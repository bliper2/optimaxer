#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod kit;
mod model;
mod net;
mod opti;
mod opti_ui;
mod shots;
mod theme;
mod update;

/// 1180 x 760 normally; `ORION_SHOTS_SIZE=1440x900` sets another size for the developer screenshot run.
fn window_size() -> [f32; 2] {
    std::env::var("ORION_SHOTS_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?])))
        .unwrap_or([1180.0, 760.0])
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Orion Store + Optimaxer")
            .with_inner_size(window_size())
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("Orion Store", options, Box::new(|cc| Ok(Box::new(app::OrionApp::new(cc)))))
}
