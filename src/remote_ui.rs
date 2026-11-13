use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui::{
    self, Button, Color32, ComboBox, Key, Rect, RichText, ScrollArea, Sense, TextEdit, Ui,
};

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
    Listed {
        profile: String,
        dir: String,
        names: Vec<String>,
    },
    OpenLocal(Vec<PathBuf>),
    Toast(String),
    Relay {
        target: u8,
        path: String,
        data: Vec<u8>,
    },
}

enum Walk {
    Size,
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
    walks: HashMap<String, Walk>,
    pending: usize,
    progress: Option<(u64, u64)>,
    store_seen: u64,
    pub store: Store,
    selected: usize,
    conn: Option<Connection>,
    connected: bool,
    queue: Vec<Cmd>,
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
    attrs: Option<AttrsDialog>,
    host_key: Option<(String, bool)>,
    editor: Option<ProfileEditor>,
    save_to: HashMap<String, PathBuf>,
    dropped: Option<Vec<PathBuf>>,
    pub want_search: bool,
    search_query: String,
    search_content: bool,
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
            walks: HashMap::new(),
            pending: 0,
            progress: None,
            store_seen: Store::generation(),
            want_search: false,
            search_query: String::new(),
            search_content: true,
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
            attrs: None,
            host_key: None,
            editor: None,
            save_to: HashMap::new(),
            dropped: None,
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
            || self.dropped.is_some()
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

    pub fn offer_drop(&mut self, paths: Vec<PathBuf>) {
        self.dropped = Some(paths);
    }

    fn push_log(&mut self, text: String, error: bool) {
        crate::trace::line(text, error);
    }

    pub fn site_url(&self, profile: &str, path: &str) -> Option<String> {
        let profile = self.store.profiles.iter().find(|p| p.name == profile)?;
        crate::preview::remote_url(&profile.url, &profile.directory, path)
    }

    fn send(&mut self, cmd: Cmd, busy: impl Into<String>) {
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
        self.send(Cmd::List(path), "Чтение каталога…");
    }

