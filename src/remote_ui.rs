use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use eframe::egui::{
    self, Button, Color32, ComboBox, Key, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui,
};

use crate::app::{dialog, dialog_button, size_label};
use crate::remote::{self, Cmd, Connection, Entry, Event, Profile, Protocol, Store};
use crate::theme::Palette;

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
        path: String,
        data: Vec<u8>,
    },
}

enum Walk {
    Size,
    Download(PathBuf),
    Relay(String),
}

#[derive(Clone)]
pub struct Dragged {
    panel: u8,
    pub paths: Vec<(String, bool)>,
}

enum Prompt {
    NewFolder(String),
    NewFile(String),
    Rename { from: String, name: String },
    Delete(Vec<(String, bool)>),
}

struct ProfileEditor {
    drafts: Vec<Profile>,
    index: usize,
    reveal: bool,
}

struct Saver {
    conn: Connection,
    connected: bool,
    queue: Vec<Cmd>,
}

pub struct RemotePanel {
    pub visible: bool,
    pub id: u8,
    pub peer: Option<String>,
    pub outgoing: Option<Vec<(String, bool)>>,
    pub incoming: Option<Dragged>,
    relay: HashMap<String, String>,
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
    marked: BTreeSet<String>,
    anchor: Option<usize>,
    busy: Option<String>,
    prompt: Option<Prompt>,
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

fn date_label(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn kind_color(name: &str, pal: &Palette) -> Color32 {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "php" | "phtml" | "twig" | "tpl" => Color32::from_rgb(0x9d, 0x8c, 0xf2),
        "js" | "mjs" | "ts" | "json" => Color32::from_rgb(0xe5, 0xc0, 0x5a),
        "css" | "scss" | "less" => Color32::from_rgb(0x5a, 0xa9, 0xf2),
        "html" | "htm" | "xml" | "svg" => Color32::from_rgb(0xf0, 0x8a, 0x5d),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => Color32::from_rgb(0x6c, 0xc7, 0x8a),
        "zip" | "gz" | "tar" | "rar" | "7z" => Color32::from_rgb(0xc9, 0x8f, 0x6b),
        _ => pal.faint,
    }
}

fn paint_icon(painter: &egui::Painter, center: Pos2, is_dir: bool, name: &str, pal: &Palette) {
    if is_dir {
        let color = pal.accent.gamma_multiply(0.9);
        let body = Rect::from_center_size(center + egui::vec2(0.0, 1.0), egui::vec2(14.0, 10.0));
        let tab = Rect::from_min_size(body.left_top() - egui::vec2(0.0, 2.5), egui::vec2(6.5, 4.0));
        painter.rect_filled(tab, 1.5, color.gamma_multiply(0.75));
        painter.rect_filled(body, 2.0, color);
        painter.line_segment(
            [
                body.left_top() + egui::vec2(1.0, 2.5),
                body.right_top() + egui::vec2(-1.0, 2.5),
            ],
            Stroke::new(1.0, pal.mantle.gamma_multiply(0.35)),
        );
    } else {
        let color = kind_color(name, pal);
        let page = Rect::from_center_size(center, egui::vec2(10.5, 13.5));
        let fold = 3.5;
        let outline = vec![
            page.left_top(),
            page.right_top() - egui::vec2(fold, 0.0),
            page.right_top() + egui::vec2(0.0, fold),
            page.right_bottom(),
            page.left_bottom(),
        ];
        painter.add(egui::Shape::closed_line(outline, Stroke::new(1.1, color)));
        painter.line_segment(
            [
                page.right_top() - egui::vec2(fold, 0.0),
                page.right_top() + egui::vec2(-fold, fold),
            ],
            Stroke::new(1.0, color),
        );
        painter.line_segment(
            [
                page.right_top() + egui::vec2(-fold, fold),
                page.right_top() + egui::vec2(0.0, fold),
            ],
            Stroke::new(1.0, color),
        );
        for (i, w) in [5.5, 4.0].into_iter().enumerate() {
            let y = page.center().y + 1.0 + i as f32 * 3.0;
            painter.line_segment(
                [
                    egui::pos2(page.left() + 2.5, y),
                    egui::pos2(page.left() + 2.5 + w, y),
                ],
                Stroke::new(1.0, color.gamma_multiply(0.7)),
            );
        }
    }
}

fn refresh_button(ui: &mut Ui, pal: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, pal.surface);
    }
    let color = if response.hovered() {
        pal.text
    } else {
        pal.subtext
    };
    let c = rect.center();
    let r = 5.5;
    let points: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = -0.4 + i as f32 / 20.0 * 5.1;
            c + egui::vec2(a.cos() * r, a.sin() * r)
        })
        .collect();
    let tip = points[0];
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.5, color)));
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            tip + egui::vec2(-3.2, -3.6),
            tip + egui::vec2(2.6, -2.6),
            tip + egui::vec2(-0.6, 2.2),
        ],
        color,
        Stroke::NONE,
    ));
    response
}

