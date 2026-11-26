use super::*;

impl RemotePanel {
    pub(super) fn sync_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        use crate::remote::sync::Mode;
        let Some(dialog) = &mut self.sync else {
            return;
        };
        let plan = dialog.plan();
        let mut go = false;
        let mut close = false;
        egui::Window::new(tr!("Синхронизация папки"))
            .frame(crate::widgets::window_frame(pal))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 460.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                crate::widgets::window_header(ui, pal, tr!("Синхронизация папки"), false);
                egui::Grid::new("tror_sync_roots").num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new(tr!("На компьютере")).color(pal.subtext));
                    ui.label(dialog.local_root.display().to_string());
                    ui.end_row();
                    ui.label(RichText::new(tr!("На сервере")).color(pal.subtext));
                    ui.label(dialog.remote_root.as_str());
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.radio_value(&mut dialog.mode, Mode::Both, tr!("В обе стороны — побеждает более новый файл"));
                ui.radio_value(&mut dialog.mode, Mode::Up, tr!("Только на сервер"));
                ui.radio_value(&mut dialog.mode, Mode::Down, tr!("Только на компьютер"));
                ui.checkbox(
                    &mut dialog.by_time,
                    tr!("При одинаковом размере сравнивать время изменения"),
                );
                if dialog.no_times {
                    ui.label(
                        RichText::new(tr!(
                            "Сервер не умеет ставить время файлам (нет команды MFMT), поэтому сравнение по времени выключено: иначе одни и те же файлы копировались бы каждый раз."
                        ))
                        .small()
                        .color(pal.warn),
                    );
                }
                let one_way = dialog.mode != Mode::Both;
                ui.add_enabled(
                    one_way,
                    egui::Checkbox::new(
                        &mut dialog.remove,
                        if dialog.mode == Mode::Down {
                            tr!("Удалять на компьютере файлы, которых нет на сервере")
                        } else {
                            tr!("Удалять на сервере файлы, которых нет на компьютере")
                        },
                    ),
                );
                egui::Grid::new("tror_sync_filter").num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new(tr!("Только")).color(pal.subtext));
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.include)
                            .desired_width(360.0)
                            .hint_text(tr!("все файлы; или маски через запятую: *.php, *.css")),
                    );
                    ui.end_row();
                    ui.label(RichText::new(tr!("Кроме")).color(pal.subtext));
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.exclude)
                            .desired_width(360.0)
                            .hint_text(tr!("папки и маски через запятую: bitrix/cache, *.log")),
                    );
                    ui.end_row();
                });
                ui.label(
                    RichText::new(if one_way && dialog.remove {
                        tr!("Лишние файлы на стороне назначения будут удалены без возможности восстановления.")
                    } else {
                        tr!("Ничего не удаляется: файлы, которых нет на одной стороне, копируются на неё. \
                         Файлы с тем же именем перезаписываются.")
                    })
                    .small()
                    .color(if one_way && dialog.remove { pal.warn } else { pal.faint }),
                );
                ui.add_space(6.0);
                let bytes = |steps: &[crate::remote::sync::Step]| {
                    size_label(steps.iter().map(|s| s.size).sum::<u64>() as usize)
                };
                ui.label(tf!("На сервер: {} ({}) · на компьютер: {} ({}) · без изменений: {}", "To the server: {} ({}) · to the computer: {} ({}) · unchanged: {}",
                    plan.up.len(),
                    bytes(&plan.up),
                    plan.down.len(),
                    bytes(&plan.down),
                    plan.same
                ));
                ui.separator();
                if !plan.remove_remote.is_empty() || !plan.remove_local.is_empty() {
                    ui.label(
                        RichText::new(tf!(
                            "Будет удалено: на сервере {}, на компьютере {}",
                            "To be deleted: on the server {}, on the computer {}",
                            plan.remove_remote.len(),
                            plan.remove_local.len()
                        ))
                        .color(pal.warn),
                    );
                }
                let removals: Vec<crate::remote::sync::Step> = plan
                    .remove_remote
                    .iter()
                    .chain(&plan.remove_local)
                    .map(|r| crate::remote::sync::Step {
                        relative: r.clone(),
                        size: 0,
                        mtime: None,
                        fresh: false,
                    })
                    .collect();
                let remote_removals = plan.remove_remote.len();
                let rows: Vec<(u8, &crate::remote::sync::Step)> = plan
                    .up
                    .iter()
                    .map(|s| (0, s))
                    .chain(plan.down.iter().map(|s| (1, s)))
                    .chain(removals.iter().enumerate().map(|(i, s)| (if i < remote_removals { 2 } else { 3 }, s)))
                    .collect();
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ScrollArea::vertical()
                        .max_height((ui.available_height() - 44.0).max(80.0))
                        .auto_shrink(false)
                        .show_rows(ui, 20.0, rows.len(), |ui, range| {
                            for (up, step) in &rows[range] {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 20.0),
                                    Sense::hover(),
                                );
                                let painter = ui.painter_at(rect);
                                let what = match (up, step.fresh) {
                                    (0, true) => tr!("→ сервер, новый"),
                                    (0, false) => tr!("→ сервер, заменить"),
                                    (1, true) => tr!("← компьютер, новый"),
                                    (1, false) => tr!("← компьютер, заменить"),
                                    (2, _) => tr!("× удалить на сервере"),
                                    _ => tr!("× удалить на компьютере"),
                                };
                                let removal = *up >= 2;
                                painter.text(
                                    egui::pos2(rect.left() + 4.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    what,
                                    egui::FontId::proportional(12.0),
                                    if step.fresh { pal.subtext } else { pal.warn },
                                );
                                if removal {
                                    painter.text(
                                        egui::pos2(rect.left() + 170.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        &step.relative,
                                        egui::FontId::proportional(12.5),
                                        pal.warn,
                                    );
                                    continue;
                                }
                                painter.text(
                                    egui::pos2(rect.left() + 170.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &step.relative,
                                    egui::FontId::proportional(12.5),
                                    pal.text,
                                );
                                painter.text(
                                    egui::pos2(rect.right() - 6.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    size_label(step.size as usize),
                                    egui::FontId::proportional(11.5),
                                    pal.faint,
                                );
                            }
                        });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let total = plan.up.len() + plan.down.len() + plan.remove_remote.len() + plan.remove_local.len();
                    let label = if total == 0 {
                        tr!("Всё уже совпадает").to_owned()
                    } else {
                        tf!("Синхронизировать, файлов: {total}", "Sync, files: {total}")
                    };
                    go = ui
                        .add_enabled(
                            total > 0,
                            crate::widgets::primary(label, pal).min_size(crate::widgets::ACTION),
                        ).lit(ui)
                        .clicked();
                    close = crate::widgets::dialog_button(ui, pal, tr!("Отмена"), false);
                });
            });
        if go {
            let Some(dialog) = self.sync.take() else {
                return;
            };
            let count = plan.up.len() + plan.down.len();
            if count > 0 {
                self.push_log(
                    tf!(
                        "Синхронизация начата: {} ↔ {}, файлов: {count}",
                        "Sync started: {} ↔ {}, files: {count}",
                        dialog.local_root.display(),
                        dialog.remote_root
                    ),
                    false,
                );
                let batch = self.batch();
                batch.downloads |= !plan.down.is_empty();
                batch.uploads |= !plan.up.is_empty();
                batch.target = dialog.remote_root.clone();
                batch.folder = Some(dialog.local_root.clone());
                for step in plan.down.iter().chain(&plan.up) {
                    batch.add(remote::join(&dialog.remote_root, &step.relative), step.size);
                }
            }
            for relative in &plan.remove_local {
                if let Some(local) = crate::remote::sync::local_path(&dialog.local_root, relative)
                    && let Err(e) = std::fs::remove_file(&local)
                {
                    self.push_log(tf!("Не удалось удалить {relative}: {e}", "Could not delete {relative}: {e}"), true);
                }
            }
            for relative in &plan.remove_remote {
                let path = remote::join(&dialog.remote_root, relative);
                self.send(Cmd::Delete { path, dir: false }, tf!("Удаление {relative}…", "Deleting {relative}…"));
            }
            if !plan.remove_local.is_empty() || !plan.remove_remote.is_empty() {
                self.push_log(
                    tf!(
                        "Синхронизация: удалено на компьютере {}, на сервере {}",
                        "Sync: deleted on the computer {}, on the server {}",
                        plan.remove_local.len(),
                        plan.remove_remote.len()
                    ),
                    false,
                );
            }
            for step in &plan.down {
                let Some(local) = crate::remote::sync::local_path(&dialog.local_root, &step.relative) else {
                    self.push_log(tf!("Пропущено небезопасное имя: {}", "Skipped an unsafe name: {}", step.relative), true);
                    continue;
                };
                if let Some(dir) = local.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let path = remote::join(&dialog.remote_root, &step.relative);
                if let Some(mtime) = step.mtime {
                    self.sync_times.insert(local.clone(), mtime);
                }
                self.save_to.insert(path.clone(), local);
                self.send(Cmd::Download(path), tf!("Синхронизация: {}…", "Sync: {}…", step.relative));
            }
            for step in &plan.up {
                let Some(local) = crate::remote::sync::local_path(&dialog.local_root, &step.relative) else {
                    continue;
                };
                match std::fs::read(&local) {
                    Ok(data) => {
                        let path = remote::join(&dialog.remote_root, &step.relative);
                        self.send(
                            Cmd::Upload {
                                path: path.clone(),
                                data,
                                doc: None,
                                expect: None,
                            },
                            tf!("Синхронизация: {}…", "Sync: {}…", step.relative),
                        );
                        if let Some(mtime) = step.mtime
                            && !self.no_times
                        {
                            self.send(Cmd::Touch { path, mtime }, String::new());
                        }
                    }
                    Err(e) => {
                        self.push_log(tf!("Не удалось прочитать {}: {e}", "Could not read {}: {e}", step.relative), true)
                    }
                }
            }
        } else if close {
            self.sync = None;
        }
    }
}
