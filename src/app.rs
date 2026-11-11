use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Button, CentralPanel, Color32, CornerRadius, FontId, Frame, Key, KeyboardShortcut,
    Layout, Margin, MenuBar, Modal, Modifiers, Panel, PopupCloseBehavior, Rect, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Ui, ViewportCommand,
    containers::menu::{MenuButton, MenuConfig},
};
use encoding_rs::Encoding;
use serde::{Deserialize, Serialize};

use crate::complete::Completion;
use crate::document::{Document, Eol};
use crate::editor;
use crate::highlight::Syntaxes;
use crate::remote_ui::{PanelEvent, RemotePanel};
use crate::search::{Pending, SearchState};
use crate::terminal::{TermEvent, Terminal};
use crate::theme::{self, Palette};

const SETTINGS_KEY: &str = "tror";
const MAX_RECENT: usize = 12;

const ENCODINGS: &[(&str, &Encoding, bool)] = &[
    ("UTF-8", encoding_rs::UTF_8, false),
    ("UTF-8 BOM", encoding_rs::UTF_8, true),
    ("Windows-1251", encoding_rs::WINDOWS_1251, false),
    ("KOI8-R", encoding_rs::KOI8_R, false),
    ("CP866", encoding_rs::IBM866, false),
    ("Windows-1252", encoding_rs::WINDOWS_1252, false),
    ("Windows-1250", encoding_rs::WINDOWS_1250, false),
    ("UTF-16 LE", encoding_rs::UTF_16LE, true),
    ("UTF-16 BE", encoding_rs::UTF_16BE, true),
];

fn encoding_label(encoding: &'static Encoding, bom: bool) -> String {
    ENCODINGS
        .iter()
        .find(|(_, e, b)| *e == encoding && (*b == bom || *e != encoding_rs::UTF_8))
        .map(|(name, _, _)| (*name).to_owned())
        .unwrap_or_else(|| encoding.name().to_owned())
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    dark: bool,
    wrap: bool,
    remote_panel: bool,
    terminal_panel: bool,
    recent: Vec<PathBuf>,
    session: Vec<PathBuf>,
    active: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            wrap: false,
            remote_panel: false,
            terminal_panel: false,
            recent: Vec::new(),
            session: Vec::new(),
            active: 0,
        }
    }
}

enum Action {
    New,
    Open,
    OpenPath(PathBuf),
    Save,
    SaveAs,
    SaveAll,
    Activate(usize),
    CloseTab(usize),
    CloseAll,
    Quit,
    Find,
    Replace,
    FindNext,
    FindPrev,
    CloseSearch,
    GotoLine,
    ToggleTheme,
    ToggleWrap,
    ToggleRemote,
    ToggleTerminal,
    NextTab,
    PrevTab,
}

pub(crate) fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} Б")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} КБ", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МБ", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub(crate) fn dialog_button(ui: &mut Ui, pal: &Palette, text: &str, primary: bool) -> bool {
    let (fill, color) = if primary {
        (pal.accent, pal.base)
    } else {
        (pal.surface, pal.text)
    };
    ui.add(
        Button::new(RichText::new(text).color(color))
            .fill(fill)
            .min_size(egui::vec2(0.0, 28.0)),
    )
    .clicked()
}

pub(crate) fn dialog(id: &'static str, pal: &Palette) -> Modal {
    Modal::new(egui::Id::unique(id)).frame(
        Frame::new()
            .fill(pal.mantle)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(12)
            .inner_margin(Margin::same(18)),
    )
}

#[derive(Clone, Copy)]
enum Icon {
    Up,
    Down,
    Close,
}

fn icon_button(ui: &mut Ui, pal: &Palette, icon: Icon) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal.surface);
    }
    let color = if response.hovered() {
        pal.text
    } else {
        pal.subtext
    };
    let stroke = Stroke::new(1.5, color);
    let c = rect.center();
    let d = 4.0;
    let lines = match icon {
        Icon::Up => [
            [c + egui::vec2(-d, 2.0), c + egui::vec2(0.0, -2.5)],
            [c + egui::vec2(0.0, -2.5), c + egui::vec2(d, 2.0)],
        ],
        Icon::Down => [
            [c + egui::vec2(-d, -2.0), c + egui::vec2(0.0, 2.5)],
            [c + egui::vec2(0.0, 2.5), c + egui::vec2(d, -2.0)],
        ],
        Icon::Close => [
            [c + egui::vec2(-d, -d), c + egui::vec2(d, d)],
            [c + egui::vec2(d, -d), c + egui::vec2(-d, d)],
        ],
    };
    for line in lines {
        ui.painter().line_segment(line, stroke);
    }
    response
}

#[derive(Clone, Copy)]
enum Confirm {
    Tab(u64),
    Quit,
    Missing(u64),
}

#[derive(Clone, Copy, PartialEq)]
enum BottomTab {
    Console,
    Log,
    Search,
}

#[derive(Default)]
struct Perf {
    frames: u32,
    total: std::time::Duration,
    worst: std::time::Duration,
    since: Option<std::time::Instant>,
}

impl Perf {
    fn record(&mut self, frame: std::time::Duration) {
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        self.frames += 1;
        self.total += frame;
        self.worst = self.worst.max(frame);
        if since.elapsed().as_secs() >= 1 {
            eprintln!(
                "кадров: {}, среднее: {:.2} мс, худший: {:.2} мс",
                self.frames,
                self.total.as_secs_f64() * 1000.0 / self.frames as f64,
                self.worst.as_secs_f64() * 1000.0
            );
            *self = Self::default();
        }
    }
}

