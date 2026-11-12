use eframe::egui;

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", feature = "webview")
))]
mod imp {
    use super::egui;
    use wry::dpi::{PhysicalPosition, PhysicalSize};

    pub const AVAILABLE: bool = true;

    #[cfg(target_os = "linux")]
    fn platform_ready() -> Result<(), String> {
        static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        INIT.get_or_init(|| gtk::init().map_err(|e| format!("GTK: {e}")))
            .clone()
    }

    #[cfg(not(target_os = "linux"))]
    fn platform_ready() -> Result<(), String> {
        Ok(())
    }

    pub fn pump(ctx: &egui::Context, active: bool) {
        #[cfg(target_os = "linux")]
        if active && platform_ready().is_ok() {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let _ = (ctx, active);
    }

    #[derive(Default)]
    pub struct Embedded {
        view: Option<wry::WebView>,
        loaded: String,
        bounds: Option<[f32; 4]>,
        built: [f32; 2],
        resized: Option<std::time::Instant>,
        visible: bool,
        pub error: Option<String>,
    }

    fn rect(bounds: [f32; 4]) -> wry::Rect {
        wry::Rect {
            position: PhysicalPosition::new(bounds[0] as i32, bounds[1] as i32).into(),
            size: PhysicalSize::new(bounds[2] as u32, bounds[3] as u32).into(),
        }
    }

    impl Embedded {
        pub fn show(&mut self, frame: &eframe::Frame, url: &str, area: egui::Rect, scale: f32) {
            let bounds = [
                (area.left() * scale).round(),
                (area.top() * scale).round(),
                (area.width() * scale).round().max(1.0),
                (area.height() * scale).round().max(1.0),
            ];
            if self.view.is_none() && self.error.is_none() {
                if let Err(e) = platform_ready() {
                    self.error = Some(e);
                    return;
                }
                let built = wry::WebViewBuilder::new()
                    .with_url(url)
                    .with_bounds(rect(bounds))
                    .with_focused(false)
                    .build_as_child(frame);
                match built {
                    Ok(view) => {
                        self.view = Some(view);
                        self.loaded = url.to_owned();
                        self.bounds = Some(bounds);
                        self.built = [bounds[2], bounds[3]];
                        self.resized = None;
                        self.visible = true;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
                return;
            }
            let Some(view) = &self.view else {
                return;
            };
            if self.loaded != url {
                let _ = view.load_url(url);
                self.loaded = url.to_owned();
            }
            if self.bounds != Some(bounds) {
                let _ = view.set_bounds(rect(bounds));
                self.bounds = Some(bounds);
            }
            if cfg!(target_os = "linux") && self.built != [bounds[2], bounds[3]] {
                let since = *self.resized.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() > std::time::Duration::from_millis(250) {
                    self.view = None;
                    self.resized = None;
                    return;
                }
            } else {
                self.resized = None;
            }
            if !self.visible {
                let _ = view.set_visible(true);
                self.visible = true;
            }
        }

        pub fn hide(&mut self) {
            if self.visible
                && let Some(view) = &self.view
            {
                let _ = view.set_visible(false);
                self.visible = false;
            }
        }

        pub fn reload(&self) {
            if let Some(view) = &self.view {
                let _ = view.reload();
            }
        }
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", feature = "webview")
)))]
mod imp {
    use super::egui;

    pub const AVAILABLE: bool = false;

    pub fn pump(_ctx: &egui::Context, _active: bool) {}

    #[derive(Default)]
    pub struct Embedded {
        pub error: Option<String>,
    }

    impl Embedded {
        pub fn show(&mut self, _frame: &eframe::Frame, _url: &str, _area: egui::Rect, _scale: f32) {
        }

        pub fn hide(&mut self) {}

        pub fn reload(&self) {}
    }
}

pub use imp::{AVAILABLE, Embedded, pump};
