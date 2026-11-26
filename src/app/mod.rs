use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Button, CentralPanel, Color32, FontId, Frame, Key, Layout, Margin, MenuBar,
    Modifiers, Panel, PopupCloseBehavior, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui,
    ViewportCommand,
    containers::menu::{MenuButton, MenuConfig},
};
use egui_stack::{DockArea, DockState, TabViewer};
use encoding_rs::Encoding;

use crate::console::ConsolePanel;
use crate::db::DbPanel;
use crate::diff::DiffPanel;
use crate::dock::{self, Pane};
use crate::files::ProjectPanel;
use crate::prefs::{self, Look, Settings};
use crate::preview::{self, Preview};
use crate::remote::panel::{PanelEvent, RemotePanel};
use crate::text::complete::Completion;
use crate::text::document::{Document, Eol};
use crate::text::editor;
use crate::text::highlight::Syntaxes;
use crate::text::search::{Pending, SearchState};
use crate::theme::{self, Palette};

mod actions;
mod bars;
mod files;
mod find;
mod frame;
mod hosts;
mod input;
mod jump;
mod menu;
mod modals;
mod owner;

mod plugins;
mod tabs;
#[cfg(test)]
mod uitest;

use crate::widgets::{
    Icon, dialog, dialog_button, icon_button, size_label, window_header,
};

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
    OpenSecondFolder,
    AddHosts,
    OpenPath(PathBuf),
    OpenProject(PathBuf),
    ToggleVisual,
    Print,
    Jump(&'static str),
    CompareServer,
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
    SearchServer(usize),
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
    Attach(Pane),
    PluginsFolder,
    PluginsReload,
    PluginsManage,
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
    CompareServer,
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

struct Conflict {
    profile: String,
    doc: u64,
    path: String,
    server: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Floats {
    pub spawn: std::collections::HashMap<Pane, [f32; 4]>,
    pub owned: std::collections::HashSet<Pane>,
}

impl Floats {
    pub fn prune(&mut self, detached: &[Pane]) {
        self.spawn.retain(|p, _| detached.contains(p));
        self.owned.retain(|p| detached.contains(p));
    }
}


enum MacroDialog {
    Save(String),
    Run(u32, bool),
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
    project2: ProjectPanel,
    sizes: dock::Sizes,
    zoom_acc: f32,
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
    jump: jump::Jump,
    plugin_dialog: Option<plugins::PluginDialog>,
    plugin_panels: std::collections::HashMap<u64, plugins::PluginPanel>,
    find_files: crate::findfiles::FindFiles,
    find_out: Option<crate::findfiles::Out>,
    update: Updater,
    macros: editor::Macro,
    macro_dialog: Option<MacroDialog>,
    find: find::FindWindow,
    floats: Floats,
    manager: Option<plugins::Manager>,
    plugin_active: Option<u64>,
    plugin_seen: std::collections::HashMap<u64, u64>,
    decorations: std::collections::HashMap<u64, Vec<crate::text::editor::Decoration>>,
    plugin_decos:
        std::collections::HashMap<(u64, usize, String), Vec<crate::text::editor::Decoration>>,
    visual: std::collections::HashMap<u64, crate::visual::Visual>,
    drop_hint: Option<Option<(usize, String)>>,
    scale: f32,
    instance: Option<std::sync::mpsc::Receiver<PathBuf>>,
    recovery: crate::text::recover::Recovery,
    confirm: Option<Confirm>,
    conflict: Option<Conflict>,
    allow_close: bool,
    focus_editor: bool,
    queued: Vec<Action>,
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
        let plugins_off = settings.plugins_off.clone();
        let panel_sizes = settings.panel_sizes.clone();
        crate::i18n::set_english(match settings.language.as_str() {
            "ru" => false,
            "en" => true,
            _ => crate::i18n::system_english(),
        });
        crate::remote::vault::set_in_file(settings.passwords_in_file);
        let mut dock = cc
            .storage
            .and_then(|s| eframe::get_value::<DockState<Pane>>(s, DOCK_KEY))
            .unwrap_or_else(dock::new);
        dock::translate(&mut dock);
        dock::hide(&mut dock, Pane::Preview);
        dock::hide(&mut dock, Pane::ServerSearch);
        dock::set_homes(settings.homes.clone());
        settings
            .detached
            .retain(|p| !matches!(p, Pane::Preview | Pane::ServerSearch));
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
        let project2 = ProjectPanel::new(settings.project2.clone());
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
            project2,
            sizes: dock::Sizes {
                known: panel_sizes,
                ..Default::default()
            },
            zoom_acc: 0.0,
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
            db: DbPanel::new(),
            preview: Preview::default(),
            plugins: crate::plugins::Plugins::new(
                eframe::storage_dir("Tror").map(|dir| dir.join("plugins")),
                plugins_off,
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
            jump: Default::default(),
            plugin_dialog: None,
            plugin_panels: Default::default(),
            find_files: Default::default(),
            find_out: None,
            update: Default::default(),
            macros: Default::default(),
            macro_dialog: None,
            find: Default::default(),
            floats: Default::default(),
            manager: None,
            plugin_active: None,
            plugin_seen: Default::default(),
            decorations: Default::default(),
            plugin_decos: Default::default(),
            visual: Default::default(),
            drop_hint: None,
            scale: 0.0,
            instance: None,
            recovery: Default::default(),
            confirm: None,
            conflict: None,
            allow_close: false,
            focus_editor: true,
            queued: Vec::new(),
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
        app.next_id = app.next_id.max(top);
        if active < app.docs.len() {
            app.active = active;
        }
        for path in crate::instance::args() {
            app.open_arg(path);
        }
        app.instance = crate::instance::listen(cc.egui_ctx.clone());
        app.sync_plugin_panels();
        if let Some(report) = crate::crash::pending() {
            app.open_path(&report, 0.0);
            app.toast = Some((
                tr!("В прошлый раз TrorPad завершился с ошибкой. Отчёт открыт во вкладке — приложите его к сообщению на GitHub.").to_owned(),
                0.0,
            ));
        }
        app
    }

    pub(super) fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some((text.into(), now));
    }

    pub(super) fn new_doc(&mut self) {
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

    fn command(&self, id: &str) -> Action {
        match id {
            "new" => Action::New,
            "open" => Action::Open,
            "open_folder" => Action::OpenFolder,
            "project" => Action::Toggle(Pane::Project),
            "save" => Action::Save,
            "save_as" => Action::SaveAs,
            "save_all" => Action::SaveAll,
            "print" => Action::Print,
            "quick_open" => Action::Jump(""),
            "palette" => Action::Jump(">"),
            "close_tab" => Action::CloseTab(self.active),
            "reopen" => Action::Reopen,
            "next_tab" => Action::NextTab,
            "prev_tab" => Action::PrevTab,
            "find" => Action::Find,
            "replace" => Action::Replace,
            "find_next" => Action::FindNext,
            "find_prev" => Action::FindPrev,
            "goto" => Action::GotoLine,
            "duplicate_line" => Action::Edit("duplicate_line"),
            "delete_line" => Action::Edit("delete_line"),
            "line_up" => Action::Edit("line_up"),
            "line_down" => Action::Edit("line_down"),
            "upper" => Action::Edit("upper"),
            "lower" => Action::Edit("lower"),
            "trim" => Action::Edit("trim"),
            "bookmark" => Action::Edit("bookmark"),
            "bookmark_next" => Action::Edit("bookmark_next"),
            "bookmark_prev" => Action::Edit("bookmark_prev"),
            "fold" => Action::Fold(0),
            "unfold" => Action::Fold(1),
            "fold_all" => Action::Fold(2),
            "unfold_all" => Action::Fold(3),
            "wrap" => Action::ToggleWrap,
            "visual" => Action::ToggleVisual,
            "theme" => Action::ToggleTheme,
            "settings" => Action::Show(Pane::Settings),
            "hosts" => Action::Toggle(Pane::Hosts),
            "console" => Action::Toggle(Pane::Console),
            "log" => Action::Toggle(Pane::Log),
            "server_search" => Action::FindFiles,
            "db" => Action::Toggle(Pane::Db),
            "compare" => Action::Compare,
            "compare_server" => Action::CompareServer,
            "find_files" => Action::FindFiles,
            "macro_record" => Action::MacroRecord,
            "macro_play" => Action::MacroPlay,
            _ => Action::Preview,
        }
    }

    fn palette(&self) -> Palette {
        self.settings.look.palette()
    }
}
