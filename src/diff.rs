use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Button, ComboBox, FontId, Rect, RichText, ScrollArea, Sense, Ui};
use similar::{Algorithm, DiffOp};

use crate::document::Document;
use crate::theme::Palette;

const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Same,
    Changed,
    Removed,
    Added,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Row {
    left: Option<u32>,
    right: Option<u32>,
    kind: Kind,
}

#[derive(Default)]
pub struct DiffPanel {
    pub left: Option<u64>,
    pub right: Option<u64>,
    key: Option<[u64; 4]>,
    texts: [Vec<String>; 2],
    rows: Vec<Row>,
    hunks: Vec<usize>,
    added: usize,
    removed: usize,
    note: String,
    jump: Option<usize>,
    at: usize,
    scroll_x: f32,
    inline: HashMap<usize, [Vec<Range<usize>>; 2]>,
}

pub fn inline(old: &str, new: &str) -> [Vec<Range<usize>>; 2] {
    let deadline = Instant::now() + Duration::from_millis(20);
    let ops = similar::TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .deadline(deadline)
        .diff_chars(old, new);
    let mut sides: [Vec<Range<usize>>; 2] = [Vec::new(), Vec::new()];
    let mut push = |side: usize, range: Range<usize>| {
        if range.is_empty() {
            return;
        }
        if let Some(last) = sides[side].last_mut()
            && last.end == range.start
        {
            last.end = range.end;
            return;
        }
        sides[side].push(range);
    };
    for op in ops.ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete {
                old_index, old_len, ..
            } => push(0, old_index..old_index + old_len),
            DiffOp::Insert {
                new_index, new_len, ..
            } => push(1, new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                push(0, old_index..old_index + old_len);
                push(1, new_index..new_index + new_len);
            }
        }
    }
    let changed = |side: &[Range<usize>], len: usize| {
        side.iter().map(|r| r.len()).sum::<usize>() * 10 > len.max(1) * 7
    };
    if changed(&sides[0], old.chars().count()) && changed(&sides[1], new.chars().count()) {
        return [Vec::new(), Vec::new()];
    }
    sides
}

