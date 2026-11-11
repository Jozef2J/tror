use std::path::PathBuf;

use eframe::egui::{
    self, Align, Button, ComboBox, Key, Layout, RichText, ScrollArea, Sense, TextEdit, Ui,
};

use crate::app::{dialog, dialog_button, size_label};
use crate::remote::{self, Cmd, Connection, Entry, Event, Profile, Protocol, Store};
use crate::theme::Palette;

pub enum PanelEvent {
    Open {
        profile: String,
        path: String,
        data: Vec<u8>,
    },
    Uploaded {
        doc: (u64, u64),
        path: String,
    },
    Toast(String),
}

enum Prompt {
    NewFolder(String),
    NewFile(String),
    Rename { from: String, name: String },
    Delete { path: String, dir: bool },
}

struct ProfileEditor {
    drafts: Vec<Profile>,
    index: usize,
}

pub struct RemotePanel {
    pub visible: bool,
    store: Store,
    selected: usize,
    conn: Option<Connection>,
    connected: bool,
    cwd: String,
    path_input: String,
    entries: Vec<Entry>,
    marked: Option<String>,
    busy: Option<String>,
    prompt: Option<Prompt>,
    host_key: Option<(String, bool)>,
    editor: Option<ProfileEditor>,
    save_to: Option<(String, PathBuf)>,
}

impl RemotePanel {
    pub fn new(visible: bool) -> Self {
        let path = eframe::storage_dir("Tror").map(|dir| dir.join("connections.json"));
        Self {
            visible,
            store: Store::load(path),
            selected: 0,
            conn: None,
            connected: false,
            cwd: String::new(),
            path_input: String::new(),
            entries: Vec::new(),
            marked: None,
            busy: None,
            prompt: None,
            host_key: None,
            editor: None,
            save_to: None,
        }
    }

    pub fn modal_open(&self) -> bool {
        self.prompt.is_some() || self.host_key.is_some() || self.editor.is_some()
    }

    fn send(&mut self, cmd: Cmd, busy: impl Into<String>) {
        if let Some(conn) = &self.conn
            && conn.tx.send(cmd).is_ok()
        {
            self.busy = Some(busy.into());
        }
    }

    fn list(&mut self, path: String) {
        self.send(Cmd::List(path), "Чтение каталога…");
    }

    fn disconnect(&mut self) {
        self.conn = None;
        self.connected = false;
        self.entries.clear();
        self.busy = None;
        self.host_key = None;
    }

    fn connect(&mut self, ctx: &egui::Context) {
        let Some(profile) = self.store.profiles.get(self.selected) else {
            return;
        };
        let key = format!("{}:{}", profile.host.trim(), profile.port);
        let known = self.store.known_hosts.get(&key).cloned();
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
        profile: &str,
        path: &str,
        data: Vec<u8>,
        doc: (u64, u64),
    ) -> Result<(), String> {
        match &self.conn {
            Some(conn) if self.connected && conn.profile == profile => {
                self.send(
                    Cmd::Upload {
                        path: path.to_owned(),
                        data,
                        doc: Some(doc),
                    },
                    format!("Загрузка {}…", remote::file_name(path)),
                );
                Ok(())
            }
            _ => Err(format!(
                "нет соединения с «{profile}» — подключитесь и повторите"
            )),
        }
    }

