use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Align, Button, Key as KeyCode, Layout, RichText, Sense};
use regex::{Captures, Regex, RegexBuilder};
use ropey::Rope;

use crate::text::document::{Document, EditKind};
use crate::theme::Palette;
use crate::widgets::{Icon, Lit, icon_button};

pub const MAX_MATCHES: usize = 100_000;
const SYNC_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy, PartialEq)]
pub enum Pending {
    FromAnchor,
    Next,
    Prev,
    Replace,
    ReplaceAll,
    DeleteAll,
}

#[derive(Clone, PartialEq)]
struct Key {
    doc: u64,
    version: u64,
    query: String,
    case: bool,
    word: bool,
    regex: bool,
    extended: bool,
}

struct Found {
    matches: Vec<(usize, usize)>,
    truncated: bool,
}

struct Job {
    rx: Receiver<Found>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub replace_mode: bool,
    pub query: String,
    pub replacement: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    pub extended: bool,
    pub files: bool,
    pub focus_query: bool,
    pub pending: Option<Pending>,
    pub matches: Vec<(usize, usize)>,
    pub truncated: bool,
    pub error: Option<String>,
    pub last_replaced: Option<usize>,
    pub deleted: bool,
    pub visual: Option<(Option<usize>, usize)>,
    re: Option<Regex>,
    key: Option<Key>,
    job: Option<Job>,
}

fn find_all(rope: &Rope, re: &Regex, cancel: &AtomicBool) -> Option<Found> {
    let text = String::from(rope);
    let mut matches = Vec::new();
    let mut truncated = false;
    for (n, m) in re.find_iter(&text).enumerate() {
        if n % 512 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        if m.is_empty() {
            continue;
        }
        if matches.len() >= MAX_MATCHES {
            truncated = true;
            break;
        }
        matches.push((rope.byte_to_char(m.start()), rope.byte_to_char(m.end())));
    }
    Some(Found { matches, truncated })
}

