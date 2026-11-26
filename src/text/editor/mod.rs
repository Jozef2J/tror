use std::sync::Arc;

use eframe::egui::{
    self, Align2, CursorIcon, Event, EventFilter, FontId, Galley, Id, ImeEvent, Key, Pos2, Rect,
    Sense, Stroke, Ui,
    text::{ByteIndex, CCursor, LayoutJob, LayoutSection, TextFormat},
};

use crate::text::complete::Completion;
use crate::text::document::Document;
use crate::text::highlight::{FRAME_BUDGET, Syntaxes};
use crate::text::search::SearchState;
use crate::theme::Palette;

mod block;
mod input;
mod layout;

pub use input::copy_selection;
use crate::theme::code_family;
use block::*;
use input::*;
use layout::*;
use serde::{Deserialize, Serialize};

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
    pub edit: Edit,
    pub macros: &'a mut Macro,
    pub complete: &'a mut Completion,
    pub decos: &'a [Decoration],
}

#[derive(Default)]
pub struct Macro {
    pub recording: bool,
    pub events: Vec<Event>,
    pub play: u32,
    pub until_end: bool,
}

impl Macro {
    fn feed(&mut self, events: &[Event]) {
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
        }
    }

    fn replay(&mut self, ui: &Ui, doc: &mut Document, m: &Metrics) {
        if self.recording || self.play == 0 || self.events.is_empty() {
            self.play = 0;
            return;
        }
        let start = std::time::Instant::now();
        while self.play > 0 && start.elapsed() < std::time::Duration::from_millis(40) {
            let before = doc.char_to_line(doc.sel.1);
            process_events(ui, doc, m, self.events.clone());
            self.play -= 1;
            if self.until_end {
                let line = doc.char_to_line(doc.sel.1);
                if line <= before || line + 1 >= doc.line_count() {
                    self.play = 0;
                }
            }
        }
        if self.play == 0 {
            self.until_end = false;
        } else {
            ui.ctx().request_repaint();
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
    adding: Option<usize>,
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
        decos,
    } = view;
    let font = FontId::new(font_size, code_family());
    let (char_w, font_h) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
    let row_h = (font_size * line_spacing).round().max(font_h.ceil());
    let lines = doc.line_count();
    let digits = lines.to_string().len().max(3);
    let gutter_w = if edit.line_numbers {
        digits as f32 * char_w + 32.0
    } else {
        16.0
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
        doc.scroll_by(-doc.scroll_x, 0.0);
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
            doc.scroll_by(0.0, -(delta.y as f64 / rows as f64));
        } else {
            doc.scroll_by(-(delta.x as f64), -(delta.y as f64));
        }
    }

    let char_at = |ui: &Ui, doc: &Document, p: Pos2| {
        let py = p.y - text_rect.top() - PAD_TOP;
        let x = (p.x - text_rect.left() - PAD_X) as f64 + doc.scroll_x;
        if !wrap {
            let y = py as f64 + doc.scroll_y;
            let visual = (y / row).floor().max(0.0) as usize;
            if visual >= doc.shown_count() {
                return doc.len();
            }
            let line = doc.line_of_visual(visual);
            return doc.line_to_char(line) + layout_line(ui, doc, line, &m).col_at(x);
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
        let at = ((py as f64 - top) / row).floor().max(0.0) as usize;
        if at >= layout.rows() && line + 1 >= doc.line_count() {
            return doc.len();
        }
        let at = at.min(layout.rows() - 1);
        doc.line_to_char(line) + layout.col_at_row(x, at, char_w)
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
        let line = doc.char_to_line(pos);
        let x = (point_x(doc, p) / char_w as f64).round() * char_w as f64;
        state.block = Some((line, x.max(0.0)));
        state.origin = None;
        doc.move_to(pos, false);
        doc.set_block((line, x), (line, x));
        doc.stop_reveal();
    } else if pressed
        && modifiers.command
        && !modifiers.shift
        && let Some(p) = pointer
    {
        let pos = char_at(ui, doc, p);
        if let Some(url) = url_at(doc, pos) {
            if let Err(e) = crate::preview::open_url(&url) {
                crate::trace::popup(tf!("Не удалось открыть ссылку: {e}", "Could not open the link: {e}"), false);
            }
        } else {
            doc.add_cursor(pos);
            state.adding = Some(pos);
        }
        state.origin = None;
        state.block = None;
    } else if response.is_pointer_button_down_on()
        && response.dragged()
        && let (Some(p), Some(start)) = (pointer, state.adding)
    {
        let clamped = egui::pos2(
            p.x.clamp(text_rect.left(), text_rect.right()),
            p.y.clamp(text_rect.top(), text_rect.bottom() - 1.0),
        );
        let head = char_at(ui, doc, clamped);
        let mut cursors = doc.cursors();
        if let Some(last) = cursors.iter().position(|c| c.0 == start) {
            cursors[last] = (start, head);
            doc.set_cursors(cursors, last);
        }
        doc.stop_reveal();
    } else if response.is_pointer_button_down_on()
        && let (Some(p), Some(anchor)) = (pointer, state.block)
    {
        let clamped = egui::pos2(
            p.x.clamp(text_rect.left(), text_rect.right()),
            p.y.clamp(text_rect.top(), text_rect.bottom() - 1.0),
        );
        let pos = char_at(ui, doc, clamped);
        let line = doc.char_to_line(pos);
        let x = (point_x(doc, clamped) / char_w as f64).round() * char_w as f64;
        doc.set_block(anchor, (line, x));
        apply_block(ui, doc, &m);
        doc.stop_reveal();
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
        doc.stop_reveal();
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
        if head < origin.0 {
            doc.select(origin.1, head);
        } else {
            doc.select(origin.0, head.max(origin.1));
        }
        let over = egui::vec2(p.x - clamped.x, p.y - clamped.y);
        if over != egui::Vec2::ZERO {
            doc.scroll_by((over.x * 0.3) as f64, (over.y * 0.3) as f64);
            ctx.request_repaint();
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        state.origin = None;
        state.block = None;
        state.adding = None;
    }
    if response.hovered()
        && !response.dragged()
        && let Some(p) = pointer
        && url_at(doc, char_at(ui, doc, p)).is_some()
    {
        ctx.set_cursor_icon(CursorIcon::PointingHand);
        response
            .clone()
            .on_hover_text_at_pointer(tr!("Ctrl+щелчок — открыть ссылку в браузере"));
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

    doc.reveal_cursor();
    let lines = doc.shown_count();
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
    let reveal = doc.take_reveal();
    let (mut scroll_x, mut scroll_y) = (doc.scroll_x, doc.scroll_y);
    let mut limit_x = max_x;
    if let Some(head) = reveal.filter(|_| wrap) {
        let line = doc.char_to_line(head);
        let layout = layout_line(ui, doc, line, &m);
        let (_, at) = layout.place(head - doc.line_to_char(line), char_w);
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
            scroll_y = (doc.to_visual(line) as f64 + at as f64 / layout.rows() as f64) * row;
        } else if below {
            let mut room = (view_h as f32 - row_h).max(0.0);
            let mut top_line = line;
            let mut top_row = at;
            let take = ((room / row_h) as usize).min(at);
            top_row -= take;
            room -= take as f32 * row_h;
            let mut rows = layout.rows();
            while top_row == 0
                && room >= row_h
                && let Some(above) = doc.prev_shown(top_line)
            {
                let prev = layout_line(ui, doc, above, &m).rows();
                let take = ((room / row_h) as usize).min(prev);
                room -= take as f32 * row_h;
                top_line = above;
                top_row = prev - take;
                rows = prev;
                if take < prev {
                    break;
                }
            }
            scroll_y = (doc.to_visual(top_line) as f64 + top_row as f64 / rows as f64) * row + 0.01;
        }
    } else if let Some(head) = reveal {
        let line = doc.char_to_line(head);
        let top = doc.to_visual(line) as f64 * row;
        if top < scroll_y - row * 3.0 || top > scroll_y + view_h + row * 2.0 {
            scroll_y = top - view_h / 2.0;
        } else if top < scroll_y {
            scroll_y = top;
        } else if top + row > scroll_y + view_h {
            scroll_y = top + row - view_h;
        }
        let col = head - doc.line_to_char(line);
        let x = layout_line(ui, doc, line, &m).x_of(col, char_w);
        if x < scroll_x + cw * 2.0 {
            scroll_x = (x - cw * 8.0).max(0.0);
        } else if x > scroll_x + view_w - cw * 3.0 {
            scroll_x = x - view_w + cw * 8.0;
        }
        limit_x = max_x.max(scroll_x.min(x));
    }
    doc.scroll_to(scroll_x, scroll_y, limit_x, max_y);

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
    doc.scroll_to(doc.scroll_x, scrolled_y, f64::INFINITY, f64::INFINITY);
    if search.showing_matches() {
        let top = full.top();
        let height = text_rect.bottom() - top;
        let total = doc.line_count();
        let chars = doc.len();
        let slots = ((height / 2.0) as usize).clamp(1, total.max(1));
        let painter = ui.painter();
        for slot in 0..slots {
            let first = slot * total / slots;
            let last = ((slot + 1) * total / slots).max(first + 1);
            let from = doc.line_to_char(first.min(total));
            let to = if last >= total {
                chars + 1
            } else {
                doc.line_to_char(last)
            };
            if search.matches_between(from, to) {
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
    doc.scroll_to(scrolled_x, doc.scroll_y, f64::INFINITY, f64::INFINITY);

    let visible = visible_lines(ui, doc, &m, view_h as f32);
    let first = visible.first().map_or(0, |v| v.0);
    let last = visible.last().map_or(1, |v| v.0 + 1);
    if doc.highlight(syn, first..last, Some(FRAME_BUDGET)) {
        ctx.request_repaint();
    }
    if search.open {
        search.update(doc, Some(&ctx));
    }

    let now = ui.input(|i| i.time);
    doc.note_activity(now, before);
    let painter = ui.painter_at(text_rect);
    let gutter_painter = ui.painter_at(gutter_rect);
    let origin_x = text_rect.left() + PAD_X;
    let scroll_x = doc.scroll_x;
    let origin_y = text_rect.top() + PAD_TOP;
    let sel = doc.selection();
    let head = doc.sel.1;
    let head_line = doc.char_to_line(head);
    let pair = if edit.brackets && sel.is_empty() {
        doc.bracket_pair(head)
    } else {
        [None, None]
    };
    let sorted = (sel.start, sel.end);
    let mut cursor_rect = None;
    let mut extra_rects: Vec<Rect> = Vec::new();
    let cursors = doc.extras();
    let extras: Vec<(usize, usize)> = cursors.iter().map(|(a, h)| (*a.min(h), *a.max(h))).collect();
    let extra_heads: Vec<usize> = cursors.iter().map(|c| c.1).collect();
    let smart = (!doc.multi() && !sel.is_empty() && sel.len() <= 200)
        .then(|| doc.selected_text())
        .filter(|t| !t.contains('\n') && !t.trim().is_empty());

    let gutter_click = ui.interact(gutter_rect, id.with("fold_gutter"), Sense::click_and_drag());
    let mut hints: Vec<(Rect, &str)> = Vec::new();
    let mut fold_rows: Vec<(usize, f32, f32)> = Vec::new();
    let mut shown_rows: Vec<(usize, f32, f32)> = Vec::new();
    let mut fold_marks: Vec<(usize, Rect)> = Vec::new();
    for (line, offset) in visible {
        let top64 = origin_y as f64 + offset;
        let row_top = |r: usize| (top64 + r as f64 * row) as f32;
        let y = top64.max(-1.0e6) as f32;
        let layout = layout_line(ui, doc, line, &m);
        let line_start = doc.line_to_char(line);
        let line_end = line_start + layout.len;
        let rows = layout.rows();
        let row_from =
            (((text_rect.top() as f64 - top64) / row).floor().max(0.0) as usize).min(rows - 1);
        let row_to =
            (((text_rect.bottom() as f64 - top64) / row).ceil().max(1.0) as usize).min(rows);
        let sx = |x: f64| origin_x + (x - scroll_x) as f32;
        if !wrap {
            doc.widen(layout.x0 + layout.galley.size().x as f64);
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
            for mt in search.matches_on(line_start, line_end) {
                let a = mt.0.max(line_start) - line_start;
                let b = mt.1.min(line_end) - line_start;
                if b > a {
                    let color = if mt == sorted {
                        pal.match_current
                    } else {
                        pal.match_bg
                    };
                    fill(a, b, 0.0, 2.0, color);
                }
            }
        }
        for deco in decos {
            
            let color = deco
                .color
                .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
                .unwrap_or(match deco.kind {
                    DecoKind::Underline => pal.warn,
                    _ => pal.accent,
                });
            match deco.kind {
                DecoKind::Gutter => {
                    if deco.start == line {
                        let center = egui::pos2(gutter_rect.left() + 7.0, y + row_h * 0.5);
                        gutter_painter.circle_filled(center, 3.5, color);
                        if !deco.message.is_empty() {
                            hints.push((Rect::from_center_size(center, egui::vec2(12.0, row_h)), &deco.message));
                        }
                    }
                }
                _ if deco.end < line_start || deco.start > line_end || deco.end == deco.start => {}
                DecoKind::Highlight => {
                    let a = deco.start.max(line_start) - line_start;
                    let b = deco.end.min(line_end) - line_start;
                    fill(a, b, 0.0, 2.0, color.gamma_multiply(0.35));
                    if !deco.message.is_empty() {
                        let (x0, r0) = layout.place(a, char_w);
                        let (x1, _) = layout.place(b, char_w);
                        let top = row_top(r0);
                        hints.push((Rect::from_min_max(egui::pos2(sx(x0), top), egui::pos2(sx(x1).max(sx(x0) + 4.0), top + row_h)), &deco.message));
                    }
                }
                DecoKind::Underline => {
                    let a = deco.start.max(line_start) - line_start;
                    let b = deco.end.min(line_end) - line_start;
                    for r in row_from..row_to {
                        let (start, end) = layout.row_range(r);
                        if a > end || b < start {
                            continue;
                        }
                        let lo = a.max(start);
                        let hi = b.min(end).max(lo + 1);
                        let x0 = sx(layout.x_in_row(r, lo, char_w));
                        let x1 = sx(layout.x_in_row(r, hi, char_w)).max(x0 + char_w);
                        let base = row_top(r) + row_h - 2.5;
                        let mut points = Vec::new();
                        let mut x = x0;
                        let mut up = false;
                        while x < x1 {
                            points.push(egui::pos2(x, if up { base - 2.0 } else { base }));
                            up = !up;
                            x += 2.5;
                        }
                        points.push(egui::pos2(x1, if up { base - 2.0 } else { base }));
                        painter.add(egui::Shape::line(points, Stroke::new(1.0, color)));
                        if !deco.message.is_empty() {
                            hints.push((Rect::from_min_max(egui::pos2(x0, row_top(r)), egui::pos2(x1, row_top(r) + row_h)), &deco.message));
                        }
                    }
                }
            }
        }
        if let Some(needle) = &smart
            && layout.len >= needle.chars().count()
            && layout.len <= 20_000
        {
            let text: String = doc.slice(line_start..line_end).to_string();
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
        if let Some(((l0, x0), (l1, x1))) = doc.block
            && line >= l0.min(l1)
            && line <= l0.max(l1)
        {
            let (a, b) = (x0.min(x1), x0.max(x1));
            let left = sx(a);
            let right = sx(b).max(left + 2.0);
            let top = row_top(0);
            let color = if b - a < 1.0 { pal.accent } else { pal.selection };
            painter.rect_filled(
                Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, top + row_h)),
                0.0,
                color,
            );
        }
        if layout.grid > 0 {
            for r in row_from..row_to {
                painter.galley(
                    egui::pos2(origin_x, row_top(r)),
                    grid_row(ui, doc, line_start, &layout, r, &m),
                    doc.text_color(),
                );
            }
        } else {
            painter.galley(
                egui::pos2(sx(layout.x0), y),
                layout.galley.clone(),
                doc.text_color(),
            );
        }
        if layout.len <= 4000 {
            let text: String = doc.line(line).chars().collect();
            if text.contains("://") || text.contains("www.") {
                let stroke = Stroke::new(1.0, doc.text_color().gamma_multiply(0.7));
                for (a, b) in url_spans(&text) {
                    let (x0, r0) = layout.place(a, char_w);
                    let (x1, r1) = layout.place(b, char_w);
                    for r in r0..=r1.min(rows - 1) {
                        let (start, end) = layout.row_range(r);
                        let xa = if r == r0 { x0 } else { layout.x_in_row(r, start, char_w) };
                        let xb = if r == r1 { x1 } else { layout.x_in_row(r, end, char_w) };
                        let yb = row_top(r) + row_h - 3.0;
                        painter.line_segment([egui::pos2(sx(xa), yb), egui::pos2(sx(xb), yb)], stroke);
                    }
                }
            }
        }
        let folded = doc.is_folded(line);
        if folded {
            let (x, at) = layout.place(layout.len, char_w);
            let top = row_top(at);
            let mark = Rect::from_min_size(
                egui::pos2(sx(x) + char_w, top + 3.0),
                egui::vec2(char_w * 3.0, row_h - 6.0),
            );
            painter.rect_filled(mark, 4.0, pal.surface);
            for dot in [-1.0, 0.0, 1.0] {
                painter.circle_filled(mark.center() + egui::vec2(dot * 4.0, 0.0), 1.3, pal.subtext);
            }
            fold_marks.push((line, mark));
        }
        shown_rows.push((line, y, y + row_h * 0.5));
        if folded || doc.foldable(line) {
            fold_rows.push((line, y, row_top(rows)));
        }
        if edit.whitespace && layout.grid == 0 && layout.len <= LONG_LINE {
            let mark = pal.faint.gamma_multiply(0.55);
            for (col, ch) in doc.line(line).chars().enumerate() {
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
        let marked = doc.has_mark(line);
        if marked {
            if edit.line_numbers {
                gutter_painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(gutter_rect.left() + 4.0, y + 3.0),
                        egui::vec2(4.0, row_h - 6.0),
                    ),
                    2.0,
                    pal.accent,
                );
            } else {
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(text_rect.left(), y),
                        egui::pos2(text_rect.right(), row_top(rows)),
                    ),
                    0.0,
                    pal.accent.gamma_multiply(0.08),
                );
            }
        }
        if edit.line_numbers {
            gutter_painter.text(
                egui::pos2(gutter_rect.right() - 18.0, y + row_h * 0.5),
                Align2::RIGHT_CENTER,
                line + 1,
                font.clone(),
                if marked {
                    pal.accent
                } else if line == head_line {
                    pal.subtext
                } else {
                    pal.faint
                },
            );
        }
    }

    if let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && let Some((_, text)) = hints.iter().find(|(r, _)| r.contains(p))
    {
        let text = text.to_string();
        if gutter_rect.contains(p) {
            gutter_click.clone().on_hover_text_at_pointer(text);
        } else {
            response.clone().on_hover_text_at_pointer(text);
        }
    }
    let fold_x = gutter_rect.right() - 8.0;
    let current = doc
        .block_around(doc.char_to_line(doc.sel.1))
        .filter(|(head, _)| !doc.is_folded(*head));
    let hover_y = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|_| gutter_click.hovered())
        .map(|p| p.y);
    let end_y = |end: usize| -> Option<f32> {
        let last = shown_rows.last()?;
        if end > last.0 {
            return Some(gutter_rect.bottom());
        }
        shown_rows
            .iter()
            .find(|r| r.0 >= end)
            .map(|r| if r.0 == end { r.2 } else { r.1 })
    };
    let mut stems: Vec<(f32, f32, bool, bool)> = Vec::new();
    for &(line, _, _) in &fold_rows {
        if doc.is_folded(line) || current.is_some_and(|c| c.0 == line) {
            continue;
        }
        if let (Some(end), Some(head)) = (doc.block_end(line), shown_rows.iter().find(|r| r.0 == line))
            && let Some(bottom) = end_y(end)
        {
            stems.push((head.2, bottom, end <= shown_rows.last().map_or(0, |r| r.0), false));
        }
    }
    if let Some((head, end)) = current
        && let Some(bottom) = end_y(end)
    {
        let top = shown_rows
            .iter()
            .find(|r| r.0 == head)
            .map_or(gutter_rect.top(), |r| r.2);
        if top < bottom {
            stems.push((top, bottom, end <= shown_rows.last().map_or(0, |r| r.0), true));
        }
    }
    for (top, bottom, closed, hot) in stems {
        let color = if hot { pal.accent } else { pal.faint.gamma_multiply(0.45) };
        let stroke = Stroke::new(1.0, color);
        gutter_painter.line_segment([egui::pos2(fold_x, top), egui::pos2(fold_x, bottom)], stroke);
        if closed {
            gutter_painter.line_segment(
                [egui::pos2(fold_x, bottom), egui::pos2(fold_x + 5.0, bottom)],
                stroke,
            );
        }
    }
    for &(line, top, bottom) in &fold_rows {
        let folded = doc.is_folded(line);
        let center = egui::pos2(fold_x, top + row_h * 0.5);
        let hot = current.is_some_and(|c| c.0 == line)
            || hover_y.is_some_and(|y| y >= top && y < bottom);
        let color = if hot || folded { pal.accent } else { pal.faint.gamma_multiply(0.8) };
        let rect = Rect::from_center_size(center, egui::vec2(9.0, 9.0));
        gutter_painter.rect_filled(rect, 1.5, pal.base);
        gutter_painter.rect_stroke(rect, 1.5, Stroke::new(1.0, color), egui::StrokeKind::Inside);
        let stroke = Stroke::new(1.0, color);
        gutter_painter.line_segment([center - egui::vec2(2.5, 0.0), center + egui::vec2(2.5, 0.0)], stroke);
        if folded {
            gutter_painter.line_segment([center - egui::vec2(0.0, 2.5), center + egui::vec2(0.0, 2.5)], stroke);
        }
    }
    let pointer_at = ui.input(|i| i.pointer.interact_pos());
    let mark_click = gutter_click.clicked()
        && edit.line_numbers
        && pointer_at.is_some_and(|p| p.x < gutter_rect.left() + 14.0);
    if mark_click
        && let Some(p) = pointer_at
        && let Some(row) = shown_rows
            .iter()
            .rev()
            .find(|r| p.y >= r.1)
    {
        doc.toggle_mark_at(row.0);
    }
    let in_numbers = |p: Pos2| {
        p.x < fold_x - 7.0 && (!edit.line_numbers || p.x >= gutter_rect.left() + 14.0)
    };
    let line_of = |p: Pos2| shown_rows.iter().rev().find(|r| p.y >= r.1).map(|r| r.0);
    let mut lines_state = ui.data(|d| d.get_temp::<Option<usize>>(id.with("gutter_lines"))).flatten();
    if ui.input(|i| i.pointer.primary_pressed())
        && let Some(p) = pointer_at
        && gutter_rect.contains(p)
        && ui.rect_contains_pointer(gutter_rect)
        && in_numbers(p)
        && let Some(line) = line_of(p)
    {
        let modifiers = ui.input(|i| i.modifiers);
        if modifiers.command {
            let (a, b) = line_span(doc, line, line);
            let mut cursors = doc.cursors();
            cursors.push((a, b));
            let primary = cursors.len() - 1;
            doc.set_cursors(cursors, primary);
            lines_state = None;
        } else {
            let anchor = if modifiers.shift {
                doc.anchor_line()
            } else {
                line
            };
            doc.clear_extra();
            let (a, b) = line_span(doc, anchor, line);
            doc.select(a, b);
            lines_state = Some(anchor);
        }
        doc.stop_reveal();
        ui.memory_mut(|mem| mem.request_focus(id));
    } else if gutter_click.dragged()
        && let (Some(anchor), Some(p)) = (lines_state, pointer_at)
        && let Some(line) = line_of(p)
    {
        let (a, b) = line_span(doc, anchor, line);
        doc.select(a, b);
        doc.stop_reveal();
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        lines_state = None;
    }
    ui.data_mut(|d| d.insert_temp(id.with("gutter_lines"), lines_state));
    let toggled = if gutter_click.clicked() && !mark_click {
        pointer_at.filter(|p| !in_numbers(*p)).and_then(|p| {
            fold_rows
                .iter()
                .find(|(_, top, bottom)| p.y >= *top && p.y < *bottom)
                .map(|r| r.0)
        })
    } else if response.clicked() {
        pointer_at.and_then(|p| fold_marks.iter().find(|(_, r)| r.contains(p)).map(|r| r.0))
    } else {
        None
    };
    if let Some(line) = toggled {
        doc.toggle_fold(line);
        ui.memory_mut(|mem| mem.request_focus(id));
        ctx.request_repaint();
    }
    for (_, mark) in &fold_marks {
        if ui.rect_contains_pointer(*mark) {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
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

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(default)]
pub struct Edit {
    pub spaces: bool,
    pub indent: u8,
    pub line_numbers: bool,
    pub current_line: bool,
    pub blink: bool,
    pub suggest: bool,
    pub hints: bool,
    pub whitespace: bool,
    pub brackets: bool,
}

impl Default for Edit {
    fn default() -> Self {
        Self {
            spaces: false,
            indent: 4,
            line_numbers: true,
            current_line: true,
            blink: true,
            suggest: true,
            hints: true,
            whitespace: false,
            brackets: true,
        }
    }
}

impl Edit {
    pub fn unit(&self) -> String {
        if self.spaces {
            " ".repeat(self.indent.clamp(1, 8) as usize)
        } else {
            "\t".to_owned()
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Decoration {
    pub kind: DecoKind,
    pub start: usize,
    pub end: usize,
    pub color: Option<[u8; 3]>,
    pub message: String,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DecoKind {
    Highlight,
    Underline,
    Gutter,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_jump_to_stops() {
        let spans = |text: &str| {
            let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::default());
            tab_stops(&mut job, 10.0);
            job.sections
                .iter()
                .map(|s| (s.byte_range.start.0, s.byte_range.end.0, s.leading_space))
                .collect::<Vec<_>>()
        };
        assert_eq!(spans("ab\tc"), vec![(0, 3, 0.0), (3, 4, 10.0)]);
        assert_eq!(spans("abc\tc"), vec![(0, 4, 0.0), (4, 5, 0.0)]);
        assert_eq!(spans("\t\tx"), vec![(0, 1, 0.0), (1, 2, 30.0), (2, 3, 30.0)]);
        assert_eq!(spans("abcd\tx"), vec![(0, 5, 0.0), (5, 6, 30.0)]);
    }

    #[test]
    fn finds_links_and_line_spans() {
        let doc = Document::from_text("см. https://example.com/a?b=1, и www.site.ru.\nвторая");
        assert_eq!(url_at(&doc, 10).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(url_at(&doc, 36).as_deref(), Some("https://www.site.ru"));
        assert_eq!(url_at(&doc, 1), None);
        assert_eq!(url_at(&doc, 48), None);
        assert_eq!(line_span(&doc, 0, 0), (0, 45));
        assert_eq!(line_span(&doc, 1, 0), (doc.len(), 0));
    }
}
