use std::collections::{BTreeSet, HashMap, HashSet};
use crate::widgets::Lit;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Button, ComboBox, Key, Rect, RichText, ScrollArea, Sense, TextEdit, Ui};

use crate::app::{dialog, dialog_button, size_label};
use crate::remote::{self, Attrs, Cmd, Connection, Entry, Event, Profile, Protocol, Store};
use crate::theme::Palette;
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
    sync: Option<crate::sync::Dialog>,
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
                    let in_batch = self.batch.as_ref().is_some_and(|b| b.sizes.contains_key(&path));
                    match self.save_to.remove(&path) {
                        Some(local) if in_batch => {
                            let result = std::fs::write(&local, data);
                            if result.is_ok()
                                && let Some(mtime) = self.sync_times.remove(&local)
                            {
                                crate::sync::set_mtime(&local, mtime);
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
                            let (text, error) = match std::fs::write(&local, data) {
                                Ok(()) => {
                                    if let Some(mtime) = self.sync_times.remove(&local) {
                                        crate::sync::set_mtime(&local, mtime);
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
                            self.sync = Some(crate::sync::Dialog {
                                local: crate::sync::local_files(&folder),
                                remote,
                                remote_root: root.clone(),
                                local_root: folder,
                                mode: crate::sync::Mode::Both,
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
                                let Some(local) = crate::sync::local_path(&folder, &relative(&path))
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

    fn transfers_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        if self.transfers.is_empty() && self.failed.is_empty() {
            return;
        }
        let waiting = self
            .transfers
            .iter()
            .filter(|t| !matches!(t.cmd, Cmd::Touch { .. }))
            .count();
        ui.add_space(4.0);
        if waiting > 0 {
            ui.horizontal(|ui| {
                let text = if self.paused {
                    tf!("Пауза · ждут {waiting}", "Paused · waiting {waiting}")
                } else {
                    tf!("Ждут передачи: {waiting}", "Waiting to transfer: {waiting}")
                };
                ui.label(RichText::new(text).small().color(pal.subtext));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(crate::widgets::secondary(tr!("Отменить"), pal).small()).clicked() {
                        let dropped: Vec<Transfer> = self.transfers.drain(..).collect();
                        self.forget(dropped);
                        self.paused = false;
                    }
                    let label = if self.paused { tr!("Продолжить") } else { tr!("Пауза") };
                    if ui.add(crate::widgets::secondary(label, pal).small()).clicked() {
                        self.paused = !self.paused;
                        self.pump();
                    }
                });
            });
        }
        if !self.failed.is_empty() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tf!("Не передано: {}", "Not transferred: {}", self.failed.len()))
                        .small()
                        .color(pal.warn),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(crate::widgets::secondary(tr!("Убрать"), pal).small()).clicked() {
                        let dropped = std::mem::take(&mut self.failed);
                        self.forget(dropped);
                    }
                    if ui.add(crate::widgets::primary(tr!("Повторить"), pal).small()).lit(ui).clicked() {
                        self.retry_failed();
                    }
                });
            });
            for transfer in self.failed.iter().take(4) {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} — {}",
                            remote::file_name(transfer.path()),
                            transfer.error
                        ))
                        .small()
                        .color(pal.faint),
                    )
                    .truncate(),
                );
            }
            if self.failed.len() > 4 {
                ui.label(
                    RichText::new(tf!("и ещё {}", "and {} more", self.failed.len() - 4))
                        .small()
                        .color(pal.faint),
                );
            }
        }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let query = self.search_query.trim().to_owned();
        if query.is_empty() || !self.connected {
            return;
        }
        let Some((profile, known)) = self.selected_profile() else {
            return;
        };
        let root = if self.search_root.trim().is_empty() {
            self.cwd.clone()
        } else {
            self.search_root.trim().to_owned()
        };
        let params = remote::SearchParams {
            root: root.clone(),
            query: query.clone(),
            content: self.search_content,
            case: self.search_case,
            word: self.search_word,
            regex: self.search_regex,
            excludes: self
                .search_excludes
                .split(',')
                .map(|e| e.trim().to_owned())
                .collect(),
        };
        let ctx = ctx.clone();
        let job = remote::search(profile, known, params, move || ctx.request_repaint());
        self.hits.clear();
        self.search_status = tr!("поиск…").into();
        self.push_log(tf!("Поиск «{query}» в {root}", "Searching \"{query}\" in {root}"), false);
        self.search_job = Some(job);
        if self.store.search_excludes.as_deref() != Some(self.search_excludes.as_str()) {
            self.store.search_excludes = Some(self.search_excludes.clone());
            let _ = self.store.save();
        }
    }

    pub fn search_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        ui.add_space(4.0);
        if !self.connected {
            ui.add(
                egui::Label::new(
                    RichText::new(tr!("Подключитесь к серверу в панели «Удалённые хосты»."))
                        .color(pal.faint)
                        .small(),
                )
                .wrap(),
            );
            return;
        }
        let mut start = false;
        use crate::widgets::{FORM_ROW, form_field, form_row, toggle};
        ui.spacing_mut().item_spacing.y = 8.0;
        form_row(ui, pal, tr!("Найти"), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = crate::widgets::field_width(ui, 3.0 * 32.0 + 34.0);
            let field = ui.add(
                form_field(&mut self.search_query, tr!("текст или выражение")).desired_width(width),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                start = true;
            }
            if crate::app::icon_button(ui, pal, crate::app::Icon::Close)
                .on_hover_text(tr!("Очистить поиск"))
                .clicked()
            {
                if let Some(job) = &self.search_job {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.search_query.clear();
                self.hits.clear();
                self.search_status.clear();
            }
            ui.add_space(6.0);
            toggle(ui, pal, &mut self.search_case, "Aa", tr!("Учитывать регистр"));
            toggle(ui, pal, &mut self.search_word, "ab", tr!("Слово целиком"));
            toggle(
                ui,
                pal,
                &mut self.search_regex,
                ".*",
                tr!("Регулярное выражение"),
            );
        });
        form_row(ui, pal, tr!("Искать"), |ui| {
            crate::widgets::segmented(
                ui,
                pal,
                &mut self.search_content,
                &[(true, tr!("В тексте файлов")), (false, tr!("В именах"))],
                120.0,
            );
        });
        form_row(ui, pal, tr!("Папка"), |ui| {
            let hint = self.cwd.clone();
            ui.add(
                form_field(&mut self.search_root, hint)
                    .desired_width(crate::widgets::field_width(ui, 94.0)),
            );
            if ui
                .add(
                    Button::new(tr!("Текущая"))
                        .min_size(egui::vec2(88.0, FORM_ROW)),
                )
                .on_hover_text(tr!("Папка, открытая в панели хоста"))
                .clicked()
            {
                self.search_root = self.cwd.clone();
            }
        });
        form_row(ui, pal, tr!("Не искать в"), |ui| {
            ui.add(
                form_field(
                    &mut self.search_excludes,
                    tr!("папки через запятую: bitrix/cache, node_modules"),
                )
                .desired_width(crate::widgets::field_width(ui, 0.0)),
            );
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let size = egui::vec2(0.0, FORM_ROW);
            if self.search_job.is_some() {
                if ui
                    .add(Button::new(tr!("Остановить")).min_size(size))
                    .clicked()
                    && let Some(job) = &self.search_job
                {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ui.spinner();
            } else if ui
                .add_enabled(
                    !self.search_query.trim().is_empty(),
                    crate::widgets::primary(tr!("Найти"), pal).min_size(size),
                ).lit(ui)
                .clicked()
            {
                start = true;
            }
            ui.label(
                RichText::new(&self.search_status)
                    .color(pal.subtext)
                    .small(),
            );
        });
        if !self.hits.is_empty() {
            ui.add_space(6.0);
            ui.separator();
        }
        if start && self.search_job.is_none() {
            self.start_search(&ctx);
        }
        let mut open = None;
        let row_h = crate::widgets::RESULT_ROW;
        ui.spacing_mut().item_spacing.y = 0.0;
        let base = if self.search_root.trim().is_empty() {
            self.cwd.clone()
        } else {
            self.search_root.trim().to_owned()
        };
        let tall = (self.hits.len() as f32 * row_h).min(crate::widgets::RESULTS);
        ScrollArea::vertical()
            .id_salt(("tror_server_hits", self.id))
            .min_scrolled_height(tall)
            .max_height(tall)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, self.hits.len(), |ui, rows| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    let (path, line, text) = &self.hits[row];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    if response.hovered() {
                        painter.rect_filled(rect, 4.0, pal.crust);
                    }
                    let shown = path
                        .strip_prefix(base.trim_end_matches('/'))
                        .map_or(path.as_str(), |p| p.trim_start_matches('/'));
                    let label = match line {
                        Some(n) => format!("{shown}:{n}"),
                        None => shown.to_owned(),
                    };
                    let placed = painter.text(
                        egui::pos2(rect.left() + 6.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(12.5),
                        pal.accent,
                    );
                    painter.text(
                        egui::pos2(placed.right() + 12.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::monospace(12.0),
                        pal.subtext,
                    );
                    if response.clicked() {
                        open = Some((path.clone(), *line));
                    }
                }
            },
        );
        if let Some((path, line)) = open {
            if let Some(line) = line {
                self.open_at.insert(path.clone(), line);
            }
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.cwd != "/" {
            rows.push(Row {
                path: remote::parent(&self.cwd),
                name: "..".into(),
                depth: 0,
                entry: None,
            });
        }
        let sorted = |entries: &[Entry]| {
            let mut list = entries.to_vec();
            self.sort.apply(&mut list, |e| {
                (e.is_dir, e.name.as_str(), e.size, e.perm.unwrap_or(0), e.mtime.unwrap_or(0))
            });
            list
        };
        if !self.tree {
            for entry in sorted(&self.entries) {
                rows.push(Row {
                    path: remote::join(&self.cwd, &entry.name),
                    name: entry.name.clone(),
                    depth: 0,
                    entry: Some(entry.clone()),
                });
            }
            return rows;
        }
        fn walk(panel: &RemotePanel, dir: &str, depth: usize, rows: &mut Vec<Row>) {
            let Some(entries) = panel.children.get(dir) else {
                return;
            };
            let mut entries = entries.clone();
            panel.sort.apply(&mut entries, |e| {
                (e.is_dir, e.name.as_str(), e.size, e.perm.unwrap_or(0), e.mtime.unwrap_or(0))
            });
            for entry in &entries {
                let path = remote::join(dir, &entry.name);
                let open = entry.is_dir && panel.expanded.contains(&path);
                rows.push(Row {
                    path: path.clone(),
                    name: entry.name.clone(),
                    depth,
                    entry: Some(entry.clone()),
                });
                if open && depth < 40 {
                    walk(panel, &path, depth + 1, rows);
                }
            }
        }
        if self.children.contains_key(&self.cwd) {
            walk(self, &self.cwd, 0, &mut rows);
        } else {
            for entry in sorted(&self.entries) {
                rows.push(Row {
                    path: remote::join(&self.cwd, &entry.name),
                    name: entry.name.clone(),
                    depth: 0,
                    entry: Some(entry.clone()),
                });
            }
        }
        rows
    }

    fn selection_paths(&self, rows: &[Row], clicked: &str) -> Vec<(String, bool)> {
        if self.marked.contains(clicked) {
            rows.iter()
                .filter(|r| r.entry.is_some() && self.marked.contains(&r.path))
                .map(|r| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir)))
                .collect()
        } else {
            rows.iter()
                .filter(|r| r.entry.is_some() && r.path == clicked)
                .map(|r| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir)))
                .collect()
        }
    }

    fn listing(&mut self, ui: &mut Ui, pal: &Palette) {
        let row_h = 22.0;
        let rows = self.rows();
        let total = rows.len();
        let show_size = !self.store.hide_size;
        let show_date = self.store.show_date;
        let show_perm = self.store.show_perm;
        let mut go = None;
        let mut download: Vec<String> = Vec::new();
        let mut fetch: Option<(Vec<(String, bool)>, PathBuf)> = None;
        let mut fetch_as: Option<(String, PathBuf)> = None;
        let mut moves: Vec<(String, String)> = Vec::new();
        let mut walk: Vec<String> = Vec::new();
        let mut toggle: Option<String> = None;
        let mut attrs: Option<Vec<(String, bool)>> = None;
        let waiting = self
            .transfers
            .iter()
            .any(|t| !matches!(t.cmd, Cmd::Touch { .. }));
        let errors = if self.failed.is_empty() {
            0.0
        } else {
            30.0 + 18.0 * (self.failed.len().min(4) + usize::from(self.failed.len() > 4)) as f32
        };
        let reserve = if self.busy.is_some() { 34.0 } else { 0.0 }
            + if waiting { 34.0 } else { 0.0 }
            + errors;
        let tree = self.tree;
        let widths = crate::widgets::column_header(ui, pal, [show_size, show_perm, show_date], &mut self.sort);
        ui.spacing_mut().item_spacing.y = 0.0;
        let outer = ui.available_rect_before_wrap();
        let height = (ui.available_height() - reserve).max(40.0);
        let blank = ui.interact(
            Rect::from_min_size(outer.min, egui::vec2(outer.width(), height)),
            ui.scope_id().with(("tror_remote_blank", self.id)),
            Sense::click(),
        );
        let root = ui.ctx().viewport_id() == egui::ViewportId::ROOT;
        self.drop_zone = root.then_some(blank.rect);
        self.drop_dirs.clear();
        let mut dirs = Vec::new();
        let wide = crate::widgets::rows_width(
            ui,
            rows.iter().map(|r| {
                (r.depth as f32 * 14.0 + if tree { 14.0 } else { 0.0 }, r.name.as_str())
            }),
            widths,
        );
        let area = ScrollArea::both()
            .id_salt(("tror_remote_rows", self.id))
            .auto_shrink(false)
            .max_height((ui.available_height() - reserve).max(40.0))
            .show_rows(ui, row_h, total, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let view = ui.clip_rect();
                for index in range {
                    let row = &rows[index];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(wide, row_h),
                        Sense::click_and_drag(),
                    );
                    let entry = row.entry.clone();
                    let name = row.name.clone();
                    let path = row.path.clone();
                    let is_dir = entry.as_ref().is_none_or(|e| e.is_dir);
                    let marked = entry.is_some() && self.marked.contains(&path);
                    let drop_target = is_dir
                        && response
                            .dnd_hover_payload::<Dragged>()
                            .is_some_and(|dragged| {
                                dragged.panel == self.id
                                    && !dragged.paths.iter().any(|(p, _)| *p == path)
                            });
                    if is_dir && root {
                        dirs.push((rect.intersect(view), path.clone()));
                    }
                    let external = is_dir && self.external_hover.as_deref() == Some(path.as_str());
                    let painter = ui.painter_at(rect);
                    if drop_target || external {
                        painter.rect_filled(rect, 5.0, pal.selection);
                    } else if marked {
                        painter.rect_filled(rect, 5.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 5.0, pal.crust);
                    }
                    let indent = row.depth as f32 * 14.0 + if tree { 14.0 } else { 0.0 };
                    if tree && is_dir && entry.is_some() {
                        let open = self.expanded.contains(&path);
                        chevron(
                            &painter,
                            egui::pos2(rect.left() + indent - 6.0, rect.center().y),
                            open,
                            pal.faint,
                        );
                        if self.loading.contains(&path) {
                            painter.circle_filled(
                                egui::pos2(rect.left() + indent - 6.0, rect.center().y + 7.0),
                                1.5,
                                pal.accent,
                            );
                        }
                    }
                    paint_icon(
                        &painter,
                        egui::pos2(rect.left() + indent + 14.0, rect.center().y),
                        is_dir,
                        &name,
                        pal,
                    );
                    let mut right = view.right().min(rect.right()) - 6.0;
                    let info = entry.as_ref();
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[2],
                        info.and_then(|e| e.mtime).map(date_label),
                        pal.faint,
                    );
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[1],
                        info.and_then(|e| e.perm).map(|mode| format!("{mode:03o}")),
                        pal.faint,
                    );
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[0],
                        info.filter(|e| !e.is_dir)
                            .map(|e| size_label(e.size as usize)),
                        pal.faint,
                    );
                    let name_clip = Rect::from_min_max(
                        egui::pos2(rect.left() + indent + 28.0, rect.top()),
                        egui::pos2(right.max(rect.left() + indent + 28.0), rect.bottom()),
                    );
                    ui.painter_at(name_clip).text(
                        egui::pos2(name_clip.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        egui::FontId::proportional(13.0),
                        if name.starts_with('.') && name != ".." {
                            pal.subtext
                        } else {
                            pal.text
                        },
                    );

                    if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        let on_chevron = tree
                            && is_dir
                            && entry.is_some()
                            && response
                                .interact_pointer_pos()
                                .is_some_and(|p| p.x < rect.left() + indent + 4.0);
                        if on_chevron {
                            toggle = Some(path.clone());
                        } else if entry.is_none() {
                            self.marked.clear();
                        } else if modifiers.shift && self.anchor.is_some() {
                            let anchor = self.anchor.clone().unwrap_or_default();
                            let a = rows.iter().position(|r| r.path == anchor).unwrap_or(index);
                            let (from, to) = (a.min(index), a.max(index));
                            if !modifiers.command {
                                self.marked.clear();
                            }
                            for r in &rows[from..=to] {
                                if r.entry.is_some() {
                                    self.marked.insert(r.path.clone());
                                }
                            }
                        } else if modifiers.command {
                            if !self.marked.remove(&path) {
                                self.marked.insert(path.clone());
                            }
                            self.anchor = Some(path.clone());
                        } else {
                            self.marked.clear();
                            self.marked.insert(path.clone());
                            self.anchor = Some(path.clone());
                            if tree && is_dir {
                                toggle = Some(path.clone());
                            }
                        }
                    }
                    if response.double_clicked() {
                        if is_dir && tree && entry.is_some() {
                        } else if is_dir {
                            go = Some(path.clone());
                        } else {
                            download.push(path.clone());
                        }
                    }
                    if entry.is_some() && response.drag_started() {
                        response.dnd_set_drag_payload(Dragged {
                            panel: self.id,
                            paths: self.selection_paths(&rows, &path),
                        });
                    }
                    if is_dir
                        && response
                            .dnd_hover_payload::<Dragged>()
                            .is_some_and(|d| d.panel == self.id)
                        && let Some(dragged) = response.dnd_release_payload::<Dragged>()
                    {
                        for (from, _) in dragged.paths.iter() {
                            let to = remote::join(&path, remote::file_name(from));
                            if *from != path && remote::parent(from) != path {
                                moves.push((from.clone(), to));
                            }
                        }
                    }
                    if entry.is_some() {
                        response.context_menu(|ui| {
                            ui.set_min_width(220.0);
                            let chosen = self.selection_paths(&rows, &path);
                            let files: Vec<String> = chosen
                                .iter()
                                .filter(|(_, dir)| !dir)
                                .map(|(p, _)| p.clone())
                                .collect();
                            let dirs: Vec<String> = chosen
                                .iter()
                                .filter(|(_, dir)| *dir)
                                .map(|(p, _)| p.clone())
                                .collect();
                            if !files.is_empty() && ui.button(tr!("Открыть")).clicked() {
                                download.extend(files.iter().cloned());
                            }
                            let archives: Vec<String> = files
                                .iter()
                                .filter(|f| remote::is_archive(f))
                                .cloned()
                                .collect();
                            if !archives.is_empty() && self.sftp() {
                                let mut extract: Option<Vec<(String, Option<String>)>> = None;
                                if ui.button(tr!("Распаковать здесь")).clicked() {
                                    extract = Some(archives.iter().map(|a| (a.clone(), None)).collect());
                                }
                                let label = match archives.as_slice() {
                                    [one] => tf!("Распаковать в «{}/»", "Extract to \"{}/\"",
                                        remote::archive_stem(one).unwrap_or_default()
                                    ),
                                    _ => tr!("Распаковать каждый в свою папку").to_owned(),
                                };
                                if ui.button(label).clicked() {
                                    extract = Some(
                                        archives
                                            .iter()
                                            .map(|a| {
                                                let stem = remote::archive_stem(a).unwrap_or("archive");
                                                (a.clone(), Some(remote::join(&remote::parent(a), stem)))
                                            })
                                            .collect(),
                                    );
                                }
                                if ui.button(tr!("Распаковать в…")).clicked() {
                                    let first = &archives[0];
                                    let suggest = remote::join(
                                        &remote::parent(first),
                                        remote::archive_stem(first).unwrap_or_default(),
                                    );
                                    self.prompt = Some(Prompt::Extract(archives.clone(), suggest));
                                }
                                for (archive, dest) in extract.into_iter().flatten() {
                                    let busy = tf!("Распаковка {}…", "Extracting {}…", remote::file_name(&archive));
                                    self.send(Cmd::Extract { path: archive, dest }, busy);
                                }
                            }
                            let picked: Vec<String> = chosen.iter().map(|(p, _)| p.clone()).collect();
                            self.clip_items(
                                ui,
                                &picked,
                                (chosen.len() == 1 && is_dir).then_some(path.as_str()),
                            );
                            let parent = remote::parent(&picked[0]);
                            if picked.iter().all(|p| remote::parent(p) == parent) {
                                ui.menu_button(tr!("Архивировать"), |ui| {
                                    for (label, zip) in [("TAR.GZ", false), ("ZIP", true)] {
                                        if ui.button(label).clicked() {
                                            let names = picked
                                                .iter()
                                                .map(|p| remote::file_name(p).to_owned())
                                                .collect();
                                            self.send(
                                                Cmd::Archive {
                                                    dir: parent.clone(),
                                                    names,
                                                    zip,
                                                },
                                                tr!("Архивирование…"),
                                            );
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            ui.separator();
                            if chosen.len() == 1 && is_dir {
                                if ui.button(tr!("Открыть консоль здесь")).clicked() {
                                    self.console_here(&path);
                                }
                                if ui.button(tr!("Новый файл здесь…")).clicked() {
                                    self.prompt = Some(Prompt::NewFile(path.clone(), String::new()));
                                }
                                if ui.button(tr!("Новая папка здесь…")).clicked() {
                                    self.prompt =
                                        Some(Prompt::NewFolder(path.clone(), String::new()));
                                }
                                if ui.button(tr!("Загрузить сюда файлы…")).clicked()
                                    && let Some(files) = rfd::FileDialog::new().pick_files()
                                {
                                    self.upload_local_to(files, &path);
                                }
                                ui.separator();
                            }
                            if files.len() == 1
                                && ui.button(tr!("Скачать как…")).clicked()
                                && let Some(local) = rfd::FileDialog::new()
                                    .set_file_name(remote::file_name(&files[0]))
                                    .save_file()
                            {
                                fetch_as = Some((files[0].clone(), local));
                            }
                            if (files.len() > 1 || !dirs.is_empty())
                                && ui.button(tr!("Скачать в папку…")).clicked()
                                && let Some(folder) = std::env::var_os("TROR_PICK_FOLDER")
                                    .map(PathBuf::from)
                                    .or_else(|| rfd::FileDialog::new().pick_folder())
                            {
                                let items = files
                                    .iter()
                                    .map(|f| (f.clone(), false))
                                    .chain(dirs.iter().map(|d| (d.clone(), true)))
                                    .collect();
                                fetch = Some((items, folder));
                            }
                            if dirs.len() == 1
                                && files.is_empty()
                                && ui.button(tr!("Синхронизировать с папкой на компьютере…")).clicked()
                                && let Some(folder) = std::env::var_os("TROR_PICK_FOLDER")
                                    .map(PathBuf::from)
                                    .or_else(|| rfd::FileDialog::new().pick_folder())
                            {
                                self.walks.insert(dirs[0].clone(), Walk::Sync(folder));
                                walk.push(dirs[0].clone());
                            }
                            if !dirs.is_empty() && ui.button(tr!("Размер папки")).clicked() {
                                for dir in &dirs {
                                    self.walks.insert(dir.clone(), Walk::Size);
                                    walk.push(dir.clone());
                                }
                            }
                            for (peer, label) in self.peers.clone() {
                                if ui.button(tf!("Передать на «{label}»", "Transfer to \"{label}\"")).clicked() {
                                    self.outgoing = Some((peer, chosen.clone()));
                                }
                            }
                            ui.separator();
                            if ui.button(tr!("Права и атрибуты…")).clicked() {
                                attrs = Some(chosen.clone());
                            }
                            if ui.button(tr!("Копировать путь")).clicked() {
                                let text: Vec<&str> =
                                    chosen.iter().map(|(p, _)| p.as_str()).collect();
                                ui.ctx().copy_text(text.join("\n"));
                            }
                            if chosen.len() == 1 && ui.button(tr!("Переименовать…")).clicked()
                            {
                                self.prompt = Some(Prompt::Rename {
                                    from: path.clone(),
                                    name: name.clone(),
                                });
                            }
                            let label = if chosen.len() > 1 {
                                tf!("Удалить выбранное ({})…", "Delete selected ({})…", chosen.len())
                            } else {
                                tr!("Удалить…").to_owned()
                            };
                            if ui.button(label).clicked() {
                                self.prompt = Some(Prompt::Delete(chosen));
                            }
                        });
                    }
                }
            });
        let inner = area.inner_rect;
        self.drop_dirs = dirs;
        if blank.clicked() {
            self.marked.clear();
        }
        let keys_area = Rect::from_min_size(outer.min, egui::vec2(outer.width(), height));
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(keys_area) {
            ui.memory_mut(|m| m.request_focus(blank.id));
        }
        if ui.memory(|m| m.has_focus(blank.id)) && self.prompt.is_none() {
            let chosen: Vec<(String, bool)> = rows
                .iter()
                .filter(|r| r.entry.is_some() && self.marked.contains(&r.path))
                .map(|r| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir)))
                .collect();
            let (delete, rename) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F2),
                )
            });
            let (cut, copy, paste) = ui.input(|i| {
                let mut found = (false, false, false);
                for event in &i.events {
                    match event {
                        egui::Event::Cut => found.0 = true,
                        egui::Event::Copy => found.1 = true,
                        egui::Event::Paste(_) => found.2 = true,
                        _ => {}
                    }
                }
                found
            });
            if (cut || copy) && !chosen.is_empty() {
                self.clip = Some((chosen.iter().map(|(p, _)| p.clone()).collect(), copy));
            }
            if paste && self.clip.is_some() {
                let dir = match chosen.as_slice() {
                    [(one, true)] => one.clone(),
                    _ => self.cwd.clone(),
                };
                self.paste(&dir);
            }
            if delete && !chosen.is_empty() {
                self.prompt = Some(Prompt::Delete(chosen.clone()));
            } else if rename && chosen.len() == 1 {
                let from = chosen[0].0.clone();
                self.prompt = Some(Prompt::Rename {
                    name: remote::file_name(&from).to_owned(),
                    from,
                });
            }
        }
        let cwd = self.cwd.clone();
        egui::Popup::context_menu(&blank).show(|ui| {
            ui.set_min_width(220.0);
            self.clip_items(ui, &[], Some(&cwd));
            self.menu_items(ui, &cwd);
            if ui.button(tr!("Открыть консоль здесь")).clicked() {
                self.console_here(&cwd);
            }
            ui.separator();
            if ui.button(tr!("Обновить")).clicked() {
                self.list(cwd.clone());
                ui.close();
            }
            if ui.button(tr!("Копировать путь папки")).clicked() {
                ui.ctx().copy_text(cwd.clone());
                ui.close();
            }
        });
        if let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx())
            && dragged.panel != self.id
            && ui.rect_contains_pointer(inner)
        {
            ui.painter().rect_stroke(
                inner.shrink(1.0),
                6.0,
                egui::Stroke::new(1.5, pal.accent),
                egui::StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                self.incoming =
                    egui::DragAndDrop::take_payload::<Dragged>(ui.ctx()).map(|d| (*d).clone());
            }
        }
        if ui.ctx().dragged_id().is_some()
            && let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx())
            && dragged.panel == self.id
            && let Some(pos) = ui.ctx().pointer_interact_pos()
        {
            let text = if dragged.paths.len() == 1 {
                remote::file_name(&dragged.paths[0].0).to_owned()
            } else {
                tf!("объектов: {}", "items: {}", dragged.paths.len())
            };
            let _ = pal;
            egui::Area::new(egui::Id::unique("tror_drag"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(14.0, 10.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(text).small());
                    });
                });
        }
        if let Some(path) = toggle {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path.clone());
                if !self.children.contains_key(&path) {
                    self.load(path);
                }
            }
        }
        if let Some(path) = go {
            self.list(path);
        }
        if let Some(items) = attrs {
            self.open_attrs(items);
        }
        if let Some((items, folder)) = fetch {
            self.download_into(items, &folder);
        }
        if let Some((path, local)) = fetch_as
            && let Some(folder) = local.parent()
        {
            self.download_into(vec![(path.clone(), false)], folder);
            self.save_to.insert(path, local);
        }
        for path in download {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
        }
        if !moves.is_empty() {
            self.prompt = Some(Prompt::Move(moves));
        }
        for path in walk {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Walk(path), tf!("Чтение папки {name}…", "Reading folder {name}…"));
        }
    }

    fn sync_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        use crate::sync::Mode;
        let Some(dialog) = &mut self.sync else {
            return;
        };
        let plan = dialog.plan();
        let mut go = false;
        let mut close = false;
        egui::Window::new(tr!("Синхронизация папки"))
            .frame(crate::widgets::window_frame(pal))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 460.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                crate::app::window_header(ui, pal, tr!("Синхронизация папки"), false);
                egui::Grid::new("tror_sync_roots").num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new(tr!("На компьютере")).color(pal.subtext));
                    ui.label(dialog.local_root.display().to_string());
                    ui.end_row();
                    ui.label(RichText::new(tr!("На сервере")).color(pal.subtext));
                    ui.label(dialog.remote_root.as_str());
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.radio_value(&mut dialog.mode, Mode::Both, tr!("В обе стороны — побеждает более новый файл"));
                ui.radio_value(&mut dialog.mode, Mode::Up, tr!("Только на сервер"));
                ui.radio_value(&mut dialog.mode, Mode::Down, tr!("Только на компьютер"));
                ui.checkbox(
                    &mut dialog.by_time,
                    tr!("При одинаковом размере сравнивать время изменения"),
                );
                if dialog.no_times {
                    ui.label(
                        RichText::new(tr!(
                            "Сервер не умеет ставить время файлам (нет команды MFMT), поэтому сравнение по времени выключено: иначе одни и те же файлы копировались бы каждый раз."
                        ))
                        .small()
                        .color(pal.warn),
                    );
                }
                let one_way = dialog.mode != Mode::Both;
                ui.add_enabled(
                    one_way,
                    egui::Checkbox::new(
                        &mut dialog.remove,
                        if dialog.mode == Mode::Down {
                            tr!("Удалять на компьютере файлы, которых нет на сервере")
                        } else {
                            tr!("Удалять на сервере файлы, которых нет на компьютере")
                        },
                    ),
                );
                egui::Grid::new("tror_sync_filter").num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new(tr!("Только")).color(pal.subtext));
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.include)
                            .desired_width(360.0)
                            .hint_text(tr!("все файлы; или маски через запятую: *.php, *.css")),
                    );
                    ui.end_row();
                    ui.label(RichText::new(tr!("Кроме")).color(pal.subtext));
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.exclude)
                            .desired_width(360.0)
                            .hint_text(tr!("папки и маски через запятую: bitrix/cache, *.log")),
                    );
                    ui.end_row();
                });
                ui.label(
                    RichText::new(if one_way && dialog.remove {
                        tr!("Лишние файлы на стороне назначения будут удалены без возможности восстановления.")
                    } else {
                        tr!("Ничего не удаляется: файлы, которых нет на одной стороне, копируются на неё. \
                         Файлы с тем же именем перезаписываются.")
                    })
                    .small()
                    .color(if one_way && dialog.remove { pal.warn } else { pal.faint }),
                );
                ui.add_space(6.0);
                let bytes = |steps: &[crate::sync::Step]| {
                    size_label(steps.iter().map(|s| s.size).sum::<u64>() as usize)
                };
                ui.label(tf!("На сервер: {} ({}) · на компьютер: {} ({}) · без изменений: {}", "To the server: {} ({}) · to the computer: {} ({}) · unchanged: {}",
                    plan.up.len(),
                    bytes(&plan.up),
                    plan.down.len(),
                    bytes(&plan.down),
                    plan.same
                ));
                ui.separator();
                if !plan.remove_remote.is_empty() || !plan.remove_local.is_empty() {
                    ui.label(
                        RichText::new(tf!(
                            "Будет удалено: на сервере {}, на компьютере {}",
                            "To be deleted: on the server {}, on the computer {}",
                            plan.remove_remote.len(),
                            plan.remove_local.len()
                        ))
                        .color(pal.warn),
                    );
                }
                let removals: Vec<crate::sync::Step> = plan
                    .remove_remote
                    .iter()
                    .chain(&plan.remove_local)
                    .map(|r| crate::sync::Step {
                        relative: r.clone(),
                        size: 0,
                        mtime: None,
                        fresh: false,
                    })
                    .collect();
                let remote_removals = plan.remove_remote.len();
                let rows: Vec<(u8, &crate::sync::Step)> = plan
                    .up
                    .iter()
                    .map(|s| (0, s))
                    .chain(plan.down.iter().map(|s| (1, s)))
                    .chain(removals.iter().enumerate().map(|(i, s)| (if i < remote_removals { 2 } else { 3 }, s)))
                    .collect();
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ScrollArea::vertical()
                        .max_height((ui.available_height() - 44.0).max(80.0))
                        .auto_shrink(false)
                        .show_rows(ui, 20.0, rows.len(), |ui, range| {
                            for (up, step) in &rows[range] {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 20.0),
                                    Sense::hover(),
                                );
                                let painter = ui.painter_at(rect);
                                let what = match (up, step.fresh) {
                                    (0, true) => tr!("→ сервер, новый"),
                                    (0, false) => tr!("→ сервер, заменить"),
                                    (1, true) => tr!("← компьютер, новый"),
                                    (1, false) => tr!("← компьютер, заменить"),
                                    (2, _) => tr!("× удалить на сервере"),
                                    _ => tr!("× удалить на компьютере"),
                                };
                                let removal = *up >= 2;
                                painter.text(
                                    egui::pos2(rect.left() + 4.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    what,
                                    egui::FontId::proportional(12.0),
                                    if step.fresh { pal.subtext } else { pal.warn },
                                );
                                if removal {
                                    painter.text(
                                        egui::pos2(rect.left() + 170.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        &step.relative,
                                        egui::FontId::proportional(12.5),
                                        pal.warn,
                                    );
                                    continue;
                                }
                                painter.text(
                                    egui::pos2(rect.left() + 170.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &step.relative,
                                    egui::FontId::proportional(12.5),
                                    pal.text,
                                );
                                painter.text(
                                    egui::pos2(rect.right() - 6.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    size_label(step.size as usize),
                                    egui::FontId::proportional(11.5),
                                    pal.faint,
                                );
                            }
                        });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let total = plan.up.len() + plan.down.len() + plan.remove_remote.len() + plan.remove_local.len();
                    let label = if total == 0 {
                        tr!("Всё уже совпадает").to_owned()
                    } else {
                        tf!("Синхронизировать, файлов: {total}", "Sync, files: {total}")
                    };
                    go = ui
                        .add_enabled(
                            total > 0,
                            crate::widgets::primary(label, pal).min_size(crate::widgets::ACTION),
                        ).lit(ui)
                        .clicked();
                    close = crate::app::dialog_button(ui, pal, tr!("Отмена"), false);
                });
            });
        if go {
            let Some(dialog) = self.sync.take() else {
                return;
            };
            let count = plan.up.len() + plan.down.len();
            if count > 0 {
                self.push_log(
                    tf!(
                        "Синхронизация начата: {} ↔ {}, файлов: {count}",
                        "Sync started: {} ↔ {}, files: {count}",
                        dialog.local_root.display(),
                        dialog.remote_root
                    ),
                    false,
                );
                let batch = self.batch();
                batch.downloads |= !plan.down.is_empty();
                batch.uploads |= !plan.up.is_empty();
                batch.target = dialog.remote_root.clone();
                batch.folder = Some(dialog.local_root.clone());
                for step in plan.down.iter().chain(&plan.up) {
                    batch.add(remote::join(&dialog.remote_root, &step.relative), step.size);
                }
            }
            for relative in &plan.remove_local {
                if let Some(local) = crate::sync::local_path(&dialog.local_root, relative)
                    && let Err(e) = std::fs::remove_file(&local)
                {
                    self.push_log(tf!("Не удалось удалить {relative}: {e}", "Could not delete {relative}: {e}"), true);
                }
            }
            for relative in &plan.remove_remote {
                let path = remote::join(&dialog.remote_root, relative);
                self.send(Cmd::Delete { path, dir: false }, tf!("Удаление {relative}…", "Deleting {relative}…"));
            }
            if !plan.remove_local.is_empty() || !plan.remove_remote.is_empty() {
                self.push_log(
                    tf!(
                        "Синхронизация: удалено на компьютере {}, на сервере {}",
                        "Sync: deleted on the computer {}, on the server {}",
                        plan.remove_local.len(),
                        plan.remove_remote.len()
                    ),
                    false,
                );
            }
            for step in &plan.down {
                let Some(local) = crate::sync::local_path(&dialog.local_root, &step.relative) else {
                    self.push_log(tf!("Пропущено небезопасное имя: {}", "Skipped an unsafe name: {}", step.relative), true);
                    continue;
                };
                if let Some(dir) = local.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let path = remote::join(&dialog.remote_root, &step.relative);
                if let Some(mtime) = step.mtime {
                    self.sync_times.insert(local.clone(), mtime);
                }
                self.save_to.insert(path.clone(), local);
                self.send(Cmd::Download(path), tf!("Синхронизация: {}…", "Sync: {}…", step.relative));
            }
            for step in &plan.up {
                let Some(local) = crate::sync::local_path(&dialog.local_root, &step.relative) else {
                    continue;
                };
                match std::fs::read(&local) {
                    Ok(data) => {
                        let path = remote::join(&dialog.remote_root, &step.relative);
                        self.send(
                            Cmd::Upload {
                                path: path.clone(),
                                data,
                                doc: None,
                                expect: None,
                            },
                            tf!("Синхронизация: {}…", "Sync: {}…", step.relative),
                        );
                        if let Some(mtime) = step.mtime
                            && !self.no_times
                        {
                            self.send(Cmd::Touch { path, mtime }, String::new());
                        }
                    }
                    Err(e) => {
                        self.push_log(tf!("Не удалось прочитать {}: {e}", "Could not read {}: {e}", step.relative), true)
                    }
                }
            }
        } else if close {
            self.sync = None;
        }
    }

    fn batch_tick(&mut self) -> Option<String> {
        let batch = self.batch.as_mut()?;
        if batch.finished.is_some() {
            return None;
        }
        let lost = !self.connected;
        if !lost && !batch.idle() {
            return None;
        }
        batch.lost = lost;
        let spent = batch.start.elapsed();
        batch.finished = Some(spent);
        let failed = batch.failed.len();
        let summary = tf!("{} из {}, {} за {}", "{} of {}, {} in {}",
            batch.done,
            batch.files,
            size_label(batch.bytes as usize),
            duration_label(spent)
        );
        let what = batch.noun();
        let (text, error) = if lost {
            (tf!("{what}: прервано, соединение закрыто ({summary})", "{what}: interrupted, connection closed ({summary})"), true)
        } else if batch.cancelled {
            (tf!("{what}: отменено ({summary})", "{what}: cancelled ({summary})"), false)
        } else if failed > 0 {
            (tf!("{what}: завершено с ошибками ({failed}), {summary}", "{what}: finished with errors ({failed}), {summary}"), true)
        } else {
            (tf!("{what}: завершено, {summary}", "{what}: finished, {summary}"), false)
        };
        let open = batch.open;
        self.push_log(text.clone(), error);
        (!open).then_some(text)
    }

    fn cancel_batch(&mut self) {
        let Some(batch) = &mut self.batch else {
            return;
        };
        batch.cancelled = true;
        let (drop, keep): (Vec<Transfer>, Vec<Transfer>) = self
            .transfers
            .drain(..)
            .partition(|t| batch.sizes.contains_key(t.path()));
        self.transfers = keep.into();
        for t in &drop {
            batch.sizes.remove(t.path());
        }
        let roots: Vec<String> = self
            .walks
            .iter()
            .filter(|(_, w)| matches!(w, Walk::Download(_)))
            .map(|(r, _)| r.clone())
            .collect();
        for root in roots {
            self.walks.remove(&root);
        }
        batch.walking = 0;
        self.forget(drop);
        self.paused = false;
        self.pump();
    }

    fn batch_window(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(batch) = &self.batch else {
            return;
        };
        if !batch.open {
            return;
        }
        let running = self
            .running
            .as_ref()
            .filter(|t| batch.sizes.contains_key(t.path()))
            .map(|t| remote::file_name(t.path()).to_owned());
        let current = if running.is_some() {
            self.progress.map_or(0, |(done, _)| done)
        } else {
            0
        };
        let bytes = batch.bytes + current;
        let finished = batch.finished;
        let elapsed = finished.unwrap_or_else(|| batch.start.elapsed());
        let speed = bytes as f64 / elapsed.as_secs_f64().max(0.5);
        let fraction = if batch.total > 0 {
            (bytes as f64 / batch.total as f64).min(1.0) as f32
        } else if batch.files > 0 {
            batch.done as f32 / batch.files as f32
        } else {
            0.0
        };
        let failed = batch.failed.len();
        let outcome = if batch.lost {
            tr!("Прервано: соединение закрыто")
        } else if batch.cancelled {
            tr!("Отменено")
        } else if failed > 0 {
            tr!("Завершено с ошибками")
        } else {
            tr!("Готово")
        };
        let walking = batch.walking > 0;
        let folder = batch.folder.clone();
        let target = batch.target.clone();
        let remote_dir = (batch.uploads && !batch.downloads).then(|| target.clone());
        let title = batch.noun();
        let (files, done, total) = (batch.files, batch.done, batch.total);
        let mut action = None;
        let mut open = true;
        let drag = batch.drag;
        let id = egui::ViewportId::from_hash_of(("tror_batch", self.id));
        let height = self.batch_height.max(150.0);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_inner_size([408.0, height]);
        if !self.batch_placed
            && let Some(root) = ctx.input(|i| i.viewport().outer_rect)
        {
            builder = builder.with_position(root.center() - egui::vec2(204.0, height / 2.0));
            self.batch_placed = true;
        }
        let mut used = height;
        let mut owned = self.batch_owned;
        let paused = self.paused;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if !owned {
                owned = crate::app::owner::adopt(title);
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                open = false;
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(pal.mantle)
                        .stroke(egui::Stroke::new(1.0, pal.border))
                        .inner_margin(egui::Margin::symmetric(14, 12)),
                )
                .show(ui, |ui| {
                    let grip = ui.interact(
                        ui.max_rect(),
                        ui.scope_id().with("tror_batch_drag"),
                        Sense::click_and_drag(),
                    );
                    if grip.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    let content = ui
                        .scope(|ui| {
                            ui.set_width(380.0);
                            if crate::app::window_header(ui, pal, title, true) {
                                open = false;
                            }
                            ui.add_space(2.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(tf!("Куда: {target}", "To: {target}")).color(pal.subtext),
                            )
                            .truncate(),
                        );
                        let state = if let Some(spent) = finished {
                            format!("{outcome} · {}", duration_label(spent))
                        } else if walking {
                            tr!("Подсчёт объёма…").to_owned()
                        } else if paused {
                            tr!("Пауза").to_owned()
                        } else {
                            running.clone().unwrap_or_else(|| tr!("Ожидание…").to_owned())
                        };
                        let success = finished.is_some() && outcome == tr!("Готово");
                        let (state, tone) = if success && drag {
                            (tr!("✓ Готово — отпустите файлы в нужном месте").to_owned(), crate::widgets::ok_color(ui))
                        } else if success {
                            (format!("✓ {state}"), crate::widgets::ok_color(ui))
                        } else if finished.is_some() {
                            (state, pal.warn)
                        } else {
                            (state, pal.text)
                        };
                        ui.add(egui::Label::new(RichText::new(state).color(tone)).truncate());
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_height(8.0)
                                .fill(if success { tone } else { pal.accent })
                                .corner_radius(4.0),
                        );
                        let volume = if total > 0 {
                            tf!("{} из {} · {}%", "{} of {} · {}%",
                                size_label(bytes as usize),
                                size_label(total as usize),
                                (fraction * 100.0) as u32
                            )
                        } else {
                            size_label(bytes as usize)
                        };
                        ui.label(RichText::new(volume).small().color(pal.subtext));
                        let mut line = tf!("Файлы: {done} из {files}", "Files: {done} of {files}");
                        if failed > 0 {
                            line.push_str(&tf!(" · ошибок: {failed}", " · errors: {failed}"));
                        }
                        if finished.is_none() && speed > 0.0 {
                            line.push_str(&tf!(" · {}/с", " · {}/s", size_label(speed as usize)));
                            if total > bytes && !walking {
                                let left = (total - bytes) as f64 / speed;
                                line.push_str(&tf!(" · осталось ~{}", " · about {} left",
                                    duration_label(std::time::Duration::from_secs_f64(left))
                                ));
                            }
                        }
                        let color = if failed > 0 { pal.warn } else { pal.subtext };
                        ui.label(RichText::new(line).small().color(color));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if finished.is_none() {
                                let label = if paused { tr!("Продолжить") } else { tr!("Остановить") };
                                if ui.add(crate::widgets::primary(label, pal).min_size(crate::widgets::ACTION)).lit(ui).clicked() {
                                    action = Some(0);
                                }
                                if ui.add(crate::widgets::secondary(tr!("Отменить"), pal).min_size(crate::widgets::ACTION)).clicked() {
                                    action = Some(1);
                                }
                                if ui.add(crate::widgets::secondary(tr!("Скрыть"), pal).min_size(crate::widgets::ACTION)).clicked() {
                                    action = Some(2);
                                }
                            } else {
                                if failed > 0
                                    && ui.add(crate::widgets::primary(tr!("Повторить"), pal).min_size(crate::widgets::ACTION)).lit(ui).clicked()
                                {
                                    action = Some(3);
                                }
                                if (folder.is_some() || remote_dir.is_some())
                                    && ui.add(crate::widgets::secondary(tr!("Открыть папку"), pal).min_size(crate::widgets::ACTION)).clicked()
                                {
                                    action = Some(4);
                                }
                                if ui.add(crate::widgets::secondary(tr!("Закрыть"), pal).min_size(crate::widgets::ACTION)).clicked() {
                                    action = Some(5);
                                }
                            }
                        });

                        })
                        .response
                        .rect;
                    used = (content.height() + 24.0).round();
                });
        });
        self.batch_owned = owned;
        if (used - self.batch_height).abs() > 0.5 {
            self.batch_height = used;
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::InnerSize(egui::vec2(408.0, used)));
        }
        if !open {
            action = Some(if finished.is_some() { 5 } else { 2 });
        }
        match action {
            Some(0) => {
                self.paused = !self.paused;
                self.pump();
            }
            Some(1) => self.cancel_batch(),
            Some(2) => {
                if let Some(batch) = &mut self.batch {
                    batch.open = false;
                }
            }
            Some(3) => {
                if let Some(batch) = &mut self.batch {
                    batch.failed.clear();
                    batch.finished = None;
                    batch.cancelled = false;
                }
                self.retry_failed();
            }
            Some(4) => {
                if let Some(folder) = &folder {
                    let _ = crate::preview::open_folder(folder);
                } else if let Some(dir) = remote_dir {
                    self.list(dir);
                }
            }
            Some(5) => self.batch = None,
            _ => {}
        }
        if self.batch.as_ref().is_some_and(|b| b.finished.is_none()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) -> Vec<PanelEvent> {
        let mut out = Vec::new();
        if let Some(text) = self.batch_tick() {
            out.push(PanelEvent::Toast(text));
        }
        if let Some(text) = self.notice.take() {
            out.push(PanelEvent::Toast(text));
        }
        let dropped = self.drag_done.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(dropped) = dropped
            && self.batch.as_ref().is_some_and(|b| b.drag)
        {
            self.batch = None;
            out.push(PanelEvent::Toast(
                if dropped {
                    tr!("Файлы перенесены")
                } else {
                    tr!("Перетаскивание отменено")
                }
                .to_owned(),
            ));
        }
        if self.batch.as_ref().is_some_and(|b| b.finished.is_some() && !b.open) {
            self.batch = None;
        }
        self.batch_window(ctx, pal);
        self.sync_dialog(ctx, pal);
        self.host_key_dialog(ctx, pal);
        self.prompt_dialog(ctx, pal);
        self.attrs_dialog(ctx, pal);
        self.editor_dialog(ctx, pal);
        out
    }

    fn host_key_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((fingerprint, changed)) = self.host_key.clone() else {
            return;
        };
        let mut answer = None;
        dialog(("tror_host_key", self.id), pal).show(ctx, |ui| {
            ui.set_width(460.0);
            if changed {
                ui.label(
                    RichText::new(tr!("Ключ сервера изменился"))
                        .size(crate::widgets::TITLE)
                        .color(pal.warn),
                );
                ui.label(
                    tr!("Сохранённый ключ этого сервера отличается от присланного. Так бывает после \
                     переустановки сервера, но так же выглядит перехват соединения."),
                );
            } else {
                crate::widgets::title(ui, pal, tr!("Новый сервер"));
                ui.label(
                    tr!("К этому серверу вы подключаетесь впервые. Отпечаток его ключа (SHA-256):"),
                );
            }
            ui.add_space(4.0);
            ui.label(RichText::new(&fingerprint).monospace().size(12.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let accept = if changed {
                    tr!("Заменить ключ и подключиться")
                } else {
                    tr!("Доверять и подключиться")
                };
                if dialog_button(ui, pal, accept, !changed) {
                    answer = Some(true);
                }
                if dialog_button(ui, pal, tr!("Отмена"), changed) {
                    answer = Some(false);
                }
            });
        });
        if let Some(trust) = answer {
            self.host_key = None;
            if trust {
                if let Some(conn) = &self.conn {
                    let _ = conn.tx.send(Cmd::TrustHost(true));
                    self.busy = Some(tr!("Вход…").into());
                }
            } else {
                self.disconnect();
            }
        }
    }

    fn attrs_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(state) = &mut self.attrs else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let sftp = self
            .store
            .profiles
            .get(self.selected)
            .is_some_and(|p| p.protocol == Protocol::Sftp);
        let modal = dialog(("tror_remote_attrs", self.id), pal).show(ctx, |ui| {
            ui.set_width(400.0);
            let title = if state.items.len() == 1 {
                tf!("Свойства «{}»", "Properties of \"{}\"", remote::file_name(&state.items[0].0))
            } else {
                tf!("Свойства выбранных объектов ({})", "Properties of the selected items ({})", state.items.len())
            };
            crate::widgets::title(ui, pal, title);
            ui.add_space(6.0);
            if !state.loaded {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tr!("чтение с сервера…")).color(pal.subtext));
                });
            }
            if let Some(info) = &state.info
                && state.items.len() == 1
            {
                egui::Grid::new("tror_attrs_info")
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new(tr!("Путь")).color(pal.subtext));
                        ui.label(&state.items[0].0);
                        ui.end_row();
                        if let Some(size) = info.size.filter(|_| !info.is_dir) {
                            ui.label(RichText::new(tr!("Размер")).color(pal.subtext));
                            ui.label(if size < 1024 {
                                size_label(size as usize)
                            } else {
                                tf!("{} ({size} Б)", "{} ({size} B)", size_label(size as usize))
                            });
                            ui.end_row();
                        }
                        if info.uid.is_some() || info.gid.is_some() {
                            ui.label(RichText::new(tr!("Владелец")).color(pal.subtext));
                            ui.label(format!(
                                "uid {}, gid {}",
                                info.uid.map_or("?".into(), |v| v.to_string()),
                                info.gid.map_or("?".into(), |v| v.to_string())
                            ));
                            ui.end_row();
                        }
                        if let Some(atime) = info.atime {
                            ui.label(RichText::new(tr!("Открыт")).color(pal.subtext));
                            ui.label(date_label(atime));
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
            }
            ui.label(RichText::new(tr!("Права доступа")).family(crate::theme::medium()));
            ui.add_space(2.0);
            widgets::perm_grid(ui, pal, &mut state.mode, &mut state.octal);
            let has_dirs = state.items.iter().any(|(_, dir)| *dir);
            if has_dirs {
                ui.add_space(4.0);
                ui.checkbox(&mut state.recursive, tr!("Применить ко всему содержимому папок"));
                if state.recursive {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        ui.checkbox(&mut state.dirs, tr!("к папкам"));
                        ui.checkbox(&mut state.files, tr!("к файлам"));
                    });
                    ui.label(
                        RichText::new(tr!("Обычно папкам ставят 755, файлам 644 — примените дважды с разными галочками."))
                            .small()
                            .color(pal.subtext),
                    );
                }
            }
            if state.items.len() == 1 {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(tr!("Дата изменения"));
                    ui.add(
                        TextEdit::singleline(&mut state.mtime)
                            .hint_text(tr!("ДД.ММ.ГГГГ ЧЧ:ММ"))
                            .desired_width(150.0),
                    );
                    if state.mtime != state.mtime_was && widgets::parse_date(&state.mtime).is_none()
                    {
                        ui.label(RichText::new(tr!("не дата")).color(pal.warn).small());
                    }
                });
                if !sftp {
                    ui.label(
                        RichText::new(tr!("По FTP дата меняется командой MFMT — её поддерживают не все серверы."))
                            .small()
                            .color(pal.subtext),
                    );
                }
            }
            if !sftp {
                ui.label(
                    RichText::new(tr!("По FTP права меняются командой SITE CHMOD; на Windows-серверах она обычно не работает."))
                        .small()
                        .color(pal.subtext),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Применить"), true) {
                    apply = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if apply && let Some(state) = self.attrs.take() {
            let recursive = state.recursive;
            for (path, dir) in &state.items {
                self.send(
                    Cmd::Chmod {
                        path: path.clone(),
                        mode: state.mode,
                        recursive: recursive && *dir,
                        dirs: state.dirs,
                        files: state.files,
                    },
                    tr!("Смена прав…"),
                );
            }
            if state.items.len() == 1
                && state.mtime != state.mtime_was
                && let Some(mtime) = widgets::parse_date(&state.mtime)
            {
                self.send(
                    Cmd::Touch {
                        path: state.items[0].0.clone(),
                        mtime,
                    },
                    tr!("Смена даты…"),
                );
            }
        } else if cancel || modal.should_close() {
            self.attrs = None;
        }
    }

    fn prompt_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        let mut confirmed = false;
        let mut cancelled = false;
        let modal = dialog(("tror_remote_prompt", self.id), pal).show(ctx, |ui| {
            ui.set_width(340.0);
            let (title, input, action) = match prompt {
                Prompt::NewFolder(dir, name) => (tf!("Новая папка в {dir}", "New folder in {dir}"), Some(name), tr!("Создать")),
                Prompt::NewFile(dir, name) => (tf!("Новый файл в {dir}", "New file in {dir}"), Some(name), tr!("Создать")),
                Prompt::Rename { name, .. } => (tr!("Переименовать").to_owned(), Some(name), tr!("Переименовать")),
                Prompt::Extract(items, dest) => (
                    if items.len() == 1 {
                        tf!("Распаковать «{}» в папку", "Extract \"{}\" to folder", remote::file_name(&items[0]))
                    } else {
                        tf!("Распаковать архивы ({}) в папку", "Extract archives ({}) to folder", items.len())
                    },
                    Some(dest),
                    tr!("Распаковать"),
                ),
                Prompt::Move(moves) => {
                    let dir = moves
                        .first()
                        .map(|(_, to)| remote::parent(to))
                        .unwrap_or_default();
                    if moves.len() == 1 {
                        ui.label(tf!("Переместить «{}» в папку «{}»?", "Move \"{}\" to folder \"{}\"?",
                            remote::file_name(&moves[0].0),
                            remote::file_name(&dir)
                        ));
                    } else {
                        ui.label(tf!("Переместить выбранные объекты ({}) в папку «{}»?", "Move the selected items ({}) to folder \"{}\"?",
                            moves.len(),
                            remote::file_name(&dir)
                        ));
                    }
                    ui.label(RichText::new(dir.clone()).small().color(pal.subtext));
                    (String::new(), None, tr!("Переместить"))
                }
                Prompt::Delete(items) => {
                    let folders = items.iter().filter(|(_, dir)| *dir).count();
                    if items.len() == 1 {
                        ui.label(tf!("Удалить {} «{}» с сервера?", "Delete {} \"{}\" from the server?",
                            if folders == 1 {
                                tr!("папку")
                            } else {
                                tr!("файл")
                            },
                            remote::file_name(&items[0].0)
                        ));
                    } else {
                        ui.label(tf!("Удалить с сервера выбранные объекты ({})?", "Delete the selected items from the server ({})?",
                            items.len()
                        ));
                    }
                    if folders > 0 {
                        ui.label(
                            RichText::new(
                                tr!("Папки удаляются вместе со всем содержимым. Отменить это нельзя."),
                            )
                            .color(pal.warn),
                        );
                    }
                    (String::new(), None, tr!("Удалить"))
                }
            };
            if let Some(input) = input {
                ui.add(egui::Label::new(title).truncate());
                ui.add_space(4.0);
                let field = ui.add(TextEdit::singleline(input).desired_width(f32::INFINITY));
                field.request_focus();
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    confirmed = true;
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = if action == tr!("Удалить") {
                    crate::app::danger_button(ui, pal, action)
                } else {
                    dialog_button(ui, pal, action, true)
                };
                if ok {
                    confirmed = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancelled = true;
                }
            });
        });
        if confirmed {
            let valid = |name: &str| {
                let name = name.trim();
                (!name.is_empty() && !name.contains('/')).then(|| name.to_owned())
            };
            match self.prompt.take() {
                Some(Prompt::Extract(items, dest)) => {
                    let dest = dest.trim().trim_end_matches('/');
                    if !dest.is_empty() {
                        let dest = if dest.starts_with('/') {
                            dest.to_owned()
                        } else {
                            remote::join(&self.cwd, dest)
                        };
                        for archive in items {
                            let busy = tf!("Распаковка {}…", "Extracting {}…", remote::file_name(&archive));
                            self.send(
                                Cmd::Extract {
                                    path: archive,
                                    dest: Some(dest.clone()),
                                },
                                busy,
                            );
                        }
                    }
                }
                Some(Prompt::NewFolder(dir, name)) => {
                    if let Some(name) = valid(&name) {
                        if self.tree {
                            self.expanded.insert(dir.clone());
                        }
                        self.send(Cmd::Mkdir(remote::join(&dir, &name)), tr!("Создание папки…"));
                    }
                }
                Some(Prompt::NewFile(dir, name)) => {
                    if let Some(name) = valid(&name) {
                        if self.tree {
                            self.expanded.insert(dir.clone());
                        }
                        self.send(
                            Cmd::Upload {
                                path: remote::join(&dir, &name),
                                data: Vec::new(),
                                doc: None,
                                expect: None,
                            },
                            tr!("Создание файла…"),
                        );
                    }
                }
                Some(Prompt::Rename { from, name }) => {
                    if let Some(name) = valid(&name) {
                        let to = remote::join(&remote::parent(&from), &name);
                        self.send(Cmd::Rename(from, to), tr!("Переименование…"));
                    }
                }
                Some(Prompt::Move(moves)) => {
                    for (from, to) in moves {
                        self.send(Cmd::Rename(from, to), tr!("Перемещение…"));
                    }
                }
                Some(Prompt::Delete(items)) => {
                    for (path, dir) in items {
                        self.marked.remove(&path);
                        self.expanded.remove(&path);
                        self.send(Cmd::Delete { path, dir }, tr!("Удаление…"));
                    }
                }
                None => {}
            }
        } else if cancelled || modal.should_close() {
            self.prompt = None;
        }
    }

    fn editor_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let mut test = false;
        let location = self
            .store
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let modal = dialog(("tror_profiles", self.id), pal).show(ctx, |ui| {
            ui.set_width(640.0);
            crate::widgets::title(ui, pal, tr!("Профили подключений"));
            ui.add_space(8.0);
            let limit = (ui.ctx().content_rect().height() - 200.0).max(200.0);
            ScrollArea::vertical()
                .id_salt("tror_profile_scroll")
                .max_height(limit)
                .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(180.0);
                        ScrollArea::vertical().max_height(270.0).show(ui, |ui| {
                            for (i, profile) in editor.drafts.iter().enumerate() {
                                let name = if profile.name.trim().is_empty() {
                                    tr!("без имени")
                                } else {
                                    profile.name.as_str()
                                };
                                if ui.selectable_label(i == editor.index, name).clicked() {
                                    editor.index = i;
                                    editor.result = None;
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.add(Button::new(tr!("Добавить"))).clicked() {
                                editor.drafts.push(Profile::new(tf!("Профиль {}", "Profile {}",
                                    editor.drafts.len() + 1
                                )));
                                editor.index = editor.drafts.len() - 1;
                                editor.result = None;
                            }
                            if ui
                                .add_enabled(
                                    !editor.drafts.is_empty(),
                                    Button::new(tr!("Удалить")),
                                )
                                .clicked()
                            {
                                editor.drafts.remove(editor.index);
                                editor.index = editor.index.saturating_sub(1);
                            }
                        });
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        let Some(p) = editor.drafts.get_mut(editor.index) else {
                            ui.label(RichText::new(tr!("Добавьте профиль.")).color(pal.subtext));
                            return;
                        };
                        egui::Grid::new("tror_profile_form")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                let wide = 320.0;
                                ui.label(tr!("Название"));
                                ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                                ui.end_row();
                                ui.label(tr!("Протокол"));
                                ui.horizontal_wrapped(|ui| {
                                    ui.set_max_width(wide);
                                    for protocol in Protocol::ALL {
                                        if ui
                                            .selectable_label(p.protocol == protocol, protocol.label())
                                            .clicked()
                                            && p.protocol != protocol
                                        {
                                            if p.port == p.protocol.default_port() {
                                                p.port = protocol.default_port();
                                            }
                                            p.protocol = protocol;
                                        }
                                    }
                                });
                                ui.end_row();
                                ui.label(tr!("Сервер"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        TextEdit::singleline(&mut p.host)
                                            .hint_text("example.org")
                                            .desired_width(wide - 70.0),
                                    );
                                    widgets::port_field(ui, &mut p.port, 60.0)
                                        .on_hover_text(tr!("Порт"));
                                });
                                ui.end_row();
                                ui.label(tr!("Пользователь"));
                                ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                                ui.end_row();
                                ui.label(tr!("Пароль"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        TextEdit::singleline(&mut p.password)
                                            .password(!editor.reveal)
                                            .desired_width(wide - 88.0),
                                    );
                                    let label = if editor.reveal { tr!("Скрыть") } else { tr!("Показать") };
                                    if ui
                                        .add_sized([80.0, 22.0], Button::new(label))
                                        .clicked()
                                    {
                                        editor.reveal = !editor.reveal;
                                    }
                                });
                                ui.end_row();
                                if p.protocol == Protocol::Sftp {
                                    ui.label(tr!("Файл ключа"));
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            TextEdit::singleline(&mut p.key_file)
                                                .hint_text(tr!("необязательно"))
                                                .desired_width(wide - 78.0),
                                        );
                                        if ui.button(tr!("Обзор…")).clicked()
                                            && let Some(file) = rfd::FileDialog::new().pick_file()
                                        {
                                            p.key_file = file.display().to_string();
                                        }
                                    });
                                    ui.end_row();
                                    if !p.key_file.trim().is_empty() {
                                        ui.label("");
                                        match remote::key_kind(&p.key_file) {
                                            Ok(kind) => ui.label(
                                                RichText::new(tf!("ключ: {kind}", "key: {kind}"))
                                                    .small()
                                                    .color(pal.subtext),
                                            ),
                                            Err(e) => {
                                                ui.label(RichText::new(e).small().color(pal.warn))
                                            }
                                        };
                                        ui.end_row();
                                        ui.label(tr!("Парольная фраза"));
                                        ui.add(
                                            TextEdit::singleline(&mut p.passphrase)
                                                .password(!editor.reveal)
                                                .hint_text(tr!("если ключ защищён"))
                                                .desired_width(wide),
                                        );
                                        ui.end_row();
                                    }
                                }
                                if p.protocol.tls() {
                                    ui.label(tr!("Сертификат"));
                                    ui.checkbox(&mut p.insecure_tls, tr!("не проверять (самоподписанный)"));
                                    ui.end_row();
                                }
                                ui.label("");
                                ui.vertical(|ui| {
                                    ui.set_max_width(wide);
                                    if editor.probe.is_some() {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(RichText::new(tr!("проверка…")).color(pal.subtext));
                                        });
                                    } else if ui
                                        .add(Button::new(tr!("Проверить подключение")))
                                        .clicked()
                                    {
                                        test = true;
                                    }
                                    if let Some(result) = &editor.result {
                                        let (text, color) = match result {
                                            Ok(text) => (tf!("Работает: {text}", "Works: {text}"), crate::widgets::ok_color(ui)),
                                            Err(e) => (tf!("Ошибка: {e}", "Error: {e}"), pal.warn),
                                        };
                                        ui.label(RichText::new(text).small().color(color));
                                    }
                                });
                                ui.end_row();
                                ui.label(tr!("Начальная папка"));
                                ui.add(
                                    TextEdit::singleline(&mut p.directory)
                                        .hint_text(tr!("домашняя папка"))
                                        .desired_width(wide),
                                );
                                ui.end_row();
                                ui.label(tr!("Адрес сайта"));
                                ui.add(
                                    TextEdit::singleline(&mut p.url)
                                        .hint_text(tr!("https://example.ru — для предпросмотра (F12)"))
                                        .desired_width(wide),
                                );
                                ui.end_row();
                                if p.protocol == Protocol::Sftp {
                                    ui.label("");
                                    ui.label(RichText::new(tr!("Консоль по этому профилю")).color(pal.subtext).small());
                                    ui.end_row();
                                    crate::console::appearance_rows(
                                        ui,
                                        pal,
                                        &mut p.term_theme,
                                        &mut p.term_font,
                                        &mut p.term_font_size,
                                        wide,
                                    );
                                }
                            });
                        ui.add_space(4.0);
                        let (text, color) = match p.protocol {
                            Protocol::Sftp => (
                                tr!("Вход по паролю, по ключу (RSA, ECDSA, Ed25519, PuTTY .ppk) или через ssh-agent, если не заданы ни пароль, ни ключ. Если у ключа нет отдельной парольной фразы, используется поле «Пароль»."),
                                pal.subtext,
                            ),
                            Protocol::Ftp => (tr!("FTP передаёт пароль и файлы без шифрования."), pal.warn),
                            _ if p.insecure_tls => (
                                tr!("Соединение шифруется, но подлинность сервера не проверяется — \
                                 включайте только для своего сервера с самоподписанным сертификатом."),
                                pal.warn,
                            ),
                            Protocol::Ftps => (
                                tr!("FTPS с явным TLS (AUTH TLS, обычно порт 21): шифруются и команды, и данные."),
                                pal.subtext,
                            ),
                            Protocol::FtpsImplicit => (
                                tr!("Неявный FTPS (обычно порт 990): TLS включается сразу при соединении. Старый способ — выбирайте его, только если хостинг не поддерживает явный FTPS."),
                                pal.subtext,
                            ),
                        };
                        ui.label(RichText::new(text).color(color).small());
                    });
                });
                });
            ui.add_space(10.0);
            ui.label(
                RichText::new(tf!("Профили хранятся в {location}. Пароли в файле скрыты от случайного взгляда, но не зашифрованы.", "Profiles are stored in {location}. Passwords in the file are hidden from a casual glance, but not encrypted."
                ))
                .color(pal.subtext)
                .small(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Сохранить"), true) {
                    save = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if test
            && let Some(editor) = &mut self.editor
            && let Some(profile) = editor.drafts.get(editor.index).cloned()
        {
            let key = format!("{}:{}", profile.host.trim(), profile.port);
            let known = self.store.known_hosts.get(&key).cloned();
            editor.result = None;
            editor.probe = Some(crate::console::probe(move || remote::test(&profile, known)));
        }
        if save
            && let Some(editor) = &mut self.editor
            && editor
                .drafts
                .get(editor.index)
                .is_some_and(|p| p.host.trim().is_empty() && !self.store.profiles.contains(p))
        {
            editor.result = Some(Err(tr!("Укажите сервер").into()));
            save = false;
        }
        if save {
            if let Some(mut editor) = self.editor.take() {
                let chosen = editor.drafts.get(editor.index).cloned();
                let known = &self.store.profiles;
                editor
                    .drafts
                    .retain(|p| !p.host.trim().is_empty() || known.contains(p));
                self.selected = chosen
                    .and_then(|c| editor.drafts.iter().position(|p| *p == c))
                    .unwrap_or(0);
                self.store.profiles = editor.drafts;
                if let Err(e) = self.store.save() {
                    self.busy = Some(tf!("Профили не сохранены: {e}", "Profiles not saved: {e}"));
                }
            }
        } else if cancel || modal.should_close() {
            self.editor = None;
        }
    }
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
