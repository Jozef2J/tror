use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Button, CentralPanel, Color32, FontId, Frame, Key, Layout, Margin, MenuBar, Modal,
    Modifiers, Panel, PopupCloseBehavior, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui,
    ViewportCommand,
    containers::menu::{MenuButton, MenuConfig},
};
use egui_stack::{DockArea, DockState, TabViewer};
use encoding_rs::Encoding;

use crate::complete::Completion;
use crate::console::ConsolePanel;
use crate::db::DbPanel;
use crate::diff::DiffPanel;
use crate::dock::{self, Pane};
use crate::document::{Document, Eol};
use crate::editor;
use crate::files::{ProjectEvent, ProjectPanel};
use crate::highlight::Syntaxes;
use crate::prefs::{self, Look, Settings};
use crate::preview::{self, Preview};
use crate::remote_ui::{PanelEvent, RemotePanel};
use crate::search::{Pending, SearchState};
use crate::theme::{self, Palette};

static VIEW_REQUEST: std::sync::Mutex<(Option<bool>, Option<bool>)> =
    std::sync::Mutex::new((None, None));

pub(crate) fn request_view(remote: Option<bool>, project: Option<bool>) {
    if let Ok(mut request) = VIEW_REQUEST.lock() {
        if remote.is_some() {
            request.0 = remote;
        }
        if project.is_some() {
            request.1 = project;
        }
    }
}

pub(crate) use crate::widgets::port_field;

const SETTINGS_KEY: &str = "tror";
const DOCK_KEY: &str = "tror_dock";
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

enum Action {
    New,
    Open,
    OpenFolder,
    AddHosts,
    OpenPath(PathBuf),
    Save,
    SaveAs,
    SaveAll,
    Activate(usize),
    CloseTab(usize),
    CloseAll,
    Reopen,
    TabCmd(u64, TabCmd),
    Quit,
    Find,
    Replace,
    FindNext,
    FindPrev,
    CloseSearch,
    GotoLine,
    ToggleTheme,
    ToggleWrap,
    Toggle(Pane),
    Show(Pane),
    ResetLayout,
    Preview,
    Compare,
    CheckUpdates,
    FindFiles,
    MacroRecord,
    MacroPlay,
    Fold(u8),
    Goto(u64, usize),
    Plugin(usize),
    Detach(Pane),
    Attach(Pane),
    PluginsFolder,
    PluginsReload,
    NextTab,
    PrevTab,
}

#[derive(Default)]
struct Updater {
    rx: Option<std::sync::mpsc::Receiver<crate::update::Event>>,
    found: Option<crate::update::Release>,
    manual: bool,
    busy: bool,
    swapped: bool,
    setup: Option<std::path::PathBuf>,
    error: String,
}

#[derive(Clone, Copy, PartialEq)]
enum TabCmd {
    Close,
    CloseOthers,
    CloseLeft,
    CloseRight,
    Save,
    SaveAs,
    Rename,
    Color(Option<u8>),
    Pin,
    NewGroup(egui_stack::Split),
    OtherGroup,
    Reveal,
}

pub const TAB_COLORS: [(&str, u32); 6] = [
    ("Жёлтый", 0xe0b84c),
    ("Зелёный", 0x5fb865),
    ("Синий", 0x4f8fe6),
    ("Оранжевый", 0xe8873a),
    ("Розовый", 0xd9609b),
    ("Фиолетовый", 0x9a6fe0),
];

fn tab_color(index: u8) -> Color32 {
    let (_, hex) = TAB_COLORS[(index as usize).min(TAB_COLORS.len() - 1)];
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
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
    let button = if primary {
        crate::widgets::primary(text, pal)
    } else {
        crate::widgets::secondary(text, pal)
    };
    ui.add(button.min_size(egui::vec2(0.0, 28.0))).clicked()
}

pub(crate) fn window_header(ui: &mut Ui, pal: &Palette, title: &str, closable: bool) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(15.5).color(pal.text));
        if closable {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = icon_button(ui, pal, Icon::Close).clicked();
            });
        }
    });
    ui.add_space(4.0);
    close
}

pub(crate) fn dialog(id: impl std::hash::Hash + std::fmt::Debug, pal: &Palette) -> Modal {
    Modal::new(egui::Id::unique(id)).frame(crate::widgets::window_frame(pal))
}

#[derive(Clone, Copy)]
pub(crate) enum Icon {
    Up,
    Down,
    Close,
}

