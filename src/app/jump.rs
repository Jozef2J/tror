use super::*;
use std::collections::HashSet;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub(super) struct Jump {
    pub open: bool,
    query: String,
    picked: usize,
    focus: bool,
    root: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    rx: Option<Receiver<Vec<PathBuf>>>,
}

#[derive(Clone)]
enum Item {
    Doc(usize),
    File(PathBuf),
    Remote(usize, String),
    Command(usize),
    Line(usize),
}

struct Row {
    item: Item,
    name: String,
    hits: Vec<usize>,
    detail: String,
    key: String,
    remote: bool,
    score: i32,
}

pub fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut hits = Vec::with_capacity(query.len());
    let mut at = 0;
    let mut total = 0;
    let mut last: Option<usize> = None;
    for q in &query {
        let found = (at..lower.len()).find(|&i| lower[i] == *q)?;
        let start = found == 0 || matches!(chars[found - 1], '/' | '\\' | '_' | '-' | '.' | ' ');
        total += 1;
        if last.is_some_and(|l| l + 1 == found) {
            total += 5;
        }
        if start {
            total += 3;
        }
        total -= (found - at).min(10) as i32 / 2;
        hits.push(found);
        last = Some(found);
        at = found + 1;
    }
    Some((total * 4 - chars.len() as i32 / 8, hits))
}

impl Jump {
    pub fn show(&mut self, prefix: &str, root: Option<PathBuf>) {
        self.open = true;
        self.query = prefix.to_owned();
        self.picked = 0;
        self.focus = true;
        if root != self.root || self.files.is_empty() {
            self.root = root.clone();
            self.files.clear();
            if let Some(root) = root {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let stop = std::sync::atomic::AtomicBool::new(false);
                    let _ = tx.send(crate::findfiles::all_files(&root, &stop));
                });
                self.rx = Some(rx);
            }
        }
    }
}

