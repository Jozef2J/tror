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
    Transfer(Vec<PathBuf>, bool),
    Dropped(Vec<PathBuf>, egui::Pos2),
    Move(Vec<PathBuf>, PathBuf, bool),
}

fn free_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name).to_string_lossy().into_owned();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

pub fn transfer(paths: &[PathBuf], dir: &Path, keep: bool) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut done = Vec::new();
    for path in paths {
        let Some(name) = path.file_name() else {
            continue;
        };
        if dir.starts_with(path) {
            return Err(tf!("нельзя переместить «{}» внутрь себя", "cannot move \"{}\" into itself", path.display()));
        }
        if path.parent() == Some(dir) && !keep {
            continue;
        }
        let target = free_name(dir, name);
        let result = if keep {
            copy_tree(path, &target)
        } else {
            std::fs::rename(path, &target).or_else(|_| {
                copy_tree(path, &target)?;
                if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                }
            })
        };
        result.map_err(|e| format!("{}: {e}", path.display()))?;
        done.push((path.clone(), target));
    }
    Ok(done)
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
    sort: crate::widgets::Sort,
    cwd: PathBuf,
    cache: HashMap<PathBuf, Vec<LocalEntry>>,
    scanned: HashMap<PathBuf, Instant>,
    expanded: BTreeSet<PathBuf>,
    marked: BTreeSet<PathBuf>,
    pub drag_out: Option<Vec<PathBuf>>,
    pub peer: Option<String>,
    pub area: egui::Rect,
    pub external_hover: Option<PathBuf>,
    drop_dirs: Vec<(Rect, PathBuf)>,
    drop_dir: PathBuf,
    dragging: Option<Vec<PathBuf>>,
    dragged_out: bool,
    anchor: Option<PathBuf>,
    clip: Option<(Vec<PathBuf>, bool)>,
    archiving: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
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
            sort: Default::default(),
            cache: HashMap::new(),
            scanned: HashMap::new(),
            expanded: BTreeSet::new(),
            marked: BTreeSet::new(),
            drag_out: None,
            peer: None,
            area: egui::Rect::NOTHING,
            external_hover: None,
            drop_dirs: Vec::new(),
            drop_dir: PathBuf::new(),
            dragging: None,
            dragged_out: false,
            anchor: None,
            clip: None,
            archiving: None,
            prompt: None,
            attrs: None,
            out: Vec::new(),
        }
    }

    pub fn dragging(&self) -> Option<&[PathBuf]> {
        self.dragging.as_deref().filter(|_| !self.dragged_out)
    }

    pub fn start_drag_out(&mut self) {
        if let Some(paths) = self.dragging.clone()
            && !self.dragged_out
        {
            self.dragged_out = true;
            self.drag_out = Some(paths);
        }
    }

    pub fn drop_target(&self, pos: egui::Pos2) -> Option<PathBuf> {
        self.root.as_ref()?;
        if !self.area.contains(pos) {
            return None;
        }
        Some(
            self.drop_dirs
                .iter()
                .find(|(rect, _)| rect.contains(pos))
                .map(|(_, dir)| dir.clone())
                .unwrap_or_else(|| self.drop_dir.clone()),
        )
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

    pub fn target_dir(&self) -> Option<PathBuf> {
        if self.tree { self.root.clone() } else { Some(self.cwd.clone()) }
    }

    pub fn reload(&mut self) {
        self.scanned.clear();
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
            let mut entries = self.entries(&cwd);
            self.sort.apply(&mut entries, |e| (e.is_dir, e.name.as_str(), e.size, 0, e.mtime.unwrap_or(0)));
            for entry in entries {
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
            let mut entries = self.entries(&dir);
            self.sort.apply(&mut entries, |e| (e.is_dir, e.name.as_str(), e.size, 0, e.mtime.unwrap_or(0)));
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

    fn paste_item(&mut self, ui: &mut Ui, dir: &Path) {
        if let Some((paths, keep)) = self.clip.clone()
            && ui.button(tr!("Вставить")).clicked()
        {
            self.out.push(ProjectEvent::Move(paths, dir.to_path_buf(), keep));
            if !keep {
                self.clip = None;
            }
            ui.close();
        }
    }

    fn create_items(&mut self, ui: &mut Ui, dir: &Path) {
        if ui.button(tr!("Новый файл…")).clicked() {
            self.prompt = Some(Prompt::NewFile(dir.to_path_buf(), String::new()));
            ui.close();
        }
        if ui.button(tr!("Новая папка…")).clicked() {
            self.prompt = Some(Prompt::NewFolder(dir.to_path_buf(), String::new()));
            ui.close();
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, recent: &[PathBuf]) {
        if let Some(rx) = &self.archiving
            && let Ok(result) = rx.try_recv()
        {
            self.archiving = None;
            self.out.push(ProjectEvent::Toast(match result {
                Ok(path) => tf!("Архив создан: {}", "Archive created: {}", path.display()),
                Err(e) => tf!("Не удалось создать архив: {e}", "Could not create the archive: {e}"),
            }));
            self.reload();
        }
        ui.take_available_width();
        ui.add_space(8.0);
        let Some(root) = self.root.clone() else {
            ui.label(RichText::new(tr!("Папка проекта не открыта.")).color(pal.subtext));
            ui.add_space(4.0);
            if ui
                .add(Button::new(tr!("Открыть папку…")))
                .clicked()
            {
                self.pick_root();
            }
            if !recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr!("Недавние")).small().color(pal.faint));
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
            if refresh_button(ui, pal).on_hover_text(tr!("Обновить")).clicked() {
                refresh = true;
            }
            let dots = dots_button(ui, pal).on_hover_text(tr!("Ещё"));
            egui::Popup::menu(&dots)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                ui.set_min_width(230.0);
                if ui.button(tr!("Открыть другую папку…")).clicked() {
                    ui.close();
                    self.pick_root();
                }
                if !recent.is_empty() {
                    ui.menu_button(tr!("Недавние папки"), |ui| {
                        for path in recent {
                            if ui.button(path.display().to_string()).clicked() && path.is_dir() {
                                self.set_root(path.clone());
                                ui.close();
                            }
                        }
                    });
                }
                if ui.button(tr!("Закрыть папку")).clicked() {
                    self.close();
                    ui.close();
                }
                ui.separator();
                let dir = if self.tree { root.clone() } else { self.cwd.clone() };
                self.create_items(ui, &dir);
                if ui.button(tr!("Показать в файловом менеджере")).clicked() {
                    let _ = crate::preview::open_folder(&dir);
                    ui.close();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(tr!("Вид:"));
                    if ui.selectable_label(!self.tree, tr!("список")).clicked() {
                        self.out.push(ProjectEvent::View(false));
                    }
                    if ui.selectable_label(self.tree, tr!("дерево")).clicked() {
                        self.out.push(ProjectEvent::View(true));
                    }
                });
                ui.checkbox(&mut self.show_size, tr!("Показывать размер"));
                ui.checkbox(&mut self.show_date, tr!("Показывать дату изменения"));
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
        let widths =
            crate::widgets::column_header(ui, pal, [self.show_size, false, self.show_date], &mut self.sort);
        ui.spacing_mut().item_spacing.y = 0.0;
        let outer = ui.available_rect_before_wrap();
        self.area = outer;
        if let (Some(paths), Some(pos)) = (&self.dragging, ui.ctx().pointer_latest_pos())
            && !self.dragged_out
            && !outer.contains(pos)
        {
            let text = match paths.len() {
                1 => paths[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                n => tf!("файлов: {n}", "files: {n}"),
            };
            egui::Area::new(ui.scope_id().with("tror_project_drag"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(14.0, 10.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(text).small());
                    });
                });
        }
        let blank = ui.interact(outer, ui.scope_id().with("tror_project_blank"), Sense::click());
        let dir = if tree {
            self.root.clone().unwrap_or_default()
        } else {
            self.cwd.clone()
        };
        let pointer = ui.ctx().pointer_latest_pos();
        let mut target: Option<PathBuf> = None;
        self.drop_dirs.clear();
        self.drop_dir = dir.clone();
        let external = self.external_hover.clone();
        let mut released: Option<(Vec<PathBuf>, egui::Pos2)> = None;
        let wide = crate::widgets::rows_width(
            ui,
            rows.iter().map(|r| {
                (r.depth as f32 * 14.0 + if tree { 14.0 } else { 0.0 }, r.name.as_str())
            }),
            widths,
        );
        let area = ScrollArea::both()
            .id_salt("tror_project_rows")
            .auto_shrink(false)
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let view = ui.clip_rect();
                for index in range {
                    let row = &rows[index];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(wide, row_h),
                        Sense::click_and_drag(),
                    );
                    if response.dragged()
                        && row.entry.is_some()
                        && !self.dragged_out
                        && ui.ctx().pointer_latest_pos().is_none_or(|p| {
                            !ui.ctx().content_rect().contains(p)
                        })
                    {
                        self.dragged_out = true;
                        self.drag_out = Some(if self.marked.contains(&row.path) {
                            self.marked.iter().cloned().collect()
                        } else {
                            vec![row.path.clone()]
                        });
                    }
                    if response.drag_started() && row.entry.is_some() {
                        self.dragging = Some(if self.marked.contains(&row.path) {
                            self.marked.iter().cloned().collect()
                        } else {
                            vec![row.path.clone()]
                        });
                    }
                    if response.drag_stopped() {
                        if !self.dragged_out
                            && let Some(paths) = self.dragging.take()
                            && let Some(pos) = ui.ctx().pointer_latest_pos()
                        {
                            released = Some((paths, pos));
                        }
                        self.dragging = None;
                        self.dragged_out = false;
                    }
                    let is_dir = row.entry.as_ref().is_none_or(|e| e.is_dir);
                    let over = (self.dragging.is_some() || released.is_some())
                        && pointer.is_some_and(|p| rect.contains(p) && view.contains(p));
                    if over {
                        target = if is_dir {
                            Some(row.path.clone())
                        } else {
                            row.path.parent().map(Path::to_path_buf)
                        };
                    }
                    let marked = row.entry.is_some() && self.marked.contains(&row.path);
                    let painter = ui.painter_at(rect);
                    if marked {
                        painter.rect_filled(rect, 5.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 5.0, pal.crust);
                    }
                    if is_dir && row.entry.is_some() {
                        self.drop_dirs.push((rect.intersect(view), row.path.clone()));
                    }
                    if (over && is_dir && self.dragging.is_some())
                        || (is_dir && external.as_ref() == Some(&row.path))
                    {
                        painter.rect_stroke(
                            rect.shrink(1.0),
                            5.0,
                            egui::Stroke::new(1.5, pal.accent),
                            egui::StrokeKind::Inside,
                        );
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
                    let mut right = view.right().min(rect.right()) - 6.0;
                    let entry = row.entry.as_ref();
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[2],
                        entry.and_then(|e| e.mtime).map(date_label),
                        pal.faint,
                    );
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[0],
                        entry
                            .filter(|e| !e.is_dir)
                            .map(|e| size_label(e.size as usize)),
                        pal.faint,
                    );
                    let clip = Rect::from_min_max(
                        egui::pos2(rect.left() + indent + 28.0, rect.top()),
                        egui::pos2(right.max(rect.left() + indent + 28.0), rect.bottom()),
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
                            if !files.is_empty() && ui.button(tr!("Открыть")).clicked() {
                                open.extend(files.iter().cloned());
                            }
                            let picked: Vec<PathBuf> = chosen.iter().map(|(p, _)| p.clone()).collect();
                            if ui.button(tr!("Вырезать")).clicked() {
                                self.clip = Some((picked.clone(), false));
                            }
                            if ui.button(tr!("Копировать")).clicked() {
                                self.clip = Some((picked, true));
                            }
                            if chosen.len() == 1 && is_dir {
                                self.paste_item(ui, &path);
                            }
                            ui.separator();
                            if chosen.len() == 1 && is_dir {
                                self.create_items(ui, &path);
                            }
                            if chosen.len() == 1 && !is_dir
                                && let Some(dir) = path.parent()
                            {
                                self.create_items(ui, &dir.to_path_buf());
                            }
                            if let Some(peer) = self.peer.clone() {
                                let paths: Vec<PathBuf> = chosen.iter().map(|(p, _)| p.clone()).collect();
                                if ui.button(tf!("Копировать в «{peer}»", "Copy to \"{peer}\"")).clicked() {
                                    self.out.push(ProjectEvent::Transfer(paths.clone(), true));
                                }
                                if ui.button(tf!("Переместить в «{peer}»", "Move to \"{peer}\"")).clicked() {
                                    self.out.push(ProjectEvent::Transfer(paths, false));
                                }
                            }
                            if let Some(target) = &self.upload_target
                                && ui.button(tf!("Загрузить на сервер ({target})", "Upload to server ({target})")).clicked()
                            {
                                self.out.push(ProjectEvent::Upload(
                                    chosen.iter().map(|(p, _)| p.clone()).collect(),
                                ));
                            }
                            let parent = chosen.first().and_then(|(p, _)| p.parent().map(Path::to_path_buf));
                            if self.archiving.is_none()
                                && chosen.iter().all(|(p, _)| p.parent().map(Path::to_path_buf) == parent)
                            {
                                ui.menu_button(tr!("Архивировать"), |ui| {
                                    for format in crate::archive::formats() {
                                        if ui.button(format.label()).clicked() {
                                            let paths: Vec<PathBuf> =
                                                chosen.iter().map(|(p, _)| p.clone()).collect();
                                            let (tx, rx) = std::sync::mpsc::channel();
                                            let ctx = ui.ctx().clone();
                                            std::thread::spawn(move || {
                                                let _ = tx.send(crate::archive::create(&paths, format));
                                                ctx.request_repaint();
                                            });
                                            self.archiving = Some(rx);
                                            self.out.push(ProjectEvent::Toast(tr!("Архивирование…").into()));
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            ui.separator();
                            if ui.button(tr!("Копировать путь")).clicked() {
                                let text: Vec<String> =
                                    chosen.iter().map(|(p, _)| p.display().to_string()).collect();
                                ui.ctx().copy_text(text.join("\n"));
                            }
                            if ui.button(tr!("Показать в файловом менеджере")).clicked() {
                                let dir = if is_dir {
                                    path.clone()
                                } else {
                                    path.parent().map(Path::to_path_buf).unwrap_or_default()
                                };
                                let _ = crate::preview::open_folder(&dir);
                            }
                            if chosen.len() == 1 && ui.button(tr!("Права и атрибуты…")).clicked()
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
                            if chosen.len() == 1 && ui.button(tr!("Переименовать…")).clicked() {
                                self.prompt = Some(Prompt::Rename(
                                    path.clone(),
                                    path.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                ));
                            }
                            if ui.button(tr!("Удалить…")).clicked() {
                                self.prompt = Some(Prompt::Delete(
                                    chosen.iter().map(|(p, _)| p.clone()).collect(),
                                ));
                            }
                        });
                    }
                }
            });
        let _ = area;
        if external.as_ref() == Some(&dir) {
            ui.painter().rect_stroke(
                outer.shrink(2.0),
                6.0,
                egui::Stroke::new(1.5, pal.accent),
                egui::StrokeKind::Inside,
            );
        }
        if (self.dragging.is_some() || released.is_some())
            && target.is_none()
            && pointer.is_some_and(|p| outer.contains(p))
        {
            target = Some(dir.clone());
        }
        let command = ui.input(|i| i.modifiers.command);
        let valid = |paths: &[PathBuf], to: &Path, keep: bool| {
            !paths.iter().any(|p| to.starts_with(p))
                && (keep || !paths.iter().all(|p| p.parent() == Some(to)))
        };
        if let Some((paths, pos)) = released {
            if outer.contains(pos) {
                if let Some(to) = target.clone()
                    && valid(&paths, &to, command)
                {
                    self.out.push(ProjectEvent::Move(paths, to, command));
                }
            } else {
                self.out.push(ProjectEvent::Dropped(paths, pos));
            }
        }
        if let (Some(paths), Some(pos)) = (&self.dragging, pointer)
            && !self.dragged_out
            && outer.contains(pos)
            && let Some(to) = &target
            && valid(paths, to, command)
        {
            let name = match paths.len() {
                1 => paths[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                n => tf!("файлов: {n}", "files: {n}"),
            };
            let folder = to
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| to.display().to_string());
            let verb = if command { tr!("копировать в") } else { tr!("переместить в") };
            egui::Area::new(ui.scope_id().with("tror_project_move"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(14.0, 10.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(format!("{name} → {verb} «{folder}»")).small());
                    });
                });
        }
        if blank.clicked() {
            self.marked.clear();
        }
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(outer) {
            ui.memory_mut(|m| m.request_focus(blank.id));
        }
        if ui.memory(|m| m.has_focus(blank.id)) && self.prompt.is_none() {
            let chosen: Vec<PathBuf> = rows
                .iter()
                .filter(|r| r.entry.is_some() && self.marked.contains(&r.path))
                .map(|r| r.path.clone())
                .collect();
            let (delete, rename) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F2),
                )
            });
            let (cut, copy, paste) = ui.input(|i| {
                let mut found = (false, false, false);
                for event in &i.events {
                    match event {
                        egui::Event::Cut => found.0 = true,
                        egui::Event::Copy => found.1 = true,
                        egui::Event::Paste(_) => found.2 = true,
                        _ => {}
                    }
                }
                found
            });
            if (cut || copy) && !chosen.is_empty() {
                self.clip = Some((chosen.clone(), copy));
            }
            if paste && let Some((paths, keep)) = self.clip.clone() {
                let to = match chosen.as_slice() {
                    [one] if one.is_dir() => one.clone(),
                    _ => dir.clone(),
                };
                self.out.push(ProjectEvent::Move(paths, to, keep));
                if !keep {
                    self.clip = None;
                }
            }
            if delete && !chosen.is_empty() {
                self.prompt = Some(Prompt::Delete(chosen));
            } else if rename && chosen.len() == 1 {
                let name = chosen[0]
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.prompt = Some(Prompt::Rename(chosen[0].clone(), name));
            }
        }
        egui::Popup::context_menu(&blank).show(|ui| {
            ui.set_min_width(220.0);
            self.paste_item(ui, &dir);
            self.create_items(ui, &dir);
            ui.separator();
            if ui.button(tr!("Обновить")).clicked() {
                self.scanned.clear();
                ui.close();
            }
            if ui.button(tr!("Показать в файловом менеджере")).clicked() {
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
                RichText::new(tf!("Свойства «{}»", "Properties of \"{}\"",
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
                    ui.label(RichText::new(tr!("Путь")).color(pal.subtext));
                    ui.add(egui::Label::new(state.path.display().to_string()).truncate());
                    ui.end_row();
                    if !state.is_dir {
                        ui.label(RichText::new(tr!("Размер")).color(pal.subtext));
                        ui.label(if state.size < 1024 {
                            size_label(state.size as usize)
                        } else {
                            tf!("{} ({} Б)", "{} ({} B)", size_label(state.size as usize), state.size)
                        });
                        ui.end_row();
                    }
                    if let Some(mtime) = state.mtime {
                        ui.label(RichText::new(tr!("Изменён")).color(pal.subtext));
                        ui.label(date_label(mtime));
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            if cfg!(unix) {
                ui.label(RichText::new(tr!("Права доступа")).family(crate::theme::medium()));
                widgets::perm_grid(ui, pal, &mut state.mode, &mut state.octal);
            } else {
                ui.checkbox(&mut state.readonly, tr!("Только чтение"));
            }
            if state.is_dir {
                ui.checkbox(&mut state.recursive, tr!("Применить ко всему содержимому"));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Применить"), true) {
                    apply = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
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
                Ok(()) => tf!("Права изменены: {count}", "Permissions changed: {count}"),
                Err(e) => tf!("Права не изменены: {e}", "Permissions not changed: {e}"),
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
                Prompt::NewFile(dir, name) => (tf!("Новый файл в {}", "New file in {}", dir.display()), Some(name), tr!("Создать")),
                Prompt::NewFolder(dir, name) => (tf!("Новая папка в {}", "New folder in {}", dir.display()), Some(name), tr!("Создать")),
                Prompt::Rename(_, name) => (tr!("Переименовать").to_owned(), Some(name), tr!("Переименовать")),
                Prompt::Delete(items) => {
                    let text = if items.len() == 1 {
                        tf!("Удалить «{}» с диска?", "Delete \"{}\" from disk?",
                            items[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        )
                    } else {
                        tf!("Удалить с диска выбранные объекты ({})?", "Delete the selected items from disk ({})?", items.len())
                    };
                    ui.label(text);
                    if items.iter().any(|p| p.is_dir()) {
                        ui.label(
                            RichText::new(tr!("Папки удаляются вместе со всем содержимым, мимо корзины. Отменить это нельзя."))
                                .color(pal.warn),
                        );
                    }
                    (String::new(), None, tr!("Удалить"))
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
                if dialog_button(ui, pal, tr!("Отмена"), false) {
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
                        Err(tf!("{name} уже существует", "{name} already exists"))
                    } else {
                        std::fs::write(&path, b"")
                            .map(|()| {
                                self.out.push(ProjectEvent::Open(vec![path.clone()]));
                                tf!("Создан файл {name}", "File {name} created")
                            })
                            .map_err(|e| e.to_string())
                    }
                }),
                Some(Prompt::NewFolder(dir, name)) => valid(&name).map(|name| {
                    self.invalidate(&dir);
                    self.expanded.insert(dir.clone());
                    std::fs::create_dir(dir.join(&name))
                        .map(|()| tf!("Создана папка {name}", "Folder {name} created"))
                        .map_err(|e| e.to_string())
                }),
                Some(Prompt::Rename(from, name)) => valid(&name).map(|name| {
                    let parent = from.parent().map(Path::to_path_buf).unwrap_or_default();
                    self.invalidate(&parent);
                    let to = parent.join(&name);
                    if to.exists() {
                        Err(tf!("{name} уже существует", "{name} already exists"))
                    } else {
                        std::fs::rename(&from, &to)
                            .map(|()| tf!("Переименовано в {name}", "Renamed to {name}"))
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
                        None => Ok(tf!("Удалено: {removed}", "Deleted: {removed}")),
                    })
                }
                None => None,
            };
            if let Some(result) = result {
                self.out.push(ProjectEvent::Toast(match result {
                    Ok(text) => text,
                    Err(e) => tf!("Не получилось: {e}", "Failed: {e}"),
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
