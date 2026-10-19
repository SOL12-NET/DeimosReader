#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
mod cache;
mod pdf_engine;
mod tab;
mod worker;

use std::path::PathBuf;

use app::{load_mars_icon_data, DeimosApp};
use eframe::egui;

fn main() -> eframe::Result<()> {
    let initial_files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Deimos Reader")
        .with_app_id("DeimosReader")
        .with_inner_size([1280.0, 860.0])
        .with_min_inner_size([640.0, 480.0])
        .with_drag_and_drop(true);

    if let Some(icon) = load_mars_icon_data() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Deimos Reader",
        native_options,
        Box::new(move |cc| Ok(Box::new(DeimosApp::new(cc, initial_files)))),
    )
}
