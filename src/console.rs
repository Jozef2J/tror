use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui::{
    self, Button, Color32, ComboBox, FontId, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit,
    Ui,
};

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
            Split::Single => tr!("Один сеанс"),
            Split::Columns => tr!("Рядом"),
            Split::Rows => tr!("Друг под другом"),
            Split::Grid => tr!("Сетка 2×2"),
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
    tabs_scroll: Option<f32>,
    tabs_overflow: bool,
    pub cwd: Option<std::path::PathBuf>,
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
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, if response.is_pointer_button_down_on() { pal.surface_hover } else { pal.surface });
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
            p.line_segment(
                [
                    egui::pos2(c.x, frame.top()),
                    egui::pos2(c.x, frame.bottom()),
                ],
                stroke,
            );
        }
        Split::Rows => {
            p.line_segment(
                [
                    egui::pos2(frame.left(), c.y),
                    egui::pos2(frame.right(), c.y),
                ],
                stroke,
            );
        }
        Split::Grid => {
            p.line_segment(
                [
                    egui::pos2(c.x, frame.top()),
                    egui::pos2(c.x, frame.bottom()),
                ],
                stroke,
            );
            p.line_segment(
                [
                    egui::pos2(frame.left(), c.y),
                    egui::pos2(frame.right(), c.y),
                ],
                stroke,
            );
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
            tabs_scroll: None,
            tabs_overflow: false,
            cwd: None,
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
            .chain(self.store.profiles.iter().map(|p| p.term_font.trim()))
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
        let place = if session.profile.protocol == TermProtocol::Local {
            format!("{} · {}", crate::local::program(), session.profile.host)
        } else {
            session.profile.address()
        };
        session.screen.process(
            format!(
                "\x1b[90m{} {place}…\x1b[0m\r\n",
                session.profile.protocol.label()
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
            TermProtocol::Local => crate::local::open(
                (!session.profile.host.is_empty()).then(|| std::path::PathBuf::from(&session.profile.host)),
                size,
                wake,
            ),
        });
        session.connected = false;
        session.host_key = None;
        session.status = tr!("подключение…").into();
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
            self.sync_looks();
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
                        session
                            .screen
                            .process(format!("\r\n\x1b[90m[{message}]\x1b[0m\r\n").as_bytes());
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
                notes.push(tf!("Не удалось сохранить ключ: {e}", "Could not save the key: {e}"));
            }
        }
        if let Some(editor) = &mut self.editor
            && let Some(rx) = &editor.probe
            && let Ok(result) = rx.try_recv()
        {
            editor.probe = None;
            editor.result = Some(result);
        }
        notes
    }

    fn local_profile(&self) -> TermProfile {
        TermProfile {
            name: tr!("Локальный терминал").into(),
            protocol: TermProtocol::Local,
            host: self
                .cwd
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn quick_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.set_width(280.0);
        ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
        let mut open = None;
        if ui
            .button(tr!("Локальный терминал"))
            .on_hover_text(tf!(
                "{} на этом компьютере, в папке проекта",
                "{} on this computer, in the project folder",
                crate::local::program()
            ))
            .clicked()
        {
            open = Some(self.local_profile());
        }
        ui.separator();
        let names: Vec<(String, String)> = self
            .store
            .terminals
            .iter()
            .map(|p| (p.name.clone(), p.protocol.label().to_owned()))
            .collect();
        let hosts: Vec<_> = self
            .store
            .profiles
            .iter()
            .filter(|p| p.protocol == remote::Protocol::Sftp)
            .cloned()
            .collect();
        if !hosts.is_empty() {
            ui.menu_button(tr!("По профилю SFTP"), |ui| {
                for host in &hosts {
                    if ui.button(&host.name).clicked() {
                        open = Some(TermProfile::from_host(host));
                    }
                }
            });
            ui.separator();
        }
        let action = crate::widgets::profile_list(
            ui,
            egui::Id::unique("tror_term_profiles"),
            &names,
            None,
            tr!("Добавить подключение…"),
        );
        if let Some(action) = action {
            self.profile_action(action, ctx);
        }
        if let Some(profile) = open {
            self.open(profile, ctx);
            ui.close();
        }
    }

    fn profile_action(&mut self, action: crate::widgets::ProfileAction, ctx: &egui::Context) {
        use crate::widgets::ProfileAction;
        match action {
            ProfileAction::Select(i) => {
                if let Some(profile) = self.store.terminals.get(i).cloned() {
                    self.open(profile, ctx);
                }
            }
            ProfileAction::Edit(i) => {
                self.open_editor(false);
                if let Some(editor) = &mut self.editor {
                    editor.index = i.min(editor.drafts.len().saturating_sub(1));
                }
            }
            ProfileAction::Rename(i, name) => {
                if let Some(profile) = self.store.terminals.get_mut(i) {
                    profile.name = name;
                    let _ = self.store.save();
                }
            }
            ProfileAction::Delete(i) => {
                if i < self.store.terminals.len() {
                    self.store.terminals.remove(i);
                    let _ = self.store.save();
                }
            }
            ProfileAction::Add => self.open_editor(true),
        }
    }

    fn open_editor(&mut self, new: bool) {
        let mut drafts = self.store.terminals.clone();
        if new || drafts.is_empty() {
            drafts.push(TermProfile {
                name: tf!("Сервер {}", "Server {}", drafts.len() + 1),
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
        let names: Vec<(String, String)> = self
            .store
            .terminals
            .iter()
            .map(|p| (p.name.clone(), p.protocol.label().to_owned()))
            .collect();
        if let Some(action) =
            crate::widgets::profile_dialogs(&ctx, pal, egui::Id::unique("tror_term_profiles"), &names)
        {
            self.profile_action(action, &ctx);
        }
        ui.add_space(4.0);
        let mut close = None;
        let mut toggle_split = false;
        let mut reconnect = false;
        let mut disconnect = false;
        let mut clear = false;
        let mut activate = None;
        let mut look: Option<(String, f32)> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let reserve = if self.tabs_overflow { 138.0 } else { 96.0 };
            let strip_w = (ui.available_width() - reserve).max(40.0);
            let mut strip = ScrollArea::horizontal()
                .id_salt("tror_console_tabs")
                .max_width(strip_w);
            if let Some(offset) = self.tabs_scroll.take() {
                strip = strip.horizontal_scroll_offset(offset);
            }
            let strip = strip
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
                                f.layout_no_wrap(title.clone(), font.clone(), pal.text)
                                    .size()
                                    .x
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
                            let x = ui.interact(
                                x_rect,
                                ui.scope_id().with(("tror_term_close", session.id)),
                                Sense::click(),
                            );
                            if x.hovered() {
                                ui.painter().rect_filled(x_rect, 4.0, pal.surface_hover);
                            }
                            let d = 3.5;
                            let c = x_rect.center();
                            let stroke =
                                Stroke::new(1.2, if x.hovered() { pal.text } else { pal.faint });
                            ui.painter().line_segment(
                                [c + egui::vec2(-d, -d), c + egui::vec2(d, d)],
                                stroke,
                            );
                            ui.painter().line_segment(
                                [c + egui::vec2(d, -d), c + egui::vec2(-d, d)],
                                stroke,
                            );
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
            let offset = strip.state.clamped_offset().x;
            let reach = (strip.content_size.x - strip.inner_rect.width()).max(0.0);
            self.tabs_overflow = reach > 1.0;
            if self.tabs_overflow {
                ui.spacing_mut().item_spacing.x = 0.0;
                if crate::widgets::scroll_arrow(ui, pal, false, offset > 0.5).clicked() {
                    self.tabs_scroll = Some((offset - 140.0).max(0.0));
                }
                if crate::widgets::scroll_arrow(ui, pal, true, offset < reach - 0.5).clicked() {
                    self.tabs_scroll = Some((offset + 140.0).min(reach));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let dots = crate::widgets::dots_button(ui, pal).on_hover_text(tr!("Ещё"));
                egui::Popup::menu(&dots)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_min_width(220.0);
                        let has = !self.sessions.is_empty();
                        if ui.add_enabled(has, Button::new(tr!("Переподключить"))).clicked()
                        {
                            reconnect = true;
                            ui.close();
                        }
                        if ui.add_enabled(has, Button::new(tr!("Отключить"))).clicked() {
                            disconnect = true;
                            ui.close();
                        }
                        if ui.add_enabled(has, Button::new(tr!("Очистить экран"))).clicked()
                        {
                            clear = true;
                            ui.close();
                        }
                        if let Some(session) = self.sessions.get(self.active) {
                            let current = session.profile.theme.clone();
                            let size = session.profile.font_size;
                            ui.menu_button(tr!("Оформление сеанса"), |ui| {
                                ui.set_min_width(220.0);
                                ui.label(
                                    RichText::new(tr!("Цветовая схема")).small().color(pal.faint),
                                );
                                ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                    if ui
                                        .selectable_label(current.is_empty(), tr!("Как в редакторе"))
                                        .clicked()
                                    {
                                        look = Some((String::new(), size));
                                    }
                                    for scheme in terminal::SCHEMES {
                                        if ui
                                            .selectable_label(current == scheme.name, tr!(scheme.name))
                                            .clicked()
                                        {
                                            look = Some((scheme.name.to_owned(), size));
                                        }
                                    }
                                });
                                ui.separator();
                                let base = if size > 0.0 { size } else { font_size.round() };
                                ui.horizontal(|ui| {
                                    ui.label(tf!("Шрифт: {base:.0} px", "Font: {base:.0} px"));
                                    if ui.small_button(" − ").clicked() {
                                        look = Some((current.clone(), (base - 1.0).max(8.0)));
                                    }
                                    if ui.small_button(" + ").clicked() {
                                        look = Some((current.clone(), (base + 1.0).min(28.0)));
                                    }
                                    if size > 0.0 && ui.small_button(tr!("как в редакторе")).clicked()
                                    {
                                        look = Some((current.clone(), 0.0));
                                    }
                                });
                            });
                        }
                        ui.separator();
                        ui.label(
                            RichText::new(tr!("Разделение экрана"))
                                .small()
                                .color(pal.faint),
                        );
                        for split in [Split::Single, Split::Columns, Split::Rows, Split::Grid] {
                            if ui
                                .selectable_label(self.split == split, split.label())
                                .clicked()
                            {
                                self.split = split;
                            }
                        }
                        ui.separator();
                        if ui.button(tr!("Настроить подключения…")).clicked() {
                            self.open_editor(false);
                            ui.close();
                        }
                    });
                if split_icon(ui, pal, self.split, tr!("Разделить экран между сеансами")).clicked()
                {
                    toggle_split = true;
                }
                let plus = crate::widgets::plus_button(ui, pal, egui::vec2(24.0, 22.0))
                    .on_hover_text(tr!("Новый сеанс"));
                egui::Popup::menu(&plus)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.quick_menu(ui, &ctx));
            });
        });
        if toggle_split {
            self.split = self.split.next();
        }
        if let Some((theme, size)) = look {
            self.restyle(theme, size);
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
                session.status = tr!("отключено").into();
                session
                    .screen
                    .process(tr!("\r\n\x1b[90m[отключено]\x1b[0m\r\n").as_bytes());
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
                let label =
                    Rect::from_min_max(pane.min, egui::pos2(pane.right(), pane.top() + top));
                ui.painter().rect_filled(label, 0.0, colors.bg);
                ui.painter().text(
                    label.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    session.title(),
                    FontId::proportional(11.5),
                    if *index == self.active {
                        colors.fg
                    } else {
                        colors.fg.gamma_multiply(0.55)
                    },
                );
            }
            let (cols, rows) = session.screen.grid(&font, ui, body);
            let dragging = ui.input(|i| i.pointer.primary_down());
            let now = ui.input(|i| i.time);
            if !dragging
                && session.screen.settle(cols, rows, now)
                && let Some(shell) = &session.shell
            {
                let _ = shell.tx.send(ShellCmd::Resize(cols, rows));
            }
            if session.screen.unsettled() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(60));
            }
            let id = egui::Id::unique(("tror_term", session.id));
            let look = terminal::Look {
                colors: &colors,
                font,
                cursor: session.connected,
                active: show_labels && *index == self.active,
            };
            let typed = session.screen.show(ui, body, id, &look);
            if !session.status.is_empty() && !session.connected
                || (session.connected && !session.status.is_empty())
            {
                let note = Rect::from_min_size(
                    egui::pos2(body.left() + 8.0, body.bottom() - 26.0),
                    egui::vec2((body.width() - 16.0).max(10.0), 20.0),
                );
                ui.painter()
                    .rect_filled(note, 4.0, colors.bg.gamma_multiply(0.9));
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

    fn restyle(&mut self, theme: String, size: f32) {
        let Some(session) = self.sessions.get_mut(self.active) else {
            return;
        };
        session.profile.theme = theme.clone();
        session.profile.font_size = size;
        let name = session.profile.name.clone();
        let saved = match session.profile.origin.clone() {
            Some(host) => self
                .store
                .profiles
                .iter_mut()
                .find(|p| p.name == host)
                .map(|p| {
                    p.term_theme = theme;
                    p.term_font_size = size;
                }),
            None => self
                .store
                .terminals
                .iter_mut()
                .find(|p| p.name == name)
                .map(|p| {
                    p.theme = theme;
                    p.font_size = size;
                }),
        };
        if saved.is_some() {
            if let Err(e) = self.store.save() {
                crate::trace::popup(tf!("Оформление не сохранено: {e}", "Appearance not saved: {e}"), false);
            }
            self.store_seen = Store::generation();
        }
    }

    fn focus_active(&mut self) {
        if let Some(session) = self.sessions.get_mut(self.active) {
            session.screen.grab_focus = true;
        }
    }

    fn empty(&mut self, ui: &mut Ui, rect: Rect, pal: &Palette, ctx: &egui::Context) {
        ui.painter().rect_filled(rect, 0.0, pal.base);
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(16.0, 4.0))));
        ScrollArea::vertical()
            .id_salt("tror_console_empty")
            .auto_shrink(false)
            .show(&mut child, |ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(tr!("Нет открытых сеансов")).color(pal.subtext));
                ui.add_space(6.0);
                let mut open = None;
                let mut action = None;
                ui.horizontal_wrapped(|ui| {
                    if ui.add(Button::new(tr!("Локальный терминал"))).clicked() {
                        open = Some(self.local_profile());
                    }
                    let mut order: Vec<_> = self.store.terminals.iter().enumerate().collect();
                    order.sort_by_cached_key(|(_, p)| p.name.to_lowercase());
                    for (index, profile) in order {
                        let text = format!("{}  ·  {}", profile.name, profile.protocol.label());
                        let chip = ui.add(Button::new(text));
                        if chip.clicked() {
                            open = Some(profile.clone());
                        }
                        chip.context_menu(|ui| {
                            let id = egui::Id::unique("tror_term_profiles");
                            if let Some(a) = crate::widgets::profile_menu(ui, id, index, &profile.name) {
                                action = Some(a);
                            }
                        });
                    }
                    for host in self
                        .store
                        .profiles
                        .iter()
                        .filter(|p| p.protocol == remote::Protocol::Sftp)
                    {
                        let text = tf!("{}  ·  SFTP-профиль", "{}  ·  SFTP profile", host.name);
                        if ui.add(Button::new(text)).clicked() {
                            open = Some(TermProfile::from_host(host));
                        }
                    }
                });
                if let Some(action) = action {
                    self.profile_action(action, ctx);
                }
                if let Some(profile) = open {
                    self.open(profile, ctx);
                }
                ui.add_space(12.0);
            });
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        self.host_key_dialog(ctx, pal);
        let editing = self.editor.is_some();
        self.editor_dialog(ctx, pal);
        if let Some(editor) = &self.editor {
            for session in &mut self.sessions {
                if session.profile.origin.is_none()
                    && let Some(draft) = editor
                        .drafts
                        .iter()
                        .find(|d| d.name == session.profile.name)
                {
                    session.profile.theme = draft.theme.clone();
                    session.profile.font = draft.font.clone();
                    session.profile.font_size = draft.font_size;
                }
            }
        } else if editing {
            self.sync_looks();
        }
    }

    fn sync_looks(&mut self) {
        for session in &mut self.sessions {
            let look = match &session.profile.origin {
                Some(host) => self
                    .store
                    .profiles
                    .iter()
                    .find(|p| p.name == *host)
                    .map(|p| (p.term_theme.clone(), p.term_font.clone(), p.term_font_size)),
                None => self
                    .store
                    .terminals
                    .iter()
                    .find(|p| p.name == session.profile.name)
                    .map(|p| (p.theme.clone(), p.font.clone(), p.font_size)),
            };
            if let Some((theme, font, size)) = look {
                session.profile.theme = theme;
                session.profile.font = font;
                session.profile.font_size = size;
            }
        }
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
                ui.label(
                    RichText::new(tf!("Ключ сервера «{title}» изменился", "The key of server \"{title}\" has changed"))
                        .color(pal.warn)
                        .strong(),
                );
                ui.label(
                    tr!("Сохранённый ключ этого сервера отличается от присланного. Так бывает после \
                     переустановки сервера, но так же выглядит перехват соединения."),
                );
            } else {
                ui.label(RichText::new(tf!("Новый сервер «{title}»", "New server \"{title}\"")).strong());
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
                session.status = tr!("подключение отменено").into();
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
            ui.label(RichText::new(tr!("Подключения консоли")).strong());
            ui.add_space(8.0);
            let limit = (ui.ctx().content_rect().height() - 200.0).max(200.0);
            ScrollArea::vertical()
                .id_salt("tror_profile_scroll")
                .max_height(limit)
                .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                            for (i, profile) in editor.drafts.iter().enumerate() {
                                let name = if profile.name.trim().is_empty() {
                                    tr!("без имени").to_owned()
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
                            if ui.add(Button::new(tr!("Добавить"))).clicked() {
                                editor.drafts.push(TermProfile {
                                    name: tf!("Сервер {}", "Server {}", editor.drafts.len() + 1),
                                    ..Default::default()
                                });
                                editor.index = editor.drafts.len() - 1;
                                editor.result = None;
                            }
                            if ui
                                .add_enabled(!editor.drafts.is_empty(), Button::new(tr!("Удалить")))
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
                            ui.label(RichText::new(tr!("Добавьте подключение.")).color(pal.subtext));
                            return;
                        };
                        egui::Grid::new("tror_term_form")
                            .num_columns(2)
                            .spacing([10.0, 7.0])
                            .show(ui, |ui| {
                                let wide = 320.0;
                                ui.label(tr!("Название"));
                                ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                                ui.end_row();
                                ui.label(tr!("Протокол"));
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
                                ui.label(tr!("Сервер"));
                                ui.horizontal(|ui| {
                                    ui.add(TextEdit::singleline(&mut p.host).hint_text("example.org").desired_width(wide - 70.0));
                                    port_field(ui, &mut p.port, 60.0);
                                });
                                ui.end_row();
                                ui.label(tr!("Пользователь"));
                                ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                                ui.end_row();
                                ui.label(tr!("Пароль"));
                                ui.horizontal(|ui| {
                                    ui.add(TextEdit::singleline(&mut p.password).password(!editor.reveal).desired_width(wide - 88.0));
                                    let label = if editor.reveal { tr!("Скрыть") } else { tr!("Показать") };
                                    if ui.add_sized([80.0, 22.0], Button::new(label)).clicked() {
                                        editor.reveal = !editor.reveal;
                                    }
                                });
                                ui.end_row();
                                if p.protocol != TermProtocol::Telnet {
                                    ui.label(tr!("Файл ключа"));
                                    ui.horizontal(|ui| {
                                        ui.add(TextEdit::singleline(&mut p.key_file).hint_text("RSA, ECDSA, Ed25519").desired_width(wide - 78.0));
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
                                            Ok(kind) => ui.label(RichText::new(tf!("ключ: {kind}", "key: {kind}")).small().color(pal.subtext)),
                                            Err(e) => ui.label(RichText::new(e).small().color(pal.warn)),
                                        };
                                        ui.end_row();
                                        ui.label(tr!("Парольная фраза"));
                                        ui.add(TextEdit::singleline(&mut p.passphrase).password(!editor.reveal).hint_text(tr!("если ключ защищён")).desired_width(wide));
                                        ui.end_row();
                                    }
                                }
                                if p.protocol == TermProtocol::Mosh {
                                    ui.label("mosh-server");
                                    ui.add(TextEdit::singleline(&mut p.mosh_server).hint_text("mosh-server").desired_width(wide));
                                    ui.end_row();
                                    ui.label(tr!("Порты UDP"));
                                    ui.add(TextEdit::singleline(&mut p.mosh_ports).hint_text("60000:61000").desired_width(wide));
                                    ui.end_row();
                                }
                                if p.protocol == TermProtocol::Ssh {
                                    ui.label(tr!("Команда при входе"));
                                    ui.add(TextEdit::singleline(&mut p.startup).hint_text(tr!("например: cd /var/www && ls")).desired_width(wide));
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
                                    } else if ui.add(Button::new(tr!("Проверить подключение"))).clicked() {
                                        test = true;
                                    }
                                    if let Some(result) = &editor.result {
                                        let (text, color) = match result {
                                            Ok(text) => (tf!("Работает: {text}", "Works: {text}"), Color32::from_rgb(0x3f, 0xc2, 0x6e)),
                                            Err(e) => (tf!("Ошибка: {e}", "Error: {e}"), pal.warn),
                                        };
                                        ui.label(RichText::new(text).small().color(color));
                                    }
                                });
                                ui.end_row();
                                appearance_rows(ui, pal, &mut p.theme, &mut p.font, &mut p.font_size, wide);
                            });
                        ui.add_space(4.0);
                        let note = match p.protocol {
                            TermProtocol::Ssh => tr!("Вход по паролю, по ключу (RSA, ECDSA, Ed25519, PuTTY .ppk) или через ssh-agent, если не заданы ни пароль, ни ключ."),
                            TermProtocol::Telnet => tr!("Telnet передаёт всё, включая пароль, без шифрования — используйте его для сетевого оборудования в своей сети. Имя и пароль, если указаны, вводятся сами в ответ на приглашение входа."),
                            TermProtocol::Local => "",
                            TermProtocol::Mosh => tr!("Mosh входит по SSH, запускает на сервере mosh-server и дальше работает по UDP: сеанс переживает обрывы, смену сети и сон ноутбука. На сервере должен быть установлен mosh, а порты UDP 60000–61000 — открыты."),
                        };
                        let color = if p.protocol == TermProtocol::Telnet { pal.warn } else { pal.subtext };
                        ui.label(RichText::new(note).small().color(color));
                    });
                });
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Сохранить"), true) {
                    save = true;
                }
                if dialog_button(ui, pal, tr!("Сохранить и подключиться"), false) {
                    save = true;
                    connect = true;
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
            let known = self.store.known_hosts.get(&profile.address()).cloned();
            editor.result = None;
            editor.probe = Some(probe(move || remote::test_terminal(&profile, known)));
        }
        if save
            && let Some(editor) = &mut self.editor
            && editor
                .drafts
                .get(editor.index)
                .is_some_and(|p| p.host.trim().is_empty() && !self.store.terminals.contains(p))
        {
            editor.result = Some(Err(tr!("Укажите сервер").into()));
            save = false;
        }
        if save {
            if let Some(mut editor) = self.editor.take() {
                let chosen = editor.drafts.get(editor.index).cloned();
                let known = &self.store.terminals;
                editor
                    .drafts
                    .retain(|p| !p.host.trim().is_empty() || known.contains(p));
                self.store.terminals = editor.drafts;
                if let Err(e) = self.store.save() {
                    crate::trace::popup(tf!("Подключения не сохранены: {e}", "Connections not saved: {e}"), false);
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

pub fn appearance_rows(
    ui: &mut Ui,
    pal: &Palette,
    theme: &mut String,
    font: &mut String,
    font_size: &mut f32,
    wide: f32,
) {
    ui.label(tr!("Цветовая схема"));
    ComboBox::from_id_salt("tror_term_theme")
        .width(wide)
        .selected_text(if theme.is_empty() {
            tr!("Как в редакторе")
        } else {
            terminal::SCHEMES
                .iter()
                .find(|s| s.name == theme.as_str())
                .map_or(theme.as_str(), |s| tr!(s.name))
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(theme, String::new(), tr!("Как в редакторе"));
            for scheme in terminal::SCHEMES {
                ui.selectable_value(theme, scheme.name.to_owned(), tr!(scheme.name));
            }
        });
    ui.end_row();
    ui.label("");
    let colors = terminal::colors(theme, pal, true);
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
    ui.label(tr!("Шрифт"));
    ui.horizontal(|ui| {
        let name = if font.trim().is_empty() {
            tr!("как в редакторе").to_owned()
        } else {
            std::path::Path::new(font.trim())
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| font.clone())
        };
        ui.label(RichText::new(name).color(pal.subtext));
        if ui.button(tr!("Выбрать…")).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter(tr!("Шрифты"), &["ttf", "otf", "ttc"])
                .pick_file()
        {
            *font = path.display().to_string();
        }
        if !font.is_empty() && ui.button(tr!("Сбросить")).clicked() {
            font.clear();
        }
    });
    ui.end_row();
    ui.label(tr!("Размер шрифта"));
    ui.horizontal(|ui| {
        let mut custom = *font_size > 0.0;
        if ui.checkbox(&mut custom, tr!("свой")).changed() {
            *font_size = if custom { 14.0 } else { 0.0 };
        }
        if custom {
            crate::widgets::stepper(ui, pal, "term_font", font_size, (8.0, 28.0), 1.0, "px");
        }
    });
    ui.end_row();
}

fn layout(rect: Rect, split: Split, count: usize) -> Vec<Rect> {
    let gap = 2.0;
    let count = count.max(1);
    match split {
        Split::Single => vec![rect],
        Split::Columns => {
            let w = (rect.width() - gap * (count as f32 - 1.0)) / count as f32;
            (0..count)
                .map(|i| {
                    Rect::from_min_size(
                        rect.min + egui::vec2(i as f32 * (w + gap), 0.0),
                        egui::vec2(w, rect.height()),
                    )
                })
                .collect()
        }
        Split::Rows => {
            let h = (rect.height() - gap * (count as f32 - 1.0)) / count as f32;
            (0..count)
                .map(|i| {
                    Rect::from_min_size(
                        rect.min + egui::vec2(0.0, i as f32 * (h + gap)),
                        egui::vec2(rect.width(), h),
                    )
                })
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
                    let width = if count == 3 && r == 1 {
                        rect.width()
                    } else {
                        w
                    };
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
