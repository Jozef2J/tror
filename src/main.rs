#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod complete;
mod document;
mod editor;
mod highlight;
mod remote;
mod remote_ui;
mod search;
mod theme;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Tror")
            .with_app_id("tror")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([480.0, 320.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Tror",
        options,
        Box::new(|cc| Ok(Box::new(app::TrorApp::new(cc)))),
    )
}