pub(crate) fn icon_button(ui: &mut Ui, pal: &Palette, icon: Icon) -> egui::Response {
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

#[derive(Default)]
struct Perf {
    frames: u32,
    total: std::time::Duration,
    worst: std::time::Duration,
    since: Option<std::time::Instant>,
    first: bool,
}

impl Perf {
    fn record(&mut self, frame: std::time::Duration) {
        if !std::mem::replace(&mut self.first, true)
            && let Some(started) = crate::STARTED.get()
        {
            eprintln!(
                "первый кадр: {:.0} мс от запуска",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
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
            *self = Self {
                first: true,
                ..Self::default()
            };
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
    settings: Settings,
    search: SearchState,
    complete: Completion,
    remotes: Vec<RemotePanel>,
    project: ProjectPanel,
    log: Vec<(String, crate::trace::Kind)>,
    console: ConsolePanel,
    term_fonts: Option<Vec<PathBuf>>,
    popups: Vec<(String, bool, f64)>,
    closed: Vec<(Option<PathBuf>, Option<(String, String)>)>,
    close_queue: std::collections::VecDeque<u64>,
    rename: Option<(u64, String)>,
    tab_orders: Vec<Vec<u64>>,
    dock: DockState<Pane>,
    diff: DiffPanel,
    db: DbPanel,
    preview: Preview,
    plugins: crate::plugins::Plugins,
    console_shown: bool,
    settings_shown: bool,
    logo: Option<egui::TextureHandle>,
    ctx: egui::Context,
    checked: (u64, bool),
    goto: Option<String>,
    find_files: crate::findfiles::FindFiles,
    update: Updater,
    macros: editor::Macro,
    recovery: crate::recover::Recovery,
    confirm: Option<Confirm>,
    allow_close: bool,
    focus_editor: bool,
    lang_filter: String,
    toast: Option<(String, f64)>,
    applied: Option<Look>,
    capturing: Option<usize>,
}

impl TrorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();
        settings.migrate();
        let mut dock = cc
            .storage
            .and_then(|s| eframe::get_value::<DockState<Pane>>(s, DOCK_KEY))
            .unwrap_or_else(dock::new);
        dock::translate(&mut dock);
        dock::hide(&mut dock, Pane::Preview);
        settings.detached.retain(|p| *p != Pane::Preview);
        crate::preview::forget_webview();
        crate::trace::install();
        let hosts = settings
            .hosts
            .max(2)
            .max(
                dock.iter_all_tabs()
                    .filter_map(|(_, tab)| tab.host_index())
                    .chain(settings.detached.iter().filter_map(|p| p.host_index()))
                    .map(|i| i + 1)
                    .max()
                    .unwrap_or(0),
            )
            .min(16);
        let project = ProjectPanel::new(settings.project.clone());
        let mut app = Self {
            perf: std::env::var_os("TROR_PERF").map(|_| Perf::default()),
            title: String::new(),
            syntaxes: Syntaxes::new(),
            docs: Vec::new(),
            active: 0,
            next_id: 1,
            settings,
            search: SearchState::default(),
            complete: Completion::default(),
            remotes: (0..hosts).map(|i| RemotePanel::new(i as u8)).collect(),
            project,
            log: Vec::new(),
            console: ConsolePanel::new(),
            term_fonts: None,
            popups: Vec::new(),
            closed: Vec::new(),
            close_queue: std::collections::VecDeque::new(),
            rename: None,
            tab_orders: Vec::new(),
            dock,
            diff: DiffPanel::default(),
            db: DbPanel::default(),
            preview: Preview::default(),
            plugins: crate::plugins::Plugins::new(
                eframe::storage_dir("Tror").map(|dir| dir.join("plugins")),
            ),
            console_shown: false,
            settings_shown: false,
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
            find_files: Default::default(),
            update: Default::default(),
            macros: Default::default(),
            recovery: Default::default(),
            confirm: None,
            allow_close: false,
            focus_editor: true,
            lang_filter: String::new(),
            toast: None,
            applied: None,
            capturing: None,
        };
        let open = std::mem::take(&mut app.settings.open);
        let active = app.settings.active;
        let mut top = 1;
        let restore = app.settings.restore;
        for (id, path) in open.iter().filter(|(_, p)| restore && p.is_file()) {
            app.next_id = *id;
            app.open_path(path, 0.0);
            top = top.max(id + 1);
        }
        let pinned = std::mem::take(&mut app.settings.pinned);
        for doc in &mut app.docs {
            doc.pinned = doc.path.as_ref().is_some_and(|p| pinned.contains(p));
        }
        crate::widgets::set_columns(app.settings.columns);
        app.recover();
        crate::update::cleanup();
        if app.settings.updates {
            app.update.rx = Some(crate::update::check());
        }
        app.next_id = top;
        if active < app.docs.len() {
            app.active = active;
        }
        for arg in std::env::args_os().skip(1) {
            let path = PathBuf::from(arg);
            let path = std::path::absolute(&path).unwrap_or(path);
            if path.is_dir() {
                app.project.set_root(path);
                dock::show(&mut app.dock, Pane::Project, &app.settings.floats);
            } else {
                app.open_path(&path, 0.0);
            }
        }
        app
    }

    fn run_plugin(&mut self, index: usize, ctx: &egui::Context) {
        use crate::plugins::{Context, Input};
        let now = ctx.input(|i| i.time);
        let Some(plugin) = self.plugins.list.get(index).cloned() else {
            return;
        };
        let doc = self.docs.get(self.active);
        if doc.is_none() && plugin.input != Input::None {
            self.notify(
                format!("Плагину «{}» нужен открытый файл", plugin.name),
                now,
            );
            return;
        }
        let mut context = Context {
            file: String::new(),
            dir: None,
            line: 1,
            column: 1,
            syntax: String::new(),
        };
        let mut input = String::new();
        let mut range = 0..0;
        let (mut id, mut version) = (0, 0);
        if let Some(doc) = doc {
            let (line, column) = doc.cursor_line_col();
            context = Context {
                file: doc.location().unwrap_or_default(),
                dir: doc
                    .path
                    .as_deref()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf()),
                line,
                column,
                syntax: doc.syntax.clone(),
            };
            range = match plugin.input {
                Input::Selection if doc.has_selection() => doc.selection(),
                Input::None => doc.selection(),
                _ => 0..doc.len(),
            };
            if plugin.input != Input::None {
                input = doc.rope.slice(range.clone()).to_string();
            }
            id = doc.id;
            version = doc.version;
        }
        let wake = ctx.clone();
        self.plugins
            .start(plugin, context, input, (id, version), range, move || {
                wake.request_repaint()
            });
    }

    fn plugin_results(&mut self, now: f64) {
        use crate::plugins::Output;
        for done in self.plugins.poll() {
            let name = done.plugin.name.clone();
            for line in done.stderr.lines() {
                crate::trace::line(format!("[plugin {name}] {line}"), done.result.is_err());
            }
            let text = match done.result {
                Ok(text) => text,
                Err(e) => {
                    crate::trace::line(format!("[plugin {name}] {e}"), true);
                    self.notify(format!("Плагин «{name}»: {e}"), now);
                    continue;
                }
            };
            let doc = self.docs.iter_mut().find(|d| d.id == done.doc);
            match (done.plugin.output, doc) {
                (Output::None, _) => {}
                (Output::Message, _) => {
                    for line in text.lines() {
                        crate::trace::line(format!("[plugin {name}] {line}"), false);
                    }
                    let first = text.lines().next().unwrap_or("готово").to_owned();
                    self.notify(format!("{name}: {first}"), now);
                }
                (Output::NewTab, _) => {
                    self.new_doc();
                    if let Some(doc) = self.docs.last_mut() {
                        doc.insert_text(&text);
                        doc.move_to(0, false);
                    }
                }
                (Output::Replace | Output::Insert, Some(doc)) if doc.version == done.version => {
                    let source = doc.rope.slice(done.range.clone()).to_string();
                    let text = match text.strip_suffix('\n') {
                        Some(cut) if !source.ends_with('\n') => {
                            cut.strip_suffix('\r').unwrap_or(cut)
                        }
                        _ => &text,
                    };
                    if done.plugin.output == Output::Insert {
                        doc.insert_text(text);
                    } else if source != text {
                        doc.replace(done.range, text, crate::document::EditKind::Other);
                    }
                }
                _ => self.notify(
                    format!("Плагин «{name}»: файл изменился или закрыт, результат не применён"),
                    now,
                ),
            }
        }
    }

    fn palette(&self) -> Palette {
        self.settings.look.palette()
    }

    fn apply_look(&mut self, ctx: &egui::Context) {
        theme::commit_term_fonts();
        let wanted_fonts = self.console.fonts();
        let fonts_changed = self.term_fonts.as_ref() != Some(&wanted_fonts);
        if self.applied.as_ref() == Some(&self.settings.look) && !fonts_changed {
            return;
        }
        let look = self.settings.look.clone();
        let old = self.applied.take();
        let fonts = |l: &Look| (l.ui_font.clone(), l.code_font.clone(), l.tab_width);
        if (old.as_ref().map(fonts) != Some(fonts(&look)) || fonts_changed)
            && let Err(e) = theme::install_fonts(
                ctx,
                look.ui_font.as_deref(),
                look.code_font.as_deref(),
                &wanted_fonts,
                look.tab_width,
            )
        {
            self.notify(format!("Шрифт не загружен: {e}"), ctx.input(|i| i.time));
        }
        self.term_fonts = Some(wanted_fonts);
        theme::apply(ctx, &look.palette(), look.dark, look.radius, look.ui_size);
        let code = |l: &Look| {
            let (scheme, colors) = l.syntax();
            (scheme.to_owned(), colors.clone())
        };
        if old.as_ref().map(code) != Some(code(&look)) {
            let (scheme, colors) = look.syntax();
            self.syntaxes.set_theme(scheme, colors);
        }
        self.applied = Some(look);
    }

    fn update_ui(&mut self, ctx: &egui::Context, pal: &Palette, now: f64) {
        use crate::update::{Event, Kind};
        let events: Vec<Event> = self
            .update
            .rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        if self.update.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        for event in events {
            self.update.rx = None;
            self.update.busy = false;
            match event {
                Event::Found(release) => self.update.found = Some(release),
                Event::Current => {
                    if std::mem::take(&mut self.update.manual) {
                        self.notify("Установлена последняя версия", now);
                    }
                }
                Event::Failed(e) => {
                    if std::mem::take(&mut self.update.manual) || self.update.found.is_some() {
                        self.notify(format!("Обновление не удалось: {e}"), now);
                    }
                }
                Event::Ready(path, Kind::Installer) => {
                    self.update.setup = Some(path);
                    self.update.found = None;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Event::Ready(path, _) => {
                    let version = self.update.found.as_ref().map(|r| r.version.clone()).unwrap_or_default();
                    match crate::update::apply(&path, &version) {
                        Ok(()) => self.update.swapped = true,
                        Err(e) => self.notify(format!("Обновление не удалось: {e}"), now),
                    }
                }
            }
        }
        let Some(release) = self.update.found.clone() else {
            return;
        };
        let kind = crate::update::kind();
        let mut close = false;
        let notes = crate::update::notes(&release.notes);
        egui::Window::new("Доступно обновление")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(crate::widgets::window_frame(pal))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -40.0))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new("Доступно обновление").small().color(pal.accent));
                        ui.label(RichText::new(format!("Tror {}", release.version)).size(17.0).color(pal.text));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.label(RichText::new(format!("у вас {}", env!("CARGO_PKG_VERSION"))).small().color(pal.faint));
                    });
                });
                if !notes.is_empty() {
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 5.0;
                        for (i, (bullet, line)) in notes.iter().enumerate() {
                            if *bullet {
                                ui.horizontal_top(|ui| {
                                    ui.label(RichText::new("•").color(pal.accent));
                                    ui.add(egui::Label::new(RichText::new(line).color(pal.subtext)).wrap());
                                });
                            } else if i == 0 {
                                ui.add(egui::Label::new(RichText::new(line).color(pal.text)).wrap());
                            } else {
                                ui.add(egui::Label::new(RichText::new(line).color(pal.subtext)).wrap());
                            }
                        }
                    });
                    ui.separator();
                }
                if self.update.swapped {
                    ui.horizontal(|ui| {
                        if ui.add(crate::widgets::primary("Перезапустить сейчас", pal)).clicked() {
                            match crate::update::restart() {
                                Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
                                Err(e) => self.update.error = e,
                            }
                        }
                        close = ui.add(crate::widgets::secondary("Позже", pal)).clicked();
                    });
                } else if self.update.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Скачиваю и проверяю файл…").color(pal.subtext));
                    });
                } else {
                    let can_install = kind != Kind::Page && release.file.is_some() && release.sums.is_some();
                    ui.horizontal(|ui| {
                        if can_install && ui.add(crate::widgets::primary("Скачать и установить", pal)).clicked() {
                            self.update.busy = true;
                            self.update.rx = Some(crate::update::download(release.clone()));
                        }
                        if ui.add(crate::widgets::secondary("Открыть страницу выпуска", pal)).clicked() {
                            let _ = preview::open_url(crate::update::PAGE);
                        }
                        close = ui.add(crate::widgets::secondary("Позже", pal)).clicked();
                    });
                }
                if !self.update.error.is_empty() {
                    ui.label(RichText::new(&self.update.error).color(pal.warn));
                }
            });
        if close {
            self.update.found = None;
        }
    }

    fn recover(&mut self) {
        let Some(dir) = crate::recover::dir() else {
            return;
        };
        let found = crate::recover::pending(&dir);
        if found.is_empty() {
            return;
        }
        let count = found.len();
        for (entry, text) in found {
            let open = entry
                .path
                .as_ref()
                .and_then(|p| self.docs.iter().position(|d| d.path.as_ref() == Some(p)));
            let index = match open {
                Some(index) => index,
                None => {
                    if let Some(path) = entry.path.as_ref().filter(|p| p.is_file()) {
                        self.open_path(path, 0.0);
                    }
                    match entry
                        .path
                        .as_ref()
                        .and_then(|p| self.docs.iter().position(|d| d.path.as_ref() == Some(p)))
                    {
                        Some(index) => index,
                        None => {
                            let mut doc = Document::empty(self.next_id, entry.untitled.max(1));
                            self.next_id += 1;
                            doc.path = entry.path.clone();
                            doc.remote = entry.remote.clone();
                            let name = entry
                                .path
                                .clone()
                                .or_else(|| entry.remote.as_ref().map(|r| PathBuf::from(&r.1)));
                            doc.syntax = self.syntaxes.detect(name.as_deref(), &text);
                            self.docs.push(doc);
                            self.docs.len() - 1
                        }
                    }
                }
            };
            self.docs[index].set_text(&text);
            self.active = index;
        }
        crate::recover::clear(&dir);
        let _ = self.recovery.snapshot(&dir, &self.docs);
        self.toast = Some((
            format!("Восстановлены несохранённые правки, вкладок: {count}"),
            0.0,
        ));
    }

    fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some((text.into(), now));
    }

    fn new_doc(&mut self) {
        let taken: Vec<u32> = self
            .docs
            .iter()
            .filter(|d| d.path.is_none() && d.remote.is_none())
            .map(|d| d.untitled)
            .collect();
        let number = (1..).find(|n| !taken.contains(n)).unwrap_or(1);
        let doc = Document::empty(self.next_id, number);
        self.next_id += 1;
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

    fn pane_dialogs(&mut self, pane: Pane, ctx: &egui::Context, pal: &Palette) {
        match pane {
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) => {
                let Some(index) = pane.host_index() else {
                    return;
                };
                if let Some(panel) = self.remotes.get_mut(index) {
                    let events = panel.dialogs(ctx, pal);
                    self.remote_events(ctx, events);
                }
            }
            Pane::Console => self.console.dialogs(ctx, pal),
            Pane::Project => self.project.dialogs(ctx, pal),
            Pane::Db => self.db.dialogs(ctx, pal, &mut self.remotes[0].store),
            _ => {}
        }
    }

    fn hosts_panes(&self) -> Vec<Pane> {
        (0..self.remotes.len()).map(Pane::hosts).collect()
    }

    fn host_shown(&self, index: usize) -> bool {
        let pane = Pane::hosts(index);
        dock::has(&self.dock, pane) || self.settings.detached.contains(&pane)
    }

    fn trim_hosts(&mut self) {
        while self.remotes.len() > 1 {
            let last = self.remotes.len() - 1;
            if self.host_shown(last) || self.remotes[last].in_use() {
                break;
            }
            self.remotes.pop();
        }
        self.settings.hosts = self.remotes.len();
    }

    fn transfers(&mut self) {
        let count = self.remotes.len();
        for from in 0..count {
            if let Some((target, files)) = self.remotes[from].outgoing.take()
                && let Some(dir) = self
                    .remotes
                    .get(target as usize)
                    .filter(|p| p.peer_label().is_some())
                    .map(|p| p.cwd().to_owned())
            {
                self.remotes[from].relay(files, target, &dir);
            }
            if let Some(dragged) = self.remotes[from].incoming.take() {
                let dir = self.remotes[from].cwd().to_owned();
                if let Some(source) = self.remotes.get_mut(dragged.panel as usize) {
                    source.relay(dragged.paths, from as u8, &dir);
                }
            }
        }
    }

    fn remote_events(&mut self, ctx: &egui::Context, events: Vec<PanelEvent>) {
        let now = ctx.input(|i| i.time);
        for event in events {
            match event {
                PanelEvent::Relay { target, path, data } => {
                    match self
                        .remotes
                        .get_mut(target as usize)
                        .filter(|p| p.peer_label().is_some())
                    {
                        Some(panel) => panel.receive(path, data),
                        None => self.notify("Передача отменена: второй сервер отключён", now),
                    }
                }
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
            && !self.remotes.iter().any(|r| r.modal_open())
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
            let index = self
                .remotes
                .iter()
                .position(|r| r.serves(&profile))
                .unwrap_or(0);
            let panel = &mut self.remotes[index];
            let result = panel.upload(&ctx, &profile, &path, doc.encoded(), (doc.id, doc.version));
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
                self.preview.touch();
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
                self.preview.touch();
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
        let doc = self.docs.remove(index);
        if doc.path.is_some() || doc.remote.is_some() {
            let entry = (doc.path.clone(), doc.remote.clone());
            self.closed.retain(|c| *c != entry);
            self.closed.push(entry);
            if self.closed.len() > 30 {
                self.closed.remove(0);
            }
        }
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
            Action::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.project.set_root(dir);
                    if !self.settings.detached.contains(&Pane::Project) {
                        dock::show(&mut self.dock, Pane::Project, &self.settings.floats);
                    }
                }
            }
            Action::AddHosts => {
                let free = (1..self.remotes.len())
                    .find(|i| !self.host_shown(*i) && !self.remotes[*i].in_use());
                let index = match free {
                    Some(index) => Some(index),
                    None if self.remotes.len() < 16 => {
                        self.remotes
                            .push(RemotePanel::new(self.remotes.len() as u8));
                        Some(self.remotes.len() - 1)
                    }
                    None => None,
                };
                if let Some(index) = index {
                    self.settings.hosts = self.remotes.len();
                    dock::show(&mut self.dock, Pane::hosts(index), &self.settings.floats);
                }
            }
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
            Action::TabCmd(id, cmd) => self.tab_cmd(id, cmd, now),
            Action::Reopen => {
                if let Some((path, remote)) = self.closed.pop() {
                    match remote {
                        Some((profile, file)) => {
                            let index = self
                                .remotes
                                .iter()
                                .position(|r| r.serves(&profile))
                                .or_else(|| self.remotes.iter().position(|r| !r.in_use()))
                                .unwrap_or(0);
                            if let Err(e) = self.remotes[index].reopen(ctx, &profile, &file) {
                                self.notify(format!("Не удалось открыть {file}: {e}"), now);
                            } else if !self.host_shown(index) {
                                dock::show(
                                    &mut self.dock,
                                    Pane::hosts(index),
                                    &self.settings.floats,
                                );
                            }
                        }
                        None => {
                            if let Some(path) = path {
                                self.open_path(&path, now);
                            }
                        }
                    }
                } else {
                    self.notify("Закрытых вкладок нет", now);
                }
            }
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
            Action::Fold(kind) => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    match kind {
                        0 => {
                            doc.fold_here();
                        }
                        1 => {
                            doc.unfold_here();
                        }
                        2 => doc.fold_all(),
                        _ => doc.unfold_all(),
                    }
                    doc.reveal = true;
                }
                self.focus_editor = true;
            }
            Action::ToggleTheme => self.settings.look.dark = !self.settings.look.dark,
            Action::ToggleWrap => self.settings.wrap = !self.settings.wrap,
            Action::Toggle(pane) => {
                if self.settings.detached.contains(&pane) {
                    self.settings.detached.retain(|p| *p != pane);
                } else {
                    dock::toggle(&mut self.dock, pane, &self.settings.floats);
                }
            }
            Action::Detach(pane) => {
                dock::hide(&mut self.dock, pane);
                if !self.settings.detached.contains(&pane) {
                    self.settings.detached.push(pane);
                }
            }
            Action::Attach(pane) => {
                self.settings.detached.retain(|p| *p != pane);
                if !matches!(pane, Pane::Doc(_)) {
                    dock::show(&mut self.dock, pane, &self.settings.floats);
                }
            }
            Action::Show(pane) => {
                if !self.settings.detached.contains(&pane) {
                    dock::show(&mut self.dock, pane, &self.settings.floats);
                }
            }
            Action::ResetLayout => {
                self.dock = dock::new();
                self.focus_editor = true;
            }
            Action::Preview => {
                let url = match self.docs.get(self.active) {
                    None => Err("Нет открытого файла".to_owned()),
                    Some(doc) => match (&doc.remote, &doc.path) {
                        (Some((profile, path)), _) => self.remotes[0].site_url(profile, path).ok_or(
                            "В профиле хоста не указан адрес сайта — добавьте его в настройках профиля"
                                .to_owned(),
                        ),
                        (None, Some(path)) => self.preview.url(path),
                        _ => Err("Сначала сохраните файл на диск".to_owned()),
                    },
                };
                match url {
                    Ok(url) => match preview::open_url(&url) {
                        Ok(()) => self.notify(format!("Открыто в браузере: {url}"), now),
                        Err(e) => self.notify(format!("Предпросмотр: {e}"), now),
                    },
                    Err(e) => self.notify(format!("Предпросмотр: {e}"), now),
                }
            }
            Action::CheckUpdates => {
                self.update.manual = true;
                self.update.rx = Some(crate::update::check());
                self.notify("Проверяю обновления…", now);
            }
            Action::FindFiles => self.find_files.show(),
            Action::MacroRecord => {
                if self.macros.recording {
                    self.macros.recording = false;
                    let count = self.macros.events.len();
                    self.notify(format!("Макрос записан, действий: {count}"), now);
                } else {
                    self.macros.events.clear();
                    self.macros.recording = true;
                    self.notify("Идёт запись макроса — нажмите сочетание ещё раз, чтобы остановить", now);
                }
                self.focus_editor = true;
            }
            Action::MacroPlay => {
                if self.macros.recording {
                    self.notify("Сначала остановите запись макроса", now);
                } else if self.macros.events.is_empty() {
                    self.notify("Макрос ещё не записан", now);
                } else {
                    self.macros.play += 1;
                    self.focus_editor = true;
                }
            }
            Action::Compare => {
                if let Some(doc) = self.docs.get(self.active) {
                    self.diff.left = Some(doc.id);
                    if self.diff.right.is_none() || self.diff.right == self.diff.left {
                        self.diff.right = self
                            .docs
                            .iter()
                            .cycle()
                            .skip(self.active + 1)
                            .take(self.docs.len().saturating_sub(1))
                            .next()
                            .map(|d| d.id);
                    }
                }
                dock::show(&mut self.dock, Pane::Diff, &self.settings.floats);
            }
            Action::Plugin(index) => self.run_plugin(index, ctx),
            Action::PluginsFolder => {
                let opened = self
                    .plugins
                    .prepare()
                    .and_then(|dir| preview::open_folder(&dir).map(|()| dir));
                match opened {
                    Ok(dir) => self.notify(format!("Папка плагинов: {}", dir.display()), now),
                    Err(e) => self.notify(format!("Папка плагинов: {e}"), now),
                }
            }
            Action::PluginsReload => {
                self.plugins.reload();
                let errors = self.plugins.errors.clone();
                for error in &errors {
                    crate::trace::line(format!("[plugin] {error}"), true);
                }
                self.notify(
                    if errors.is_empty() {
                        format!("Плагинов загружено: {}", self.plugins.list.len())
                    } else {
                        format!(
                            "Плагинов загружено: {}, с ошибками: {} (подробности в журнале)",
                            self.plugins.list.len(),
                            errors.len()
                        )
                    },
                    now,
                );
            }
            Action::Goto(id, line) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    self.active = i;
                    self.focus_editor = true;
                    self.docs[i].goto_line(line);
                }
            }
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
        if self.capturing.is_some() {
            return;
        }
        let keys = crate::keys::Keys::new(&self.settings.keys);
        for (index, shortcut) in keys.bound() {
            let (id, _, _, editing) = crate::keys::COMMANDS[*index];
            if self.console.focused() && editing {
                continue;
            }
            if !ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
                continue;
            }
            if shortcut.modifiers.alt {
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
            }
            actions.push(match id {
                "new" => Action::New,
                "open" => Action::Open,
                "open_folder" => Action::OpenFolder,
                "project" => Action::Toggle(Pane::Project),
                "save" => Action::Save,
                "save_as" => Action::SaveAs,
                "save_all" => Action::SaveAll,
                "close_tab" => Action::CloseTab(self.active),
                "reopen" => Action::Reopen,
                "next_tab" => Action::NextTab,
                "prev_tab" => Action::PrevTab,
                "find" => Action::Find,
                "replace" => Action::Replace,
                "find_next" => Action::FindNext,
                "find_prev" => Action::FindPrev,
                "goto" => Action::GotoLine,
                "fold" => Action::Fold(0),
                "unfold" => Action::Fold(1),
                "fold_all" => Action::Fold(2),
                "unfold_all" => Action::Fold(3),
                "wrap" => Action::ToggleWrap,
                "theme" => Action::ToggleTheme,
                "settings" => Action::Show(Pane::Settings),
                "hosts" => Action::Toggle(Pane::Hosts),
                "console" => Action::Toggle(Pane::Console),
                "log" => Action::Toggle(Pane::Log),
                "server_search" => Action::Toggle(Pane::ServerSearch),
                "db" => Action::Toggle(Pane::Db),
                "compare" => Action::Compare,
                "find_files" => Action::FindFiles,
                "macro_record" => Action::MacroRecord,
                "macro_play" => Action::MacroPlay,
                _ => Action::Preview,
            });
        }
        for (index, plugin) in self.plugins.list.iter().enumerate() {
            if let Some(shortcut) = crate::keys::parse(&plugin.shortcut)
                && crate::keys::usable(&shortcut)
                && ctx.input_mut(|i| i.consume_shortcut(&shortcut))
            {
                actions.push(Action::Plugin(index));
            }
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

    fn menu(&mut self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let has_doc = !self.docs.is_empty();
        let item = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool| {
            ui.add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
                .clicked()
        };
        let keys = crate::keys::Keys::new(&self.settings.keys);
        let key = |id: &str| keys.label(id);
        MenuBar::new().ui(ui, |ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(18.0, 18.0)));
                ui.add_space(2.0);
            }
            ui.menu_button("Файл", |ui| {
                ui.set_min_width(240.0);
                if item(ui, "Новый", &key("new"), true) {
                    actions.push(Action::New);
                }
                if item(ui, "Открыть…", &key("open"), true) {
                    actions.push(Action::Open);
                }
                if item(ui, "Открыть папку…", &key("open_folder"), true) {
                    actions.push(Action::OpenFolder);
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
                if !self.settings.recent_projects.is_empty() {
                    ui.menu_button("Недавние папки", |ui| {
                        for path in self.settings.recent_projects.clone() {
                            if ui.button(path.display().to_string()).clicked() && path.is_dir() {
                                self.project.set_root(path);
                                actions.push(Action::Show(Pane::Project));
                            }
                        }
                    });
                }
                ui.separator();
                if item(ui, "Сохранить", &key("save"), has_doc) {
                    actions.push(Action::Save);
                }
                if item(ui, "Сохранить как…", &key("save_as"), has_doc) {
                    actions.push(Action::SaveAs);
                }
                if item(ui, "Сохранить все", "", has_doc) {
                    actions.push(Action::SaveAll);
                }
                ui.separator();
                if item(ui, "Закрыть вкладку", &key("close_tab"), has_doc) {
                    actions.push(Action::CloseTab(self.active));
                }
                if item(ui, "Закрыть все", "", has_doc) {
                    actions.push(Action::CloseAll);
                }
                if item(
                    ui,
                    "Открыть закрытую вкладку",
                    &key("reopen"),
                    !self.closed.is_empty(),
                ) {
                    actions.push(Action::Reopen);
                }
                ui.separator();
                if item(ui, "Настройки", &key("settings"), true) {
                    actions.push(Action::Show(Pane::Settings));
                }
                ui.separator();
                if item(ui, "Выход", "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button("Поиск", |ui| {
                ui.set_min_width(240.0);
                if item(ui, "Найти…", &key("find"), has_doc) {
                    actions.push(Action::Find);
                }
                if item(ui, "Заменить…", &key("replace"), has_doc) {
                    actions.push(Action::Replace);
                }
                if item(ui, "Найти далее", "F3", has_doc) {
                    actions.push(Action::FindNext);
                }
                if item(ui, "Найти ранее", "Shift+F3", has_doc) {
                    actions.push(Action::FindPrev);
                }
                ui.separator();
                if item(ui, "Перейти к строке…", &key("goto"), has_doc) {
                    actions.push(Action::GotoLine);
                }
                ui.separator();
                for (kind, label, id) in [
                    (0, "Свернуть блок", "fold"),
                    (1, "Развернуть блок", "unfold"),
                    (2, "Свернуть всё", "fold_all"),
                    (3, "Развернуть всё", "unfold_all"),
                ] {
                    if item(ui, label, &key(id), has_doc) {
                        actions.push(Action::Fold(kind));
                    }
                }
                ui.separator();
                if item(ui, "Найти в файлах проекта…", &key("find_files"), true) {
                    actions.push(Action::FindFiles);
                }
            });
            ui.menu_button("Вид", |ui| {
                ui.set_min_width(260.0);
                let theme = if self.settings.look.dark {
                    "Светлая тема"
                } else {
                    "Тёмная тема"
                };
                if item(ui, theme, "", true) {
                    actions.push(Action::ToggleTheme);
                }
                let wrap = if self.settings.wrap {
                    "Выключить перенос строк"
                } else {
                    "Перенос строк"
                };
                if item(ui, wrap, &key("wrap"), true) {
                    actions.push(Action::ToggleWrap);
                }
                ui.separator();
                let extra: Vec<(Pane, String, String)> = self
                    .hosts_panes()
                    .into_iter()
                    .skip(1)
                    .filter(|pane| {
                        let i = pane.host_index().unwrap_or(0);
                        self.host_shown(i) || self.remotes[i].in_use()
                    })
                    .map(|pane| {
                        let n = pane.host_index().unwrap_or(0) + 1;
                        (
                            pane,
                            format!("Подключение {n} (SFTP/FTP)"),
                            format!("Скрыть подключение {n}"),
                        )
                    })
                    .collect();
                for (pane, show, hide) in &extra {
                    let label =
                        if dock::has(&self.dock, *pane) || self.settings.detached.contains(pane) {
                            hide
                        } else {
                            show
                        };
                    if item(ui, label, "", true) {
                        actions.push(Action::Toggle(*pane));
                    }
                }
                if self.remotes.len() < 16 && item(ui, "Ещё одно подключение", "", true)
                {
                    actions.push(Action::AddHosts);
                }
                ui.separator();
                let panes = [
                    (
                        Pane::Project,
                        "Папка проекта",
                        "Скрыть папку проекта",
                        "project",
                    ),
                    (
                        Pane::Hosts,
                        "Удалённые хосты (SFTP/FTP)",
                        "Скрыть удалённые хосты",
                        "hosts",
                    ),
                    (
                        Pane::Console,
                        "Консоль (SSH, Telnet, Mosh)",
                        "Скрыть консоль",
                        "console",
                    ),
                    (Pane::Log, "Журнал", "Скрыть журнал", "log"),
                    (
                        Pane::ServerSearch,
                        "Поиск на сервере",
                        "Скрыть поиск на сервере",
                        "server_search",
                    ),
                    (Pane::Db, "База данных", "Скрыть базу данных", "db"),
                    (
                        Pane::Diff,
                        "Сравнение файлов",
                        "Скрыть сравнение файлов",
                        "",
                    ),
                    (Pane::Settings, "Настройки", "Скрыть настройки", ""),
                ];
                for (pane, show, hide, shortcut) in panes {
                    let label =
                        if dock::has(&self.dock, pane) || self.settings.detached.contains(&pane) {
                            hide
                        } else {
                            show
                        };
                    if item(ui, label, &key(shortcut), true) {
                        actions.push(Action::Toggle(pane));
                    }
                }
                ui.separator();
                if item(ui, "Сбросить расположение панелей", "", true) {
                    actions.push(Action::ResetLayout);
                }
                ui.separator();
                ui.weak("Масштаб: Ctrl + / Ctrl − / Ctrl 0");
            });
            ui.menu_button("Инструменты", |ui| {
                ui.set_min_width(260.0);
                if item(ui, "Сравнить с другим файлом", &key("compare"), has_doc)
                {
                    actions.push(Action::Compare);
                }
                if item(ui, "Предпросмотр в браузере", &key("preview"), has_doc)
                {
                    actions.push(Action::Preview);
                }
                ui.separator();
                if item(ui, "Проверить обновления", "", true) {
                    actions.push(Action::CheckUpdates);
                }
                ui.separator();
                let record = if self.macros.recording {
                    "Остановить запись макроса"
                } else {
                    "Записать макрос"
                };
                if item(ui, record, &key("macro_record"), has_doc) {
                    actions.push(Action::MacroRecord);
                }
                let ready = has_doc && !self.macros.recording && !self.macros.events.is_empty();
                if item(ui, "Выполнить макрос", &key("macro_play"), ready) {
                    actions.push(Action::MacroPlay);
                }
                ui.separator();
                if item(ui, "База данных", "", true) {
                    actions.push(Action::Show(Pane::Db));
                }
                if item(ui, "Поиск на сервере", "", true) {
                    actions.push(Action::Show(Pane::ServerSearch));
                }
            });
            ui.menu_button("Плагины", |ui| {
                ui.set_min_width(260.0);
                for (index, plugin) in self.plugins.list.iter().enumerate() {
                    let shortcut = crate::keys::parse(&plugin.shortcut)
                        .map(|s| crate::keys::format(&s))
                        .unwrap_or_default();
                    if item(ui, &plugin.name, &shortcut, true) {
                        actions.push(Action::Plugin(index));
                    }
                }
                if self.plugins.list.is_empty() {
                    ui.weak("Плагинов пока нет");
                }
                ui.separator();
                if item(ui, "Открыть папку плагинов", "", true) {
                    actions.push(Action::PluginsFolder);
                }
                if item(ui, "Перечитать", "", true) {
                    actions.push(Action::PluginsReload);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let theme_hint = if self.settings.look.dark {
                    "Светлая тема"
                } else {
                    "Тёмная тема"
                };
                if Self::tool_button(ui, pal, 3, false, theme_hint) {
                    actions.push(Action::ToggleTheme);
                }
                if Self::tool_button(ui, pal, 2, self.settings.wrap, "Перенос строк") {
                    actions.push(Action::ToggleWrap);
                }
                if Self::tool_button(ui, pal, 1, dock::has(&self.dock, Pane::Console), "Консоль")
                {
                    actions.push(Action::Toggle(Pane::Console));
                }
                if Self::tool_button(
                    ui,
                    pal,
                    0,
                    dock::has(&self.dock, Pane::Hosts),
                    "Удалённые хосты",
                ) {
                    actions.push(Action::Toggle(Pane::Hosts));
                }
                if crate::widgets::plus_button(ui, pal, egui::vec2(30.0, 26.0))
                    .on_hover_text("Новая вкладка")
                    .clicked()
                {
                    actions.push(Action::New);
                }
            });
        });
    }

    fn show_popups(&mut self, ctx: &egui::Context, pal: &Palette) {
        if self.popups.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - 44.0;
        let mut close = None;
        for (index, (text, ok, _)) in self.popups.iter().enumerate().rev() {
            let width = (screen.width() * 0.5).clamp(260.0, 520.0);
            let font = FontId::proportional(13.0);
            let galley =
                ctx.fonts_mut(|f| f.layout(text.clone(), font.clone(), pal.text, width - 46.0));
            let height = galley.size().y + 22.0;
            y -= height;
            let rect = Rect::from_min_size(
                egui::pos2(screen.right() - width - 16.0, y),
                egui::vec2(width, height),
            );
            y -= 8.0;
            let color = if *ok {
                Color32::from_rgb(0x3f, 0xc2, 0x6e)
            } else {
                pal.warn
            };
            egui::Area::new(egui::Id::unique(("tror_popup", index)))
                .order(egui::Order::Tooltip)
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(rect.size(), Sense::click());
                    let painter = ui.painter();
                    painter.rect(
                        rect,
                        10.0,
                        pal.mantle,
                        Stroke::new(1.0, color.gamma_multiply(0.7)),
                        egui::StrokeKind::Inside,
                    );
                    painter.rect_filled(
                        Rect::from_min_size(
                            rect.min + egui::vec2(0.0, 8.0),
                            egui::vec2(3.0, rect.height() - 16.0),
                        ),
                        1.5,
                        color,
                    );
                    let c = rect.left_top() + egui::vec2(21.0, 19.0);
                    painter.circle_stroke(c, 6.5, Stroke::new(1.4, color));
                    let mark = if *ok {
                        vec![
                            c + egui::vec2(-3.0, 0.2),
                            c + egui::vec2(-0.8, 2.6),
                            c + egui::vec2(3.2, -2.4),
                        ]
                    } else {
                        vec![c + egui::vec2(-2.6, -2.6), c + egui::vec2(2.6, 2.6)]
                    };
                    painter.add(egui::Shape::line(mark, Stroke::new(1.5, color)));
                    if !*ok {
                        painter.line_segment(
                            [c + egui::vec2(2.6, -2.6), c + egui::vec2(-2.6, 2.6)],
                            Stroke::new(1.5, color),
                        );
                    }
                    painter.galley(rect.left_top() + egui::vec2(34.0, 11.0), galley, pal.text);
                    if response.clicked() {
                        close = Some(index);
                    }
                });
        }
        if let Some(index) = close {
            self.popups.remove(index);
        }
    }

    fn tool_button(ui: &mut Ui, pal: &Palette, kind: u8, active: bool, hint: &str) -> bool {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 26.0), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, 7.0, pal.surface);
        }
        let color = if active {
            pal.accent
        } else if response.hovered() {
            pal.text
        } else {
            pal.subtext
        };
        let stroke = Stroke::new(1.4, color);
        let c = rect.center();
        let p = ui.painter();
        let frame = Rect::from_center_size(c, egui::vec2(16.0, 13.0));
        match kind {
            0 => {
                p.rect_stroke(frame, 2.5, stroke, egui::StrokeKind::Inside);
                p.line_segment(
                    [
                        egui::pos2(frame.left() + 5.5, frame.top() + 1.0),
                        egui::pos2(frame.left() + 5.5, frame.bottom() - 1.0),
                    ],
                    stroke,
                );
            }
            1 => {
                p.rect_stroke(frame, 2.5, stroke, egui::StrokeKind::Inside);
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(-4.0, -2.5),
                        c + egui::vec2(-1.0, 0.0),
                        c + egui::vec2(-4.0, 2.5),
                    ],
                    stroke,
                ));
                p.line_segment([c + egui::vec2(0.5, 3.0), c + egui::vec2(4.5, 3.0)], stroke);
            }
            2 => {
                p.line_segment(
                    [c + egui::vec2(-7.0, -5.0), c + egui::vec2(7.0, -5.0)],
                    stroke,
                );
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(-7.0, 0.0),
                        c + egui::vec2(5.0, 0.0),
                        c + egui::vec2(7.0, 2.5),
                        c + egui::vec2(5.0, 5.0),
                        c + egui::vec2(0.0, 5.0),
                    ],
                    stroke,
                ));
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(2.5, 2.5),
                        c + egui::vec2(0.0, 5.0),
                        c + egui::vec2(2.5, 7.5),
                    ],
                    stroke,
                ));
                p.line_segment(
                    [c + egui::vec2(-7.0, 5.0), c + egui::vec2(-3.5, 5.0)],
                    stroke,
                );
            }
            4 => {
                p.line_segment([c - egui::vec2(5.5, 0.0), c + egui::vec2(5.5, 0.0)], stroke);
                p.line_segment([c - egui::vec2(0.0, 5.5), c + egui::vec2(0.0, 5.5)], stroke);
            }
            _ => {
                p.circle_stroke(c, 6.0, stroke);
                let half: Vec<egui::Pos2> = (0..=12)
                    .map(|i| {
                        let a =
                            std::f32::consts::FRAC_PI_2 + i as f32 / 12.0 * std::f32::consts::PI;
                        c + egui::vec2(a.cos() * 6.0, a.sin() * 6.0)
                    })
                    .collect();
                p.add(egui::Shape::convex_polygon(half, color, Stroke::NONE));
            }
        }
        response.on_hover_text(hint).clicked()
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
        let field_w = (ui.available_width() - 196.0).max(60.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let query = ui.add(
                TextEdit::singleline(&mut s.query)
                    .hint_text("Найти")
                    .desired_width(field_w)
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
            if icon_button(ui, pal, Icon::Close)
                .on_hover_text("Закрыть (Esc)")
                .clicked()
            {
                actions.push(Action::CloseSearch);
            }
        });
        if s.replace_mode {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add(
                    TextEdit::singleline(&mut s.replacement)
                        .hint_text(if s.regex {
                            "Заменить на ($1, ${name})"
                        } else {
                            "Заменить на"
                        })
                        .desired_width(field_w)
                        .margin(Margin::symmetric(8, 5)),
                );
                if ui.button("Заменить").clicked() {
                    s.pending = Some(Pending::Replace);
                }
                if ui.button("Все").on_hover_text("Заменить все").clicked() {
                    s.pending = Some(Pending::ReplaceAll);
                }
            });
        }
        ui.horizontal(|ui| {
            let info = if let Some(error) = &s.error {
                RichText::new(error).color(pal.warn)
            } else if let Some(n) = s.last_replaced {
                RichText::new(format!("заменено: {n}")).color(pal.subtext)
            } else if s.query.is_empty() {
                RichText::new("введите текст").color(pal.faint)
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
                    None => RichText::new(format!("совпадений: {total}")),
                }
                .color(pal.subtext)
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if s.replace_mode {
                    "Скрыть замену"
                } else {
                    "Замена"
                };
                if ui
                    .add(
                        Button::new(RichText::new(label).small().color(pal.subtext))
                            .frame(false),
                    )
                    .clicked()
                {
                    s.replace_mode = !s.replace_mode;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(info.small()).truncate());
                });
            });
        });
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
                        .small(),
                );
                return;
            };
            let left = toast.unwrap_or_else(|| doc.location().unwrap_or_else(|| doc.title()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let small = |text: String| RichText::new(text).small().color(pal.subtext);

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
            if ui.add(button("Открыть папку…")).clicked() {
                actions.push(Action::OpenFolder);
            }
        });
    }

    fn doc_color(&self, doc: &Document) -> Option<Color32> {
        let key = doc.location()?;
        self.settings.tab_colors.get(&key).map(|c| tab_color(*c))
    }

    fn leaf_docs(&self, id: u64) -> Vec<u64> {
        self.tab_orders
            .iter()
            .find(|order| order.contains(&id))
            .cloned()
            .unwrap_or_else(|| vec![id])
    }

    fn refresh_tab_orders(&mut self) {
        self.tab_orders = self
            .dock
            .iter_leaves()
            .map(|(_, leaf)| {
                leaf.tabs()
                    .iter()
                    .filter_map(|t| match t {
                        Pane::Doc(d) => Some(*d),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
    }

    fn tab_cmd(&mut self, id: u64, cmd: TabCmd, now: f64) {
        let Some(index) = self.docs.iter().position(|d| d.id == id) else {
            return;
        };
        match cmd {
            TabCmd::Close => self.close_queue.push_back(id),
            TabCmd::CloseOthers | TabCmd::CloseLeft | TabCmd::CloseRight => {
                let order = self.leaf_docs(id);
                let at = order.iter().position(|d| *d == id).unwrap_or(0);
                let ids: Vec<u64> = match cmd {
                    TabCmd::CloseLeft => order[..at].to_vec(),
                    TabCmd::CloseRight => order[at + 1..].to_vec(),
                    _ => order.into_iter().filter(|d| *d != id).collect(),
                };
                let pinned = |id: &u64| self.docs.iter().any(|d| d.id == *id && d.pinned);
                let ids: Vec<u64> = ids.into_iter().filter(|id| !pinned(id)).collect();
                self.close_queue.extend(ids);
            }
            TabCmd::Save => {
                self.save(index, now);
            }
            TabCmd::SaveAs => {
                self.save_as(index, now);
            }
            TabCmd::Rename => self.rename = Some((id, self.docs[index].title())),
            TabCmd::Color(color) => {
                if let Some(key) = self.docs[index].location() {
                    match color {
                        Some(c) => {
                            self.settings.tab_colors.insert(key, c);
                        }
                        None => {
                            self.settings.tab_colors.remove(&key);
                        }
                    }
                }
            }
            TabCmd::Pin => self.docs[index].pinned = !self.docs[index].pinned,
            TabCmd::NewGroup(split) => {
                if let Some(path) = self.dock.find_tab(&Pane::Doc(id)) {
                    let count = self.leaf_docs(id).len();
                    if count > 1 {
                        self.dock.move_tab(
                            path,
                            (path.node_path(), egui_stack::TabInsert::Split(split)),
                        );
                    }
                }
            }
            TabCmd::OtherGroup => {
                let Some(path) = self.dock.find_tab(&Pane::Doc(id)) else {
                    return;
                };
                let target = self
                    .dock
                    .iter_leaves()
                    .find(|(node, leaf)| {
                        *node != path.node_path()
                            && leaf
                                .tabs()
                                .iter()
                                .any(|t| matches!(t, Pane::Doc(_) | Pane::Welcome))
                    })
                    .map(|(node, _)| node);
                match target {
                    Some(node) => self
                        .dock
                        .move_tab(path, (node, egui_stack::TabInsert::Append)),
                    None if self.leaf_docs(id).len() > 1 => self.dock.move_tab(
                        path,
                        (
                            path.node_path(),
                            egui_stack::TabInsert::Split(egui_stack::Split::Right),
                        ),
                    ),
                    None => {}
                }
            }
            TabCmd::Reveal => {
                if let Some(dir) = self.docs[index].path.as_ref().and_then(|p| p.parent()) {
                    let _ = preview::open_folder(dir);
                }
            }
        }
    }

    fn drain_close_queue(&mut self) {
        while self.confirm.is_none() {
            let Some(id) = self.close_queue.pop_front() else {
                break;
            };
            if let Some(index) = self.docs.iter().position(|d| d.id == id) {
                self.request_close(index);
            }
        }
    }

    fn rename_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((id, name)) = &mut self.rename else {
            return;
        };
        let id = *id;
        let mut done = false;
        let mut go = false;
        let modal = dialog("tror_rename_tab", pal).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label("Новое имя файла");
            ui.add_space(4.0);
            let field = ui.add(TextEdit::singleline(name).desired_width(f32::INFINITY));
            if !field.has_focus() && !ui.input(|i| i.pointer.any_down()) {
                field.request_focus();
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Переименовать", true) {
                    go = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
                    done = true;
                }
            });
        });
        let now = ctx.input(|i| i.time);
        if go {
            let name = name.trim().to_owned();
            done = true;
            if let Some(index) = self.docs.iter().position(|d| d.id == id)
                && !name.is_empty()
                && !name.contains(['/', '\\'])
                && name != self.docs[index].title()
            {
                let old_key = self.docs[index].location();
                let result = match (
                    self.docs[index].remote.clone(),
                    self.docs[index].path.clone(),
                ) {
                    (Some((profile, from)), _) => {
                        let to = crate::remote::join(&crate::remote::parent(&from), &name);
                        let panel = self.remotes.iter().position(|r| r.serves(&profile));
                        match panel {
                            Some(p) => {
                                self.remotes[p].rename_file(&profile, &from, &to).map(|()| {
                                    self.docs[index].remote = Some((profile, to));
                                })
                            }
                            None => Err(format!("сначала подключитесь к «{profile}»")),
                        }
                    }
                    (None, Some(path)) => {
                        let to = path.with_file_name(&name);
                        if to.exists() {
                            Err("файл с таким именем уже есть".into())
                        } else {
                            std::fs::rename(&path, &to)
                                .map_err(|e| e.to_string())
                                .map(|()| {
                                    let first = self.docs[index].first_line();
                                    self.docs[index].syntax =
                                        self.syntaxes.detect(Some(&to), &first);
                                    self.docs[index].path = Some(to.clone());
                                    self.remember(&to);
                                })
                        }
                    }
                    _ => Err("файл ещё не сохранён — используйте «Сохранить как»".into()),
                };
                match result {
                    Ok(()) => {
                        if let (Some(old), Some(new)) = (old_key, self.docs[index].location())
                            && let Some(color) = self.settings.tab_colors.remove(&old)
                        {
                            self.settings.tab_colors.insert(new, color);
                        }
                        self.notify(format!("Переименовано в {name}"), now);
                    }
                    Err(e) => self.notify(format!("Не удалось переименовать: {e}"), now),
                }
            }
        }
        if done || modal.should_close() {
            self.rename = None;
            self.focus_editor = true;
        }
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