impl TrorApp {
    fn jump_rows(&self) -> Vec<Row> {
        let query = self.jump.query.trim_start();
        let mut rows = Vec::new();
        if let Some(command) = query.strip_prefix('>') {
            let keys = self.settings.shortcuts();
            for (index, (_, label, _, _)) in crate::keys::COMMANDS.iter().enumerate() {
                let name = tr!(label).to_owned();
                if let Some((score, hits)) = score(command.trim(), &name) {
                    rows.push(Row {
                        item: Item::Command(index),
                        name,
                        hits,
                        detail: String::new(),
                        key: keys.label(crate::keys::COMMANDS[index].0),
                        remote: false,
                        score,
                    });
                }
            }
        } else if let Some(line) = query.strip_prefix(':') {
            if let Ok(line) = line.trim().parse::<usize>() {
                rows.push(Row {
                    item: Item::Line(line),
                    name: tf!("Перейти к строке {line}", "Go to line {line}"),
                    hits: Vec::new(),
                    detail: String::new(),
                    key: String::new(),
                    remote: false,
                    score: 0,
                });
            }
        } else {
            let mut seen: HashSet<PathBuf> = HashSet::new();
            for (index, doc) in self.docs.iter().enumerate() {
                if let Some(path) = &doc.path {
                    seen.insert(path.clone());
                }
                let name = doc.title();
                if let Some((score, hits)) = score(query, &name) {
                    rows.push(Row {
                        item: Item::Doc(index),
                        name,
                        hits,
                        detail: doc.location().unwrap_or_default(),
                        key: tr!("открыт").to_owned(),
                        remote: doc.remote.is_some(),
                        score: score + 20,
                    });
                }
            }
            let root = self.jump.root.as_deref();
            for path in &self.jump.files {
                if seen.contains(path) {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let relative = root
                    .and_then(|r| path.strip_prefix(r).ok())
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let found = score(query, &name)
                    .or_else(|| score(query, &relative).map(|(s, _)| (s - 30, Vec::new())));
                if let Some((score, hits)) = found {
                    let folder = relative.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                    rows.push(Row {
                        item: Item::File(path.clone()),
                        name,
                        hits,
                        detail: folder.to_owned(),
                        key: String::new(),
                        remote: false,
                        score,
                    });
                }
            }
            for (panel, remote) in self.remotes.iter().enumerate() {
                let Some((profile, files)) = remote.quick_files() else {
                    continue;
                };
                for path in files {
                    let name = crate::remote::file_name(&path).to_owned();
                    if let Some((score, hits)) = score(query, &name) {
                        let folder = crate::remote::parent(&path);
                        rows.push(Row {
                            item: Item::Remote(panel, path),
                            name,
                            hits,
                            detail: format!("{profile}: {folder}"),
                            key: String::new(),
                            remote: true,
                            score,
                        });
                    }
                }
            }
        }
        if query.trim_start_matches(['>', ':']).trim().is_empty() {
            rows.sort_by_key(|r| std::cmp::Reverse(r.score));
        } else {
            rows.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| a.name.len().cmp(&b.name.len()))
            });
        }
        rows.truncate(12);
        rows
    }

    pub(super) fn jump_modal(
        &mut self,
        ctx: &egui::Context,
        pal: &Palette,
        actions: &mut Vec<Action>,
    ) {
        if !self.jump.open {
            return;
        }
        if let Some(rx) = &self.jump.rx
            && let Ok(files) = rx.try_recv()
        {
            self.jump.files = files;
            self.jump.rx = None;
        }
        if self.jump.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let rows = self.jump_rows();
        let (up, down, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down && !rows.is_empty() {
            self.jump.picked = (self.jump.picked + 1) % rows.len();
        }
        if up && !rows.is_empty() {
            self.jump.picked = (self.jump.picked + rows.len() - 1) % rows.len();
        }
        self.jump.picked = self.jump.picked.min(rows.len().saturating_sub(1));
        let mut chosen = enter
            .then(|| rows.get(self.jump.picked).map(|r| r.item.clone()))
            .flatten();
        let mut close = escape;
        let width = 520.0_f32.min(ctx.content_rect().width() - 40.0);
        let left = ctx.content_rect().center().x - width / 2.0;
        let area = egui::Area::new(egui::Id::unique("tror_jump"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(left, 88.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(pal.mantle)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(10.0)
                    .inner_margin(Margin::same(10))
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 18,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(90),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        let field = ui.add(
                            TextEdit::singleline(&mut self.jump.query)
                                .desired_width(f32::INFINITY)
                                .margin(Margin::symmetric(10, 7))
                                .hint_text(tr!("Имя файла · > команда · : строка")),
                        );
                        if std::mem::take(&mut self.jump.focus) {
                            field.request_focus();
                        }
                        if field.changed() {
                            self.jump.picked = 0;
                        }
                        ui.add_space(6.0);
                        if rows.is_empty() {
                            let text = if self.jump.rx.is_some() {
                                tr!("Читаю список файлов проекта…")
                            } else {
                                tr!("Ничего не найдено")
                            };
                            ui.label(RichText::new(text).small().color(pal.subtext));
                        }
                        for (index, row) in rows.iter().enumerate() {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 32.0),
                                Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name)
                            });
                            if response.hovered()
                                && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                            {
                                self.jump.picked = index;
                            }
                            if response.clicked() {
                                chosen = Some(row.item.clone());
                            }
                            let painter = ui.painter_at(rect);
                            if index == self.jump.picked {
                                painter.rect_filled(rect, 6.0, pal.selection);
                            }
                            let mut x = rect.left() + 12.0;
                            if !matches!(row.item, Item::Command(_) | Item::Line(_)) {
                                painter.circle_filled(
                                    egui::pos2(x + 3.0, rect.center().y),
                                    3.5,
                                    if row.remote { pal.accent } else { pal.faint },
                                );
                                x += 14.0;
                            }
                            let mut job = egui::text::LayoutJob::default();
                            for (i, ch) in row.name.chars().enumerate() {
                                let hit = row.hits.contains(&i);
                                job.append(
                                    &ch.to_string(),
                                    0.0,
                                    egui::TextFormat {
                                        font_id: egui::FontId::proportional(14.0),
                                        color: if hit { pal.accent } else { pal.text },
                                        ..Default::default()
                                    },
                                );
                            }
                            let galley = ui.fonts_mut(|f| f.layout_job(job));
                            let name_w = galley.size().x;
                            painter.galley(
                                egui::pos2(x, rect.center().y - galley.size().y / 2.0),
                                galley,
                                pal.text,
                            );
                            let right = if row.key.is_empty() {
                                rect.right() - 10.0
                            } else {
                                let key = painter.text(
                                    egui::pos2(rect.right() - 12.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    &row.key,
                                    egui::FontId::monospace(12.0),
                                    pal.subtext,
                                );
                                key.left() - 12.0
                            };
                            if !row.detail.is_empty() {
                                let detail_rect = egui::Rect::from_min_max(
                                    egui::pos2(x + name_w + 12.0, rect.top()),
                                    egui::pos2(right, rect.bottom()),
                                );
                                ui.painter_at(detail_rect).text(
                                    egui::pos2(detail_rect.left(), rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &row.detail,
                                    egui::FontId::proportional(12.0),
                                    pal.faint,
                                );
                            }
                        }
                        ui.add_space(4.0);
                        let hint = if self.jump.query.trim_start().starts_with('>') {
                            tr!("↑↓ выбрать · Enter выполнить · Esc закрыть")
                        } else {
                            tr!("↑↓ выбрать · Enter открыть · Esc закрыть · ● файл с сервера")
                        };
                        ui.label(RichText::new(hint).small().color(pal.faint));
                    });
            });
        if ctx.input(|i| i.pointer.any_pressed())
            && !area
                .response
                .rect
                .contains(ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default())
        {
            close = true;
        }
        if let Some(item) = chosen {
            close = true;
            match item {
                Item::Doc(index) => self.active = index,
                Item::File(path) => self.open_path(&path, ctx.input(|i| i.time)),
                Item::Remote(panel, path) => self.remotes[panel].open_remote(path),
                Item::Line(line) => {
                    if let Some(doc) = self.docs.get_mut(self.active) {
                        doc.goto_line(line);
                    }
                }
                Item::Command(index) => {
                    let id = crate::keys::COMMANDS[index].0;
                    actions.push(self.command(id));
                    self.jump.open = false;
                    self.focus_editor = true;
                    return;
                }
            }
        }
        if close {
            self.jump.open = false;
            self.focus_editor = true;
        }
    }
}
