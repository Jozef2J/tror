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
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWLP_HWNDPARENT, GetForegroundWindow, GetWindowTextW,
        GetWindowThreadProcessId, SetWindowLongPtrW,
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
        }
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

    pub fn adopt(_title: &str) -> bool {
        true
    }
}

pub(super) use imp::{adopt, remember_main};

pub(super) const OWNED: bool = cfg!(windows);
