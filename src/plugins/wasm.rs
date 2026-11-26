use crate::text::editor::{DecoKind, Decoration};
use std::path::{Path, PathBuf};

use ropey::Rope;
use serde::Deserialize;
use serde_json::{Value, json};
use wasmi::{
    Caller, Config, Engine, Extern, Instance, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc,
};

#[cfg(not(test))]
const FUEL: u64 = 4_000_000_000;
#[cfg(test)]
const FUEL: u64 = 2_000_000;
const API: u32 = 2;
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
pub struct PanelDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub dock: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SettingDef {
    pub key: String,
    pub title: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub options: Vec<String>,
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
    #[serde(default)]
    pub panels: Vec<PanelDef>,
    #[serde(default)]
    pub settings: Vec<SettingDef>,
}

fn default_wasm() -> String {
    "plugin.wasm".into()
}

#[derive(Default, Debug)]
pub struct Outcome {
    pub effects: Vec<Effect>,
    pub sel: Option<(usize, usize)>,
    pub error: Option<String>,
    pub broken: bool,
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
    Prompt(String, String, String),
    Pick(String, String, Vec<String>),
    Panel(String, Value),
    ShowPanel(String),
    Decorate(String, Vec<Decoration>),
}

fn hex_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn http(params: &Value) -> Result<Value, String> {
    use std::io::Read;
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(tr!("адрес должен начинаться с http:// или https://").into());
    }
    let method = params
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .user_agent(concat!("trorpad-plugin/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let headers: Vec<(String, String)> = params
        .get("headers")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let body = params
        .get("body")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let request = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(url);
    let request = headers
        .iter()
        .fold(request, |r, (k, v)| r.header(k.as_str(), v.as_str()));
    let response = match body {
        Some(body) => agent.run(request.body(body).map_err(|e| e.to_string())?),
        None => agent.run(request.body(()).map_err(|e| e.to_string())?),
    }
    .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let mut out = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(10 << 20)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(json!({"status": status, "body": String::from_utf8_lossy(&out)}))
}

struct Host {
    snap: Option<Snapshot>,
    effects: Vec<Effect>,
    reply: Vec<u8>,
    manifest: Manifest,
    dir: PathBuf,
    limits: StoreLimits,
}

pub struct Loaded {
    pub manifest: Manifest,
    pub dir: PathBuf,
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
    let start = ptr as usize;
    let end = start.checked_add(len as usize)?;
    memory.data(caller).get(start..end).map(<[u8]>::to_vec)
}

fn inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    let full = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let base = root.canonicalize().map_err(|e| e.to_string())?;
    let parent = full.parent().ok_or(tr!("неверный путь"))?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let target = parent.join(full.file_name().ok_or(tr!("неверный путь"))?);
    let real = match target.canonicalize() {
        Ok(real) => real,
        Err(_) if target.symlink_metadata().is_err() => target.clone(),
        Err(e) => return Err(e.to_string()),
    };
    if !parent.starts_with(&base) || !real.starts_with(&base) {
        return Err(tr!("путь вне папки проекта").into());
    }
    Ok(target)
}

pub fn settings_of(dir: &Path, manifest: &Manifest) -> serde_json::Map<String, Value> {
    let saved = storage_load(&dir.join("settings.json"));
    manifest
        .settings
        .iter()
        .map(|d| {
            let value = saved
                .get(&d.key)
                .cloned()
                .unwrap_or_else(|| d.default.clone());
            (d.key.clone(), value)
        })
        .collect()
}

pub fn save_settings(dir: &Path, values: &serde_json::Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(values).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("settings.json"), text).map_err(|e| e.to_string())
}

