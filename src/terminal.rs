use eframe::egui::{
    self, Button, Color32, Event, EventFilter, FontId, Key, Modifiers, Rect, RichText, Sense,
    Stroke, Ui,
    text::{LayoutJob, TextFormat},
};

use crate::app::{dialog, dialog_button};
use crate::remote::{self, Profile, Protocol, Shell, ShellCmd, ShellEvent};
use crate::theme::Palette;

const SCROLLBACK: usize = 5000;

const DARK: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70,
    0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xcdd6f4,
];
const LIGHT: [u32; 16] = [
    0x5c5f77, 0xd20f39, 0x40a02b, 0xb8860b, 0x1e66f5, 0xea76cb, 0x179299, 0x7c7f93, 0x6c6f85,
    0xd20f39, 0x40a02b, 0xb8860b, 0x1e66f5, 0xea76cb, 0x179299, 0x4c4f69,
];

fn nz(value: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(value.max(1)).unwrap_or(std::num::NonZeroU16::MIN)
}

fn new_parser(cols: u16, rows: u16) -> vt100::Parser {
    vt100::Parser::new(nz(rows), nz(cols), SCROLLBACK)
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn indexed(index: u8, dark: bool) -> Color32 {
    match index {
        0..=15 => rgb(if dark { DARK } else { LIGHT }[index as usize]),
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

fn resolve(color: vt100::Color, dark: bool) -> Option<Color32> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(indexed(i, dark)),
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
        Key::F10 => b"\x1b[21~".to_vec(),
        _ => return None,
    })
}

pub enum TermEvent {
    Trust { host: String, fingerprint: String },
}

pub struct Terminal {
    pub visible: bool,
    pub focused: bool,
    shell: Option<Shell>,
    connected: bool,
    host_key: Option<(String, bool)>,
    address: String,
    parser: vt100::Parser,
    size: (u16, u16),
    status: String,
    wheel: f32,
    grab_focus: bool,
}

impl Terminal {
    pub fn new(visible: bool) -> Self {
        Self {
            visible,
            focused: false,
            shell: None,
            connected: false,
            host_key: None,
            address: String::new(),
            parser: new_parser(80, 24),
            size: (80, 24),
            status: String::new(),
            wheel: 0.0,
            grab_focus: false,
        }
    }

    pub fn modal_open(&self) -> bool {
        self.host_key.is_some()
    }

    fn connect(&mut self, profile: &Profile, known: Option<String>, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.parser = new_parser(self.size.0, self.size.1);
        self.address = format!("{}:{}", profile.host.trim(), profile.port);
        self.shell = Some(remote::open_shell(profile, known, self.size, move || {
            ctx.request_repaint()
        }));
        self.connected = false;
        self.status = "подключение…".into();
    }

    fn disconnect(&mut self, status: &str) {
        self.shell = None;
        self.connected = false;
        self.host_key = None;
        self.status = status.to_owned();
    }

    fn send(&self, bytes: Vec<u8>) {
        if let Some(shell) = &self.shell
            && self.connected
        {
            let _ = shell.tx.send(ShellCmd::Input(bytes));
        }
    }

