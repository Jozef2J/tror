#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod app;
mod text;
mod bench;
mod console;
mod crash;
mod db;
mod plugins;
mod instance;
mod remote;
mod trace;
mod update;
mod visual;
mod widgets;
mod theme;
mod dock;
mod prefs;
mod keys;
mod preview;
mod diff;
mod files;
mod findfiles;

use eframe::egui;

pub static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() -> eframe::Result {
    STARTED.get_or_init(std::time::Instant::now);
    if matches!(std::env::args().nth(1).as_deref(), Some("--version" | "-V")) {
        println!("trorpad {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--bench") {
        std::process::exit(bench::run());
    }
    if std::env::args().nth(1).as_deref() == Some("--forget-passwords") {
        remote::vault::clear();
        return Ok(());
    }
    crash::install();
    #[cfg(debug_assertions)]
    if std::env::var_os("TROR_TEST_PANIC").is_some() {
        std::thread::spawn(|| panic!("test panic")).join().ok();
    }
    if !instance::restarted() && instance::forward(&instance::args()) {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TrorPad")
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