impl RemotePanel {
    pub fn new(id: u8) -> Self {
        let path = eframe::storage_dir("Tror").map(|dir| dir.join("connections.json"));
        let store = Store::load(path);
        Self {
            visible: false,
            id,
            peer: None,
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
            marked: BTreeSet::new(),
            anchor: None,
            busy: None,
            prompt: None,
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
        self.send(Cmd::List(path), "Чтение каталога…");
    }

    fn disconnect(&mut self) {
        self.conn = None;
        self.connected = false;
        self.queue.clear();
        self.entries.clear();
        self.marked.clear();
        self.busy = None;
        self.pending = 0;
        self.progress = None;
        self.relay.clear();
        self.walks.clear();
        self.host_key = None;
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
                    let path = remote::join(&self.cwd, &relative);
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

    pub fn serves(&self, profile: &str) -> bool {
        self.conn.as_ref().is_some_and(|c| c.profile == profile)
    }

    pub fn relay(&mut self, items: Vec<(String, bool)>, target_dir: &str) {
        for (path, dir) in items {
            let name = remote::file_name(&path).to_owned();
            if dir {
                self.walks
                    .insert(path.clone(), Walk::Relay(target_dir.to_owned()));
                self.send(Cmd::Walk(path), format!("Чтение папки {name}…"));
                continue;
            }
            self.relay
                .insert(path.clone(), remote::join(target_dir, &name));
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
                    if path != self.cwd {
                        self.marked.clear();
                        self.anchor = None;
                    }
                    self.path_input = path.clone();
                    self.cwd = path;
                    self.marked
                        .retain(|name| entries.iter().any(|e| &e.name == name));
                    self.entries = entries;
                }
                Event::Downloaded { path, data } => {
                    self.finished();
                    if let Some(target) = self.relay.remove(&path) {
                        out.push(PanelEvent::Relay { path: target, data });
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
                    let refresh = remote::parent(&path) == self.cwd;
                    match doc {
                        Some(doc) => out.push(PanelEvent::Uploaded { doc, path }),
                        None => out.push(PanelEvent::Toast(format!(
                            "Загружено: {}",
                            remote::file_name(&path)
                        ))),
                    }
                    if refresh {
                        self.list(self.cwd.clone());
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
                                crate::app::size_label(bytes as usize)
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
                        Some(Walk::Relay(target)) => {
                            for (path, _) in files {
                                self.relay
                                    .insert(path.clone(), remote::join(&target, &relative(&path)));
                                self.send(Cmd::Download(path), format!("Передача папки {name}…"));
                            }
                        }
                    }
                }
                Event::Changed(dir) => {
                    self.finished();
                    if dir != self.cwd {
                        self.push_log(format!("Изменён каталог {dir}"), false);
                    }
                    self.list(self.cwd.clone());
                }
                Event::Error(e) => {
                    self.finished();
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
        });
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
        ui.horizontal(|ui| {
            let label = if active {
                "Отключить"
            } else {
                "Подключить"
            };
            let button_w = 96.0;
            ui.add_enabled_ui(!active, |ui| {
                ComboBox::from_id_salt("tror_profile")
                    .width((ui.available_width() - button_w - 30.0).max(60.0))
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
            if ui
                .add_sized([button_w, 24.0], Button::new(label).fill(pal.surface))
                .clicked()
            {
                if active {
                    self.disconnect();
                } else {
                    self.connect(&ctx);
                }
            }
        });
        if edit {
            self.open_editor();
        }

        if self.connected {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let field = ui.add(
                    TextEdit::singleline(&mut self.path_input)
                        .desired_width((ui.available_width() - 84.0).max(40.0))
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
                }
                ui.menu_button("Ещё", |ui| {
                    if ui.button("Поиск на сервере…").clicked() {
                        self.want_search = true;
                    }
                    ui.separator();
                    if ui.button("Новый файл…").clicked() {
                        self.prompt = Some(Prompt::NewFile(String::new()));
                    }
                    if ui.button("Новая папка…").clicked() {
                        self.prompt = Some(Prompt::NewFolder(String::new()));
                    }
                    if ui.button("Загрузить файлы с компьютера…").clicked()
                        && let Some(files) = rfd::FileDialog::new().pick_files()
                    {
                        self.upload_local(files);
                    }
                    ui.separator();
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
                    Some((done, _)) => format!("  {}", crate::app::size_label(done as usize)),
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
            ui.label(
                RichText::new("  Подключитесь к серверу в панели файлов (F8) — поиск идёт в открытой там папке.")
                    .color(pal.faint)
                    .size(12.5),
            );
            return;
        }
        let mut start = false;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(6.0);
            let field_w = (ui.available_width() - 20.0).clamp(120.0, 280.0);
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
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Не искать в папках:")
                    .color(pal.subtext)
                    .size(12.5),
            );
            ui.add(
                TextEdit::singleline(&mut self.search_excludes)
                    .desired_width(ui.available_width() - 12.0)
                    .font(egui::TextStyle::Small),
            );
        });
        if start && self.search_job.is_none() {
            self.start_search(&ctx);
        }
        let mut open = None;
        let row_h = 20.0;
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

    fn selection_paths(&self, clicked: &str) -> Vec<(String, bool)> {
        let names: Vec<&str> = if self.marked.contains(clicked) {
            self.marked.iter().map(String::as_str).collect()
        } else {
            vec![clicked]
        };
        self.entries
            .iter()
            .filter(|e| names.contains(&e.name.as_str()))
            .map(|e| (remote::join(&self.cwd, &e.name), e.is_dir))
            .collect()
    }

    fn listing(&mut self, ui: &mut Ui, pal: &Palette) {
        let row_h = 22.0;
        let has_parent = self.cwd != "/";
        let offset = has_parent as usize;
        let total = self.entries.len() + offset;
        let show_size = !self.store.hide_size;
        let show_date = self.store.show_date;
        let mut go = None;
        let mut download: Vec<String> = Vec::new();
        let mut moves: Vec<(String, String)> = Vec::new();
        let mut walk: Vec<String> = Vec::new();
        let reserve = if self.busy.is_some() { 34.0 } else { 0.0 };
        let area = ScrollArea::vertical()
            .auto_shrink(false)
            .max_height((ui.available_height() - reserve).max(40.0))
            .show_rows(ui, row_h, total, |ui, rows| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click_and_drag(),
                    );
                    let index = row.checked_sub(offset).filter(|_| row >= offset);
                    let entry = index.map(|i| self.entries[i].clone());
                    let name = entry.as_ref().map_or("..", |e| e.name.as_str()).to_owned();
                    let is_dir = entry.as_ref().is_none_or(|e| e.is_dir);
                    let marked = entry.is_some() && self.marked.contains(&name);
                    let path = match &entry {
                        Some(e) => remote::join(&self.cwd, &e.name),
                        None => remote::parent(&self.cwd),
                    };
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
                    paint_icon(
                        &painter,
                        egui::pos2(rect.left() + 14.0, rect.center().y),
                        is_dir,
                        &name,
                        pal,
                    );
                    let mut right = rect.right() - 6.0;
                    let small = egui::FontId::proportional(11.5);
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
                        right -= 100.0;
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
                        right -= 58.0;
                    }
                    let name_clip = Rect::from_min_max(
                        egui::pos2(rect.left() + 28.0, rect.top()),
                        egui::pos2(right.max(rect.left() + 60.0), rect.bottom()),
                    );
                    ui.painter_at(name_clip).text(
                        egui::pos2(name_clip.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        egui::FontId::proportional(13.0),
                        pal.text,
                    );

                    if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        match index {
                            Some(i) if modifiers.shift && self.anchor.is_some() => {
                                let a = self.anchor.unwrap_or(i).min(self.entries.len() - 1);
                                let (from, to) = (a.min(i), a.max(i));
                                if !modifiers.command {
                                    self.marked.clear();
                                }
                                for e in &self.entries[from..=to] {
                                    self.marked.insert(e.name.clone());
                                }
                            }
                            Some(i) if modifiers.command => {
                                if !self.marked.remove(&name) {
                                    self.marked.insert(name.clone());
                                }
                                self.anchor = Some(i);
                            }
                            Some(i) => {
                                self.marked.clear();
                                self.marked.insert(name.clone());
                                self.anchor = Some(i);
                            }
                            None => self.marked.clear(),
                        }
                    }
                    if response.double_clicked() {
                        if is_dir {
                            go = Some(path.clone());
                        } else {
                            download.push(path.clone());
                        }
                    }
                    if entry.is_some() && response.drag_started() {
                        response.dnd_set_drag_payload(Dragged {
                            panel: self.id,
                            paths: self.selection_paths(&name),
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
                            let chosen = self.selection_paths(&name);
                            let files: Vec<String> = chosen
                                .iter()
                                .filter(|(_, dir)| !dir)
                                .map(|(p, _)| p.clone())
                                .collect();
                            if !files.is_empty() && ui.button("Открыть").clicked() {
                                download.extend(files.iter().cloned());
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
                            let dirs: Vec<String> = chosen
                                .iter()
                                .filter(|(_, dir)| *dir)
                                .map(|(p, _)| p.clone())
                                .collect();
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
                            if let Some(peer) = &self.peer
                                && ui.button(format!("Передать на «{peer}»")).clicked()
                            {
                                self.outgoing = Some(chosen.clone());
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
            })
            .inner_rect;
        if let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx())
            && dragged.panel != self.id
            && ui.rect_contains_pointer(area)
        {
            ui.painter().rect_stroke(
                area.shrink(1.0),
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
        if let Some(path) = go {
            self.list(path);
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
            ui.horizontal(|ui| {
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

    fn prompt_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        let mut confirmed = false;
        let mut cancelled = false;
        let modal = dialog(("tror_remote_prompt", self.id), pal).show(ctx, |ui| {
            ui.set_width(340.0);
            let (title, input, action) = match prompt {
                Prompt::NewFolder(name) => ("Новая папка", Some(name), "Создать"),
                Prompt::NewFile(name) => ("Новый файл", Some(name), "Создать"),
                Prompt::Rename { name, .. } => ("Переименовать", Some(name), "Переименовать"),
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
                    ("", None, "Удалить")
                }
            };
            if let Some(input) = input {
                ui.label(title);
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
            let cwd = self.cwd.clone();
            let valid = |name: &str| {
                let name = name.trim();
                (!name.is_empty() && !name.contains('/')).then(|| name.to_owned())
            };
            match self.prompt.take() {
                Some(Prompt::NewFolder(name)) => {
                    if let Some(name) = valid(&name) {
                        self.send(Cmd::Mkdir(remote::join(&cwd, &name)), "Создание папки…");
                    }
                }
                Some(Prompt::NewFile(name)) => {
                    if let Some(name) = valid(&name) {
                        self.send(
                            Cmd::Upload {
                                path: remote::join(&cwd, &name),
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
        let location = self
            .store
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let modal = dialog(("tror_profiles", self.id), pal).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.label(RichText::new("Профили подключений").strong());
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(180.0);
                    ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                        for (i, profile) in editor.drafts.iter().enumerate() {
                            let name = if profile.name.trim().is_empty() {
                                "без имени"
                            } else {
                                profile.name.as_str()
                            };
                            if ui.selectable_label(i == editor.index, name).clicked() {
                                editor.index = i;
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
                            let wide = 300.0;
                            ui.label("Название");
                            ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                            ui.end_row();
                            ui.label("Протокол");
                            ui.horizontal(|ui| {
                                for protocol in [Protocol::Sftp, Protocol::Ftp, Protocol::Ftps] {
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
                                        .desired_width(wide - 78.0),
                                );
                                ui.add(egui::DragValue::new(&mut p.port).range(1..=65535));
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
                            }
                            if p.protocol == Protocol::Ftps {
                                ui.label("Сертификат");
                                ui.checkbox(&mut p.insecure_tls, "не проверять (самоподписанный)");
                                ui.end_row();
                            }
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
                        });
                    if p.protocol == Protocol::Sftp {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "С ключом поле «Пароль» используется как парольная фраза ключа. \
                                 Без пароля и ключа используется ssh-agent.",
                            )
                            .color(pal.subtext)
                            .size(12.0),
                        );
                    } else if p.protocol == Protocol::Ftp {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("FTP передаёт пароль и файлы без шифрования.")
                                .color(pal.warn)
                                .size(12.0),
                        );
                    } else {
                        ui.add_space(4.0);
                        let (text, color) = if p.insecure_tls {
                            (
                                "Соединение шифруется, но подлинность сервера не проверяется — \
                                 включайте только для своего сервера с самоподписанным сертификатом.",
                                pal.warn,
                            )
                        } else {
                            (
                                "FTPS с явным TLS (AUTH TLS, обычно порт 21): шифруются и команды, и данные.",
                                pal.subtext,
                            )
                        };
                        ui.label(RichText::new(text).color(color).size(12.0));
                    }
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