pub struct TrorApp {
    perf: Option<Perf>,
    title: String,
    syntaxes: Syntaxes,
    docs: Vec<Document>,
    active: usize,
    next_id: u64,
    next_untitled: u32,
    settings: Settings,
    search: SearchState,
    complete: Completion,
    remote: RemotePanel,
    terminal: Terminal,
    bottom_tab: BottomTab,
    logo: Option<egui::TextureHandle>,
    ctx: egui::Context,
    checked: (u64, bool),
    goto: Option<String>,
    confirm: Option<Confirm>,
    allow_close: bool,
    focus_editor: bool,
    lang_filter: String,
    toast: Option<(String, f64)>,
    applied_theme: Option<bool>,
}

impl TrorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();
        let mut app = Self {
            perf: std::env::var_os("TROR_PERF").map(|_| Perf::default()),
            title: String::new(),
            syntaxes: Syntaxes::new(),
            docs: Vec::new(),
            active: 0,
            next_id: 1,
            next_untitled: 1,
            settings,
            search: SearchState::default(),
            complete: Completion::default(),
            remote: RemotePanel::new(false),
            terminal: Terminal::new(false),
            bottom_tab: BottomTab::Console,
            logo: eframe::icon_data::from_png_bytes(include_bytes!("../assets/tror.png"))
                .ok()
                .map(|icon| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [icon.width as usize, icon.height as usize],
                        &icon.rgba,
                    );
                    cc.egui_ctx
                        .load_texture("tror_logo", image, egui::TextureOptions::LINEAR)
                }),
            ctx: cc.egui_ctx.clone(),
            checked: (0, false),
            goto: None,
            confirm: None,
            allow_close: false,
            focus_editor: true,
            lang_filter: String::new(),
            toast: None,
            applied_theme: None,
        };
        app.remote.visible = app.settings.remote_panel;
        app.terminal.visible = app.settings.terminal_panel;
        let session = std::mem::take(&mut app.settings.session);
        let active = app.settings.active;
        for path in session.iter().filter(|p| p.is_file()) {
            app.open_path(path, 0.0);
        }
        if active < app.docs.len() {
            app.active = active;
        }
        for arg in std::env::args_os().skip(1) {
            let path = PathBuf::from(arg);
            let path = std::path::absolute(&path).unwrap_or(path);
            app.open_path(&path, 0.0);
        }
        app
    }

    fn palette(&self) -> Palette {
        theme::palette(self.settings.dark)
    }

    fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some((text.into(), now));
    }

    fn new_doc(&mut self) {
        let doc = Document::empty(self.next_id, self.next_untitled);
        self.next_id += 1;
        self.next_untitled += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
        self.focus_editor = true;
    }

    fn open_path(&mut self, path: &Path, now: f64) {
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(path))
        {
            self.active = i;
            self.focus_editor = true;
            return;
        }
        let started = std::time::Instant::now();
        match Document::open(self.next_id, path) {
            Ok(mut doc) => {
                self.next_id += 1;
                doc.syntax = self.syntaxes.detect(Some(path), &doc.first_line());
                if self.perf.is_some() {
                    eprintln!(
                        "открыт {} ({}) за {:.0} мс",
                        path.display(),
                        size_label(doc.rope.len_bytes()),
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                let replace_blank = self.docs.len() == 1
                    && self.docs[0].path.is_none()
                    && !self.docs[0].dirty()
                    && self.docs[0].len() == 0;
                if replace_blank {
                    self.docs.clear();
                }
                self.docs.push(doc);
                self.active = self.docs.len() - 1;
                self.focus_editor = true;
                self.remember(path);
            }
            Err(e) => self.notify(format!("Не удалось открыть {}: {e}", path.display()), now),
        }
    }

    fn remote_events(&mut self, ctx: &egui::Context, events: Vec<PanelEvent>) {
        let now = ctx.input(|i| i.time);
        for event in events {
            match event {
                PanelEvent::Toast(text) => self.notify(text, now),
                PanelEvent::OpenLocal(paths) => {
                    for path in paths {
                        self.open_path(&path, now);
                    }
                }
                PanelEvent::Listed {
                    profile,
                    dir,
                    names,
                } => {
                    for doc in &mut self.docs {
                        if let Some((p, path)) = &doc.remote
                            && *p == profile
                            && crate::remote::parent(path) == dir
                        {
                            let exists = names.iter().any(|n| n == crate::remote::file_name(path));
                            if exists {
                                doc.missing = false;
                                doc.missing_ack = false;
                            } else {
                                doc.missing = true;
                            }
                        }
                    }
                }
                PanelEvent::Uploaded { doc, path } => {
                    if let Some(d) = self.docs.iter_mut().find(|d| d.id == doc.0) {
                        d.mark_saved(doc.1);
                    }
                    self.notify(format!("Сохранено на сервере: {path}"), now);
                }
                PanelEvent::Open {
                    profile,
                    path,
                    data,
                    line,
                } => {
                    let target = Some((profile.clone(), path.clone()));
                    if let Some(i) = self.docs.iter().position(|d| d.remote == target) {
                        self.active = i;
                        self.focus_editor = true;
                        match line {
                            Some(line) => self.docs[i].goto_line(line),
                            None => self.notify("Файл уже открыт — показана открытая вкладка", now),
                        }
                        continue;
                    }
                    match Document::open_remote(self.next_id, &profile, &path, data) {
                        Ok(mut doc) => {
                            self.next_id += 1;
                            doc.syntax = self
                                .syntaxes
                                .detect(Some(Path::new(&path)), &doc.first_line());
                            if let Some(line) = line {
                                doc.goto_line(line);
                            }
                            self.docs.push(doc);
                            self.active = self.docs.len() - 1;
                            self.focus_editor = true;
                        }
                        Err(e) => self.notify(format!("Не удалось открыть {path}: {e}"), now),
                    }
                }
            }
        }
    }

    fn check_missing(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        if self.checked != (doc.id, focused) {
            self.checked = (doc.id, focused);
            if focused && let Some(path) = &doc.path {
                if path.exists() {
                    doc.missing = false;
                    doc.missing_ack = false;
                } else {
                    doc.missing = true;
                }
            }
        }
        if doc.missing
            && !doc.missing_ack
            && self.confirm.is_none()
            && self.goto.is_none()
            && !self.remote.modal_open()
        {
            self.confirm = Some(Confirm::Missing(doc.id));
        }
    }

    fn remember(&mut self, path: &Path) {
        self.settings.recent.retain(|p| p != path);
        self.settings.recent.insert(0, path.to_path_buf());
        self.settings.recent.truncate(MAX_RECENT);
    }

    fn open_dialog(&mut self, now: f64) {
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = self
            .docs
            .get(self.active)
            .and_then(|d| d.path.as_deref())
            .and_then(Path::parent)
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(paths) = dialog.pick_files() {
            for path in paths {
                self.open_path(&path, now);
            }
        }
    }

    fn save(&mut self, index: usize, now: f64) -> bool {
        let Some(doc) = self.docs.get_mut(index) else {
            return false;
        };
        if let Some((profile, path)) = doc.remote.clone() {
            let ctx = self.ctx.clone();
            let result =
                self.remote
                    .upload(&ctx, &profile, &path, doc.encoded(), (doc.id, doc.version));
            match result {
                Ok(text) => self.notify(text, now),
                Err(e) => self.notify(format!("Не сохранено: {e}"), now),
            }
            return false;
        }
        if doc.path.is_none() {
            return self.save_as(index, now);
        }
        match doc.save() {
            Ok(()) => {
                let title = doc.title();
                self.notify(format!("Сохранено: {title}"), now);
                true
            }
            Err(e) => {
                self.notify(format!("Ошибка сохранения: {e}"), now);
                false
            }
        }
    }

    fn save_as(&mut self, index: usize, now: f64) -> bool {
        let Some(doc) = self.docs.get(index) else {
            return false;
        };
        let mut dialog = rfd::FileDialog::new();
        match &doc.path {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    dialog = dialog.set_directory(dir);
                }
                dialog = dialog.set_file_name(doc.title());
            }
            None => dialog = dialog.set_file_name(format!("{}.txt", doc.title())),
        }
        let Some(path) = dialog.save_file() else {
            return false;
        };
        let doc = &mut self.docs[index];
        let previous = doc.path.replace(path.clone());
        match doc.save() {
            Ok(()) => {
                doc.syntax = self.syntaxes.detect(Some(&path), &doc.first_line());
                self.remember(&path);
                self.notify(format!("Сохранено: {}", path.display()), now);
                true
            }
            Err(e) => {
                self.docs[index].path = previous;
                self.notify(format!("Ошибка сохранения: {e}"), now);
                false
            }
        }
    }

    fn save_all(&mut self, now: f64) -> bool {
        for i in 0..self.docs.len() {
            if self.docs[i].dirty() && !self.save(i, now) {
                return false;
            }
        }
        true
    }

    fn remove_doc(&mut self, index: usize) {
        if index >= self.docs.len() {
            return;
        }
        self.docs.remove(index);
        if self.active > index || self.active >= self.docs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.focus_editor = true;
    }

    fn request_close(&mut self, index: usize) {
        match self.docs.get(index) {
            Some(doc) if doc.dirty() => {
                self.active = index;
                self.confirm = Some(Confirm::Tab(doc.id));
            }
            Some(_) => self.remove_doc(index),
            None => {}
        }
    }

    fn open_search(&mut self, replace: bool) {
        self.search.open = true;
        self.search.replace_mode = replace;
        self.search.focus_query = true;
        self.search.last_replaced = None;
        if let Some(doc) = self.docs.get(self.active) {
            let range = doc.selection();
            if !range.is_empty() && range.len() < 200 {
                let selected = doc.selected_text();
                if !selected.contains('\n') {
                    self.search.query = selected;
                }
            }
        }
    }

    fn run(&mut self, action: Action, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.complete.close();
        match action {
            Action::New => self.new_doc(),
            Action::Open => self.open_dialog(now),
            Action::OpenPath(path) => self.open_path(&path, now),
            Action::Save => {
                self.save(self.active, now);
            }
            Action::SaveAs => {
                self.save_as(self.active, now);
            }
            Action::SaveAll => {
                self.save_all(now);
            }
            Action::Activate(i) => {
                if i < self.docs.len() {
                    self.active = i;
                    self.focus_editor = true;
                }
            }
            Action::CloseTab(i) => self.request_close(i),
            Action::CloseAll => {
                if self.docs.iter().any(|d| d.dirty()) {
                    self.notify(
                        "Есть несохранённые вкладки — закройте их по отдельности",
                        now,
                    );
                    self.docs.retain(|d| d.dirty());
                    self.active = 0;
                } else {
                    self.docs.clear();
                    self.active = 0;
                }
            }
            Action::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Action::Find => self.open_search(false),
            Action::Replace => self.open_search(true),
            Action::FindNext => {
                if self.search.query.is_empty() {
                    self.open_search(false);
                } else {
                    self.search.open = true;
                    self.search.pending = Some(Pending::Next);
                }
            }
            Action::FindPrev => {
                if !self.search.query.is_empty() {
                    self.search.open = true;
                    self.search.pending = Some(Pending::Prev);
                }
            }
            Action::CloseSearch => {
                self.search.open = false;
                self.focus_editor = true;
            }
            Action::GotoLine => {
                if !self.docs.is_empty() {
                    self.goto = Some(String::new());
                }
            }
            Action::ToggleTheme => self.settings.dark = !self.settings.dark,
            Action::ToggleWrap => self.settings.wrap = !self.settings.wrap,
            Action::ToggleRemote => self.remote.visible = !self.remote.visible,
            Action::ToggleTerminal => self.terminal.visible = !self.terminal.visible,
            Action::NextTab => {
                if !self.docs.is_empty() {
                    self.active = (self.active + 1) % self.docs.len();
                    self.focus_editor = true;
                }
            }
            Action::PrevTab => {
                if !self.docs.is_empty() {
                    self.active = (self.active + self.docs.len() - 1) % self.docs.len();
                    self.focus_editor = true;
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let table = [
            (cmd_shift, Key::S, 0),
            (cmd_shift, Key::Tab, 1),
            (cmd, Key::N, 2),
            (cmd, Key::O, 3),
            (cmd, Key::S, 4),
            (cmd, Key::W, 5),
            (cmd, Key::F, 6),
            (cmd, Key::H, 7),
            (cmd, Key::G, 8),
            (Modifiers::NONE, Key::F8, 12),
            (Modifiers::NONE, Key::F9, 13),
            (Modifiers::ALT, Key::Z, 14),
            (cmd, Key::Tab, 9),
            (Modifiers::SHIFT, Key::F3, 10),
            (Modifiers::NONE, Key::F3, 11),
        ];
        for (modifiers, key, code) in table {
            if self.terminal.focused && code < 12 {
                continue;
            }
            if !ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key))) {
                continue;
            }
            actions.push(match code {
                0 => Action::SaveAs,
                1 => Action::PrevTab,
                2 => Action::New,
                3 => Action::Open,
                4 => Action::Save,
                5 => Action::CloseTab(self.active),
                6 => Action::Find,
                7 => Action::Replace,
                8 => Action::GotoLine,
                12 => Action::ToggleRemote,
                13 => Action::ToggleTerminal,
                14 => {
                    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
                    Action::ToggleWrap
                }
                9 => Action::NextTab,
                10 => Action::FindPrev,
                _ => Action::FindNext,
            });
        }
        if self.search.open
            && self.goto.is_none()
            && self.confirm.is_none()
            && !self.complete.open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            actions.push(Action::CloseSearch);
        }
    }

    fn menu(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let has_doc = !self.docs.is_empty();
        let item = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool| {
            ui.add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
                .clicked()
        };
        let key = |s: &str| {
            if cfg!(target_os = "macos") {
                s.replace("Ctrl", "⌘")
            } else {
                s.to_owned()
            }
        };
        MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Файл", |ui| {
                ui.set_min_width(240.0);
                if item(ui, "Новый", &key("Ctrl+N"), true) {
                    actions.push(Action::New);
                }
                if item(ui, "Открыть…", &key("Ctrl+O"), true) {
                    actions.push(Action::Open);
                }
                ui.menu_button("Недавние", |ui| {
                    if self.settings.recent.is_empty() {
                        ui.weak("пусто");
                    }
                    for path in &self.settings.recent {
                        if ui.button(path.display().to_string()).clicked() {
                            actions.push(Action::OpenPath(path.clone()));
                        }
                    }
                });
                ui.separator();
                if item(ui, "Сохранить", &key("Ctrl+S"), has_doc) {
                    actions.push(Action::Save);
                }
                if item(ui, "Сохранить как…", &key("Ctrl+Shift+S"), has_doc) {
                    actions.push(Action::SaveAs);
                }
                if item(ui, "Сохранить все", "", has_doc) {
                    actions.push(Action::SaveAll);
                }
                ui.separator();
                if item(ui, "Закрыть вкладку", &key("Ctrl+W"), has_doc) {
                    actions.push(Action::CloseTab(self.active));
                }
                if item(ui, "Закрыть все", "", has_doc) {
                    actions.push(Action::CloseAll);
                }
                ui.separator();
                if item(ui, "Выход", "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button("Поиск", |ui| {
                ui.set_min_width(240.0);
                if item(ui, "Найти…", &key("Ctrl+F"), has_doc) {
                    actions.push(Action::Find);
                }
                if item(ui, "Заменить…", &key("Ctrl+H"), has_doc) {
                    actions.push(Action::Replace);
                }
                if item(ui, "Найти далее", "F3", has_doc) {
                    actions.push(Action::FindNext);
                }
                if item(ui, "Найти ранее", "Shift+F3", has_doc) {
                    actions.push(Action::FindPrev);
                }
                ui.separator();
                if item(ui, "Перейти к строке…", &key("Ctrl+G"), has_doc) {
                    actions.push(Action::GotoLine);
                }
            });
            ui.menu_button("Вид", |ui| {
                ui.set_min_width(240.0);
                let theme = if self.settings.dark {
                    "Светлая тема"
                } else {
                    "Тёмная тема"
                };
                if item(ui, theme, "", true) {
                    actions.push(Action::ToggleTheme);
                }
                ui.separator();
                let wrap = if self.settings.wrap {
                    "Выключить перенос строк"
                } else {
                    "Перенос строк"
                };
                if item(ui, wrap, "Alt+Z", true) {
                    actions.push(Action::ToggleWrap);
                }
                let panel = if self.remote.visible {
                    "Скрыть удалённые файлы"
                } else {
                    "Удалённые файлы (SFTP/FTP)"
                };
                if item(ui, panel, "F8", true) {
                    actions.push(Action::ToggleRemote);
                }
                let console = if self.terminal.visible {
                    "Скрыть SSH-консоль"
                } else {
                    "SSH-консоль"
                };
                if item(ui, console, "F9", true) {
                    actions.push(Action::ToggleTerminal);
                }
                ui.separator();
                ui.weak(key("Масштаб: Ctrl + / Ctrl − / Ctrl 0"));
            });
        });
    }

    fn tabs(&mut self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let height = 34.0;
        ScrollArea::horizontal()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, doc) in self.docs.iter().enumerate() {
                        let active = i == self.active;
                        let color = if doc.missing {
                            pal.warn
                        } else if active {
                            pal.text
                        } else {
                            pal.subtext
                        };
                        let galley = ui.painter().layout_no_wrap(
                            doc.title(),
                            FontId::proportional(13.5),
                            color,
                        );
                        let size = egui::vec2(galley.size().x + 50.0, height);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        let close_rect = Rect::from_center_size(
                            egui::pos2(rect.right() - 17.0, rect.center().y),
                            egui::vec2(18.0, 18.0),
                        );
                        let close = ui.interact(
                            close_rect,
                            ui.scope_id().with(("tab_close", doc.id)),
                            Sense::click(),
                        );
                        let hovered = response.hovered() || close.hovered();
                        let painter = ui.painter();
                        if active || hovered {
                            painter.rect_filled(
                                rect.shrink2(egui::vec2(0.0, 3.0))
                                    .translate(egui::vec2(0.0, 3.0)),
                                CornerRadius {
                                    nw: 8,
                                    ne: 8,
                                    sw: 0,
                                    se: 0,
                                },
                                if active { pal.base } else { pal.crust },
                            );
                        }
                        if active {
                            painter.line_segment(
                                [
                                    egui::pos2(rect.left() + 10.0, rect.top() + 4.0),
                                    egui::pos2(rect.right() - 10.0, rect.top() + 4.0),
                                ],
                                Stroke::new(2.0, pal.accent),
                            );
                        }
                        painter.galley(
                            egui::pos2(
                                rect.left() + 14.0,
                                rect.center().y - galley.size().y / 2.0 + 2.0,
                            ),
                            galley.clone(),
                            color,
                        );
                        if doc.missing {
                            let y = rect.center().y + 2.5;
                            painter.line_segment(
                                [
                                    egui::pos2(rect.left() + 14.0, y),
                                    egui::pos2(rect.left() + 14.0 + galley.size().x, y),
                                ],
                                Stroke::new(1.0, pal.warn),
                            );
                        }
                        let center = close_rect.center() + egui::vec2(0.0, 2.0);
                        if close.hovered() {
                            painter.rect_filled(
                                Rect::from_center_size(center, egui::vec2(18.0, 18.0)),
                                5.0,
                                pal.surface,
                            );
                        }
                        if doc.dirty() && !close.hovered() {
                            painter.circle_filled(center, 3.5, pal.accent);
                        } else if hovered || active {
                            let stroke = Stroke::new(
                                1.3,
                                if close.hovered() { pal.text } else { pal.faint },
                            );
                            let d = 3.5;
                            painter.line_segment(
                                [center + egui::vec2(-d, -d), center + egui::vec2(d, d)],
                                stroke,
                            );
                            painter.line_segment(
                                [center + egui::vec2(d, -d), center + egui::vec2(-d, d)],
                                stroke,
                            );
                        }
                        if close.clicked() || response.middle_clicked() {
                            actions.push(Action::CloseTab(i));
                        } else if response.clicked() {
                            actions.push(Action::Activate(i));
                        }
                        if let Some(location) = doc.location() {
                            response.on_hover_text(location);
                        }
                    }
                    let plus = ui.add_sized(
                        [28.0, 26.0],
                        Button::new(RichText::new("+").size(16.0).color(pal.subtext)),
                    );
                    if plus.on_hover_text("Новая вкладка").clicked() {
                        actions.push(Action::New);
                    }
                });
            });
    }

    fn search_bar(&mut self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let selection = self.docs.get(self.active).map(|d| {
            let range = d.selection();
            (range.start, range.end)
        });
        let s = &mut self.search;
        let toggle = |ui: &mut Ui, value: &mut bool, label: &str, hint: &str| {
            let text = RichText::new(label).monospace().color(if *value {
                pal.accent
            } else {
                pal.subtext
            });
            let mut button = Button::new(text).min_size(egui::vec2(28.0, 24.0));
            if *value {
                button = button.fill(pal.surface);
            }
            let clicked = ui.add(button).on_hover_text(hint).clicked();
            if clicked {
                *value = !*value;
            }
            clicked
        };
        ui.horizontal(|ui| {
            let query = ui.add(
                TextEdit::singleline(&mut s.query)
                    .hint_text("Найти")
                    .desired_width(260.0)
                    .margin(Margin::symmetric(8, 5)),
            );
            if std::mem::take(&mut s.focus_query) {
                query.request_focus();
            }
            let mut changed = query.changed();
            if query.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                s.pending = Some(if ui.input(|i| i.modifiers.shift) {
                    Pending::Prev
                } else {
                    Pending::Next
                });
                query.request_focus();
            }
            changed |= toggle(ui, &mut s.case, "Aa", "Учитывать регистр");
            changed |= toggle(ui, &mut s.word, "ab", "Слово целиком");
            changed |= toggle(ui, &mut s.regex, ".*", "Регулярное выражение");
            if changed {
                s.pending = Some(Pending::FromAnchor);
                s.last_replaced = None;
            }
            if icon_button(ui, pal, Icon::Up)
                .on_hover_text("Найти ранее (Shift+F3)")
                .clicked()
            {
                s.pending = Some(Pending::Prev);
            }
            if icon_button(ui, pal, Icon::Down)
                .on_hover_text("Найти далее (F3)")
                .clicked()
            {
                s.pending = Some(Pending::Next);
            }
            let info = if let Some(error) = &s.error {
                RichText::new(error).color(pal.warn)
            } else if let Some(n) = s.last_replaced {
                RichText::new(format!("заменено: {n}")).color(pal.subtext)
            } else if s.query.is_empty() {
                RichText::new("")
            } else if s.searching() {
                RichText::new("поиск…").color(pal.subtext)
            } else if s.matches.is_empty() {
                RichText::new("нет совпадений").color(pal.warn)
            } else {
                let total = if s.truncated {
                    format!("{}+", s.matches.len())
                } else {
                    s.matches.len().to_string()
                };
                match selection.and_then(|sel| s.current(sel)) {
                    Some(i) => RichText::new(format!("{} из {total}", i + 1)),
                    None => RichText::new(total),
                }
                .color(pal.subtext)
            };
            ui.label(info);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, pal, Icon::Close)
                    .on_hover_text("Закрыть (Esc)")
                    .clicked()
                {
                    actions.push(Action::CloseSearch);
                }
                let label = if s.replace_mode {
                    "Скрыть замену"
                } else {
                    "Замена"
                };
                if ui.button(label).clicked() {
                    s.replace_mode = !s.replace_mode;
                }
            });
        });
        if s.replace_mode {
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut s.replacement)
                        .hint_text("Заменить на")
                        .desired_width(260.0)
                        .margin(Margin::symmetric(8, 5)),
                );
                if ui.button("Заменить").clicked() {
                    s.pending = Some(Pending::Replace);
                }
                if ui.button("Заменить все").clicked() {
                    s.pending = Some(Pending::ReplaceAll);
                }
                if s.regex {
                    ui.weak("группы: $1, ${name}");
                }
            });
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, pal: &Palette) {
        let now = ui.input(|i| i.time);
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, at)| now - at < 4.0)
            .map(|(text, _)| text.clone());
        if toast.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        let mut error = None;
        ui.horizontal(|ui| {
            let Some(doc) = self.docs.get_mut(self.active) else {
                ui.label(
                    RichText::new(toast.unwrap_or_default())
                        .color(pal.subtext)
                        .size(12.5),
                );
                return;
            };
            let left = toast.unwrap_or_else(|| doc.location().unwrap_or_else(|| doc.title()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let small = |text: String| RichText::new(text).size(12.5).color(pal.subtext);

                ui.menu_button(small(doc.eol.label().into()), |ui| {
                    for eol in [Eol::Lf, Eol::Crlf, Eol::Cr] {
                        let name = match eol {
                            Eol::Lf => "LF — Unix",
                            Eol::Crlf => "CRLF — Windows",
                            Eol::Cr => "CR — классический Mac",
                        };
                        if ui.selectable_label(doc.eol == eol, name).clicked() && doc.eol != eol {
                            doc.eol = eol;
                            doc.mark_modified();
                        }
                    }
                });

                ui.menu_button(small(encoding_label(doc.encoding, doc.bom)), |ui| {
                    ui.weak("Сохранять в кодировке");
                    for (name, encoding, bom) in ENCODINGS {
                        let selected = doc.encoding == *encoding && doc.bom == *bom;
                        if ui.selectable_label(selected, *name).clicked() && !selected {
                            doc.encoding = encoding;
                            doc.bom = *bom;
                            doc.mark_modified();
                        }
                    }
                    if doc.path.is_some() {
                        ui.separator();
                        ui.menu_button("Переоткрыть как", |ui| {
                            for (name, encoding, bom) in ENCODINGS {
                                if *bom && *encoding == encoding_rs::UTF_8 {
                                    continue;
                                }
                                if ui.button(*name).clicked()
                                    && let Err(e) = doc.reopen_as(encoding)
                                {
                                    error = Some(e);
                                }
                            }
                        });
                    }
                });

                let config =
                    MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
                MenuButton::new(small(doc.syntax.clone()))
                    .config(config)
                    .ui(ui, |ui| {
                        ui.set_width(220.0);
                        ui.add(
                            TextEdit::singleline(&mut self.lang_filter)
                                .hint_text("Язык…")
                                .desired_width(f32::INFINITY),
                        )
                        .request_focus();
                        let filter = self.lang_filter.to_lowercase();
                        ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for name in &self.syntaxes.names {
                                if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                                    continue;
                                }
                                let selected = *name == doc.syntax;
                                if ui.selectable_label(selected, name.as_str()).clicked() {
                                    doc.syntax = name.clone();
                                    self.lang_filter.clear();
                                    ui.close();
                                }
                            }
                        });
                    });

                ui.add_space(8.0);
                let (line, col) = doc.cursor_line_col();
                ui.label(small(format!(
                    "Стр {line}, Стлб {col}   ·   строк: {}   ·   {}",
                    doc.line_count(),
                    size_label(doc.rope.len_bytes())
                )));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(small(left)).truncate());
                });
            });
        });
        if let Some(e) = error {
            self.notify(format!("Не удалось переоткрыть: {e}"), now);
        }
    }

    fn welcome(&self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, pal.base);
        ui.vertical_centered(|ui| {
            ui.add_space((rect.height() * 0.3).max(20.0));
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                ui.add_space(6.0);
            }
            ui.label(RichText::new("Tror").size(34.0).color(pal.faint));
            ui.add_space(4.0);
            ui.label(
                RichText::new("Перетащите файлы сюда или начните с пустой вкладки")
                    .color(pal.subtext),
            );
            ui.add_space(14.0);
            let button = |text: &str| {
                Button::new(RichText::new(text).color(pal.text))
                    .fill(pal.surface)
                    .min_size(egui::vec2(200.0, 32.0))
            };
            if ui.add(button("Новый файл")).clicked() {
                actions.push(Action::New);
            }
            if ui.add(button("Открыть…")).clicked() {
                actions.push(Action::Open);
            }
        });
    }

    fn goto_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(input) = &mut self.goto else {
            return;
        };
        let mut done = false;
        let mut go = false;
        let modal = dialog("tror_goto", pal).show(ctx, |ui| {
            ui.set_width(260.0);
            ui.label("Перейти к строке");
            ui.add_space(4.0);
            let field = ui.add(
                TextEdit::singleline(input)
                    .hint_text("номер строки")
                    .desired_width(f32::INFINITY),
            );
            field.request_focus();
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Перейти", true) {
                    go = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
                    done = true;
                }
            });
        });
        if go {
            if let (Ok(line), Some(doc)) = (
                input.trim().parse::<usize>(),
                self.docs.get_mut(self.active),
            ) {
                doc.goto_line(line);
            }
            done = true;
        }
        if done || modal.should_close() {
            self.goto = None;
            self.focus_editor = true;
        }
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let pal = self.palette();
        let message = match confirm {
            Confirm::Tab(id) => match self.docs.iter().find(|d| d.id == id) {
                Some(doc) => format!("Сохранить изменения в «{}»?", doc.title()),
                None => {
                    self.confirm = None;
                    return;
                }
            },
            Confirm::Quit => format!(
                "Несохранённых файлов: {}. Сохранить перед выходом?",
                self.docs.iter().filter(|d| d.dirty()).count()
            ),
            Confirm::Missing(id) => match self.docs.iter().find(|d| d.id == id) {
                Some(doc) => format!(
                    "Файл «{}» больше не существует {}. Оставить вкладку открытой?",
                    doc.title(),
                    if doc.remote.is_some() {
                        "на сервере"
                    } else {
                        "на диске"
                    }
                ),
                None => {
                    self.confirm = None;
                    return;
                }
            },
        };
        let missing = matches!(confirm, Confirm::Missing(_));
        let mut choice = None;
        let modal = dialog("tror_confirm", &pal).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(message);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if missing {
                    if dialog_button(ui, &pal, "Оставить", true) {
                        choice = Some(2);
                    }
                    if dialog_button(ui, &pal, "Закрыть вкладку", false) {
                        choice = Some(1);
                    }
                    return;
                }
                if dialog_button(ui, &pal, "Сохранить", true) {
                    choice = Some(0);
                }
                if dialog_button(ui, &pal, "Не сохранять", false) {
                    choice = Some(1);
                }
                if dialog_button(ui, &pal, "Отмена", false) {
                    choice = Some(2);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(2);
        }
        let Some(choice) = choice else {
            return;
        };
        self.confirm = None;
        match (confirm, choice) {
            (Confirm::Tab(id), 0 | 1) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id)
                    && (choice == 1 || self.save(i, now))
                {
                    self.remove_doc(i);
                }
            }
            (Confirm::Missing(id), _) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    if choice == 1 {
                        self.remove_doc(i);
                    } else {
                        self.docs[i].missing_ack = true;
                    }
                }
            }
            (Confirm::Quit, 0 | 1) if choice == 1 || self.save_all(now) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            _ => {}
        }
    }
}

