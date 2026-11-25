use super::*;

impl DbPanel {
    pub(super) fn search_view(&mut self, ui: &mut Ui, pal: &Palette) {
        use crate::widgets::{FORM_ROW, form_field, form_row, toggle};
        let mut start = false;
        ui.spacing_mut().item_spacing.y = 8.0;
        form_row(ui, pal, tr!("Найти"), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = crate::widgets::field_width(ui, 2.0 * 32.0 + 8.0);
            let field = ui.add(
                form_field(&mut self.find.needle, tr!("значение в любом столбце"))
                    .desired_width(width),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                start = true;
            }
            ui.add_space(6.0);
            toggle(ui, pal, &mut self.find.case, "Aa", tr!("Учитывать регистр"));
            toggle(ui, pal, &mut self.find.exact, "=", tr!("Значение целиком"));
        });
        let what = match self.kind {
            DbKind::Postgres => tr!("Схема"),
            _ => tr!("База"),
        };
        let schemas: Vec<String> = self.schemas.iter().map(|s| s.name.clone()).collect();
        if self.find.schema.is_empty() || !schemas.contains(&self.find.schema) {
            self.find.schema = if schemas.contains(&self.current) {
                self.current.clone()
            } else {
                schemas.first().cloned().unwrap_or_default()
            };
        }
        let tables: Vec<String> = self
            .schemas
            .iter()
            .find(|s| s.name == self.find.schema)
            .and_then(|s| s.tables.clone())
            .unwrap_or_default();
        let mut load = None;
        form_row(ui, pal, what, |ui| {
            egui::ComboBox::from_id_salt("tror_db_find_schema")
                .width(crate::widgets::field_width(ui, 2.0 * 32.0 + 8.0))
                .selected_text(&self.find.schema)
                .show_ui(ui, |ui| {
                    for name in &schemas {
                        if ui
                            .selectable_label(self.find.schema == *name, name)
                            .clicked()
                        {
                            self.find.schema = name.clone();
                            self.find.table = None;
                        }
                    }
                });
        });
        form_row(ui, pal, tr!("Таблица"), |ui| {
            let all = tr!("все таблицы");
            let response = egui::ComboBox::from_id_salt("tror_db_find_table")
                .width(crate::widgets::field_width(ui, 2.0 * 32.0 + 8.0))
                .selected_text(self.find.table.as_deref().unwrap_or(all))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.find.table.is_none(), all)
                        .clicked()
                    {
                        self.find.table = None;
                    }
                    for name in &tables {
                        if ui
                            .selectable_label(self.find.table.as_deref() == Some(name), name)
                            .clicked()
                        {
                            self.find.table = Some(name.clone());
                        }
                    }
                });
            if response.response.clicked() && tables.is_empty() {
                load = Some(self.find.schema.clone());
            }
        });
        if let Some(schema) = load {
            self.send(Cmd::Tables(schema));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let size = egui::vec2(0.0, FORM_ROW);
            if ui
                .add_enabled(
                    !self.find.needle.is_empty() && !self.busy && !self.find.schema.is_empty(),
                    crate::widgets::primary(tr!("Найти"), pal).min_size(size),
                )
                .lit(ui)
                .clicked()
            {
                start = true;
            }
            if self.busy && self.searching {
                ui.spinner();
            }
        });
        if start && !self.busy && !self.find.needle.is_empty() {
            self.found = None;
            self.searching = true;
            self.status = tr!("Поиск…").into();
            self.error = false;
            self.send(Cmd::Search(self.find.clone()));
        }
        let Some((found, _)) = &self.found else {
            return;
        };
        if found.is_empty() {
            return;
        }
        ui.add_space(4.0);
        ui.separator();
        let mut open = None;
        let row_h = crate::widgets::RESULT_ROW;
        ui.spacing_mut().item_spacing.y = 0.0;
        ScrollArea::vertical()
            .id_salt("tror_db_found")
            .auto_shrink([false, true])
            .show_rows(ui, row_h, found.len(), |ui, rows| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    let (table, count, sql) = &found[row];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let rows_label = tf!("строк: {count}", "rows: {count}");
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{table} · {rows_label}"))
                    });
                    let painter = ui.painter_at(rect);
                    if response.hovered() {
                        painter.rect_filled(rect, 4.0, pal.crust);
                    }
                    let placed = painter.text(
                        egui::pos2(rect.left() + 6.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        table,
                        egui::FontId::proportional(12.5),
                        pal.accent,
                    );
                    painter.text(
                        egui::pos2(placed.right() + 12.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        rows_label,
                        egui::FontId::proportional(12.0),
                        pal.subtext,
                    );
                    if response
                        .on_hover_text(tr!("Открыть найденные строки"))
                        .clicked()
                    {
                        open = Some((table.clone(), sql.clone()));
                    }
                }
            });
        if let Some((table, sql)) = open {
            let schema = self.find.schema.clone();
            self.searching = false;
            self.mode = Mode::Query;
            self.sql = sql.clone();
            self.current = schema.clone();
            self.run_on(sql, Some(Source { schema, table }));
        }
    }
}
