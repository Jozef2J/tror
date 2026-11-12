use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Button, Color32, ComboBox, FontId, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui};

use crate::app::{dialog, dialog_button, port_field};
use crate::remote::{self, Shell, ShellCmd, ShellEvent, Store, TermProfile, TermProtocol};
use crate::terminal::{self, Screen};
use crate::theme::Palette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Split {
    Single,
    Columns,
    Rows,
    Grid,
}

impl Split {
    fn next(self) -> Self {
        match self {
            Split::Single => Split::Columns,
            Split::Columns => Split::Rows,
            Split::Rows => Split::Grid,
            Split::Grid => Split::Single,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Split::Single => "Один сеанс",
            Split::Columns => "Рядом",
            Split::Rows => "Друг под другом",
            Split::Grid => "Сетка 2×2",
        }
    }
}

struct Session {
    id: u64,
    profile: TermProfile,
    shell: Option<Shell>,
    connected: bool,
    host_key: Option<(String, bool)>,
    status: String,
    screen: Screen,
}

impl Session {
    fn send(&self, bytes: Vec<u8>) {
        if let Some(shell) = &self.shell
            && self.connected
        {
            let _ = shell.tx.send(ShellCmd::Input(bytes));
        }
    }

    fn title(&self) -> String {
        if self.profile.name.trim().is_empty() {
            self.profile.host.trim().to_owned()
        } else {
            self.profile.name.trim().to_owned()
        }
    }
}

struct Editor {
    drafts: Vec<TermProfile>,
    index: usize,
    reveal: bool,
    probe: Option<Receiver<Result<String, String>>>,
    result: Option<Result<String, String>>,
}

pub struct ConsolePanel {
    pub store: Store,
    store_seen: u64,
    sessions: Vec<Session>,
    active: usize,
    split: Split,
    next_id: u64,
    editor: Option<Editor>,
    focused: bool,
}

pub fn probe<F>(job: F) -> Receiver<Result<String, String>>
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx
}

fn split_icon(ui: &mut Ui, pal: &Palette, split: Split, hint: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, pal.surface);
    }
    let color = if split != Split::Single {
        pal.accent
    } else if response.hovered() {
        pal.text
    } else {
        pal.subtext
    };
    let stroke = Stroke::new(1.3, color);
    let frame = Rect::from_center_size(rect.center(), egui::vec2(15.0, 12.0));
    let p = ui.painter();
    p.rect_stroke(frame, 2.0, stroke, egui::StrokeKind::Inside);
    let c = frame.center();
    match split {
        Split::Single => {}
        Split::Columns => {
            p.line_segment([egui::pos2(c.x, frame.top()), egui::pos2(c.x, frame.bottom())], stroke);
        }
        Split::Rows => {
            p.line_segment([egui::pos2(frame.left(), c.y), egui::pos2(frame.right(), c.y)], stroke);
        }
        Split::Grid => {
            p.line_segment([egui::pos2(c.x, frame.top()), egui::pos2(c.x, frame.bottom())], stroke);
            p.line_segment([egui::pos2(frame.left(), c.y), egui::pos2(frame.right(), c.y)], stroke);
        }
    }
    response.on_hover_text(hint)
}

impl ConsolePanel {
    pub fn new() -> Self {
        let path = eframe::storage_dir("Tror").map(|dir| dir.join("connections.json"));
        Self {
            store: Store::load(path),
            store_seen: Store::generation(),
            sessions: Vec::new(),
            active: 0,
            split: Split::Single,
            next_id: 1,
            editor: None,
            focused: false,
        }
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
        for session in &mut self.sessions {
            session.screen.focused = false;
        }
    }

    pub fn modal_open(&self) -> bool {
        self.editor.is_some() || self.sessions.iter().any(|s| s.host_key.is_some())
    }

    pub fn fonts(&self) -> Vec<PathBuf> {
        let mut fonts: Vec<PathBuf> = self
            .store
            .terminals
            .iter()
            .chain(self.sessions.iter().map(|s| &s.profile))
            .map(|p| p.font.trim())
            .filter(|f| !f.is_empty())
            .map(PathBuf::from)
            .collect();
        fonts.sort();
        fonts.dedup();
        fonts
    }

    fn known(&self, profile: &TermProfile) -> Option<String> {
        self.store.known_hosts.get(&profile.address()).cloned()
    }

