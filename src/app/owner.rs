use std::collections::{HashMap, HashSet};

use super::Pane;

#[derive(Default)]
pub(super) struct Floats {
    pub spawn: HashMap<Pane, [f32; 4]>,
    pub owned: HashSet<Pane>,
}

impl Floats {
    pub fn prune(&mut self, detached: &[Pane]) {
        self.spawn.retain(|p, _| detached.contains(p));
        self.owned.retain(|p| detached.contains(p));
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWLP_HWNDPARENT, GetForegroundWindow, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
        WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
    };

    static MAIN: AtomicIsize = AtomicIsize::new(0);

    fn ours(hwnd: HWND) -> bool {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid == unsafe { GetCurrentProcessId() }
    }

    pub fn remember_main() {
        if MAIN.load(Ordering::Relaxed) != 0 {
            return;
        }
        let hwnd = unsafe { GetForegroundWindow() };
        if !hwnd.is_null() && ours(hwnd) {
            MAIN.store(hwnd as isize, Ordering::Relaxed);
            unsafe { SetWindowSubclass(hwnd, Some(steady_dpi), 1, 0) };
        }
    }

    static MOVING: AtomicBool = AtomicBool::new(false);
    static RECENT: std::sync::Mutex<Vec<std::time::Instant>> = std::sync::Mutex::new(Vec::new());

    fn rect(hwnd: HWND) -> RECT {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut r) };
        r
    }

    fn place(hwnd: HWND, r: &RECT) {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }

    fn storm() -> bool {
        let now = std::time::Instant::now();
        let Ok(mut recent) = RECENT.lock() else {
            return false;
        };
        recent.retain(|t| now.duration_since(*t).as_millis() < 1000);
        recent.push(now);
        recent.len() > 3
    }

    unsafe extern "system" fn steady_dpi(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match msg {
            WM_ENTERSIZEMOVE => MOVING.store(true, Ordering::Relaxed),
            WM_EXITSIZEMOVE => MOVING.store(false, Ordering::Relaxed),
            WM_DPICHANGED => {
                let before = rect(hwnd);
                let suggested = unsafe { *(lparam as *const RECT) };
                let moving = MOVING.load(Ordering::Relaxed);
                let storm = storm();
                let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
                let keep = moving || storm;
                place(hwnd, if keep { &before } else { &suggested });
                crate::trace::line(
                    tf!(
                        "[DPI] {} dpi, {}: окно {}×{} в ({}, {})",
                        "[DPI] {} dpi, {}: window {}×{} at ({}, {})",
                        wparam & 0xFFFF,
                        if storm {
                            tr!("частая смена — геометрия сохранена")
                        } else if moving {
                            tr!("перетаскивание — геометрия сохранена")
                        } else {
                            tr!("прямоугольник Windows")
                        },
                        if keep {
                            before.right - before.left
                        } else {
                            suggested.right - suggested.left
                        },
                        if keep {
                            before.bottom - before.top
                        } else {
                            suggested.bottom - suggested.top
                        },
                        if keep { before.left } else { suggested.left },
                        if keep { before.top } else { suggested.top },
                    ),
                    false,
                );
                return result;
            }
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    pub fn cursor() -> Option<(f32, f32)> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let main = MAIN.load(Ordering::Relaxed);
        if main == 0 {
            return None;
        }
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0
            || unsafe { ScreenToClient(main as HWND, &mut point) } == 0
        {
            return None;
        }
        Some((point.x as f32, point.y as f32))
    }

    pub fn adopt(title: &str) -> bool {
        let main = MAIN.load(Ordering::Relaxed);
        if main == 0 {
            return false;
        }
        struct Search {
            title: Vec<u16>,
            main: isize,
            found: HWND,
        }
        unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> i32 {
            let search = unsafe { &mut *(data as *mut Search) };
            if hwnd as isize == search.main || !ours(hwnd) {
                return 1;
            }
            let mut buf = [0u16; 256];
            let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
            if len > 0 && buf[..len as usize] == search.title[..] {
                search.found = hwnd;
                return 0;
            }
            1
        }
        let mut search = Search {
            title: title.encode_utf16().collect(),
            main,
            found: std::ptr::null_mut(),
        };
        unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
        if search.found.is_null() {
            return false;
        }
        unsafe { SetWindowLongPtrW(search.found, GWLP_HWNDPARENT, main) };
        true
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn remember_main() {}

    pub fn cursor() -> Option<(f32, f32)> {
        None
    }

    pub fn adopt(_title: &str) -> bool {
        true
    }
}

pub(crate) use imp::adopt;
pub(super) use imp::{cursor, remember_main};

pub(super) const OWNED: bool = cfg!(windows);
