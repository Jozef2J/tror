use std::collections::{BTreeSet, HashMap, HashSet};
use crate::widgets::Lit;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Button, ComboBox, Key, Rect, RichText, ScrollArea, Sense, TextEdit, Ui};

use crate::app::{dialog, dialog_button, size_label};
use crate::remote::{self, Attrs, Cmd, Connection, Entry, Event, Profile, Protocol, Store};
use crate::theme::Palette;
mod dialogs;
mod listing;
mod search;
mod sync_ui;
mod transfers;

use crate::widgets::{self, chevron, date_label, dots_button, paint_icon, refresh_button};

pub enum PanelEvent {
    Open {
        profile: String,
        path: String,
        data: Vec<u8>,
        line: Option<usize>,
    },
    Uploaded {
        doc: (u64, u64),
        path: String,
    },
    Conflict {
        profile: String,
        doc: (u64, u64),
        path: String,
        server: Vec<u8>,
    },
    Listed {
        profile: String,
        dir: String,
        names: Vec<String>,
    },
    Toast(String),
    Compare {
        profile: String,
        path: String,
        server: Vec<u8>,
    },
    Console(Box<remote::TermProfile>),
    Relay {
        target: u8,
        path: String,
        data: Vec<u8>,
    },
}

enum Walk {
    Size,
    Sync(PathBuf),
    Download(PathBuf),
    Relay(u8, String),
}

#[derive(Clone)]
pub struct Dragged {
    pub panel: u8,
    pub paths: Vec<(String, bool)>,
}

enum Prompt {
    NewFolder(String, String),
    NewFile(String, String),
    Rename { from: String, name: String },
    Delete(Vec<(String, bool)>),
    Extract(Vec<String>, String),
    Move(Vec<(String, String)>),
}

struct AttrsDialog {
    items: Vec<(String, bool)>,
    loaded: bool,
    info: Option<Attrs>,
    mode: u32,
    octal: String,
    recursive: bool,
    dirs: bool,
    files: bool,
    mtime: String,
    mtime_was: String,
}

struct ProfileEditor {
    drafts: Vec<Profile>,
    index: usize,
    reveal: bool,
    probe: Option<Receiver<Result<String, String>>>,
    result: Option<Result<String, String>>,
    note: Option<Result<String, String>>,
}

struct Saver {
    conn: Connection,
    connected: bool,
    queue: Vec<Cmd>,
}

struct Transfer {
    cmd: Cmd,
    label: String,
    error: String,
}

impl Transfer {
    fn path(&self) -> &str {
        match &self.cmd {
            Cmd::Download(path) | Cmd::Upload { path, .. } | Cmd::Touch { path, .. } => path,
            _ => "",
        }
    }
}

struct Batch {
    folder: Option<PathBuf>,
    target: String,
    downloads: bool,
    uploads: bool,
    start: std::time::Instant,
    files: usize,
    done: usize,
    total: u64,
    bytes: u64,
    sizes: HashMap<String, u64>,
    failed: HashSet<String>,
    walking: usize,
    cancelled: bool,
    lost: bool,
    finished: Option<std::time::Duration>,
    open: bool,
    drag: bool,
}

impl Batch {
    fn new() -> Batch {
        Batch {
            folder: None,
            target: String::new(),
            downloads: false,
            uploads: false,
            start: std::time::Instant::now(),
            files: 0,
            done: 0,
            total: 0,
            bytes: 0,
            sizes: HashMap::new(),
            failed: HashSet::new(),
            walking: 0,
            cancelled: false,
            lost: false,
            finished: None,
            open: true,
            drag: false,
        }
    }

    fn add(&mut self, path: String, size: u64) {
        self.files += 1;
        self.total += size;
        self.sizes.insert(path, size);
    }

    fn noun(&self) -> &'static str {
        match (self.downloads, self.uploads) {
            (true, false) => tr!("Скачивание с сервера"),
            (false, true) => tr!("Загрузка на сервер"),
            _ => tr!("Передача файлов"),
        }
    }

    fn idle(&self) -> bool {
        self.walking == 0 && self.sizes.len() == self.failed.len()
    }
}