impl TrorApp {
    fn doc_pane(
        &mut self,
        ui: &mut Ui,
        id: u64,
        pal: &Palette,
        actions: &mut Vec<Action>,
        modal_open: bool,
    ) {
        let Some(index) = self.docs.iter().position(|d| d.id == id) else {
            return;
        };
        let active = index == self.active;
        let pane = ui.available_rect_before_wrap();
        let look = &self.settings.look;
        let (wrap, font_size, line_spacing) =
            (self.settings.wrap, look.font_size, look.line_spacing);
        if active {
            let idle = !self.search.open
                && !self.console.focused()
                && ui.ctx().memory(|m| m.focused().is_none());
            let focus = (std::mem::take(&mut self.focus_editor) || idle) && !modal_open;
            editor::show(
                ui,
                &mut self.docs[index],
                &mut self.search,
                editor::View {
                    syn: &self.syntaxes,
                    pal,
                    focus,
                    wrap,
                    font_size,
                    line_spacing,
                    edit: self.settings.edit,
                    macros: &mut self.macros,
                    complete: &mut self.complete,
                },
            );
            if self.search.open {
                let width = (pane.width() - 44.0).clamp(250.0, 430.0);
                egui::Area::new(egui::Id::unique("tror_find"))
                    .order(egui::Order::Middle)
                    .pivot(egui::Align2::RIGHT_TOP)
                    .fixed_pos(pane.right_top() + egui::vec2(-22.0, 8.0))
                    .show(ui.ctx(), |ui| {
                        Frame::new()
                            .fill(pal.mantle)
                            .stroke(Stroke::new(1.0, pal.border))
                            .corner_radius(self.settings.look.radius.saturating_add(2))
                            .shadow(ui.style().visuals.popup_shadow)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(width);
                                self.search_bar(ui, pal, actions);
                            });
                    });
            }
            return;
        }
        let mut search = SearchState::default();
        let mut complete = Completion::default();
        let mut macros = editor::Macro::default();
        editor::show(
            ui,
            &mut self.docs[index],
            &mut search,
            editor::View {
                syn: &self.syntaxes,
                pal,
                focus: false,
                wrap,
                font_size,
                line_spacing,
                edit: self.settings.edit,
                macros: &mut macros,
                complete: &mut complete,
            },
        );
        if ui.memory(|m| m.has_focus(editor::editor_id(id))) {
            actions.push(Action::Activate(index));
        }
    }

    fn log_pane(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                if ui.add(Button::new("Очистить").fill(pal.surface)).clicked() {
                    self.log.clear();
                }
                if ui.add(Button::new("Копировать всё").fill(pal.surface)).clicked() {
                    let text: Vec<&str> = self.log.iter().map(|(t, _)| t.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("Сырой обмен с сервером: команды и ответы FTP, этапы SSH/SFTP, запросы к базе")
                                .small()
                                .color(pal.faint),
                        )
                        .truncate(),
                    );
                });
            });
        });
        ui.add_space(2.0);
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, pal.base);
        let font = FontId::monospace(12.5);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let [server, client, error, info] = self.settings.look.log_colors();
        ui.spacing_mut().item_spacing.y = 0.0;
        ScrollArea::both()
            .id_salt("tror_log")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_h, self.log.len(), |ui, range| {
                for (text, kind) in &self.log[range] {
                    let color = match kind {
                        crate::trace::Kind::Server => server,
                        crate::trace::Kind::Client => client,
                        crate::trace::Kind::Error => error,
                        crate::trace::Kind::Info => info,
                    };
                    let (stamp, body) = text.split_at(text.find("  ").unwrap_or(0));
                    let mut job = egui::text::LayoutJob::default();
                    job.append("  ", 0.0, egui::TextFormat::simple(font.clone(), pal.faint));
                    job.append(
                        stamp,
                        0.0,
                        egui::TextFormat::simple(font.clone(), pal.faint),
                    );
                    job.append(body, 0.0, egui::TextFormat::simple(font.clone(), color));
                    ui.add(egui::Label::new(job).extend());
                }
            });
    }
}

