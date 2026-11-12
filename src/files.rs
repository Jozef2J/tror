use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Button, Key, Rect, RichText, ScrollArea, Sense, TextEdit, Ui};

use crate::app::{dialog, dialog_button, size_label};
use crate::theme::Palette;
use crate::widgets::{self, chevron, date_label, dots_button, paint_icon, refresh_button};

const RESCAN: Duration = Duration::from_millis(2500);

#[derive(Clone, Debug, PartialEq)]
pub struct LocalEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u64>,
}

pub enum ProjectEvent {
    Open(Vec<PathBuf>),
    Upload(Vec<PathBuf>),
    Toast(String),
    Opened(PathBuf),
    View(bool),
}

enum Prompt {
    NewFile(PathBuf, String),
    NewFolder(PathBuf, String),
    Rename(PathBuf, String),
    Delete(Vec<PathBuf>),
}

struct Attrs {
    path: PathBuf,
    is_dir: bool,
    size: u64,
    mtime: Option<u64>,
    mode: u32,
    octal: String,
    readonly: bool,
    recursive: bool,
}

pub struct ProjectPanel {
    pub root: Option<PathBuf>,
    pub tree: bool,
    pub upload_target: Option<String>,
    pub show_size: bool,
    pub show_date: bool,
    cwd: PathBuf,
    cache: HashMap<PathBuf, Vec<LocalEntry>>,
    scanned: HashMap<PathBuf, Instant>,
    expanded: BTreeSet<PathBuf>,
    marked: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    prompt: Option<Prompt>,
    attrs: Option<Attrs>,
    out: Vec<ProjectEvent>,
}

pub fn read_dir(path: &Path) -> Result<Vec<LocalEntry>, String> {
    let mut entries: Vec<LocalEntry> = std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = std::fs::metadata(entry.path())
                .or_else(|_| entry.metadata())
                .ok()?;
            Some(LocalEntry {
                name,
                is_dir: meta.is_dir(),
                size: meta.len(),
                mtime: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase()))
    });
    Ok(entries)
}

struct Row {
    path: PathBuf,
    name: String,
    depth: usize,
    entry: Option<LocalEntry>,
}

fn mode_of(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        if meta.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }
}

fn set_mode(path: &Path, mode: u32, readonly: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = readonly;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        let mut perm = std::fs::metadata(path)?.permissions();
        perm.set_readonly(readonly);
        std::fs::set_permissions(path, perm)
    }
}

fn set_tree(path: &Path, mode: u32, readonly: bool, count: &mut usize) -> std::io::Result<()> {
    set_mode(path, mode, readonly)?;
    *count += 1;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)?.flatten() {
            set_tree(&entry.path(), mode, readonly, count)?;
        }
    }
    Ok(())
}

impl ProjectPanel {
    pub fn new(root: Option<PathBuf>) -> Self {
        let root = root.filter(|p| p.is_dir());
        Self {
            cwd: root.clone().unwrap_or_default(),
            root,
            tree: true,
            upload_target: None,
            show_size: false,
            show_date: false,
            cache: HashMap::new(),
            scanned: HashMap::new(),
            expanded: BTreeSet::new(),
            marked: BTreeSet::new(),
            anchor: None,
            prompt: None,
            attrs: None,
            out: Vec::new(),
        }
    }

    pub fn modal_open(&self) -> bool {
        self.prompt.is_some() || self.attrs.is_some()
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.cwd = root.clone();
        self.root = Some(root.clone());
        self.cache.clear();
        self.scanned.clear();
        self.expanded.clear();
        self.marked.clear();
        self.out.push(ProjectEvent::Opened(root));
    }

    pub fn close(&mut self) {
        self.root = None;
        self.cache.clear();
        self.expanded.clear();
        self.marked.clear();
    }

    pub fn take_events(&mut self) -> Vec<ProjectEvent> {
        std::mem::take(&mut self.out)
    }

