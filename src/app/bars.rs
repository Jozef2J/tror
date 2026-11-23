use super::*;
use crate::widgets::Lit;

impl TrorApp {
    pub(super) fn show_popups(&mut self, ctx: &egui::Context, pal: &Palette) {
        if self.popups.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - 44.0;
        let mut close = None;
        for (index, (text, ok, _)) in self.popups.iter().enumerate().rev() {
            let width = (screen.width() * 0.5).clamp(260.0, 520.0);
            let font = FontId::proportional(13.0);
            let galley =
                ctx.fonts_mut(|f| f.layout(text.clone(), font.clone(), pal.text, width - 46.0));
            let height = galley.size().y + 22.0;
            y -= height;
            let rect = Rect::from_min_size(
                egui::pos2(screen.right() - width - 16.0, y),
                egui::vec2(width, height),
            );
            y -= 8.0;
            let color = if *ok {
                Color32::from_rgb(0x3f, 0xc2, 0x6e)
            } else {
                pal.warn
            };
            egui::Area::new(egui::Id::unique(("tror_popup", index)))
                .order(egui::Order::Tooltip)
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(rect.size(), Sense::click());
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    let painter = ui.painter();
                    painter.rect(
                        rect,
                        10.0,
                        pal.mantle,
                        Stroke::new(1.0, color.gamma_multiply(0.7)),
                        egui::StrokeKind::Inside,
                    );
                    painter.rect_filled(
                        Rect::from_min_size(
                            rect.min + egui::vec2(0.0, 8.0),
                            egui::vec2(3.0, rect.height() - 16.0),
                        ),
                        1.5,
                        color,
                    );
                    let c = rect.left_top() + egui::vec2(21.0, 19.0);
                    painter.circle_stroke(c, 6.5, Stroke::new(1.4, color));
                    let mark = if *ok {
                        vec![
                            c + egui::vec2(-3.0, 0.2),
                            c + egui::vec2(-0.8, 2.6),
                            c + egui::vec2(3.2, -2.4),
                        ]
                    } else {
                        vec![c + egui::vec2(-2.6, -2.6), c + egui::vec2(2.6, 2.6)]
                    };
                    painter.add(egui::Shape::line(mark, Stroke::new(1.5, color)));
                    if !*ok {
                        painter.line_segment(
                            [c + egui::vec2(2.6, -2.6), c + egui::vec2(-2.6, 2.6)],
                            Stroke::new(1.5, color),
                        );
                    }
                    painter.galley(rect.left_top() + egui::vec2(34.0, 11.0), galley, pal.text);
                    if response.clicked() {
                        close = Some(index);
                    }
                });
        }
        if let Some(index) = close {
            self.popups.remove(index);
        }
    }

    pub(super) fn tool_button(
        ui: &mut Ui,
        pal: &Palette,
        kind: u8,
        active: bool,
        hint: &str,
    ) -> bool {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 26.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                7.0,
                if response.is_pointer_button_down_on() {
                    pal.surface_hover
                } else {
                    pal.surface
                },
            );
        }
        let color = if active {
            pal.accent
        } else if response.hovered() {
            pal.text
        } else {
            pal.subtext
        };
        let stroke = Stroke::new(1.4, color);
        let c = rect.center();
        let p = ui.painter();
        let frame = Rect::from_center_size(c, egui::vec2(16.0, 13.0));
        match kind {
            5 => {
                p.rect_stroke(frame, 2.5, stroke, egui::StrokeKind::Inside);
                p.text(
                    c + egui::vec2(0.0, -0.5),
                    egui::Align2::CENTER_CENTER,
                    "A",
                    egui::FontId::proportional(10.5),
                    color,
                );
                p.line_segment(
                    [c + egui::vec2(-4.0, 4.5), c + egui::vec2(4.0, 4.5)],
                    Stroke::new(1.2, color),
                );
            }
            0 => {
                p.rect_stroke(frame, 2.5, stroke, egui::StrokeKind::Inside);
                p.line_segment(
                    [
                        egui::pos2(frame.left() + 5.5, frame.top() + 1.0),
                        egui::pos2(frame.left() + 5.5, frame.bottom() - 1.0),
                    ],
                    stroke,
                );
            }
            1 => {
                p.rect_stroke(frame, 2.5, stroke, egui::StrokeKind::Inside);
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(-4.0, -2.5),
                        c + egui::vec2(-1.0, 0.0),
                        c + egui::vec2(-4.0, 2.5),
                    ],
                    stroke,
                ));
                p.line_segment([c + egui::vec2(0.5, 3.0), c + egui::vec2(4.5, 3.0)], stroke);
            }
            2 => {
                p.line_segment(
                    [c + egui::vec2(-7.0, -5.0), c + egui::vec2(7.0, -5.0)],
                    stroke,
                );
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(-7.0, 0.0),
                        c + egui::vec2(5.0, 0.0),
                        c + egui::vec2(7.0, 2.5),
                        c + egui::vec2(5.0, 5.0),
                        c + egui::vec2(0.0, 5.0),
                    ],
                    stroke,
                ));
                p.add(egui::Shape::line(
                    vec![
                        c + egui::vec2(2.5, 2.5),
                        c + egui::vec2(0.0, 5.0),
                        c + egui::vec2(2.5, 7.5),
                    ],
                    stroke,
                ));
                p.line_segment(
                    [c + egui::vec2(-7.0, 5.0), c + egui::vec2(-3.5, 5.0)],
                    stroke,
                );
            }
            4 => {
                p.line_segment([c - egui::vec2(5.5, 0.0), c + egui::vec2(5.5, 0.0)], stroke);
                p.line_segment([c - egui::vec2(0.0, 5.5), c + egui::vec2(0.0, 5.5)], stroke);
            }
            _ => {
                p.circle_stroke(c, 6.0, stroke);
                let half: Vec<egui::Pos2> = (0..=12)
                    .map(|i| {
                        let a =
                            std::f32::consts::FRAC_PI_2 + i as f32 / 12.0 * std::f32::consts::PI;
                        c + egui::vec2(a.cos() * 6.0, a.sin() * 6.0)
                    })
                    .collect();
                p.add(egui::Shape::convex_polygon(half, color, Stroke::NONE));
            }
        }
        response.on_hover_text(hint).clicked()
    }

    pub(super) fn search_bar(&mut self, ui: &mut Ui, pal: &Palette, _actions: &mut Vec<Action>) {
        use crate::widgets::{FORM_ROW, form_field, form_row, toggle};
        let selection = self.docs.get(self.active).map(|d| {
            let range = d.selection();
            (range.start, range.end)
        });
        let s = &mut self.search;
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
            if query.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
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
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        let matches = &self.search.matches;
        if self.search.query.is_empty() || matches.is_empty() || self.search.visual.is_some() {
            return;
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
        if let Some(sel) = pick {
            doc.sel = sel;
            doc.goal_x = None;
            doc.reveal = true;
        }
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        let now = ui.input(|i| i.time);
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, at)| now - at < 4.0)
            .map(|(text, _)| text.clone())
            .or_else(|| {
                self.plugins
                    .busy()
                    .map(|name| tf!("Плагин «{name}» работает…", "Plugin \"{name}\" is running…"))
            });
        if toast.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        let mut error = None;
        ui.horizontal(|ui| {
            let Some(doc) = self.docs.get_mut(self.active) else {
                ui.label(
                    RichText::new(toast.unwrap_or_default())
                        .color(pal.subtext)
                        .small(),
                );
                return;
            };
            let left = toast.unwrap_or_else(|| doc.location().unwrap_or_else(|| doc.title()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let small = |text: String| RichText::new(text).small().color(pal.subtext);

                ui.menu_button(small(doc.eol.label().into()), |ui| {
                    for eol in [Eol::Lf, Eol::Crlf, Eol::Cr] {
                        let name = match eol {
                            Eol::Lf => "LF — Unix",
                            Eol::Crlf => "CRLF — Windows",
                            Eol::Cr => tr!("CR — классический Mac"),
                        };
                        if ui.selectable_label(doc.eol == eol, name).clicked() && doc.eol != eol {
                            doc.eol = eol;
                            doc.mark_modified();
                        }
                    }
                });

                ui.menu_button(small(encoding_label(doc.encoding, doc.bom)), |ui| {
                    ui.weak(tr!("Сохранять в кодировке"));
                    for (name, encoding, bom) in ENCODINGS {
                        let selected = doc.encoding == *encoding && doc.bom == *bom;
                        if ui.selectable_label(selected, *name).clicked() && !selected {
                            doc.encoding = encoding;
                            doc.bom = *bom;
                            doc.mark_modified();
                        }
                    }
                    if doc.path.is_some() {
                        ui.separator();
                        ui.menu_button(tr!("Переоткрыть как"), |ui| {
                            for (name, encoding, bom) in ENCODINGS {
                                if *bom && *encoding == encoding_rs::UTF_8 {
                                    continue;
                                }
                                if ui.button(*name).clicked()
                                    && let Err(e) = doc.reopen_as(encoding)
                                {
                                    error = Some(e);
                                }
                            }
                        });
                    }
                });

                let config =
                    MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
                MenuButton::new(small(doc.syntax.clone()))
                    .config(config)
                    .ui(ui, |ui| {
                        ui.set_width(220.0);
                        ui.add(
                            TextEdit::singleline(&mut self.lang_filter)
                                .hint_text(tr!("Язык…"))
                                .desired_width(f32::INFINITY),
                        )
                        .request_focus();
                        let filter = self.lang_filter.to_lowercase();
                        ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for name in &self.syntaxes.names {
                                if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                                    continue;
                                }
                                let selected = *name == doc.syntax;
                                if ui.selectable_label(selected, name.as_str()).clicked() {
                                    doc.syntax = name.clone();
                                    self.lang_filter.clear();
                                    ui.close();
                                }
                            }
                        });
                    });

                ui.add_space(8.0);
                let (line, col) = doc.cursor_line_col();
                ui.label(small(tf!(
                    "Стр {line}, Стлб {col}   ·   строк: {}   ·   {}",
                    "Ln {line}, Col {col}   ·   lines: {}   ·   {}",
                    doc.line_count(),
                    size_label(doc.rope.len_bytes())
                )));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(small(left)).truncate());
                });
            });
        });
        if let Some(e) = error {
            self.notify(
                tf!("Не удалось переоткрыть: {e}", "Could not reopen: {e}"),
                now,
            );
        }
    }

    pub(super) fn welcome(&self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, pal.base);
        let files: Vec<PathBuf> = self.settings.recent.iter().take(6).cloned().collect();
        let folders: Vec<PathBuf> = self
            .settings
            .recent_projects
            .iter()
            .take(4)
            .cloned()
            .collect();
        let logo = rect.height() >= 460.0;
        let list_rows = files.len() + folders.len();
        let lists = list_rows as f32 * 26.0
            + if files.is_empty() { 0.0 } else { 34.0 }
            + if folders.is_empty() { 0.0 } else { 34.0 };
        let content = if logo { 102.0 } else { 0.0 } + 44.0 + 24.0 + 14.0 + 3.0 * 38.0 + lists;
        egui::ScrollArea::vertical()
            .id_salt("tror_welcome")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(
                        ((rect.height() - content) / 2.0).clamp(12.0, rect.height() * 0.3),
                    );
                    if logo && let Some(image) = &self.logo {
                        ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                        ui.add_space(6.0);
                    }
                    ui.label(RichText::new("TrorPad").size(34.0).color(pal.faint));
                    ui.label(
                        RichText::new(tf!("версия {}", "version {}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(pal.faint),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(tr!("Перетащите файлы сюда или начните с пустой вкладки"))
                            .color(pal.subtext),
                    );
                    ui.add_space(14.0);
                    let button = |text: &str| {
                        Button::new(RichText::new(text).color(pal.text))
                            .min_size(egui::vec2(200.0, 32.0))
                    };
                    if ui.add(button(tr!("Новый файл"))).clicked() {
                        actions.push(Action::New);
                    }
                    if ui.add(button(tr!("Открыть…"))).clicked() {
                        actions.push(Action::Open);
                    }
                    if ui.add(button(tr!("Открыть папку…"))).clicked() {
                        actions.push(Action::OpenFolder);
                    }
                    let recent = |ui: &mut Ui,
                                  title: &str,
                                  paths: &[PathBuf],
                                  folder: bool,
                                  actions: &mut Vec<Action>| {
                        if paths.is_empty() {
                            return;
                        }
                        ui.add_space(18.0);
                        ui.label(RichText::new(title).small().color(pal.faint));
                        ui.add_space(4.0);
                        for path in paths {
                            let name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            let parent = path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            let mut job = egui::text::LayoutJob::default();
                            job.append(
                                &name,
                                0.0,
                                egui::TextFormat::simple(
                                    egui::FontId::proportional(13.5),
                                    pal.text,
                                ),
                            );
                            job.append(
                                &parent,
                                10.0,
                                egui::TextFormat::simple(
                                    egui::FontId::proportional(12.0),
                                    pal.faint,
                                ),
                            );
                            job.wrap.max_width = 420.0;
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let response = ui
                                .add(
                                    Button::new(job)
                                        .frame_when_inactive(false)
                                        .min_size(egui::vec2(420.0, 24.0)),
                                )
                                .on_hover_text(path.display().to_string());
                            if response.clicked() {
                                actions.push(if folder {
                                    Action::OpenProject(path.clone())
                                } else {
                                    Action::OpenPath(path.clone())
                                });
                            }
                        }
                    };
                    recent(ui, tr!("НЕДАВНИЕ ФАЙЛЫ"), &files, false, actions);
                    recent(ui, tr!("НЕДАВНИЕ ПАПКИ"), &folders, true, actions);
                    ui.add_space(12.0);
                });
            });
    }
}
