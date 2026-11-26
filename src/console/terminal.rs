use eframe::egui::{
    self, Color32, Event, EventFilter, FontId, Key, Modifiers, Rect, Sense, Stroke, Ui,
    text::{LayoutJob, TextFormat},
};

use crate::theme::Colors;

pub static SCROLLBACK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(5000);

fn nz(value: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(value.max(1)).unwrap_or(std::num::NonZeroU16::MIN)
}

fn new_parser(cols: u16, rows: u16) -> vt100::Parser {
    vt100::Parser::new(
        nz(rows),
        nz(cols),
        SCROLLBACK.load(std::sync::atomic::Ordering::Relaxed),
    )
}

type Style = (vt100::Color, vt100::Color, bool, bool, bool, bool, bool);

fn style_of(cell: &vt100::Cell) -> Style {
    (
        cell.fgcolor(),
        cell.bgcolor(),
        cell.bold(),
        cell.dim(),
        cell.italic(),
        cell.underline(),
        cell.inverse(),
    )
}

fn push_style(out: &mut Vec<u8>, (fg, bg, bold, dim, italic, underline, inverse): Style) {
    let mut code = String::from("\x1b[0");
    for (on, flag) in [
        (bold, 1),
        (dim, 2),
        (italic, 3),
        (underline, 4),
        (inverse, 7),
    ] {
        if on {
            code.push_str(&format!(";{flag}"));
        }
    }
    for (color, base) in [(fg, 38), (bg, 48)] {
        match color {
            vt100::Color::Default => {}
            vt100::Color::Idx(i) => code.push_str(&format!(";{base};5;{i}")),
            vt100::Color::Rgb(r, g, b) => code.push_str(&format!(";{base};2;{r};{g};{b}")),
        }
    }
    code.push('m');
    out.extend_from_slice(code.as_bytes());
}

fn push_row(screen: &vt100::Screen, row: u16, cols: u16, keep: u16, out: &mut Vec<u8>) -> usize {
    let wrapped = screen.row_wrapped(row);
    let plain = Style::default();
    let mut end = cols;
    if !wrapped {
        while end > keep
            && screen
                .cell(row, end - 1)
                .is_none_or(|c| !c.has_contents() && style_of(c).1 == plain.1)
        {
            end -= 1;
        }
    }
    let mut style = plain;
    for col in 0..end {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let next = style_of(cell);
        if next != style {
            push_style(out, next);
            style = next;
        }
        if cell.has_contents() {
            out.extend_from_slice(cell.contents().as_bytes());
        } else {
            out.push(b' ');
        }
    }
    if style != plain {
        out.extend_from_slice(b"\x1b[m");
    }
    end as usize
}

fn reflow(old: &mut vt100::Parser, cols: u16, rows: u16) -> vt100::Parser {
    let screen = old.screen_mut();
    screen.set_scrollback(0);
    let (_, old_cols) = screen.size();
    let old_cols = old_cols.get();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let modes = screen.input_mode_formatted();
    let hidden = screen.hide_cursor();
    let mut out = Vec::new();
    let fit = old_cols.min(cols.max(1));
    let mut held = cursor_row;
    while held > 0 && screen.row_wrapped(held - 1) {
        held -= 1;
    }
    let mut held_back = 0;
    if held == 0 {
        while held_back < screen.scrollback_len() {
            screen.set_scrollback(held_back + 1);
            if !screen.row_wrapped(0) {
                break;
            }
            held_back += 1;
        }
    }
    let mut joined = false;
    for back in (1..=screen.scrollback_len()).rev() {
        screen.set_scrollback(back);
        if back <= held_back {
            out.extend_from_slice(b"\r\n");
            push_row(screen, 0, fit, 0, &mut out);
            continue;
        }
        if !joined {
            out.extend_from_slice(b"\r\n");
        }
        push_row(screen, 0, old_cols, 0, &mut out);
        joined = screen.row_wrapped(0);
    }
    screen.set_scrollback(0);
    for row in 0..held {
        if !joined {
            out.extend_from_slice(b"\r\n");
        }
        push_row(screen, row, old_cols, 0, &mut out);
        joined = screen.row_wrapped(row);
    }
    for row in held..=cursor_row {
        out.extend_from_slice(b"\r\n");
        let keep = if row == cursor_row { cursor_col } else { 0 };
        push_row(screen, row, fit, keep.min(fit), &mut out);
    }
    out.extend_from_slice(format!("\x1b[{}G", cursor_col.min(fit - 1) + 1).as_bytes());
    let mut parser = new_parser(cols, rows);
    parser.process(out.get(2..).unwrap_or(&[]));
    parser.process(modes.as_bytes());
    if hidden {
        parser.process(b"\x1b[?25l");
    }
    parser
}