impl eframe::App for TrorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        if self.applied_theme != Some(self.settings.dark) {
            theme::apply(&ctx, self.settings.dark);
            self.applied_theme = Some(self.settings.dark);
        }
        let pal = self.palette();
        let mut actions = Vec::new();

        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.docs.iter().any(|d| d.dirty())
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() && self.remote.can_upload() {
            self.remote.offer_drop(dropped);
        } else {
            for path in dropped {
                actions.push(Action::OpenPath(path));
            }
        }
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            ctx.set_zoom_factor((ctx.zoom_factor() * zoom).clamp(0.5, 3.0));
        }
        self.check_missing(&ctx);
        let modal_open = self.goto.is_some()
            || self.confirm.is_some()
            || self.remote.modal_open()
            || self.terminal.modal_open();
        if std::mem::take(&mut self.remote.want_search) {
            self.terminal.visible = true;
            self.bottom_tab = BottomTab::Search;
        }
        let events = self.remote.poll();
        self.remote_events(&ctx, events);
        for event in self.terminal.poll() {
            let TermEvent::Trust { host, fingerprint } = event;
            if let Err(e) = self.remote.trust(host, fingerprint) {
                self.notify(
                    format!("Не удалось сохранить ключ: {e}"),
                    ctx.input(|i| i.time),
                );
            }
        }
        if !modal_open {
            self.shortcuts(&ctx, &mut actions);
        }

        let bar = Frame::new().fill(pal.mantle);
        Panel::top("tror_menu")
            .frame(bar.inner_margin(Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 0,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.menu(ui, &mut actions));
        if !self.docs.is_empty() {
            Panel::top("tror_tabs")
                .frame(bar.inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 0,
                    bottom: 0,
                }))
                .show_separator_line(false)
                .show(ui, |ui| self.tabs(ui, &pal, &mut actions));
        }
        Panel::bottom("tror_status")
            .frame(bar.inner_margin(Margin::symmetric(10, 3)))
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui, &pal));
        if self.search.open && !self.docs.is_empty() {
            Panel::top("tror_search")
                .frame(
                    Frame::new()
                        .fill(pal.base)
                        .inner_margin(Margin::symmetric(10, 8))
                        .stroke(Stroke::new(1.0, pal.border)),
                )
                .show_separator_line(false)
                .show(ui, |ui| self.search_bar(ui, &pal, &mut actions));
        }

        if self.terminal.visible {
            let (profile, known) = match self.remote.selected_profile() {
                Some((profile, known)) => (Some(profile.clone()), known),
                None => (None, None),
            };
            let profile = profile.as_ref();
            Panel::bottom("tror_terminal")
                .frame(bar.inner_margin(Margin::symmetric(8, 0)))
                .resizable(true)
                .default_size(240.0)
                .min_size(90.0)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        for (tab, label) in
                            [
                                (BottomTab::Console, "SSH-консоль"),
                                (BottomTab::Log, "Журнал"),
                                (BottomTab::Search, "Поиск на сервере"),
                            ]
                        {
                            let active = self.bottom_tab == tab;
                            let text = RichText::new(label).size(12.5).color(if active {
                                pal.text
                            } else {
                                pal.subtext
                            });
                            let mut button = Button::new(text);
                            if active {
                                button = button.fill(pal.surface);
                            }
                            if ui.add(button).clicked() {
                                self.bottom_tab = tab;
                            }
                        }
                        match self.bottom_tab {
                            BottomTab::Console => self.terminal.header(ui, &pal, profile, known),
                            BottomTab::Search => {}
                            BottomTab::Log => {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(Button::new("Очистить").fill(pal.surface)).clicked() {
                                        self.remote.log.clear();
                                    }
                                });
                            }
                        }
                    });
                    ui.add_space(2.0);
                    match self.bottom_tab {
                        BottomTab::Console => self.terminal.body(ui, &pal, self.settings.dark),
                        BottomTab::Search => {
                            self.terminal.focused = false;
                            self.remote.search_ui(ui, &pal);
                        }
                        BottomTab::Log => {
                            self.terminal.focused = false;
                            let rect = ui.available_rect_before_wrap();
                            ui.painter().rect_filled(rect, 0.0, pal.base);
                            ScrollArea::vertical()
                                .auto_shrink(false)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add_space(4.0);
                                    if self.remote.log.is_empty() {
                                        ui.label(
                                            RichText::new("  Здесь появятся сообщения о подключении и операциях с сервером.")
                                                .color(pal.faint)
                                                .size(12.5),
                                        );
                                    }
                                    for (text, error) in &self.remote.log {
                                        ui.label(
                                            RichText::new(format!("  {text}"))
                                                .monospace()
                                                .size(12.5)
                                                .color(if *error { pal.warn } else { pal.subtext }),
                                        );
                                    }
                                });
                        }
                    }
                });
        } else {
            self.terminal.focused = false;
        }
        if self.remote.visible {
            Panel::left("tror_remote")
                .frame(bar.inner_margin(Margin::symmetric(8, 0)))
                .resizable(true)
                .default_size(270.0)
                .min_size(200.0)
                .show(ui, |ui| self.remote.ui(ui, &pal));
        }

        CentralPanel::no_frame().show(ui, |ui| {
            let idle = !self.search.open
                && !self.terminal.focused
                && ctx.memory(|m| m.focused().is_none());
            let focus = (std::mem::take(&mut self.focus_editor) || idle) && !modal_open;
            match self.docs.get_mut(self.active) {
                Some(doc) => editor::show(
                    ui,
                    doc,
                    &mut self.search,
                    editor::View {
                        syn: &self.syntaxes,
                        pal: &pal,
                        dark: self.settings.dark,
                        focus,
                        wrap: self.settings.wrap,
                        complete: &mut self.complete,
                    },
                ),
                None => self.welcome(ui, &pal, &mut actions),
            }
        });

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("tror_drop"),
            ));
            painter.rect_filled(screen, 0.0, pal.crust.gamma_multiply(0.85));
            painter.rect_stroke(
                screen.shrink(14.0),
                14.0,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Отпустите, чтобы открыть файл",
                FontId::proportional(22.0),
                pal.text,
            );
        }

        self.goto_modal(&ctx, &pal);
        self.confirm_modal(&ctx);
        let events = self.remote.dialogs(&ctx, &pal);
        self.remote_events(&ctx, events);
        self.terminal.dialogs(&ctx, &pal);

        for action in actions {
            self.run(action, &ctx);
        }

        let title = match self.docs.get(self.active) {
            Some(doc) => format!(
                "{}{} — Tror",
                if doc.dirty() { "● " } else { "" },
                doc.title()
            ),
            None => "Tror".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if let Some(perf) = &mut self.perf {
            perf.record(started.elapsed());
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.remote_panel = self.remote.visible;
        self.settings.terminal_panel = self.terminal.visible;
        self.settings.session = self.docs.iter().filter_map(|d| d.path.clone()).collect();
        self.settings.active = self.active;
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c: Color32 = self.palette().mantle;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }
}
