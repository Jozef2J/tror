use super::*;
use crate::widgets::Lit;

impl TrorApp {
    pub(super) fn update_ui(&mut self, ctx: &egui::Context, pal: &Palette, now: f64) {
        use crate::update::{Event, Kind};
        let events: Vec<Event> = self
            .update
            .rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        if self.update.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        for event in events {
            self.update.rx = None;
            self.update.busy = false;
            match event {
                Event::Found(release) => self.update.found = Some(release),
                Event::Current => {
                    if std::mem::take(&mut self.update.manual) {
                        self.notify(tr!("Установлена последняя версия"), now);
                    }
                }
                Event::Failed(e) => {
                    if std::mem::take(&mut self.update.manual) || self.update.found.is_some() {
                        self.notify(tf!("Обновление не удалось: {e}", "Update failed: {e}"), now);
                    }
                }
                Event::Ready(path, Kind::Installer) => {
                    self.update.setup = Some(path);
                    self.update.found = None;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Event::Ready(path, _) => {
                    let version = self
                        .update
                        .found
                        .as_ref()
                        .map(|r| r.version.clone())
                        .unwrap_or_default();
                    match crate::update::apply(&path, &version) {
                        Ok(()) => self.update.swapped = true,
                        Err(e) => self
                            .notify(tf!("Обновление не удалось: {e}", "Update failed: {e}"), now),
                    }
                }
            }
        }
        let Some(release) = self.update.found.clone() else {
            return;
        };
        let kind = crate::update::kind();
        let mut close = false;
        let notes = crate::update::notes(&release.notes);
        egui::Window::new(tr!("Доступно обновление"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(crate::widgets::window_frame(pal))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -40.0))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(tr!("Доступно обновление"))
                                .small()
                                .color(pal.accent),
                        );
                        ui.label(
                            RichText::new(format!("TrorPad {}", release.version))
                                .size(17.0)
                                .color(pal.text),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.label(
                            RichText::new(tf!(
                                "у вас {}",
                                "you have {}",
                                env!("CARGO_PKG_VERSION")
                            ))
                            .small()
                            .color(pal.faint),
                        );
                    });
                });
                if !notes.is_empty() {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 5.0;
                            for (i, (bullet, line)) in notes.iter().enumerate() {
                                if *bullet {
                                    ui.horizontal_top(|ui| {
                                        ui.label(RichText::new("•").color(pal.accent));
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(line).color(pal.subtext),
                                            )
                                            .wrap(),
                                        );
                                    });
                                } else if i == 0 {
                                    ui.add(
                                        egui::Label::new(RichText::new(line).color(pal.text))
                                            .wrap(),
                                    );
                                } else {
                                    ui.add(
                                        egui::Label::new(RichText::new(line).color(pal.subtext))
                                            .wrap(),
                                    );
                                }
                            }
                        });
                    ui.separator();
                }
                if self.update.swapped {
                    ui.horizontal(|ui| {
                        if ui
                            .add(crate::widgets::primary(tr!("Перезапустить сейчас"), pal))
                            .lit(ui)
                            .clicked()
                        {
                            match crate::update::restart() {
                                Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
                                Err(e) => self.update.error = e,
                            }
                        }
                        close = ui
                            .add(crate::widgets::secondary(tr!("Позже"), pal))
                            .clicked();
                    });
                } else if self.update.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(tr!("Скачиваю и проверяю файл…")).color(pal.subtext),
                        );
                    });
                } else {
                    let can_install =
                        kind != Kind::Page && release.file.is_some() && release.sums.is_some();
                    ui.horizontal(|ui| {
                        if can_install
                            && ui
                                .add(crate::widgets::primary(tr!("Скачать и установить"), pal))
                                .lit(ui)
                                .clicked()
                        {
                            self.update.busy = true;
                            self.update.rx = Some(crate::update::download(release.clone()));
                        }
                        if ui
                            .add(crate::widgets::secondary(
                                tr!("Открыть страницу выпуска"),
                                pal,
                            ))
                            .clicked()
                        {
                            let _ = preview::open_url(crate::update::PAGE);
                        }
                        close = ui
                            .add(crate::widgets::secondary(tr!("Позже"), pal))
                            .clicked();
                    });
                }
                if !self.update.busy && !self.update.swapped && kind == Kind::Page {
                    ui.label(
                        RichText::new(tr!(
                            "Установить отсюда нельзя: папка программы закрыта для записи. \
                             Скачайте новую версию со страницы выпуска."
                        ))
                        .small()
                        .color(pal.subtext),
                    );
                }
                if !self.update.error.is_empty() {
                    ui.label(RichText::new(&self.update.error).color(pal.warn));
                }
            });
        if close {
            self.update.found = None;
        }
    }

    pub(super) fn rename_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((id, name)) = &mut self.rename else {
            return;
        };
        let id = *id;
        let mut done = false;
        let mut go = false;
        let modal = dialog("tror_rename_tab", pal).show(ctx, |ui| {
            ui.set_width(340.0);
            crate::widgets::title(ui, pal, tr!("Новое имя файла"));
            ui.add_space(4.0);
            let field = ui.add(TextEdit::singleline(name).desired_width(f32::INFINITY));
            if !field.has_focus() && !ui.input(|i| i.pointer.any_down()) {
                field.request_focus();
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Переименовать"), true) {
                    go = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    done = true;
                }
            });
        });
        let now = ctx.input(|i| i.time);
        if go {
            let name = name.trim().to_owned();
            done = true;
            if let Some(index) = self.docs.iter().position(|d| d.id == id)
                && !name.is_empty()
                && !name.contains(['/', '\\'])
                && name != self.docs[index].title()
            {
                let old_key = self.docs[index].location();
                let result = match (
                    self.docs[index].remote.clone(),
                    self.docs[index].path.clone(),
                ) {
                    (Some((profile, from)), _) => {
                        let to = crate::remote::join(&crate::remote::parent(&from), &name);
                        let panel = self.remotes.iter().position(|r| r.serves(&profile));
                        match panel {
                            Some(p) => {
                                self.remotes[p].rename_file(&profile, &from, &to).map(|()| {
                                    self.docs[index].remote = Some((profile, to));
                                })
                            }
                            None => Err(tf!(
                                "сначала подключитесь к «{profile}»",
                                "connect to \"{profile}\" first"
                            )),
                        }
                    }
                    (None, Some(path)) => {
                        let to = path.with_file_name(&name);
                        if to.exists() {
                            Err(tr!("файл с таким именем уже есть").into())
                        } else {
                            std::fs::rename(&path, &to)
                                .map_err(|e| e.to_string())
                                .map(|()| {
                                    let first = self.docs[index].first_line();
                                    self.docs[index].syntax =
                                        self.syntaxes.detect(Some(&to), &first);
                                    self.docs[index].path = Some(to.clone());
                                    self.remember(&to);
                                })
                        }
                    }
                    (None, None) => {
                        let first = self.docs[index].first_line();
                        self.docs[index].syntax =
                            self.syntaxes.detect(Some(Path::new(&name)), &first);
                        self.docs[index].name = Some(name.clone());
                        Ok(())
                    }
                };
                match result {
                    Ok(()) => {
                        if let (Some(old), Some(new)) = (old_key, self.docs[index].location()) {
                            self.settings.move_tab_color(&old, new);
                        }
                        self.notify(tf!("Переименовано в {name}", "Renamed to {name}"), now);
                    }
                    Err(e) => self.notify(
                        tf!("Не удалось переименовать: {e}", "Could not rename: {e}"),
                        now,
                    ),
                }
            }
        }
        if done || modal.should_close() {
            self.rename = None;
            self.focus_editor = true;
        }
    }

    pub(super) fn macro_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(state) = &mut self.macro_dialog else {
            return;
        };
        let mut done = false;
        let mut confirm = false;
        let modal = dialog("tror_macro", pal).show(ctx, |ui| {
            ui.set_width(300.0);
            match state {
                MacroDialog::Save(name) => {
                    window_header(ui, pal, tr!("Сохранить макрос"), false);
                    let field = ui.add(
                        TextEdit::singleline(name)
                            .hint_text(tr!("название"))
                            .desired_width(f32::INFINITY),
                    );
                    field.request_focus();
                }
                MacroDialog::Run(times, until_end) => {
                    window_header(ui, pal, tr!("Выполнить макрос"), false);
                    ui.add_enabled_ui(!*until_end, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr!("Сколько раз"));
                            let mut value = *times as f32;
                            if crate::widgets::stepper(
                                ui,
                                pal,
                                "macro_times",
                                &mut value,
                                (1.0, 100_000.0),
                                1.0,
                                "",
                            ) {
                                *times = value as u32;
                            }
                        });
                    });
                    ui.checkbox(until_end, tr!("До конца файла"));
                }
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                confirm = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = match state {
                    MacroDialog::Save(_) => tr!("Сохранить"),
                    MacroDialog::Run(..) => tr!("Выполнить"),
                };
                if dialog_button(ui, pal, label, true) {
                    confirm = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    done = true;
                }
            });
        });
        if confirm {
            match state {
                MacroDialog::Save(name) => {
                    let name = name.trim().to_owned();
                    if !name.is_empty() {
                        let events = self.macros.events.clone();
                        self.settings.save_macro(name, events);
                        done = true;
                    }
                }
                MacroDialog::Run(times, until_end) => {
                    self.macros.until_end = *until_end;
                    self.macros.play = if *until_end { 1_000_000 } else { *times };
                    done = true;
                }
            }
        }
        if done || modal.should_close() {
            self.macro_dialog = None;
            self.focus_editor = true;
        }
    }

    pub(super) fn goto_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(input) = &mut self.goto else {
            return;
        };
        let mut done = false;
        let mut go = false;
        let modal = dialog("tror_goto", pal).show(ctx, |ui| {
            ui.set_width(260.0);
            crate::widgets::title(ui, pal, tr!("Перейти к строке"));
            ui.add_space(4.0);
            let field = ui.add(
                TextEdit::singleline(input)
                    .hint_text(tr!("номер строки"))
                    .desired_width(f32::INFINITY),
            );
            field.request_focus();
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Перейти"), true) {
                    go = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    done = true;
                }
            });
        });
        if go {
            if let (Ok(line), Some(doc)) = (
                input.trim().parse::<usize>(),
                self.docs.get_mut(self.active),
            ) {
                doc.goto_line(line);
            }
            done = true;
        }
        if done || modal.should_close() {
            self.goto = None;
            self.focus_editor = true;
        }
    }

    pub(super) fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let pal = self.palette();
        let message = match confirm {
            Confirm::Tab(id) => match self.docs.iter().find(|d| d.id == id) {
                Some(doc) => tf!(
                    "Сохранить изменения в «{}»?",
                    "Save changes to \"{}\"?",
                    doc.title()
                ),
                None => {
                    self.confirm = None;
                    return;
                }
            },
            Confirm::Quit => tf!(
                "Несохранённых файлов: {}. Сохранить перед выходом?",
                "Unsaved files: {}. Save before exiting?",
                self.docs.iter().filter(|d| d.dirty()).count()
            ),
            Confirm::Missing(id) => match self.docs.iter().find(|d| d.id == id) {
                Some(doc) => tf!(
                    "Файл «{}» больше не существует {}. Оставить вкладку открытой?",
                    "File \"{}\" no longer exists {}. Keep the tab open?",
                    doc.title(),
                    if doc.remote.is_some() {
                        tr!("на сервере")
                    } else {
                        tr!("на диске")
                    }
                ),
                None => {
                    self.confirm = None;
                    return;
                }
            },
        };
        let missing = matches!(confirm, Confirm::Missing(_));
        let mut choice = None;
        let modal = dialog("tror_confirm", &pal).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(message);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if missing {
                    if dialog_button(ui, &pal, tr!("Оставить"), true) {
                        choice = Some(2);
                    }
                    if dialog_button(ui, &pal, tr!("Закрыть вкладку"), false) {
                        choice = Some(1);
                    }
                    return;
                }
                if dialog_button(ui, &pal, tr!("Сохранить"), true) {
                    choice = Some(0);
                }
                if dialog_button(ui, &pal, tr!("Не сохранять"), false) {
                    choice = Some(1);
                }
                if dialog_button(ui, &pal, tr!("Отмена"), false) {
                    choice = Some(2);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(2);
        }
        let Some(choice) = choice else {
            return;
        };
        self.confirm = None;
        match (confirm, choice) {
            (Confirm::Tab(id), 0 | 1) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id)
                    && (choice == 1 || self.save(i, now))
                {
                    self.remove_doc(i);
                }
            }
            (Confirm::Missing(id), _) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    if choice == 1 {
                        self.remove_doc(i);
                    } else {
                        self.docs[i].missing_ack = true;
                    }
                }
            }
            (Confirm::Quit, 0 | 1) if choice == 1 || self.save_all(now) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            _ => {}
        }
    }

    pub(super) fn conflict_modal(&mut self, ctx: &egui::Context) {
        let Some(conflict) = &self.conflict else {
            return;
        };
        let Some(index) = self.docs.iter().position(|d| d.id == conflict.doc) else {
            self.conflict = None;
            return;
        };
        let now = ctx.input(|i| i.time);
        let pal = self.palette();
        let name = crate::remote::file_name(&conflict.path).to_owned();
        let mut choice = None;
        let modal = dialog("tror_conflict", &pal).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(tf!("Файл «{name}» изменился на сервере после того, как вы его открыли. Ваши правки пока не записаны.", "File \"{name}\" was changed on the server after you opened it. Your edits are not written yet."
            ));
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr!("При перезаписи версия с сервера сохранится в резервные копии."))
                    .small()
                    .color(pal.subtext),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, &pal, tr!("Сравнить"), true) {
                    choice = Some(0);
                }
                if dialog_button(ui, &pal, tr!("Перезаписать"), false) {
                    choice = Some(1);
                }
                if dialog_button(ui, &pal, tr!("Отмена"), false) {
                    choice = Some(2);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(2);
        }
        let Some(choice) = choice else {
            return;
        };
        let Some(conflict) = self.conflict.take() else {
            return;
        };
        match choice {
            0 => self.show_server_diff(
                index,
                &conflict.profile,
                &conflict.path,
                conflict.server,
                now,
            ),
            1 => {
                self.docs[index].server_hash = None;
                self.save(index, now);
            }
            _ => self.notify(tr!("Сохранение отменено"), now),
        }
    }

    pub(super) fn show_server_diff(
        &mut self,
        index: usize,
        profile: &str,
        path: &str,
        server: Vec<u8>,
        now: f64,
    ) {
        let name = crate::remote::file_name(path).to_owned();
        match Document::open_remote(self.next_id, profile, path, server) {
            Ok(mut server) => {
                self.next_id += 1;
                server.remote = None;
                server.name = Some(tf!("{name} (сервер)", "{name} (server)"));
                server.syntax = self.docs[index].syntax.clone();
                let local = self.docs[index].id;
                let id = server.id;
                self.diff.left = Some(id);
                self.diff.right = Some(local);
                self.docs.push(server);
                self.settings.show(&mut self.dock, Pane::Doc(id));
                self.settings.show(&mut self.dock, Pane::Diff);
            }
            Err(e) => self.notify(
                tf!(
                    "Не удалось открыть версию с сервера: {e}",
                    "Could not open the server version: {e}"
                ),
                now,
            ),
        }
    }
}
