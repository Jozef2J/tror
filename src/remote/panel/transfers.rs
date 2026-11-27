use super::*;

impl RemotePanel {
    pub(super) fn transfers_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        if self.transfers.is_empty() && self.failed.is_empty() {
            return;
        }
        let waiting = self
            .transfers
            .iter()
            .filter(|t| !matches!(t.cmd, Cmd::Touch { .. }))
            .count();
        ui.add_space(4.0);
        if waiting > 0 {
            ui.horizontal(|ui| {
                let text = if self.paused {
                    tf!("Пауза · ждут {waiting}", "Paused · waiting {waiting}")
                } else {
                    tf!("Ждут передачи: {waiting}", "Waiting to transfer: {waiting}")
                };
                ui.label(RichText::new(text).small().color(pal.subtext));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(crate::widgets::secondary(tr!("Отменить"), pal).small()).clicked() {
                        let dropped: Vec<Transfer> = self.transfers.drain(..).collect();
                        self.forget(dropped);
                        self.paused = false;
                    }
                    let label = if self.paused { tr!("Продолжить") } else { tr!("Пауза") };
                    if ui.add(crate::widgets::secondary(label, pal).small()).clicked() {
                        self.paused = !self.paused;
                        self.pump();
                    }
                });
            });
        }
        if !self.failed.is_empty() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tf!("Не передано: {}", "Not transferred: {}", self.failed.len()))
                        .small()
                        .color(pal.warn),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(crate::widgets::secondary(tr!("Убрать"), pal).small()).clicked() {
                        let dropped = std::mem::take(&mut self.failed);
                        self.forget(dropped);
                    }
                    if ui.add(crate::widgets::primary(tr!("Повторить"), pal).small()).lit(ui).clicked() {
                        self.retry_failed();
                    }
                });
            });
            for transfer in self.failed.iter().take(4) {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} — {}",
                            remote::file_name(transfer.path()),
                            transfer.error
                        ))
                        .small()
                        .color(pal.faint),
                    )
                    .truncate(),
                );
            }
            if self.failed.len() > 4 {
                ui.label(
                    RichText::new(tf!("и ещё {}", "and {} more", self.failed.len() - 4))
                        .small()
                        .color(pal.faint),
                );
            }
        }
    }

    pub(super) fn batch_tick(&mut self) -> Option<String> {
        let batch = self.batch.as_mut()?;
        if batch.finished.is_some() {
            return None;
        }
        let lost = !self.connected;
        if !lost && !batch.idle() {
            return None;
        }
        batch.lost = lost;
        let spent = batch.start.elapsed();
        batch.finished = Some(spent);
        let failed = batch.failed.len();
        let summary = tf!("{} из {}, {} за {}", "{} of {}, {} in {}",
            batch.done,
            batch.files,
            size_label(batch.bytes as usize),
            duration_label(spent)
        );
        let what = batch.noun();
        let (text, error) = if lost {
            (tf!("{what}: прервано, соединение закрыто ({summary})", "{what}: interrupted, connection closed ({summary})"), true)
        } else if batch.cancelled {
            (tf!("{what}: отменено ({summary})", "{what}: cancelled ({summary})"), false)
        } else if failed > 0 {
            (tf!("{what}: завершено с ошибками ({failed}), {summary}", "{what}: finished with errors ({failed}), {summary}"), true)
        } else {
            (tf!("{what}: завершено, {summary}", "{what}: finished, {summary}"), false)
        };
        let open = batch.open;
        self.push_log(text.clone(), error);
        (!open).then_some(text)
    }

    pub(super) fn cancel_batch(&mut self) {
        let Some(batch) = &mut self.batch else {
            return;
        };
        batch.cancelled = true;
        let (drop, keep): (Vec<Transfer>, Vec<Transfer>) = self
            .transfers
            .drain(..)
            .partition(|t| batch.sizes.contains_key(t.path()));
        self.transfers = keep.into();
        for t in &drop {
            batch.sizes.remove(t.path());
        }
        let roots: Vec<String> = self
            .walks
            .iter()
            .filter(|(_, w)| matches!(w, Walk::Download(_)))
            .map(|(r, _)| r.clone())
            .collect();
        for root in roots {
            self.walks.remove(&root);
        }
        batch.walking = 0;
        self.forget(drop);
        self.paused = false;
        self.pump();
    }

    pub(super) fn batch_window(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(batch) = &self.batch else {
            return;
        };
        if !batch.open {
            return;
        }
        let running = self
            .running
            .as_ref()
            .filter(|t| batch.sizes.contains_key(t.path()))
            .map(|t| remote::file_name(t.path()).to_owned());
        let current = if running.is_some() {
            self.progress.map_or(0, |(done, _)| done)
        } else {
            0
        };
        let bytes = batch.bytes + current;
        let finished = batch.finished;
        let elapsed = finished.unwrap_or_else(|| batch.start.elapsed());
        let speed = bytes as f64 / elapsed.as_secs_f64().max(0.5);
        let fraction = if batch.total > 0 {
            (bytes as f64 / batch.total as f64).min(1.0) as f32
        } else if batch.files > 0 {
            batch.done as f32 / batch.files as f32
        } else {
            0.0
        };
        let failed = batch.failed.len();
        let outcome = if batch.lost {
            tr!("Прервано: соединение закрыто")
        } else if batch.cancelled {
            tr!("Отменено")
        } else if failed > 0 {
            tr!("Завершено с ошибками")
        } else {
            tr!("Готово")
        };
        let walking = batch.walking > 0;
        let folder = batch.folder.clone();
        let target = batch.target.clone();
        let remote_dir = (batch.uploads && !batch.downloads).then(|| target.clone());
        let title = batch.noun();
        let (files, done, total) = (batch.files, batch.done, batch.total);
        let mut action = None;
        let mut open = true;
        let drag = batch.drag;
        let id = egui::ViewportId::from_hash_of(("tror_batch", self.id));
        let height = self.batch_height.max(150.0);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_inner_size([408.0, height]);
        if !self.batch_placed
            && let Some(root) = ctx.input(|i| i.viewport().outer_rect)
        {
            builder = builder.with_position(root.center() - egui::vec2(204.0, height / 2.0));
            self.batch_placed = true;
        }
        let mut used = height;
        self.batch_title = Some(title);
        let paused = self.paused;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested()) {
                open = false;
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(pal.mantle)
                        .stroke(egui::Stroke::new(1.0, pal.border))
                        .inner_margin(egui::Margin::symmetric(14, 12)),
                )
                .show(ui, |ui| {
                    let grip = ui.interact(
                        ui.max_rect(),
                        ui.scope_id().with("tror_batch_drag"),
                        Sense::click_and_drag(),
                    );
                    if grip.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    let content = ui
                        .scope(|ui| {
                            ui.set_width(380.0);
                            if crate::widgets::window_header(ui, pal, title, true) {
                                open = false;
                            }
                            ui.add_space(2.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(tf!("Куда: {target}", "To: {target}")).color(pal.subtext),
                            )
                            .truncate(),
                        );
                        let state = if let Some(spent) = finished {
                            format!("{outcome} · {}", duration_label(spent))
                        } else if walking {
                            tr!("Подсчёт объёма…").to_owned()
                        } else if paused {
                            tr!("Пауза").to_owned()
                        } else {
                            running.clone().unwrap_or_else(|| tr!("Ожидание…").to_owned())
                        };
                        let success = finished.is_some() && outcome == tr!("Готово");
                        let (state, tone) = if success && drag {
                            (tr!("✓ Готово — отпустите файлы в нужном месте").to_owned(), crate::widgets::ok_color(ui))
                        } else if success {
                            (format!("✓ {state}"), crate::widgets::ok_color(ui))
                        } else if finished.is_some() {
                            (state, pal.warn)
                        } else {
                            (state, pal.text)
                        };
                        ui.add(egui::Label::new(RichText::new(state).color(tone)).truncate());
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_height(8.0)
                                .fill(if success { tone } else { pal.accent })
                                .corner_radius(4.0),
                        );
                        let volume = if total > 0 {
                            tf!("{} из {} · {}%", "{} of {} · {}%",
                                size_label(bytes as usize),
                                size_label(total as usize),
                                (fraction * 100.0) as u32
                            )
                        } else {
                            size_label(bytes as usize)
                        };
                        ui.label(RichText::new(volume).small().color(pal.subtext));
                        let mut line = tf!("Файлы: {done} из {files}", "Files: {done} of {files}");
                        if failed > 0 {
                            line.push_str(&tf!(" · ошибок: {failed}", " · errors: {failed}"));
                        }
                        if finished.is_none() && speed > 0.0 {
                            line.push_str(&tf!(" · {}/с", " · {}/s", size_label(speed as usize)));
                            if total > bytes && !walking {
                                let left = (total - bytes) as f64 / speed;
                                line.push_str(&tf!(" · осталось ~{}", " · about {} left",
                                    duration_label(std::time::Duration::from_secs_f64(left))
                                ));
                            }
                        }
                        let color = if failed > 0 { pal.warn } else { pal.subtext };
                        ui.label(RichText::new(line).small().color(color));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if finished.is_none() {
                                let label = if paused { tr!("Продолжить") } else { tr!("Остановить") };
                                if ui.add(crate::widgets::primary(label, pal).min_size(crate::widgets::ACTION)).lit(ui).clicked() {
                                    action = Some(0);
                                }
                                if ui.add(crate::widgets::secondary(tr!("Отменить"), pal).min_size(crate::widgets::ACTION)).clicked() {
                                    action = Some(1);
                                }
                                if ui.add(crate::widgets::secondary(tr!("Скрыть"), pal).min_size(crate::widgets::ACTION)).clicked() {
                                    action = Some(2);
                                }
                            } else {
                                if failed > 0
                                    && ui.add(crate::widgets::primary(tr!("Повторить"), pal).min_size(crate::widgets::ACTION)).lit(ui).clicked()
                                {
                                    action = Some(3);
                                }
                                if (folder.is_some() || remote_dir.is_some())
                                    && ui.add(crate::widgets::secondary(tr!("Открыть папку"), pal).min_size(crate::widgets::ACTION)).clicked()
                                {
                                    action = Some(4);
                                }
                                if ui.add(crate::widgets::secondary(tr!("Закрыть"), pal).min_size(crate::widgets::ACTION)).clicked() {
                                    action = Some(5);
                                }
                            }
                        });

                        })
                        .response
                        .rect;
                    used = (content.height() + 24.0).round();
                });
        });
        if (used - self.batch_height).abs() > 0.5 {
            self.batch_height = used;
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::InnerSize(egui::vec2(408.0, used)));
        }
        if !open {
            action = Some(if finished.is_some() { 5 } else { 2 });
        }
        match action {
            Some(0) => {
                self.paused = !self.paused;
                self.pump();
            }
            Some(1) => self.cancel_batch(),
            Some(2) => {
                if let Some(batch) = &mut self.batch {
                    batch.open = false;
                }
            }
            Some(3) => {
                if let Some(batch) = &mut self.batch {
                    batch.failed.clear();
                    batch.finished = None;
                    batch.cancelled = false;
                }
                self.retry_failed();
            }
            Some(4) => {
                if let Some(folder) = &folder {
                    let _ = crate::text::preview::open_folder(folder);
                } else if let Some(dir) = remote_dir {
                    self.list(dir);
                }
            }
            Some(5) => self.batch = None,
            _ => {}
        }
        if self.batch.as_ref().is_some_and(|b| b.finished.is_none()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}