    fn entries(&mut self, dir: &Path) -> Vec<LocalEntry> {
        let stale = self
            .scanned
            .get(dir)
            .is_none_or(|t| t.elapsed() > RESCAN);
        if stale {
            let list = read_dir(dir).unwrap_or_default();
            self.cache.insert(dir.to_path_buf(), list);
            self.scanned.insert(dir.to_path_buf(), Instant::now());
        }
        self.cache.get(dir).cloned().unwrap_or_default()
    }

    fn invalidate(&mut self, dir: &Path) {
        self.scanned.remove(dir);
    }

    fn rows(&mut self) -> Vec<Row> {
        let mut rows = Vec::new();
        let Some(root) = self.root.clone() else {
            return rows;
        };
        if !self.tree {
            if self.cwd != root
                && let Some(parent) = self.cwd.parent()
            {
                rows.push(Row {
                    path: parent.to_path_buf(),
                    name: "..".into(),
                    depth: 0,
                    entry: None,
                });
            }
            let cwd = self.cwd.clone();
            for entry in self.entries(&cwd) {
                rows.push(Row {
                    path: cwd.join(&entry.name),
                    name: entry.name.clone(),
                    depth: 0,
                    entry: Some(entry),
                });
            }
            return rows;
        }
        let mut stack = vec![(root, 0usize)];
        let mut order: Vec<Row> = Vec::new();
        while let Some((dir, depth)) = stack.pop() {
            let entries = self.entries(&dir);
            let mut local = Vec::new();
            for entry in entries {
                local.push(Row {
                    path: dir.join(&entry.name),
                    name: entry.name.clone(),
                    depth,
                    entry: Some(entry),
                });
            }
            let at = order
                .iter()
                .position(|r| r.path == dir)
                .map_or(0, |i| i + 1);
            let open: Vec<(PathBuf, usize)> = local
                .iter()
                .filter(|r| {
                    r.entry.as_ref().is_some_and(|e| e.is_dir)
                        && self.expanded.contains(&r.path)
                        && depth < 40
                })
                .map(|r| (r.path.clone(), depth + 1))
                .collect();
            order.splice(at..at, local);
            stack.extend(open.into_iter().rev());
        }
        rows.extend(order);
        rows
    }