fn duration_label(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

struct Row {
    path: String,
    name: String,
    depth: usize,
    entry: Option<Entry>,
}

pub struct RemotePanel {
    pub visible: bool,
    pub id: u8,
    pub tree: bool,
    pub peers: Vec<(u8, String)>,
    pub outgoing: Option<(u8, Vec<(String, bool)>)>,
    pub incoming: Option<Dragged>,
    relay: HashMap<String, (u8, String)>,
    sort: crate::widgets::Sort,
    area: Rect,
    walks: HashMap<String, Walk>,
    sync: Option<crate::remote::sync::Dialog>,
    sync_times: HashMap<PathBuf, u64>,
    no_times: bool,
    pending: usize,
    progress: Option<(u64, u64)>,
    store_seen: u64,
    pub store: Store,
    selected: usize,
    conn: Option<Connection>,
    connected: bool,
    queue: Vec<Cmd>,
    transfers: std::collections::VecDeque<Transfer>,
    running: Option<Transfer>,
    failed: Vec<Transfer>,
    paused: bool,
    saver: Option<Saver>,
    cwd: String,
    path_input: String,
    entries: Vec<Entry>,
    children: HashMap<String, Vec<Entry>>,
    expanded: BTreeSet<String>,
    loading: HashSet<String>,
    marked: BTreeSet<String>,
    anchor: Option<String>,
    busy: Option<String>,
    prompt: Option<Prompt>,
    clip: Option<(Vec<String>, bool)>,
    notes: Vec<PanelEvent>,
    attrs: Option<AttrsDialog>,
    host_key: Option<(String, bool)>,
    editor: Option<ProfileEditor>,
    save_to: HashMap<String, PathBuf>,
    compare: HashSet<String>,
    batch: Option<Batch>,
    pub out_drag: Option<(PathBuf, Vec<PathBuf>)>,
    pub drag_done: std::sync::Arc<std::sync::Mutex<Option<bool>>>,
    notice: Option<String>,
    batch_height: f32,
    batch_placed: bool,
    batch_owned: bool,
    drop_zone: Option<Rect>,
    drop_dirs: Vec<(Rect, String)>,
    pub external_hover: Option<String>,
    pub want_search: bool,
    search_query: String,
    search_root: String,
    search_content: bool,
    search_case: bool,
    search_word: bool,
    search_regex: bool,
    search_excludes: String,
    search_job: Option<remote::SearchJob>,
    hits: Vec<(String, Option<usize>, String)>,
    search_status: String,
    open_at: HashMap<String, usize>,
}

const DEFAULT_EXCLUDES: &str = "bitrix/cache, bitrix/managed_cache, bitrix/stack_cache, bitrix/backup, upload/resize_cache, node_modules, .git";

impl RemotePanel {
    pub fn new(id: u8) -> Self {
        let path = eframe::storage_dir("Tror").map(|dir| dir.join("connections.json"));
        let store = Store::load(path);
        Self {
            visible: false,
            id,
            tree: false,
            peers: Vec::new(),
            outgoing: None,
            incoming: None,
            relay: HashMap::new(),
            sort: Default::default(),
            area: Rect::NOTHING,
            walks: HashMap::new(),
            sync: None,
            sync_times: HashMap::new(),
            no_times: false,
            pending: 0,
            progress: None,
            store_seen: Store::generation(),
            want_search: false,
            search_query: String::new(),
            search_root: String::new(),
            search_content: true,
            search_case: false,
            search_word: false,
            search_regex: false,
            search_excludes: store
                .search_excludes
                .clone()
                .unwrap_or_else(|| DEFAULT_EXCLUDES.to_owned()),
            search_job: None,
            hits: Vec::new(),
            search_status: String::new(),
            open_at: HashMap::new(),
            store,
            selected: 0,
            conn: None,
            connected: false,
            queue: Vec::new(),
            transfers: Default::default(),
            running: None,
            failed: Vec::new(),
            paused: false,
            saver: None,
            cwd: String::new(),
            path_input: String::new(),
            entries: Vec::new(),
            children: HashMap::new(),
            expanded: BTreeSet::new(),
            loading: HashSet::new(),
            marked: BTreeSet::new(),
            anchor: None,
            busy: None,
            prompt: None,
            clip: None,
            notes: Vec::new(),
            attrs: None,
            host_key: None,
            editor: None,
            compare: HashSet::new(),
            save_to: HashMap::new(),
            batch: None,
            out_drag: None,
            drag_done: Default::default(),
            notice: None,
            batch_height: 0.0,
            batch_placed: false,
            batch_owned: false,
            drop_zone: None,
            drop_dirs: Vec::new(),
            external_hover: None,
        }
    }

    pub fn quick_files(&self) -> Option<(String, Vec<String>)> {
        if !self.connected {
            return None;
        }
        let name = self.store.profiles.get(self.selected)?.name.clone();
        let mut files: Vec<String> = self
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| remote::join(&self.cwd, &e.name))
            .collect();
        for (dir, list) in &self.children {
            files.extend(list.iter().filter(|e| !e.is_dir).map(|e| remote::join(dir, &e.name)));
        }
        files.sort();
        files.dedup();
        Some((name, files))
    }

    pub fn connected_profile(&self) -> Option<&str> {
        self.connected
            .then(|| self.conn.as_ref().map(|c| c.profile.as_str()))
            .flatten()
    }

    pub fn compare_remote(&mut self, path: String) {
        let name = remote::file_name(&path).to_owned();
        self.compare.insert(path.clone());
        self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
    }

    pub fn open_remote(&mut self, path: String) {
        let name = remote::file_name(&path).to_owned();
        self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
    }

    pub fn selected_profile(&self) -> Option<(&Profile, Option<String>)> {
        let profile = self.store.profiles.get(self.selected)?;
        Some((profile, self.known(profile)))
    }

    fn known(&self, profile: &Profile) -> Option<String> {
        let key = format!("{}:{}", profile.host.trim(), profile.port);
        self.store.known_hosts.get(&key).cloned()
    }

    pub fn trust(&mut self, host: String, fingerprint: String) -> Result<(), String> {
        self.store.known_hosts.insert(host, fingerprint);
        self.store.save()
    }

    pub fn modal_open(&self) -> bool {
        self.prompt.is_some()
            || self.host_key.is_some()
            || self.editor.is_some()
            || self.attrs.is_some()
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn in_use(&self) -> bool {
        self.conn.is_some() || self.saver.is_some() || self.pending > 0
    }

    pub fn can_upload(&self) -> bool {
        self.visible && self.connected
    }

    pub fn drop_zone(&self) -> Option<Rect> {
        self.drop_zone.filter(|_| self.can_upload())
    }

    fn entry_size(&self, path: &str) -> u64 {
        let dir = remote::parent(path);
        let name = remote::file_name(path);
        self.children
            .get(&dir)
            .into_iter()
            .chain((dir == self.cwd).then_some(&self.entries))
            .flatten()
            .find(|e| e.name == name)
            .map_or(0, |e| e.size)
    }

    fn batch(&mut self) -> &mut Batch {
        if self.batch.as_ref().is_none_or(|b| b.finished.is_some()) {
            self.batch = Some(Batch::new());
        }
        let batch = self.batch.as_mut().expect("batch");
        batch.open = true;
        batch
    }

    pub fn download_into(&mut self, items: Vec<(String, bool)>, folder: &Path) {
        let what = match items.as_slice() {
            [(path, _)] => remote::file_name(path).to_owned(),
            _ => tf!("объектов: {}", "items: {}", items.len()),
        };
        self.push_log(
            tf!("Скачивание начато: {what} → {}", "Download started: {what} → {}", folder.display()),
            false,
        );
        let batch = self.batch();
        batch.downloads = true;
        if batch.folder.is_none() && !batch.uploads {
            batch.folder = Some(folder.to_path_buf());
            batch.target = folder.display().to_string();
        } else if batch.folder.as_deref() != Some(folder) {
            batch.folder = None;
        }
        for (path, dir) in &items {
            let size = if *dir { 0 } else { self.entry_size(path) };
            if let Some(batch) = &mut self.batch {
                if *dir {
                    batch.walking += 1;
                } else {
                    batch.add(path.clone(), size);
                }
            }
        }
        for (path, dir) in items {
            if dir {
                self.walks.insert(path.clone(), Walk::Download(folder.to_path_buf()));
                let name = remote::file_name(&path).to_owned();
                self.send(Cmd::Walk(path), tf!("Чтение папки {name}…", "Reading folder {name}…"));
            } else {
                let name = remote::file_name(&path).to_owned();
                self.save_to.insert(path.clone(), folder.join(&name));
                self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
            }
        }
    }

    pub fn dragging(&self, ctx: &egui::Context) -> Option<Vec<(String, bool)>> {
        egui::DragAndDrop::payload::<Dragged>(ctx)
            .filter(|d| d.panel == self.id)
            .map(|d| d.paths.clone())
    }

    pub fn prepare_drag_out(&mut self, items: Vec<(String, bool)>) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let root = std::env::temp_dir().join("trorpad-drag");
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let old = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 3600);
                if old {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        let temp = root.join(stamp.to_string());
        if std::fs::create_dir_all(&temp).is_err() {
            return;
        }
        let tops = items
            .iter()
            .map(|(path, _)| temp.join(remote::file_name(path)))
            .collect();
        self.download_into(items, &temp);
        if let Some(batch) = &mut self.batch {
            batch.drag = true;
            batch.folder = None;
            batch.target = tr!("для перетаскивания").to_owned();
        }
        *self.drag_done.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.out_drag = Some((temp, tops));
    }

    pub fn abandon_drag_out(&mut self) {
        if self.out_drag.take().is_none() {
            return;
        }
        if self.batch.as_ref().is_some_and(|b| b.drag) {
            self.cancel_batch();
            self.batch = None;
        }
        *self.drag_done.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.notice = Some(
            tr!("Перетаскивание отменено: кнопку мыши отпустили раньше, чем файлы скачались. Держите её, пока окно передачи не покажет «Готово», или используйте «Скачать в папку…».")
                .to_owned(),
        );
    }

    pub fn drag_out_ready(&self) -> Option<Vec<PathBuf>> {
        let (temp, tops) = self.out_drag.as_ref()?;
        let busy = self.save_to.values().any(|p| p.starts_with(temp))
            || self
                .walks
                .values()
                .any(|w| matches!(w, Walk::Download(f) if f.starts_with(temp)));
        (!busy).then(|| tops.clone())
    }

    pub fn drop_target(&self, pos: egui::Pos2) -> Option<String> {
        if !self.can_upload() {
            return None;
        }
        self.drop_zone.filter(|z| z.contains(pos))?;
        Some(
            self.drop_dirs
                .iter()
                .find(|(rect, _)| rect.contains(pos))
                .map(|(_, dir)| dir.clone())
                .unwrap_or_else(|| self.cwd.clone()),
        )
    }

    fn push_log(&mut self, text: String, error: bool) {
        crate::trace::line(text, error);
    }

    pub fn site_url(&self, profile: &str, path: &str) -> Option<String> {
        let profile = self.store.profiles.iter().find(|p| p.name == profile)?;
        crate::preview::remote_url(&profile.url, &profile.directory, path)
    }

    fn sftp(&self) -> bool {
        self.conn.as_ref().is_some_and(|c| {
            self.store
                .profiles
                .iter()
                .any(|p| p.name == c.profile && p.protocol == Protocol::Sftp)
        })
    }

    fn send(&mut self, cmd: Cmd, busy: impl Into<String>) {
        let bulk = match &cmd {
            Cmd::Download(path) => self.save_to.contains_key(path) || self.relay.contains_key(path),
            Cmd::Upload { doc, .. } => doc.is_none(),
            Cmd::Touch { .. } => true,
            _ => false,
        };
        if bulk {
            self.transfers.push_back(Transfer {
                cmd,
                label: busy.into(),
                error: String::new(),
            });
            self.pump();
            return;
        }
        self.dispatch(cmd, busy);
    }

    fn pump(&mut self) {
        if self.running.is_some() || self.paused || !self.connected {
            return;
        }
        if let Some(transfer) = self.transfers.pop_front() {
            self.dispatch(transfer.cmd.clone(), transfer.label.clone());
            self.running = Some(transfer);
        }
    }

    fn settle(&mut self, path: &str, error: Option<String>) {
        let touch = matches!(self.running.as_ref().map(|t| &t.cmd), Some(Cmd::Touch { .. }));
        if self
            .running
            .as_ref()
            .is_some_and(|t| t.path() == path || (touch && remote::parent(t.path()) == path))
            && let Some(mut transfer) = self.running.take()
            && let Some(error) = error
        {
            transfer.error = error;
            self.failed.push(transfer);
        }
        self.pump();
    }

    fn retry_failed(&mut self) {
        for mut transfer in std::mem::take(&mut self.failed) {
            transfer.error.clear();
            self.transfers.push_back(transfer);
        }
        self.pump();
    }

    fn forget(&mut self, transfers: Vec<Transfer>) {
        for transfer in transfers {
            self.save_to.remove(transfer.path());
            self.relay.remove(transfer.path());
        }
    }

    fn dispatch(&mut self, cmd: Cmd, busy: impl Into<String>) {
        if let Some(conn) = &self.conn
            && conn.tx.send(cmd).is_ok()
        {
            self.busy = Some(busy.into());
            self.pending += 1;
        }
    }

    fn finished(&mut self) {
        self.pending = self.pending.saturating_sub(1);
        self.progress = None;
        if self.pending == 0 {
            self.busy = None;
        }
    }

    fn list(&mut self, path: String) {
        self.loading.remove(&path);
        self.send(Cmd::List(path), tr!("Чтение каталога…"));
    }

    fn load(&mut self, path: String) {
        if self.loading.insert(path.clone()) {
            self.send(Cmd::List(path), tr!("Чтение каталога…"));
        }
    }

    fn refresh_dir(&mut self, dir: &str) {
        if dir == self.cwd {
            self.list(self.cwd.clone());
        } else if self.tree && self.children.contains_key(dir) {
            self.load(dir.to_owned());
        }
    }

    fn disconnect(&mut self) {
        self.conn = None;
        self.connected = false;
        self.queue.clear();
        self.transfers.clear();
        self.running = None;
        self.failed.clear();
        self.paused = false;
        self.entries.clear();
        self.children.clear();
        self.loading.clear();
        self.marked.clear();
        self.busy = None;
        self.pending = 0;
        self.progress = None;
        self.relay.clear();
        self.walks.clear();
        self.host_key = None;
        self.attrs = None;
    }

    fn connect(&mut self, ctx: &egui::Context) {
        let Some((profile, known)) = self.selected_profile() else {
            return;
        };
        let ctx = ctx.clone();
        self.conn = Some(remote::connect(profile, known, move || {
            ctx.request_repaint()
        }));
        self.connected = false;
        self.entries.clear();
        self.children.clear();
        self.expanded.clear();
        self.busy = Some(tr!("Подключение…").into());
    }

    pub fn upload(
        &mut self,
        ctx: &egui::Context,
        profile: &str,
        path: &str,
        data: Vec<u8>,
        doc: (u64, u64),
        expect: Option<u64>,
    ) -> Result<&'static str, String> {
        let cmd = Cmd::Upload {
            path: path.to_owned(),
            data,
            doc: Some(doc),
            expect,
        };
        let busy = tf!("Загрузка {}…", "Uploading {}…", remote::file_name(path));
        if let Some(conn) = &self.conn
            && conn.profile == profile
        {
            if self.connected {
                self.send(cmd, busy);
                return Ok(tr!("Загрузка на сервер…"));
            }
            self.queue.push(cmd);
            return Ok(tr!("Файл будет загружен после подключения…"));
        }
        let Some(index) = self.store.profiles.iter().position(|p| p.name == profile) else {
            return Err(tf!("профиль «{profile}» не найден", "profile \"{profile}\" not found"));
        };
        if self.conn.is_none() {
            self.selected = index;
            self.connect(ctx);
            self.queue.push(cmd);
            return Ok(tr!("Подключаюсь к серверу, чтобы сохранить файл…"));
        }
        match &mut self.saver {
            Some(saver) if saver.conn.profile == profile => {
                if saver.connected {
                    let _ = saver.conn.tx.send(cmd);
                } else {
                    saver.queue.push(cmd);
                }
            }
            _ => {
                let target = &self.store.profiles[index];
                let known = self.known(target);
                let ctx = ctx.clone();
                self.saver = Some(Saver {
                    conn: remote::connect(target, known, move || ctx.request_repaint()),
                    connected: false,
                    queue: vec![cmd],
                });
            }
        }
        Ok(tr!("Сохраняю через отдельное подключение…"))
    }

    pub fn upload_local(&mut self, paths: Vec<PathBuf>) {
        let dir = self.cwd.clone();
        self.upload_local_to(paths, &dir);
    }

    pub fn upload_local_to(&mut self, paths: Vec<PathBuf>, dir: &str) {
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        let mut stack: Vec<(PathBuf, String)> = paths
            .into_iter()
            .filter_map(|p| {
                let name = p.file_name()?.to_string_lossy().into_owned();
                Some((p, name))
            })
            .collect();
        while let Some((local, relative)) = stack.pop() {
            if !local.is_dir() {
                files.push((local, relative));
                continue;
            }
            match std::fs::read_dir(&local) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        stack.push((entry.path(), format!("{relative}/{name}")));
                    }
                }
                Err(e) => {
                    self.push_log(tf!("Не удалось прочитать папку {relative}: {e}", "Could not read folder {relative}: {e}"), true)
                }
            }
        }
        files.sort_by(|a, b| a.1.cmp(&b.1));
        if files.is_empty() {
            return;
        }
        let what = match files.as_slice() {
            [(_, relative)] => relative.clone(),
            _ => tf!("файлов: {}", "files: {}", files.len()),
        };
        self.push_log(tf!("Загрузка начата: {what} → {dir}", "Upload started: {what} → {dir}"), false);
        let batch = self.batch();
        batch.uploads = true;
        if batch.target.is_empty() || batch.downloads {
            batch.target = dir.to_owned();
        }
        if batch.downloads {
            batch.folder = None;
        }
        for (local, relative) in files {
            match std::fs::read(&local) {
                Ok(data) => {
                    let path = remote::join(dir, &relative);
                    if let Some(batch) = &mut self.batch {
                        batch.add(path.clone(), data.len() as u64);
                    }
                    self.send(
                        Cmd::Upload {
                            path,
                            data,
                            doc: None,
                            expect: None,
                        },
                        tf!("Загрузка {relative}…", "Uploading {relative}…"),
                    );
                }
                Err(e) => self.push_log(tf!("Не удалось прочитать {relative}: {e}", "Could not read {relative}: {e}"), true),
            }
        }
    }

    fn drain(conn: &Connection) -> (Vec<Event>, bool) {
        let mut events = Vec::new();
        loop {
            match conn.rx.try_recv() {
                Ok(event) => events.push(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => return (events, false),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return (events, true),
            }
        }
    }

    fn poll_saver(&mut self, out: &mut Vec<PanelEvent>) {
        let Some(saver) = &mut self.saver else {
            return;
        };
        let (events, dead) = Self::drain(&saver.conn);
        let mut logs = Vec::new();
        let close = dead;
        for event in events {
            match event {
                Event::Connected { .. } => {
                    saver.connected = true;
                    for cmd in saver.queue.drain(..) {
                        let _ = saver.conn.tx.send(cmd);
                    }
                }
                Event::Uploaded {
                    path,
                    doc: Some(doc),
                } => out.push(PanelEvent::Uploaded { doc, path }),
                Event::Conflict { path, doc, server } => out.push(PanelEvent::Conflict {
                    profile: saver.conn.profile.clone(),
                    doc,
                    path,
                    server,
                }),
                Event::Log(text, error) => logs.push((text, error)),
                Event::HostKey { .. } => {
                    let _ = saver.conn.tx.send(Cmd::TrustHost(false));
                    out.push(PanelEvent::Toast(
                        tr!("Не сохранено: ключ сервера нужно подтвердить — подключитесь к нему в панели").into(),
                    ));
                }
                Event::Error(e) | Event::Closed(e) | Event::Failed { error: e, .. } => {
                    out.push(PanelEvent::Toast(tf!("Не сохранено: {e}", "Not saved: {e}")));
                }
                _ => {}
            }
        }
        if close {
            self.saver = None;
        }
        for (text, error) in logs {
            self.push_log(text, error);
        }
    }

    pub fn peer_label(&self) -> Option<String> {
        let conn = self.conn.as_ref().filter(|_| self.connected)?;
        Some(format!("{}: {}", conn.profile, self.cwd))
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn reopen(&mut self, ctx: &egui::Context, profile: &str, path: &str) -> Result<(), String> {
        let cmd = Cmd::Download(path.to_owned());
        let busy = tf!("Скачивание {}…", "Downloading {}…", remote::file_name(path));
        if self.serves(profile) {
            if self.connected {
                self.send(cmd, busy);
            } else {
                self.queue.push(cmd);
            }
            return Ok(());
        }
        if self.conn.is_some() {
            return Err(tf!("панель подключена к другому серверу, а файл лежит на «{profile}»", "the panel is connected to another server, and the file is on \"{profile}\""));
        }
        let index = self
            .store
            .profiles
            .iter()
            .position(|p| p.name == profile)
            .ok_or_else(|| tf!("профиль «{profile}» не найден", "profile \"{profile}\" not found"))?;
        self.selected = index;
        self.connect(ctx);
        self.queue.push(cmd);
        Ok(())
    }

    pub fn rename_file(&mut self, profile: &str, from: &str, to: &str) -> Result<(), String> {
        if !self.serves(profile) || !self.connected {
            return Err(tf!("сначала подключитесь к «{profile}»", "connect to \"{profile}\" first"));
        }
        self.send(Cmd::Rename(from.to_owned(), to.to_owned()), tr!("Переименование…"));
        Ok(())
    }

    pub fn serves(&self, profile: &str) -> bool {
        self.conn.as_ref().is_some_and(|c| c.profile == profile)
    }

    pub fn relay(&mut self, items: Vec<(String, bool)>, target: u8, target_dir: &str) {
        for (path, dir) in items {
            let name = remote::file_name(&path).to_owned();
            if dir {
                self.walks
                    .insert(path.clone(), Walk::Relay(target, target_dir.to_owned()));
                self.send(Cmd::Walk(path), tf!("Чтение папки {name}…", "Reading folder {name}…"));
                continue;
            }
            self.relay
                .insert(path.clone(), (target, remote::join(target_dir, &name)));
            self.send(Cmd::Download(path), tf!("Передача {name}…", "Transferring {name}…"));
        }
    }

    pub fn receive(&mut self, path: String, data: Vec<u8>) {
        let name = remote::file_name(&path).to_owned();
        self.send(
            Cmd::Upload {
                path,
                data,
                doc: None,
                expect: None,
            },
            tf!("Приём {name}…", "Receiving {name}…"),
        );
    }

    pub fn poll(&mut self) -> Vec<PanelEvent> {
        if self.store_seen != Store::generation() {
            self.store_seen = Store::generation();
            self.store = Store::load(self.store.path().map(|p| p.to_path_buf()));
            self.selected = self
                .selected
                .min(self.store.profiles.len().saturating_sub(1));
        }
        let mut out = std::mem::take(&mut self.notes);
        if let Some(editor) = &mut self.editor
            && let Some(rx) = &editor.probe
            && let Ok(result) = rx.try_recv()
        {
            editor.probe = None;
            editor.result = Some(result);
        }
        self.poll_saver(&mut out);
        if let Some(job) = &self.search_job {
            let mut finished = false;
            while let Ok(event) = job.rx.try_recv() {
                match event {
                    remote::SearchEvent::Hit { path, line, text } => {
                        self.hits.push((path, line, text))
                    }
                    remote::SearchEvent::Status(text) => self.search_status = text,
                    remote::SearchEvent::Done(text) => {
                        self.search_status = text;
                        finished = true;
                    }
                    remote::SearchEvent::Failed(e) => {
                        self.search_status = tf!("ошибка: {e}", "error: {e}");
                        finished = true;
                    }
                }
            }
            if finished {
                let status = self.search_status.clone();
                self.push_log(tf!("Поиск на сервере: {status}", "Server search: {status}"), false);
                self.search_job = None;
            }
        }
        let (events, dead) = match &self.conn {
            Some(conn) => Self::drain(conn),
            None => (Vec::new(), false),
        };
        let profile = self
            .conn
            .as_ref()
            .map(|c| c.profile.clone())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Log(text, error) => self.push_log(text, error),
                Event::HostKey {
                    fingerprint,
                    changed,
                } => {
                    self.finished();
                    self.host_key = Some((fingerprint, changed));
                }
                Event::Connected { home, fingerprint } => {
                    self.connected = true;
                    self.no_times = false;
                    self.pending = 0;
                    if let (Some(print), Some(p)) =
                        (fingerprint, self.store.profiles.get(self.selected))
                    {
                        let key = format!("{}:{}", p.host.trim(), p.port);
                        if let Err(e) = self.trust(key, print) {
                            out.push(PanelEvent::Toast(tf!("Не удалось сохранить ключ: {e}", "Could not save the key: {e}")));
                        }
                    }
                    for cmd in std::mem::take(&mut self.queue) {
                        self.send(cmd, tr!("Загрузка…"));
                    }
                    self.list(home);
                    self.pump();
                }
                Event::Listing { path, entries } => {
                    self.finished();
                    out.push(PanelEvent::Listed {
                        profile: profile.clone(),
                        dir: path.clone(),
                        names: entries.iter().map(|e| e.name.clone()).collect(),
                    });
                    let inside = |p: &str, dir: &str| {
                        remote::parent(p) == dir
                    };
                    if self.loading.remove(&path) {
                        self.marked.retain(|m| {
                            !inside(m, &path)
                                || entries.iter().any(|e| remote::join(&path, &e.name) == *m)
                        });
                        if path == self.cwd {
                            self.entries = entries.clone();
                        }
                        self.children.insert(path, entries);
                        continue;
                    }
                    if path != self.cwd {
                        self.marked.clear();
                        self.anchor = None;
                        self.expanded.clear();
                        self.children.clear();
                    }
                    self.path_input = path.clone();
                    self.cwd = path.clone();
                    self.marked.retain(|m| {
                        !inside(m, &path)
                            || entries.iter().any(|e| remote::join(&path, &e.name) == *m)
                    });
                    self.children.insert(path, entries.clone());
                    self.entries = entries;
                    let stale: Vec<String> = self
                        .expanded
                        .iter()
                        .filter(|dir| *dir != &self.cwd)
                        .cloned()
                        .collect();
                    for dir in stale {
                        self.load(dir);
                    }
                }
                Event::Downloaded { path, data } => {
                    self.finished();
                    self.settle(&path, None);
                    if let Some((target, to)) = self.relay.remove(&path) {
                        out.push(PanelEvent::Relay {
                            target,
                            path: to,
                            data,
                        });
                        continue;
                    }
                    if self.compare.remove(&path) {
                        out.push(PanelEvent::Compare {
                            profile: profile.clone(),
                            path,
                            server: data,
                        });
                        continue;
                    }
                    let in_batch = self.batch.as_ref().is_some_and(|b| b.sizes.contains_key(&path));
                    match self.save_to.remove(&path) {
                        Some(local) if in_batch => {
                            let result = crate::text::document::write_safely(&local, &data);
                            if result.is_ok()
                                && let Some(mtime) = self.sync_times.remove(&local)
                            {
                                crate::remote::sync::set_mtime(&local, mtime);
                            }
                            if let Some(batch) = &mut self.batch {
                                match &result {
                                    Ok(()) => {
                                        batch.done += 1;
                                        batch.bytes += batch.sizes.remove(&path).unwrap_or(0);
                                    }
                                    Err(_) => {
                                        batch.failed.insert(path.clone());
                                    }
                                }
                            }
                            if let Err(e) = result {
                                self.push_log(tf!("Не удалось записать {}: {e}", "Could not write {}: {e}", local.display()), true);
                            }
                        }
                        Some(local) => {
                            let (text, error) = match crate::text::document::write_safely(&local, &data) {
                                Ok(()) => {
                                    if let Some(mtime) = self.sync_times.remove(&local) {
                                        crate::remote::sync::set_mtime(&local, mtime);
                                    }
                                    (tf!("Скачано: {}", "Downloaded: {}", local.display()), false)
                                }
                                Err(e) => (
                                    tf!("Не удалось записать {}: {e}", "Could not write {}: {e}", local.display()),
                                    true,
                                ),
                            };
                            self.push_log(text.clone(), error);
                            out.push(PanelEvent::Toast(text));
                        }
                        None => out.push(PanelEvent::Open {
                            profile: profile.clone(),
                            line: self.open_at.remove(&path),
                            path,
                            data,
                        }),
                    }
                }
                Event::Conflict { path, doc, server } => {
                    self.finished();
                    self.settle(&path, None);
                    out.push(PanelEvent::Conflict {
                        profile: profile.clone(),
                        doc,
                        path,
                        server,
                    });
                }
                Event::Uploaded { path, doc } => {
                    self.finished();
                    self.settle(&path, None);
                    let dir = remote::parent(&path);
                    if doc.is_none()
                        && let Some(batch) = &mut self.batch
                        && let Some(size) = batch.sizes.remove(&path)
                    {
                        batch.done += 1;
                        batch.bytes += size;
                        if self.pending == 0 || dir != self.cwd {
                            self.refresh_dir(&dir);
                        }
                        continue;
                    }
                    match doc {
                        Some(doc) => out.push(PanelEvent::Uploaded { doc, path }),
                        None => out.push(PanelEvent::Toast(tf!("Загружено: {}", "Uploaded: {}",
                            remote::file_name(&path)
                        ))),
                    }
                    if self.pending == 0 || dir != self.cwd {
                        self.refresh_dir(&dir);
                    }
                }
                Event::Progress { done, total } => self.progress = Some((done, total)),
                Event::NoTimes(path) => {
                    self.finished();
                    self.settle(&path, None);
                    if !self.no_times {
                        self.no_times = true;
                        self.push_log(
                            tr!("Сервер не поддерживает установку времени файлов (MFMT): при синхронизации сравнение по времени выключено").to_owned(),
                            true,
                        );
                    }
                }
                Event::Walked { root, files, times } => {
                    self.finished();
                    let name = remote::file_name(&root).to_owned();
                    let base = remote::parent(&root);
                    let relative = |path: &str| {
                        path.strip_prefix(base.as_str())
                            .unwrap_or(path)
                            .trim_start_matches('/')
                            .to_owned()
                    };
                    match self.walks.remove(&root) {
                        None => {}
                        Some(Walk::Sync(folder)) => {
                            let prefix = format!("{}/", root.trim_end_matches('/'));
                            let remote = files
                                .iter()
                                .zip(&times)
                                .filter_map(|((path, size), time)| {
                                    Some((path.strip_prefix(&prefix)?.to_owned(), (*size, *time)))
                                })
                                .collect();
                            self.sync = Some(crate::remote::sync::Dialog {
                                local: crate::remote::sync::local_files(&folder),
                                remote,
                                remote_root: root.clone(),
                                local_root: folder,
                                mode: crate::remote::sync::Mode::Both,
                                by_time: !self.no_times,
                                remove: false,
                                include: String::new(),
                                exclude: String::new(),
                                no_times: self.no_times,
                            });
                        }
                        Some(Walk::Size) => {
                            let bytes: u64 = files.iter().map(|(_, size)| size).sum();
                            let text = tf!("Папка {name}: файлов {}, {}", "Folder {name}: files {}, {}",
                                files.len(),
                                size_label(bytes as usize)
                            );
                            self.push_log(text.clone(), false);
                            out.push(PanelEvent::Toast(text));
                        }
                        Some(Walk::Download(folder)) => {
                            if let Some(batch) = &mut self.batch {
                                batch.walking = batch.walking.saturating_sub(1);
                                for (path, size) in &files {
                                    batch.add(path.clone(), *size);
                                }
                            }
                            for (path, _) in files {
                                let Some(local) = crate::remote::sync::local_path(&folder, &relative(&path))
                                else {
                                    self.push_log(tf!("Пропущено небезопасное имя: {path}", "Skipped an unsafe name: {path}"), true);
                                    if let Some(batch) = &mut self.batch {
                                        batch.failed.insert(path.clone());
                                    }
                                    continue;
                                };
                                if let Some(dir) = local.parent() {
                                    let _ = std::fs::create_dir_all(dir);
                                }
                                self.save_to.insert(path.clone(), local);
                                self.send(Cmd::Download(path), tf!("Скачивание папки {name}…", "Downloading folder {name}…"));
                            }
                        }
                        Some(Walk::Relay(target, dir)) => {
                            for (path, _) in files {
                                self.relay.insert(
                                    path.clone(),
                                    (target, remote::join(&dir, &relative(&path))),
                                );
                                self.send(Cmd::Download(path), tf!("Передача папки {name}…", "Transferring folder {name}…"));
                            }
                        }
                    }
                }
                Event::Stat { path, attrs } => {
                    self.finished();
                    if let Some(dialog) = &mut self.attrs
                        && dialog.items.first().is_some_and(|(p, _)| *p == path)
                    {
                        dialog.loaded = true;
                        if let Some(mode) = attrs.perm {
                            dialog.mode = mode & 0o7777;
                            dialog.octal = format!("{:03o}", dialog.mode);
                        }
                        dialog.mtime = attrs.mtime.map(date_label).unwrap_or_default();
                        dialog.mtime_was = dialog.mtime.clone();
                        dialog.info = Some(attrs);
                    }
                }
                Event::Changed(dir) => {
                    self.finished();
                    self.settle(&dir, None);
                    if dir != self.cwd && self.batch.as_ref().is_none_or(|b| b.finished.is_some()) {
                        self.push_log(tf!("Изменён каталог {dir}", "Folder changed {dir}"), false);
                    }
                    self.refresh_dir(&dir);
                    if dir != self.cwd && !self.children.contains_key(&dir) {
                        self.list(self.cwd.clone());
                    }
                }
                Event::Error(e) => {
                    self.finished();
                    self.loading.clear();
                    if let Some(batch) = &mut self.batch
                        && batch.walking > 0
                        && let Some(root) = self
                            .walks
                            .iter()
                            .find(|(_, w)| matches!(w, Walk::Download(_)))
                            .map(|(r, _)| r.clone())
                    {
                        self.walks.remove(&root);
                        batch.walking -= 1;
                        self.push_log(tf!("Не удалось прочитать папку {root}: {e}", "Could not read folder {root}: {e}"), true);
                        continue;
                    }
                    out.push(PanelEvent::Toast(tf!("Ошибка сервера: {e}", "Server error: {e}")));
                }
                Event::Failed { path, error } => {
                    self.finished();
                    self.push_log(tf!("Не удалось: {path}: {error}", "Failed: {path}: {error}"), true);
                    if let Some(batch) = &mut self.batch
                        && batch.sizes.contains_key(&path)
                    {
                        batch.failed.insert(path.clone());
                        self.settle(&path, Some(error));
                        continue;
                    }
                    out.push(PanelEvent::Toast(tf!("Не удалось передать {}: {error}", "Could not transfer {}: {error}",
                        remote::file_name(&path)
                    )));
                    self.settle(&path, Some(error));
                }
                Event::Closed(e) => {
                    self.disconnect();
                    out.push(PanelEvent::Toast(tf!("Соединение закрыто: {e}", "Connection closed: {e}")));
                }
            }
        }
        if dead && self.conn.is_some() {
            self.disconnect();
        }
        out
    }

    fn open_editor(&mut self) {
        let mut drafts = self.store.profiles.clone();
        if drafts.is_empty() {
            drafts.push(Profile::new(tr!("Новый профиль").into()));
        }
        self.editor = Some(ProfileEditor {
            index: self.selected.min(drafts.len() - 1),
            drafts,
            reveal: false,
            probe: None,
            result: None,
            note: None,
        });
    }

    fn open_attrs(&mut self, items: Vec<(String, bool)>) {
        let Some((first, _)) = items.first().cloned() else {
            return;
        };
        let known = self
            .entries
            .iter()
            .chain(self.children.values().flatten())
            .find(|e| remote::join(&remote::parent(&first), &e.name) == first)
            .and_then(|e| e.perm)
            .unwrap_or(0o644);
        let has_dirs = items.iter().any(|(_, dir)| *dir);
        self.attrs = Some(AttrsDialog {
            items,
            loaded: false,
            info: None,
            mode: known,
            octal: format!("{known:03o}"),
            recursive: false,
            dirs: has_dirs,
            files: true,
            mtime: String::new(),
            mtime_was: String::new(),
        });
        self.send(Cmd::Stat(first), tr!("Чтение свойств…"));
    }

    fn paste(&mut self, dir: &str) {
        let Some((paths, copy)) = self.clip.clone() else {
            return;
        };
        for from in paths {
            let name = remote::file_name(&from).to_owned();
            if from == dir || dir.starts_with(&format!("{from}/")) {
                self.notes.push(PanelEvent::Toast(tf!("Нельзя вставить «{name}» внутрь самой себя", "Cannot paste \"{name}\" into itself"
                )));
                continue;
            }
            if copy {
                let target = if remote::parent(&from) == dir {
                    let (stem, ext) = match name.rfind('.').filter(|&i| i > 0) {
                        Some(i) => (&name[..i], &name[i..]),
                        None => (name.as_str(), ""),
                    };
                    remote::join(dir, &tf!("{stem} (копия){ext}", "{stem} (copy){ext}"))
                } else {
                    remote::join(dir, &name)
                };
                self.send(Cmd::Copy(from, target), tr!("Копирование…"));
            } else if remote::parent(&from) != dir && !dir.starts_with(&format!("{from}/")) && from != dir {
                self.send(Cmd::Rename(from.clone(), remote::join(dir, &name)), tr!("Перемещение…"));
            }
        }
        if !copy {
            self.clip = None;
        }
    }

    fn clip_items(&mut self, ui: &mut Ui, chosen: &[String], dir: Option<&str>) {
        if !chosen.is_empty() {
            if ui.button(tr!("Вырезать")).clicked() {
                self.clip = Some((chosen.to_vec(), false));
            }
            if ui.button(tr!("Копировать")).clicked() {
                self.clip = Some((chosen.to_vec(), true));
            }
        }
        if let Some(dir) = dir
            && self.clip.is_some()
            && ui.button(tr!("Вставить")).clicked()
        {
            self.paste(dir);
        }
    }

    fn console_here(&mut self, dir: &str) {
        let Some(name) = self.conn.as_ref().map(|c| c.profile.clone()) else {
            return;
        };
        let Some(profile) = self.store.profiles.iter().find(|p| p.name == name) else {
            return;
        };
        if profile.protocol != remote::Protocol::Sftp {
            self.notes.push(PanelEvent::Toast(
                tr!("Консоль доступна для SFTP-профилей (по SSH)").into(),
            ));
            return;
        }
        let mut term = remote::TermProfile::from_host(profile);
        term.startup = format!("cd {}", remote::quote(dir));
        self.notes.push(PanelEvent::Console(Box::new(term)));
    }

    fn menu_items(&mut self, ui: &mut Ui, dir: &str) {
        if ui.button(tr!("Новый файл…")).clicked() {
            self.prompt = Some(Prompt::NewFile(dir.to_owned(), String::new()));
            ui.close();
        }
        if ui.button(tr!("Новая папка…")).clicked() {
            self.prompt = Some(Prompt::NewFolder(dir.to_owned(), String::new()));
            ui.close();
        }
        if ui.button(tr!("Загрузить файлы с компьютера…")).clicked() {
            ui.close();
            if let Some(files) = rfd::FileDialog::new().pick_files() {
                self.upload_local_to(files, dir);
            }
        }
        if ui.button(tr!("Загрузить папку с компьютера…")).clicked() {
            ui.close();
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                self.upload_local_to(vec![folder], dir);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        self.area = ui.max_rect();
        ui.take_available_width();
        ui.add_space(8.0);
        if self.store.profiles.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("Подключений пока нет.")).color(pal.subtext));
            if ui
                .add(Button::new(tr!("Создать профиль")))
                .clicked()
            {
                self.open_editor();
            }
            return;
        }
        self.selected = self.selected.min(self.store.profiles.len() - 1);
        let active = self.conn.is_some();
        let label = if active { tr!("Отключить") } else { tr!("Подключить") };
        let names: Vec<(String, String)> = self
            .store
            .profiles
            .iter()
            .map(|p| (p.name.clone(), p.protocol.label().to_owned()))
            .collect();
        let list_id = egui::Id::unique(("tror_profile", self.id));
        let mut action = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = (ui.available_width() - 32.0).max(40.0);
            ui.add_enabled_ui(!active, |ui| {
                let combo = ComboBox::from_id_salt(list_id)
                    .width(width)
                    .truncate()
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .selected_text(names[self.selected].0.clone())
                    .show_ui(ui, |ui| {
                        action = crate::widgets::profile_list(
                            ui,
                            list_id,
                            &names,
                            Some(self.selected),
                            tr!("Добавить профиль…"),
                        );
                    });
                let current = self.selected;
                let name = names[current].0.clone();
                if let Some(a) =
                    crate::widgets::profile_context(ui, list_id, &combo.response, current, &name)
                {
                    action = Some(a);
                }
            });
            if crate::widgets::plus_button(ui, pal, egui::vec2(26.0, 24.0))
                .on_hover_text(tr!("Добавить профиль"))
                .clicked()
            {
                action = Some(crate::widgets::ProfileAction::Add);
            }
        });
        if let Some(a) = crate::widgets::profile_dialogs(&ctx, pal, list_id, &names) {
            action = Some(a);
        }
        if ui.add(Button::new(label)).clicked() {
            if active {
                self.disconnect();
            } else {
                self.connect(&ctx);
            }
        }
        match action {
            Some(crate::widgets::ProfileAction::Select(i)) => self.selected = i,
            Some(crate::widgets::ProfileAction::Edit(i)) => {
                self.selected = i;
                self.open_editor();
            }
            Some(crate::widgets::ProfileAction::Rename(i, name)) => {
                if let Some(profile) = self.store.profiles.get_mut(i) {
                    profile.name = name;
                    let _ = self.store.save();
                }
            }
            Some(crate::widgets::ProfileAction::Delete(i)) => {
                if i < self.store.profiles.len() && !active {
                    self.store.profiles.remove(i);
                    let _ = self.store.save();
                    self.selected = self.selected.min(self.store.profiles.len().saturating_sub(1));
                    if self.store.profiles.is_empty() {
                        return;
                    }
                }
            }
            Some(crate::widgets::ProfileAction::Add) => {
                self.open_editor();
                if let Some(editor) = &mut self.editor {
                    editor.drafts.push(Profile::new(tf!("Сервер {}", "Server {}", editor.drafts.len() + 1)));
                    editor.index = editor.drafts.len() - 1;
                }
            }
            None => {}
        }

        if !self.connected && self.busy.is_none() {
            ui.add_space(10.0);
            ui.label(
                RichText::new(tr!("Выберите профиль и нажмите «Подключить». Здесь появятся файлы сервера — их можно открывать, перетаскивать в проект и обратно."))
                    .small()
                    .color(pal.subtext),
            );
        }
        if self.connected {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let field = ui.add(
                    TextEdit::singleline(&mut self.path_input)
                        .desired_width((ui.available_width() - 60.0).max(30.0))
                        .font(egui::TextStyle::Small),
                );
                if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.list(self.path_input.trim().to_owned());
                }
                if refresh_button(ui, pal)
                    .on_hover_text(tr!("Обновить каталог"))
                    .clicked()
                {
                    self.list(self.cwd.clone());
                    let open: Vec<String> = self.expanded.iter().cloned().collect();
                    for dir in open {
                        self.load(dir);
                    }
                }
                let dots = dots_button(ui, pal).on_hover_text(tr!("Ещё"));
                egui::Popup::menu(&dots)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                    ui.set_min_width(230.0);
                    if ui.button(tr!("Поиск на сервере…")).clicked() {
                        self.want_search = true;
                        ui.close();
                    }
                    ui.separator();
                    let cwd = self.cwd.clone();
                    self.clip_items(ui, &[], Some(&cwd));
                    self.menu_items(ui, &cwd);
                    if ui.button(tr!("Открыть консоль здесь")).clicked() {
                        self.console_here(&cwd);
                    }
                    ui.separator();
                    let mut tree = self.tree;
                    ui.horizontal(|ui| {
                        ui.label(tr!("Вид:"));
                        if ui.selectable_label(!tree, tr!("список")).clicked() {
                            tree = false;
                        }
                        if ui.selectable_label(tree, tr!("дерево")).clicked() {
                            tree = true;
                        }
                    });
                    if tree != self.tree {
                        crate::app::request_view(Some(tree), None);
                    }
                    let mut size = !self.store.hide_size;
                    if ui.checkbox(&mut size, tr!("Показывать размер")).changed() {
                        self.store.hide_size = !size;
                        let _ = self.store.save();
                    }
                    if ui
                        .checkbox(&mut self.store.show_date, tr!("Показывать дату изменения"))
                        .changed()
                    {
                        let _ = self.store.save();
                    }
                    if ui
                        .checkbox(&mut self.store.show_perm, tr!("Показывать права"))
                        .changed()
                    {
                        let _ = self.store.save();
                    }
                });
            });
            self.listing(ui, pal);
        }
        self.transfers_ui(ui, pal);
        if let Some(busy) = &self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                let waiting = self.transfers.len() + self.pending.saturating_sub(1);
                let queue = if waiting > 0 {
                    tf!("  ·  в очереди {waiting}", "  ·  queued {waiting}")
                } else {
                    String::new()
                };
                let percent = match self.progress {
                    Some((done, total)) if total > 0 => {
                        format!("  {}%", (done * 100 / total).min(100))
                    }
                    Some((done, _)) => format!("  {}", size_label(done as usize)),
                    None => String::new(),
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{busy}{percent}{queue}"))
                            .color(pal.subtext)
                            .small(),
                    )
                    .truncate(),
                );
            });
            if let Some((done, total)) = self.progress
                && total > 0
            {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), Sense::hover());
                ui.painter().rect_filled(rect, 1.5, pal.surface);
                let mut filled = rect;
                filled.set_width(rect.width() * (done as f32 / total as f32).min(1.0));
                ui.painter().rect_filled(filled, 1.5, pal.accent);
            }
        }
    }

}

