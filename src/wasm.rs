use std::path::{Path, PathBuf};

use ropey::Rope;
use serde::Deserialize;
use serde_json::{Value, json};
use wasmi::{
    Caller, Config, Engine, Extern, Instance, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc,
};

#[cfg(not(test))]
const FUEL: u64 = 1_000_000_000;
#[cfg(test)]
const FUEL: u64 = 2_000_000;
const API: u32 = 1;
const MEMORY: usize = 512 << 20;
#[cfg(test)]
const FUEL_PROBE: u64 = 300_000_000;

#[derive(Deserialize, Clone, Debug)]
pub struct CommandDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub key: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default = "default_wasm")]
    pub wasm: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub commands: Vec<CommandDef>,
}

fn default_wasm() -> String {
    "plugin.wasm".into()
}

#[derive(Default, Debug)]
pub struct Outcome {
    pub effects: Vec<Effect>,
    pub sel: Option<(usize, usize)>,
    pub error: Option<String>,
}

pub struct Snapshot {
    pub rope: Rope,
    pub sel: (usize, usize),
    pub path: Option<String>,
    pub syntax: String,
    pub root: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    Edit(std::ops::Range<usize>, String),
    Select(usize, usize),
    Notify(String),
    Log(String, bool),
    OpenTab(String, String),
}

struct Host {
    snap: Option<Snapshot>,
    effects: Vec<Effect>,
    reply: Vec<u8>,
    permissions: Vec<String>,
    storage: PathBuf,
    limits: StoreLimits,
}

pub struct Loaded {
    pub manifest: Manifest,
    store: Store<Host>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    free: TypedFunc<(u32, u32), ()>,
    handle: TypedFunc<(u32, u32), u64>,
    pub broken: Option<String>,
}

fn memory_of(caller: &Caller<'_, Host>) -> Option<Memory> {
    caller.get_export("memory").and_then(Extern::into_memory)
}

fn read_bytes(caller: &Caller<'_, Host>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    let memory = memory_of(caller)?;
    let mut buf = vec![0u8; len as usize];
    memory.read(caller, ptr as usize, &mut buf).ok()?;
    Some(buf)
}

fn inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    let full = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let base = root.canonicalize().map_err(|e| e.to_string())?;
    let parent = full.parent().ok_or("неверный путь")?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let target = parent.join(full.file_name().ok_or("неверный путь")?);
    let real = match target.canonicalize() {
        Ok(real) => real,
        Err(_) if target.symlink_metadata().is_err() => target.clone(),
        Err(e) => return Err(e.to_string()),
    };
    if !parent.starts_with(&base) || !real.starts_with(&base) {
        return Err("путь вне папки проекта".into());
    }
    Ok(target)
}