    pub fn poll(&mut self) -> Vec<PanelEvent> {
        let mut out = Vec::new();
        let mut events = Vec::new();
        let mut dead = false;
        if let Some(conn) = &self.conn {
            loop {
                match conn.rx.try_recv() {
                    Ok(event) => events.push(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        dead = true;
                        break;
                    }
                }
            }
        }
        let profile = self
            .conn
            .as_ref()
            .map(|c| c.profile.clone())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::HostKey {
                    fingerprint,
                    changed,
                } => {
                    self.busy = None;
                    self.host_key = Some((fingerprint, changed));
                }
                Event::Connected { home, fingerprint } => {
                    self.connected = true;
                    if let (Some(print), Some(p)) =
                        (fingerprint, self.store.profiles.get(self.selected))
                    {
                        let key = format!("{}:{}", p.host.trim(), p.port);
                        self.store.known_hosts.insert(key, print);
                        if let Err(e) = self.store.save() {
                            out.push(PanelEvent::Toast(format!("Не удалось сохранить ключ: {e}")));
                        }
                    }
                    self.list(home);
                }
                Event::Listing { path, entries } => {
                    self.busy = None;
                    self.path_input = path.clone();
                    self.cwd = path;
                    self.entries = entries;
                    self.marked = None;
                }
                Event::Downloaded { path, data } => {
                    self.busy = None;
                    match self.save_to.take() {
                        Some((remote_path, local)) if remote_path == path => {
                            out.push(PanelEvent::Toast(match std::fs::write(&local, data) {
                                Ok(()) => format!("Скачано: {}", local.display()),
                                Err(e) => format!("Не удалось записать файл: {e}"),
                            }));
                        }
                        other => {
                            self.save_to = other;
                            out.push(PanelEvent::Open {
                                profile: profile.clone(),
                                path,
                                data,
                            });
                        }
                    }
                }
                Event::Uploaded { path, doc } => {
                    self.busy = None;
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
                Event::Changed(dir) => {
                    self.busy = None;
                    if dir == self.cwd {
                        self.list(dir);
                    }
                }
                Event::Error(e) => {
                    self.busy = None;
                    self.save_to = None;
                    out.push(PanelEvent::Toast(format!("Ошибка сервера: {e}")));
                }
                Event::Closed(e) => {
                    self.disconnect();
                    out.push(PanelEvent::Toast(format!("Соединение не установлено: {e}")));
                }
            }
        }
        if dead && self.conn.is_some() {
            let was_connected = self.connected;
            self.disconnect();
            if was_connected {
                out.push(PanelEvent::Toast("Соединение закрыто".into()));
            }
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
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Удалённые файлы")
                    .color(pal.subtext)
                    .size(12.5),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Профили").clicked() {
                    self.open_editor();
                }
            });
        });
        if self.store.profiles.is_empty() {
            ui.add_space(12.0);
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
        ui.horizontal(|ui| {
            let label = if active {
                "Отключить"
            } else {
                "Подключить"
            };
            let button_w = 96.0;
            ui.add_enabled_ui(!active, |ui| {
                ComboBox::from_id_salt("tror_profile")
                    .width((ui.available_width() - button_w - 8.0).max(60.0))
                    .selected_text(self.store.profiles[self.selected].name.clone())
                    .show_ui(ui, |ui| {
                        for (i, profile) in self.store.profiles.iter().enumerate() {
                            let text = format!("{}  ·  {}", profile.name, profile.protocol.label());
                            ui.selectable_value(&mut self.selected, i, text);
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

        if self.connected {
            ui.horizontal(|ui| {
                let field = ui.add(
                    TextEdit::singleline(&mut self.path_input)
                        .desired_width(ui.available_width() - 76.0)
                        .font(egui::TextStyle::Small),
                );
                if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.list(self.path_input.trim().to_owned());
                }
                ui.menu_button("Ещё", |ui| {
                    if ui.button("Обновить").clicked() {
                        self.list(self.cwd.clone());
                    }
                    if ui.button("Новый файл…").clicked() {
                        self.prompt = Some(Prompt::NewFile(String::new()));
                    }
                    if ui.button("Новая папка…").clicked() {
                        self.prompt = Some(Prompt::NewFolder(String::new()));
                    }
                    if ui.button("Загрузить файл с компьютера…").clicked()
                        && let Some(local) = rfd::FileDialog::new().pick_file()
                    {
                        match (std::fs::read(&local), local.file_name()) {
                            (Ok(data), Some(name)) => {
                                let path = remote::join(&self.cwd, &name.to_string_lossy());
                                self.send(
                                    Cmd::Upload {
                                        path,
                                        data,
                                        doc: None,
                                    },
                                    "Загрузка файла…",
                                );
                            }
                            _ => self.busy = None,
                        }
                    }
                });
            });
            self.listing(ui, pal);
        }
        if let Some(busy) = &self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(busy).color(pal.subtext).size(12.5));
            });
        }
    }

    fn listing(&mut self, ui: &mut Ui, pal: &Palette) {
        let row_h = 22.0;
        let has_parent = self.cwd != "/";
        let offset = has_parent as usize;
        let total = self.entries.len() + offset;
        let mut go = None;
        let mut download = None;
        let reserve = if self.busy.is_some() { 26.0 } else { 0.0 };
        ScrollArea::vertical()
            .auto_shrink(false)
            .max_height((ui.available_height() - reserve).max(40.0))
            .show_rows(ui, row_h, total, |ui, rows| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let entry = (row >= offset).then(|| self.entries[row - offset].clone());
                    let name = entry.as_ref().map_or("..", |e| e.name.as_str());
                    let is_dir = entry.as_ref().is_none_or(|e| e.is_dir);
                    let marked = entry.is_some() && self.marked.as_deref() == Some(name);
                    let painter = ui.painter_at(rect);
                    if marked {
                        painter.rect_filled(rect, 5.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 5.0, pal.crust);
                    }
                    let icon = egui::Rect::from_center_size(
                        egui::pos2(rect.left() + 13.0, rect.center().y),
                        egui::vec2(11.0, 9.0),
                    );
                    if is_dir {
                        painter.rect_filled(icon, 2.0, pal.accent.gamma_multiply(0.75));
                    } else {
                        painter.rect_stroke(
                            icon.shrink2(egui::vec2(1.5, 0.0)),
                            1.5,
                            egui::Stroke::new(1.0, pal.faint),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let size = entry
                        .as_ref()
                        .filter(|e| !e.is_dir)
                        .map(|e| size_label(e.size as usize));
                    let size_w = if size.is_some() { 62.0 } else { 0.0 };
                    if let Some(size) = size {
                        painter.text(
                            egui::pos2(rect.right() - 6.0, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            size,
                            egui::FontId::proportional(11.5),
                            pal.faint,
                        );
                    }
                    let name_clip = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 26.0, rect.top()),
                        egui::pos2(rect.right() - size_w, rect.bottom()),
                    );
                    ui.painter_at(name_clip).text(
                        egui::pos2(name_clip.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(13.0),
                        pal.text,
                    );
                    let path = match &entry {
                        Some(e) => remote::join(&self.cwd, &e.name),
                        None => remote::parent(&self.cwd),
                    };
                    if response.clicked() {
                        self.marked = entry.as_ref().map(|e| e.name.clone());
                    }
                    if response.double_clicked() {
                        if is_dir {
                            go = Some(path.clone());
                        } else {
                            download = Some(path.clone());
                        }
                    }
                    if let Some(e) = &entry {
                        response.context_menu(|ui| {
                            if !e.is_dir && ui.button("Открыть").clicked() {
                                download = Some(path.clone());
                            }
                            if !e.is_dir
                                && ui.button("Скачать как…").clicked()
                                && let Some(local) =
                                    rfd::FileDialog::new().set_file_name(&e.name).save_file()
                            {
                                self.save_to = Some((path.clone(), local));
                                download = Some(path.clone());
                            }
                            if ui.button("Переименовать…").clicked() {
                                self.prompt = Some(Prompt::Rename {
                                    from: path.clone(),
                                    name: e.name.clone(),
                                });
                            }
                            if ui.button("Удалить…").clicked() {
                                self.prompt = Some(Prompt::Delete {
                                    path: path.clone(),
                                    dir: e.is_dir,
                                });
                            }
                        });
                    }
                }
            });
        if let Some(path) = go {
            self.list(path);
        }
        if let Some(path) = download {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Download(path), format!("Скачивание {name}…"));
        }
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        self.host_key_dialog(ctx, pal);
        self.prompt_dialog(ctx, pal);
        self.editor_dialog(ctx, pal);
    }

    fn host_key_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((fingerprint, changed)) = self.host_key.clone() else {
            return;
        };
        let mut answer = None;
        dialog("tror_host_key", pal).show(ctx, |ui| {
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
                self.send(Cmd::TrustHost(true), "Вход…");
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
        let modal = dialog("tror_remote_prompt", pal).show(ctx, |ui| {
            ui.set_width(320.0);
            let (title, input, action) = match prompt {
                Prompt::NewFolder(name) => ("Новая папка", Some(name), "Создать"),
                Prompt::NewFile(name) => ("Новый файл", Some(name), "Создать"),
                Prompt::Rename { name, .. } => ("Переименовать", Some(name), "Переименовать"),
                Prompt::Delete { path, dir } => {
                    ui.label(format!(
                        "Удалить {} «{}» с сервера?",
                        if *dir { "папку" } else { "файл" },
                        remote::file_name(path)
                    ));
                    if *dir {
                        ui.label(
                            RichText::new("Удаляются только пустые папки.").color(pal.subtext),
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
                Some(Prompt::Delete { path, dir }) => {
                    self.send(Cmd::Delete { path, dir }, "Удаление…");
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
        let modal = dialog("tror_profiles", pal).show(ctx, |ui| {
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
                                for protocol in [Protocol::Sftp, Protocol::Ftp] {
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
                            ui.add(
                                TextEdit::singleline(&mut p.password)
                                    .password(true)
                                    .desired_width(wide),
                            );
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
                            ui.label("Начальная папка");
                            ui.add(
                                TextEdit::singleline(&mut p.directory)
                                    .hint_text("домашняя папка")
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
                    } else {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("FTP передаёт пароль и файлы без шифрования.")
                                .color(pal.warn)
                                .size(12.0),
                        );
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
