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

mod actions;
mod bars;
mod files;
mod find;
mod frame;
mod hosts;
mod look;
mod menu;
mod modals;
mod plugin_run;
mod plugin_ui;
mod tabs;

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
    MacroDialog(bool),
    MacroLoad(usize, bool),
    MacroDelete(usize),
    Fold(u8),
    Edit(&'static str),
    Goto(u64, usize),
    Plugin(usize),
    PluginCommand(usize, usize),
    PluginPanel(usize, usize),
    PluginSettings(usize),
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

enum MacroDialog {
    Save(String),
    Run(u32, bool),
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
    plugin_dialog: Option<plugin_ui::PluginDialog>,
    plugin_panels: std::collections::HashMap<u64, plugin_ui::PluginPanel>,
    find_files: crate::findfiles::FindFiles,
    update: Updater,
    macros: editor::Macro,
    macro_dialog: Option<MacroDialog>,
    find: find::FindWindow,
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
        crate::vault::set_in_file(settings.passwords_in_file);
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
            logo: eframe::icon_data::from_png_bytes(include_bytes!("../../assets/tror.png"))
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
            plugin_dialog: None,
            plugin_panels: Default::default(),
            find_files: Default::default(),
            update: Default::default(),
            macros: Default::default(),
            macro_dialog: None,
            find: Default::default(),
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
        app.sync_plugin_panels();
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
}
