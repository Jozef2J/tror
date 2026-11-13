#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod complete;
mod console;
mod db;
mod diff;
mod dock;
mod document;
mod editor;
mod files;
mod findfiles;
mod highlight;
mod keys;
mod mosh;
mod plugins;
mod prefs;
mod preview;
mod recover;
mod remote;
mod remote_ui;
mod search;
mod sync;
mod telnet;
mod terminal;
mod theme;
mod trace;
mod update;
mod webview;
mod widgets;

use eframe::egui;

pub static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() -> eframe::Result {
    STARTED.get_or_init(std::time::Instant::now);
    #[cfg(all(target_os = "linux", feature = "webview"))]
    if std::env::var_os("DISPLAY").is_some() {
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Tror")
            .with_app_id("tror")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([480.0, 320.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/tror.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Tror",
        options,
        Box::new(|cc| Ok(Box::new(app::TrorApp::new(cc)))),
    )
}
