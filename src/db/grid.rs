use super::*;

impl DbPanel {
    pub(super) fn editable(&self) -> bool {
        self.source.is_some()
            && !self.keys.is_empty()
            && self
                .result
                .as_ref()
                .is_some_and(|t| !t.columns.is_empty() && !t.truncated)
    }

    pub(super) fn statements(&self) -> Vec<String> {
        let (Some(table), Some(source)) = (&self.result, &self.source) else {
            return Vec::new();
        };
        let kind = self.kind;
        let full = format!("{}.{}", kind.quote(&source.schema), kind.quote(&source.table));
        let key_index: Vec<usize> = self
            .keys
            .iter()
            .filter_map(|k| table.columns.iter().position(|c| c == k))
            .collect();
        let condition = |row: &[Option<String>]| {
            key_index
                .iter()
                .map(|&i| match &row[i] {
                    Some(value) => format!("{} = {}", kind.quote(&table.columns[i]), kind.literal(Some(value))),
                    None => format!("{} IS NULL", kind.quote(&table.columns[i])),
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        let mut out = Vec::new();
        let mut by_row: BTreeMap<usize, Vec<(usize, &Option<String>)>> = BTreeMap::new();
        for ((row, col), value) in &self.edits.cells {
            if !self.edits.deleted.contains(row) {
                by_row.entry(*row).or_default().push((*col, value));
            }
        }
        for (row, changes) in by_row {
            let Some(original) = table.rows.get(row) else {
                continue;
            };
            let set: Vec<String> = changes
                .iter()
                .map(|(col, value)| {
                    format!(
                        "{} = {}",
                        kind.quote(&table.columns[*col]),
                        kind.literal(value.as_deref())
                    )
                })
                .collect();
            out.push(format!(
                "UPDATE {full} SET {} WHERE {}",
                set.join(", "),
                condition(original)
            ));
        }
        for row in &self.edits.deleted {
            if let Some(original) = table.rows.get(*row) {
                out.push(format!("DELETE FROM {full} WHERE {}", condition(original)));
            }
        }
        for added in &self.edits.added {
            let pairs: Vec<(usize, &String)> = added
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.as_ref().map(|v| (i, v)))
                .collect();
            if pairs.is_empty() {
                out.push(match kind {
                    DbKind::MySql => format!("INSERT INTO {full} () VALUES ()"),
                    _ => format!("INSERT INTO {full} DEFAULT VALUES"),
                });
                continue;
            }
            let columns: Vec<String> = pairs.iter().map(|(i, _)| kind.quote(&table.columns[*i])).collect();
            let values: Vec<String> = pairs.iter().map(|(_, v)| kind.literal(Some(v))).collect();
            out.push(format!(
                "INSERT INTO {full} ({}) VALUES ({})",
                columns.join(", "),
                values.join(", ")
            ));
        }
        out
    }

    pub(super) fn grid(&mut self, ui: &mut Ui, pal: &Palette, font: &FontId, font_size: f32) {
        let editable = self.editable();
        let Some(table) = &self.result else {
            return;
        };
        if table.columns.is_empty() {
            return;
        }
        let char_w = ui.fonts_mut(|f| f.glyph_width(font, '0'));
        let row_h = (font_size * 1.7).round();
        let pad = 10.0;
        let widths: Vec<f32> = self.widths.iter().map(|w| w * char_w + pad * 2.0).collect();
        let number_w = 6.0 * char_w + pad;
        let total: f32 = number_w + widths.iter().sum::<f32>();
        let (header, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::hover());
        ui.painter().rect_filled(header, 0.0, pal.crust);
        let clip = ui.painter().with_clip_rect(header);
        let mut x = header.left() + number_w - self.scroll_x;
        for (name, w) in table.columns.iter().zip(&widths) {
            let key = self.keys.contains(name);
            let mut text_x = x + pad;
            if key {
                let cy = header.center().y;
                let stroke = egui::Stroke::new(1.4, pal.accent);
                clip.circle_stroke(egui::pos2(text_x + 3.5, cy), 3.0, stroke);
                clip.line_segment([egui::pos2(text_x + 6.5, cy), egui::pos2(text_x + 12.0, cy)], stroke);
                clip.line_segment([egui::pos2(text_x + 10.5, cy), egui::pos2(text_x + 10.5, cy + 3.0)], stroke);
                text_x += 16.0;
            }
            clip.text(
                egui::pos2(text_x, header.center().y),
                Align2::LEFT_CENTER,
                name.clone(),
                font.clone(),
                if key { pal.accent } else { pal.text },
            );
            x += w;
        }
        let base_rows = table.rows.len();
        let total_rows = base_rows + self.edits.added.len();
        let mut copy = None;
        let mut picked = self.picked;
        let mut start_edit = None;
        let mut set_null = None;
        let mut toggle_delete = None;
        let mut commit = None;
        let mut cancel_edit = false;
        let changed_color = pal.match_bg;
        ui.spacing_mut().item_spacing.y = 0.0;
        let out = ScrollArea::both()
            .id_salt("tror_db_grid")
            .auto_shrink(false)
            .show_rows(ui, row_h, total_rows, |ui, range| {
                for index in range {
                    let added = index >= base_rows;
                    let row_values: Vec<Option<String>> = if added {
                        self.edits.added[index - base_rows].clone()
                    } else {
                        table.rows[index].clone()
                    };
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(total.max(ui.available_width()), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter().clone();
                    let deleted = !added && self.edits.deleted.contains(&index);
                    if index % 2 == 1 {
                        painter.rect_filled(rect, 0.0, pal.base);
                    }
                    if added {
                        painter.rect_filled(rect, 0.0, pal.selection.gamma_multiply(0.45));
                    }
                    if response.hovered() {
                        painter.rect_filled(rect, 0.0, pal.current_line);
                    }
                    painter.text(
                        egui::pos2(rect.left() + number_w - pad, rect.center().y),
                        Align2::RIGHT_CENTER,
                        if added { "+".to_owned() } else { (index + 1).to_string() },
                        font.clone(),
                        pal.faint,
                    );
                    let pointer = response.interact_pointer_pos().or_else(|| {
                        response
                            .secondary_clicked()
                            .then(|| ui.input(|i| i.pointer.interact_pos()))
                            .flatten()
                    });
                    let mut x = rect.left() + number_w;
                    for (col, w) in widths.iter().enumerate() {
                        let cell = egui::Rect::from_min_size(
                            egui::pos2(x, rect.top()),
                            egui::vec2(*w, row_h),
                        );
                        if let Some(pos) = pointer
                            && cell.contains(pos)
                            && (response.clicked() || response.secondary_clicked())
                        {
                            picked = Some((index, col));
                        }
                        if let Some(pos) = pointer
                            && cell.contains(pos)
                            && response.double_clicked()
                            && editable
                            && !deleted
                        {
                            start_edit = Some((index, col));
                        }
                        let edited = if added {
                            None
                        } else {
                            self.edits.cells.get(&(index, col))
                        };
                        if edited.is_some() {
                            painter.rect_filled(cell.shrink(1.0), 2.0, changed_color);
                        }
                        let value = match edited {
                            Some(v) => v.clone(),
                            None => row_values.get(col).cloned().flatten(),
                        };
                        let editing_here = self
                            .edits
                            .editing
                            .as_ref()
                            .is_some_and(|(r, c, _)| *r == index && *c == col);
                        if editing_here {
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell.shrink(2.0)));
                            if let Some((_, _, text)) = &mut self.edits.editing {
                                let field = child.add(
                                    TextEdit::singleline(text)
                                        .font(font.clone())
                                        .desired_width(cell.width() - 6.0)
                                        .margin(egui::Margin::symmetric(4, 2)),
                                );
                                field.request_focus();
                                if child.input(|i| i.key_pressed(Key::Enter)) {
                                    commit = Some((index, col, text.clone()));
                                } else if child.input(|i| i.key_pressed(Key::Escape)) {
                                    cancel_edit = true;
                                } else if field.lost_focus() {
                                    commit = Some((index, col, text.clone()));
                                }
                            }
                        } else {
                            if picked == Some((index, col)) {
                                painter.rect_stroke(
                                    cell.shrink(1.0),
                                    3.0,
                                    Stroke::new(1.0, pal.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let (text, color) = match &value {
                                Some(text) => (text.as_str(), if deleted { pal.faint } else { pal.text }),
                                None => ("NULL", pal.faint),
                            };
                            let shown: String = text
                                .chars()
                                .take(self.widths[col] as usize)
                                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                                .collect();
                            let galley = painter
                                .with_clip_rect(cell.intersect(painter.clip_rect()))
                                .text(
                                    egui::pos2(x + pad, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    shown,
                                    font.clone(),
                                    color,
                                );
                            if deleted {
                                painter.line_segment(
                                    [galley.left_center(), galley.right_center()],
                                    Stroke::new(1.0, pal.warn),
                                );
                            }
                        }
                        x += w;
                    }
                    response.context_menu(|ui| {
                        if let Some((r, c)) = picked
                            && r == index
                        {
                            if ui.button(tr!("Копировать значение")).clicked() {
                                let value = if added {
                                    self.edits.added[r - base_rows][c].clone()
                                } else {
                                    self.edits
                                        .cells
                                        .get(&(r, c))
                                        .cloned()
                                        .unwrap_or_else(|| table.rows[r][c].clone())
                                };
                                copy = Some(value.unwrap_or_default());
                                ui.close();
                            }
                            if editable && !deleted {
                                if ui.button(tr!("Изменить значение")).clicked() {
                                    start_edit = Some((r, c));
                                    ui.close();
                                }
                                if ui.button(tr!("Сделать NULL")).clicked() {
                                    set_null = Some((r, c));
                                    ui.close();
                                }
                            }
                        }
                        if ui.button(tr!("Копировать строку")).clicked() {
                            copy = Some(
                                row_values
                                    .iter()
                                    .map(|c| c.as_deref().unwrap_or("NULL"))
                                    .collect::<Vec<_>>()
                                    .join("\t"),
                            );
                            ui.close();
                        }
                        if editable {
                            ui.separator();
                            let label = if added {
                                tr!("Убрать новую строку")
                            } else if deleted {
                                tr!("Не удалять строку")
                            } else {
                                tr!("Удалить строку")
                            };
                            if ui.button(label).clicked() {
                                toggle_delete = Some(index);
                                ui.close();
                            }
                        }
                    });
                }
            });
        self.scroll_x = out.state.clamped_offset().x;
        self.picked = picked;
        if let Some((row, col)) = start_edit {
            let current = if row >= base_rows {
                self.edits.added[row - base_rows][col].clone()
            } else {
                self.edits
                    .cells
                    .get(&(row, col))
                    .cloned()
                    .unwrap_or_else(|| table.rows[row][col].clone())
            };
            let long = current.as_ref().is_some_and(|v| v.chars().count() > MAX_CELL);
            if long {
                self.status = tr!("Значение длиннее 4000 символов показано не целиком — измените его запросом").into();
                self.error = true;
            } else if current.as_deref().is_some_and(|v| v.starts_with("[BLOB")) {
                self.status = tr!("Двоичные данные в сетке не правятся").into();
                self.error = true;
            } else {
                self.edits.editing = Some((row, col, current.unwrap_or_default()));
            }
        }
        if cancel_edit {
            self.edits.editing = None;
        }
        if let Some((row, col, text)) = commit {
            self.edits.editing = None;
            self.set_cell(row, col, Some(text));
        }
        if let Some((row, col)) = set_null {
            self.set_cell(row, col, None);
        }
        if let Some(row) = toggle_delete {
            if row >= base_rows {
                self.edits.added.remove(row - base_rows);
            } else if let Some(i) = self.edits.deleted.iter().position(|r| *r == row) {
                self.edits.deleted.remove(i);
            } else {
                self.edits.deleted.push(row);
            }
        }
        if let Some((r, c)) = picked
            && self.edits.editing.is_none()
            && ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
            && let Some(table) = &self.result
            && r < table.rows.len()
        {
            copy = Some(table.rows[r][c].clone().unwrap_or_default());
        }
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
        }
    }

    pub(super) fn set_cell(&mut self, row: usize, col: usize, value: Option<String>) {
        let Some(table) = &self.result else {
            return;
        };
        if row >= table.rows.len() {
            if let Some(added) = self.edits.added.get_mut(row - table.rows.len()) {
                added[col] = value;
            }
            return;
        }
        if table.rows[row][col] == value {
            self.edits.cells.remove(&(row, col));
        } else {
            self.edits.cells.insert((row, col), value);
        }
    }

    pub(super) fn open_editor(&mut self, store: &Store, new: bool) {
        let mut drafts = store.drafts::<DbProfile>();
        if new || drafts.is_empty() {
            drafts.push(DbProfile {
                name: tf!("База {}", "Database {}", drafts.len() + 1),
                ..Default::default()
            });
        }
        let index = if new {
            drafts.len() - 1
        } else {
            self.selected.min(drafts.len() - 1)
        };
        self.editor = Some(Editor {
            drafts,
            index,
            show_password: false,
            probe: None,
            result: None,
        });
    }
}
