use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

mod en;

static ENGLISH: AtomicBool = AtomicBool::new(false);
static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

macro_rules! tr {
    ($ru:expr) => {
        $crate::i18n::t($ru)
    };
}

macro_rules! tf {
    ($ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::english() {
            format!($en $(, $arg)*)
        } else {
            format!($ru $(, $arg)*)
        }
    };
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn t(ru: &'static str) -> &'static str {
    if !english() {
        return ru;
    }
    TABLE
        .get_or_init(|| en::EN.iter().copied().collect())
        .get(ru)
        .copied()
        .unwrap_or(ru)
}

pub fn system_english() -> bool {
    let russian = |tag: &str| {
        let tag = tag.to_ascii_lowercase();
        ["ru", "uk", "be", "kk"].iter().any(|l| tag.starts_with(l))
    };
    #[cfg(windows)]
    {
        let mut buffer = [0u16; 85];
        let len = unsafe {
            windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        if len > 1 {
            return !russian(&String::from_utf16_lossy(&buffer[..len as usize - 1]));
        }
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
            && value != "C"
            && value != "POSIX"
        {
            return !russian(&value);
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(first) = text.split('"').nth(1) {
            return !russian(first);
        }
    }
    true
}