    fn chosen(&self, rows: &[Row], clicked: &Path) -> Vec<(PathBuf, bool)> {
        let pick = |r: &Row| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir));
        if self.marked.contains(clicked) {
            rows.iter()
                .filter(|r| r.entry.is_some() && self.marked.contains(&r.path))
                .map(pick)
                .collect()
        } else {
            rows.iter()
                .filter(|r| r.entry.is_some() && r.path == clicked)
                .map(pick)
                .collect()
        }
    }

    fn pick_root(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.set_root(dir);
        }
    }

    fn create_items(&mut self, ui: &mut Ui, dir: &Path) {
        if ui.button("Новый файл…").clicked() {
            self.prompt = Some(Prompt::NewFile(dir.to_path_buf(), String::new()));
            ui.close();
        }
        if ui.button("Новая папка…").clicked() {
            self.prompt = Some(Prompt::NewFolder(dir.to_path_buf(), String::new()));
            ui.close();
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, recent: &[PathBuf]) {
        ui.take_available_width();
        ui.add_space(8.0);
        let Some(root) = self.root.clone() else {
            ui.label(RichText::new("Папка проекта не открыта.").color(pal.subtext));
            ui.add_space(4.0);
            if ui
                .add(Button::new("Открыть папку…").fill(pal.surface))
                .clicked()
            {
                self.pick_root();
            }
            if !recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Недавние").size(12.0).color(pal.faint));
                for path in recent.iter().take(8) {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    if ui
                        .add(Button::new(name).frame(false).truncate())
                        .on_hover_text(path.display().to_string())
                        .clicked()
                        && path.is_dir()
                    {
                        self.set_root(path.clone());
                    }
                }
            }
            return;
        };
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let shown = if self.tree || self.cwd == root {
                root.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string())
            } else {
                self.cwd
                    .strip_prefix(&root)
                    .map(|p| {
                        format!(
                            "{}/{}",
                            root.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                            p.display()
                        )
                    })
                    .unwrap_or_else(|_| self.cwd.display().to_string())
            };
            let width = (ui.available_width() - 56.0).max(20.0);
            let hover = self.cwd.display().to_string();
            ui.allocate_ui_with_layout(
                egui::vec2(width, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(width);
                    ui.add(
                        egui::Label::new(RichText::new(shown).family(crate::theme::medium()))
                            .truncate(),
                    )
                    .on_hover_text(hover);
                },
            );
            if refresh_button(ui, pal).on_hover_text("Обновить").clicked() {
                refresh = true;
            }
            let dots = dots_button(ui, pal).on_hover_text("Ещё");
            egui::Popup::menu(&dots).show(|ui| {
                ui.set_min_width(230.0);
                if ui.button("Открыть другую папку…").clicked() {
                    ui.close();
                    self.pick_root();
                }
                if !recent.is_empty() {
                    ui.menu_button("Недавние папки", |ui| {
                        for path in recent {
                            if ui.button(path.display().to_string()).clicked() && path.is_dir() {
                                self.set_root(path.clone());
                                ui.close();
                            }
                        }
                    });
                }
                if ui.button("Закрыть папку").clicked() {
                    self.close();
                    ui.close();
                }
                ui.separator();
                let dir = if self.tree { root.clone() } else { self.cwd.clone() };
                self.create_items(ui, &dir);
                if ui.button("Показать в файловом менеджере").clicked() {
                    let _ = crate::preview::open_folder(&dir);
                    ui.close();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Вид:");
                    if ui.selectable_label(!self.tree, "список").clicked() {
                        self.out.push(ProjectEvent::View(false));
                    }
                    if ui.selectable_label(self.tree, "дерево").clicked() {
                        self.out.push(ProjectEvent::View(true));
                    }
                });
                ui.checkbox(&mut self.show_size, "Показывать размер");
                ui.checkbox(&mut self.show_date, "Показывать дату изменения");
            });
        });
        if refresh {
            self.scanned.clear();
        }
        self.listing(ui, pal);
    }

    fn listing(&mut self, ui: &mut Ui, pal: &Palette) {
        let row_h = 22.0;
        let rows = self.rows();
        let tree = self.tree;
        let mut toggle = None;
        let mut go = None;
        let mut open = Vec::new();
        let outer = ui.available_rect_before_wrap();
        let blank = ui.interact(outer, ui.scope_id().with("tror_project_blank"), Sense::click());
        let area = ScrollArea::vertical()
            .id_salt("tror_project_rows")
            .auto_shrink(false)
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let row = &rows[index];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let is_dir = row.entry.as_ref().is_none_or(|e| e.is_dir);
                    let marked = row.entry.is_some() && self.marked.contains(&row.path);
                    let painter = ui.painter_at(rect);
                    if marked {
                        painter.rect_filled(rect, 5.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 5.0, pal.crust);
                    }
                    let indent = row.depth as f32 * 14.0 + if tree { 14.0 } else { 0.0 };
                    if tree && is_dir && row.entry.is_some() {
                        chevron(
                            &painter,
                            egui::pos2(rect.left() + indent - 6.0, rect.center().y),
                            self.expanded.contains(&row.path),
                            pal.faint,
                        );
                    }
                    paint_icon(
                        &painter,
                        egui::pos2(rect.left() + indent + 14.0, rect.center().y),
                        is_dir,
                        &row.name,
                        pal,
                    );
                    let mut right = rect.right() - 6.0;
                    let small = egui::FontId::proportional(11.5);
                    if self.show_date && rect.width() - indent > 230.0 {
                        if let Some(secs) = row.entry.as_ref().and_then(|e| e.mtime) {
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
                    if self.show_size && rect.width() - indent > 150.0 {
                        if let Some(e) = row.entry.as_ref().filter(|e| !e.is_dir) {
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
                    let clip = Rect::from_min_max(
                        egui::pos2(rect.left() + indent + 28.0, rect.top()),
                        egui::pos2(right.max(rect.left() + indent + 60.0), rect.bottom()),
                    );
                    ui.painter_at(clip).text(
                        egui::pos2(clip.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &row.name,
                        egui::FontId::proportional(13.0),
                        if row.name.starts_with('.') && row.name != ".." {
                            pal.subtext
                        } else {
                            pal.text
                        },
                    );
                    if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        let on_chevron = tree
                            && is_dir
                            && row.entry.is_some()
                            && response
                                .interact_pointer_pos()
                                .is_some_and(|p| p.x < rect.left() + indent + 4.0);
                        if on_chevron {
                            toggle = Some(row.path.clone());
                        } else if row.entry.is_none() {
                            self.marked.clear();
                        } else if modifiers.shift && self.anchor.is_some() {
                            let anchor = self.anchor.clone().unwrap_or_default();
                            let a = rows.iter().position(|r| r.path == anchor).unwrap_or(index);
                            if !modifiers.command {
                                self.marked.clear();
                            }
                            for r in &rows[a.min(index)..=a.max(index)] {
                                if r.entry.is_some() {
                                    self.marked.insert(r.path.clone());
                                }
                            }
                        } else if modifiers.command {
                            if !self.marked.remove(&row.path) {
                                self.marked.insert(row.path.clone());
                            }
                            self.anchor = Some(row.path.clone());
                        } else {
                            self.marked.clear();
                            self.marked.insert(row.path.clone());
                            self.anchor = Some(row.path.clone());
                            if tree && is_dir {
                                toggle = Some(row.path.clone());
                            }
                        }
                    }
                    if response.double_clicked() {
                        if !is_dir {
                            open.push(row.path.clone());
                        } else if !tree {
                            go = Some(row.path.clone());
                        }
                    }
                    if row.entry.is_some() {
                        let path = row.path.clone();
                        response.context_menu(|ui| {
                            ui.set_min_width(220.0);
                            let chosen = self.chosen(&rows, &path);
                            let files: Vec<PathBuf> = chosen
                                .iter()
                                .filter(|(_, d)| !d)
                                .map(|(p, _)| p.clone())
                                .collect();
                            if !files.is_empty() && ui.button("Открыть").clicked() {
                                open.extend(files.iter().cloned());
                            }
                            if chosen.len() == 1 && is_dir {
                                self.create_items(ui, &path);
                            }
                            if chosen.len() == 1 && !is_dir
                                && let Some(dir) = path.parent()
                            {
                                self.create_items(ui, &dir.to_path_buf());
                            }
                            if let Some(target) = &self.upload_target
                                && ui.button(format!("Загрузить на сервер ({target})")).clicked()
                            {
                                self.out.push(ProjectEvent::Upload(
                                    chosen.iter().map(|(p, _)| p.clone()).collect(),
                                ));
                            }
                            ui.separator();
                            if ui.button("Копировать путь").clicked() {
                                let text: Vec<String> =
                                    chosen.iter().map(|(p, _)| p.display().to_string()).collect();
                                ui.ctx().copy_text(text.join("\n"));
                            }
                            if ui.button("Показать в файловом менеджере").clicked() {
                                let dir = if is_dir {
                                    path.clone()
                                } else {
                                    path.parent().map(Path::to_path_buf).unwrap_or_default()
                                };
                                let _ = crate::preview::open_folder(&dir);
                            }
                            if chosen.len() == 1 && ui.button("Права и атрибуты…").clicked()
                                && let Ok(meta) = std::fs::metadata(&path)
                            {
                                let mode = mode_of(&meta);
                                self.attrs = Some(Attrs {
                                    path: path.clone(),
                                    is_dir: meta.is_dir(),
                                    size: meta.len(),
                                    mtime: meta
                                        .modified()
                                        .ok()
                                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                        .map(|d| d.as_secs()),
                                    mode,
                                    octal: format!("{mode:03o}"),
                                    readonly: meta.permissions().readonly(),
                                    recursive: false,
                                });
                            }
                            if chosen.len() == 1 && ui.button("Переименовать…").clicked() {
                                self.prompt = Some(Prompt::Rename(
                                    path.clone(),
                                    path.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                ));
                            }
                            if ui.button("Удалить…").clicked() {
                                self.prompt = Some(Prompt::Delete(
                                    chosen.iter().map(|(p, _)| p.clone()).collect(),
                                ));
                            }
                        });
                    }
                }
            });
        let _ = area;
        if blank.clicked() {
            self.marked.clear();
        }
        let dir = if tree {
            self.root.clone().unwrap_or_default()
        } else {
            self.cwd.clone()
        };
        egui::Popup::context_menu(&blank).show(|ui| {
            ui.set_min_width(220.0);
            self.create_items(ui, &dir);
            ui.separator();
            if ui.button("Обновить").clicked() {
                self.scanned.clear();
                ui.close();
            }
            if ui.button("Показать в файловом менеджере").clicked() {
                let _ = crate::preview::open_folder(&dir);
                ui.close();
            }
        });
        if let Some(path) = toggle
            && !self.expanded.remove(&path)
        {
            self.expanded.insert(path);
        }
        if let Some(path) = go {
            self.cwd = path;
            self.marked.clear();
        }
        if !open.is_empty() {
            self.out.push(ProjectEvent::Open(open));
        }
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        self.prompt_dialog(ctx, pal);
        self.attrs_dialog(ctx, pal);
    }

    fn attrs_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(state) = &mut self.attrs else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let modal = dialog("tror_local_attrs", pal).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                RichText::new(format!(
                    "Свойства «{}»",
                    state
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ))
                .strong(),
            );
            ui.add_space(6.0);
            egui::Grid::new("tror_local_attrs_info")
                .num_columns(2)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Путь").color(pal.subtext));
                    ui.add(egui::Label::new(state.path.display().to_string()).truncate());
                    ui.end_row();
                    if !state.is_dir {
                        ui.label(RichText::new("Размер").color(pal.subtext));
                        ui.label(if state.size < 1024 {
                            size_label(state.size as usize)
                        } else {
                            format!("{} ({} Б)", size_label(state.size as usize), state.size)
                        });
                        ui.end_row();
                    }
                    if let Some(mtime) = state.mtime {
                        ui.label(RichText::new("Изменён").color(pal.subtext));
                        ui.label(date_label(mtime));
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            if cfg!(unix) {
                ui.label(RichText::new("Права доступа").family(crate::theme::medium()));
                widgets::perm_grid(ui, pal, &mut state.mode, &mut state.octal);
            } else {
                ui.checkbox(&mut state.readonly, "Только чтение");
            }
            if state.is_dir {
                ui.checkbox(&mut state.recursive, "Применить ко всему содержимому");
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
            let mut count = 0;
            let result = if state.recursive {
                set_tree(&state.path, state.mode, state.readonly, &mut count)
            } else {
                count = 1;
                set_mode(&state.path, state.mode, state.readonly)
            };
            self.out.push(ProjectEvent::Toast(match result {
                Ok(()) => format!("Права изменены: {count}"),
                Err(e) => format!("Права не изменены: {e}"),
            }));
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
        let modal = dialog("tror_local_prompt", pal).show(ctx, |ui| {
            ui.set_width(360.0);
            let (title, input, action) = match prompt {
                Prompt::NewFile(dir, name) => (format!("Новый файл в {}", dir.display()), Some(name), "Создать"),
                Prompt::NewFolder(dir, name) => (format!("Новая папка в {}", dir.display()), Some(name), "Создать"),
                Prompt::Rename(_, name) => ("Переименовать".to_owned(), Some(name), "Переименовать"),
                Prompt::Delete(items) => {
                    let text = if items.len() == 1 {
                        format!(
                            "Удалить «{}» с диска?",
                            items[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        )
                    } else {
                        format!("Удалить с диска выбранные объекты ({})?", items.len())
                    };
                    ui.label(text);
                    if items.iter().any(|p| p.is_dir()) {
                        ui.label(
                            RichText::new("Папки удаляются вместе со всем содержимым, мимо корзины. Отменить это нельзя.")
                                .color(pal.warn),
                        );
                    }
                    (String::new(), None, "Удалить")
                }
            };
            if let Some(input) = input {
                ui.add(egui::Label::new(title).truncate());
                ui.add_space(4.0);
                ui.add(TextEdit::singleline(input).desired_width(f32::INFINITY))
                    .request_focus();
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
                (!name.is_empty() && !name.contains(['/', '\\'])).then(|| name.to_owned())
            };
            let result = match self.prompt.take() {
                Some(Prompt::NewFile(dir, name)) => valid(&name).map(|name| {
                    let path = dir.join(&name);
                    self.invalidate(&dir);
                    self.expanded.insert(dir.clone());
                    if path.exists() {
                        Err(format!("{name} уже существует"))
                    } else {
                        std::fs::write(&path, b"")
                            .map(|()| {
                                self.out.push(ProjectEvent::Open(vec![path.clone()]));
                                format!("Создан файл {name}")
                            })
                            .map_err(|e| e.to_string())
                    }
                }),
                Some(Prompt::NewFolder(dir, name)) => valid(&name).map(|name| {
                    self.invalidate(&dir);
                    self.expanded.insert(dir.clone());
                    std::fs::create_dir(dir.join(&name))
                        .map(|()| format!("Создана папка {name}"))
                        .map_err(|e| e.to_string())
                }),
                Some(Prompt::Rename(from, name)) => valid(&name).map(|name| {
                    let parent = from.parent().map(Path::to_path_buf).unwrap_or_default();
                    self.invalidate(&parent);
                    let to = parent.join(&name);
                    if to.exists() {
                        Err(format!("{name} уже существует"))
                    } else {
                        std::fs::rename(&from, &to)
                            .map(|()| format!("Переименовано в {name}"))
                            .map_err(|e| e.to_string())
                    }
                }),
                Some(Prompt::Delete(items)) => {
                    let mut removed = 0;
                    let mut error = None;
                    for path in items {
                        if let Some(parent) = path.parent() {
                            self.invalidate(parent);
                        }
                        let result = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };
                        match result {
                            Ok(()) => {
                                removed += 1;
                                self.marked.remove(&path);
                            }
                            Err(e) => error = Some(format!("{}: {e}", path.display())),
                        }
                    }
                    Some(match error {
                        Some(e) => Err(e),
                        None => Ok(format!("Удалено: {removed}")),
                    })
                }
                None => None,
            };
            if let Some(result) = result {
                self.out.push(ProjectEvent::Toast(match result {
                    Ok(text) => text,
                    Err(e) => format!("Не получилось: {e}"),
                }));
            }
        } else if cancelled || modal.should_close() {
            self.prompt = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_rows_follow_expansion() {
        let dir = std::env::temp_dir().join(format!("tror-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/inner")).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("README.md"), "x").unwrap();
        std::fs::write(dir.join("src/main.rs"), "y").unwrap();
        std::fs::write(dir.join("src/inner/deep.rs"), "z").unwrap();
        let mut panel = ProjectPanel::new(Some(dir.clone()));
        let names = |panel: &mut ProjectPanel| -> Vec<(String, usize)> {
            panel.rows().into_iter().map(|r| (r.name, r.depth)).collect()
        };
        assert_eq!(
            names(&mut panel),
            [("assets".into(), 0), ("src".into(), 0), ("README.md".into(), 0)]
        );
        panel.expanded.insert(dir.join("src"));
        panel.expanded.insert(dir.join("src/inner"));
        assert_eq!(
            names(&mut panel),
            [
                ("assets".to_owned(), 0),
                ("src".to_owned(), 0),
                ("inner".to_owned(), 1),
                ("deep.rs".to_owned(), 2),
                ("main.rs".to_owned(), 1),
                ("README.md".to_owned(), 0)
            ]
        );
        panel.tree = false;
        panel.cwd = dir.join("src");
        assert_eq!(
            names(&mut panel),
            [("..".to_owned(), 0), ("inner".to_owned(), 0), ("main.rs".to_owned(), 0)]
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