    pub fn open(&mut self, profile: TermProfile, ctx: &egui::Context) {
        let id = self.next_id;
        self.next_id += 1;
        let mut session = Session {
            id,
            profile,
            shell: None,
            connected: false,
            host_key: None,
            status: String::new(),
            screen: Screen::new(),
        };
        if let Some(current) = self.sessions.get(self.active) {
            let (cols, rows) = current.screen.size();
            session.screen.resize(cols, rows);
        }
        self.connect(&mut session, ctx);
        self.sessions.push(session);
        self.active = self.sessions.len() - 1;
    }

    fn connect(&self, session: &mut Session, ctx: &egui::Context) {
        let wake = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        let known = self.known(&session.profile);
        let size = session.screen.size();
        session.screen.reset();
        session.screen.process(
            format!(
                "\x1b[90m{} {}…\x1b[0m\r\n",
                session.profile.protocol.label(),
                session.profile.address()
            )
            .as_bytes(),
        );
        session.shell = Some(match session.profile.protocol {
            TermProtocol::Ssh => remote::open_shell(
                &session.profile.ssh(),
                known,
                size,
                session.profile.startup.clone(),
                wake,
            ),
            TermProtocol::Telnet => crate::telnet::open(&session.profile, size, wake),
            TermProtocol::Mosh => crate::mosh::open(&session.profile, known, size, wake),
        });
        session.connected = false;
        session.host_key = None;
        session.status = "подключение…".into();
    }