fn storage_load(path: &Path) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn store() -> crate::remote::Store {
    crate::remote::Store::load(eframe::storage_dir("Tror").map(|d| d.join("connections.json")))
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
        if host.manifest.permissions.iter().any(|x| x == p) {
            Ok(())
        } else {
            Err(tf!(
                "нет разрешения «{p}» в plugin.json",
                "no \"{p}\" permission in plugin.json"
            ))
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
        "prompt" => {
            host.effects
                .push(Effect::Prompt(s("id"), s("title"), s("value")));
            Ok(Value::Null)
        }
        "pick" => {
            let items = params
                .get("items")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|v| v.as_str().unwrap_or_default().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            host.effects.push(Effect::Pick(s("id"), s("title"), items));
            Ok(Value::Null)
        }
        "panel" | "show_panel" => {
            let id = s("id");
            if !host.manifest.panels.iter().any(|p| p.id == id) {
                return Err(tf!(
                    "панель «{id}» не описана в plugin.json",
                    "panel \"{id}\" is not described in plugin.json"
                ));
            }
            host.effects.push(if method == "panel" {
                Effect::Panel(id, params.get("ui").cloned().unwrap_or(Value::Null))
            } else {
                Effect::ShowPanel(id)
            });
            Ok(Value::Null)
        }
        "setting" => settings_of(&host.dir, &host.manifest)
            .get(&s("key"))
            .cloned()
            .ok_or_else(|| {
                tf!(
                    "настройка «{}» не описана в plugin.json",
                    "setting \"{}\" is not described in plugin.json",
                    s("key")
                )
            }),
        "storage_get" => Ok(storage_load(&host.dir.join("storage.json"))
            .get(&s("key"))
            .cloned()
            .unwrap_or(Value::Null)),
        "storage_set" => {
            let mut map = storage_load(&host.dir.join("storage.json"));
            map.insert(
                s("key"),
                params.get("value").cloned().unwrap_or(Value::Null),
            );
            let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
            std::fs::write(host.dir.join("storage.json"), text).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "http" => {
            allowed(host, "network")?;
            http(params)
        }
        "hosts" | "databases" => {
            allowed(host, if method == "hosts" { "hosts" } else { "db" })?;
            let store = store();
            Ok(if method == "hosts" {
                Value::Array(
                    store
                        .profiles
                        .iter()
                        .map(|p| json!({"name": p.name, "protocol": p.protocol.label(), "host": p.host, "directory": p.directory}))
                        .collect(),
                )
            } else {
                Value::Array(
                    store
                        .databases
                        .iter()
                        .map(|p| json!({"name": p.name, "kind": p.kind.label()}))
                        .collect(),
                )
            })
        }
        "remote_list" | "remote_read" | "remote_write" => {
            allowed(host, "hosts")?;
            let store = store();
            let name = s("profile");
            let profile = store
                .profiles
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| tf!("профиль «{name}» не найден", "profile \"{name}\" not found"))?;
            let known = store
                .known_hosts
                .get(&format!("{}:{}", profile.host.trim(), profile.port))
                .cloned();
            let op = match method {
                "remote_list" => crate::remote::OneShot::List(s("path")),
                "remote_read" => crate::remote::OneShot::Read(s("path")),
                _ => crate::remote::OneShot::Write(s("path"), s("text").into_bytes()),
            };
            crate::remote::one_shot(profile, known.as_deref(), op)
        }
        "db_query" => {
            allowed(host, "db")?;
            crate::db::one_shot(&store(), &s("profile"), &s("sql"))
        }
        "decorate" => {
            let snap = host.snap.as_ref().ok_or(tr!("нет открытого файла"))?;
            let len = snap.rope.len_chars();
            let lines = snap.rope.len_lines();
            let mut items = Vec::new();
            for item in params
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = match item
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("highlight")
                {
                    "underline" => DecoKind::Underline,
                    "gutter" => DecoKind::Gutter,
                    _ => DecoKind::Highlight,
                };
                let num = |k: &str| item.get(k).and_then(Value::as_u64).map(|v| v as usize);
                let (start, end) = if kind == DecoKind::Gutter {
                    let line = num("line").unwrap_or(1).clamp(1, lines) - 1;
                    (line, line)
                } else {
                    let a = num("start").unwrap_or(0).min(len);
                    let b = num("end").unwrap_or(a).min(len);
                    (a.min(b), a.max(b))
                };
                items.push(Decoration {
                    kind,
                    start,
                    end,
                    color: item
                        .get("color")
                        .and_then(Value::as_str)
                        .and_then(hex_color),
                    message: item
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
                if items.len() >= 5000 {
                    break;
                }
            }
            host.effects.push(Effect::Decorate(s("key"), items));
            Ok(Value::Null)
        }
        "read_file" | "write_file" => {
            allowed(host, "fs")?;
            let root = host
                .snap
                .as_ref()
                .and_then(|s| s.root.clone())
                .ok_or(tr!("нет открытой папки проекта"))?;
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
            let snap = host.snap.as_mut().ok_or(tr!("нет открытого файла"))?;
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
                _ => Err(tf!(
                    "неизвестный запрос «{method}»",
                    "unknown request \"{method}\""
                )),
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
        None => json!({ "error": tr!("неверное сообщение") }),
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
            manifest: manifest.clone(),
            dir: dir.clone(),
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
            .ok_or(tr!("нет экспорта memory"))?;
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
            dir,
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
                error: Some(tf!(
                    "плагин остановлен: {reason}",
                    "plugin stopped: {reason}"
                )),
                broken: true,
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
                broken: false,
            },
            Err(e) => {
                effects.retain(|e| matches!(e, Effect::Log(..)));
                let panic = effects.iter().rev().find_map(|e| match e {
                    Effect::Log(text, true) if text.starts_with(tr!("паника")) => {
                        Some(text.clone())
                    }
                    _ => None,
                });
                let reason = match panic {
                    Some(text) => text,
                    None if e.contains("fuel") => tr!("превышен лимит работы").to_owned(),
                    None => e,
                };
                self.broken = Some(reason.clone());
                Outcome {
                    effects,
                    sel: None,
                    error: Some(reason),
                    broken: true,
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
}

pub struct Job {
    pub message: Value,
    pub snap: Option<Snapshot>,
    pub doc: Option<(u64, u64)>,
    pub wake: Box<dyn Fn() + Send>,
}

pub struct Done {
    pub plugin: usize,
    pub doc: Option<(u64, u64)>,
    pub outcome: Outcome,
    pub took: std::time::Duration,
}

pub struct Worker {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub broken: Option<String>,
    pub busy: usize,
    tx: std::sync::mpsc::Sender<Job>,
}

impl Worker {
    pub fn spawn(plugin: usize, mut loaded: Loaded, done: std::sync::mpsc::Sender<Done>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let manifest = loaded.manifest.clone();
        let dir = loaded.dir.clone();
        std::thread::spawn(move || {
            for job in rx {
                let started = std::time::Instant::now();
                let outcome = loaded.send(&job.message, job.snap);
                let sent = done.send(Done {
                    plugin,
                    doc: job.doc,
                    outcome,
                    took: started.elapsed(),
                });
                (job.wake)();
                if sent.is_err() {
                    break;
                }
            }
        });
        Self {
            manifest,
            dir,
            broken: None,
            busy: 0,
            tx,
        }
    }

    pub fn run(&mut self, job: Job) {
        if self.tx.send(job).is_ok() {
            self.busy += 1;
        }
    }

    pub fn wants(&self, event: &str) -> bool {
        self.manifest.events.iter().any(|e| e == event)
    }
}

pub fn manifests(dir: &Path) -> Vec<(PathBuf, Result<Manifest, String>)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("plugin.json").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|d| {
            let manifest = std::fs::read_to_string(d.join("plugin.json"))
                .map_err(|e| e.to_string())
                .and_then(|t| {
                    serde_json::from_str::<Manifest>(&t).map_err(|e| format!("plugin.json: {e}"))
                });
            (d, manifest)
        })
        .collect()
}

pub fn load_all(dir: &Path) -> (Vec<Loaded>, Vec<String>) {
    load_enabled(dir, &[])
}

pub fn load_enabled(dir: &Path, off: &[String]) -> (Vec<Loaded>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    let dirs: Vec<PathBuf> = manifests(dir)
        .into_iter()
        .filter(|(_, m)| !m.as_ref().is_ok_and(|m| off.contains(&m.id)))
        .map(|(d, _)| d)
        .collect();
    for dir in dirs {
        match Loaded::load(&dir) {
            Ok(plugin)
                if loaded
                    .iter()
                    .any(|p: &Loaded| p.manifest.id == plugin.manifest.id) =>
            {
                errors.push(tf!(
                    "{}: id «{}» уже занят другим плагином",
                    "{}: id \"{}\" is already used by another plugin",
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
            panels: Vec::new(),
            settings: Vec::new(),
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

    fn test_host(snap: Snapshot, dir: PathBuf) -> Host {
        let mut manifest = plugin().manifest;
        manifest.panels = serde_json::from_value(json!([{"id":"main","title":"Главная"}])).unwrap();
        manifest.settings = serde_json::from_value(json!([
            {"key":"name","title":"Имя","type":"text","default":"мир"}
        ]))
        .unwrap();
        Host {
            snap: Some(snap),
            effects: Vec::new(),
            reply: Vec::new(),
            manifest,
            dir,
            limits: StoreLimits::default(),
        }
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
    fn decorations_and_permissions() {
        let mut host = test_host(snap("one\ntwo\nthree", None), std::env::temp_dir());
        request(
            &mut host,
            "decorate",
            &json!({"key":"k","items":[
                {"kind":"highlight","start":2,"end":99,"color":"#ff0000","message":"m"},
                {"kind":"gutter","line":50},
                {"kind":"underline","start":5,"end":4}
            ]}),
        )
        .unwrap();
        let Some(Effect::Decorate(key, items)) = host.effects.pop() else {
            panic!("нет декораций");
        };
        assert_eq!(key, "k");
        assert_eq!(
            (items[0].start, items[0].end, items[0].color),
            (2, 13, Some([255, 0, 0]))
        );
        assert_eq!((items[1].kind, items[1].start), (DecoKind::Gutter, 2));
        assert_eq!((items[2].start, items[2].end), (4, 5));
        for method in ["http", "hosts", "databases", "remote_read", "db_query"] {
            let err =
                request(&mut host, method, &json!({"url":"https://example.com"})).unwrap_err();
            assert!(err.contains("нет разрешения"), "{method}: {err}");
        }
        host.manifest.permissions.push("network".into());
        let err = request(&mut host, "http", &json!({"url":"file:///etc/passwd"})).unwrap_err();
        assert!(err.contains("http"));
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
        let mut host = test_host(snap("x", Some(dir.clone())), dir.clone());
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
        host.manifest.permissions.clear();
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
    fn dialogs_panels_and_settings() {
        let dir = std::env::temp_dir().join(format!("tror-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut host = test_host(snap("x", None), dir.clone());
        request(
            &mut host,
            "prompt",
            &json!({"id":"q","title":"Имя?","value":"a"}),
        )
        .unwrap();
        request(
            &mut host,
            "pick",
            &json!({"id":"p","title":"Что?","items":["x","y"]}),
        )
        .unwrap();
        request(
            &mut host,
            "panel",
            &json!({"id":"main","ui":{"type":"text","text":"привет"}}),
        )
        .unwrap();
        assert!(request(&mut host, "panel", &json!({"id":"nope"})).is_err());
        request(&mut host, "show_panel", &json!({"id":"main"})).unwrap();
        assert_eq!(
            host.effects,
            vec![
                Effect::Prompt("q".into(), "Имя?".into(), "a".into()),
                Effect::Pick("p".into(), "Что?".into(), vec!["x".into(), "y".into()]),
                Effect::Panel("main".into(), json!({"type":"text","text":"привет"})),
                Effect::ShowPanel("main".into()),
            ]
        );
        assert_eq!(
            request(&mut host, "setting", &json!({"key":"name"})).unwrap(),
            json!("мир")
        );
        let mut values = settings_of(&dir, &host.manifest);
        values.insert("name".into(), json!("Jozef"));
        save_settings(&dir, &values).unwrap();
        assert_eq!(
            request(&mut host, "setting", &json!({"key":"name"})).unwrap(),
            json!("Jozef")
        );
        assert!(request(&mut host, "setting", &json!({"key":"x"})).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn worker_runs_jobs_in_background() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut worker = Worker::spawn(3, plugin(), tx);
        let woke = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = woke.clone();
        worker.run(Job {
            message: json!({"method":"command","params":{"id":"x"}}),
            snap: Some(snap("hello world", None)),
            doc: Some((7, 2)),
            wake: Box::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst)),
        });
        assert_eq!(worker.busy, 1);
        let done = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!((done.plugin, done.doc), (3, Some((7, 2))));
        assert_eq!(done.outcome.effects.len(), 2);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(woke.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn memory_growth_is_limited() {
        let mut p = plugin();
        assert_eq!(p.send(&json!({"method":"grow"}), None).error, None);
    }

    #[test]
    fn replace_keeps_selection_consistent() {
        let mut host = test_host(snap("abcdef", None), PathBuf::new());
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
