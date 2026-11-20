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
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
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
    static PENDING: AtomicBool = AtomicBool::new(false);
    static START: AtomicU64 = AtomicU64::new(0);

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

    fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
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
            WM_ENTERSIZEMOVE => {
                let r = rect(hwnd);
                let dpi = unsafe { GetDpiForWindow(hwnd) } as u64;
                let w = (r.right - r.left).max(0) as u64;
                let h = (r.bottom - r.top).max(0) as u64;
                START.store(dpi << 40 | w << 20 | h, Ordering::Relaxed);
                MOVING.store(true, Ordering::Relaxed);
                PENDING.store(false, Ordering::Relaxed);
            }
            WM_DPICHANGED if MOVING.load(Ordering::Relaxed) => {
                let before = rect(hwnd);
                let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
                let after = rect(hwnd);
                place(
                    hwnd,
                    after.left,
                    after.top,
                    before.right - before.left,
                    before.bottom - before.top,
                );
                PENDING.store(true, Ordering::Relaxed);
                return result;
            }
            WM_EXITSIZEMOVE => {
                MOVING.store(false, Ordering::Relaxed);
                let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
                if PENDING.swap(false, Ordering::Relaxed) {
                    let start = START.load(Ordering::Relaxed);
                    let dpi = (start >> 40) as i64;
                    let now = unsafe { GetDpiForWindow(hwnd) } as i64;
                    if dpi > 0 && now > 0 && dpi != now {
                        let w = ((start >> 20) & 0xFFFFF) as i64 * now / dpi;
                        let h = (start & 0xFFFFF) as i64 * now / dpi;
                        let r = rect(hwnd);
                        place(hwnd, r.left, r.top, w as i32, h as i32);
                    }
                }
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

pub(super) use imp::{adopt, cursor, remember_main};

pub(super) const OWNED: bool = cfg!(windows);