fn indexed(index: u8, c: &Colors) -> Color32 {
    match index {
        0..=15 => c.ansi[index as usize],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (index - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

fn resolve(color: vt100::Color, c: &Colors) -> Option<Color32> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(indexed(i, c)),
        vt100::Color::Rgb(r, g, b) => Some(Color32::from_rgb(r, g, b)),
    }
}

pub fn key_bytes(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let arrow = |c: u8| {
        if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    if modifiers.ctrl || modifiers.command {
        let name = key.name();
        if let [letter @ b'A'..=b'Z'] = name.as_bytes() {
            return Some(vec![letter - b'A' + 1]);
        }
    }
    Some(match key {
        Key::Enter => vec![b'\r'],
        Key::Backspace => vec![0x7f],
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1b],
        Key::ArrowUp => arrow(b'A'),
        Key::ArrowDown => arrow(b'B'),
        Key::ArrowRight => arrow(b'C'),
        Key::ArrowLeft => arrow(b'D'),
        Key::Home => arrow(b'H'),
        Key::End => arrow(b'F'),
        Key::Insert => b"\x1b[2~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => b"\x1b[15~".to_vec(),
        Key::F6 => b"\x1b[17~".to_vec(),
        Key::F7 => b"\x1b[18~".to_vec(),
        Key::F8 => b"\x1b[19~".to_vec(),
        Key::F9 => b"\x1b[20~".to_vec(),
        Key::F10 => b"\x1b[21~".to_vec(),
        Key::F11 => b"\x1b[23~".to_vec(),
        Key::F12 => b"\x1b[24~".to_vec(),
        _ => return None,
    })
}

pub struct Look<'a> {
    pub colors: &'a Colors,
    pub font: FontId,
    pub cursor: bool,
    pub active: bool,
}

pub struct Screen {
    parser: vt100::Parser,
    size: (u16, u16),
    wheel: f32,
    select: Option<((u16, u16), (u16, u16))>,
    wanted: Option<((u16, u16), f64)>,
    pub grab_focus: bool,
    pub focused: bool,
    tail: Vec<u8>,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            parser: new_parser(80, 24),
            size: (80, 24),
            wheel: 0.0,
            select: None,
            wanted: None,
            grab_focus: false,
            focused: false,
            tail: Vec::new(),
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn reset(&mut self) {
        self.parser = new_parser(self.size.0, self.size.1);
        self.select = None;
    }

    pub fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        let done = self.tail.len();
        let mut seen = std::mem::take(&mut self.tail);
        seen.extend_from_slice(bytes);
        let mut reply = Vec::new();
        let mut fed = done;
        let mut at = 0;
        while let Some(i) = seen[at..].iter().position(|&b| b == 0x1b) {
            let start = at + i;
            let rest = &seen[start..];
            if rest.starts_with(b"\x1b[6n") {
                let end = (start + 4).max(fed);
                self.parser.process(&seen[fed..end]);
                fed = end;
                let (row, col) = self.parser.screen().cursor_position();
                reply.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            } else if rest.starts_with(b"\x1b[c") || rest.starts_with(b"\x1b[0c") {
                reply.extend_from_slice(b"\x1b[?1;2c");
            } else if rest.len() < 4 && (b"\x1b[6n".starts_with(rest) || b"\x1b[0c".starts_with(rest)) {
                self.tail = rest.to_vec();
            }
            at = start + 1;
        }
        self.parser.process(&seen[fed..]);
        reply
    }

    pub fn settle(&mut self, cols: u16, rows: u16, now: f64) -> bool {
        if (cols, rows) == self.size {
            self.wanted = None;
            return false;
        }
        match self.wanted {
            Some((size, since)) if size == (cols, rows) => {
                if now - since < 0.2 {
                    return false;
                }
                self.wanted = None;
                self.resize(cols, rows)
            }
            _ => {
                self.wanted = Some(((cols, rows), now));
                false
            }
        }
    }

    pub fn unsettled(&self) -> bool {
        self.wanted.is_some()
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> bool {
        if (cols, rows) == self.size {
            return false;
        }
        self.size = (cols, rows);
        self.select = None;
        if self.parser.screen().alternate_screen() {
            self.parser.screen_mut().set_size(nz(rows), nz(cols));
        } else {
            self.parser = reflow(&mut self.parser, cols, rows);
        }
        true
    }

    fn selection(&self) -> Option<((u16, u16), (u16, u16))> {
        let (a, b) = self.select?;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        (start != end).then_some((start, end))
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((r0, c0), (r1, c1)) = self.selection()?;
        let text = self.parser.screen().contents_between(r0, c0, r1, c1);
        let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
        Some(lines.join("\n"))
    }

    #[cfg(test)]
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    pub fn grid(&self, font: &FontId, ui: &Ui, rect: Rect) -> (u16, u16) {
        let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(font, '0'), f.row_height(font)));
        let area = rect.shrink2(egui::vec2(8.0, 4.0));
        let cols = ((area.width() / char_w.max(1.0)) as u16).clamp(10, 500);
        let rows = ((area.height() / row_h.round().max(1.0)) as u16).clamp(2, 300);
        (cols, rows)
    }

    pub fn show(&mut self, ui: &mut Ui, rect: Rect, id: egui::Id, look: &Look) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let font = &look.font;
        let c = look.colors;
        let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(font, '0'), f.row_height(font)));
        let row_h = row_h.round().max(1.0);
        ui.painter().rect_filled(rect, 0.0, c.bg);
        let area = rect.shrink2(egui::vec2(8.0, 4.0));
        let (cols, rows) = self.size;

        let response = ui.interact(rect, id, Sense::click_and_drag());
        if response.clicked() || response.drag_started() || std::mem::take(&mut self.grab_focus) {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let cell_at = |pos: egui::Pos2| {
            let row = ((pos.y - area.top()) / row_h)
                .floor()
                .clamp(0.0, rows as f32 - 1.0);
            let col = ((pos.x - area.left()) / char_w)
                .round()
                .clamp(0.0, cols as f32);
            (row as u16, col as u16)
        };
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        {
            self.select = Some((cell_at(pos), cell_at(pos)));
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let (Some(pos), Some((anchor, _))) = (response.interact_pointer_pos(), self.select)
        {
            self.select = Some((anchor, cell_at(pos)));
        } else if response.double_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (row, col) = cell_at(pos);
            let screen = self.parser.screen();
            let word = |col: u16| {
                screen.cell(row, col).is_some_and(|cell| {
                    let text = cell.contents();
                    !text.is_empty() && !text.chars().all(char::is_whitespace)
                })
            };
            let mut from = col.min(cols.saturating_sub(1));
            let mut to = from;
            while from > 0 && word(from - 1) {
                from -= 1;
            }
            while to < cols && word(to) {
                to += 1;
            }
            self.select = Some(((row, from), (row, to)));
        } else if response.clicked() {
            self.select = None;
        }
        if response.secondary_clicked() {
            if let Some(text) = self.selected_text() {
                ui.ctx().copy_text(text);
                self.select = None;
            } else {
                ui.memory_mut(|m| m.request_focus(id));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
        }
        self.focused = ui.memory(|m| m.has_focus(id));
        if self.focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            out = self.input(ui);
        }
        if response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            self.wheel += delta;
            let lines = (self.wheel / row_h) as isize;
            if lines != 0 {
                self.wheel -= lines as f32 * row_h;
                let screen = self.parser.screen_mut();
                let target = (screen.scrollback() as isize + lines).max(0) as usize;
                screen.set_scrollback(target);
                self.select = None;
            }
        }
        let selection = self.selection();

        let screen = self.parser.screen();
        let painter = ui.painter_at(rect);
        let format = |color, background, underline: bool, italic: bool| {
            let mut format = TextFormat::simple(font.clone(), color);
            format.background = background;
            format.italics = italic;
            if underline {
                format.underline = Stroke::new(1.0, color);
            }
            format
        };
        for row in 0..rows {
            if let Some(((r0, c0), (r1, c1))) = selection
                && row >= r0
                && row <= r1
            {
                let from = if row == r0 { c0 } else { 0 };
                let to = if row == r1 { c1 } else { cols };
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(
                            area.left() + from as f32 * char_w,
                            area.top() + row as f32 * row_h,
                        ),
                        egui::pos2(
                            area.left() + to as f32 * char_w,
                            area.top() + (row + 1) as f32 * row_h,
                        ),
                    ),
                    0.0,
                    c.selection,
                );
            }
            let mut job = LayoutJob::default();
            job.wrap.max_width = f32::INFINITY;
            let mut heavy = LayoutJob::default();
            heavy.wrap.max_width = f32::INFINITY;
            let mut any_bold = false;
            let mut run = String::new();
            type Style = (Color32, Color32, bool, bool, bool);
            let mut run_style: Option<Style> = None;
            let mut flush = |job: &mut LayoutJob, run: &mut String, style: Option<Style>| {
                if let Some((fg, bg, underline, italic, bold)) = style
                    && !run.is_empty()
                {
                    job.append(run, 0.0, format(fg, bg, underline, italic));
                    let ink = if bold { fg } else { Color32::TRANSPARENT };
                    heavy.append(run, 0.0, format(ink, Color32::TRANSPARENT, false, italic));
                    any_bold |= bold;
                    run.clear();
                }
            };
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    break;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = resolve(cell.fgcolor(), c).unwrap_or(c.fg);
                if cell.bold()
                    && let vt100::Color::Idx(i @ 0..=7) = cell.fgcolor()
                {
                    fg = indexed(i + 8, c);
                }
                let mut bg = resolve(cell.bgcolor(), c);
                if cell.inverse() {
                    let old_fg = fg;
                    fg = bg.unwrap_or(c.bg);
                    bg = Some(old_fg);
                }
                if cell.dim() {
                    fg = fg.gamma_multiply(0.65);
                }
                let style = Some((
                    fg,
                    bg.unwrap_or(Color32::TRANSPARENT),
                    cell.underline(),
                    cell.italic(),
                    cell.bold(),
                ));
                if style != run_style {
                    flush(&mut job, &mut run, run_style);
                    run_style = style;
                }
                if cell.has_contents() {
                    run.push_str(cell.contents());
                } else {
                    run.push(' ');
                }
            }
            flush(&mut job, &mut run, run_style);
            let origin = egui::pos2(area.left(), area.top() + row as f32 * row_h);
            if !job.text.trim_end().is_empty()
                || job
                    .sections
                    .iter()
                    .any(|s| s.format.background != Color32::TRANSPARENT)
            {
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                painter.galley(
                    egui::pos2(area.left(), area.top() + row as f32 * row_h),
                    galley,
                    c.fg,
                );
            }
            if any_bold {
                let galley = ui.fonts_mut(|f| f.layout_job(heavy));
                painter.galley(origin + egui::vec2((char_w * 0.07).max(0.6), 0.0), galley, c.fg);
            }
        }
        if look.cursor && !screen.hide_cursor() && screen.scrollback() == 0 {
            let (row, col) = screen.cursor_position();
            let cursor = Rect::from_min_size(
                egui::pos2(
                    area.left() + col as f32 * char_w,
                    area.top() + row as f32 * row_h,
                ),
                egui::vec2(char_w, row_h),
            );
            if self.focused {
                painter.rect_filled(cursor, 1.0, c.cursor.gamma_multiply(0.6));
            } else {
                painter.rect_stroke(
                    cursor,
                    1.0,
                    Stroke::new(1.0, c.cursor.gamma_multiply(0.7)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if look.active {
            painter.rect_stroke(
                rect.shrink(0.5),
                0.0,
                Stroke::new(1.0, c.cursor.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
        }
        out
    }

    fn input(&mut self, ui: &Ui) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let events = ui.input(|i| i.events.clone());
        let app_cursor = self.parser.screen().application_cursor();
        let bracketed = self.parser.screen().bracketed_paste();
        for event in events {
            let bytes = match event {
                Event::Text(text) => Some(text.into_bytes()),
                Event::Ime(egui::ImeEvent::Commit(text)) => Some(text.into_bytes()),
                Event::Paste(text) => {
                    let text = text.replace("\r\n", "\r").replace('\n', "\r");
                    Some(if bracketed {
                        format!("\x1b[200~{text}\x1b[201~").into_bytes()
                    } else {
                        text.into_bytes()
                    })
                }
                Event::Copy if self.has_selection() => {
                    if let Some(text) = self.selected_text() {
                        ui.ctx().copy_text(text);
                    }
                    self.select = None;
                    None
                }
                Event::Copy => Some(vec![0x03]),
                Event::Cut => Some(vec![0x18]),
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => key_bytes(key, modifiers, app_cursor),
                _ => None,
            };
            if let Some(bytes) = bytes
                && !bytes.is_empty()
            {
                out.push(bytes);
            }
        }
        if !out.is_empty() {
            self.parser.screen_mut().set_scrollback(0);
            self.select = None;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{SCHEMES, colors};

    #[test]
    fn answers_cursor_and_device_queries() {
        let mut t = Screen::new();
        assert_eq!(t.process(b"ab\x1b[6n"), b"\x1b[1;3R");
        assert_eq!(t.process(b"\r\nx\x1b"), b"");
        assert_eq!(t.process(b"[6ny"), b"\x1b[2;2R");
        assert_eq!(t.process(b"\x1b[c"), b"\x1b[?1;2c");
        assert_eq!(t.process(b"plain"), b"");
    }

    #[test]
    fn keys_map_to_terminal_sequences() {
        let none = Modifiers::NONE;
        assert_eq!(key_bytes(Key::Enter, none, false), Some(vec![b'\r']));
        assert_eq!(
            key_bytes(Key::ArrowUp, none, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes(Key::ArrowUp, none, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(key_bytes(Key::D, Modifiers::CTRL, false), Some(vec![4]));
        assert_eq!(key_bytes(Key::L, Modifiers::CTRL, false), Some(vec![12]));
        assert_eq!(
            key_bytes(Key::Delete, none, false),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(key_bytes(Key::A, none, false), None);
    }

    #[test]
    fn palette_covers_all_indices() {
        let pal = crate::theme::palette(true, 0, &Default::default());
        let dark = colors("", &pal, true);
        let light = colors("", &pal, false);
        assert_eq!(indexed(16, &dark), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231, &dark), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(232, &dark), Color32::from_rgb(8, 8, 8));
        assert_eq!(indexed(255, &dark), Color32::from_rgb(238, 238, 238));
        assert_ne!(indexed(1, &dark), indexed(1, &light));
        let dracula = colors("Dracula", &pal, true);
        assert_eq!(dracula.bg, Color32::from_rgb(0x28, 0x2a, 0x36));
        assert!(SCHEMES.iter().all(|s| !s.name.is_empty()));
    }

    #[test]
    fn output_is_parsed_into_cells_with_colors() {
        let mut t = Screen::new();
        t.process(b"ok \x1b[31mred\x1b[0m\r\n$ ");
        let screen = t.parser.screen();
        assert_eq!(screen.cell(0, 0).unwrap().contents(), "o");
        assert_eq!(screen.cell(0, 3).unwrap().fgcolor(), vt100::Color::Idx(1));
        assert_eq!(screen.cursor_position(), (1, 2));
    }

    #[test]
    fn shrinking_and_growing_keeps_all_output() {
        let mut t = Screen::new();
        let line = "0123456789".repeat(7);
        for i in 0..5 {
            t.process(format!("{i}:{line}\r\n").as_bytes());
        }
        t.resize(20, 6);
        t.resize(30, 4);
        t.resize(120, 24);
        let text = t.text();
        for i in 0..5 {
            assert!(text.contains(&format!("{i}:{line}")), "{text}");
        }
    }

    fn all_text(t: &mut Screen) -> String {
        let screen = t.parser.screen_mut();
        let mut lines = Vec::new();
        for back in (1..=screen.scrollback_len()).rev() {
            screen.set_scrollback(back);
            lines.push(screen.rows(0, 500).next().unwrap_or_default());
        }
        screen.set_scrollback(0);
        lines.extend(screen.rows(0, 500));
        lines.join("\n")
    }

    #[test]
    fn prompt_redraw_after_resize_does_not_erase_output() {
        let mut t = Screen::new();
        let prompt = "user@host:~/sites/shop/local/templates$ ";
        for round in 0..3 {
            t.process(
                format!("out-{round}-a\r\nout-{round}-b\r\nout-{round}-c\r\n{prompt}").as_bytes(),
            );
        }
        let mut above = 0;
        for (cols, rows) in [(12, 3), (100, 30), (10, 2), (160, 50), (11, 2), (35, 6)]
            .into_iter()
            .cycle()
            .take(24)
        {
            t.resize(cols, rows);
            let climb = "\r\x1b[A\x1b[K".repeat(above);
            t.process(format!("\r\x1b[K{climb}\r{prompt}").as_bytes());
            above = (prompt.len() - 1) / cols as usize;
        }
        t.resize(100, 30);
        let climb = "\r\x1b[A\x1b[K".repeat(above);
        t.process(format!("\r\x1b[K{climb}\r{prompt}").as_bytes());
        let text = all_text(&mut t);
        for round in 0..3 {
            for part in ["a", "b", "c"] {
                assert!(text.contains(&format!("out-{round}-{part}")), "{text}");
            }
        }
        assert!(text.trim_end().ends_with(prompt.trim_end()), "{text}");
        assert_eq!(text.matches("user@host").count(), 3, "{text}");
    }

    #[test]
    fn reflow_keeps_colors_cursor_and_long_history() {
        let mut t = Screen::new();
        for i in 0..400 {
            t.process(format!("row {i} \x1b[1;32mgreen\x1b[0m tail\r\n").as_bytes());
        }
        t.process(b"$ ab\x1b[D");
        for (cols, rows) in [(15, 4), (200, 50), (10, 2), (80, 24)] {
            t.resize(cols, rows);
        }
        let text = all_text(&mut t);
        for i in [0, 1, 199, 399] {
            assert!(text.contains(&format!("row {i} green tail")), "{i}");
        }
        let screen = t.parser.screen();
        let (row, col) = screen.cursor_position();
        assert_eq!(col, 3);
        assert_eq!(screen.cell(row, 2).unwrap().contents(), "a");
        let above = screen.cell(row - 1, 8).unwrap();
        assert_eq!(
            screen.rows(0, 80).nth(row as usize - 1).unwrap(),
            "row 399 green tail"
        );
        assert_eq!(above.fgcolor(), vt100::Color::Idx(2));
        assert!(above.bold());
        t.process(b"X");
        assert_eq!(t.parser.screen().cell(row, 3).unwrap().contents(), "X");
    }

    #[test]
    fn full_screen_program_survives_resize() {
        let mut t = Screen::new();
        t.process(b"before\r\n\x1b[?1049h\x1b[2J\x1b[Hvim");
        t.resize(40, 10);
        assert!(t.parser.screen().alternate_screen());
        t.process(b"\x1b[?1049l");
        assert!(t.text().contains("before"));
    }

    #[test]
    #[ignore]
    fn live_shell_keeps_output_through_resizes() {
        use crate::remote::{Profile, ShellCmd, ShellEvent, open_shell};
        let mut profile = Profile::new("live".into());
        profile.host = "127.0.0.1".into();
        profile.port = 2222;
        profile.user = "jozef".into();
        profile.password = "secret".into();
        let shell = open_shell(&profile, None, (100, 30), String::new(), || {});
        let mut t = Screen::new();
        t.resize(100, 30);
        let pump = |t: &mut Screen, millis: u64| {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(millis);
            while std::time::Instant::now() < until {
                match shell.rx.recv_timeout(std::time::Duration::from_millis(20)) {
                    Ok(ShellEvent::HostKey { .. }) => {
                        shell.tx.send(ShellCmd::TrustHost(true)).unwrap();
                    }
                    Ok(ShellEvent::Output(bytes)) => {
                        t.process(&bytes);
                    }
                    Ok(ShellEvent::Closed(e)) => panic!("{e}"),
                    _ => {}
                }
            }
        };
        pump(&mut t, 1500);
        for command in [
            "PS1='jozef@production-server:/var/www/html/project$ '",
            "echo first-marker; seq 101 130",
            "ls -la /tmp/srvroot/www",
            "echo second-marker-with-a-rather-long-tail-of-words-to-force-wrapping-on-narrow-screens",
            "echo third-marker",
        ] {
            shell
                .tx
                .send(ShellCmd::Input(format!("{command}\r").into_bytes()))
                .unwrap();
            pump(&mut t, 400);
        }
        let before = all_text(&mut t);
        assert!(before.contains("third-marker"), "{before}");
        for (cols, rows) in [
            (10, 2),
            (100, 30),
            (12, 3),
            (160, 50),
            (10, 2),
            (35, 6),
            (100, 30),
        ]
        .into_iter()
        .cycle()
        .take(28)
        {
            if t.resize(cols, rows) {
                shell.tx.send(ShellCmd::Resize(cols, rows)).unwrap();
            }
            pump(&mut t, 120);
        }
        let after = all_text(&mut t);
        let kept: Vec<&str> = after.lines().map(str::trim_end).collect();
        let lost: Vec<&str> = before
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty() && !line.contains('$'))
            .filter(|line| !kept.contains(line))
            .collect();
        assert!(lost.is_empty(), "пропало {lost:#?}\nбыло:\n{before}\nстало:\n{after}");
    }

    #[test]
    #[ignore]
    fn perf_reflow() {
        let mut t = Screen::new();
        t.resize(160, 40);
        for i in 0..6000 {
            t.process(format!("\x1b[32mline {i}\x1b[0m some text to fill the row with a realistic amount of output 0123456789\r\n").as_bytes());
        }
        for (cols, rows) in [(80, 24), (160, 40), (30, 10), (160, 40)] {
            let at = std::time::Instant::now();
            t.resize(cols, rows);
            eprintln!(
                "reflow -> {cols}x{rows}: {:.1} мс",
                at.elapsed().as_secs_f64() * 1e3
            );
        }
    }
}
