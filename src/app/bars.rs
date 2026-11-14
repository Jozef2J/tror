use super::*;

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
        if response.hovered() {
            ui.painter().rect_filled(rect, 7.0, pal.surface);
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

    pub(super) fn search_bar(&mut self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let selection = self.docs.get(self.active).map(|d| {
            let range = d.selection();
            (range.start, range.end)
        });
        let s = &mut self.search;
        let toggle = |ui: &mut Ui, value: &mut bool, label: &str, hint: &str| {
            let text = RichText::new(label).monospace().color(if *value {
                pal.accent
            } else {
                pal.subtext
            });
            let mut button = Button::new(text).min_size(egui::vec2(28.0, 24.0));
            if *value {
                button = button.fill(pal.surface);
            }
            let clicked = ui.add(button).on_hover_text(hint).clicked();
            if clicked {
                *value = !*value;
            }
            clicked
        };
        let field_w = (ui.available_width() - 196.0).max(60.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let query = ui.add(
                TextEdit::singleline(&mut s.query)
                    .hint_text("Найти")
                    .desired_width(field_w)
                    .margin(Margin::symmetric(8, 5)),
            );
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
            changed |= toggle(ui, &mut s.case, "Aa", "Учитывать регистр");
            changed |= toggle(ui, &mut s.word, "ab", "Слово целиком");
            changed |= toggle(ui, &mut s.regex, ".*", "Регулярное выражение");
            if changed {
                s.pending = Some(Pending::FromAnchor);
                s.last_replaced = None;
            }
            if icon_button(ui, pal, Icon::Up)
                .on_hover_text("Найти ранее (Shift+F3)")
                .clicked()
            {
                s.pending = Some(Pending::Prev);
            }
            if icon_button(ui, pal, Icon::Down)
                .on_hover_text("Найти далее (F3)")
                .clicked()
            {
                s.pending = Some(Pending::Next);
            }
            if icon_button(ui, pal, Icon::Close)
                .on_hover_text("Закрыть (Esc)")
                .clicked()
            {
                actions.push(Action::CloseSearch);
            }
        });
        if s.replace_mode {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add(
                    TextEdit::singleline(&mut s.replacement)
                        .hint_text(if s.regex {
                            "Заменить на ($1, ${name})"
                        } else {
                            "Заменить на"
                        })
                        .desired_width(field_w)
                        .margin(Margin::symmetric(8, 5)),
                );
                if ui.button("Заменить").clicked() {
                    s.pending = Some(Pending::Replace);
                }
                if ui.button("Все").on_hover_text("Заменить все").clicked() {
                    s.pending = Some(Pending::ReplaceAll);
                }
            });
        }
        ui.horizontal(|ui| {
            let info = if let Some(error) = &s.error {
                RichText::new(error).color(pal.warn)
            } else if let Some(n) = s.last_replaced {
                RichText::new(format!("заменено: {n}")).color(pal.subtext)
            } else if s.query.is_empty() {
                RichText::new("введите текст").color(pal.faint)
            } else if s.searching() {
                RichText::new("поиск…").color(pal.subtext)
            } else if s.matches.is_empty() {
                RichText::new("нет совпадений").color(pal.warn)
            } else {
                let total = if s.truncated {
                    format!("{}+", s.matches.len())
                } else {
                    s.matches.len().to_string()
                };
                match selection.and_then(|sel| s.current(sel)) {
                    Some(i) => RichText::new(format!("{} из {total}", i + 1)),
                    None => RichText::new(format!("совпадений: {total}")),
                }
                .color(pal.subtext)
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if s.replace_mode {
                    "Скрыть замену"
                } else {
                    "Замена"
                };
                if ui
                    .add(Button::new(RichText::new(label).small().color(pal.subtext)).frame(false))
                    .clicked()
                {
                    s.replace_mode = !s.replace_mode;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(info.small()).truncate());
                });
            });
        });
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui, pal: &Palette) {
        let now = ui.input(|i| i.time);
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, at)| now - at < 4.0)
            .map(|(text, _)| text.clone());
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
                            Eol::Cr => "CR — классический Mac",
                        };
                        if ui.selectable_label(doc.eol == eol, name).clicked() && doc.eol != eol {
                            doc.eol = eol;
                            doc.mark_modified();
                        }
                    }
                });

                ui.menu_button(small(encoding_label(doc.encoding, doc.bom)), |ui| {
                    ui.weak("Сохранять в кодировке");
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
                        ui.menu_button("Переоткрыть как", |ui| {
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
                                .hint_text("Язык…")
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
                ui.label(small(format!(
                    "Стр {line}, Стлб {col}   ·   строк: {}   ·   {}",
                    doc.line_count(),
                    size_label(doc.rope.len_bytes())
                )));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(small(left)).truncate());
                });
            });
        });
        if let Some(e) = error {
            self.notify(format!("Не удалось переоткрыть: {e}"), now);
        }
    }

    pub(super) fn welcome(&self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, pal.base);
        ui.vertical_centered(|ui| {
            ui.add_space((rect.height() * 0.3).max(20.0));
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                ui.add_space(6.0);
            }
            ui.label(RichText::new("Tror").size(34.0).color(pal.faint));
            ui.add_space(4.0);
            ui.label(
                RichText::new("Перетащите файлы сюда или начните с пустой вкладки")
                    .color(pal.subtext),
            );
            ui.add_space(14.0);
            let button = |text: &str| {
                Button::new(RichText::new(text).color(pal.text))
                    .fill(pal.surface)
                    .min_size(egui::vec2(200.0, 32.0))
            };
            if ui.add(button("Новый файл")).clicked() {
                actions.push(Action::New);
            }
            if ui.add(button("Открыть…")).clicked() {
                actions.push(Action::Open);
            }
            if ui.add(button("Открыть папку…")).clicked() {
                actions.push(Action::OpenFolder);
            }
        });
    }
}
