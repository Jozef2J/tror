use std::sync::Arc;

use eframe::egui::{
    self, Align2, CursorIcon, Event, EventFilter, FontId, Galley, Id, ImeEvent, Key, Pos2, Rect,
    Sense, Stroke, Ui,
    text::{ByteIndex, CCursor, LayoutJob, LayoutSection, TextFormat},
};

use crate::complete::Completion;
use crate::document::Document;
use crate::highlight::{FRAME_BUDGET, Syntaxes};
use crate::search::SearchState;
use crate::theme::Palette;

const LONG_LINE: usize = 5000;
const LONG_PAD: usize = 80;
const PAD_X: f32 = 8.0;
const PAD_TOP: f32 = 4.0;
const BAR: f32 = 12.0;

pub fn editor_id(doc_id: u64) -> Id {
    Id::unique(("tror_editor", doc_id))
}

pub struct View<'a> {
    pub syn: &'a Syntaxes,
    pub pal: &'a Palette,
    pub focus: bool,
    pub wrap: bool,
    pub font_size: f32,
    pub line_spacing: f32,
    pub edit: crate::prefs::Edit,
    pub macros: &'a mut Macro,
    pub complete: &'a mut Completion,
}

#[derive(Default)]
pub struct Macro {
    pub recording: bool,
    pub events: Vec<Event>,
    pub play: u32,
}

impl Macro {
    fn feed(&mut self, events: &mut Vec<Event>) {
        if self.recording {
            self.events.extend(
                events
                    .iter()
                    .filter(|event| {
                        matches!(
                            event,
                            Event::Text(_)
                                | Event::Paste(_)
                                | Event::Copy
                                | Event::Cut
                                | Event::Key { pressed: true, .. }
                        )
                    })
                    .cloned(),
            );
        } else {
            for _ in 0..std::mem::take(&mut self.play) {
                events.extend(self.events.iter().cloned());
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Pointer {
    time: f64,
    pos: usize,
    clicks: u8,
    origin: Option<(usize, usize)>,
    block: Option<(usize, f64)>,
}

static BLOCK_CLIP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn remember_block(text: Option<String>) {
    if let Ok(mut clip) = BLOCK_CLIP.lock() {
        *clip = text;
    }
}

fn is_block_clip(text: &str) -> bool {
    BLOCK_CLIP
        .lock()
        .ok()
        .and_then(|c| c.clone())
        .is_some_and(|c| c == text)
}

fn apply_block(ui: &Ui, doc: &mut Document, m: &Metrics) {
    let Some(((l0, x0), (l1, x1))) = doc.block else {
        return;
    };
    let last = doc.line_count().saturating_sub(1);
    let (l0, l1) = (l0.min(last), l1.min(last));
    let lines: Vec<usize> = if l0 <= l1 {
        (l0..=l1).collect()
    } else {
        (l1..=l0).rev().collect()
    };
    let mut cursors = Vec::with_capacity(lines.len());
    for line in &lines {
        let layout = layout_line(ui, doc, *line, m);
        let start = doc.rope.line_to_char(*line);
        let a = start + layout.col_at(x0);
        let h = start + layout.col_at(x1);
        cursors.push((a, h));
    }
    let primary = cursors.len().saturating_sub(1);
    let block = doc.block;
    let sel = cursors[primary];
    doc.set_cursors(cursors.clone(), primary);
    if doc.cursors().len() != cursors.len() {
        doc.sel = sel;
        doc.extra = cursors[..primary].to_vec();
    }
    doc.block = block;
    doc.reveal = true;
}

fn block_step(ui: &Ui, doc: &mut Document, m: &Metrics, key: Key) {
    let head = doc.sel.1;
    let (anchor, mut target) = match doc.block {
        Some(block) => block,
        None => {
            let line = doc.rope.char_to_line(head);
            let col = head - doc.rope.line_to_char(line);
            let x = layout_line(ui, doc, line, m).x_of(col, m.char_w);
            let anchor_line = doc.rope.char_to_line(doc.sel.0);
            let anchor_col = doc.sel.0 - doc.rope.line_to_char(anchor_line);
            let ax = layout_line(ui, doc, anchor_line, m).x_of(anchor_col, m.char_w);
            ((anchor_line, ax), (line, x))
        }
    };
    let cw = m.char_w as f64;
    match key {
        Key::ArrowUp => target.0 = target.0.saturating_sub(1),
        Key::ArrowDown => target.0 = (target.0 + 1).min(doc.line_count().saturating_sub(1)),
        Key::ArrowLeft => target.1 = (target.1 - cw).max(0.0),
        Key::ArrowRight => target.1 += cw,
        _ => {}
    }
    target.1 = (target.1 / cw).round() * cw;
    doc.block = Some((anchor, target));
    apply_block(ui, doc, m);
}

fn vertical_char(doc: &Document, pos: usize, down: bool) -> usize {
    let line = doc.rope.char_to_line(pos);
    let col = pos - doc.rope.line_to_char(line);
    let target = if down {
        if line + 1 >= doc.line_count() {
            return doc.len();
        }
        line + 1
    } else {
        if line == 0 {
            return 0;
        }
        line - 1
    };
    doc.rope.line_to_char(target) + col.min(doc.line_len(target))
}

struct Metrics {
    font: FontId,
    char_w: f32,
    row_h: f32,
    view_w: f32,
    rows: usize,
    wrap: bool,
}

struct LineLayout {
    galley: Arc<Galley>,
    x0: f64,
    c0: usize,
    c1: usize,
    len: usize,
    grid: usize,
    starts: Vec<usize>,
}

impl LineLayout {
    fn x_of(&self, col: usize, char_w: f32) -> f64 {
        if col >= self.c0 && col <= self.c1 {
            let local = self.galley.pos_from_cursor(CCursor::new(col - self.c0));
            self.x0 + local.min.x as f64
        } else {
            col as f64 * char_w as f64
        }
    }

    fn col_at(&self, x: f64) -> usize {
        let local = self
            .galley
            .cursor_from_pos(egui::vec2((x - self.x0) as f32, 0.0));
        (self.c0 + local.index.0).min(self.len)
    }

    fn rows(&self) -> usize {
        if self.grid > 0 {
            self.len.div_ceil(self.grid).max(1)
        } else {
            self.starts.len().max(1)
        }
    }

    fn row_range(&self, row: usize) -> (usize, usize) {
        if self.grid > 0 {
            (
                (row * self.grid).min(self.len),
                ((row + 1) * self.grid).min(self.len),
            )
        } else if self.starts.is_empty() {
            (0, self.len)
        } else {
            let start = self.starts[row.min(self.starts.len() - 1)];
            let end = self.starts.get(row + 1).copied().unwrap_or(self.len);
            (start, end)
        }
    }

    fn x_in_row(&self, row: usize, col: usize, char_w: f32) -> f64 {
        let (start, _) = self.row_range(row);
        if self.grid > 0 {
            (col - start) as f64 * char_w as f64
        } else if self.starts.is_empty() {
            self.x_of(col, char_w)
        } else {
            match self.galley.rows.get(row) {
                Some(placed) => placed.x_offset(CCursor::new(col - start).index) as f64,
                None => 0.0,
            }
        }
    }

    fn place(&self, col: usize, char_w: f32) -> (f64, usize) {
        let rows = self.rows();
        let mut row = col
            .checked_div(self.grid)
            .unwrap_or_else(|| self.starts.partition_point(|s| *s <= col).saturating_sub(1));
        if row >= rows {
            row = rows - 1;
        }
        (self.x_in_row(row, col.min(self.len), char_w), row)
    }

    fn col_at_row(&self, x: f64, row: usize, char_w: f32) -> usize {
        let row = row.min(self.rows() - 1);
        let (start, end) = self.row_range(row);
        let last = end == self.len;
        let limit = if last {
            end
        } else {
            end.saturating_sub(1).max(start)
        };
        if self.grid > 0 {
            (start + (x / char_w as f64).round().max(0.0) as usize).min(limit)
        } else if self.starts.is_empty() {
            self.col_at(x)
        } else {
            let y = self
                .galley
                .rows
                .get(row)
                .map_or(0.0, |r| r.pos.y + r.height() * 0.5);
            let index = self.galley.cursor_from_pos(egui::vec2(x as f32, y)).index.0;
            index.clamp(start, limit)
        }
    }
}

fn plain_job(text: String, m: &Metrics, color: egui::Color32, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text,
        TextFormat {
            line_height: Some(m.row_h),
            ..TextFormat::simple(m.font.clone(), color)
        },
    );
    job.wrap.max_width = wrap_width;
    job
}

fn layout_line(ui: &Ui, doc: &Document, line: usize, m: &Metrics) -> LineLayout {
    let len = doc.line_len(line);
    let start = doc.rope.line_to_char(line);
    if m.wrap && len > LONG_LINE {
        let empty = plain_job(String::new(), m, doc.hl.default_color, f32::INFINITY);
        return LineLayout {
            galley: ui.fonts_mut(|f| f.layout_job(empty)),
            x0: 0.0,
            c0: 0,
            c1: 0,
            len,
            grid: ((m.view_w / m.char_w) as usize).max(8),
            starts: Vec::new(),
        };
    }
    let wrap_width = if m.wrap { m.view_w } else { f32::INFINITY };
    let (c0, c1) = if len > LONG_LINE {
        let first = (doc.scroll_x / m.char_w as f64) as usize;
        let c0 = first.saturating_sub(LONG_PAD).min(len);
        let c1 = (first + (m.view_w / m.char_w) as usize + LONG_PAD).min(len);
        (c0, c1)
    } else {
        (0, len)
    };
    let text = String::from(doc.rope.slice(start + c0..start + c1));
    let format = |color| TextFormat {
        line_height: Some(m.row_h),
        ..TextFormat::simple(m.font.clone(), color)
    };
    let default = doc.hl.default_color;
    let mut sections = Vec::new();
    if c0 == 0 && c1 == len {
        let mut offset = 0;
        for span in doc.hl.spans(line) {
            let end = (offset + span.len as usize).min(text.len());
            if !text.is_char_boundary(end) {
                sections.clear();
                offset = 0;
                break;
            }
            if end > offset {
                sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: ByteIndex(offset)..ByteIndex(end),
                    format: format(span.color),
                });
            }
            offset = end;
        }
        if offset < text.len() {
            sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(offset)..ByteIndex(text.len()),
                format: format(default),
            });
        }
    }
    let job = if sections.is_empty() {
        plain_job(text, m, default, wrap_width)
    } else {
        let mut job = LayoutJob {
            text,
            sections,
            ..Default::default()
        };
        job.wrap.max_width = wrap_width;
        job
    };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let mut starts = Vec::new();
    if m.wrap {
        let mut at = 0;
        for row in &galley.rows {
            starts.push(at);
            at += row.char_count_including_newline().0;
        }
    }
    LineLayout {
        galley,
        x0: c0 as f64 * m.char_w as f64,
        c0,
        c1,
        len,
        grid: 0,
        starts,
    }
}

