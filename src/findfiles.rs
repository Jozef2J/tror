use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, channel},
    },
};

use crate::widgets::Lit;
use eframe::egui::{self, Button, RichText, ScrollArea, Sense, Ui};
use regex::{Regex, RegexBuilder};

use crate::{text::document::Document, theme::Palette};

const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_HITS: usize = 5000;
const SKIP: [&str; 5] = [".git", ".svn", ".hg", "node_modules", ".idea"];

pub struct Hit {
    pub path: PathBuf,
    pub doc: Option<u64>,
    pub line: usize,
    pub text: String,
    pub spans: Vec<(usize, usize)>,
    pub count: usize,
}

struct Ignore {
    base: String,
    rules: Vec<(String, bool, bool, bool)>,
}

impl Ignore {
    fn load(dir: &Path, base: &str) -> Option<Ignore> {
        let text = std::fs::read_to_string(dir.join(".gitignore")).ok()?;
        let rules = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (negate, l) = l.strip_prefix('!').map_or((false, l), |r| (true, r));
                let only_dir = l.ends_with('/');
                let l = l.trim_end_matches('/');
                let l = l.strip_prefix("**/").unwrap_or(l);
                let anchored = l.starts_with('/') || l.contains('/');
                (
                    l.trim_start_matches('/').to_owned(),
                    negate,
                    only_dir,
                    anchored,
                )
            })
            .collect();
        Some(Ignore {
            base: base.to_owned(),
            rules,
        })
    }

    fn copy(&self) -> Ignore {
        Ignore {
            base: self.base.clone(),
            rules: self.rules.clone(),
        }
    }

    fn verdict(&self, relative: &str, is_dir: bool) -> Option<bool> {
        let local = if self.base.is_empty() {
            relative
        } else {
            relative.strip_prefix(&format!("{}/", self.base))?
        };
        let name = local.rsplit('/').next().unwrap_or(local);
        let mut out = None;
        for (pattern, negate, only_dir, anchored) in &self.rules {
            if *only_dir && !is_dir {
                continue;
            }
            let hit = if *anchored {
                crate::remote::sync::glob(pattern, local)
                    || crate::remote::sync::glob(&format!("{pattern}/**"), local)
            } else {
                crate::remote::sync::glob(pattern, name)
            };
            if hit {
                out = Some(!negate);
            }
        }
        out
    }
}

fn ignored(rules: &[Ignore], relative: &str, is_dir: bool) -> bool {
    rules
        .iter()
        .filter_map(|r| r.verdict(relative, is_dir))
        .next_back()
        .unwrap_or(false)
}

enum Msg {
    Hits(Vec<Hit>),
    Done {
        files: usize,
        capped: bool,
    },
    Replaced {
        files: usize,
        count: usize,
        failed: Vec<String>,
        undo: Vec<(PathBuf, PathBuf)>,
    },
}

pub enum Out {
    Open(PathBuf, usize),
    OpenDoc(u64, usize),
    Changed,
    ReplaceTabs(Regex, String, bool),
}

#[derive(Default)]
pub struct FindFiles {
    pub focus: bool,
    pub dirty: Vec<PathBuf>,
    dir: String,
    query: String,
    replace: String,
    case: bool,
    word: bool,
    regex: bool,
    mask: String,
    hits: Vec<Hit>,
    rx: Option<Receiver<Msg>>,
    stop: Option<Arc<AtomicBool>>,
    note: String,
    confirm: bool,
    searched: String,
    pub all_files: bool,
    pub in_tabs: bool,
    undo: Vec<(PathBuf, PathBuf)>,
}

pub fn pattern(query: &str, regex: bool, case: bool, word: bool) -> Result<Regex, String> {
    let mut source = if regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    if word {
        source = format!(r"\b(?:{source})\b");
    }
    RegexBuilder::new(&source)
        .case_insensitive(!case)
        .multi_line(true)
        .build()
        .map_err(|e| tf!("Ошибка в выражении: {e}", "Error in the expression: {e}"))
}

