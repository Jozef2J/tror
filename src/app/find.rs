use super::*;

const TITLE: &str = "Поиск — TrorPad";
const WIDTH: f32 = 450.0;
const FADED: u8 = 115;

#[derive(Default)]
pub(super) struct FindWindow {
    pub anchor: Option<egui::Pos2>,
    pub focus: bool,
    moved: bool,
    placed: Option<egui::Pos2>,
    height: f32,
    faded: Option<bool>,
    focused: bool,
    on_top: Option<bool>,
}

impl TrorApp {
    pub(super) fn find_window(
        &mut self,
        ctx: &egui::Context,
        pal: Palette,
        actions: &mut Vec<Action>,
    ) {
        let id = egui::ViewportId::from_hash_of("tror_find_window");
        let app_focused = ctx.input(|i| i.viewport().focused).unwrap_or(true) || self.find.focused;
        if !self.search.open {
            self.find.faded = None;
            self.find.focused = false;
            self.find.on_top = None;
            return;
        }
        if self.find.on_top != Some(app_focused) {
            self.find.on_top = Some(app_focused);
            let level = if app_focused {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            };
            ctx.send_viewport_cmd_to(id, ViewportCommand::WindowLevel(level));
        }
        let root = ctx.input(|i| i.viewport().inner_rect);
        if !self.find.moved
            && let (Some(anchor), Some(root)) = (self.find.anchor, root)
        {
            let pos = root.min + anchor.to_vec2() - egui::vec2(WIDTH, 0.0);
            if self.find.placed.is_none_or(|p| (p - pos).length() > 1.0) {
                if self.find.placed.is_some() {
                    ctx.send_viewport_cmd_to(id, ViewportCommand::OuterPosition(pos));
                }
                self.find.placed = Some(pos);
            }
        }
        let height = if self.find.height > 0.0 {
            self.find.height
        } else {
            86.0
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_inner_size([WIDTH, height])
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        if let Some(pos) = self.find.placed {
            builder = builder.with_position(pos);
        }
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
            self.find.focused = focused;
            if std::mem::take(&mut self.find.focus) {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            if self.find.faded != Some(!focused) && set_alpha(if focused { 255 } else { FADED }) {
                self.find.faded = Some(!focused);
            }
            self.shortcuts(&ctx, actions);
            CentralPanel::default()
                .frame(
                    Frame::new()
                        .fill(pal.mantle)
                        .stroke(Stroke::new(1.0, pal.border))
                        .inner_margin(Margin::same(8)),
                )
                .show(ui, |ui| {
                    if !focused && !NATIVE_ALPHA {
                        ui.set_opacity(0.55);
                    }
                    let drag = ui.interact(
                        ui.max_rect(),
                        ui.scope_id().with("tror_find_drag"),
                        Sense::click_and_drag(),
                    );
                    if drag.drag_started() {
                        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                        self.find.moved = true;
                    }
                    if drag.double_clicked() {
                        self.find.moved = false;
                        self.find.placed = Some(egui::pos2(f32::MAX, f32::MAX));
                    }
                    self.search_bar(ui, &pal, actions);
                    let used = ui.min_rect().height() + 16.0;
                    if (used - self.find.height).abs() > 1.0 {
                        self.find.height = used;
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, used)));
                    }
                });
            if ctx.input(|i| i.viewport().close_requested()) {
                actions.push(Action::CloseSearch);
            }
            if ctx.input(|i| !i.events.is_empty()) {
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        });
    }
}

const NATIVE_ALPHA: bool = cfg!(any(windows, target_os = "macos"));

#[cfg(windows)]
fn set_alpha(alpha: u8) -> bool {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId,
        LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
    };
    struct Search {
        title: Vec<u16>,
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid != search.pid {
            return 1;
        }
        let mut buf = [0u16; 128];
        let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if len > 0 && buf[..len as usize] == search.title[..] {
            search.found = hwnd;
            return 0;
        }
        1
    }
    let mut search = Search {
        title: TITLE.encode_utf16().collect(),
        pid: unsafe { GetCurrentProcessId() },
        found: std::ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
    if search.found.is_null() {
        return false;
    }
    unsafe {
        let style = GetWindowLongPtrW(search.found, GWL_EXSTYLE);
        SetWindowLongPtrW(search.found, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
        SetLayeredWindowAttributes(search.found, 0, alpha, LWA_ALPHA) != 0
    }
}

#[cfg(target_os = "macos")]
fn set_alpha(alpha: u8) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let mut found = false;
    for window in app.windows().iter() {
        if window.title().to_string() == TITLE {
            window.setAlphaValue(alpha as f64 / 255.0);
            found = true;
        }
    }
    found
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set_alpha(_alpha: u8) -> bool {
    true
}
