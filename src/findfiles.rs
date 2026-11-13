use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, channel},
    },
};

use eframe::egui::{self, Button, Color32, RichText, ScrollArea, Sense, TextEdit};
use regex::{Regex, RegexBuilder};

use crate::{document::Document, theme::Palette};

const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_HITS: usize = 5000;
const SKIP: [&str; 5] = [".git", ".svn", ".hg", "node_modules", ".idea"];

pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
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
    },
}

pub enum Out {
    Open(PathBuf, usize),
    Changed,
}

#[derive(Default)]
pub struct FindFiles {
    pub open: bool,
    pub focus: bool,
    query: String,
    replace: String,
    case: bool,
    regex: bool,
    mask: String,
    hits: Vec<Hit>,
    rx: Option<Receiver<Msg>>,
    stop: Option<Arc<AtomicBool>>,
    note: String,
    confirm: bool,
    searched: String,
}

pub fn pattern(query: &str, regex: bool, case: bool) -> Result<Regex, String> {
    let source = if regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&source)
        .case_insensitive(!case)
        .multi_line(true)
        .build()
        .map_err(|e| format!("Ошибка в выражении: {e}"))
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

pub fn files(root: &Path, mask: &str, stop: &AtomicBool) -> Vec<PathBuf> {
    let masks: Vec<String> = mask
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .collect();
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(list) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in list.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Ok(kind) = item.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push(item.path());
                }
            } else if kind.is_file()
                && wanted(&name, &masks)
                && item.metadata().is_ok_and(|m| m.len() <= MAX_FILE)
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
    let mut last = usize::MAX;
    let mut line = 0;
    let mut scanned = 0;
    for found in re.find_iter(&text) {
        line += memchr::memchr_iter(b'\n', &text.as_bytes()[scanned..found.start()]).count();
        scanned = found.start();
        if line == last {
            continue;
        }
        last = line;
        let from = text[..found.start()].rfind('\n').map_or(0, |i| i + 1);
        let to = text[found.start()..]
            .find('\n')
            .map_or(text.len(), |i| found.start() + i);
        let shown: String = text[from..to].trim().chars().take(220).collect();
        out.push(Hit {
            path: path.to_path_buf(),
            line: line + 1,
            text: shown,
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
        self.open = true;
        self.focus = true;
    }

    fn start(&mut self, root: &Path, ctx: &egui::Context) {
        self.cancel();
        self.hits.clear();
        self.confirm = false;
        let re = match pattern(&self.query, self.regex, self.case) {
            Ok(re) => re,
            Err(e) => {
                self.note = e;
                return;
            }
        };
        self.note = "Идёт поиск…".to_owned();
        self.searched = self.query.clone();
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.rx = Some(rx);
        self.stop = Some(stop.clone());
        let (root, mask, ctx) = (root.to_path_buf(), self.mask.clone(), ctx.clone());
        std::thread::spawn(move || {
            let list = files(&root, &mask, &stop);
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

    fn start_replace(&mut self, skip: Vec<PathBuf>, ctx: &egui::Context) {
        let Ok(re) = pattern(&self.query, self.regex, self.case) else {
            return;
        };
        let mut paths: Vec<PathBuf> = self.hits.iter().map(|h| h.path.clone()).collect();
        paths.dedup();
        let (with, literal, ctx) = (self.replace.clone(), !self.regex, ctx.clone());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.note = "Идёт замена…".to_owned();
        self.hits.clear();
        std::thread::spawn(move || {
            let (mut files, mut count, mut failed) = (0, 0, Vec::new());
            for path in paths {
                if skip.contains(&path) {
                    failed.push(format!(
                        "{} — открыт с несохранёнными правками",
                        path.display()
                    ));
                    continue;
                }
                match replace_file(&path, &re, &with, literal) {
                    Ok(0) => {}
                    Ok(n) => {
                        files += 1;
                        count += n;
                    }
                    Err(e) => failed.push(format!("{}: {e}", path.display())),
                }
            }
            let _ = tx.send(Msg::Replaced {
                files,
                count,
                failed,
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
                        format!("Ничего не найдено, просмотрено файлов: {files}")
                    } else if capped {
                        format!(
                            "Показаны первые {} строк — уточните запрос",
                            self.hits.len()
                        )
                    } else {
                        format!(
                            "Строк: {}, файлов: {} из {files}",
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
                } => {
                    self.note = format!("Заменено: {count}, файлов: {files}");
                    if !failed.is_empty() {
                        self.note
                            .push_str(&format!(". Пропущено: {}", failed.join("; ")));
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

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        pal: &Palette,
        root: Option<&Path>,
        dirty: &[PathBuf],
    ) -> Option<Out> {
        let mut out = self.pump().then_some(Out::Changed);
        if !self.open {
            return out;
        }
        let mut open = true;
        let busy = self.rx.is_some();
        egui::Window::new("Поиск в файлах проекта")
            .frame(crate::widgets::window_frame(pal))
            .title_bar(false)
            .default_size([720.0, 460.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .min_width(420.0)
            .min_height(240.0)
            .collapsible(false)
            .show(ctx, |ui| {
                if crate::app::window_header(ui, pal, "Поиск в файлах проекта", true) {
                    open = false;
                }
                let Some(root) = root else {
                    ui.label(
                        RichText::new("Сначала откройте папку проекта: Файл → Открыть папку.")
                            .color(pal.subtext),
                    );
                    return;
                };
                let mut go = false;
                egui::Grid::new("tror_find_files")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Найти");
                        let field = ui.add(
                            TextEdit::singleline(&mut self.query)
                                .desired_width(f32::INFINITY)
                                .hint_text("текст или выражение"),
                        );
                        if std::mem::take(&mut self.focus) {
                            field.request_focus();
                        }
                        go |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.end_row();
                        ui.label("Заменить на");
                        ui.add(
                            TextEdit::singleline(&mut self.replace).desired_width(f32::INFINITY),
                        );
                        ui.end_row();
                        ui.label("Файлы");
                        ui.add(
                            TextEdit::singleline(&mut self.mask)
                                .desired_width(f32::INFINITY)
                                .hint_text("все; или маски через запятую: *.php, *.js"),
                        );
                        ui.end_row();
                    });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.case, "Учитывать регистр");
                    ui.checkbox(&mut self.regex, "Регулярное выражение");
                });
                ui.horizontal_wrapped(|ui| {
                    let ready = !self.query.is_empty() && !busy;
                    go |= ui
                        .add_enabled(
                            ready,
                            crate::widgets::primary("Найти", pal),
                        )
                        .clicked();
                    if busy
                        && ui
                            .add(Button::new("Остановить").fill(pal.surface))
                            .clicked()
                    {
                        self.cancel();
                        self.note = "Остановлено".to_owned();
                    }
                    let fresh = !self.hits.is_empty() && self.searched == self.query && !busy;
                    if self.confirm {
                        let paths: std::collections::BTreeSet<&PathBuf> =
                            self.hits.iter().map(|h| &h.path).collect();
                        let label = format!("Да, заменить в {} файлах", paths.len());
                        if ui
                            .add(
                                Button::new(RichText::new(label).color(Color32::WHITE))
                                    .fill(Color32::from_rgb(0xc8, 0x3c, 0x3c)),
                            )
                            .clicked()
                        {
                            self.confirm = false;
                            self.start_replace(dirty.to_vec(), ui.ctx());
                        }
                        if ui.add(Button::new("Отмена").fill(pal.surface)).clicked() {
                            self.confirm = false;
                        }
                    } else if ui
                        .add_enabled(
                            fresh,
                            Button::new("Заменить во всех найденных…").fill(pal.surface),
                        )
                        .on_disabled_hover_text("Сначала выполните поиск")
                        .clicked()
                    {
                        self.confirm = true;
                    }
                });
                if self.confirm {
                    ui.label(
                        RichText::new("Файлы будут перезаписаны на диске. Отменить замену нельзя.")
                            .color(pal.warn),
                    );
                }
                if go && !self.query.is_empty() {
                    self.start(root, ui.ctx());
                }
                if !self.note.is_empty() {
                    ui.label(RichText::new(&self.note).small().color(pal.subtext));
                }
                ui.separator();
                ui.spacing_mut().item_spacing.y = 0.0;
                let row_h = 22.0;
                ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    row_h,
                    self.hits.len(),
                    |ui, range| {
                        for hit in &self.hits[range] {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), row_h),
                                Sense::click(),
                            );
                            if response.hovered() {
                                ui.painter().rect_filled(rect, 4.0, pal.surface_hover);
                            }
                            let rel = hit.path.strip_prefix(root).unwrap_or(&hit.path);
                            let place = format!("{}:{}", rel.display(), hit.line);
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
                            painter.text(
                                egui::pos2(end + 12.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &hit.text,
                                egui::FontId::monospace(12.5),
                                pal.text,
                            );
                            if response.clicked() {
                                out = Some(Out::Open(hit.path.clone(), hit.line));
                            }
                        }
                    },
                );
            });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
            self.cancel();
        }
        out
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
        let all = files(&dir, "", &stop);
        assert_eq!(all.len(), 4);
        assert_eq!(files(&dir, "*.js", &stop).len(), 1);

        let re = pattern("$price", false, true).unwrap();
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
        let loose = pattern("price", false, false).unwrap();
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
}