fn wanted(name: &str, masks: &[String]) -> bool {
    masks.is_empty()
        || masks.iter().any(|mask| match mask.strip_prefix("*.") {
            Some(ext) => name
                .rsplit_once('.')
                .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext)),
            None => name.eq_ignore_ascii_case(mask),
        })
}

pub fn files(root: &Path, mask: &str, stop: &AtomicBool, gitignore: bool) -> Vec<PathBuf> {
    walk(root, mask, stop, gitignore, MAX_FILE)
}

pub fn all_files(root: &Path, stop: &AtomicBool) -> Vec<PathBuf> {
    walk(root, "", stop, true, u64::MAX)
}

fn walk(root: &Path, mask: &str, stop: &AtomicBool, gitignore: bool, max: u64) -> Vec<PathBuf> {
    let masks: Vec<String> = mask
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .collect();
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), Vec::<Ignore>::new())];
    while let Some((dir, prefix, mut rules)) = stack.pop() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(list) = std::fs::read_dir(&dir) else {
            continue;
        };
        if gitignore && let Some(rule) = Ignore::load(&dir, &prefix) {
            rules.push(rule);
        }
        for item in list.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Ok(kind) = item.file_type() else {
                continue;
            };
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if gitignore && ignored(&rules, &relative, kind.is_dir()) {
                continue;
            }
            if kind.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push((
                        item.path(),
                        relative,
                        rules.iter().map(Ignore::copy).collect(),
                    ));
                }
            } else if kind.is_file()
                && wanted(&name, &masks)
                && (max == u64::MAX || item.metadata().is_ok_and(|m| m.len() <= max))
            {
                out.push(item.path());
            }
        }
    }
    out.sort();
    out
}

fn text_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if memchr::memchr(0, &bytes[..bytes.len().min(8192)]).is_some() {
        return None;
    }
    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(_) => Document::open(0, path).ok().map(|d| d.rope.to_string()),
    }
}

pub fn search_file(path: &Path, re: &Regex, out: &mut Vec<Hit>) {
    let Some(text) = text_of(path) else {
        return;
    };
    search_text(&text, path, None, re, out);
}

pub fn search_text(text: &str, path: &Path, doc: Option<u64>, re: &Regex, out: &mut Vec<Hit>) {
    let mut last = usize::MAX;
    let mut line = 0;
    let mut scanned = 0;
    for found in re.find_iter(text) {
        if found.is_empty() {
            continue;
        }
        line += memchr::memchr_iter(b'\n', &text.as_bytes()[scanned..found.start()]).count();
        scanned = found.start();
        let from = text[..found.start()].rfind('\n').map_or(0, |i| i + 1);
        let row = &text[from..];
        let row = &row[..row.find('\n').unwrap_or(row.len())];
        let lead = row.len() - row.trim_start().len();
        let start = text[from + lead..found.start()].chars().count();
        let end = start + found.as_str().chars().count();
        if line == last
            && let Some(hit) = out.last_mut()
        {
            hit.count += 1;
            if end <= 220 {
                hit.spans.push((start, end));
            }
            continue;
        }
        last = line;
        let shown: String = row.trim().chars().take(220).collect();
        out.push(Hit {
            path: path.to_path_buf(),
            doc,
            line: line + 1,
            text: shown,
            spans: if end <= 220 {
                vec![(start, end)]
            } else {
                Vec::new()
            },
            count: 1,
        });
    }
}

pub fn replace_file(path: &Path, re: &Regex, with: &str, literal: bool) -> Result<usize, String> {
    let mut doc = Document::open(0, path)?;
    let text = doc.rope.to_string();
    let count = re.find_iter(&text).count();
    if count == 0 {
        return Ok(0);
    }
    let changed = if literal {
        re.replace_all(&text, regex::NoExpand(with))
    } else {
        re.replace_all(&text, with)
    };
    doc.set_text(&changed);
    doc.save()?;
    Ok(count)
}

impl FindFiles {
    pub fn show(&mut self) {
        self.focus = true;
    }