fn grid_row(
    ui: &Ui,
    doc: &Document,
    line_start: usize,
    layout: &LineLayout,
    row: usize,
    m: &Metrics,
) -> Arc<Galley> {
    let (start, end) = layout.row_range(row);
    let text = String::from(doc.rope.slice(line_start + start..line_start + end));
    let job = plain_job(text, m, doc.hl.default_color, f32::INFINITY);
    ui.fonts_mut(|f| f.layout_job(job))
}

fn visible_lines(ui: &Ui, doc: &Document, m: &Metrics, view_h: f32) -> Vec<(usize, f64)> {
    let lines = doc.line_count();
    let row = m.row_h as f64;
    let scroll = doc.scroll_y.max(0.0);
    let first = ((scroll / row) as usize).min(lines - 1);
    if !m.wrap {
        let offset = -(scroll - first as f64 * row);
        return (first..(first + m.rows + 2).min(lines))
            .enumerate()
            .map(|(i, line)| (line, offset + i as f64 * row))
            .collect();
    }
    let mut out = Vec::new();
    let mut y = 0.0;
    for line in first..lines {
        let height = layout_line(ui, doc, line, m).rows() as f64 * row;
        if line == first {
            let frac = (scroll / row - first as f64).clamp(0.0, 1.0);
            y = -(frac * height).min(height - 1.0).max(0.0);
        }
        out.push((line, y));
        y += height;
        if y > view_h as f64 {
            break;
        }
    }
    out
}

fn wrapped_step(ui: &Ui, doc: &Document, m: &Metrics, pos: usize, goal: f64, down: bool) -> usize {
    let line = doc.rope.char_to_line(pos);
    let start = doc.rope.line_to_char(line);
    let layout = layout_line(ui, doc, line, m);
    let (_, row) = layout.place(pos - start, m.char_w);
    if down {
        if row + 1 < layout.rows() {
            start + layout.col_at_row(goal, row + 1, m.char_w)
        } else if line + 1 < doc.line_count() {
            let next = layout_line(ui, doc, line + 1, m);
            doc.rope.line_to_char(line + 1) + next.col_at_row(goal, 0, m.char_w)
        } else {
            doc.len()
        }
    } else if row > 0 {
        start + layout.col_at_row(goal, row - 1, m.char_w)
    } else if line > 0 {
        let prev = layout_line(ui, doc, line - 1, m);
        doc.rope.line_to_char(line - 1) + prev.col_at_row(goal, prev.rows() - 1, m.char_w)
    } else {
        0
    }
}

