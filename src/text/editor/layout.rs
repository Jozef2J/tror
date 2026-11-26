use super::*;

use crate::theme::TAB;

pub(super) fn tab_stops(job: &mut LayoutJob, char_w: f32) {
    if !job.text.contains('\t') {
        return;
    }
    let width = TAB.load(std::sync::atomic::Ordering::Relaxed).max(1) as usize;
    let mut gaps: Vec<(usize, f32)> = Vec::new();
    let mut col = 0;
    for (at, ch) in job.text.char_indices() {
        if ch == '\t' {
            let next = (col / width + 1) * width;
            gaps.push((at + 1, (next - col - 1) as f32 * char_w));
            col = next;
        } else {
            col += 1;
        }
    }
    let mut out = Vec::with_capacity(job.sections.len() + gaps.len());
    for section in std::mem::take(&mut job.sections) {
        let (start, end) = (section.byte_range.start.0, section.byte_range.end.0);
        let mut from = start;
        let mut lead = section.leading_space
            + gaps.iter().find(|(at, _)| *at == start).map_or(0.0, |g| g.1);
        for &(at, gap) in gaps.iter().filter(|(at, _)| *at > start && *at < end) {
            out.push(LayoutSection {
                leading_space: lead,
                byte_range: ByteIndex(from)..ByteIndex(at),
                format: section.format.clone(),
            });
            from = at;
            lead = gap;
        }
        out.push(LayoutSection {
            leading_space: lead,
            byte_range: ByteIndex(from)..ByteIndex(end),
            format: section.format,
        });
    }
    job.sections = out;
}

pub(super) fn plain_job(text: String, m: &Metrics, color: egui::Color32, wrap_width: f32) -> LayoutJob {
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

pub(super) fn layout_line(ui: &Ui, doc: &Document, line: usize, m: &Metrics) -> LineLayout {
    let len = doc.line_len(line);
    let start = doc.line_to_char(line);
    if m.wrap && len > LONG_LINE {
        let empty = plain_job(String::new(), m, doc.text_color(), f32::INFINITY);
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
    let text = String::from(doc.slice(start + c0..start + c1));
    let format = |color| TextFormat {
        line_height: Some(m.row_h),
        ..TextFormat::simple(m.font.clone(), color)
    };
    let default = doc.text_color();
    let mut sections = Vec::new();
    if c0 == 0 && c1 == len {
        let mut offset = 0;
        for span in doc.spans(line) {
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
    let mut job = if sections.is_empty() {
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
    if c0 == 0 {
        tab_stops(&mut job, m.char_w);
    }
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

pub(super) fn grid_row(
    ui: &Ui,
    doc: &Document,
    line_start: usize,
    layout: &LineLayout,
    row: usize,
    m: &Metrics,
) -> Arc<Galley> {
    let (start, end) = layout.row_range(row);
    let text = String::from(doc.slice(line_start + start..line_start + end));
    let job = plain_job(text, m, doc.text_color(), f32::INFINITY);
    ui.fonts_mut(|f| f.layout_job(job))
}

pub(super) fn visible_lines(ui: &Ui, doc: &Document, m: &Metrics, view_h: f32) -> Vec<(usize, f64)> {
    let row = m.row_h as f64;
    let scroll = doc.scroll_y.max(0.0);
    let first_row = ((scroll / row) as usize).min(doc.shown_count() - 1);
    let first = doc.line_of_visual(first_row);
    if !m.wrap {
        let offset = -(scroll - first_row as f64 * row);
        let mut out = Vec::new();
        let mut next = Some(first);
        while let Some(line) = next
            && out.len() < m.rows + 2
        {
            out.push((line, offset + out.len() as f64 * row));
            next = doc.next_shown(line);
        }
        return out;
    }
    let mut out = Vec::new();
    let mut y = 0.0;
    let mut next = Some(first);
    while let Some(line) = next {
        next = doc.next_shown(line);
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

pub(super) fn wrapped_step(ui: &Ui, doc: &Document, m: &Metrics, pos: usize, goal: f64, down: bool) -> usize {
    let line = doc.char_to_line(pos);
    let start = doc.line_to_char(line);
    let layout = layout_line(ui, doc, line, m);
    let (_, row) = layout.place(pos - start, m.char_w);
    if down {
        if row + 1 < layout.rows() {
            start + layout.col_at_row(goal, row + 1, m.char_w)
        } else if let Some(below) = doc.next_shown(line) {
            let next = layout_line(ui, doc, below, m);
            doc.line_to_char(below) + next.col_at_row(goal, 0, m.char_w)
        } else {
            doc.len()
        }
    } else if row > 0 {
        start + layout.col_at_row(goal, row - 1, m.char_w)
    } else if let Some(above) = doc.prev_shown(line) {
        let prev = layout_line(ui, doc, above, m);
        doc.line_to_char(above) + prev.col_at_row(goal, prev.rows() - 1, m.char_w)
    } else {
        0
    }
}

pub(super) fn move_vertical(ui: &Ui, doc: &mut Document, m: &Metrics, delta: isize, extend: bool) {
    let head = doc.sel.1;
    let line = doc.char_to_line(head);
    let col = head - doc.line_to_char(line);
    if m.wrap {
        let goal = doc.goal_x;
        let goal = goal.unwrap_or_else(|| layout_line(ui, doc, line, m).place(col, m.char_w).0);
        let mut pos = head;
        for _ in 0..delta.unsigned_abs() {
            let next = wrapped_step(ui, doc, m, pos, goal, delta > 0);
            if next == pos {
                break;
            }
            pos = next;
        }
        doc.move_to_goal(pos, extend, goal);
        return;
    }
    let goal = doc.goal_x;
    let goal = goal.unwrap_or_else(|| layout_line(ui, doc, line, m).x_of(col, m.char_w));
    let visual = (doc.to_visual(line) as isize + delta).clamp(0, doc.shown_count() as isize - 1);
    let target = doc.line_of_visual(visual as usize);
    let pos = if target == line && delta < 0 {
        0
    } else if target == line {
        doc.len()
    } else {
        doc.line_to_char(target) + layout_line(ui, doc, target, m).col_at(goal)
    };
    doc.move_to_goal(pos, extend, goal);
}
