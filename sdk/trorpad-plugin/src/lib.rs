use std::alloc::Layout;

pub use serde_json::{self, Value, json};

pub trait Plugin: Default + 'static {
    fn command(&mut self, id: &str) -> Result<(), String>;

    fn event(&mut self, _name: &str, _params: &Value) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "trorpad")]
unsafe extern "C" {
    #[link_name = "call"]
    fn host_call(ptr: *const u8, len: usize) -> usize;
    #[link_name = "take"]
    fn host_take(ptr: *mut u8);
}

#[cfg(target_arch = "wasm32")]
fn exchange(message: &[u8]) -> Vec<u8> {
    unsafe {
        let size = host_call(message.as_ptr(), message.len());
        let mut reply = vec![0u8; size];
        host_take(reply.as_mut_ptr());
        reply
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn exchange(_message: &[u8]) -> Vec<u8> {
    r#"{"error":"плагин запущен не в TrorPad"}"#.as_bytes().to_vec()
}

pub fn request(method: &str, params: Value) -> Result<Value, String> {
    let message = serde_json::to_vec(&json!({ "method": method, "params": params }))
        .map_err(|e| e.to_string())?;
    let reply: Value = serde_json::from_slice(&exchange(&message)).map_err(|e| e.to_string())?;
    match reply.get("error") {
        Some(error) => Err(error.as_str().unwrap_or("ошибка хоста").to_owned()),
        None => Ok(reply.get("ok").cloned().unwrap_or(Value::Null)),
    }
}

pub mod host {
    use super::{Value, json, request};
    use serde::Deserialize;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    #[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Selection {
        pub start: usize,
        pub end: usize,
        pub anchor: usize,
        pub head: usize,
    }

    impl Selection {
        pub fn is_empty(&self) -> bool {
            self.start == self.end
        }
    }

    #[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
    pub struct Info {
        pub path: Option<String>,
        pub syntax: String,
        pub line: usize,
        pub column: usize,
        pub length: usize,
        pub root: Option<String>,
    }

    fn parse<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    fn unit(method: &str, params: Value) -> Result<(), String> {
        request(method, params).map(|_| ())
    }

    pub fn text() -> Result<String, String> {
        parse(request("text", Value::Null)?)
    }

    pub fn selection() -> Result<Selection, String> {
        parse(request("selection", Value::Null)?)
    }

    pub fn info() -> Result<Info, String> {
        parse(request("info", Value::Null)?)
    }

    pub fn selected_text() -> Result<String, String> {
        let sel = selection()?;
        Ok(crate::text::slice(&text()?, sel.start, sel.end).to_owned())
    }

    pub fn replace(start: usize, end: usize, text: &str) -> Result<(), String> {
        unit(
            "replace",
            json!({ "start": start, "end": end, "text": text }),
        )
    }

    pub fn replace_selection(text: &str) -> Result<(), String> {
        let sel = selection()?;
        replace(sel.start, sel.end, text)
    }

    pub fn select(anchor: usize, head: usize) -> Result<(), String> {
        unit("select", json!({ "anchor": anchor, "head": head }))
    }

    pub fn notify(text: &str) -> Result<(), String> {
        unit("notify", json!({ "text": text }))
    }

    pub fn log(text: &str) -> Result<(), String> {
        unit("log", json!({ "text": text }))
    }

    pub fn log_error(text: &str) -> Result<(), String> {
        unit("log", json!({ "text": text, "error": true }))
    }

    pub fn open_tab(title: &str, text: &str) -> Result<(), String> {
        unit("open_tab", json!({ "title": title, "text": text }))
    }

    pub fn read_file(path: &str) -> Result<String, String> {
        parse(request("read_file", json!({ "path": path }))?)
    }

    pub fn write_file(path: &str, text: &str) -> Result<(), String> {
        unit("write_file", json!({ "path": path, "text": text }))
    }

    pub fn storage_get<T: DeserializeOwned>(key: &str) -> Result<Option<T>, String> {
        match request("storage_get", json!({ "key": key }))? {
            Value::Null => Ok(None),
            value => parse(value).map(Some),
        }
    }

    pub fn storage_set<T: Serialize>(key: &str, value: &T) -> Result<(), String> {
        let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
        unit("storage_set", json!({ "key": key, "value": value }))
    }
}

pub mod text {
    pub fn byte_of(text: &str, char_index: usize) -> usize {
        text.char_indices()
            .nth(char_index)
            .map_or(text.len(), |(byte, _)| byte)
    }

    pub fn slice(text: &str, start: usize, end: usize) -> &str {
        let (start, end) = (start.min(end), start.max(end));
        let from = byte_of(text, start);
        let to = from + byte_of(&text[from..], end - start);
        &text[from..to]
    }

    pub fn lines_span(text: &str, start: usize, end: usize) -> (usize, usize) {
        let chars: Vec<char> = text.chars().collect();
        let (start, mut end) = (
            start.min(end).min(chars.len()),
            start.max(end).min(chars.len()),
        );
        if end > start && chars[end - 1] == '\n' {
            end -= 1;
        }
        let mut from = start;
        while from > 0 && chars[from - 1] != '\n' {
            from -= 1;
        }
        let mut to = end;
        while to < chars.len() && chars[to] != '\n' {
            to += 1;
        }
        if to > from && chars[to - 1] == '\r' {
            to -= 1;
        }
        (from, to)
    }

    pub fn chars(text: &str) -> usize {
        text.chars().count()
    }
}

fn layout(len: usize) -> Layout {
    Layout::from_size_align(len.max(1), 1).unwrap_or(Layout::new::<u8>())
}

#[doc(hidden)]
pub fn __alloc(len: usize) -> *mut u8 {
    unsafe { std::alloc::alloc(layout(len)) }
}

/// # Safety
/// `ptr` должен быть получен из `__alloc(len)` с тем же `len`.
#[doc(hidden)]
pub unsafe fn __free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        unsafe { std::alloc::dealloc(ptr, layout(len)) }
    }
}