    fn load(&mut self, path: String) {
        if self.loading.insert(path.clone()) {
            self.send(Cmd::List(path), "Чтение каталога…");
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
        self.busy = Some("Подключение…".into());
    }

    pub fn upload(
        &mut self,
        ctx: &egui::Context,
        profile: &str,
        path: &str,
        data: Vec<u8>,
        doc: (u64, u64),
    ) -> Result<&'static str, String> {
        let cmd = Cmd::Upload {
            path: path.to_owned(),
            data,
            doc: Some(doc),
        };
        let busy = format!("Загрузка {}…", remote::file_name(path));
        if let Some(conn) = &self.conn
            && conn.profile == profile
        {
            if self.connected {
                self.send(cmd, busy);
                return Ok("Загрузка на сервер…");
            }
            self.queue.push(cmd);
            return Ok("Файл будет загружен после подключения…");
        }
        let Some(index) = self.store.profiles.iter().position(|p| p.name == profile) else {
            return Err(format!("профиль «{profile}» не найден"));
        };
        if self.conn.is_none() {
            self.selected = index;
            self.connect(ctx);
            self.queue.push(cmd);
            return Ok("Подключаюсь к серверу, чтобы сохранить файл…");
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
        Ok("Сохраняю через отдельное подключение…")
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
                    self.push_log(format!("Не удалось прочитать папку {relative}: {e}"), true)
                }
            }
        }
        files.sort_by(|a, b| a.1.cmp(&b.1));
        for (local, relative) in files {
            match std::fs::read(&local) {
                Ok(data) => {
                    let path = remote::join(dir, &relative);
                    self.send(
                        Cmd::Upload {
                            path,
                            data,
                            doc: None,
                        },
                        format!("Загрузка {relative}…"),
                    );
                }
                Err(e) => self.push_log(format!("Не удалось прочитать {relative}: {e}"), true),
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
                Event::Log(text, error) => logs.push((text, error)),
                Event::HostKey { .. } => {
                    let _ = saver.conn.tx.send(Cmd::TrustHost(false));
                    out.push(PanelEvent::Toast(
                        "Не сохранено: ключ сервера нужно подтвердить — подключитесь к нему в панели".into(),
                    ));
                }
                Event::Error(e) | Event::Closed(e) => {
                    out.push(PanelEvent::Toast(format!("Не сохранено: {e}")));
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

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn reopen(&mut self, ctx: &egui::Context, profile: &str, path: &str) -> Result<(), String> {
        let cmd = Cmd::Download(path.to_owned());
        let busy = format!("Скачивание {}…", remote::file_name(path));
        if self.serves(profile) {
            if self.connected {
                self.send(cmd, busy);
            } else {
                self.queue.push(cmd);
            }
            return Ok(());
        }
        if self.conn.is_some() {
            return Err(format!("панель подключена к другому серверу, а файл лежит на «{profile}»"));
        }
        let index = self
            .store
            .profiles
            .iter()
            .position(|p| p.name == profile)
            .ok_or_else(|| format!("профиль «{profile}» не найден"))?;
        self.selected = index;
        self.connect(ctx);
        self.queue.push(cmd);
        Ok(())
    }

    pub fn rename_file(&mut self, profile: &str, from: &str, to: &str) -> Result<(), String> {
        if !self.serves(profile) || !self.connected {
            return Err(format!("сначала подключитесь к «{profile}»"));
        }
        self.send(Cmd::Rename(from.to_owned(), to.to_owned()), "Переименование…");
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
                self.send(Cmd::Walk(path), format!("Чтение папки {name}…"));
                continue;
            }
            self.relay
                .insert(path.clone(), (target, remote::join(target_dir, &name)));
            self.send(Cmd::Download(path), format!("Передача {name}…"));
        }
    }

    pub fn receive(&mut self, path: String, data: Vec<u8>) {
        let name = remote::file_name(&path).to_owned();
        self.send(
            Cmd::Upload {
                path,
                data,
                doc: None,
            },
            format!("Приём {name}…"),
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
        let mut out = Vec::new();
        if let Some(editor) = &mut self.editor
            && let Some(rx) = &editor.probe
            && let Ok(result) = rx.try_recv()
        {
            editor.probe = None;
            match &result {
                Ok(text) => crate::trace::popup(format!("Подключение работает: {text}"), true),
                Err(e) => crate::trace::popup(format!("Не удалось подключиться: {e}"), false),
            }
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
                        self.search_status = format!("ошибка: {e}");
                        finished = true;
                    }
                }
            }
            if finished {
                let status = self.search_status.clone();
                self.push_log(format!("Поиск на сервере: {status}"), false);
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
                    self.pending = 0;
                    if let (Some(print), Some(p)) =
                        (fingerprint, self.store.profiles.get(self.selected))
                    {
                        let key = format!("{}:{}", p.host.trim(), p.port);
                        if let Err(e) = self.trust(key, print) {
                            out.push(PanelEvent::Toast(format!("Не удалось сохранить ключ: {e}")));
                        }
                    }
                    for cmd in std::mem::take(&mut self.queue) {
                        self.send(cmd, "Загрузка…");
                    }
                    self.list(home);
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
                    if let Some((target, to)) = self.relay.remove(&path) {
                        out.push(PanelEvent::Relay {
                            target,
                            path: to,
                            data,
                        });
                        continue;
                    }
                    match self.save_to.remove(&path) {
                        Some(local) => {
                            let (text, error) = match std::fs::write(&local, data) {
                                Ok(()) => (format!("Скачано: {}", local.display()), false),
                                Err(e) => (
                                    format!("Не удалось записать {}: {e}", local.display()),
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
                Event::Uploaded { path, doc } => {
                    self.finished();
                    let dir = remote::parent(&path);
                    match doc {
                        Some(doc) => out.push(PanelEvent::Uploaded { doc, path }),
                        None => out.push(PanelEvent::Toast(format!(
                            "Загружено: {}",
                            remote::file_name(&path)
                        ))),
                    }
                    if self.pending == 0 || dir != self.cwd {
                        self.refresh_dir(&dir);
                    }
                }
                Event::Progress { done, total } => self.progress = Some((done, total)),
                Event::Walked { root, files } => {
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
                        Some(Walk::Size) => {
                            let bytes: u64 = files.iter().map(|(_, size)| size).sum();
                            let text = format!(
                                "Папка {name}: файлов {}, {}",
                                files.len(),
                                size_label(bytes as usize)
                            );
                            self.push_log(text.clone(), false);
                            out.push(PanelEvent::Toast(text));
                        }
                        Some(Walk::Download(folder)) => {
                            for (path, _) in files {
                                let mut local = folder.clone();
                                local.extend(relative(&path).split('/'));
                                if let Some(dir) = local.parent() {
                                    let _ = std::fs::create_dir_all(dir);
                                }
                                self.save_to.insert(path.clone(), local);
                                self.send(Cmd::Download(path), format!("Скачивание папки {name}…"));
                            }
                        }
                        Some(Walk::Relay(target, dir)) => {
                            for (path, _) in files {
                                self.relay.insert(
                                    path.clone(),
                                    (target, remote::join(&dir, &relative(&path))),
                                );
                                self.send(Cmd::Download(path), format!("Передача папки {name}…"));
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
                    if dir != self.cwd {
                        self.push_log(format!("Изменён каталог {dir}"), false);
                    }
                    self.refresh_dir(&dir);
                    if dir != self.cwd && !self.children.contains_key(&dir) {
                        self.list(self.cwd.clone());
                    }
                }
                Event::Error(e) => {
                    self.finished();
                    self.loading.clear();
                    out.push(PanelEvent::Toast(format!("Ошибка сервера: {e}")));
                }
                Event::Closed(e) => {
                    self.disconnect();
                    out.push(PanelEvent::Toast(format!("Соединение закрыто: {e}")));
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
            drafts.push(Profile::new("Новый профиль".into()));
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
        self.send(Cmd::Stat(first), "Чтение свойств…");
    }

    fn menu_items(&mut self, ui: &mut Ui, dir: &str) {
        if ui.button("Новый файл…").clicked() {
            self.prompt = Some(Prompt::NewFile(dir.to_owned(), String::new()));
            ui.close();
        }
        if ui.button("Новая папка…").clicked() {
            self.prompt = Some(Prompt::NewFolder(dir.to_owned(), String::new()));
            ui.close();
        }
        if ui.button("Загрузить файлы с компьютера…").clicked() {
            ui.close();
            if let Some(files) = rfd::FileDialog::new().pick_files() {
                self.upload_local_to(files, dir);
            }
        }
        if ui.button("Загрузить папку с компьютера…").clicked() {
            ui.close();
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                self.upload_local_to(vec![folder], dir);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        ui.take_available_width();
        ui.add_space(8.0);
        if self.store.profiles.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Подключений пока нет.").color(pal.subtext));
            if ui
                .add(Button::new("Создать профиль").fill(pal.surface))
                .clicked()
            {
                self.open_editor();
            }
            return;
        }
        self.selected = self.selected.min(self.store.profiles.len() - 1);
        let active = self.conn.is_some();
        let mut edit = false;
        let label = if active { "Отключить" } else { "Подключить" };
        let button_w = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                label.to_owned(),
                egui::TextStyle::Button.resolve(ui.style()),
                pal.text,
            )
            .size()
            .x
        }) + 22.0;
        let stacked = ui.available_width() < button_w + 110.0;
        let mut toggle = false;
        let mut combo = |ui: &mut Ui, width: f32| {
            ui.add_enabled_ui(!active, |ui| {
                ComboBox::from_id_salt(("tror_profile", self.id))
                    .width(width)
                    .truncate()
                    .selected_text(self.store.profiles[self.selected].name.clone())
                    .show_ui(ui, |ui| {
                        for (i, profile) in self.store.profiles.iter().enumerate() {
                            let text = format!("{}  ·  {}", profile.name, profile.protocol.label());
                            ui.selectable_value(&mut self.selected, i, text);
                        }
                        ui.separator();
                        if ui.button("Настроить профили…").clicked() {
                            edit = true;
                        }
                    });
            });
        };
        if stacked {
            let width = (ui.available_width() - 4.0).max(20.0);
            combo(ui, width);
            if ui
                .add_sized([width, 24.0], Button::new(label).fill(pal.surface).truncate())
                .clicked()
            {
                toggle = true;
            }
        } else {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - button_w - 10.0).max(40.0);
                combo(ui, width);
                if ui
                    .add_sized([button_w, 24.0], Button::new(label).fill(pal.surface))
                    .clicked()
                {
                    toggle = true;
                }
            });
        }
        if toggle {
            if active {
                self.disconnect();
            } else {
                self.connect(&ctx);
            }
        }
        if edit {
            self.open_editor();
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
                    .on_hover_text("Обновить каталог")
                    .clicked()
                {
                    self.list(self.cwd.clone());
                    let open: Vec<String> = self.expanded.iter().cloned().collect();
                    for dir in open {
                        self.load(dir);
                    }
                }
                let dots = dots_button(ui, pal).on_hover_text("Ещё");
                egui::Popup::menu(&dots)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                    ui.set_min_width(230.0);
                    if ui.button("Поиск на сервере…").clicked() {
                        self.want_search = true;
                        ui.close();
                    }
                    ui.separator();
                    let cwd = self.cwd.clone();
                    self.menu_items(ui, &cwd);
                    ui.separator();
                    let mut tree = self.tree;
                    ui.horizontal(|ui| {
                        ui.label("Вид:");
                        if ui.selectable_label(!tree, "список").clicked() {
                            tree = false;
                        }
                        if ui.selectable_label(tree, "дерево").clicked() {
                            tree = true;
                        }
                    });
                    if tree != self.tree {
                        crate::app::request_view(Some(tree), None);
                    }
                    let mut size = !self.store.hide_size;
                    if ui.checkbox(&mut size, "Показывать размер").changed() {
                        self.store.hide_size = !size;
                        let _ = self.store.save();
                    }
                    if ui
                        .checkbox(&mut self.store.show_date, "Показывать дату изменения")
                        .changed()
                    {
                        let _ = self.store.save();
                    }
                    if ui
                        .checkbox(&mut self.store.show_perm, "Показывать права")
                        .changed()
                    {
                        let _ = self.store.save();
                    }
                });
            });
            self.listing(ui, pal);
        }
        if let Some(busy) = &self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                let queue = if self.pending > 1 {
                    format!("  ·  в очереди {}", self.pending)
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
                            .size(12.5),
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

    fn start_search(&mut self, ctx: &egui::Context) {
        let query = self.search_query.trim().to_owned();
        if query.is_empty() || !self.connected {
            return;
        }
        let Some((profile, known)) = self.selected_profile() else {
            return;
        };
        let params = remote::SearchParams {
            root: self.cwd.clone(),
            query: query.clone(),
            content: self.search_content,
            excludes: self
                .search_excludes
                .split(',')
                .map(|e| e.trim().to_owned())
                .collect(),
        };
        let ctx = ctx.clone();
        let job = remote::search(profile, known, params, move || ctx.request_repaint());
        self.hits.clear();
        self.search_status = "поиск…".into();
        self.push_log(format!("Поиск «{query}» в {}", self.cwd), false);
        self.search_job = Some(job);
        if self.store.search_excludes.as_deref() != Some(self.search_excludes.as_str()) {
            self.store.search_excludes = Some(self.search_excludes.clone());
            let _ = self.store.save();
        }
    }

    pub fn search_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, pal.base);
        ui.add_space(6.0);
        if !self.connected {
            ui.add(
                egui::Label::new(
                    RichText::new("  Подключитесь к серверу в панели файлов (F8) — поиск идёт в открытой там папке.")
                        .color(pal.faint)
                        .size(12.5),
                )
                .wrap(),
            );
            return;
        }
        let mut start = false;
        crate::widgets::wrapped_row(ui, 6.0, |ui| {
            let field_w = (ui.available_width() - 20.0).clamp(80.0, 280.0);
            let field = ui.add(
                TextEdit::singleline(&mut self.search_query)
                    .hint_text(format!("Искать в {}", self.cwd))
                    .desired_width(field_w),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                start = true;
            }
            for (content, label) in [(true, "В тексте файлов"), (false, "В именах")]
            {
                let active = self.search_content == content;
                let mut button = Button::new(RichText::new(label).size(12.5));
                if active {
                    button = button.fill(pal.surface);
                }
                if ui.add(button).clicked() {
                    self.search_content = content;
                }
            }
            if self.search_job.is_some() {
                if ui.add(Button::new("Стоп").fill(pal.surface)).clicked()
                    && let Some(job) = &self.search_job
                {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ui.spinner();
            } else if ui.add(Button::new("Найти").fill(pal.surface)).clicked() {
                start = true;
            }
            ui.label(
                RichText::new(&self.search_status)
                    .color(pal.subtext)
                    .size(12.5),
            );
        });
        crate::widgets::wrapped_row(ui, 6.0, |ui| {
            ui.label(
                RichText::new("Не искать в папках:")
                    .color(pal.subtext)
                    .size(12.5),
            );
            ui.add(
                TextEdit::singleline(&mut self.search_excludes)
                    .desired_width((ui.available_width() - 12.0).max(80.0))
                    .font(egui::TextStyle::Small),
            );
        });
        if start && self.search_job.is_none() {
            self.start_search(&ctx);
        }
        let mut open = None;
        let row_h = 20.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_h,
            self.hits.len(),
            |ui, rows| {
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
                        .strip_prefix(self.cwd.trim_end_matches('/'))
                        .map_or(path.as_str(), |p| p.trim_start_matches('/'));
                    let label = match line {
                        Some(n) => format!("{shown}:{n}"),
                        None => shown.to_owned(),
                    };
                    let placed = painter.text(
                        egui::pos2(rect.left() + 10.0, rect.center().y),
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
                    if response.double_clicked() {
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
            self.send(Cmd::Download(path), format!("Скачивание {name}…"));
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
        if !self.tree {
            for entry in &self.entries {
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
            for entry in entries {
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
            for entry in &self.entries {
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
        let mut moves: Vec<(String, String)> = Vec::new();
        let mut walk: Vec<String> = Vec::new();
        let mut toggle: Option<String> = None;
        let mut attrs: Option<Vec<(String, bool)>> = None;
        let reserve = if self.busy.is_some() { 34.0 } else { 0.0 };
        let tree = self.tree;
        crate::widgets::column_header(ui, pal, [show_size, show_perm, show_date]);
        let widths = crate::widgets::columns();
        let gap = crate::widgets::COLUMN_GAP;
        ui.spacing_mut().item_spacing.y = 0.0;
        let outer = ui.available_rect_before_wrap();
        let height = (ui.available_height() - reserve).max(40.0);
        let blank = ui.interact(
            Rect::from_min_size(outer.min, egui::vec2(outer.width(), height)),
            ui.scope_id().with(("tror_remote_blank", self.id)),
            Sense::click(),
        );
        let area = ScrollArea::vertical()
            .id_salt(("tror_remote_rows", self.id))
            .auto_shrink(false)
            .max_height((ui.available_height() - reserve).max(40.0))
            .show_rows(ui, row_h, total, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let row = &rows[index];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
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
                    let painter = ui.painter_at(rect);
                    if drop_target {
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
                    let mut right = rect.right() - 6.0;
                    let small = egui::FontId::proportional(11.5);
                    let mut room = rect.width() - indent - 90.0;
                    let mut fits = |on: bool, width: f32| {
                        let yes = on && room > width + gap;
                        if yes {
                            room -= width + gap;
                        }
                        yes
                    };
                    let show_size = fits(show_size, widths[0]);
                    let show_date = fits(show_date, widths[2]);
                    let show_perm = fits(show_perm, widths[1]);
                    if show_date {
                        if let Some(secs) = entry.as_ref().and_then(|e| e.mtime) {
                            painter.text(
                                egui::pos2(right, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                date_label(secs),
                                small.clone(),
                                pal.faint,
                            );
                        }
                        right -= widths[2] + gap;
                    }
                    if show_perm {
                        if let Some(mode) = entry.as_ref().and_then(|e| e.perm) {
                            painter.text(
                                egui::pos2(right, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                format!("{mode:03o}"),
                                egui::FontId::monospace(11.0),
                                pal.faint,
                            );
                        }
                        right -= widths[1] + gap;
                    }
                    if show_size {
                        if let Some(e) = entry.as_ref().filter(|e| !e.is_dir) {
                            painter.text(
                                egui::pos2(right, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                size_label(e.size as usize),
                                small,
                                pal.faint,
                            );
                        }
                        right -= widths[0] + gap;
                    }
                    let name_clip = Rect::from_min_max(
                        egui::pos2(rect.left() + indent + 28.0, rect.top()),
                        egui::pos2(right.max(rect.left() + indent + 60.0), rect.bottom()),
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
                            if !files.is_empty() && ui.button("Открыть").clicked() {
                                download.extend(files.iter().cloned());
                            }
                            if chosen.len() == 1 && is_dir {
                                if ui.button("Новый файл здесь…").clicked() {
                                    self.prompt = Some(Prompt::NewFile(path.clone(), String::new()));
                                }
                                if ui.button("Новая папка здесь…").clicked() {
                                    self.prompt =
                                        Some(Prompt::NewFolder(path.clone(), String::new()));
                                }
                                if ui.button("Загрузить сюда файлы…").clicked()
                                    && let Some(files) = rfd::FileDialog::new().pick_files()
                                {
                                    self.upload_local_to(files, &path);
                                }
                                ui.separator();
                            }
                            if files.len() == 1
                                && ui.button("Скачать как…").clicked()
                                && let Some(local) = rfd::FileDialog::new()
                                    .set_file_name(remote::file_name(&files[0]))
                                    .save_file()
                            {
                                self.save_to.insert(files[0].clone(), local);
                                download.push(files[0].clone());
                            }
                            if (files.len() > 1 || !dirs.is_empty())
                                && ui.button("Скачать в папку…").clicked()
                                && let Some(folder) = rfd::FileDialog::new().pick_folder()
                            {
                                for file in &files {
                                    self.save_to
                                        .insert(file.clone(), folder.join(remote::file_name(file)));
                                    download.push(file.clone());
                                }
                                for dir in &dirs {
                                    self.walks
                                        .insert(dir.clone(), Walk::Download(folder.clone()));
                                    walk.push(dir.clone());
                                }
                            }
                            if !dirs.is_empty() && ui.button("Размер папки").clicked() {
                                for dir in &dirs {
                                    self.walks.insert(dir.clone(), Walk::Size);
                                    walk.push(dir.clone());
                                }
                            }
                            for (peer, label) in self.peers.clone() {
                                if ui.button(format!("Передать на «{label}»")).clicked() {
                                    self.outgoing = Some((peer, chosen.clone()));
                                }
                            }
                            ui.separator();
                            if ui.button("Права и атрибуты…").clicked() {
                                attrs = Some(chosen.clone());
                            }
                            if ui.button("Копировать путь").clicked() {
                                let text: Vec<&str> =
                                    chosen.iter().map(|(p, _)| p.as_str()).collect();
                                ui.ctx().copy_text(text.join("\n"));
                            }
                            if chosen.len() == 1 && ui.button("Переименовать…").clicked()
                            {
                                self.prompt = Some(Prompt::Rename {
                                    from: path.clone(),
                                    name: name.clone(),
                                });
                            }
                            let label = if chosen.len() > 1 {
                                format!("Удалить выбранное ({})…", chosen.len())
                            } else {
                                "Удалить…".to_owned()
                            };
                            if ui.button(label).clicked() {
                                self.prompt = Some(Prompt::Delete(chosen));
                            }
                        });
                    }
                }
            });
        let inner = area.inner_rect;
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
            self.menu_items(ui, &cwd);
            ui.separator();
            if ui.button("Обновить").clicked() {
                self.list(cwd.clone());
                ui.close();
            }
            if ui.button("Копировать путь папки").clicked() {
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
                format!("объектов: {}", dragged.paths.len())
            };
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::unique("tror_drag"),
                ))
                .text(
                    pos + egui::vec2(14.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    egui::FontId::proportional(12.5),
                    pal.text,
                );
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
        for path in download {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Download(path), format!("Скачивание {name}…"));
        }
        for (from, to) in moves {
            self.send(Cmd::Rename(from, to), "Перемещение…");
        }
        for path in walk {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Walk(path), format!("Чтение папки {name}…"));
        }
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) -> Vec<PanelEvent> {
        let mut out = Vec::new();
        self.host_key_dialog(ctx, pal);
        self.prompt_dialog(ctx, pal);
        self.attrs_dialog(ctx, pal);
        self.editor_dialog(ctx, pal);
        self.drop_dialog(ctx, pal, &mut out);
        out
    }

    fn drop_dialog(&mut self, ctx: &egui::Context, pal: &Palette, out: &mut Vec<PanelEvent>) {
        let Some(paths) = &self.dropped else {
            return;
        };
        let mut choice = None;
        let count = paths.len();
        let modal = dialog(("tror_drop_choice", self.id), pal).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(format!("Что сделать с перетащенными файлами ({count})?"));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if dialog_button(ui, pal, "Открыть в редакторе", true) {
                    choice = Some(0);
                }
                if dialog_button(ui, pal, &format!("Загрузить в {}", self.cwd), false) {
                    choice = Some(1);
                }
                if dialog_button(ui, pal, "Отмена", false) {
                    choice = Some(2);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(2);
        }
        if let Some(choice) = choice {
            let paths = self.dropped.take().unwrap_or_default();
            match choice {
                0 => out.push(PanelEvent::OpenLocal(paths)),
                1 => self.upload_local(paths),
                _ => {}
            }
        }
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
                    RichText::new("Ключ сервера изменился")
                        .color(pal.warn)
                        .strong(),
                );
                ui.label(
                    "Сохранённый ключ этого сервера отличается от присланного. Так бывает после \
                     переустановки сервера, но так же выглядит перехват соединения.",
                );
            } else {
                ui.label(RichText::new("Новый сервер").strong());
                ui.label(
                    "К этому серверу вы подключаетесь впервые. Отпечаток его ключа (SHA-256):",
                );
            }
            ui.add_space(4.0);
            ui.label(RichText::new(&fingerprint).monospace().size(12.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let accept = if changed {
                    "Заменить ключ и подключиться"
                } else {
                    "Доверять и подключиться"
                };
                if dialog_button(ui, pal, accept, !changed) {
                    answer = Some(true);
                }
                if dialog_button(ui, pal, "Отмена", changed) {
                    answer = Some(false);
                }
            });
        });
        if let Some(trust) = answer {
            self.host_key = None;
            if trust {
                if let Some(conn) = &self.conn {
                    let _ = conn.tx.send(Cmd::TrustHost(true));
                    self.busy = Some("Вход…".into());
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
                format!("Свойства «{}»", remote::file_name(&state.items[0].0))
            } else {
                format!("Свойства выбранных объектов ({})", state.items.len())
            };
            ui.label(RichText::new(title).strong());
            ui.add_space(6.0);
            if !state.loaded {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("чтение с сервера…").color(pal.subtext));
                });
            }
            if let Some(info) = &state.info
                && state.items.len() == 1
            {
                egui::Grid::new("tror_attrs_info")
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Путь").color(pal.subtext));
                        ui.label(&state.items[0].0);
                        ui.end_row();
                        if let Some(size) = info.size.filter(|_| !info.is_dir) {
                            ui.label(RichText::new("Размер").color(pal.subtext));
                            ui.label(if size < 1024 {
                                size_label(size as usize)
                            } else {
                                format!("{} ({size} Б)", size_label(size as usize))
                            });
                            ui.end_row();
                        }
                        if info.uid.is_some() || info.gid.is_some() {
                            ui.label(RichText::new("Владелец").color(pal.subtext));
                            ui.label(format!(
                                "uid {}, gid {}",
                                info.uid.map_or("?".into(), |v| v.to_string()),
                                info.gid.map_or("?".into(), |v| v.to_string())
                            ));
                            ui.end_row();
                        }
                        if let Some(atime) = info.atime {
                            ui.label(RichText::new("Открыт").color(pal.subtext));
                            ui.label(date_label(atime));
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
            }
            ui.label(RichText::new("Права доступа").family(crate::theme::medium()));
            ui.add_space(2.0);
            widgets::perm_grid(ui, pal, &mut state.mode, &mut state.octal);
            let has_dirs = state.items.iter().any(|(_, dir)| *dir);
            if has_dirs {
                ui.add_space(4.0);
                ui.checkbox(&mut state.recursive, "Применить ко всему содержимому папок");
                if state.recursive {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        ui.checkbox(&mut state.dirs, "к папкам");
                        ui.checkbox(&mut state.files, "к файлам");
                    });
                    ui.label(
                        RichText::new("Обычно папкам ставят 755, файлам 644 — примените дважды с разными галочками.")
                            .size(12.0)
                            .color(pal.subtext),
                    );
                }
            }
            if state.items.len() == 1 {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Дата изменения");
                    ui.add(
                        TextEdit::singleline(&mut state.mtime)
                            .hint_text("ДД.ММ.ГГГГ ЧЧ:ММ")
                            .desired_width(150.0),
                    );
                    if state.mtime != state.mtime_was && widgets::parse_date(&state.mtime).is_none()
                    {
                        ui.label(RichText::new("не дата").color(pal.warn).size(12.0));
                    }
                });
                if !sftp {
                    ui.label(
                        RichText::new("По FTP дата меняется командой MFMT — её поддерживают не все серверы.")
                            .size(12.0)
                            .color(pal.subtext),
                    );
                }
            }
            if !sftp {
                ui.label(
                    RichText::new("По FTP права меняются командой SITE CHMOD; на Windows-серверах она обычно не работает.")
                        .size(12.0)
                        .color(pal.subtext),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Применить", true) {
                    apply = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
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
                    "Смена прав…",
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
                    "Смена даты…",
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
                Prompt::NewFolder(dir, name) => (format!("Новая папка в {dir}"), Some(name), "Создать"),
                Prompt::NewFile(dir, name) => (format!("Новый файл в {dir}"), Some(name), "Создать"),
                Prompt::Rename { name, .. } => ("Переименовать".to_owned(), Some(name), "Переименовать"),
                Prompt::Delete(items) => {
                    let folders = items.iter().filter(|(_, dir)| *dir).count();
                    if items.len() == 1 {
                        ui.label(format!(
                            "Удалить {} «{}» с сервера?",
                            if folders == 1 {
                                "папку"
                            } else {
                                "файл"
                            },
                            remote::file_name(&items[0].0)
                        ));
                    } else {
                        ui.label(format!(
                            "Удалить с сервера выбранные объекты ({})?",
                            items.len()
                        ));
                    }
                    if folders > 0 {
                        ui.label(
                            RichText::new(
                                "Папки удаляются вместе со всем содержимым. Отменить это нельзя.",
                            )
                            .color(pal.warn),
                        );
                    }
                    (String::new(), None, "Удалить")
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
                if dialog_button(ui, pal, action, true) {
                    confirmed = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
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
                Some(Prompt::NewFolder(dir, name)) => {
                    if let Some(name) = valid(&name) {
                        if self.tree {
                            self.expanded.insert(dir.clone());
                        }
                        self.send(Cmd::Mkdir(remote::join(&dir, &name)), "Создание папки…");
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
                            },
                            "Создание файла…",
                        );
                    }
                }
                Some(Prompt::Rename { from, name }) => {
                    if let Some(name) = valid(&name) {
                        let to = remote::join(&remote::parent(&from), &name);
                        self.send(Cmd::Rename(from, to), "Переименование…");
                    }
                }
                Some(Prompt::Delete(items)) => {
                    for (path, dir) in items {
                        self.marked.remove(&path);
                        self.expanded.remove(&path);
                        self.send(Cmd::Delete { path, dir }, "Удаление…");
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
            ui.label(RichText::new("Профили подключений").strong());
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(180.0);
                    ScrollArea::vertical().max_height(270.0).show(ui, |ui| {
                        for (i, profile) in editor.drafts.iter().enumerate() {
                            let name = if profile.name.trim().is_empty() {
                                "без имени"
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
                        if ui.add(Button::new("Добавить").fill(pal.surface)).clicked() {
                            editor.drafts.push(Profile::new(format!(
                                "Профиль {}",
                                editor.drafts.len() + 1
                            )));
                            editor.index = editor.drafts.len() - 1;
                            editor.result = None;
                        }
                        if ui
                            .add_enabled(
                                !editor.drafts.is_empty(),
                                Button::new("Удалить").fill(pal.surface),
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
                        ui.label(RichText::new("Добавьте профиль.").color(pal.subtext));
                        return;
                    };
                    egui::Grid::new("tror_profile_form")
                        .num_columns(2)
                        .spacing([10.0, 8.0])
                        .show(ui, |ui| {
                            let wide = 320.0;
                            ui.label("Название");
                            ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                            ui.end_row();
                            ui.label("Протокол");
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
                            ui.label("Сервер");
                            ui.horizontal(|ui| {
                                ui.add(
                                    TextEdit::singleline(&mut p.host)
                                        .hint_text("example.org")
                                        .desired_width(wide - 70.0),
                                );
                                widgets::port_field(ui, &mut p.port, 60.0)
                                    .on_hover_text("Порт");
                            });
                            ui.end_row();
                            ui.label("Пользователь");
                            ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                            ui.end_row();
                            ui.label("Пароль");
                            ui.horizontal(|ui| {
                                ui.add(
                                    TextEdit::singleline(&mut p.password)
                                        .password(!editor.reveal)
                                        .desired_width(wide - 88.0),
                                );
                                let label = if editor.reveal { "Скрыть" } else { "Показать" };
                                if ui
                                    .add_sized([80.0, 22.0], Button::new(label))
                                    .clicked()
                                {
                                    editor.reveal = !editor.reveal;
                                }
                            });
                            ui.end_row();
                            if p.protocol == Protocol::Sftp {
                                ui.label("Файл ключа");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        TextEdit::singleline(&mut p.key_file)
                                            .hint_text("необязательно")
                                            .desired_width(wide - 78.0),
                                    );
                                    if ui.button("Обзор…").clicked()
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
                                            RichText::new(format!("ключ: {kind}"))
                                                .size(12.0)
                                                .color(pal.subtext),
                                        ),
                                        Err(e) => {
                                            ui.label(RichText::new(e).size(12.0).color(pal.warn))
                                        }
                                    };
                                    ui.end_row();
                                    ui.label("Парольная фраза");
                                    ui.add(
                                        TextEdit::singleline(&mut p.passphrase)
                                            .password(!editor.reveal)
                                            .hint_text("если ключ защищён")
                                            .desired_width(wide),
                                    );
                                    ui.end_row();
                                }
                            }
                            if p.protocol.tls() {
                                ui.label("Сертификат");
                                ui.checkbox(&mut p.insecure_tls, "не проверять (самоподписанный)");
                                ui.end_row();
                            }
                            ui.label("");
                            ui.vertical(|ui| {
                                ui.set_max_width(wide);
                                if editor.probe.is_some() {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new("проверка…").color(pal.subtext));
                                    });
                                } else if ui
                                    .add(Button::new("Проверить подключение").fill(pal.surface))
                                    .clicked()
                                {
                                    test = true;
                                }
                                if let Some(result) = &editor.result {
                                    let (text, color) = match result {
                                        Ok(text) => (format!("Работает: {text}"), Color32::from_rgb(0x3f, 0xc2, 0x6e)),
                                        Err(e) => (format!("Ошибка: {e}"), pal.warn),
                                    };
                                    ui.label(RichText::new(text).size(12.5).color(color));
                                }
                            });
                            ui.end_row();
                            ui.label("Начальная папка");
                            ui.add(
                                TextEdit::singleline(&mut p.directory)
                                    .hint_text("домашняя папка")
                                    .desired_width(wide),
                            );
                            ui.end_row();
                            ui.label("Адрес сайта");
                            ui.add(
                                TextEdit::singleline(&mut p.url)
                                    .hint_text("https://example.ru — для предпросмотра (F12)")
                                    .desired_width(wide),
                            );
                            ui.end_row();
                            if p.protocol == Protocol::Sftp {
                                ui.label("");
                                ui.label(RichText::new("Консоль по этому профилю").color(pal.subtext).size(12.0));
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
                            "Вход по паролю, по ключу (RSA, ECDSA, Ed25519) или через ssh-agent, если не заданы ни пароль, ни ключ. Если у ключа нет отдельной парольной фразы, используется поле «Пароль».",
                            pal.subtext,
                        ),
                        Protocol::Ftp => ("FTP передаёт пароль и файлы без шифрования.", pal.warn),
                        _ if p.insecure_tls => (
                            "Соединение шифруется, но подлинность сервера не проверяется — \
                             включайте только для своего сервера с самоподписанным сертификатом.",
                            pal.warn,
                        ),
                        Protocol::Ftps => (
                            "FTPS с явным TLS (AUTH TLS, обычно порт 21): шифруются и команды, и данные.",
                            pal.subtext,
                        ),
                        Protocol::FtpsImplicit => (
                            "Неявный FTPS (обычно порт 990): TLS включается сразу при соединении. Старый способ — выбирайте его, только если хостинг не поддерживает явный FTPS.",
                            pal.subtext,
                        ),
                    };
                    ui.label(RichText::new(text).color(color).size(12.0));
                });
            });
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "Профили хранятся в {location}. Пароли в файле скрыты от случайного взгляда, но не зашифрованы."
                ))
                .color(pal.subtext)
                .size(12.0),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Сохранить", true) {
                    save = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
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
        if save {
            if let Some(editor) = self.editor.take() {
                self.selected = editor.index;
                self.store.profiles = editor.drafts;
                if let Err(e) = self.store.save() {
                    self.busy = Some(format!("Профили не сохранены: {e}"));
                }
            }
        } else if cancel || modal.should_close() {
            self.editor = None;
        }
    }
}