fn move_vertical(ui: &Ui, doc: &mut Document, m: &Metrics, delta: isize, extend: bool) {
    let head = doc.sel.1;
    let line = doc.rope.char_to_line(head);
    let col = head - doc.rope.line_to_char(line);
    if m.wrap {
        let goal = doc
            .goal_x
            .unwrap_or_else(|| layout_line(ui, doc, line, m).place(col, m.char_w).0);
        let mut pos = head;
        for _ in 0..delta.unsigned_abs() {
            let next = wrapped_step(ui, doc, m, pos, goal, delta > 0);
            if next == pos {
                break;
            }
            pos = next;
        }
        doc.move_to(pos, extend);
        doc.goal_x = Some(goal);
        return;
    }
    let goal = doc
        .goal_x
        .unwrap_or_else(|| layout_line(ui, doc, line, m).x_of(col, m.char_w));
    let target = (line as isize + delta).clamp(0, doc.line_count() as isize - 1) as usize;
    let pos = if target == line && delta < 0 {
        0
    } else if target == line {
        doc.len()
    } else {
        doc.rope.line_to_char(target) + layout_line(ui, doc, target, m).col_at(goal)
    };
    doc.move_to(pos, extend);
    doc.goal_x = Some(goal);
}

#[derive(Clone, Copy, PartialEq)]
enum Typed {
    None,
    Text,
    Backspace,
}