pub fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some(kind @ ('x' | 'u')) => {
                let len = if kind == 'x' { 2 } else { 4 };
                let digits: String = (0..len)
                    .filter_map(|_| chars.next_if(|c| c.is_ascii_hexdigit()))
                    .collect();
                match u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(ch) if digits.len() == len => out.push(ch),
                    _ => {
                        out.push('\\');
                        out.push(kind);
                        out.push_str(&digits);
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

impl SearchState {
    pub fn open(&mut self, replace: bool, selected: Option<String>) {
        self.open = true;
        self.files = false;
        self.replace_mode = replace;
        self.focus_query = true;
        self.last_replaced = None;
        if let Some(selected) = selected {
            if !selected.contains(['\n', '\r', '\t']) {
                self.query = selected;
            } else if !self.regex {
                self.query = escape(&selected);
                self.extended = true;
            }
        }
    }

    pub fn new(query: &str, regex: bool) -> Self {
        Self {
            query: query.to_owned(),
            regex,
            ..Default::default()
        }
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn matches_on(&self, start: usize, end: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        let from = self.matches.partition_point(|m| m.1 <= start);
        self.matches[from..].iter().copied().take_while(move |m| m.0 <= end)
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn has_query(&self) -> bool {
        !self.query.is_empty()
    }

    pub fn step(&mut self, pending: Pending) -> bool {
        if self.query.is_empty() {
            return false;
        }
        self.open = true;
        self.pending = Some(pending);
        true
    }

    pub fn show_files(&mut self, files: bool) {
        self.files = files;
        if !files {
            self.focus_query = true;
        }
    }

    pub fn showing_matches(&self) -> bool {
        self.open && !self.matches.is_empty()
    }

    pub fn matches_between(&self, from: usize, to: usize) -> bool {
        let at = self.matches.partition_point(|m| m.0 < from);
        self.matches.get(at).is_some_and(|m| m.0 < to)
    }

    pub fn leave_visual(&mut self) {
        self.visual = None;
    }

    pub fn visual_failed(&mut self, error: Option<String>) {
        self.pending = None;
        self.error = error;
        self.visual = Some((None, 0));
    }

    pub fn visual_start(&mut self) -> Option<Pending> {
        self.error = None;
        self.pending.take()
    }

    pub fn visual_found(&mut self, current: Option<usize>, total: usize, replaced: Option<(usize, bool)>) {
        self.visual = Some((current, total));
        if let Some((count, deleted)) = replaced {
            self.last_replaced = Some(count);
            self.deleted = deleted;
        }
    }

    pub fn key(&self) -> (String, bool, bool, bool, bool) {
        (self.query.clone(), self.case, self.word, self.regex, self.extended)
    }

    pub fn form(&mut self, ui: &mut egui::Ui, pal: &Palette, selection: Option<(usize, usize)>) {
        use crate::widgets::{FORM_ROW, form_field, form_row, toggle};
        let s = self;
        let tools_w = 4.0 * 32.0 + 3.0 * 30.0 + 10.0;
        ui.spacing_mut().item_spacing.y = 8.0;
        form_row(ui, pal, tr!("Найти"), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = crate::widgets::field_width(ui, tools_w);
            let query =
                ui.add(form_field(&mut s.query, tr!("текст или выражение")).desired_width(width));
            if std::mem::take(&mut s.focus_query) {
                query.request_focus();
            }
            let mut changed = query.changed();
            if query.lost_focus() && ui.input(|i| i.key_pressed(KeyCode::Enter)) {
                s.pending = Some(if ui.input(|i| i.modifiers.shift) {
                    Pending::Prev
                } else {
                    Pending::Next
                });
                query.request_focus();
            }
            if icon_button(ui, pal, Icon::Close)
                .on_hover_text(tr!("Очистить поиск"))
                .clicked()
            {
                s.query.clear();
                s.replacement.clear();
                s.matches.clear();
                s.error = None;
                s.focus_query = true;
                changed = true;
            }
            ui.add_space(6.0);
            changed |= toggle(ui, pal, &mut s.case, "Aa", tr!("Учитывать регистр"));
            changed |= toggle(ui, pal, &mut s.word, "ab", tr!("Слово целиком"));
            changed |= toggle(ui, pal, &mut s.regex, ".*", tr!("Регулярное выражение"));
            changed |= toggle(
                ui,
                pal,
                &mut s.extended,
                "\\n",
                tr!("Расширенный режим: \\n, \\r, \\t, \\0, \\xFF, \\uFFFF"),
            );
            if changed {
                s.pending = Some(Pending::FromAnchor);
                s.last_replaced = None;
            }
            ui.add_space(2.0);
            if icon_button(ui, pal, Icon::Up)
                .on_hover_text(tr!("Найти ранее (Shift+F3)"))
                .clicked()
            {
                s.pending = Some(Pending::Prev);
            }
            if icon_button(ui, pal, Icon::Down)
                .on_hover_text(tr!("Найти далее (F3)"))
                .clicked()
            {
                s.pending = Some(Pending::Next);
            }
        });
        form_row(ui, pal, tr!("Заменить на"), |ui| {
            let hint = if s.regex { "$1, ${name}" } else { "" };
            let replace = ui.add(
                form_field(&mut s.replacement, hint)
                    .desired_width(crate::widgets::field_width(ui, 0.0)),
            );
            if std::mem::take(&mut s.replace_mode) {
                replace.request_focus();
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let ready = !s.query.is_empty();
            let size = egui::vec2(0.0, FORM_ROW);
            if ui
                .add_enabled(
                    ready,
                    crate::widgets::primary(tr!("Найти"), pal).min_size(size),
                )
                .lit(ui)
                .on_hover_text(tr!("Найти далее (Enter, F3)"))
                .clicked()
            {
                s.pending = Some(Pending::Next);
            }
            if ui
                .add_enabled(ready, Button::new(tr!("Заменить")).min_size(size))
                .clicked()
            {
                s.pending = Some(Pending::Replace);
            }
            if ui
                .add_enabled(ready, Button::new(tr!("Заменить все")).min_size(size))
                .clicked()
            {
                s.pending = Some(Pending::ReplaceAll);
            }
            if ui
                .add_enabled(
                    ready && (!s.matches.is_empty() || s.visual.is_some_and(|v| v.1 > 0)),
                    Button::new(tr!("Удалить повторения")).min_size(size),
                )
                .on_hover_text(tr!(
                    "Удаляет все найденные совпадения. Если совпадение занимает строку целиком, \
                     строка удаляется вместе с переводом строки."
                ))
                .clicked()
            {
                s.pending = Some(Pending::DeleteAll);
            }
            let info = if let Some(error) = &s.error {
                RichText::new(error).color(pal.warn)
            } else if let Some(n) = s.last_replaced {
                let verb = if s.deleted {
                    tr!("удалено")
                } else {
                    tr!("заменено")
                };
                RichText::new(format!("{verb}: {n}")).color(pal.subtext)
            } else if s.query.is_empty() {
                RichText::new(tr!("введите текст")).color(pal.faint)
            } else if s.searching() {
                RichText::new(tr!("поиск…")).color(pal.subtext)
            } else if let Some((current, total)) = s.visual {
                match (current, total) {
                    (_, 0) => RichText::new(tr!("нет совпадений")).color(pal.warn),
                    (Some(i), _) => RichText::new(tf!("{} из {total}", "{} of {total}", i + 1))
                        .color(pal.subtext),
                    (None, _) => RichText::new(tf!("совпадений: {total}", "matches: {total}"))
                        .color(pal.subtext),
                }
            } else if s.matches.is_empty() {
                RichText::new(tr!("нет совпадений")).color(pal.warn)
            } else {
                let total = if s.truncated {
                    format!("{}+", s.matches.len())
                } else {
                    s.matches.len().to_string()
                };
                match selection.and_then(|sel| s.current(sel)) {
                    Some(i) => RichText::new(tf!("{} из {total}", "{} of {total}", i + 1)),
                    None => RichText::new(tf!("совпадений: {total}", "matches: {total}")),
                }
                .color(pal.subtext)
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::Label::new(info.small()).truncate());
            });
        });
    }

    pub fn results(&self, ui: &mut egui::Ui, pal: &Palette, doc: &Document) -> Option<(usize, usize)> {
        let matches = &self.matches;
        if self.query.is_empty() || matches.is_empty() || self.visual.is_some() {
            return None;
        }
        ui.add_space(6.0);
        ui.separator();
        let current = matches.partition_point(|m| m.0 < doc.sel.0.min(doc.sel.1));
        let row_h = crate::widgets::RESULT_ROW;
        let mut pick = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        let tall = (matches.len() as f32 * row_h).min(crate::widgets::RESULTS);
        egui::ScrollArea::vertical()
            .id_salt("tror_find_doc_hits")
            .min_scrolled_height(tall)
            .max_height(tall)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, matches.len(), |ui, rows| {
                for index in rows {
                    let (start, end) = matches[index];
                    let line = doc.rope.char_to_line(start.min(doc.len()));
                    let text: String = doc
                        .rope
                        .line(line)
                        .chars()
                        .take(300)
                        .collect::<String>()
                        .trim()
                        .replace('\t', "  ");
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    if index == current && doc.sel == (start, end) {
                        painter.rect_filled(rect, 4.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 4.0, pal.crust);
                    }
                    let font = egui::FontId::monospace(12.0);
                    let placed = painter.text(
                        rect.left_center() + egui::vec2(6.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{}", line + 1),
                        font.clone(),
                        pal.accent,
                    );
                    painter.text(
                        egui::pos2(
                            rect.left() + 60.0_f32.max(placed.width() + 16.0),
                            rect.center().y,
                        ),
                        egui::Align2::LEFT_CENTER,
                        text,
                        font,
                        pal.text,
                    );
                    if response.clicked() {
                        pick = Some((start, end));
                    }
                }
            });
        pick
    }

    pub fn pattern(&self) -> Result<Regex, String> {
        self.build()
    }

    pub fn replace_with(&self, re: &Regex, found: &str) -> String {
        if self.regex {
            re.replace(found, self.replacement.as_str()).into_owned()
        } else {
            self.replacement_text()
        }
    }

    fn replacement_text(&self) -> String {
        if self.extended && !self.regex {
            unescape(&self.replacement)
        } else {
            self.replacement.clone()
        }
    }

    pub fn searching(&self) -> bool {
        self.job.is_some()
    }

    fn build(&self) -> Result<Regex, String> {
        if self.extended && !self.regex {
            pattern(&unescape(&self.query), false, self.case, self.word)
        } else {
            pattern(&self.query, self.regex, self.case, self.word)
        }
    }

    pub fn update(&mut self, doc: &Document, ctx: Option<&egui::Context>) {
        if let Some(job) = &self.job
            && let Ok(found) = job.rx.try_recv()
        {
            self.matches = found.matches;
            self.truncated = found.truncated;
            self.job = None;
        }
        let key = Key {
            doc: doc.id,
            version: doc.version,
            query: self.query.clone(),
            case: self.case,
            word: self.word,
            regex: self.regex,
            extended: self.extended,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.key = Some(key);
        self.matches.clear();
        self.truncated = false;
        self.error = None;
        self.re = None;
        if self.query.is_empty() {
            return;
        }
        let re = match self.build() {
            Ok(re) => re,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.re = Some(re.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        match ctx {
            Some(ctx) if doc.rope.len_bytes() > SYNC_LIMIT => {
                let (tx, rx) = channel();
                let rope = doc.rope.clone();
                let flag = cancel.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    if let Some(found) = find_all(&rope, &re, &flag) {
                        let _ = tx.send(found);
                        ctx.request_repaint();
                    }
                });
                self.job = Some(Job { rx, cancel });
            }
            _ => {
                if let Some(found) = find_all(&doc.rope, &re, &cancel) {
                    self.matches = found.matches;
                    self.truncated = found.truncated;
                }
            }
        }
    }

    pub fn current(&self, selection: (usize, usize)) -> Option<usize> {
        let i = self.matches.partition_point(|m| m.0 < selection.0);
        (self.matches.get(i)? == &selection).then_some(i)
    }

    pub fn apply_pending(&mut self, doc: &mut Document, ctx: Option<&egui::Context>) {
        let Some(pending) = self.pending else {
            return;
        };
        self.update(doc, ctx);
        match pending {
            Pending::Replace => {
                self.pending = Some(Pending::Next);
                let range = doc.selection();
                if let Some(re) = &self.re
                    && !range.is_empty()
                {
                    let selected = doc.selected_text();
                    let whole = re
                        .find(&selected)
                        .is_some_and(|m| m.start() == 0 && m.end() == selected.len());
                    if whole {
                        let replaced = if self.regex {
                            re.replace(&selected, self.replacement.as_str())
                                .into_owned()
                        } else {
                            self.replacement_text()
                        };
                        doc.replace(range, &replaced, EditKind::Other);
                    }
                }
                self.apply_pending(doc, ctx);
            }
            Pending::DeleteAll => {
                self.pending = None;
                if let Some(re) = &self.re {
                    let text = String::from(&doc.rope);
                    let mut out = String::with_capacity(text.len());
                    let mut last = 0;
                    let mut count = 0;
                    for m in re.find_iter(&text) {
                        if m.is_empty() || m.start() < last {
                            continue;
                        }
                        let mut end = m.end();
                        let line_start = m.start() == 0 || text.as_bytes()[m.start() - 1] == b'\n';
                        if line_start && text[end..].starts_with("\r\n") {
                            end += 2;
                        } else if line_start && text[end..].starts_with('\n') {
                            end += 1;
                        }
                        out.push_str(&text[last..m.start()]);
                        last = end;
                        count += 1;
                    }
                    out.push_str(&text[last..]);
                    if count > 0 {
                        doc.set_text(&out);
                    }
                    self.last_replaced = Some(count);
                    self.deleted = true;
                    self.update(doc, ctx);
                }
            }
            Pending::ReplaceAll => {
                self.pending = None;
                if let Some(re) = &self.re {
                    let text = String::from(&doc.rope);
                    let mut count = 0;
                    let replacement_text = self.replacement_text();
                    let replacement = replacement_text.as_str();
                    let regex_mode = self.regex;
                    let replaced = re.replace_all(&text, |caps: &Captures<'_>| {
                        count += 1;
                        let mut out = String::new();
                        if regex_mode {
                            caps.expand(replacement, &mut out);
                        } else {
                            out.push_str(replacement);
                        }
                        out
                    });
                    if count > 0 {
                        doc.set_text(&replaced);
                    }
                    self.last_replaced = Some(count);
                    self.deleted = false;
                    self.update(doc, ctx);
                }
            }
            Pending::FromAnchor | Pending::Next | Pending::Prev => {
                if self.searching() {
                    return;
                }
                self.pending = None;
                if self.matches.is_empty() {
                    return;
                }
                let range = doc.selection();
                let index = match pending {
                    Pending::Prev => {
                        let i = self.matches.partition_point(|m| m.1 <= range.start);
                        if i == 0 {
                            self.matches.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    _ => {
                        let anchor = if pending == Pending::Next {
                            range.end
                        } else {
                            range.start
                        };
                        let i = self.matches.partition_point(|m| m.0 < anchor);
                        if i == self.matches.len() { 0 } else { i }
                    }
                };
                let (start, end) = self.matches[index];
                doc.sel = (start, end);
                doc.goal_x = None;
                doc.reveal = true;
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn step(s: &mut SearchState, d: &mut Document, pending: Pending) {
        s.pending = Some(pending);
        s.apply_pending(d, None);
    }

    #[test]
    fn finds_in_char_indices_and_wraps() {
        let mut d = Document::from_text("тест Тест\nтест");
        let mut s = SearchState {
            query: "тест".into(),
            ..Default::default()
        };
        s.update(&d, None);
        assert_eq!(s.matches, vec![(0, 4), (5, 9), (10, 14)]);
        s.case = true;
        s.update(&d, None);
        assert_eq!(s.matches, vec![(0, 4), (10, 14)]);

        step(&mut s, &mut d, Pending::FromAnchor);
        assert_eq!(d.sel, (0, 4));
        assert_eq!(s.current(d.sel), Some(0));
        step(&mut s, &mut d, Pending::Next);
        assert_eq!(d.sel, (10, 14));
        step(&mut s, &mut d, Pending::Next);
        assert_eq!(d.sel, (0, 4));
        step(&mut s, &mut d, Pending::Prev);
        assert_eq!(d.sel, (10, 14));
    }

    #[test]
    fn extended_mode_and_delete_all() {
        assert_eq!(unescape(r"a\tb\nc\x41\u0416\\\q"), "a\tb\ncAЖ\\\\q");
        assert_eq!(escape("a\n\tb\\"), r"a\n\tb\\");
        let mut d = Document::from_text("x\r\nfoo\r\ny foo\r\nfoo");
        let mut s = SearchState {
            query: r"foo\r\n".into(),
            extended: true,
            ..Default::default()
        };
        s.update(&d, None);
        assert_eq!(s.matches.len(), 2);
        s.query = "foo".into();
        step(&mut s, &mut d, Pending::DeleteAll);
        assert_eq!(d.rope.to_string(), "x\r\ny \r\n");
        assert_eq!(s.last_replaced, Some(3));
        let mut d = Document::from_text("a,b");
        s.query = ",".into();
        s.replacement = r"\n".into();
        step(&mut s, &mut d, Pending::ReplaceAll);
        assert_eq!(d.rope.to_string(), "a\nb");
    }

    #[test]
    fn whole_word_and_invalid_regex() {
        let d = Document::from_text("cat concat cat_ cat");
        let mut s = SearchState {
            query: "cat".into(),
            word: true,
            ..Default::default()
        };
        s.update(&d, None);
        assert_eq!(s.matches, vec![(0, 3), (16, 19)]);
        s.regex = true;
        s.query = "(".into();
        s.update(&d, None);
        assert!(s.error.is_some());
        assert!(s.matches.is_empty());
    }

    #[test]
    fn replace_one_then_all_with_groups_and_undo() {
        let mut d = Document::from_text("width: 10px; height: 20px; top: 5px");
        let mut s = SearchState {
            query: r"(\d+)px".into(),
            replacement: "${1}rem".into(),
            regex: true,
            ..Default::default()
        };
        step(&mut s, &mut d, Pending::FromAnchor);
        step(&mut s, &mut d, Pending::Replace);
        assert_eq!(d.rope.to_string(), "width: 10rem; height: 20px; top: 5px");
        assert_eq!(d.sel, (22, 26));
        step(&mut s, &mut d, Pending::ReplaceAll);
        assert_eq!(d.rope.to_string(), "width: 10rem; height: 20rem; top: 5rem");
        assert_eq!(s.last_replaced, Some(2));
        assert!(d.dirty());
        d.undo();
        assert_eq!(d.rope.to_string(), "width: 10rem; height: 20px; top: 5px");
    }

    #[test]
    fn plain_replacement_is_literal() {
        let mut d = Document::from_text("a.b a.b");
        let mut s = SearchState {
            query: "a.b".into(),
            replacement: "$1".into(),
            ..Default::default()
        };
        step(&mut s, &mut d, Pending::ReplaceAll);
        assert_eq!(d.rope.to_string(), "$1 $1");
    }

    #[test]
    fn background_search_delivers_results() {
        let line = "SELECT * FROM orders WHERE status = 'new';\n";
        let d = Document::from_text(&line.repeat(SYNC_LIMIT / line.len() + 10));
        let mut s = SearchState {
            query: "orders".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        s.update(&d, Some(&ctx));
        assert!(s.searching());
        for _ in 0..500 {
            s.update(&d, Some(&ctx));
            if !s.searching() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(s.matches.len(), d.line_count() - 1);
        assert_eq!(s.matches[1], (line.len() + 14, line.len() + 20));
    }

    #[test]
    #[ignore]
    fn perf_replace_all_and_save() {
        let mut d = Document::open(1, std::path::Path::new("/tmp/demo/dump.sql")).unwrap();
        let mut s = SearchState {
            query: "VALUES".into(),
            replacement: "values".into(),
            case: true,
            ..Default::default()
        };
        let at = std::time::Instant::now();
        step(&mut s, &mut d, Pending::ReplaceAll);
        eprintln!(
            "заменить все: {:.0} мс, замен {:?}",
            at.elapsed().as_secs_f64() * 1e3,
            s.last_replaced
        );
        let at = std::time::Instant::now();
        let bytes = d.encoded();
        std::fs::write("/tmp/tror_perf_save.sql", &bytes).unwrap();
        eprintln!(
            "сохранение {} МБ: {:.0} мс",
            bytes.len() / 1024 / 1024,
            at.elapsed().as_secs_f64() * 1e3
        );
        let _ = std::fs::remove_file("/tmp/tror_perf_save.sql");
    }
}
