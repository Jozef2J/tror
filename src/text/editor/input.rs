use super::*;

pub(super) fn url_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"(?:https?|ftp)://[^\s"'<>()\[\]{}`]+|www\.[^\s"'<>()\[\]{}`]+"#)
            .unwrap_or_else(|_| regex::Regex::new("$^").unwrap_or_else(|_| unreachable!()))
    })
}

pub(super) fn url_spans(text: &str) -> Vec<(usize, usize)> {
    url_re()
        .find_iter(text)
        .map(|m| {
            let url = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let start = text[..m.start()].chars().count();
            (start, start + url.chars().count())
        })
        .collect()
}

pub(super) fn url_at(doc: &Document, pos: usize) -> Option<String> {
    let re = url_re();
    if pos > doc.len() {
        return None;
    }
    let line = doc.rope.char_to_line(pos);
    let start = doc.rope.line_to_char(line);
    if doc.line_len(line) > 4000 {
        return None;
    }
    let text: String = doc.rope.line(line).chars().collect();
    let col = pos - start;
    let byte = text.char_indices().nth(col).map_or(text.len(), |(b, _)| b);
    let found = re.find_iter(&text).find(|m| m.start() <= byte && byte < m.end())?;
    let url = found.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
    Some(if url.starts_with("www.") {
        format!("https://{url}")
    } else {
        url.to_owned()
    })
}

pub(super) fn line_span(doc: &Document, a: usize, b: usize) -> (usize, usize) {
    let (lo, hi) = (a.min(b), a.max(b));
    let start = doc.rope.line_to_char(lo);
    let end = doc.rope.line_to_char(hi) + doc.line_len(hi);
    if b < a { (end, start) } else { (start, end) }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Typed {
    None,
    Text,
    Backspace,
}

pub(super) fn handle_events(ui: &Ui, doc: &mut Document, m: &Metrics, macros: &mut Macro) -> Typed {
    let events = ui.input(|i| i.events.clone());
    macros.feed(&events);
    let typed = process_events(ui, doc, m, events);
    macros.replay(ui, doc, m);
    typed
}

pub(super) fn process_events(ui: &Ui, doc: &mut Document, m: &Metrics, events: Vec<Event>) -> Typed {
    let mut typed = Typed::None;
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
                let clean: String = text.chars().filter(|c| !c.is_control()).collect();
                if !clean.is_empty() {
                    doc.insert_text(&clean);
                }
            }
            Event::Paste(text) => {
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                if !text.is_empty()
                    && let Some(((l0, x0), (l1, x1))) = doc.block
                {
                    let col = (x0.min(x1) / m.char_w as f64).round().max(0.0) as usize;
                    let top = l0.min(l1);
                    let rows = l0.max(l1) - top + 1;
                    let body = text.strip_suffix('\n').unwrap_or(&text);
                    let mut lines: Vec<&str> = body.split('\n').collect();
                    if lines.len() == 1 && rows > 1 {
                        lines = vec![lines[0]; rows];
                    }
                    if doc.cursors().iter().any(|c| c.0 != c.1) {
                        doc.insert_text("");
                    }
                    doc.block_paste_at(top, col, &lines);
                    doc.block = None;
                } else if !text.is_empty() {
                    if !doc.multi() && is_block_clip(&text) {
                        if doc.has_selection() {
                            doc.insert_text("");
                        }
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

pub fn copy_selection(ctx: &egui::Context, doc: &mut Document, cut: bool) {
    let text = if doc.multi() {
        let text = doc.texts().join("\n");
        remember_block(Some(text.clone()));
        text
    } else {
        remember_block(None);
        doc.selected_text()
    };
    ctx.copy_text(text);
    if cut {
        doc.insert_text("");
    }
}

pub(super) fn sentence_case(text: &str) -> String {
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

pub(super) fn title_case(text: &str) -> String {
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

pub(super) fn invert_case(text: &str) -> String {
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

pub(super) fn context_items(ui: &mut Ui, doc: &mut Document, id: Id) {
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
    if item(ui, tr!("Вырезать"), keys("X"), selected) {
        copy_selection(ui.ctx(), doc, true);
    }
    if item(ui, tr!("Копировать"), keys("C"), selected) {
        copy_selection(ui.ctx(), doc, false);
    }
    if item(ui, tr!("Вставить"), keys("V"), true) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        ui.memory_mut(|mem| mem.request_focus(id));
    }
    if item(ui, tr!("Удалить"), "Delete".into(), true) {
        if selected {
            doc.insert_text("");
        } else {
            doc.delete(false);
        }
    }
    ui.separator();
    if item(ui, tr!("Выделить всё"), keys("A"), true) {
        doc.select_all();
    }
    ui.separator();
    if item(ui, tr!("Закомментировать / раскомментировать"), keys("/"), true) {
        doc.toggle_comment();
    }
    if item(ui, tr!("Дублировать строку"), String::new(), true) {
        doc.duplicate_lines();
    }
    if item(ui, tr!("Удалить строку"), String::new(), true) {
        doc.delete_lines();
    }
    if item(ui, tr!("Увеличить отступ"), "Tab".into(), true) {
        doc.indent();
    }
    if item(ui, tr!("Уменьшить отступ"), "Shift+Tab".into(), true) {
        doc.outdent();
    }
    ui.menu_button(tr!("Сворачивание"), |ui| {
        ui.set_min_width(200.0);
        if ui.button(tr!("Свернуть блок")).clicked() {
            doc.fold_here();
            ui.close();
        }
        if ui.button(tr!("Развернуть блок")).clicked() {
            doc.unfold_here();
            ui.close();
        }
        if ui.button(tr!("Свернуть всё")).clicked() {
            doc.fold_all();
            ui.close();
        }
        if ui.button(tr!("Развернуть всё")).clicked() {
            doc.unfold_all();
            ui.close();
        }
    });
    ui.separator();
    ui.menu_button(tr!("Регистр"), |ui| {
        ui.set_min_width(220.0);
        let cases: [(&str, fn(&str) -> String); 5] = [
            (tr!("ПРОПИСНЫЕ"), |t| t.to_uppercase()),
            (tr!("строчные"), |t| t.to_lowercase()),
            (tr!("Как в предложении"), sentence_case),
            (tr!("Каждое Слово С Заглавной"), title_case),
            (tr!("иНВЕРСИЯ рЕГИСТРА"), invert_case),
        ];
        for (label, f) in cases {
            if ui.button(label).clicked() {
                doc.transform(f);
                ui.close();
            }
        }
    });
}