fn dispatch<T: Plugin>(slot: &mut Option<T>, bytes: &[u8]) -> Result<(), String> {
    let message: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let string = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    if method == "initialize" {
        std::panic::set_hook(Box::new(|info| {
            let _ = host::log_error(&format!("паника: {info}"));
        }));
        *slot = Some(T::default());
        return Ok(());
    }
    let plugin = slot.as_mut().ok_or("плагин не инициализирован")?;
    match method {
        "command" => plugin.command(&string("id")),
        "event" => plugin.event(&string("name"), &params),
        other => Err(format!("неизвестный метод «{other}»")),
    }
}

/// # Safety
/// `ptr..ptr+len` должен быть читаемой памятью модуля.
#[doc(hidden)]
pub unsafe fn __handle<T: Plugin>(slot: &mut Option<T>, ptr: *const u8, len: usize) -> u64 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let Err(error) = dispatch(slot, bytes) else {
        return 0;
    };
    let reply = serde_json::to_vec(&json!({ "error": error })).unwrap_or_default();
    let out = __alloc(reply.len());
    unsafe { std::ptr::copy_nonoverlapping(reply.as_ptr(), out, reply.len()) };
    ((out as usize as u64) << 32) | reply.len() as u64
}

#[macro_export]
macro_rules! export {
    ($plugin:ty) => {
        ::std::thread_local! {
            static __TRORPAD_PLUGIN: ::std::cell::RefCell<::std::option::Option<$plugin>> =
                const { ::std::cell::RefCell::new(::std::option::Option::None) };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn trorpad_alloc(len: usize) -> *mut u8 {
            $crate::__alloc(len)
        }

        /// # Safety
        /// Вызывается только хостом TrorPad.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn trorpad_free(ptr: *mut u8, len: usize) {
            unsafe { $crate::__free(ptr, len) }
        }

        /// # Safety
        /// Вызывается только хостом TrorPad.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn trorpad_handle(ptr: *const u8, len: usize) -> u64 {
            __TRORPAD_PLUGIN.with_borrow_mut(|slot| unsafe { $crate::__handle(slot, ptr, len) })
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Probe {
        seen: Vec<String>,
    }

    impl Plugin for Probe {
        fn command(&mut self, id: &str) -> Result<(), String> {
            self.seen.push(id.to_owned());
            if id == "bad" {
                Err("нельзя".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn dispatch_routes_commands_and_reports_errors() {
        let mut slot: Option<Probe> = None;
        assert!(dispatch(&mut slot, br#"{"method":"command","params":{"id":"a"}}"#).is_err());
        dispatch(&mut slot, br#"{"method":"initialize","params":{}}"#).unwrap();
        dispatch(&mut slot, br#"{"method":"command","params":{"id":"a"}}"#).unwrap();
        assert_eq!(
            dispatch(&mut slot, br#"{"method":"command","params":{"id":"bad"}}"#),
            Err("нельзя".into())
        );
        assert!(
            dispatch(&mut slot, br#"{"method":"x"}"#)
                .unwrap_err()
                .contains("x")
        );
        assert_eq!(slot.unwrap().seen, ["a", "bad"]);
    }

    #[test]
    fn lines_span_expands_to_whole_lines() {
        let t = "один\nдва\r\nтри\n";
        assert_eq!(text::lines_span(t, 6, 6), (5, 8));
        assert_eq!(text::lines_span(t, 1, 10), (0, 8));
        assert_eq!(text::lines_span(t, 1, 11), (0, 13));
        assert_eq!(text::lines_span(t, 0, 5), (0, 4));
        assert_eq!(text::slice(t, 5, 8), "два");
        assert_eq!(text::slice("абв", 1, 9), "бв");
    }
}