fn lines(doc: &Document) -> Vec<String> {
    let mut lines: Vec<String> = doc
        .rope
        .lines()
        .map(|line| {
            let mut text = String::from(line);
            if text.ends_with('\n') {
                text.pop();
            }
            text
        })
        .collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

fn build(old: &[String], new: &[String]) -> Vec<Row> {
    let deadline = Instant::now() + Duration::from_millis(1500);
    let ops = similar::capture_diff_slices_deadline(Algorithm::Myers, old, new, Some(deadline));
    let mut rows = Vec::new();
    let mut push = |left: Option<usize>, right: Option<usize>, kind| {
        rows.push(Row {
            left: left.map(|i| i as u32),
            right: right.map(|i| i as u32),
            kind,
        })
    };
    for op in ops {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..len {
                    push(Some(old_index + i), Some(new_index + i), Kind::Same);
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..old_len {
                    push(Some(old_index + i), None, Kind::Removed);
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..new_len {
                    push(None, Some(new_index + i), Kind::Added);
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for i in 0..old_len.max(new_len) {
                    let left = (i < old_len).then_some(old_index + i);
                    let right = (i < new_len).then_some(new_index + i);
                    let kind = match (left, right) {
                        (Some(_), Some(_)) => Kind::Changed,
                        (Some(_), None) => Kind::Removed,
                        _ => Kind::Added,
                    };
                    push(left, right, kind);
                }
            }
        }
    }
    rows
}

impl DiffPanel {
    fn refresh(&mut self, docs: &[Document]) {
        let find = |id: Option<u64>| id.and_then(|id| docs.iter().find(|d| d.id == id));
        let (Some(a), Some(b)) = (find(self.left), find(self.right)) else {
            self.key = None;
            self.rows.clear();
            self.hunks.clear();
            self.note = "Выберите два открытых файла".into();
            return;
        };
        let key = [a.id, a.version, b.id, b.version];
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.rows.clear();
        self.hunks.clear();
        self.note.clear();
        if a.rope.len_bytes() > MAX_BYTES || b.rope.len_bytes() > MAX_BYTES {
            self.note = "Файлы больше 32 МБ не сравниваются".into();
            return;
        }
        self.texts = [lines(a), lines(b)];
        self.rows = build(&self.texts[0], &self.texts[1]);
        self.added = 0;
        self.removed = 0;
        let mut inside = false;
        for (i, row) in self.rows.iter().enumerate() {
            if row.kind != Kind::Same && !inside {
                self.hunks.push(i);
            }
            inside = row.kind != Kind::Same;
            match row.kind {
                Kind::Same => {}
                Kind::Changed => {
                    self.added += 1;
                    self.removed += 1;
                }
                Kind::Removed => self.removed += 1,
                Kind::Added => self.added += 1,
            }
        }
        if self.hunks.is_empty() {
            self.note = "Различий нет".into();
        }
        self.at = 0;
        self.jump = self.hunks.first().copied();
        self.inline.clear();
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        docs: &[Document],
        dark: bool,
        font_size: f32,
    ) -> Option<(u64, usize)> {
        self.refresh(docs);
        let mut open = None;
        ui.add_space(8.0);
        crate::widgets::wrapped_row(ui, 8.0, |ui| {
            let name = |id: Option<u64>| {
                id.and_then(|id| docs.iter().find(|d| d.id == id))
                    .map(|d| d.title())
                    .unwrap_or_else(|| "— выбрать —".into())
            };
            for (salt, side) in [("tror_diff_left", 0), ("tror_diff_right", 1)] {
                let slot = if side == 0 {
                    &mut self.left
                } else {
                    &mut self.right
                };
                ComboBox::from_id_salt(salt)
                    .width(200.0)
                    .selected_text(name(*slot))
                    .show_ui(ui, |ui| {
                        for doc in docs {
                            let label = doc.location().unwrap_or_else(|| doc.title());
                            ui.selectable_value(slot, Some(doc.id), label);
                        }
                    });
                if side == 0
                    && ui
                        .add(Button::new("← →").fill(pal.surface))
                        .on_hover_text("Поменять местами")
                        .clicked()
                {
                    std::mem::swap(&mut self.left, &mut self.right);
                }
            }
            if !self.hunks.is_empty() {
                let count = self.hunks.len();
                if ui.add(Button::new("↑").fill(pal.surface)).clicked() {
                    self.at = (self.at + count - 1) % count;
                    self.jump = Some(self.hunks[self.at]);
                }
                if ui.add(Button::new("↓").fill(pal.surface)).clicked() {
                    self.at = (self.at + 1) % count;
                    self.jump = Some(self.hunks[self.at]);
                }
                ui.label(
                    RichText::new(format!(
                        "изменение {} из {count}   +{}  −{}",
                        self.at + 1,
                        self.added,
                        self.removed
                    ))
                    .small()
                    .color(pal.subtext),
                );
            }
            ui.label(RichText::new(&self.note).small().color(pal.subtext));
        });
        ui.add_space(6.0);
        if self.rows.is_empty() {
            return None;
        }
        let font = FontId::monospace(font_size - 1.0);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let row_h = (font_size * 1.5).round();
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, pal.base);
        if ui.rect_contains_pointer(area) {
            let dx = ui.input(|i| i.smooth_scroll_delta.x);
            self.scroll_x = (self.scroll_x - dx).max(0.0);
        }
        let (red, green, red_strong, green_strong) = if dark {
            (
                egui::Color32::from_rgb(0x3f, 0x20, 0x28),
                egui::Color32::from_rgb(0x1b, 0x38, 0x28),
                egui::Color32::from_rgb(0x7a, 0x2c, 0x3a),
                egui::Color32::from_rgb(0x23, 0x6b, 0x41),
            )
        } else {
            (
                egui::Color32::from_rgb(0xff, 0xdc, 0xdc),
                egui::Color32::from_rgb(0xd8, 0xf5, 0xdc),
                egui::Color32::from_rgb(0xff, 0xa8, 0xa8),
                egui::Color32::from_rgb(0x9b, 0xe3, 0xa8),
            )
        };
        let digits = self.texts[0]
            .len()
            .max(self.texts[1].len())
            .to_string()
            .len() as f32;
        let gutter = (digits + 2.0) * char_w;
        let skip = (self.scroll_x / char_w) as usize;
        let mut scroll = ScrollArea::vertical()
            .id_salt("tror_diff_rows")
            .auto_shrink(false);
        if let Some(row) = self.jump.take() {
            scroll = scroll.vertical_scroll_offset((row as f32 - 3.0).max(0.0) * row_h);
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        scroll.show_rows(ui, row_h, self.rows.len(), |ui, range| {
            for index in range {
                let row = self.rows[index];
                if row.kind == Kind::Changed
                    && !self.inline.contains_key(&index)
                    && let (Some(l), Some(r)) = (row.left, row.right)
                {
                    let ranges = inline(
                        &self.texts[0][l as usize],
                        &self.texts[1][r as usize],
                    );
                    self.inline.insert(index, ranges);
                }
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::click());
                let (left, right) = rect.split_left_right_at_fraction(0.5);
                let painter = ui.painter();
                for (side, half, line) in [(0, left, row.left), (1, right, row.right)] {
                    let fill = match (row.kind, side, line) {
                        (Kind::Same, _, _) => None,
                        (_, _, None) => Some(pal.mantle),
                        (_, 0, _) => Some(red),
                        _ => Some(green),
                    };
                    if let Some(fill) = fill {
                        painter.rect_filled(half, 0.0, fill);
                    }
                    let Some(line) = line else {
                        continue;
                    };
                    painter.text(
                        egui::pos2(half.left() + gutter - char_w, half.center().y),
                        Align2::RIGHT_CENTER,
                        (line + 1).to_string(),
                        font.clone(),
                        pal.faint,
                    );
                    let text_rect = Rect::from_min_max(
                        egui::pos2(half.left() + gutter, half.top()),
                        half.right_bottom(),
                    );
                    let room = (text_rect.width() / char_w) as usize + 2;
                    if let Some(ranges) = self.inline.get(&index) {
                        let strong = if side == 0 { red_strong } else { green_strong };
                        let clip = painter.with_clip_rect(text_rect.intersect(painter.clip_rect()));
                        for range in &ranges[side] {
                            if range.end <= skip || range.start > skip + room {
                                continue;
                            }
                            let from = range.start.max(skip) - skip;
                            let to = range.end.min(skip + room) - skip;
                            clip.rect_filled(
                                Rect::from_min_max(
                                    egui::pos2(text_rect.left() + from as f32 * char_w, half.top() + 2.0),
                                    egui::pos2(text_rect.left() + to as f32 * char_w, half.bottom() - 2.0),
                                ),
                                2.0,
                                strong,
                            );
                        }
                    }
                    let shown: String = self.texts[side][line as usize]
                        .chars()
                        .skip(skip)
                        .take(room)
                        .map(|c| if c == '\t' { ' ' } else { c })
                        .collect();
                    painter
                        .with_clip_rect(text_rect.intersect(painter.clip_rect()))
                        .text(
                            text_rect.left_center(),
                            Align2::LEFT_CENTER,
                            shown,
                            font.clone(),
                            pal.text,
                        );
                }
                painter.line_segment(
                    [left.right_top(), left.right_bottom()],
                    egui::Stroke::new(1.0, pal.border),
                );
                if response.double_clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let (id, line) = if pos.x < left.right() {
                        (self.left, row.left.or(row.right))
                    } else {
                        (self.right, row.right.or(row.left))
                    };
                    if let (Some(id), Some(line)) = (id, line) {
                        open = Some((id, line as usize + 1));
                    }
                }
            }
        });
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn rows_align_equal_lines_and_mark_changes() {
        let old = text(&["a", "b", "c", "d", "e"]);
        let new = text(&["a", "B", "c", "e", "f", "g"]);
        let rows = build(&old, &new);
        let kinds: Vec<Kind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Same,
                Kind::Changed,
                Kind::Same,
                Kind::Removed,
                Kind::Same,
                Kind::Added,
                Kind::Added
            ]
        );
        assert_eq!(rows[1].left, Some(1));
        assert_eq!(rows[1].right, Some(1));
        assert_eq!(
            rows[3],
            Row {
                left: Some(3),
                right: None,
                kind: Kind::Removed
            }
        );
        assert_eq!(rows[4].left, Some(4));
        assert_eq!(rows[4].right, Some(3));
        let left: Vec<u32> = rows.iter().filter_map(|r| r.left).collect();
        let right: Vec<u32> = rows.iter().filter_map(|r| r.right).collect();
        assert_eq!(left, (0..5).collect::<Vec<u32>>());
        assert_eq!(right, (0..6).collect::<Vec<u32>>());
    }

    #[test]
    fn inline_changes_mark_only_differing_chars() {
        let [left, right] = inline("$price = 100;", "$price = 250;");
        assert!(!left.is_empty() && !right.is_empty());
        assert!(left.iter().chain(&right).all(|r| r.start >= 9 && r.end <= 12));
        let [left, right] = inline("echo $a;", "echo $a . $b;");
        assert!(left.is_empty());
        assert_eq!(right, vec![7..12]);
        let [left, right] = inline("abc", "xyz");
        assert!(left.is_empty() && right.is_empty());
    }

    #[test]
    fn identical_texts_have_no_changes() {
        let a = text(&["x", "y"]);
        assert!(build(&a, &a).iter().all(|r| r.kind == Kind::Same));
    }
}