    pub fn poll(&mut self) -> Vec<TermEvent> {
        let mut out = Vec::new();
        let mut events = Vec::new();
        if let Some(shell) = &self.shell {
            while let Ok(event) = shell.rx.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                ShellEvent::HostKey {
                    fingerprint,
                    changed,
                } => self.host_key = Some((fingerprint, changed)),
                ShellEvent::Connected { fingerprint } => {
                    self.connected = true;
                    self.grab_focus = true;
                    self.focused = true;
                    self.status.clear();
                    if let Some(fingerprint) = fingerprint {
                        out.push(TermEvent::Trust {
                            host: self.address.clone(),
                            fingerprint,
                        });
                    }
                }
                ShellEvent::Output(bytes) => self.parser.process(&bytes),
                ShellEvent::Closed(message) => self.disconnect(&message),
            }
        }
        out
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((fingerprint, changed)) = self.host_key.clone() else {
            return;
        };
        let mut answer = None;
        dialog("tror_shell_host_key", pal).show(ctx, |ui| {
            ui.set_width(460.0);
            if changed {
                ui.label(
                    RichText::new("Ключ сервера изменился")
                        .color(pal.warn)
                        .strong(),
                );
                ui.label(
                    "Сохранённый ключ этого сервера отличается от присланного. Так бывает после \
                     переустановки сервера, но так же выглядит перехват соединения.",
                );
            } else {
                ui.label(RichText::new("Новый сервер").strong());
                ui.label(
                    "К этому серверу вы подключаетесь впервые. Отпечаток его ключа (SHA-256):",
                );
            }
            ui.add_space(4.0);
            ui.label(RichText::new(&fingerprint).monospace().size(12.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let accept = if changed {
                    "Заменить ключ и подключиться"
                } else {
                    "Доверять и подключиться"
                };
                if dialog_button(ui, pal, accept, !changed) {
                    answer = Some(true);
                }
                if dialog_button(ui, pal, "Отмена", changed) {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                self.host_key = None;
                if let Some(shell) = &self.shell {
                    let _ = shell.tx.send(ShellCmd::TrustHost(true));
                }
            }
            Some(false) => self.disconnect("подключение отменено"),
            None => {}
        }
    }

    pub fn header(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        profile: Option<&Profile>,
        known: Option<String>,
    ) {
        let ctx = ui.ctx().clone();
        {
            if let Some(shell) = &self.shell {
                ui.label(RichText::new(&shell.host).color(pal.subtext).size(12.5));
            }
            if !self.status.is_empty() {
                ui.label(RichText::new(&self.status).color(pal.faint).size(12.5));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.shell.is_some() {
                    if ui.add(Button::new("Отключить").fill(pal.surface)).clicked() {
                        self.disconnect("отключено");
                    }
                } else {
                    match profile {
                        Some(p) if p.protocol == Protocol::Sftp => {
                            let label = format!("Подключить: {}", p.name);
                            if ui.add(Button::new(label).fill(pal.surface)).clicked() {
                                self.connect(p, known, &ctx);
                            }
                        }
                        Some(_) => {
                            ui.label(
                                RichText::new("для консоли выберите профиль SFTP")
                                    .color(pal.faint)
                                    .size(12.5),
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new("создайте профиль в панели удалённых файлов (F8)")
                                    .color(pal.faint)
                                    .size(12.5),
                            );
                        }
                    }
                }
            });
        }
    }

    pub fn body(&mut self, ui: &mut Ui, pal: &Palette, dark: bool, font_size: f32) {
        let font = FontId::monospace(font_size);
        let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
        let row_h = row_h.round().max(1.0);
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        ui.painter().rect_filled(rect, 0.0, pal.base);
        let area = rect.shrink2(egui::vec2(8.0, 4.0));
        let cols = ((area.width() / char_w) as u16).clamp(20, 400);
        let rows = ((area.height() / row_h) as u16).clamp(2, 200);
        if (cols, rows) != self.size {
            self.size = (cols, rows);
            self.parser.screen_mut().set_size(nz(rows), nz(cols));
            if let Some(shell) = &self.shell {
                let _ = shell.tx.send(ShellCmd::Resize(cols, rows));
            }
        }

        let id = egui::Id::unique("tror_terminal");
        let response = ui.interact(rect, id, Sense::click());
        if response.clicked() || std::mem::take(&mut self.grab_focus) {
            ui.memory_mut(|m| m.request_focus(id));
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
            self.input(ui);
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
            }
        }

        let screen = self.parser.screen();
        let painter = ui.painter_at(rect);
        let plain = |color| TextFormat::simple(font.clone(), color);
        for row in 0..rows {
            let mut job = LayoutJob::default();
            job.wrap.max_width = f32::INFINITY;
            let mut run = String::new();
            let mut run_style: Option<(Color32, Option<Color32>)> = None;
            let flush = |job: &mut LayoutJob,
                         run: &mut String,
                         style: Option<(Color32, Option<Color32>)>| {
                if let Some((fg, bg)) = style
                    && !run.is_empty()
                {
                    let mut format = plain(fg);
                    if let Some(bg) = bg {
                        format.background = bg;
                    }
                    job.append(run, 0.0, format);
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
                let mut fg = resolve(cell.fgcolor(), dark).unwrap_or(pal.text);
                let mut bg = resolve(cell.bgcolor(), dark);
                if cell.inverse() {
                    let old_fg = fg;
                    fg = bg.unwrap_or(pal.base);
                    bg = Some(old_fg);
                }
                if cell.dim() {
                    fg = fg.gamma_multiply(0.65);
                }
                let style = Some((fg, bg));
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
                    pal.text,
                );
            }
        }
        if self.connected && !screen.hide_cursor() && screen.scrollback() == 0 {
            let (row, col) = screen.cursor_position();
            let cursor = Rect::from_min_size(
                egui::pos2(
                    area.left() + col as f32 * char_w,
                    area.top() + row as f32 * row_h,
                ),
                egui::vec2(char_w, row_h),
            );
            if self.focused {
                painter.rect_filled(cursor, 1.0, pal.accent.gamma_multiply(0.55));
            } else {
                painter.rect_stroke(
                    cursor,
                    1.0,
                    Stroke::new(1.0, pal.faint),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    fn input(&mut self, ui: &Ui) {
        let events = ui.input(|i| i.events.clone());
        let app_cursor = self.parser.screen().application_cursor();
        let bracketed = self.parser.screen().bracketed_paste();
        let mut typed = false;
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
                self.send(bytes);
                typed = true;
            }
        }
        if typed {
            self.parser.screen_mut().set_scrollback(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(indexed(16, true), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231, true), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(232, true), Color32::from_rgb(8, 8, 8));
        assert_eq!(indexed(255, true), Color32::from_rgb(238, 238, 238));
        assert_ne!(indexed(1, true), indexed(1, false));
    }

    #[test]
    fn output_is_parsed_into_cells_with_colors() {
        let mut t = Terminal::new(true);
        t.parser.process(b"ok \x1b[31mred\x1b[0m\r\n$ ");
        let screen = t.parser.screen();
        assert_eq!(screen.cell(0, 0).unwrap().contents(), "o");
        assert_eq!(screen.cell(0, 3).unwrap().fgcolor(), vt100::Color::Idx(1));
        assert_eq!(screen.cursor_position(), (1, 2));
    }
}
