use super::*;

impl RemotePanel {
    pub(super) fn start_search(&mut self, ctx: &egui::Context) {
        let query = self.search_query.trim().to_owned();
        if query.is_empty() || !self.connected {
            return;
        }
        let Some((profile, known)) = self.selected_profile() else {
            return;
        };
        let root = if self.search_root.trim().is_empty() {
            self.cwd.clone()
        } else {
            self.search_root.trim().to_owned()
        };
        let params = remote::SearchParams {
            root: root.clone(),
            query: query.clone(),
            content: self.search_content,
            case: self.search_case,
            word: self.search_word,
            regex: self.search_regex,
            excludes: self
                .search_excludes
                .split(',')
                .map(|e| e.trim().to_owned())
                .collect(),
        };
        let ctx = ctx.clone();
        let job = remote::search(profile, known, params, move || ctx.request_repaint());
        self.hits.clear();
        self.search_status = tr!("поиск…").into();
        self.push_log(tf!("Поиск «{query}» в {root}", "Searching \"{query}\" in {root}"), false);
        self.search_job = Some(job);
        let _ = self.store.set_excludes(self.search_excludes.clone());
    }

    pub fn search_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let ctx = ui.ctx().clone();
        ui.add_space(4.0);
        if !self.connected {
            ui.add(
                egui::Label::new(
                    RichText::new(tr!("Подключитесь к серверу в панели «Удалённые хосты»."))
                        .color(pal.faint)
                        .small(),
                )
                .wrap(),
            );
            return;
        }
        let mut start = false;
        use crate::widgets::{FORM_ROW, form_field, form_row, toggle};
        ui.spacing_mut().item_spacing.y = 8.0;
        form_row(ui, pal, tr!("Найти"), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = crate::widgets::field_width(ui, 3.0 * 32.0 + 34.0);
            let field = ui.add(
                form_field(&mut self.search_query, tr!("текст или выражение")).desired_width(width),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                start = true;
            }
            if crate::widgets::icon_button(ui, pal, crate::widgets::Icon::Close)
                .on_hover_text(tr!("Очистить поиск"))
                .clicked()
            {
                if let Some(job) = &self.search_job {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.search_query.clear();
                self.hits.clear();
                self.search_status.clear();
            }
            ui.add_space(6.0);
            toggle(ui, pal, &mut self.search_case, "Aa", tr!("Учитывать регистр"));
            toggle(ui, pal, &mut self.search_word, "ab", tr!("Слово целиком"));
            toggle(
                ui,
                pal,
                &mut self.search_regex,
                ".*",
                tr!("Регулярное выражение"),
            );
        });
        form_row(ui, pal, tr!("Искать"), |ui| {
            crate::widgets::segmented(
                ui,
                pal,
                &mut self.search_content,
                &[(true, tr!("В тексте файлов")), (false, tr!("В именах"))],
                120.0,
            );
        });
        form_row(ui, pal, tr!("Папка"), |ui| {
            let hint = self.cwd.clone();
            ui.add(
                form_field(&mut self.search_root, hint)
                    .desired_width(crate::widgets::field_width(ui, 94.0)),
            );
            if ui
                .add(
                    Button::new(tr!("Текущая"))
                        .min_size(egui::vec2(88.0, FORM_ROW)),
                )
                .on_hover_text(tr!("Папка, открытая в панели хоста"))
                .clicked()
            {
                self.search_root = self.cwd.clone();
            }
        });
        form_row(ui, pal, tr!("Не искать в"), |ui| {
            ui.add(
                form_field(
                    &mut self.search_excludes,
                    tr!("папки через запятую: bitrix/cache, node_modules"),
                )
                .desired_width(crate::widgets::field_width(ui, 0.0)),
            );
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let size = egui::vec2(0.0, FORM_ROW);
            if self.search_job.is_some() {
                if ui
                    .add(Button::new(tr!("Остановить")).min_size(size))
                    .clicked()
                    && let Some(job) = &self.search_job
                {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ui.spinner();
            } else if ui
                .add_enabled(
                    !self.search_query.trim().is_empty(),
                    crate::widgets::primary(tr!("Найти"), pal).min_size(size),
                ).lit(ui)
                .clicked()
            {
                start = true;
            }
            ui.label(
                RichText::new(&self.search_status)
                    .color(pal.subtext)
                    .small(),
            );
        });
        if !self.hits.is_empty() {
            ui.add_space(6.0);
            ui.separator();
        }
        if start && self.search_job.is_none() {
            self.start_search(&ctx);
        }
        let mut open = None;
        let row_h = crate::widgets::RESULT_ROW;
        ui.spacing_mut().item_spacing.y = 0.0;
        let base = if self.search_root.trim().is_empty() {
            self.cwd.clone()
        } else {
            self.search_root.trim().to_owned()
        };
        let tall = (self.hits.len() as f32 * row_h).min(crate::widgets::RESULTS);
        ScrollArea::vertical()
            .id_salt(("tror_server_hits", self.id))
            .min_scrolled_height(tall)
            .max_height(tall)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, self.hits.len(), |ui, rows| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    let (path, line, text) = &self.hits[row];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    if response.hovered() {
                        painter.rect_filled(rect, 4.0, pal.crust);
                    }
                    let shown = path
                        .strip_prefix(base.trim_end_matches('/'))
                        .map_or(path.as_str(), |p| p.trim_start_matches('/'));
                    let label = match line {
                        Some(n) => format!("{shown}:{n}"),
                        None => shown.to_owned(),
                    };
                    let placed = painter.text(
                        egui::pos2(rect.left() + 6.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(12.5),
                        pal.accent,
                    );
                    painter.text(
                        egui::pos2(placed.right() + 12.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::monospace(12.0),
                        pal.subtext,
                    );
                    if response.clicked() {
                        open = Some((path.clone(), *line));
                    }
                }
            },
        );
        if let Some((path, line)) = open {
            if let Some(line) = line {
                self.open_at.insert(path.clone(), line);
            }
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
        }
    }
}
