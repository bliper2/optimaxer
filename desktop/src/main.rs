#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod appearance;
mod kit;
mod model;
mod net;
mod notifications;
mod opti;
mod opti_ui;
mod shots;
mod software_ui;
mod theme;
mod update;
mod winget;
mod winget_icons;

/// 1180 x 760 normally; `ORION_SHOTS_SIZE=760x520` tests the minimum layout and `1440x900` tests a wide layout.
fn window_size() -> [f32; 2] {
    std::env::var("ORION_SHOTS_SIZE")
        .ok()
        .and_then(|v| {
            v.split_once('x')
                .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))
        })
        .unwrap_or([1180.0, 760.0])
}

fn window_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../../assets/logo.png"))
        .expect("assets/logo.png")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(window_icon())
            .with_title("Orion Store + Optimaxer")
            // no system title bar: the app draws its own caption buttons; the screenshot run keeps the frame because wgpu
            // fails to allocate while capturing an undecorated window
            .with_decorations(std::env::var_os("ORION_SHOTS").is_some())
            .with_inner_size(window_size())
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Orion Store",
        options,
        Box::new(|cc| Ok(Box::new(app::OrionApp::new(cc)))),
    )
}
