use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const SERVICE: &str = "Tror";
const CHUNK: usize = 2000;

static IN_FILE: AtomicBool = AtomicBool::new(false);
static CACHE: Mutex<Option<BTreeMap<String, String>>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn in_file() -> bool {
    IN_FILE.load(Ordering::Relaxed)
}

pub fn set_in_file(on: bool) {
    IN_FILE.store(on, Ordering::Relaxed);
}

pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|e| e.clone())
}

fn note(result: &Result<(), String>) {
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = result.as_ref().err().cloned();
    }
}

pub fn read() -> Result<BTreeMap<String, String>, String> {
    let mut cache = CACHE.lock().map_err(|e| e.to_string())?;
    if let Some(map) = cache.as_ref() {
        return Ok(map.clone());
    }
    let mut bytes = Vec::new();
    for index in 0.. {
        match backend::get(index) {
            Ok(Some(part)) => bytes.extend(part),
            Ok(None) => break,
            Err(e) => {
                note(&Err(e.clone()));
                return Err(e);
            }
        }
    }
    let map: BTreeMap<String, String> = if bytes.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
    };
    *cache = Some(map.clone());
    Ok(map)
}

pub fn write(map: &BTreeMap<String, String>) -> Result<(), String> {
    let mut cache = CACHE.lock().map_err(|e| e.to_string())?;
    if cache.as_ref() == Some(map) {
        return Ok(());
    }
    let result = store(map);
    note(&result);
    if result.is_ok() {
        *cache = Some(map.clone());
    } else {
        *cache = None;
    }
    result
}

pub fn clear() {
    if let Ok(mut cache) = CACHE.lock()
        && cache.as_ref().is_none_or(|map| !map.is_empty())
        && store(&BTreeMap::new()).is_ok()
    {
        *cache = Some(BTreeMap::new());
    }
}

fn store(map: &BTreeMap<String, String>) -> Result<(), String> {
    let bytes = if map.is_empty() {
        Vec::new()
    } else {
        serde_json::to_vec(map).map_err(|e| e.to_string())?
    };
    let parts: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    for (index, part) in parts.iter().enumerate() {
        backend::set(index, part)?;
    }
    let mut index = parts.len();
    while backend::delete(index)? {
        index += 1;
    }
    Ok(())
}

#[cfg(not(test))]
mod backend {
    use super::SERVICE;
    use keyring_core::{Entry, Error};
    use std::sync::OnceLock;

    static READY: OnceLock<Result<(), String>> = OnceLock::new();

    fn ready() -> Result<(), String> {
        READY
            .get_or_init(|| {
                #[cfg(target_os = "macos")]
                let store = apple_native_keyring_store::keychain::Store::new();
                #[cfg(windows)]
                let store = windows_native_keyring_store::Store::new();
                #[cfg(target_os = "linux")]
                let store = dbus_secret_service_keyring_store::Store::new();
                store
                    .map(|store| keyring_core::set_default_store(store))
                    .map_err(|e| e.to_string())
            })
            .clone()
    }

    fn entry(index: usize) -> Result<Entry, String> {
        ready()?;
        let account = if index == 0 {
            "connections".to_owned()
        } else {
            format!("connections.{index}")
        };
        Entry::new(SERVICE, &account).map_err(|e| e.to_string())
    }

    pub fn get(index: usize) -> Result<Option<Vec<u8>>, String> {
        match entry(index)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn set(index: usize, bytes: &[u8]) -> Result<(), String> {
        entry(index)?.set_secret(bytes).map_err(|e| e.to_string())
    }

    pub fn delete(index: usize) -> Result<bool, String> {
        match entry(index)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(Error::NoEntry) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
pub mod backend {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    pub static ENTRIES: Mutex<BTreeMap<usize, Vec<u8>>> = Mutex::new(BTreeMap::new());
    pub static BROKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn check() -> Result<(), String> {
        if BROKEN.load(std::sync::atomic::Ordering::Relaxed) {
            Err("хранилище недоступно".into())
        } else {
            Ok(())
        }
    }

    pub fn get(index: usize) -> Result<Option<Vec<u8>>, String> {
        check()?;
        Ok(ENTRIES.lock().unwrap().get(&index).cloned())
    }

    pub fn set(index: usize, bytes: &[u8]) -> Result<(), String> {
        check()?;
        ENTRIES.lock().unwrap().insert(index, bytes.to_vec());
        Ok(())
    }

    pub fn delete(index: usize) -> Result<bool, String> {
        check()?;
        Ok(ENTRIES.lock().unwrap().remove(&index).is_some())
    }
}

#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub fn reset_cache_for_test() {
    *CACHE.lock().unwrap() = None;
}

#[cfg(test)]
pub fn reset_for_test() {
    *CACHE.lock().unwrap() = None;
    backend::ENTRIES.lock().unwrap().clear();
    backend::BROKEN.store(false, Ordering::Relaxed);
    set_in_file(false);
}