    fn close(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.sessions.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
        }
    }

    pub fn poll(&mut self) -> Vec<String> {
        if self.store_seen != Store::generation() {
            self.store_seen = Store::generation();
            self.store = Store::load(self.store.path().map(|p| p.to_path_buf()));
        }
        let mut notes = Vec::new();
        let mut trusted = Vec::new();
        for session in &mut self.sessions {
            let mut events = Vec::new();
            if let Some(shell) = &session.shell {
                while let Ok(event) = shell.rx.try_recv() {
                    events.push(event);
                }
            }
            for event in events {
                match event {
                    ShellEvent::HostKey {
                        fingerprint,
                        changed,
                    } => session.host_key = Some((fingerprint, changed)),
                    ShellEvent::Connected { fingerprint } => {
                        session.connected = true;
                        session.screen.grab_focus = true;
                        session.status.clear();
                        if let Some(fingerprint) = fingerprint {
                            trusted.push((session.profile.address(), fingerprint));
                        }
                    }
                    ShellEvent::Output(bytes) => session.screen.process(&bytes),
                    ShellEvent::Status(text) => session.status = text,
                    ShellEvent::Closed(message) => {
                        session.shell = None;
                        session.connected = false;
                        session.host_key = None;
                        session.screen.process(
                            format!("\r\n\x1b[90m[{message}]\x1b[0m\r\n").as_bytes(),
                        );
                        session.status = message;
                    }
                }
            }
        }
        if !trusted.is_empty() {
            for (host, print) in trusted {
                self.store.known_hosts.insert(host, print);
            }
            if let Err(e) = self.store.save() {
                notes.push(format!("Не удалось сохранить ключ: {e}"));
            }
        }
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
        notes
    }

    fn quick_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.set_min_width(240.0);
        let mut open = None;
        if self.store.terminals.is_empty() {
            ui.weak("Своих подключений консоли пока нет");
        }
        for profile in &self.store.terminals {
            let label = format!("{}  ·  {}", profile.name, profile.protocol.label());
            if ui.button(label).clicked() {
                open = Some(profile.clone());
            }
        }
        let hosts: Vec<_> = self
            .store
            .profiles
            .iter()
            .filter(|p| p.protocol == remote::Protocol::Sftp)
            .cloned()
            .collect();
        if !hosts.is_empty() {
            ui.separator();
            ui.menu_button("По профилю SFTP", |ui| {
                for host in &hosts {
                    if ui.button(&host.name).clicked() {
                        open = Some(TermProfile::from_host(host));
                    }
                }
            });
        }
        ui.separator();
        if ui.button("Новое подключение…").clicked() {
            self.open_editor(true);
        }
        if ui.button("Настроить подключения…").clicked() {
            self.open_editor(false);
        }
        if let Some(profile) = open {
            self.open(profile, ctx);
            ui.close();
        }
    }

    fn open_editor(&mut self, new: bool) {
        let mut drafts = self.store.terminals.clone();
        if new || drafts.is_empty() {
            drafts.push(TermProfile {
                name: format!("Сервер {}", drafts.len() + 1),
                ..Default::default()
            });
        }
        let index = if new {
            drafts.len() - 1
        } else {
            self.sessions
                .get(self.active)
                .and_then(|s| drafts.iter().position(|d| d.name == s.profile.name))
                .unwrap_or(0)
        };
        self.editor = Some(Editor {
            drafts,
            index,
            reveal: false,
            probe: None,
            result: None,
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, dark: bool, font_size: f32) {
        let ctx = ui.ctx().clone();
        ui.add_space(4.0);
        let mut close = None;
        let mut toggle_split = false;
        let mut reconnect = false;
        let mut disconnect = false;
        let mut clear = false;
        let mut activate = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let reserve = 96.0;
            let strip_w = (ui.available_width() - reserve).max(40.0);
            ScrollArea::horizontal()
                .id_salt("tror_console_tabs")
                .max_width(strip_w)
                .auto_shrink([true, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for (index, session) in self.sessions.iter().enumerate() {
                            let active = index == self.active;
                            let title = session.title();
                            let font = FontId::proportional(12.5);
                            let text_w = ui.fonts_mut(|f| {
                                f.layout_no_wrap(title.clone(), font.clone(), pal.text).size().x
                            });
                            let size = egui::vec2(text_w + 46.0, 24.0);
                            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                            let fill = if active {
                                pal.surface
                            } else if response.hovered() {
                                pal.crust
                            } else {
                                Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(rect, 6.0, fill);
                            let dot = if session.connected {
                                Color32::from_rgb(0x3f, 0xc2, 0x6e)
                            } else if session.shell.is_some() {
                                pal.warn
                            } else {
                                pal.faint
                            };
                            ui.painter().circle_filled(
                                egui::pos2(rect.left() + 11.0, rect.center().y),
                                3.5,
                                dot,
                            );
                            ui.painter().text(
                                egui::pos2(rect.left() + 20.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &title,
                                font,
                                if active { pal.text } else { pal.subtext },
                            );
                            let x_rect = Rect::from_center_size(
                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                egui::vec2(16.0, 16.0),
                            );
                            let x = ui.interact(x_rect, ui.scope_id().with(("tror_term_close", session.id)), Sense::click());
                            if x.hovered() {
                                ui.painter().rect_filled(x_rect, 4.0, pal.surface_hover);
                            }
                            let d = 3.5;
                            let c = x_rect.center();
                            let stroke = Stroke::new(1.2, if x.hovered() { pal.text } else { pal.faint });
                            ui.painter().line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], stroke);
                            ui.painter().line_segment([c + egui::vec2(d, -d), c + egui::vec2(-d, d)], stroke);
                            if x.clicked() || response.middle_clicked() {
                                close = Some(index);
                            } else if response.clicked() {
                                activate = Some(index);
                            }
                            let hint = format!(
                                "{} {}{}",
                                session.profile.protocol.label(),
                                session.profile.address(),
                                if session.status.is_empty() {
                                    String::new()
                                } else {
                                    format!(" — {}", session.status)
                                }
                            );
                            response.on_hover_text(hint);
                        }
                    });
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let dots = crate::widgets::dots_button(ui, pal).on_hover_text("Ещё");
                egui::Popup::menu(&dots).show(|ui| {
                    ui.set_min_width(220.0);
                    let has = !self.sessions.is_empty();
                    if ui.add_enabled(has, Button::new("Переподключить")).clicked() {
                        reconnect = true;
                    }
                    if ui.add_enabled(has, Button::new("Отключить")).clicked() {
                        disconnect = true;
                    }
                    if ui.add_enabled(has, Button::new("Очистить экран")).clicked() {
                        clear = true;
                    }
                    ui.separator();
                    ui.label(RichText::new("Разделение экрана").size(12.0).color(pal.faint));
                    for split in [Split::Single, Split::Columns, Split::Rows, Split::Grid] {
                        if ui.selectable_label(self.split == split, split.label()).clicked() {
                            self.split = split;
                        }
                    }
                    ui.separator();
                    if ui.button("Настроить подключения…").clicked() {
                        self.open_editor(false);
                    }
                });
                if split_icon(ui, pal, self.split, "Разделить экран между сеансами").clicked() {
                    toggle_split = true;
                }
                let plus = ui.menu_button(RichText::new("+").size(16.0), |ui| self.quick_menu(ui, &ctx));
                plus.response.on_hover_text("Новый сеанс");
            });
        });
        if toggle_split {
            self.split = self.split.next();
        }
        if let Some(index) = activate {
            self.active = index;
            self.focus_active();
        }
        if let Some(index) = close {
            self.close(index);
        }
        if let Some(session) = self.sessions.get_mut(self.active) {
            if disconnect {
                session.shell = None;
                session.connected = false;
                session.status = "отключено".into();
                session.screen.process("\r\n\x1b[90m[отключено]\x1b[0m\r\n".as_bytes());
            }
            if clear {
                session.screen.reset();
                session.send(vec![0x0c]);
            }
        }
        if reconnect && self.active < self.sessions.len() {
            let mut session = self.sessions.remove(self.active);
            self.connect(&mut session, &ctx);
            self.sessions.insert(self.active, session);
        }
        ui.add_space(2.0);
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        if self.sessions.is_empty() {
            self.empty(ui, rect, pal, &ctx);
            self.focused = false;
            return;
        }
        self.active = self.active.min(self.sessions.len() - 1);
        let visible: Vec<usize> = match self.split {
            Split::Single => vec![self.active],
            _ => {
                let n = self.sessions.len();
                let start = if self.active < 4 { 0 } else { self.active - 3 };
                (start..n.min(start + 4)).collect()
            }
        };
        let rects = layout(rect, self.split, visible.len());
        let mut focused = false;
        let mut clicked = None;
        let show_labels = visible.len() > 1;
        for (slot, index) in visible.iter().enumerate() {
            let Some(&pane) = rects.get(slot) else {
                break;
            };
            let session = &mut self.sessions[*index];
            let colors = terminal::colors(&session.profile.theme, pal, dark);
            let size = if session.profile.font_size > 0.0 {
                session.profile.font_size
            } else {
                font_size
            };
            let family = crate::theme::term_family(session.profile.font.trim())
                .unwrap_or(egui::FontFamily::Monospace);
            let font = FontId::new(size, family);
            let top = if show_labels { 18.0 } else { 0.0 };
            let body = Rect::from_min_max(pane.min + egui::vec2(0.0, top), pane.max);
            if show_labels {
                let label = Rect::from_min_max(pane.min, egui::pos2(pane.right(), pane.top() + top));
                ui.painter().rect_filled(label, 0.0, colors.bg);
                ui.painter().text(
                    label.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    session.title(),
                    FontId::proportional(11.5),
                    if *index == self.active { colors.fg } else { colors.fg.gamma_multiply(0.55) },
                );
            }
            let (cols, rows) = session.screen.grid(&font, ui, body);
            if session.screen.resize(cols, rows)
                && let Some(shell) = &session.shell
            {
                let _ = shell.tx.send(ShellCmd::Resize(cols, rows));
            }
            let id = egui::Id::unique(("tror_term", session.id));
            let look = terminal::Look {
                colors: &colors,
                font,
                cursor: session.connected,
                active: show_labels && *index == self.active,
            };
            let typed = session.screen.show(ui, body, id, &look);
            if !session.status.is_empty() && !session.connected || (session.connected && !session.status.is_empty()) {
                let note = Rect::from_min_size(
                    egui::pos2(body.left() + 8.0, body.bottom() - 26.0),
                    egui::vec2((body.width() - 16.0).max(10.0), 20.0),
                );
                ui.painter().rect_filled(note, 4.0, colors.bg.gamma_multiply(0.9));
                ui.painter().with_clip_rect(note).text(
                    note.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &session.status,
                    FontId::proportional(12.0),
                    colors.fg.gamma_multiply(0.7),
                );
            }
            for bytes in typed {
                session.send(bytes);
            }
            if session.screen.focused {
                focused = true;
                if *index != self.active {
                    clicked = Some(*index);
                }
            }
        }
        if let Some(index) = clicked {
            self.active = index;
        }
        self.focused = focused;
    }

    fn focus_active(&mut self) {
        if let Some(session) = self.sessions.get_mut(self.active) {
            session.screen.grab_focus = true;
        }
    }

    fn empty(&mut self, ui: &mut Ui, rect: Rect, pal: &Palette, ctx: &egui::Context) {
        ui.painter().rect_filled(rect, 0.0, pal.base);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(16.0)));
        child.vertical(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Нет открытых сеансов").color(pal.subtext));
            ui.add_space(6.0);
            let mut open = None;
            ui.horizontal_wrapped(|ui| {
                for profile in &self.store.terminals {
                    let text = format!("{}  ·  {}", profile.name, profile.protocol.label());
                    if ui.add(Button::new(text).fill(pal.surface)).clicked() {
                        open = Some(profile.clone());
                    }
                }
                for host in self
                    .store
                    .profiles
                    .iter()
                    .filter(|p| p.protocol == remote::Protocol::Sftp)
                {
                    let text = format!("{}  ·  SFTP-профиль", host.name);
                    if ui.add(Button::new(text).fill(pal.surface)).clicked() {
                        open = Some(TermProfile::from_host(host));
                    }
                }
            });
            ui.add_space(6.0);
            if ui
                .add(Button::new("Новое подключение (SSH, Telnet, Mosh)…").fill(pal.surface))
                .clicked()
            {
                self.open_editor(true);
            }
            if let Some(profile) = open {
                self.open(profile, ctx);
            }
        });
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        self.host_key_dialog(ctx, pal);
        self.editor_dialog(ctx, pal);
    }

    fn host_key_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(index) = self.sessions.iter().position(|s| s.host_key.is_some()) else {
            return;
        };
        let Some((fingerprint, changed)) = self.sessions[index].host_key.clone() else {
            return;
        };
        let title = self.sessions[index].title();
        let mut answer = None;
        dialog(("tror_console_host_key", self.sessions[index].id), pal).show(ctx, |ui| {
            ui.set_width(460.0);
            if changed {
                ui.label(RichText::new(format!("Ключ сервера «{title}» изменился")).color(pal.warn).strong());
                ui.label(
                    "Сохранённый ключ этого сервера отличается от присланного. Так бывает после \
                     переустановки сервера, но так же выглядит перехват соединения.",
                );
            } else {
                ui.label(RichText::new(format!("Новый сервер «{title}»")).strong());
                ui.label("К этому серверу вы подключаетесь впервые. Отпечаток его ключа (SHA-256):");
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
        let session = &mut self.sessions[index];
        match answer {
            Some(true) => {
                session.host_key = None;
                if let Some(shell) = &session.shell {
                    let _ = shell.tx.send(ShellCmd::TrustHost(true));
                }
            }
            Some(false) => {
                if let Some(shell) = &session.shell {
                    let _ = shell.tx.send(ShellCmd::TrustHost(false));
                }
                session.host_key = None;
                session.shell = None;
                session.status = "подключение отменено".into();
            }
            None => {}
        }
    }

    fn editor_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let mut connect = false;
        let mut test = false;
        let modal = dialog("tror_console_profiles", pal).show(ctx, |ui| {
            ui.set_width(660.0);
            ui.label(RichText::new("Подключения консоли").strong());
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                        for (i, profile) in editor.drafts.iter().enumerate() {
                            let name = if profile.name.trim().is_empty() {
                                "без имени".to_owned()
                            } else {
                                format!("{}  ·  {}", profile.name, profile.protocol.label())
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
                            editor.drafts.push(TermProfile {
                                name: format!("Сервер {}", editor.drafts.len() + 1),
                                ..Default::default()
                            });
                            editor.index = editor.drafts.len() - 1;
                            editor.result = None;
                        }
                        if ui
                            .add_enabled(!editor.drafts.is_empty(), Button::new("Удалить").fill(pal.surface))
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
                        ui.label(RichText::new("Добавьте подключение.").color(pal.subtext));
                        return;
                    };
                    egui::Grid::new("tror_term_form")
                        .num_columns(2)
                        .spacing([10.0, 7.0])
                        .show(ui, |ui| {
                            let wide = 320.0;
                            ui.label("Название");
                            ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                            ui.end_row();
                            ui.label("Протокол");
                            ui.horizontal(|ui| {
                                for protocol in TermProtocol::ALL {
                                    if ui.selectable_label(p.protocol == protocol, protocol.label()).clicked()
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
                                ui.add(TextEdit::singleline(&mut p.host).hint_text("example.org").desired_width(wide - 70.0));
                                port_field(ui, &mut p.port, 60.0);
                            });
                            ui.end_row();
                            ui.label("Пользователь");
                            ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                            ui.end_row();
                            ui.label("Пароль");
                            ui.horizontal(|ui| {
                                ui.add(TextEdit::singleline(&mut p.password).password(!editor.reveal).desired_width(wide - 88.0));
                                let label = if editor.reveal { "Скрыть" } else { "Показать" };
                                if ui.add_sized([80.0, 22.0], Button::new(label)).clicked() {
                                    editor.reveal = !editor.reveal;
                                }
                            });
                            ui.end_row();
                            if p.protocol != TermProtocol::Telnet {
                                ui.label("Файл ключа");
                                ui.horizontal(|ui| {
                                    ui.add(TextEdit::singleline(&mut p.key_file).hint_text("RSA, ECDSA, Ed25519").desired_width(wide - 78.0));
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
                                        Ok(kind) => ui.label(RichText::new(format!("ключ: {kind}")).size(12.0).color(pal.subtext)),
                                        Err(e) => ui.label(RichText::new(e).size(12.0).color(pal.warn)),
                                    };
                                    ui.end_row();
                                    ui.label("Парольная фраза");
                                    ui.add(TextEdit::singleline(&mut p.passphrase).password(!editor.reveal).hint_text("если ключ защищён").desired_width(wide));
                                    ui.end_row();
                                }
                            }
                            if p.protocol == TermProtocol::Mosh {
                                ui.label("mosh-server");
                                ui.add(TextEdit::singleline(&mut p.mosh_server).hint_text("mosh-server").desired_width(wide));
                                ui.end_row();
                                ui.label("Порты UDP");
                                ui.add(TextEdit::singleline(&mut p.mosh_ports).hint_text("60000:61000").desired_width(wide));
                                ui.end_row();
                            }
                            if p.protocol == TermProtocol::Ssh {
                                ui.label("Команда при входе");
                                ui.add(TextEdit::singleline(&mut p.startup).hint_text("например: cd /var/www && ls").desired_width(wide));
                                ui.end_row();
                            }
                            ui.label("Цветовая схема");
                            ComboBox::from_id_salt("tror_term_theme")
                                .width(wide)
                                .selected_text(if p.theme.is_empty() { "Как в редакторе" } else { p.theme.as_str() })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut p.theme, String::new(), "Как в редакторе");
                                    for scheme in terminal::SCHEMES {
                                        ui.selectable_value(&mut p.theme, scheme.name.to_owned(), scheme.name);
                                    }
                                });
                            ui.end_row();
                            ui.label("");
                            let colors = terminal::colors(&p.theme, pal, true);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(wide, 22.0), Sense::hover());
                            ui.painter().rect_filled(rect, 4.0, colors.bg);
                            for (i, color) in colors.ansi.iter().take(8).enumerate() {
                                ui.painter().circle_filled(
                                    egui::pos2(rect.left() + 14.0 + i as f32 * 18.0, rect.center().y),
                                    5.5,
                                    *color,
                                );
                            }
                            ui.painter().text(
                                egui::pos2(rect.left() + 162.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                "user@host:~$ ls",
                                FontId::monospace(12.0),
                                colors.fg,
                            );
                            ui.end_row();
                            ui.label("Шрифт");
                            ui.horizontal(|ui| {
                                let name = if p.font.trim().is_empty() {
                                    "как в редакторе".to_owned()
                                } else {
                                    std::path::Path::new(p.font.trim())
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| p.font.clone())
                                };
                                ui.label(RichText::new(name).color(pal.subtext));
                                if ui.button("Выбрать…").clicked()
                                    && let Some(path) = rfd::FileDialog::new().add_filter("Шрифты", &["ttf", "otf", "ttc"]).pick_file()
                                {
                                    p.font = path.display().to_string();
                                }
                                if !p.font.is_empty() && ui.button("Сбросить").clicked() {
                                    p.font.clear();
                                }
                            });
                            ui.end_row();
                            ui.label("Размер шрифта");
                            ui.horizontal(|ui| {
                                let mut custom = p.font_size > 0.0;
                                if ui.checkbox(&mut custom, "свой").changed() {
                                    p.font_size = if custom { 14.0 } else { 0.0 };
                                }
                                if custom {
                                    ui.add(egui::Slider::new(&mut p.font_size, 8.0..=28.0).step_by(1.0).suffix(" px"));
                                }
                            });
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    let note = match p.protocol {
                        TermProtocol::Ssh => "Вход по паролю, по ключу (RSA, ECDSA, Ed25519) или через ssh-agent, если не заданы ни пароль, ни ключ.",
                        TermProtocol::Telnet => "Telnet передаёт всё, включая пароль, без шифрования — используйте его для сетевого оборудования в своей сети. Имя и пароль, если указаны, вводятся сами в ответ на приглашение входа.",
                        TermProtocol::Mosh => "Mosh входит по SSH, запускает на сервере mosh-server и дальше работает по UDP: сеанс переживает обрывы, смену сети и сон ноутбука. На сервере должен быть установлен mosh, а порты UDP 60000–61000 — открыты.",
                    };
                    let color = if p.protocol == TermProtocol::Telnet { pal.warn } else { pal.subtext };
                    ui.label(RichText::new(note).size(12.0).color(color));
                    if let Some(result) = &editor.result {
                        let (text, color) = match result {
                            Ok(text) => (format!("Работает: {text}"), Color32::from_rgb(0x3f, 0xc2, 0x6e)),
                            Err(e) => (format!("Ошибка: {e}"), pal.warn),
                        };
                        ui.label(RichText::new(text).size(12.5).color(color));
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Сохранить", true) {
                    save = true;
                }
                if dialog_button(ui, pal, "Сохранить и подключиться", false) {
                    save = true;
                    connect = true;
                }
                let testing = editor.probe.is_some();
                if testing {
                    ui.spinner();
                } else if dialog_button(ui, pal, "Проверить", false) {
                    test = true;
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
            let known = self.store.known_hosts.get(&profile.address()).cloned();
            editor.result = None;
            editor.probe = Some(probe(move || remote::test_terminal(&profile, known)));
        }
        if save {
            if let Some(editor) = self.editor.take() {
                let chosen = editor.drafts.get(editor.index).cloned();
                self.store.terminals = editor.drafts;
                if let Err(e) = self.store.save() {
                    crate::trace::popup(format!("Подключения не сохранены: {e}"), false);
                }
                if connect && let Some(profile) = chosen {
                    self.open(profile, ctx);
                }
            }
        } else if cancel || modal.should_close() {
            self.editor = None;
        }
    }
}

fn layout(rect: Rect, split: Split, count: usize) -> Vec<Rect> {
    let gap = 2.0;
    let count = count.max(1);
    match split {
        Split::Single => vec![rect],
        Split::Columns => {
            let w = (rect.width() - gap * (count as f32 - 1.0)) / count as f32;
            (0..count)
                .map(|i| Rect::from_min_size(rect.min + egui::vec2(i as f32 * (w + gap), 0.0), egui::vec2(w, rect.height())))
                .collect()
        }
        Split::Rows => {
            let h = (rect.height() - gap * (count as f32 - 1.0)) / count as f32;
            (0..count)
                .map(|i| Rect::from_min_size(rect.min + egui::vec2(0.0, i as f32 * (h + gap)), egui::vec2(rect.width(), h)))
                .collect()
        }
        Split::Grid => {
            let columns = if count > 1 { 2 } else { 1 };
            let rows = count.div_ceil(columns);
            let w = (rect.width() - gap * (columns as f32 - 1.0)) / columns as f32;
            let h = (rect.height() - gap * (rows as f32 - 1.0)) / rows as f32;
            (0..count)
                .map(|i| {
                    let (r, c) = (i / columns, i % columns);
                    let width = if count == 3 && r == 1 { rect.width() } else { w };
                    Rect::from_min_size(
                        rect.min + egui::vec2(c as f32 * (w + gap), r as f32 * (h + gap)),
                        egui::vec2(width, h),
                    )
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_layouts_cover_the_area() {
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(802.0, 402.0));
        let cols = layout(rect, Split::Columns, 2);
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].width(), 400.0);
        assert_eq!(cols[1].left(), 402.0);
        let grid = layout(rect, Split::Grid, 4);
        assert_eq!(grid[3].min, egui::pos2(402.0, 202.0));
        let three = layout(rect, Split::Grid, 3);
        assert_eq!(three[2].width(), 802.0);
        assert_eq!(Split::Grid.next(), Split::Single);
    }
}