    fn start(&mut self, root: &Path, ctx: &egui::Context) {
        self.cancel();
        self.hits.clear();
        self.confirm = false;
        let re = match pattern(&self.query, self.regex, self.case, self.word) {
            Ok(re) => re,
            Err(e) => {
                self.note = e;
                return;
            }
        };
        self.note = tr!("Идёт поиск…").to_owned();
        self.searched = self.query.clone();
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.rx = Some(rx);
        self.stop = Some(stop.clone());
        let (root, mask, ctx) = (root.to_path_buf(), self.mask.clone(), ctx.clone());
        let gitignore = !self.all_files;
        std::thread::spawn(move || {
            let list = files(&root, &mask, &stop, gitignore);
            let mut total = 0;
            let mut capped = false;
            for chunk in list.chunks(64) {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let mut hits = Vec::new();
                for path in chunk {
                    search_file(path, &re, &mut hits);
                }
                total += hits.len();
                if !hits.is_empty() && tx.send(Msg::Hits(hits)).is_err() {
                    return;
                }
                ctx.request_repaint();
                if total >= MAX_HITS {
                    capped = true;
                    break;
                }
            }
            let _ = tx.send(Msg::Done {
                files: list.len(),
                capped,
            });
            ctx.request_repaint();
        });
    }

    fn search_tabs(&mut self, docs: &[Document]) {
        self.cancel();
        self.hits.clear();
        self.confirm = false;
        self.undo.clear();
        let re = match pattern(&self.query, self.regex, self.case, self.word) {
            Ok(re) => re,
            Err(e) => {
                self.note = e;
                return;
            }
        };
        self.searched = self.query.clone();
        for doc in docs {
            let path = doc
                .path
                .clone()
                .unwrap_or_else(|| PathBuf::from(doc.title()));
            search_text(
                &doc.rope.to_string(),
                &path,
                Some(doc.id),
                &re,
                &mut self.hits,
            );
            if self.hits.len() >= MAX_HITS {
                self.hits.truncate(MAX_HITS);
                break;
            }
        }
        let tabs: std::collections::BTreeSet<u64> =
            self.hits.iter().filter_map(|h| h.doc).collect();
        self.note = if self.hits.is_empty() {
            tf!(
                "Ничего не найдено во вкладках: {}",
                "Nothing found in tabs: {}",
                docs.len()
            )
        } else {
            tf!(
                "Строк: {}, вкладок: {} из {}",
                "Lines: {}, tabs: {} of {}",
                self.hits.len(),
                tabs.len(),
                docs.len()
            )
        };
    }

    fn restore(&mut self) {
        let (mut back, mut failed) = (0, Vec::new());
        for (path, copy) in self.undo.drain(..) {
            match std::fs::copy(&copy, &path) {
                Ok(_) => back += 1,
                Err(e) => failed.push(format!("{}: {e}", path.display())),
            }
        }
        self.note = tf!("Возвращено файлов: {back}", "Files restored: {back}");
        if !failed.is_empty() {
            self.note
                .push_str(&tf!(". Ошибки: {}", ". Errors: {}", failed.join("; ")));
        }
    }

