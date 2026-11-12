use eframe::egui;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use super::egui;
    use wry::dpi::{LogicalPosition, LogicalSize};

    pub const AVAILABLE: bool = true;

    #[derive(Default)]
    pub struct Embedded {
        view: Option<wry::WebView>,
        loaded: String,
        bounds: Option<[f32; 4]>,
        visible: bool,
        pub error: Option<String>,
    }

    fn rect(bounds: [f32; 4]) -> wry::Rect {
        wry::Rect {
            position: LogicalPosition::new(bounds[0] as f64, bounds[1] as f64).into(),
            size: LogicalSize::new(bounds[2] as f64, bounds[3] as f64).into(),
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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::egui;

    pub const AVAILABLE: bool = false;

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

pub use imp::{AVAILABLE, Embedded};
