//! Meadow — a fast Minecraft terrain editor.

mod app;
mod dialogs;
mod jobs;
mod map2d;
mod panels;
mod settings;
mod view3d;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Meadow")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0]),
        renderer: eframe::Renderer::Wgpu,
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("Meadow", options, Box::new(|cc| Ok(Box::new(app::MeadowApp::new(cc)))))
}