    fn start_replace(&mut self, skip: Vec<PathBuf>, ctx: &egui::Context) {
        let Ok(re) = pattern(&self.query, self.regex, self.case, self.word) else {
            return;
        };
        let mut paths: Vec<PathBuf> = self.hits.iter().map(|h| h.path.clone()).collect();
        paths.dedup();
        let (with, literal, ctx) = (self.replace.clone(), !self.regex, ctx.clone());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.note = tr!("Идёт замена…").to_owned();
        self.hits.clear();
        self.undo.clear();
        let store = crate::remote::backups_dir().map(|d| {
            d.join("replace")
                .join(chrono::Local::now().format("%Y%m%d-%H%M%S").to_string())
        });
        std::thread::spawn(move || {
            let (mut files, mut count, mut failed, mut undo) = (0, 0, Vec::new(), Vec::new());
            for (index, path) in paths.into_iter().enumerate() {
                if skip.contains(&path) {
                    failed.push(tf!(
                        "{} — открыт с несохранёнными правками",
                        "{} — open with unsaved edits",
                        path.display()
                    ));
                    continue;
                }
                let copy = store.as_ref().and_then(|store| {
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    let copy = store.join(format!("{index}-{name}"));
                    std::fs::create_dir_all(store).ok()?;
                    std::fs::copy(&path, &copy).ok()?;
                    Some(copy)
                });
                let Some(copy) = copy else {
                    failed.push(tf!(
                        "{} — не удалось сохранить копию",
                        "{} — could not save a backup copy",
                        path.display()
                    ));
                    continue;
                };
                match replace_file(&path, &re, &with, literal) {
                    Ok(0) => {
                        let _ = std::fs::remove_file(&copy);
                    }
                    Ok(n) => {
                        files += 1;
                        count += n;
                        undo.push((path, copy));
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&copy);
                        failed.push(format!("{}: {e}", path.display()));
                    }
                }
            }
            let _ = tx.send(Msg::Replaced {
                files,
                count,
                failed,
                undo,
            });
            ctx.request_repaint();
        });
    }

    fn cancel(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.rx = None;
    }

    fn pump(&mut self) -> bool {
        let mut changed = false;
        let Some(rx) = &self.rx else {
            return false;
        };
        let mut finished = false;
        for msg in rx.try_iter() {
            match msg {
                Msg::Hits(hits) => {
                    self.hits.extend(hits);
                    self.hits.truncate(MAX_HITS);
                }
                Msg::Done { files, capped } => {
                    let found: std::collections::BTreeSet<&PathBuf> =
                        self.hits.iter().map(|h| &h.path).collect();
                    self.note = if self.hits.is_empty() {
                        tf!(
                            "Ничего не найдено, просмотрено файлов: {files}",
                            "Nothing found, files scanned: {files}"
                        )
                    } else if capped {
                        tf!(
                            "Показаны первые {} строк — уточните запрос",
                            "First {} lines shown — refine the query",
                            self.hits.len()
                        )
                    } else {
                        tf!(
                            "Строк: {}, файлов: {} из {files}",
                            "Lines: {}, files: {} of {files}",
                            self.hits.len(),
                            found.len()
                        )
                    };
                    finished = true;
                }
                Msg::Replaced {
                    files,
                    count,
                    failed,
                    undo,
                } => {
                    self.undo = undo;
                    self.note = tf!(
                        "Заменено: {count}, файлов: {files}",
                        "Replaced: {count}, files: {files}"
                    );
                    if !failed.is_empty() {
                        self.note.push_str(&tf!(
                            ". Пропущено: {}",
                            ". Skipped: {}",
                            failed.join("; ")
                        ));
                    }
                    finished = true;
                    changed = true;
                }
            }
        }
        if finished {
            self.rx = None;
            self.stop = None;
        }
        changed
    }

    pub fn poll(&mut self) -> Option<Out> {
        self.pump().then_some(Out::Changed)
    }

    pub fn body(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        project: Option<&Path>,
        docs: &[Document],
    ) -> Option<Out> {
        let mut out = None;
        let busy = self.rx.is_some();
        let chosen = PathBuf::from(self.dir.trim());
        let root: Option<PathBuf> = if self.dir.trim().is_empty() {
            project.map(Path::to_path_buf)
        } else {
            Some(chosen)
        };
        let mut go = false;
        use crate::widgets::{FORM_ROW, form_field, form_row, toggle};
        ui.spacing_mut().item_spacing.y = 8.0;
        form_row(ui, pal, tr!("Найти"), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = crate::widgets::field_width(ui, 3.0 * 32.0 + 34.0);
            let query = ui
                .add(form_field(&mut self.query, tr!("текст или выражение")).desired_width(width));
            if std::mem::take(&mut self.focus) {
                query.request_focus();
            }
            go |= query.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if crate::app::icon_button(ui, pal, crate::app::Icon::Close)
                .on_hover_text(tr!("Очистить поиск"))
                .clicked()
            {
                self.cancel();
                self.query.clear();
                self.replace.clear();
                self.hits.clear();
                self.note.clear();
                self.confirm = false;
                self.focus = true;
            }
            ui.add_space(6.0);
            toggle(ui, pal, &mut self.case, "Aa", tr!("Учитывать регистр"));
            toggle(ui, pal, &mut self.word, "ab", tr!("Слово целиком"));
            toggle(ui, pal, &mut self.regex, ".*", tr!("Регулярное выражение"));
        });
        form_row(ui, pal, tr!("Заменить на"), |ui| {
            let hint = if self.regex { "$1, ${name}" } else { "" };
            ui.add(
                form_field(&mut self.replace, hint)
                    .desired_width(crate::widgets::field_width(ui, 0.0)),
            );
        });
        let in_tabs = self.in_tabs;
        ui.add_enabled_ui(!in_tabs, |ui| {
            form_row(ui, pal, tr!("Папка"), |ui| {
                let hint = project
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| tr!("выберите папку").into());
                ui.add(
                    form_field(&mut self.dir, hint)
                        .desired_width(crate::widgets::field_width(ui, 94.0)),
                );
                if ui
                    .add(Button::new(tr!("Выбрать…")).min_size(egui::vec2(88.0, FORM_ROW)))
                    .clicked()
                {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(start) = root.as_deref().or(project) {
                        dialog = dialog.set_directory(start);
                    }
                    if let Some(dir) = dialog.pick_folder() {
                        self.dir = dir.display().to_string();
                    }
                }
            });
            form_row(ui, pal, tr!("Файлы"), |ui| {
                ui.add(
                    form_field(
                        &mut self.mask,
                        tr!("все; или маски через запятую: *.php, *.js"),
                    )
                    .desired_width(crate::widgets::field_width(ui, 0.0)),
                );
            });
        });
        ui.horizontal_wrapped(|ui| {
            let mut gitignore = !self.all_files;
            if ui
                .add_enabled(!in_tabs, egui::Checkbox::new(&mut gitignore, tr!("Пропускать файлы из .gitignore")))
                .changed()
            {
                self.all_files = !gitignore;
            }
            ui.add_space(12.0);
            if ui
                .checkbox(&mut self.in_tabs, tr!("Искать в открытых вкладках"))
                .on_hover_text(tr!("Ищет и заменяет в тексте вкладок, включая несохранённые правки; замена отменяется Ctrl+Z"))
                .changed()
            {
                self.hits.clear();
                self.note.clear();
                self.confirm = false;
            }
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let size = egui::vec2(0.0, FORM_ROW);
            let ready = !self.query.is_empty() && !busy && (root.is_some() || self.in_tabs);
            go |= ui
                .add_enabled(
                    ready,
                    crate::widgets::primary(tr!("Найти"), pal).min_size(size),
                )
                .lit(ui)
                .clicked();
            if busy
                && ui
                    .add(Button::new(tr!("Остановить")).min_size(size))
                    .clicked()
            {
                self.cancel();
                self.note = tr!("Остановлено").to_owned();
            }
            let fresh = !self.hits.is_empty() && self.searched == self.query && !busy;
            if self.confirm {
                let paths: std::collections::BTreeSet<&PathBuf> =
                    self.hits.iter().map(|h| &h.path).collect();
                let label = if self.in_tabs {
                    tf!(
                        "Да, заменить во вкладках: {}",
                        "Yes, replace in tabs: {}",
                        paths.len()
                    )
                } else {
                    tf!(
                        "Да, заменить в {} файлах",
                        "Yes, replace in {} files",
                        paths.len()
                    )
                };
                if ui
                    .add(crate::widgets::danger(label, pal).min_size(crate::widgets::ACTION))
                    .lit(ui)
                    .clicked()
                {
                    self.confirm = false;
                    if self.in_tabs {
                        if let Ok(re) = pattern(&self.query, self.regex, self.case, self.word) {
                            out = Some(Out::ReplaceTabs(re, self.replace.clone(), !self.regex));
                            self.hits.clear();
                            self.note = tr!(
                                "Заменено во вкладках — отменить можно Ctrl+Z в каждой вкладке"
                            )
                            .to_owned();
                        }
                    } else {
                        let dirty = self.dirty.clone();
                        self.start_replace(dirty, ui.ctx());
                    }
                }
                if ui.add(Button::new(tr!("Отмена"))).clicked() {
                    self.confirm = false;
                }
            } else if ui
                .add_enabled(
                    fresh,
                    Button::new(tr!("Заменить во всех найденных…")).min_size(size),
                )
                .on_disabled_hover_text(tr!("Сначала выполните поиск"))
                .clicked()
            {
                self.confirm = true;
            }
            if !self.undo.is_empty()
                && !busy
                && ui
                    .add(Button::new(tr!("Вернуть как было")).min_size(size))
                    .on_hover_text(tr!("Восстановить файлы из копий, сделанных перед заменой"))
                    .clicked()
            {
                self.restore();
                out = Some(Out::Changed);
            }
        });
        if self.confirm && !self.in_tabs {
            ui.label(
                RichText::new(tr!(
                    "Файлы будут перезаписаны на диске. Перед заменой сохраняются копии — замену можно отменить кнопкой «Вернуть как было»."
                ))
                .color(pal.warn),
            );
        }
        if self.in_tabs {
            if go && !self.query.is_empty() {
                self.search_tabs(docs);
            }
        } else if root.is_none() {
            ui.label(
                RichText::new(tr!("Откройте папку проекта или выберите папку для поиска."))
                    .color(pal.subtext),
            );
            return out;
        }
        let root = root.unwrap_or_default();
        if go && !self.query.is_empty() && !self.in_tabs {
            if root.is_dir() {
                self.start(&root, ui.ctx());
            } else {
                self.note = tr!("Папка не найдена").to_owned();
            }
        }
        if !self.note.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(&self.note).small().color(pal.subtext));
        }
        if !self.hits.is_empty() {
            ui.add_space(6.0);
            ui.separator();
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let row_h = crate::widgets::RESULT_ROW;
        let tall = (self.hits.len() as f32 * row_h).min(crate::widgets::RESULTS);
        ScrollArea::vertical()
            .id_salt("tror_find_files_hits")
            .min_scrolled_height(tall)
            .max_height(tall)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, self.hits.len(), |ui, range| {
                for hit in &self.hits[range] {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 4.0, pal.surface_hover);
                    }
                    let rel = hit.path.strip_prefix(&root).unwrap_or(&hit.path);
                    let place = if hit.count > 1 {
                        format!("{}:{} ×{}", rel.display(), hit.line, hit.count)
                    } else {
                        format!("{}:{}", rel.display(), hit.line)
                    };
                    let painter = ui.painter_at(rect);
                    let end = painter
                        .text(
                            egui::pos2(rect.left() + 6.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            place,
                            egui::FontId::proportional(12.5),
                            pal.accent,
                        )
                        .right();
                    let mut job = egui::text::LayoutJob::default();
                    let font = egui::FontId::monospace(12.5);
                    let plain = egui::TextFormat::simple(font.clone(), pal.text);
                    let marked = egui::TextFormat {
                        background: pal.match_bg,
                        ..egui::TextFormat::simple(font, pal.text)
                    };
                    let chars: Vec<char> = hit.text.chars().collect();
                    let mut at = 0;
                    for &(a, b) in &hit.spans {
                        let (a, b) = (a.min(chars.len()).max(at), b.min(chars.len()));
                        if a >= b {
                            continue;
                        }
                        job.append(&chars[at..a].iter().collect::<String>(), 0.0, plain.clone());
                        job.append(&chars[a..b].iter().collect::<String>(), 0.0, marked.clone());
                        at = b;
                    }
                    job.append(&chars[at..].iter().collect::<String>(), 0.0, plain);
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    painter.galley(
                        egui::pos2(end + 12.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        pal.text,
                    );
                    if response.clicked() {
                        out = Some(match hit.doc {
                            Some(id) => Out::OpenDoc(id, hit.line),
                            None => Out::Open(hit.path.clone(), hit.line),
                        });
                    }
                }
            });
        out
    }

    pub fn close(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lines_and_replaces_on_disk_keeping_encoding() {
        let dir = std::env::temp_dir().join(format!("tror-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join("a.php"),
            "<?php\n$price = 1;\necho $price; $price++;\n",
        )
        .unwrap();
        std::fs::write(dir.join("sub/b.js"), "let Price = 2;\r\n").unwrap();
        std::fs::write(dir.join(".git/c.php"), "$price").unwrap();
        std::fs::write(dir.join("bin.dat"), b"\0\0$price").unwrap();
        let (cp1251, _, _) = encoding_rs::WINDOWS_1251.encode("цена $price руб\n");
        std::fs::write(dir.join("ru.php"), &cp1251).unwrap();

        let stop = AtomicBool::new(false);
        let all = files(&dir, "", &stop, false);
        assert_eq!(all.len(), 4);
        assert_eq!(files(&dir, "*.js", &stop, false).len(), 1);

        let re = pattern("$price", false, true, false).unwrap();
        let mut hits = Vec::new();
        for path in &all {
            search_file(path, &re, &mut hits);
        }
        let lines: Vec<(String, usize)> = hits
            .iter()
            .map(|h| {
                (
                    h.path.file_name().unwrap().to_string_lossy().into_owned(),
                    h.line,
                )
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                ("a.php".to_owned(), 2),
                ("a.php".to_owned(), 3),
                ("ru.php".to_owned(), 1)
            ]
        );
        let loose = pattern("price", false, false, false).unwrap();
        let mut any = Vec::new();
        search_file(&dir.join("sub/b.js"), &loose, &mut any);
        assert_eq!(any.len(), 1);

        assert_eq!(
            replace_file(&dir.join("a.php"), &re, "$cost", true).unwrap(),
            3
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("a.php")).unwrap(),
            "<?php\n$cost = 1;\necho $cost; $cost++;\n"
        );
        assert_eq!(
            replace_file(&dir.join("ru.php"), &re, "$cost", true).unwrap(),
            1
        );
        let (expected, _, _) = encoding_rs::WINDOWS_1251.encode("цена $cost руб\n");
        assert_eq!(
            std::fs::read(dir.join("ru.php")).unwrap(),
            expected.as_ref()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gitignore_and_several_matches_per_line() {
        let dir = std::env::temp_dir().join(format!("tror-ignore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["vendor/lib", "src", "cache", "src/build"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(
            dir.join(".gitignore"),
            "vendor/\n*.log\n/cache\n!keep.log\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/.gitignore"), "build\n").unwrap();
        for file in [
            "a.php",
            "x.log",
            "keep.log",
            "vendor/lib/v.php",
            "cache/c.php",
            "src/b.php",
            "src/build/o.php",
            "src/cache.php",
        ] {
            std::fs::write(dir.join(file), "price price\n").unwrap();
        }
        let stop = AtomicBool::new(false);
        let names = |gitignore| -> Vec<String> {
            files(&dir, "", &stop, gitignore)
                .iter()
                .map(|p| {
                    p.strip_prefix(&dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        };
        assert_eq!(
            names(true),
            [
                ".gitignore",
                "a.php",
                "keep.log",
                "src/.gitignore",
                "src/b.php",
                "src/cache.php"
            ]
        );
        assert_eq!(names(false).len(), 10);
        let re = pattern("price", false, false, false).unwrap();
        let mut hits = Vec::new();
        search_text(
            "  one price, two price\nnone\n",
            Path::new("t"),
            None,
            &re,
            &mut hits,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].count, 2);
        assert_eq!(hits[0].text, "one price, two price");
        assert_eq!(hits[0].spans, [(4, 9), (15, 20)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