struct Viewer<'a> {
    app: &'a mut TrorApp,
    pal: Palette,
    actions: &'a mut Vec<Action>,
    modal_open: bool,
}

impl Viewer<'_> {
    fn tab_menu(&mut self, ui: &mut Ui, id: u64) {
        let Some(doc) = self.app.docs.iter().find(|d| d.id == id) else {
            return;
        };
        ui.set_min_width(250.0);
        let keys = crate::keys::Keys::new(&self.app.settings.keys);
        let key = |name: &str| keys.label(name);
        let order = self.app.leaf_docs(id);
        let at = order.iter().position(|d| *d == id).unwrap_or(0);
        let location = doc.location();
        let path = doc.path.clone();
        let remote = doc.remote.clone();
        let saved = path.is_some() || remote.is_some();
        let color = location
            .as_ref()
            .and_then(|k| self.app.settings.tab_colors.get(k))
            .copied();
        let pinned = doc.pinned;
        let mut cmd = None;
        let row = |ui: &mut Ui, text: &str, shortcut: String, enabled: bool| {
            ui.add_enabled(enabled, Button::new(text).shortcut_text(shortcut))
                .clicked()
        };
        if row(ui, "Закрыть", key("close_tab"), true) {
            cmd = Some(TabCmd::Close);
        }
        if row(ui, "Закрыть остальные", String::new(), order.len() > 1) {
            cmd = Some(TabCmd::CloseOthers);
        }
        if row(ui, "Закрыть вкладки слева", String::new(), at > 0) {
            cmd = Some(TabCmd::CloseLeft);
        }
        if row(
            ui,
            "Закрыть вкладки справа",
            String::new(),
            at + 1 < order.len(),
        ) {
            cmd = Some(TabCmd::CloseRight);
        }
        ui.separator();
        let pin = if pinned {
            "Открепить вкладку"
        } else {
            "Закрепить вкладку"
        };
        if row(ui, pin, String::new(), true) {
            cmd = Some(TabCmd::Pin);
        }
        ui.separator();
        if row(ui, "Сохранить", key("save"), true) {
            cmd = Some(TabCmd::Save);
        }
        if row(ui, "Сохранить как…", key("save_as"), true) {
            cmd = Some(TabCmd::SaveAs);
        }
        if row(ui, "Переименовать…", String::new(), saved) {
            cmd = Some(TabCmd::Rename);
        }
        ui.separator();
        let full = match (&remote, &path) {
            (Some((_, p)), _) => Some(p.clone()),
            (None, Some(p)) => Some(p.display().to_string()),
            _ => None,
        };
        let name = doc.title();
        let dir = match (&remote, &path) {
            (Some((_, p)), _) => Some(crate::remote::parent(p)),
            (None, Some(p)) => p.parent().map(|d| d.display().to_string()),
            _ => None,
        };
        if row(ui, "Копировать полный путь", String::new(), full.is_some())
            && let Some(text) = &full
        {
            ui.ctx().copy_text(text.clone());
        }
        if row(ui, "Копировать имя файла", String::new(), true) {
            ui.ctx().copy_text(name.clone());
        }
        if row(ui, "Копировать путь к папке", String::new(), dir.is_some())
            && let Some(text) = &dir
        {
            ui.ctx().copy_text(text.clone());
        }
        if path.is_some() && remote.is_none() && row(ui, "Показать в папке", String::new(), true)
        {
            cmd = Some(TabCmd::Reveal);
        }
        ui.separator();
        ui.add_enabled_ui(location.is_some(), |ui| {
            ui.menu_button("Цвет вкладки", |ui| {
                ui.set_min_width(170.0);
                for (i, (label, _)) in TAB_COLORS.iter().enumerate() {
                    let swatch = tab_color(i as u8);
                    let response =
                        ui.selectable_label(color == Some(i as u8), format!("     {label}"));
                    let r = response.rect;
                    ui.painter().circle_filled(
                        egui::pos2(r.left() + 12.0, r.center().y),
                        5.5,
                        swatch,
                    );
                    if response.clicked() {
                        cmd = Some(TabCmd::Color(Some(i as u8)));
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(color.is_some(), Button::new("Без цвета"))
                    .clicked()
                {
                    cmd = Some(TabCmd::Color(None));
                }
            });
        });
        ui.menu_button("Область вкладок", |ui| {
            ui.set_min_width(230.0);
            let alone = order.len() < 2;
            if ui
                .add_enabled(!alone, Button::new("Новая область справа"))
                .clicked()
            {
                cmd = Some(TabCmd::NewGroup(egui_stack::Split::Right));
            }
            if ui
                .add_enabled(!alone, Button::new("Новая область снизу"))
                .clicked()
            {
                cmd = Some(TabCmd::NewGroup(egui_stack::Split::Below));
            }
            if ui.button("Переместить в другую область").clicked() {
                cmd = Some(TabCmd::OtherGroup);
            }
        });
        if let Some(cmd) = cmd {
            self.actions.push(Action::TabCmd(id, cmd));
            ui.close();
        }
    }
}

impl TabViewer for Viewer<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::unique(*tab)
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        dock::set_mark(dock::Mark::default());
        let Pane::Doc(id) = *tab else {
            return tab.title().into();
        };
        let Some(index) = self.app.docs.iter().position(|d| d.id == id) else {
            return "".into();
        };
        let doc = &self.app.docs[index];
        let pal = &self.pal;
        let active = index == self.app.active;
        dock::set_mark(dock::Mark {
            dirty: doc.dirty(),
            pinned: doc.pinned,
            accent: pal.accent,
        });
        let color = if doc.missing {
            pal.warn
        } else if active {
            pal.text
        } else {
            pal.subtext
        };
        let family = if active {
            theme::medium()
        } else {
            egui::FontFamily::Proportional
        };
        let size = self.app.settings.look.ui_size - 0.5;
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &doc.title(),
            0.0,
            egui::TextFormat {
                font_id: FontId::new(size, family),
                color,
                valign: Align::Center,
                strikethrough: if doc.missing {
                    Stroke::new(1.0, pal.warn)
                } else {
                    Stroke::NONE
                },
                ..Default::default()
            },
        );
        job.into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Pane) {
        let pal = self.pal;
        let app = &mut *self.app;
        let look = &app.settings.look;
        let (dark, font_size) = (look.dark, look.font_size);
        let side = Frame::new().inner_margin(Margin::symmetric(8, 0));
        let pane = *tab;
        let actions = &mut *self.actions;
        let modal_open = self.modal_open;
        let mut body = |ui: &mut Ui| match pane {
            Pane::Welcome => app.welcome(ui, &pal, actions),
            Pane::Doc(id) => app.doc_pane(ui, id, &pal, actions, modal_open),
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) => {
                let index = pane.host_index().unwrap_or(0);
                if let Some(panel) = app.remotes.get_mut(index) {
                    side.show(ui, |ui| panel.ui(ui, &pal));
                }
            }
            Pane::Project => {
                let recent = app.settings.recent_projects.clone();
                app.project.show_size = app.settings.project_size;
                app.project.show_date = app.settings.project_date;
                side.show(ui, |ui| app.project.ui(ui, &pal, &recent));
                app.settings.project_size = app.project.show_size;
                app.settings.project_date = app.project.show_date;
            }
            Pane::Console => {
                app.console_shown = true;
                side.show(ui, |ui| app.console.ui(ui, &pal, dark, font_size));
            }
            Pane::Log => app.log_pane(ui, &pal),
            Pane::ServerSearch => {
                side.show(ui, |ui| app.remotes[0].search_ui(ui, &pal));
            }
            Pane::Settings => {
                app.settings_shown = true;
                let note = prefs::ui(ui, &pal, &mut app.settings, &mut app.capturing);
                if let Some(text) = note {
                    let now = ui.input(|i| i.time);
                    app.notify(text, now);
                }
            }
            Pane::Diff => {
                if let Some((id, line)) = app.diff.ui(ui, &pal, &app.docs, dark, font_size) {
                    actions.push(Action::Goto(id, line));
                }
            }
            Pane::Db => app.db.ui(ui, &pal, &mut app.remotes[0].store, font_size),
            Pane::Preview => {}
        };
        let (min_w, min_h) = pane.min_size();
        let room = ui.available_size();
        let fixed = matches!(pane, Pane::Doc(_) | Pane::Welcome | Pane::Preview);
        if !fixed && (room.x + 0.5 < min_w || room.y + 0.5 < min_h) {
            let size = egui::vec2(room.x.max(min_w), room.y.max(min_h));
            ScrollArea::both()
                .id_salt(("tror_narrow", pane))
                .auto_shrink(false)
                .show(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let mut inner = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    inner.set_clip_rect(inner.clip_rect().intersect(ui.clip_rect()));
                    body(&mut inner);
                });
        } else {
            body(ui);
        }
    }

    fn context_menu(&mut self, ui: &mut Ui, tab: &mut Pane, _path: egui_stack::TabPath) {
        if let Pane::Doc(id) = *tab {
            self.tab_menu(ui, id);
            ui.separator();
        }
        if !matches!(*tab, Pane::Welcome | Pane::Preview)
            && ui.button("В отдельное окно системы").clicked()
        {
            self.actions.push(Action::Detach(*tab));
            ui.close();
        }
    }

    fn on_tab_button(&mut self, tab: &mut Pane, response: &egui::Response) {
        let Pane::Doc(id) = *tab else {
            return;
        };
        let Some(index) = self.app.docs.iter().position(|d| d.id == id) else {
            return;
        };
        if response.middle_clicked() && !self.app.docs[index].pinned {
            self.actions.push(Action::CloseTab(index));
        }
        if let Some(location) = self.app.docs[index].location() {
            response.clone().on_hover_text(location);
        }
    }

    fn on_close(&mut self, tab: &mut Pane) -> egui_stack::tab_viewer::OnCloseResponse {
        if let Pane::Doc(id) = *tab {
            if let Some(index) = self.app.docs.iter().position(|d| d.id == id) {
                if self.app.docs[index].pinned {
                    self.actions.push(Action::TabCmd(id, TabCmd::Pin));
                } else {
                    self.actions.push(Action::CloseTab(index));
                }
            }
            return egui_stack::tab_viewer::OnCloseResponse::Ignore;
        }
        egui_stack::tab_viewer::OnCloseResponse::Close
    }

    fn tab_style_override(
        &self,
        tab: &Pane,
        global: &egui_stack::TabStyle,
    ) -> Option<egui_stack::TabStyle> {
        let Pane::Doc(id) = *tab else {
            return None;
        };
        let doc = self.app.docs.iter().find(|d| d.id == id)?;
        let color = self.app.doc_color(doc)?;
        let mut style = global.clone();
        let mix = |base: Color32, amount: f32| theme::blend(base, color, amount);
        style.inactive.bg_fill = mix(self.pal.mantle, 0.16);
        style.hovered.bg_fill = mix(self.pal.crust, 0.24);
        style.inactive_with_kb_focus.bg_fill = style.inactive.bg_fill;
        for state in [
            &mut style.active,
            &mut style.focused,
            &mut style.active_with_kb_focus,
            &mut style.focused_with_kb_focus,
        ] {
            state.bg_fill = mix(state.bg_fill, 0.22);
            state.outline_color = mix(state.outline_color, 0.5);
        }
        Some(style)
    }

    fn tab_group(&self, tab: &Pane) -> u8 {
        match tab {
            Pane::Doc(id) => self.app.docs.iter().any(|d| d.id == *id && d.pinned) as u8,
            _ => 0,
        }
    }

    fn close_in_tab_menu(&self, tab: &Pane) -> bool {
        !matches!(tab, Pane::Doc(_))
    }

    fn is_closeable(&self, tab: &Pane) -> bool {
        *tab != Pane::Welcome
    }

    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] {
        [false, false]
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn drag_out(frame: &eframe::Frame, paths: Vec<PathBuf>) {
    let icon = include_bytes!("../assets/tror.png").to_vec();
    if let Err(e) = drag::start_drag(
        frame,
        drag::DragItem::Files(paths),
        drag::Image::Raw(icon),
        |_, _| {},
        drag::Options::default(),
    ) {
        crate::trace::popup(format!("Не удалось начать перетаскивание: {e}"), false);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn drag_out(_frame: &eframe::Frame, _paths: Vec<PathBuf>) {}

impl eframe::App for TrorApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(path) = self.update.setup.take() {
            let _ = crate::update::run_setup(&path);
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        self.apply_look(&ctx);
        let pal = self.palette();
        let mut actions = Vec::new();

        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.docs.iter().any(|d| d.dirty())
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        } else if ctx.input(|i| i.viewport().close_requested())
            && let Some(dir) = crate::recover::dir()
        {
            crate::recover::clear(&dir);
            self.recovery = Default::default();
            self.recovery.last = f64::INFINITY;
        }
        let detached = self.settings.detached.clone();
        for (index, panel) in self.remotes.iter_mut().enumerate() {
            let pane = Pane::hosts(index);
            panel.visible = dock::has(&self.dock, pane) || detached.contains(&pane);
            panel.tree = self.settings.remote_tree;
        }
        if let Ok(mut request) = VIEW_REQUEST.lock() {
            if let Some(tree) = request.0.take() {
                self.settings.remote_tree = tree;
            }
            if let Some(tree) = request.1.take() {
                self.settings.project_tree = tree;
            }
        }
        self.project.tree = self.settings.project_tree;
        self.project.upload_target = self.remotes.iter().find_map(|r| r.peer_label());
        crate::complete::set_custom(&self.settings.dictionaries);
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            if let Some(panel) = self.remotes.iter_mut().find(|r| r.can_upload()) {
                panel.offer_drop(dropped);
            } else if dropped.len() == 1 && dropped[0].is_dir() {
                self.project.set_root(dropped[0].clone());
                actions.push(Action::Show(Pane::Project));
            } else {
                for path in dropped {
                    actions.push(Action::OpenPath(path));
                }
            }
        }
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            ctx.set_zoom_factor((ctx.zoom_factor() * zoom).clamp(0.5, 3.0));
        }
        self.check_missing(&ctx);
        let modal_open = self.goto.is_some()
            || self.rename.is_some()
            || self.confirm.is_some()
            || self.remotes.iter().any(|r| r.modal_open())
            || self.console.modal_open()
            || self.project.modal_open()
            || self.db.modal_open();
        if self
            .remotes
            .iter_mut()
            .any(|r| std::mem::take(&mut r.want_search))
        {
            dock::show(&mut self.dock, Pane::ServerSearch, &self.settings.floats);
        }
        self.log.extend(crate::trace::take());
        if self.log.len() > 5000 {
            self.log.drain(..1000);
        }
        let now = ctx.input(|i| i.time);
        for (text, ok) in crate::trace::take_popups() {
            self.popups.push((text, ok, now));
        }
        self.popups.retain(|(_, _, at)| now - at < 6.0);
        for index in 0..self.remotes.len() {
            let events = self.remotes[index].poll();
            self.remote_events(&ctx, events);
        }
        let labels: Vec<Option<String>> = self.remotes.iter().map(|r| r.peer_label()).collect();
        for (index, panel) in self.remotes.iter_mut().enumerate() {
            panel.peers = labels
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .filter_map(|(i, label)| Some((i as u8, label.clone()?)))
                .collect();
        }
        self.transfers();
        self.db.poll();
        self.plugin_results(now);
        for note in self.console.poll() {
            self.notify(note, now);
        }
        for event in self.project.take_events() {
            match event {
                ProjectEvent::Open(paths) => {
                    for path in paths {
                        actions.push(Action::OpenPath(path));
                    }
                }
                ProjectEvent::Upload(paths) => {
                    if let Some(panel) = self.remotes.iter_mut().find(|r| r.connected()) {
                        panel.upload_local(paths);
                    }
                }
                ProjectEvent::Toast(text) => self.notify(text, now),
                ProjectEvent::Opened(path) => {
                    self.settings.recent_projects.retain(|p| *p != path);
                    self.settings.recent_projects.insert(0, path.clone());
                    self.settings.recent_projects.truncate(MAX_RECENT);
                    self.settings.project = Some(path);
                }
                ProjectEvent::View(tree) => self.settings.project_tree = tree,
            }
        }
        if self.project.root.is_none() {
            self.settings.project = None;
        }
        self.project.show_size = self.settings.project_size;
        self.project.show_date = self.settings.project_date;
        self.settings.columns = crate::widgets::columns();
        crate::document::set_indent(self.settings.edit.unit());
        crate::terminal::SCROLLBACK.store(
            self.settings.scrollback as usize,
            std::sync::atomic::Ordering::Relaxed,
        );
        crate::db::MAX_ROWS.store(
            self.settings.db_rows as usize,
            std::sync::atomic::Ordering::Relaxed,
        );
        if !modal_open {
            self.shortcuts(&ctx, &mut actions);
        }
        let now = ctx.input(|i| i.time);
        let unsaved: Vec<PathBuf> = self
            .docs
            .iter()
            .filter(|d| d.dirty())
            .filter_map(|d| d.path.clone())
            .collect();
        match self
            .find_files
            .ui(&ctx, &pal, self.project.root.as_deref(), &unsaved)
        {
            Some(crate::findfiles::Out::Open(path, line)) => {
                self.open_path(&path, now);
                if let Some(doc) = self.docs.get_mut(self.active)
                    && doc.path.as_deref() == Some(path.as_path())
                {
                    doc.goto_line(line);
                }
            }
            Some(crate::findfiles::Out::Changed) => {
                for doc in self.docs.iter_mut().filter(|d| !d.dirty() && d.path.is_some()) {
                    let _ = doc.reopen_as(doc.encoding);
                }
            }
            None => {}
        }
        self.update_ui(&ctx, &pal, now);
        if let Some(paths) = self.project.drag_out.take() {
            drag_out(frame, paths);
        }
        if self.settings.autosave && now - self.recovery.last > 10.0 {
            self.recovery.last = now;
            if let Some(dir) = crate::recover::dir() {
                let _ = self.recovery.snapshot(&dir, &self.docs);
            }
            if !unsaved.is_empty() || self.docs.iter().any(|d| d.dirty()) {
                ctx.request_repaint_after(std::time::Duration::from_secs(11));
            }
        }

        let bar = Frame::new().fill(pal.mantle);
        Panel::top("tror_menu")
            .frame(bar.inner_margin(Margin {
                left: 10,
                right: 8,
                top: 5,
                bottom: 3,
            }))
            .show_separator_line(true)
            .show(ui, |ui| self.menu(ui, &pal, &mut actions));
        Panel::bottom("tror_status")
            .frame(bar.inner_margin(Margin::symmetric(10, 4)))
            .show_separator_line(true)
            .show(ui, |ui| self.status_bar(ui, &pal));

        self.settings.detached.retain(|pane| match pane {
            Pane::Doc(id) => self.docs.iter().any(|d| d.id == *id),
            _ => true,
        });
        let ids: Vec<u64> = self
            .docs
            .iter()
            .map(|d| d.id)
            .filter(|id| !self.settings.detached.contains(&Pane::Doc(*id)))
            .collect();
        dock::sync_with(&mut self.dock, &ids, self.docs.is_empty());
        if self.focus_editor
            && let Some(doc) = self.docs.get(self.active)
            && let Some(path) = self.dock.find_tab(&Pane::Doc(doc.id))
        {
            let _ = self.dock.focus_tab(path);
        }
        self.refresh_tab_orders();
        let mut dock = std::mem::replace(&mut self.dock, DockState::new(Vec::new()));
        let style = dock::style(&pal, ui.style(), self.settings.look.radius);
        self.console_shown = false;
        self.settings_shown = false;
        CentralPanel::no_frame().show(ui, |ui| {
            let mut viewer = Viewer {
                app: self,
                pal,
                actions: &mut actions,
                modal_open,
            };
            DockArea::new(&mut dock)
                .painter(&dock::TAB_PAINTER)
                .style(style)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .leaf_min_size(&|tabs: &[Pane]| dock::leaf_min(tabs) - egui::vec2(0.0, 34.0))
                .show_inside(ui, &mut viewer);
        });
        if let Some((_, Pane::Doc(id))) = dock.find_active_focused()
            && let Some(index) = self.docs.iter().position(|d| d.id == *id)
            && index != self.active
        {
            self.active = index;
            self.complete.close();
        }
        let pinned: Vec<u64> = self.docs.iter().filter(|d| d.pinned).map(|d| d.id).collect();
        dock::pins_first(&mut dock, &pinned);
        if !ctx.input(|i| i.pointer.any_down()) {
            dock::enforce_min(&mut dock);
        }
        dock::remember(&dock, &mut self.settings.floats);
        self.dock = dock;
        for pane in self.settings.detached.clone() {
            let title = match pane {
                Pane::Doc(id) => self
                    .docs
                    .iter()
                    .find(|d| d.id == id)
                    .map(|d| format!("{} — Tror", d.title()))
                    .unwrap_or_default(),
                _ => format!("{} — Tror", pane.title()),
            };
            let mut closed = false;
            let (min_w, min_h) = pane.min_size();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("tror_window", pane)),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([860.0, 580.0])
                    .with_min_inner_size([min_w + 16.0, min_h + 16.0]),
                |ui, _class| {
                    closed = ui.input(|i| i.viewport().close_requested());
                    if !modal_open {
                        self.shortcuts(ui.ctx(), &mut actions);
                    }
                    CentralPanel::default()
                        .frame(Frame::new().fill(pal.mantle))
                        .show(ui, |ui| {
                            let mut viewer = Viewer {
                                app: self,
                                pal,
                                actions: &mut actions,
                                modal_open,
                            };
                            let mut tab = pane;
                            viewer.ui(ui, &mut tab);
                        });
                    self.pane_dialogs(pane, ui.ctx(), &pal);
                },
            );
            if closed {
                actions.push(Action::Attach(pane));
            }
        }
        self.focus_editor = false;
        if !self.console_shown {
            self.console.unfocus();
        }
        if !self.settings_shown {
            self.capturing = None;
        }

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
        self.rename_modal(&ctx, &pal);
        self.confirm_modal(&ctx);
        let mut panes = self.hosts_panes();
        panes.extend([Pane::Console, Pane::Db, Pane::Project]);
        for pane in panes {
            if !self.settings.detached.contains(&pane) {
                self.pane_dialogs(pane, &ctx, &pal);
            }
        }
        self.show_popups(&ctx, &pal);

        for action in actions {
            self.run(action, &ctx);
        }
        self.drain_close_queue();
        self.trim_hosts();

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
        self.settings.open = self
            .docs
            .iter()
            .filter_map(|d| Some((d.id, d.path.clone()?)))
            .collect();
        self.settings.pinned = self
            .docs
            .iter()
            .filter(|d| d.pinned)
            .filter_map(|d| d.path.clone())
            .collect();
        self.settings.active = self.active;
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        eframe::set_value(storage, DOCK_KEY, &self.dock);
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