fn storage_load(path: &Path) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn request(host: &mut Host, method: &str, params: &Value) -> Result<Value, String> {
    let s = |k: &str| {
        params
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let n = |k: &str| params.get(k).and_then(Value::as_u64).unwrap_or_default() as usize;
    let allowed = |host: &Host, p: &str| {
        if host.permissions.iter().any(|x| x == p) {
            Ok(())
        } else {
            Err(format!("нет разрешения «{p}» в plugin.json"))
        }
    };
    match method {
        "api" => Ok(json!(API)),
        "notify" => {
            host.effects.push(Effect::Notify(s("text")));
            Ok(Value::Null)
        }
        "log" => {
            let error = params
                .get("error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            host.effects.push(Effect::Log(s("text"), error));
            Ok(Value::Null)
        }
        "open_tab" => {
            host.effects.push(Effect::OpenTab(s("title"), s("text")));
            Ok(Value::Null)
        }
        "storage_get" => Ok(storage_load(&host.storage)
            .get(&s("key"))
            .cloned()
            .unwrap_or(Value::Null)),
        "storage_set" => {
            let mut map = storage_load(&host.storage);
            map.insert(
                s("key"),
                params.get("value").cloned().unwrap_or(Value::Null),
            );
            let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
            std::fs::write(&host.storage, text).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "read_file" | "write_file" => {
            allowed(host, "fs")?;
            let root = host
                .snap
                .as_ref()
                .and_then(|s| s.root.clone())
                .ok_or("нет открытой папки проекта")?;
            let path = inside(&root, &s("path"))?;
            if method == "read_file" {
                let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                Ok(json!(text))
            } else {
                std::fs::write(&path, s("text")).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
        }
        _ => {
            let snap = host.snap.as_mut().ok_or("нет открытого файла")?;
            let len = snap.rope.len_chars();
            match method {
                "text" => Ok(json!(snap.rope.to_string())),
                "selection" => Ok(
                    json!({"start": snap.sel.0.min(snap.sel.1), "end": snap.sel.0.max(snap.sel.1), "anchor": snap.sel.0, "head": snap.sel.1}),
                ),
                "info" => {
                    let line = snap.rope.char_to_line(snap.sel.1.min(len));
                    let column = snap.sel.1.min(len) - snap.rope.line_to_char(line);
                    Ok(json!({
                        "path": snap.path,
                        "syntax": snap.syntax,
                        "line": line + 1,
                        "column": column + 1,
                        "length": len,
                        "root": snap.root.as_ref().map(|r| r.display().to_string()),
                    }))
                }
                "replace" => {
                    let (start, end) = (n("start").min(len), n("end").min(len));
                    let (start, end) = (start.min(end), start.max(end));
                    let text = s("text");
                    snap.rope.remove(start..end);
                    snap.rope.insert(start, &text);
                    let added = text.chars().count();
                    let shift = |p: usize| {
                        if p <= start {
                            p
                        } else if p >= end {
                            p - (end - start) + added
                        } else {
                            start + added
                        }
                    };
                    snap.sel = (shift(snap.sel.0), shift(snap.sel.1));
                    host.effects.push(Effect::Edit(start..end, text));
                    Ok(Value::Null)
                }
                "select" => {
                    let (a, b) = (n("anchor").min(len), n("head").min(len));
                    snap.sel = (a, b);
                    host.effects.push(Effect::Select(a, b));
                    Ok(Value::Null)
                }
                _ => Err(format!("неизвестный запрос «{method}»")),
            }
        }
    }
}

fn call(mut caller: Caller<'_, Host>, ptr: u32, len: u32) -> u32 {
    let reply = match read_bytes(&caller, ptr, len)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    {
        Some(message) => {
            let method = message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            match request(caller.data_mut(), &method, &params) {
                Ok(value) => json!({ "ok": value }),
                Err(e) => json!({ "error": e }),
            }
        }
        None => json!({ "error": "неверное сообщение" }),
    };
    let bytes = serde_json::to_vec(&reply).unwrap_or_default();
    let size = bytes.len() as u32;
    caller.data_mut().reply = bytes;
    size
}

fn take(mut caller: Caller<'_, Host>, ptr: u32) {
    let bytes = std::mem::take(&mut caller.data_mut().reply);
    if let Some(memory) = memory_of(&caller) {
        let _ = memory.write(&mut caller, ptr as usize, &bytes);
    }
}

fn engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    Engine::new(&config)
}

impl Loaded {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(dir.join("plugin.json")).map_err(|e| e.to_string())?;
        let manifest: Manifest =
            serde_json::from_str(&text).map_err(|e| format!("plugin.json: {e}"))?;
        let bytes = std::fs::read(dir.join(&manifest.wasm))
            .map_err(|e| format!("{}: {e}", manifest.wasm))?;
        let mut loaded = Self::from_bytes(manifest, dir.to_path_buf(), &bytes)?;
        let init = json!({"method": "initialize", "params": {"api": API, "app": env!("CARGO_PKG_VERSION")}});
        match loaded.send(&init, None).error {
            Some(e) => Err(format!("initialize: {e}")),
            None => Ok(loaded),
        }
    }

    fn from_bytes(manifest: Manifest, dir: PathBuf, bytes: &[u8]) -> Result<Self, String> {
        let engine = engine();
        let module = Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        let host = Host {
            snap: None,
            effects: Vec::new(),
            reply: Vec::new(),
            permissions: manifest.permissions.clone(),
            storage: dir.join("storage.json"),
            limits: StoreLimitsBuilder::new().memory_size(MEMORY).build(),
        };
        let mut store = Store::new(&engine, host);
        store.limiter(|host| &mut host.limits);
        store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        let mut linker = <Linker<Host>>::new(&engine);
        linker
            .func_wrap("trorpad", "call", call)
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap("trorpad", "take", take)
            .map_err(|e| e.to_string())?;
        let instance: Instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| e.to_string())?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or("нет экспорта memory")?;
        let alloc = instance
            .get_typed_func::<u32, u32>(&store, "trorpad_alloc")
            .map_err(|e| format!("trorpad_alloc: {e}"))?;
        let free = instance
            .get_typed_func::<(u32, u32), ()>(&store, "trorpad_free")
            .map_err(|e| format!("trorpad_free: {e}"))?;
        let handle = instance
            .get_typed_func::<(u32, u32), u64>(&store, "trorpad_handle")
            .map_err(|e| format!("trorpad_handle: {e}"))?;
        Ok(Self {
            manifest,
            store,
            memory,
            alloc,
            free,
            handle,
            broken: None,
        })
    }

    pub fn send(&mut self, message: &Value, snap: Option<Snapshot>) -> Outcome {
        if let Some(reason) = &self.broken {
            return Outcome {
                error: Some(format!("плагин остановлен: {reason}")),
                ..Outcome::default()
            };
        }
        self.store.data_mut().snap = snap;
        self.store.data_mut().effects.clear();
        let result = self
            .store
            .set_fuel(FUEL)
            .map_err(|e| e.to_string())
            .and_then(|()| self.exchange(message));
        let sel = self.store.data_mut().snap.take().map(|s| s.sel);
        let mut effects = std::mem::take(&mut self.store.data_mut().effects);
        match result {
            Ok(error) => Outcome {
                effects,
                sel,
                error,
            },
            Err(e) => {
                effects.retain(|e| matches!(e, Effect::Log(..)));
                let panic = effects.iter().rev().find_map(|e| match e {
                    Effect::Log(text, true) if text.starts_with("паника") => {
                        Some(text.clone())
                    }
                    _ => None,
                });
                let reason = match panic {
                    Some(text) => text,
                    None if e.contains("fuel") => "превышен лимит работы".to_owned(),
                    None => e,
                };
                self.broken = Some(reason.clone());
                Outcome {
                    effects,
                    sel: None,
                    error: Some(reason),
                }
            }
        }
    }

    fn exchange(&mut self, message: &Value) -> Result<Option<String>, String> {
        let bytes = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        let len = bytes.len() as u32;
        let ptr = self
            .alloc
            .call(&mut self.store, len)
            .map_err(|e| e.to_string())?;
        self.memory
            .write(&mut self.store, ptr as usize, &bytes)
            .map_err(|e| e.to_string())?;
        let packed = self
            .handle
            .call(&mut self.store, (ptr, len))
            .map_err(|e| e.to_string())?;
        let _ = self.free.call(&mut self.store, (ptr, len));
        if packed == 0 {
            return Ok(None);
        }
        let (out_ptr, out_len) = ((packed >> 32) as u32, packed as u32);
        let mut out = vec![0u8; out_len as usize];
        self.memory
            .read(&self.store, out_ptr as usize, &mut out)
            .map_err(|e| e.to_string())?;
        let _ = self.free.call(&mut self.store, (out_ptr, out_len));
        let reply: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
        Ok(reply
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    pub fn wants(&self, event: &str) -> bool {
        self.manifest.events.iter().any(|e| e == event)
    }
}

pub fn load_all(dir: &Path) -> (Vec<Loaded>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (loaded, errors);
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("plugin.json").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        match Loaded::load(&dir) {
            Ok(plugin)
                if loaded
                    .iter()
                    .any(|p: &Loaded| p.manifest.id == plugin.manifest.id) =>
            {
                errors.push(format!(
                    "{}: id «{}» уже занят другим плагином",
                    dir.file_name().unwrap_or_default().to_string_lossy(),
                    plugin.manifest.id
                ))
            }
            Ok(plugin) => loaded.push(plugin),
            Err(e) => errors.push(format!(
                "{}: {e}",
                dir.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    (loaded, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ECHO: &str = r#"
(module
  (import "trorpad" "call" (func $call (param i32 i32) (result i32)))
  (import "trorpad" "take" (func $take (param i32)))
  (memory (export "memory") 2)
  (global $next (mut i32) (i32.const 4096))
  (data (i32.const 0) "{\"method\":\"replace\",\"params\":{\"start\":0,\"end\":5,\"text\":\"HELLO\"}}")
  (data (i32.const 200) "{\"method\":\"notify\",\"params\":{\"text\":\"done\"}}")
  (data (i32.const 300) "{\"method\":\"read_file\",\"params\":{\"path\":\"../secret\"}}")
  (data (i32.const 400) "{\"error\":\"boom\"}")
  (func (export "trorpad_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))
    (local.get $p))
  (func (export "trorpad_free") (param i32 i32))
  (func (export "trorpad_handle") (param $ptr i32) (param $len i32) (result i64)
    (local $n i32)
    (if (i32.eq (i32.load8_u (i32.add (local.get $ptr) (i32.const 11))) (i32.const 105))
      (then (return (i64.const 0))))
    (if (i32.eq (i32.load8_u (i32.add (local.get $ptr) (i32.const 11))) (i32.const 103))
      (then
        (if (i32.eq (memory.grow (i32.const 9000)) (i32.const -1))
          (then (return (i64.const 0))))
        (return (i64.or (i64.shl (i64.const 400) (i64.const 32)) (i64.const 16)))))
    (if (i32.eq (i32.load8_u (i32.add (local.get $ptr) (i32.const 11))) (i32.const 108))
      (then (loop $spin (br $spin))))
    (if (i32.eq (i32.load8_u (i32.add (local.get $ptr) (i32.const 11))) (i32.const 101))
      (then
        (local.set $n (call $call (i32.const 300) (i32.const 52)))
        (call $take (i32.const 8000))
        (return (i64.or (i64.shl (i64.const 400) (i64.const 32)) (i64.const 16)))))
    (drop (call $call (i32.const 0) (i32.const 64)))
    (call $take (i32.const 8000))
    (drop (call $call (i32.const 200) (i32.const 44)))
    (call $take (i32.const 8000))
    (i64.const 0))
)"#;

    fn plugin() -> Loaded {
        let manifest = Manifest {
            id: "echo".into(),
            name: "Echo".into(),
            version: String::new(),
            wasm: String::new(),
            permissions: vec!["fs".into()],
            events: vec!["saved".into()],
            commands: Vec::new(),
        };
        let bytes = wat::parse_str(ECHO).unwrap();
        let dir = std::env::temp_dir();
        let mut loaded = Loaded::from_bytes(manifest, dir, &bytes).unwrap();
        assert_eq!(
            loaded.send(&json!({"method":"initialize"}), None).error,
            None
        );
        loaded
    }

    fn snap(text: &str, root: Option<PathBuf>) -> Snapshot {
        Snapshot {
            rope: Rope::from_str(text),
            sel: (7, 7),
            path: None,
            syntax: "Text".into(),
            root,
        }
    }

    #[test]
    fn command_edits_and_notifies() {
        let mut p = plugin();
        let out = p.send(
            &json!({"method":"command","params":{"id":"x"}}),
            Some(snap("hello world", None)),
        );
        assert_eq!(out.error, None);
        assert_eq!(out.sel, Some((7, 7)));
        assert_eq!(
            out.effects,
            vec![
                Effect::Edit(0..5, "HELLO".into()),
                Effect::Notify("done".into())
            ]
        );
    }

    #[test]
    fn plugin_errors_and_path_escape_are_reported() {
        let dir = std::env::temp_dir().join(format!("tror-wasm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = plugin();
        let out = p.send(
            &json!({"method":"error"}),
            Some(snap("x", Some(dir.clone()))),
        );
        assert_eq!(out.error.as_deref(), Some("boom"));
        let mut host = Host {
            snap: Some(snap("x", Some(dir.clone()))),
            effects: Vec::new(),
            reply: Vec::new(),
            permissions: vec!["fs".into()],
            storage: dir.join("s.json"),
            limits: StoreLimits::default(),
        };
        assert!(request(&mut host, "read_file", &json!({"path":"../secret"})).is_err());
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("tror-out-{}", std::process::id()));
            std::fs::write(&outside, "x").unwrap();
            std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
            let err =
                request(&mut host, "write_file", &json!({"path":"link","text":"y"})).unwrap_err();
            assert!(err.contains("вне"), "{err}");
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "x");
            std::fs::remove_file(outside).ok();
        }
        std::fs::write(dir.join("a.txt"), "data").unwrap();
        assert_eq!(
            request(&mut host, "read_file", &json!({"path":"a.txt"})).unwrap(),
            json!("data")
        );
        host.permissions.clear();
        assert!(
            request(&mut host, "read_file", &json!({"path":"a.txt"}))
                .unwrap_err()
                .contains("разрешения")
        );
        request(&mut host, "storage_set", &json!({"key":"k","value":[1,2]})).unwrap();
        assert_eq!(
            request(&mut host, "storage_get", &json!({"key":"k"})).unwrap(),
            json!([1, 2])
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn endless_loop_is_stopped_by_fuel() {
        let mut p = plugin();
        let err = p.send(&json!({"method":"loop"}), None).error.unwrap();
        assert!(err.contains("лимит"), "{err}");
        assert!(
            p.send(&json!({"method":"initialize"}), None)
                .error
                .is_some()
        );
    }

    #[test]
    fn memory_growth_is_limited() {
        let mut p = plugin();
        assert_eq!(p.send(&json!({"method":"grow"}), None).error, None);
    }

    #[test]
    fn replace_keeps_selection_consistent() {
        let mut host = Host {
            snap: Some(snap("abcdef", None)),
            effects: Vec::new(),
            reply: Vec::new(),
            permissions: Vec::new(),
            storage: PathBuf::new(),
            limits: StoreLimits::default(),
        };
        host.snap.as_mut().unwrap().sel = (4, 6);
        request(
            &mut host,
            "replace",
            &json!({"start":1,"end":3,"text":"XYZW"}),
        )
        .unwrap();
        let s = host.snap.as_ref().unwrap();
        assert_eq!(s.rope.to_string(), "aXYZWdef");
        assert_eq!(s.sel, (6, 8));
    }
}

#[cfg(test)]
mod speed {
    #[test]
    #[ignore]
    fn fuel_speed() {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = wasmi::Engine::new(&config);
        let bytes = wat::parse_str(r#"(module (func (export "f") (loop $l (br $l))))"#).unwrap();
        let module = wasmi::Module::new(&engine, &bytes).unwrap();
        let mut store = wasmi::Store::new(&engine, ());
        store.set_fuel(super::FUEL_PROBE).unwrap();
        let linker = <wasmi::Linker<()>>::new(&engine);
        let inst = linker.instantiate_and_start(&mut store, &module).unwrap();
        let f = inst.get_typed_func::<(), ()>(&store, "f").unwrap();
        let start = std::time::Instant::now();
        let _ = f.call(&mut store, ());
        eprintln!("{} fuel in {:?}", super::FUEL_PROBE, start.elapsed());
    }
}