fn handle_events(ui: &Ui, doc: &mut Document, m: &Metrics, macros: &mut Macro) -> Typed {
    let mut typed = Typed::None;
    let mut events = ui.input(|i| i.events.clone());
    macros.feed(&mut events);
    for event in events {
        if let Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = &event
        {
            let key = *key;
            let alt_shift = modifiers.alt && modifiers.shift && !modifiers.command;
            if alt_shift
                && matches!(
                    key,
                    Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight
                )
            {
                block_step(ui, doc, m, key);
                continue;
            }
            if modifiers.command && modifiers.alt && matches!(key, Key::ArrowUp | Key::ArrowDown) {
                doc.add_cursor_vertical(key == Key::ArrowDown);
                continue;
            }
            if modifiers.command && !modifiers.shift && !modifiers.alt && key == Key::D {
                doc.select_next_match();
                continue;
            }
            if key == Key::Escape && (doc.multi() || doc.block.is_some()) {
                let sel = doc.sel;
                doc.clear_extra();
                doc.sel = sel;
                continue;
            }
            if doc.multi() {
                let shift = modifiers.shift;
                let cmd = modifiers.command;
                let handled = match key {
                    Key::ArrowLeft => {
                        doc.move_each(shift, |d, c| {
                            if cmd {
                                d.word_left(c.1)
                            } else if !shift && c.0 != c.1 {
                                c.0.min(c.1)
                            } else {
                                c.1.saturating_sub(1)
                            }
                        });
                        true
                    }
                    Key::ArrowRight => {
                        doc.move_each(shift, |d, c| {
                            if cmd {
                                d.word_right(c.1)
                            } else if !shift && c.0 != c.1 {
                                c.0.max(c.1)
                            } else {
                                c.1 + 1
                            }
                        });
                        true
                    }
                    Key::ArrowUp | Key::ArrowDown if !cmd => {
                        let down = key == Key::ArrowDown;
                        doc.move_each(shift, |d, c| vertical_char(d, c.1, down));
                        true
                    }
                    Key::Home => {
                        doc.move_each(shift, |d, c| d.line_home(c.1));
                        true
                    }
                    Key::End => {
                        doc.move_each(shift, |d, c| d.line_end(c.1));
                        true
                    }
                    _ => false,
                };
                if handled {
                    continue;
                }
            }
        }
        match event {
            Event::Text(text) => {
                let clean: String = text.chars().filter(|c| !c.is_control()).collect();
                if !clean.is_empty() {
                    doc.insert_text(&clean);
                    typed = Typed::Text;
                }
            }
            Event::Ime(ImeEvent::Commit(text)) => {
                if !text.is_empty() {
                    doc.insert_text(&text);
                }
            }
            Event::Paste(text) => {
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                if !text.is_empty() {
                    if !doc.multi() && !doc.has_selection() && is_block_clip(&text) {
                        doc.block_paste(&text);
                    } else {
                        doc.insert_text(&text);
                    }
                }
            }
            Event::Copy | Event::Cut => {
                let cut = matches!(event, Event::Cut);
                if doc.multi() {
                    let text = doc.texts().join("\n");
                    remember_block((doc.block.is_some() || doc.multi()).then(|| text.clone()));
                    ui.ctx().copy_text(text);
                    if cut {
                        doc.insert_text("");
                    }
                } else if doc.has_selection() {
                    remember_block(None);
                    ui.ctx().copy_text(doc.selected_text());
                    if cut {
                        doc.insert_text("");
                    }
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let cmd = modifiers.command;
                let shift = modifiers.shift;
                let head = doc.sel.1;
                let range = doc.selection();
                match key {
                    Key::ArrowLeft => {
                        let pos = if cmd {
                            doc.word_left(head)
                        } else if !shift && doc.has_selection() {
                            range.start
                        } else {
                            head.saturating_sub(1)
                        };
                        doc.move_to(pos, shift);
                    }
                    Key::ArrowRight => {
                        let pos = if cmd {
                            doc.word_right(head)
                        } else if !shift && doc.has_selection() {
                            range.end
                        } else {
                            head + 1
                        };
                        doc.move_to(pos, shift);
                    }
                    Key::ArrowUp if cmd => doc.scroll_y -= m.row_h as f64,
                    Key::ArrowDown if cmd => doc.scroll_y += m.row_h as f64,
                    Key::ArrowUp => move_vertical(ui, doc, m, -1, shift),
                    Key::ArrowDown => move_vertical(ui, doc, m, 1, shift),
                    Key::PageUp => move_vertical(ui, doc, m, -(m.rows.max(2) as isize - 1), shift),
                    Key::PageDown => move_vertical(ui, doc, m, m.rows.max(2) as isize - 1, shift),
                    Key::Home => {
                        let pos = if cmd { 0 } else { doc.line_home(head) };
                        doc.move_to(pos, shift);
                    }
                    Key::End => {
                        let pos = if cmd { doc.len() } else { doc.line_end(head) };
                        doc.move_to(pos, shift);
                    }
                    Key::Enter => doc.newline(),
                    Key::Tab if shift => doc.outdent(),
                    Key::Tab => doc.indent(),
                    Key::Backspace => {
                        doc.backspace(cmd);
                        typed = Typed::Backspace;
                    }
                    Key::Delete => doc.delete(cmd),
                    Key::Slash if cmd => doc.toggle_comment(),
                    Key::A if cmd => doc.select_all(),
                    Key::Z if cmd && shift => doc.redo(),
                    Key::Z if cmd => doc.undo(),
                    Key::Y if cmd => doc.redo(),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    typed
}

fn copy_selection(ui: &Ui, doc: &mut Document, cut: bool) {
    let text = if doc.multi() {
        let text = doc.texts().join("\n");
        remember_block(Some(text.clone()));
        text
    } else {
        remember_block(None);
        doc.selected_text()
    };
    ui.ctx().copy_text(text);
    if cut {
        doc.insert_text("");
    }
}

fn sentence_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for c in text.chars() {
        if start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            start = false;
        } else {
            out.extend(c.to_lowercase());
        }
        if matches!(c, '.' | '!' | '?' | '\n') {
            start = true;
        }
    }
    out
}

fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

fn invert_case(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_uppercase() {
                c.to_lowercase().collect::<Vec<_>>()
            } else {
                c.to_uppercase().collect::<Vec<_>>()
            }
        })
        .collect()
}

fn context_items(ui: &mut Ui, doc: &mut Document, id: Id) {
    ui.set_min_width(250.0);
    let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
    let keys = |k: &str| format!("{cmd}{k}");
    let selected = doc.cursors().iter().any(|(a, h)| a != h);
    let item = |ui: &mut Ui, text: &str, shortcut: String, enabled: bool| {
        let clicked = ui
            .add_enabled(enabled, egui::Button::new(text).shortcut_text(shortcut))
            .clicked();
        if clicked {
            ui.close();
        }
        clicked
    };
    if item(ui, "Вырезать", keys("X"), selected) {
        copy_selection(ui, doc, true);
    }
    if item(ui, "Копировать", keys("C"), selected) {
        copy_selection(ui, doc, false);
    }
    if item(ui, "Вставить", keys("V"), true) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        ui.memory_mut(|mem| mem.request_focus(id));
    }
    if item(ui, "Удалить", "Delete".into(), true) {
        if selected {
            doc.insert_text("");
        } else {
            doc.delete(false);
        }
    }
    ui.separator();
    if item(ui, "Выделить всё", keys("A"), true) {
        doc.select_all();
    }
    ui.separator();
    if item(ui, "Закомментировать / раскомментировать", keys("/"), true) {
        doc.toggle_comment();
    }
    if item(ui, "Дублировать строку", String::new(), true) {
        doc.duplicate_lines();
    }
    if item(ui, "Удалить строку", String::new(), true) {
        doc.delete_lines();
    }
    if item(ui, "Увеличить отступ", "Tab".into(), true) {
        doc.indent();
    }
    if item(ui, "Уменьшить отступ", "Shift+Tab".into(), true) {
        doc.outdent();
    }
    ui.separator();
    ui.menu_button("Регистр", |ui| {
        ui.set_min_width(220.0);
        let cases: [(&str, fn(&str) -> String); 5] = [
            ("ПРОПИСНЫЕ", |t| t.to_uppercase()),
            ("строчные", |t| t.to_lowercase()),
            ("Как в предложении", sentence_case),
            ("Каждое Слово С Заглавной", title_case),
            ("иНВЕРСИЯ рЕГИСТРА", invert_case),
        ];
        for (label, f) in cases {
            if ui.button(label).clicked() {
                doc.transform(f);
                ui.close();
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn scrollbar(
    ui: &Ui,
    id: Id,
    rect: Rect,
    vertical: bool,
    offset: f64,
    max: f64,
    view: f64,
    pal: &Palette,
) -> f64 {
    if max <= 0.0 {
        return 0.0;
    }
    let along = |p: Pos2| {
        if vertical {
            p.y - rect.top()
        } else {
            p.x - rect.left()
        }
    };
    let track = if vertical {
        rect.height()
    } else {
        rect.width()
    };
    let thumb = ((view / (max + view)) as f32 * track).clamp(28.0_f32.min(track), track);
    let travel = (track - thumb).max(0.0);
    let mut start = travel * (offset / max) as f32;
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let mut offset = offset;
    if response.is_pointer_button_down_on()
        && let Some(p) = response.interact_pointer_pos()
    {
        let at = along(p);
        if ui.input(|i| i.pointer.primary_pressed()) {
            let grab = if at >= start && at <= start + thumb {
                at - start
            } else {
                thumb / 2.0
            };
            ui.data_mut(|d| d.insert_temp(id, grab));
        }
        let grab = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(thumb / 2.0);
        start = (at - grab).clamp(0.0, travel);
        offset = if travel > 0.0 {
            (start / travel) as f64 * max
        } else {
            0.0
        };
    }
    let active = response.hovered() || response.is_pointer_button_down_on();
    let color = if active { pal.subtext } else { pal.faint };
    let thumb_rect = if vertical {
        Rect::from_min_size(
            egui::pos2(rect.left() + 3.0, rect.top() + start),
            egui::vec2(rect.width() - 6.0, thumb),
        )
    } else {
        Rect::from_min_size(
            egui::pos2(rect.left() + start, rect.top() + 3.0),
            egui::vec2(thumb, rect.height() - 6.0),
        )
    };
    ui.painter()
        .rect_filled(thumb_rect, 3.0, color.gamma_multiply(0.6));
    offset
}

pub fn show(ui: &mut Ui, doc: &mut Document, search: &mut SearchState, view: View<'_>) {
    let View {
        syn,
        pal,
        focus,
        wrap,
        font_size,
        line_spacing,
        edit,
        macros,
        complete,
    } = view;
    let font = FontId::monospace(font_size);
    let (char_w, font_h) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
    let row_h = (font_size * line_spacing).round().max(font_h.ceil());
    let lines = doc.line_count();
    let digits = lines.to_string().len().max(3);
    let gutter_w = if edit.line_numbers {
        digits as f32 * char_w + 28.0
    } else {
        14.0
    };
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    ui.painter().rect_filled(full, 0.0, pal.base);
    let gutter_rect = Rect::from_min_max(full.min, egui::pos2(full.min.x + gutter_w, full.max.y));
    let text_rect = Rect::from_min_max(
        egui::pos2(gutter_rect.max.x, full.min.y),
        egui::pos2(full.max.x - BAR, full.max.y - BAR),
    );
    if text_rect.width() < 20.0 || text_rect.height() < row_h {
        return;
    }
    let m = Metrics {
        font: font.clone(),
        char_w,
        row_h,
        view_w: text_rect.width() - PAD_X,
        rows: ((text_rect.height() - PAD_TOP) / row_h) as usize,
        wrap,
    };
    if wrap {
        doc.scroll_x = 0.0;
    }
    let view_h = (text_rect.height() - PAD_TOP) as f64;
    let row = row_h as f64;

    let id = editor_id(doc.id);
    let ctx = ui.ctx().clone();
    if search.open {
        search.apply_pending(doc, Some(&ctx));
    }

    let response = ui.interact(text_rect, id, Sense::click_and_drag());
    let pressed = response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed());
    let had_focus = ui.memory(|mem| mem.has_focus(id));
    if focus || pressed {
        ui.memory_mut(|mem| mem.request_focus(id));
    }
    let focused = ui.memory(|mem| mem.has_focus(id));
    if focused {
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            )
        });
    }
    if response.hovered() {
        ctx.set_cursor_icon(CursorIcon::Text);
    }

    let before = (doc.version, doc.sel);
    if focused && !had_focus {
        complete.close();
    } else if focused {
        let mut manual = false;
        ui.input_mut(|i| {
            manual = i.consume_key(egui::Modifiers::CTRL, Key::Space);
            if complete.open {
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                    complete.step(1);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                    complete.step(-1);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    complete.close();
                }
            } else if complete.hint.is_some() && i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                complete.hint = None;
            }
        });
        if complete.open
            && ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, Key::Enter)
                    | i.consume_key(egui::Modifiers::NONE, Key::Tab)
            })
        {
            complete.accept(doc);
        }
        let stepped = complete.open;
        let typed = handle_events(ui, doc, &m, macros);
        if doc.multi() {
            complete.close();
            complete.hint = None;
        } else if manual {
            complete.refresh(doc, true);
        } else if typed == Typed::Text || (typed == Typed::Backspace && stepped) {
            if edit.suggest {
                complete.refresh(doc, false);
            } else {
                complete.close();
            }
        } else if before != (doc.version, doc.sel) {
            complete.close();
        }
        if !doc.multi() && (before != (doc.version, doc.sel) || manual) {
            complete.update_hint(doc);
        }
    } else {
        complete.close();
        complete.hint = None;
    }
    if ui.rect_contains_pointer(full) {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if wrap {
            let rows = visible_lines(ui, doc, &m, 1.0)
                .first()
                .map_or(1, |(line, _)| layout_line(ui, doc, *line, &m).rows());
            doc.scroll_y -= delta.y as f64 / rows as f64;
        } else {
            doc.scroll_x -= delta.x as f64;
            doc.scroll_y -= delta.y as f64;
        }
    }

    let char_at = |ui: &Ui, doc: &Document, p: Pos2| {
        let py = p.y - text_rect.top() - PAD_TOP;
        let x = (p.x - text_rect.left() - PAD_X) as f64 + doc.scroll_x;
        if !wrap {
            let y = py as f64 + doc.scroll_y;
            let line = ((y / row).floor().max(0.0) as usize).min(doc.line_count() - 1);
            return doc.rope.line_to_char(line) + layout_line(ui, doc, line, &m).col_at(x);
        }
        let visible = visible_lines(ui, doc, &m, view_h as f32);
        let (line, top) = visible
            .iter()
            .rev()
            .find(|(_, y)| *y <= py as f64)
            .or(visible.first())
            .copied()
            .unwrap_or((0, 0.0));
        let layout = layout_line(ui, doc, line, &m);
        let at = (((py as f64 - top) / row).floor().max(0.0) as usize).min(layout.rows() - 1);
        doc.rope.line_to_char(line) + layout.col_at_row(x, at, char_w)
    };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut state = ui.data(|d| d.get_temp::<Pointer>(id)).unwrap_or_default();
    if pressed {
        complete.close();
    }
    let modifiers = ui.input(|i| i.modifiers);
    let point_x = |doc: &Document, p: Pos2| (p.x - text_rect.left() - PAD_X) as f64 + doc.scroll_x;
    if pressed
        && modifiers.alt
        && !modifiers.command
        && let Some(p) = pointer
    {
        let pos = char_at(ui, doc, p);
        let line = doc.rope.char_to_line(pos);
        let x = (point_x(doc, p) / char_w as f64).round() * char_w as f64;
        state.block = Some((line, x.max(0.0)));
        state.origin = None;
        doc.move_to(pos, false);
        doc.block = Some(((line, x.max(0.0)), (line, x.max(0.0))));
        doc.reveal = false;
    } else if pressed
        && modifiers.command
        && !modifiers.shift
        && let Some(p) = pointer
    {
        let pos = char_at(ui, doc, p);
        doc.add_cursor(pos);
        state.origin = None;
        state.block = None;
    } else if response.is_pointer_button_down_on()
        && let (Some(p), Some(anchor)) = (pointer, state.block)
    {
        let clamped = egui::pos2(
            p.x.clamp(text_rect.left(), text_rect.right()),
            p.y.clamp(text_rect.top(), text_rect.bottom() - 1.0),
        );
        let pos = char_at(ui, doc, clamped);
        let line = doc.rope.char_to_line(pos);
        let x = (point_x(doc, clamped) / char_w as f64).round() * char_w as f64;
        doc.block = Some((anchor, (line, x.max(0.0))));
        apply_block(ui, doc, &m);
        doc.reveal = false;
    } else if pressed && let Some(p) = pointer {
        state.block = None;
        let pos = char_at(ui, doc, p);
        let now = ui.input(|i| i.time);
        let shift = modifiers.shift;
        state.clicks = if now - state.time < 0.4 && state.pos == pos && state.clicks < 3 {
            state.clicks + 1
        } else {
            1
        };
        state.time = now;
        state.pos = pos;
        match state.clicks {
            2 => doc.select_word_at(pos),
            3 => doc.select_line_at(pos),
            _ => doc.move_to(pos, shift),
        }
        doc.reveal = false;
        state.origin = Some(if state.clicks == 1 {
            (doc.sel.0, doc.sel.0)
        } else {
            (doc.sel.0, doc.sel.1)
        });
    } else if response.is_pointer_button_down_on()
        && response.dragged()
        && let (Some(p), Some(origin)) = (pointer, state.origin)
    {
        let clamped = egui::pos2(
            p.x.clamp(text_rect.left(), text_rect.right()),
            p.y.clamp(text_rect.top(), text_rect.bottom() - 1.0),
        );
        let head = char_at(ui, doc, clamped);
        doc.sel = if head < origin.0 {
            (origin.1, head)
        } else {
            (origin.0, head.max(origin.1))
        };
        doc.goal_x = None;
        let over = egui::vec2(p.x - clamped.x, p.y - clamped.y);
        if over != egui::Vec2::ZERO {
            doc.scroll_x += (over.x * 0.3) as f64;
            doc.scroll_y += (over.y * 0.3) as f64;
            ctx.request_repaint();
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        state.origin = None;
        state.block = None;
    }
    ui.data_mut(|d| d.insert_temp(id, state));
    if response.secondary_clicked()
        && let Some(p) = pointer
    {
        let pos = char_at(ui, doc, p);
        let inside = doc.cursors().iter().any(|(a, h)| {
            let (lo, hi) = (*a.min(h), *a.max(h));
            lo < hi && pos >= lo && pos <= hi
        });
        if !inside {
            doc.clear_extra();
            doc.move_to(pos, false);
        }
        complete.close();
        ui.memory_mut(|mem| mem.request_focus(id));
    }
    response.context_menu(|ui| context_items(ui, doc, id));

    let lines = doc.line_count();
    let cw = char_w as f64;
    let view_w = m.view_w as f64;
    let content_w = doc.content_w.max(doc.max_line_chars as f64 * cw);
    let max_x = if wrap {
        0.0
    } else {
        (content_w - view_w + cw * 6.0).max(0.0)
    };
    let max_y = if wrap {
        lines as f64 * row * (1.0 - 1e-12)
    } else {
        (lines as f64 * row - view_h + row * 3.0).max(0.0)
    };
    let reveal = std::mem::take(&mut doc.reveal);
    if reveal && wrap {
        let head = doc.sel.1;
        let line = doc.rope.char_to_line(head);
        let layout = layout_line(ui, doc, line, &m);
        let (_, at) = layout.place(head - doc.rope.line_to_char(line), char_w);
        let visible = visible_lines(ui, doc, &m, view_h as f32);
        let shown = visible.iter().find(|(l, _)| *l == line).map(|(_, y)| *y);
        let cursor_y = shown.map(|y| y + at as f64 * row);
        let above = match (visible.first(), cursor_y) {
            (Some((first, _)), None) => line < *first,
            (_, Some(y)) => y < 0.0,
            _ => false,
        };
        let below = cursor_y.is_none_or(|y| y + row > view_h);
        if above {
            doc.scroll_y = (line as f64 + at as f64 / layout.rows() as f64) * row;
        } else if below {
            let mut room = (view_h as f32 - row_h).max(0.0);
            let mut top_line = line;
            let mut top_row = at;
            let take = ((room / row_h) as usize).min(at);
            top_row -= take;
            room -= take as f32 * row_h;
            let mut rows = layout.rows();
            while top_row == 0 && top_line > 0 && room >= row_h {
                let prev = layout_line(ui, doc, top_line - 1, &m).rows();
                let take = ((room / row_h) as usize).min(prev);
                room -= take as f32 * row_h;
                top_line -= 1;
                top_row = prev - take;
                rows = prev;
                if take < prev {
                    break;
                }
            }
            doc.scroll_y = (top_line as f64 + top_row as f64 / rows as f64) * row + 0.01;
        }
    } else if reveal {
        let head = doc.sel.1;
        let line = doc.rope.char_to_line(head);
        let top = line as f64 * row;
        if top < doc.scroll_y - row * 3.0 || top > doc.scroll_y + view_h + row * 2.0 {
            doc.scroll_y = top - view_h / 2.0;
        } else if top < doc.scroll_y {
            doc.scroll_y = top;
        } else if top + row > doc.scroll_y + view_h {
            doc.scroll_y = top + row - view_h;
        }
        let col = head - doc.rope.line_to_char(line);
        let x = layout_line(ui, doc, line, &m).x_of(col, char_w);
        if x < doc.scroll_x + cw * 2.0 {
            doc.scroll_x = (x - cw * 8.0).max(0.0);
        } else if x > doc.scroll_x + view_w - cw * 3.0 {
            doc.scroll_x = x - view_w + cw * 8.0;
        }
        doc.scroll_x = doc.scroll_x.clamp(0.0, max_x.max(doc.scroll_x.min(x)));
    } else {
        doc.scroll_x = doc.scroll_x.clamp(0.0, max_x);
    }
    doc.scroll_y = doc.scroll_y.clamp(0.0, max_y);

    let scrolled_y = scrollbar(
        ui,
        id.with("vbar"),
        Rect::from_min_max(
            egui::pos2(text_rect.right(), full.top()),
            egui::pos2(full.right(), text_rect.bottom()),
        ),
        true,
        doc.scroll_y,
        max_y,
        if wrap {
            (max_y / 40.0).max(1.0)
        } else {
            view_h
        },
        pal,
    );
    doc.scroll_y = scrolled_y;
    if search.open && !search.matches.is_empty() {
        let top = full.top();
        let height = text_rect.bottom() - top;
        let total = doc.line_count();
        let chars = doc.rope.len_chars();
        let slots = ((height / 2.0) as usize).clamp(1, total.max(1));
        let painter = ui.painter();
        for slot in 0..slots {
            let first = slot * total / slots;
            let last = ((slot + 1) * total / slots).max(first + 1);
            let from = doc.rope.line_to_char(first.min(total));
            let to = if last >= total {
                chars + 1
            } else {
                doc.rope.line_to_char(last)
            };
            let at = search.matches.partition_point(|m| m.0 < from);
            if search.matches.get(at).is_some_and(|m| m.0 < to) {
                let y = top + height * slot as f32 / slots as f32;
                painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(full.right() - 6.0, y),
                        egui::vec2(5.0, (height / slots as f32).clamp(2.0, 3.0)),
                    ),
                    1.0,
                    pal.match_current,
                );
            }
        }
    }
    let scrolled_x = scrollbar(
        ui,
        id.with("hbar"),
        Rect::from_min_max(
            egui::pos2(text_rect.left(), text_rect.bottom()),
            egui::pos2(text_rect.right(), full.bottom()),
        ),
        false,
        doc.scroll_x,
        max_x as f64,
        m.view_w as f64,
        pal,
    );
    doc.scroll_x = scrolled_x;

    let visible = visible_lines(ui, doc, &m, view_h as f32);
    let first = visible.first().map_or(0, |v| v.0);
    let last = visible.last().map_or(1, |v| v.0 + 1);
    if doc.hl.process(
        syn,
        &doc.rope,
        &doc.syntax,
        doc.version,
        first..last,
        Some(FRAME_BUDGET),
    ) {
        ctx.request_repaint();
    }
    if search.open {
        search.update(doc, Some(&ctx));
    }

    let now = ui.input(|i| i.time);
    if before != (doc.version, doc.sel) {
        doc.activity = now;
    }
    let painter = ui.painter_at(text_rect);
    let gutter_painter = ui.painter_at(gutter_rect);
    let origin_x = text_rect.left() + PAD_X;
    let scroll_x = doc.scroll_x;
    let origin_y = text_rect.top() + PAD_TOP;
    let sel = doc.selection();
    let head = doc.sel.1;
    let head_line = doc.rope.char_to_line(head);
    let pair = if edit.brackets && sel.is_empty() {
        doc.bracket_pair(head)
    } else {
        [None, None]
    };
    let sorted = (sel.start, sel.end);
    let mut cursor_rect = None;
    let mut extra_rects: Vec<Rect> = Vec::new();
    let extras: Vec<(usize, usize)> = doc
        .extra
        .iter()
        .map(|(a, h)| (*a.min(h), *a.max(h)))
        .collect();
    let extra_heads: Vec<usize> = doc.extra.iter().map(|c| c.1).collect();
    let smart = (!doc.multi() && !sel.is_empty() && sel.len() <= 200)
        .then(|| doc.selected_text())
        .filter(|t| !t.contains('\n') && !t.trim().is_empty());

    for (line, offset) in visible {
        let top64 = origin_y as f64 + offset;
        let row_top = |r: usize| (top64 + r as f64 * row) as f32;
        let y = top64.max(-1.0e6) as f32;
        let layout = layout_line(ui, doc, line, &m);
        let line_start = doc.rope.line_to_char(line);
        let line_end = line_start + layout.len;
        let rows = layout.rows();
        let row_from =
            (((text_rect.top() as f64 - top64) / row).floor().max(0.0) as usize).min(rows - 1);
        let row_to =
            (((text_rect.bottom() as f64 - top64) / row).ceil().max(1.0) as usize).min(rows);
        let sx = |x: f64| origin_x + (x - scroll_x) as f32;
        if !wrap {
            doc.content_w = doc.content_w.max(layout.x0 + layout.galley.size().x as f64);
        }
        let fill = |a: usize, b: usize, tail: f32, radius: f32, color: egui::Color32| {
            for r in row_from..row_to {
                let (start, end) = layout.row_range(r);
                if a > end || b < start {
                    continue;
                }
                let lo = a.max(start);
                let hi = b.min(end);
                let extra = if b > end && r + 1 == rows { tail } else { 0.0 };
                if hi > lo || extra > 0.0 {
                    let top = row_top(r);
                    painter.rect_filled(
                        Rect::from_min_max(
                            egui::pos2(sx(layout.x_in_row(r, lo, char_w)), top),
                            egui::pos2(sx(layout.x_in_row(r, hi, char_w)) + extra, top + row_h),
                        ),
                        radius,
                        color,
                    );
                }
            }
        };

        if edit.current_line && line == head_line && sel.is_empty() {
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(text_rect.left(), y),
                    egui::pos2(
                        text_rect.right(),
                        row_top(rows).min(text_rect.bottom() + 4.0),
                    ),
                ),
                0.0,
                pal.current_line,
            );
        }
        if search.open {
            let from = search.matches.partition_point(|mt| mt.1 <= line_start);
            for mt in &search.matches[from..] {
                if mt.0 > line_end {
                    break;
                }
                let a = mt.0.max(line_start) - line_start;
                let b = mt.1.min(line_end) - line_start;
                if b > a {
                    let color = if *mt == sorted {
                        pal.match_current
                    } else {
                        pal.match_bg
                    };
                    fill(a, b, 0.0, 2.0, color);
                }
            }
        }
        if let Some(needle) = &smart
            && layout.len >= needle.chars().count()
            && layout.len <= 20_000
        {
            let text: String = doc.rope.slice(line_start..line_end).to_string();
            let width = needle.chars().count();
            let mut chars_before = 0;
            let mut last_byte = 0;
            for (byte, _) in text.match_indices(needle.as_str()) {
                chars_before += text[last_byte..byte].chars().count();
                last_byte = byte;
                let a = chars_before;
                if line_start + a != sel.start {
                    fill(a, a + width, 0.0, 2.0, pal.smart);
                }
            }
        }
        if !sel.is_empty() && sel.start <= line_end && sel.end > line_start {
            let a = sel.start.max(line_start) - line_start;
            let b = sel.end.min(line_end) - line_start;
            let past = if sel.end > line_end { b + 1 } else { b };
            fill(a, past, char_w * 0.6, 0.0, pal.selection);
        }
        for (lo, hi) in &extras {
            if lo < hi && *lo <= line_end && *hi > line_start {
                let a = lo.max(&line_start) - line_start;
                let b = hi.min(&line_end) - line_start;
                let past = if *hi > line_end { b + 1 } else { b };
                fill(a, past, char_w * 0.6, 0.0, pal.selection);
            }
        }
        if layout.grid > 0 {
            for r in row_from..row_to {
                painter.galley(
                    egui::pos2(origin_x, row_top(r)),
                    grid_row(ui, doc, line_start, &layout, r, &m),
                    doc.hl.default_color,
                );
            }
        } else {
            painter.galley(
                egui::pos2(sx(layout.x0), y),
                layout.galley.clone(),
                doc.hl.default_color,
            );
        }
        if edit.whitespace && layout.grid == 0 && layout.len <= LONG_LINE {
            let mark = pal.faint.gamma_multiply(0.55);
            for (col, ch) in doc.rope.line(line).chars().enumerate() {
                if ch != ' ' && ch != '\t' {
                    continue;
                }
                let (x, at) = layout.place(col, char_w);
                let mid = row_top(at) + row_h * 0.5;
                if ch == ' ' {
                    painter.circle_filled(egui::pos2(sx(x) + char_w * 0.5, mid), 1.0, mark);
                } else {
                    let (next, _) = layout.place(col + 1, char_w);
                    let (from, to) = (sx(x) + 2.0, (sx(next) - 2.0).max(sx(x) + 6.0));
                    let stroke = Stroke::new(1.0, mark);
                    painter.line_segment([egui::pos2(from, mid), egui::pos2(to, mid)], stroke);
                    painter.line_segment([egui::pos2(to - 3.0, mid - 3.0), egui::pos2(to, mid)], stroke);
                    painter.line_segment([egui::pos2(to - 3.0, mid + 3.0), egui::pos2(to, mid)], stroke);
                }
            }
        }
        for at in pair.iter().flatten() {
            if *at >= line_start && *at < line_end.max(line_start + 1) && layout.grid == 0 {
                let (x, r) = layout.place(at - line_start, char_w);
                let top = row_top(r);
                painter.rect_stroke(
                    Rect::from_min_max(
                        egui::pos2(sx(x), top + 1.0),
                        egui::pos2(sx(x) + char_w, top + row_h - 1.0),
                    ),
                    2.0,
                    Stroke::new(1.0, pal.accent),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if line == head_line {
            let (x, at) = layout.place(head - line_start, char_w);
            let top = row_top(at);
            cursor_rect = Some(Rect::from_min_max(
                egui::pos2(sx(x), top),
                egui::pos2(sx(x) + 1.0, top + row_h),
            ));
        }
        for extra in &extra_heads {
            if *extra >= line_start && *extra <= line_end {
                let (x, at) = layout.place(extra - line_start, char_w);
                let top = row_top(at);
                extra_rects.push(Rect::from_min_max(
                    egui::pos2(sx(x), top),
                    egui::pos2(sx(x) + 1.0, top + row_h),
                ));
            }
        }
        if edit.line_numbers {
            gutter_painter.text(
                egui::pos2(gutter_rect.right() - 14.0, y + row_h * 0.5),
                Align2::RIGHT_CENTER,
                line + 1,
                font.clone(),
                if line == head_line {
                    pal.subtext
                } else {
                    pal.faint
                },
            );
        }
    }

    if focused && let Some(rect) = cursor_rect {
        let phase = now - doc.activity;
        if !edit.blink || (phase * 2.0) as i64 % 2 == 0 {
            painter.line_segment(
                [rect.left_top(), rect.left_bottom()],
                Stroke::new(2.0, pal.accent),
            );
            for extra in &extra_rects {
                painter.line_segment(
                    [extra.left_top(), extra.left_bottom()],
                    Stroke::new(2.0, pal.accent.gamma_multiply(0.8)),
                );
            }
        }
        if edit.blink {
            let wait = 0.5 - phase % 0.5;
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.02)));
        }
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: Default::default(),
                rect: text_rect,
                cursor_rect: rect,
                should_interrupt_composition: false,
            })
        });
    }
    if !edit.hints {
        complete.hint = None;
    }
    if complete.open
        && let Some(rect) = cursor_rect
    {
        const ROW: f32 = 22.0;
        const VISIBLE: usize = 9;
        let count = complete.items.len().min(VISIBLE);
        let top = complete
            .selected
            .saturating_sub(VISIBLE - 1)
            .min(complete.items.len() - count);
        let longest = complete
            .items
            .iter()
            .map(|item| item.text.chars().count())
            .max()
            .unwrap_or(0)
            .min(60);
        let width = (longest as f32 * char_w + 110.0).max(220.0);
        let height = count as f32 * ROW + 8.0;
        let y = if rect.bottom() + height + 4.0 > full.bottom() {
            rect.top() - height - 2.0
        } else {
            rect.bottom() + 2.0
        };
        let x = rect.left().min(full.right() - width - 4.0).max(full.left());
        let mut clicked = None;
        egui::Area::new(id.with("complete"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(x, y))
            .show(&ctx, |ui| {
                let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
                let painter = ui.painter();
                painter.rect(
                    area,
                    8.0,
                    pal.mantle,
                    Stroke::new(1.0, pal.border),
                    egui::StrokeKind::Outside,
                );
                for row in 0..count {
                    let index = top + row;
                    let item = &complete.items[index];
                    let item_rect = Rect::from_min_size(
                        egui::pos2(area.left() + 4.0, area.top() + 4.0 + row as f32 * ROW),
                        egui::vec2(width - 8.0, ROW),
                    );
                    let response =
                        ui.interact(item_rect, id.with(("complete", row)), Sense::click());
                    if index == complete.selected {
                        painter.rect_filled(item_rect, 5.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(item_rect, 5.0, pal.crust);
                    }
                    painter.text(
                        egui::pos2(item_rect.left() + 8.0, item_rect.center().y),
                        Align2::LEFT_CENTER,
                        item.text.chars().take(60).collect::<String>(),
                        font.clone(),
                        pal.text,
                    );
                    painter.text(
                        egui::pos2(item_rect.right() - 8.0, item_rect.center().y),
                        Align2::RIGHT_CENTER,
                        item.kind,
                        FontId::proportional(11.5),
                        pal.faint,
                    );
                    if response.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        if let Some(index) = clicked {
            complete.selected = index;
            complete.accept(doc);
            ctx.request_repaint();
        }
    }
    if focused
        && let (Some(hint), Some(rect)) = (&complete.hint, cursor_rect)
    {
        let small = FontId::proportional(12.0);
        let code = FontId::monospace((font_size - 1.0).max(10.0));
        let mut jobs = Vec::new();
        for sig in hint.sigs.iter().take(3) {
            let mut job = LayoutJob::default();
            job.wrap.max_width = (full.width() - 40.0).clamp(200.0, 720.0);
            let plain = TextFormat::simple(code.clone(), pal.subtext);
            job.append(&sig.name, 0.0, TextFormat::simple(code.clone(), pal.text));
            job.append("(", 0.0, plain.clone());
            let active = hint
                .active
                .min(sig.params.len().saturating_sub(1));
            for (i, param) in sig.params.iter().enumerate() {
                if i > 0 {
                    job.append(", ", 0.0, plain.clone());
                }
                let variadic = param.contains("...") || param.starts_with('*');
                let current = i == active || (variadic && hint.active >= i);
                let mut format = TextFormat::simple(
                    code.clone(),
                    if current { pal.accent } else { pal.subtext },
                );
                if current {
                    format.underline = Stroke::new(1.0, pal.accent);
                }
                job.append(param, 0.0, format);
            }
            job.append(")", 0.0, plain.clone());
            if !sig.ret.is_empty() {
                job.append(&format!(": {}", sig.ret), 0.0, TextFormat::simple(code.clone(), pal.faint));
            }
            if !sig.doc.is_empty() {
                job.append(&format!("\n{}", sig.doc), 0.0, TextFormat::simple(small.clone(), pal.faint));
            }
            jobs.push(ui.fonts_mut(|f| f.layout_job(job)));
        }
        if !jobs.is_empty() {
            let width = jobs.iter().map(|g| g.size().x).fold(0.0, f32::max) + 20.0;
            let height: f32 = jobs.iter().map(|g| g.size().y + 4.0).sum::<f32>() + 12.0;
            let above = rect.top() - height - 4.0;
            let y = if complete.open || above < full.top() {
                if complete.open {
                    above.max(full.top())
                } else {
                    rect.bottom() + 4.0
                }
            } else {
                above
            };
            let x = rect.left().min(full.right() - width - 4.0).max(full.left());
            egui::Area::new(id.with("signature"))
                .order(egui::Order::Foreground)
                .interactable(false)
                .fixed_pos(egui::pos2(x, y))
                .show(&ctx, |ui| {
                    let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
                    let painter = ui.painter();
                    painter.rect(
                        area,
                        7.0,
                        pal.mantle,
                        Stroke::new(1.0, pal.border),
                        egui::StrokeKind::Outside,
                    );
                    let mut top = area.top() + 6.0;
                    for galley in jobs {
                        let h = galley.size().y;
                        painter.galley(egui::pos2(area.left() + 10.0, top), galley, pal.text);
                        top += h + 4.0;
                    }
                });
        }
    }
}
