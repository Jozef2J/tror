use super::*;

pub(super) static BLOCK_CLIP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(super) fn remember_block(text: Option<String>) {
    if let Ok(mut clip) = BLOCK_CLIP.lock() {
        *clip = text;
    }
}

pub(super) fn is_block_clip(text: &str) -> bool {
    BLOCK_CLIP
        .lock()
        .ok()
        .and_then(|c| c.clone())
        .is_some_and(|c| c == text)
}

pub(super) fn apply_block(ui: &Ui, doc: &mut Document, m: &Metrics) {
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
        let start = doc.line_to_char(*line);
        let a = start + layout.col_at(x0);
        let h = start + layout.col_at(x1);
        cursors.push((a, h));
    }
    doc.set_block_cursors(cursors);
}

pub(super) fn block_step(ui: &Ui, doc: &mut Document, m: &Metrics, key: Key) {
    let head = doc.sel.1;
    let (anchor, mut target) = match doc.block {
        Some(block) => block,
        None => {
            let line = doc.char_to_line(head);
            let col = head - doc.line_to_char(line);
            let x = layout_line(ui, doc, line, m).x_of(col, m.char_w);
            let anchor_line = doc.char_to_line(doc.sel.0);
            let anchor_col = doc.sel.0 - doc.line_to_char(anchor_line);
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
    doc.set_block(anchor, target);
    apply_block(ui, doc, m);
}

pub(super) fn vertical_char(doc: &Document, pos: usize, down: bool) -> usize {
    let line = doc.char_to_line(pos);
    let col = pos - doc.line_to_char(line);
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
    doc.line_to_char(target) + col.min(doc.line_len(target))
}