fn merge_profiles(drafts: &mut Vec<Profile>, found: Vec<Profile>) -> usize {
    if drafts.len() == 1 && drafts[0].host.trim().is_empty() {
        drafts.clear();
    }
    let mut added = 0;
    for profile in found {
        let same = drafts.iter().any(|d| {
            d.host.eq_ignore_ascii_case(&profile.host)
                && d.port == profile.port
                && d.user == profile.user
                && d.protocol == profile.protocol
        });
        if !same {
            drafts.push(profile);
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(panel: &mut RemotePanel, done: impl Fn(&RemotePanel) -> bool) {
        let start = std::time::Instant::now();
        while !done(panel) {
            assert!(start.elapsed().as_secs() < 20, "timeout");
            panel.poll();
            if panel.host_key.take().is_some()
                && let Some(conn) = &panel.conn
            {
                let _ = conn.tx.send(Cmd::TrustHost(true));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    #[ignore]
    fn live_transfer_queue_pauses_and_retries() {
        let ctx = egui::Context::default();
        let mut panel = RemotePanel::new(9);
        panel.store = Store::default();
        panel.store.profiles.push(Profile {
            name: "t".into(),
            protocol: remote::Protocol::Sftp,
            host: "127.0.0.1".into(),
            port: 2222,
            user: "sftpu".into(),
            password: "pw".into(),
            ..Profile::new("t".into())
        });
        panel.selected = 0;
        panel.connect(&ctx);
        wait(&mut panel, |p| p.connected && p.pending == 0);
        let out = std::env::temp_dir().join(format!("tror-queue-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        panel.paused = true;
        for name in ["a.txt", "b.txt", "locked.txt"] {
            let path = format!("/home/sftpu/www/q/{name}");
            panel.save_to.insert(path.clone(), out.join(name));
            panel.send(Cmd::Download(path), name.to_owned());
        }
        assert_eq!(panel.transfers.len(), 3);
        assert!(panel.running.is_none());
        std::thread::sleep(std::time::Duration::from_millis(300));
        panel.poll();
        assert!(!out.join("a.txt").exists());
        panel.paused = false;
        panel.pump();
        wait(&mut panel, |p| p.transfers.is_empty() && p.running.is_none());
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "aaa\n");
        assert_eq!(std::fs::read_to_string(out.join("b.txt")).unwrap(), "bbb\n");
        assert_eq!(panel.failed.len(), 1);
        assert!(panel.failed[0].path().ends_with("locked.txt"));
        assert!(!out.join("locked.txt").exists());
        std::process::Command::new("chmod")
            .args(["644", "/home/sftpu/www/q/locked.txt"])
            .status()
            .unwrap();
        panel.retry_failed();
        wait(&mut panel, |p| p.transfers.is_empty() && p.running.is_none());
        assert!(panel.failed.is_empty());
        assert_eq!(std::fs::read_to_string(out.join("locked.txt")).unwrap(), "lll\n");
        std::fs::remove_dir_all(out).ok();
    }
}
