use super::*;

impl RemotePanel {
    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) -> Vec<PanelEvent> {
        let mut out = Vec::new();
        if let Some(text) = self.batch_tick() {
            out.push(PanelEvent::Toast(text));
        }
        if let Some(text) = self.notice.take() {
            out.push(PanelEvent::Toast(text));
        }
        let dropped = self.drag_done.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(dropped) = dropped
            && self.batch.as_ref().is_some_and(|b| b.drag)
        {
            self.batch = None;
            out.push(PanelEvent::Toast(
                if dropped {
                    tr!("Файлы перенесены")
                } else {
                    tr!("Перетаскивание отменено")
                }
                .to_owned(),
            ));
        }
        if self.batch.as_ref().is_some_and(|b| b.finished.is_some() && !b.open) {
            self.batch = None;
        }
        self.batch_window(ctx, pal);
        self.sync_dialog(ctx, pal);
        self.host_key_dialog(ctx, pal);
        self.prompt_dialog(ctx, pal);
        self.attrs_dialog(ctx, pal);
        self.editor_dialog(ctx, pal);
        out
    }

    pub(super) fn host_key_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((fingerprint, changed)) = self.host_key.clone() else {
            return;
        };
        let mut answer = None;
        dialog(("tror_host_key", self.id), pal).show(ctx, |ui| {
            ui.set_width(460.0);
            if changed {
                ui.label(
                    RichText::new(tr!("Ключ сервера изменился"))
                        .size(crate::widgets::TITLE)
                        .color(pal.warn),
                );
                ui.label(
                    tr!("Сохранённый ключ этого сервера отличается от присланного. Так бывает после \
                     переустановки сервера, но так же выглядит перехват соединения."),
                );
            } else {
                crate::widgets::title(ui, pal, tr!("Новый сервер"));
                ui.label(
                    tr!("К этому серверу вы подключаетесь впервые. Отпечаток его ключа (SHA-256):"),
                );
            }
            ui.add_space(4.0);
            ui.label(RichText::new(&fingerprint).monospace().size(12.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let accept = if changed {
                    tr!("Заменить ключ и подключиться")
                } else {
                    tr!("Доверять и подключиться")
                };
                if dialog_button(ui, pal, accept, !changed) {
                    answer = Some(true);
                }
                if dialog_button(ui, pal, tr!("Отмена"), changed) {
                    answer = Some(false);
                }
            });
        });
        if let Some(trust) = answer {
            self.host_key = None;
            if trust {
                if let Some(conn) = &self.conn {
                    let _ = conn.tx.send(Cmd::TrustHost(true));
                    self.busy = Some(tr!("Вход…").into());
                }
            } else {
                self.disconnect();
            }
        }
    }

    pub(super) fn attrs_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(state) = &mut self.attrs else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let sftp = self
            .store
            .profiles
            .get(self.selected)
            .is_some_and(|p| p.protocol == Protocol::Sftp);
        let modal = dialog(("tror_remote_attrs", self.id), pal).show(ctx, |ui| {
            ui.set_width(400.0);
            let title = if state.items.len() == 1 {
                tf!("Свойства «{}»", "Properties of \"{}\"", remote::file_name(&state.items[0].0))
            } else {
                tf!("Свойства выбранных объектов ({})", "Properties of the selected items ({})", state.items.len())
            };
            crate::widgets::title(ui, pal, title);
            ui.add_space(6.0);
            if !state.loaded {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tr!("чтение с сервера…")).color(pal.subtext));
                });
            }
            if let Some(info) = &state.info
                && state.items.len() == 1
            {
                egui::Grid::new("tror_attrs_info")
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new(tr!("Путь")).color(pal.subtext));
                        ui.label(&state.items[0].0);
                        ui.end_row();
                        if let Some(size) = info.size.filter(|_| !info.is_dir) {
                            ui.label(RichText::new(tr!("Размер")).color(pal.subtext));
                            ui.label(if size < 1024 {
                                size_label(size as usize)
                            } else {
                                tf!("{} ({size} Б)", "{} ({size} B)", size_label(size as usize))
                            });
                            ui.end_row();
                        }
                        if info.uid.is_some() || info.gid.is_some() {
                            ui.label(RichText::new(tr!("Владелец")).color(pal.subtext));
                            ui.label(format!(
                                "uid {}, gid {}",
                                info.uid.map_or("?".into(), |v| v.to_string()),
                                info.gid.map_or("?".into(), |v| v.to_string())
                            ));
                            ui.end_row();
                        }
                        if let Some(atime) = info.atime {
                            ui.label(RichText::new(tr!("Открыт")).color(pal.subtext));
                            ui.label(date_label(atime));
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
            }
            ui.label(RichText::new(tr!("Права доступа")).family(crate::theme::medium()));
            ui.add_space(2.0);
            widgets::perm_grid(ui, pal, &mut state.mode, &mut state.octal);
            let has_dirs = state.items.iter().any(|(_, dir)| *dir);
            if has_dirs {
                ui.add_space(4.0);
                ui.checkbox(&mut state.recursive, tr!("Применить ко всему содержимому папок"));
                if state.recursive {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        ui.checkbox(&mut state.dirs, tr!("к папкам"));
                        ui.checkbox(&mut state.files, tr!("к файлам"));
                    });
                    ui.label(
                        RichText::new(tr!("Обычно папкам ставят 755, файлам 644 — примените дважды с разными галочками."))
                            .small()
                            .color(pal.subtext),
                    );
                }
            }
            if state.items.len() == 1 {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(tr!("Дата изменения"));
                    ui.add(
                        TextEdit::singleline(&mut state.mtime)
                            .hint_text(tr!("ДД.ММ.ГГГГ ЧЧ:ММ"))
                            .desired_width(150.0),
                    );
                    if state.mtime != state.mtime_was && widgets::parse_date(&state.mtime).is_none()
                    {
                        ui.label(RichText::new(tr!("не дата")).color(pal.warn).small());
                    }
                });
                if !sftp {
                    ui.label(
                        RichText::new(tr!("По FTP дата меняется командой MFMT — её поддерживают не все серверы."))
                            .small()
                            .color(pal.subtext),
                    );
                }
            }
            if !sftp {
                ui.label(
                    RichText::new(tr!("По FTP права меняются командой SITE CHMOD; на Windows-серверах она обычно не работает."))
                        .small()
                        .color(pal.subtext),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Применить"), true) {
                    apply = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if apply && let Some(state) = self.attrs.take() {
            let recursive = state.recursive;
            for (path, dir) in &state.items {
                self.send(
                    Cmd::Chmod {
                        path: path.clone(),
                        mode: state.mode,
                        recursive: recursive && *dir,
                        dirs: state.dirs,
                        files: state.files,
                    },
                    tr!("Смена прав…"),
                );
            }
            if state.items.len() == 1
                && state.mtime != state.mtime_was
                && let Some(mtime) = widgets::parse_date(&state.mtime)
            {
                self.send(
                    Cmd::Touch {
                        path: state.items[0].0.clone(),
                        mtime,
                    },
                    tr!("Смена даты…"),
                );
            }
        } else if cancel || modal.should_close() {
            self.attrs = None;
        }
    }

    pub(super) fn prompt_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        let mut confirmed = false;
        let mut cancelled = false;
        let modal = dialog(("tror_remote_prompt", self.id), pal).show(ctx, |ui| {
            ui.set_width(340.0);
            let (title, input, action) = match prompt {
                Prompt::NewFolder(dir, name) => (tf!("Новая папка в {dir}", "New folder in {dir}"), Some(name), tr!("Создать")),
                Prompt::NewFile(dir, name) => (tf!("Новый файл в {dir}", "New file in {dir}"), Some(name), tr!("Создать")),
                Prompt::Rename { name, .. } => (tr!("Переименовать").to_owned(), Some(name), tr!("Переименовать")),
                Prompt::Extract(items, dest) => (
                    if items.len() == 1 {
                        tf!("Распаковать «{}» в папку", "Extract \"{}\" to folder", remote::file_name(&items[0]))
                    } else {
                        tf!("Распаковать архивы ({}) в папку", "Extract archives ({}) to folder", items.len())
                    },
                    Some(dest),
                    tr!("Распаковать"),
                ),
                Prompt::Move(moves) => {
                    let dir = moves
                        .first()
                        .map(|(_, to)| remote::parent(to))
                        .unwrap_or_default();
                    if moves.len() == 1 {
                        ui.label(tf!("Переместить «{}» в папку «{}»?", "Move \"{}\" to folder \"{}\"?",
                            remote::file_name(&moves[0].0),
                            remote::file_name(&dir)
                        ));
                    } else {
                        ui.label(tf!("Переместить выбранные объекты ({}) в папку «{}»?", "Move the selected items ({}) to folder \"{}\"?",
                            moves.len(),
                            remote::file_name(&dir)
                        ));
                    }
                    ui.label(RichText::new(dir.clone()).small().color(pal.subtext));
                    (String::new(), None, tr!("Переместить"))
                }
                Prompt::Delete(items) => {
                    let folders = items.iter().filter(|(_, dir)| *dir).count();
                    if items.len() == 1 {
                        ui.label(tf!("Удалить {} «{}» с сервера?", "Delete {} \"{}\" from the server?",
                            if folders == 1 {
                                tr!("папку")
                            } else {
                                tr!("файл")
                            },
                            remote::file_name(&items[0].0)
                        ));
                    } else {
                        ui.label(tf!("Удалить с сервера выбранные объекты ({})?", "Delete the selected items from the server ({})?",
                            items.len()
                        ));
                    }
                    if folders > 0 {
                        ui.label(
                            RichText::new(
                                tr!("Папки удаляются вместе со всем содержимым. Отменить это нельзя."),
                            )
                            .color(pal.warn),
                        );
                    }
                    (String::new(), None, tr!("Удалить"))
                }
            };
            if let Some(input) = input {
                ui.add(egui::Label::new(title).truncate());
                ui.add_space(4.0);
                let field = ui.add(TextEdit::singleline(input).desired_width(f32::INFINITY));
                field.request_focus();
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    confirmed = true;
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = if action == tr!("Удалить") {
                    crate::widgets::danger_button(ui, pal, action)
                } else {
                    dialog_button(ui, pal, action, true)
                };
                if ok {
                    confirmed = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancelled = true;
                }
            });
        });
        if confirmed {
            let valid = |name: &str| {
                let name = name.trim();
                (!name.is_empty() && !name.contains('/')).then(|| name.to_owned())
            };
            match self.prompt.take() {
                Some(Prompt::Extract(items, dest)) => {
                    let dest = dest.trim().trim_end_matches('/');
                    if !dest.is_empty() {
                        let dest = if dest.starts_with('/') {
                            dest.to_owned()
                        } else {
                            remote::join(&self.cwd, dest)
                        };
                        for archive in items {
                            let busy = tf!("Распаковка {}…", "Extracting {}…", remote::file_name(&archive));
                            self.send(
                                Cmd::Extract {
                                    path: archive,
                                    dest: Some(dest.clone()),
                                },
                                busy,
                            );
                        }
                    }
                }
                Some(Prompt::NewFolder(dir, name)) => {
                    if let Some(name) = valid(&name) {
                        if self.tree {
                            self.expanded.insert(dir.clone());
                        }
                        self.send(Cmd::Mkdir(remote::join(&dir, &name)), tr!("Создание папки…"));
                    }
                }
                Some(Prompt::NewFile(dir, name)) => {
                    if let Some(name) = valid(&name) {
                        if self.tree {
                            self.expanded.insert(dir.clone());
                        }
                        self.send(
                            Cmd::Upload {
                                path: remote::join(&dir, &name),
                                data: Vec::new(),
                                doc: None,
                                expect: None,
                            },
                            tr!("Создание файла…"),
                        );
                    }
                }
                Some(Prompt::Rename { from, name }) => {
                    if let Some(name) = valid(&name) {
                        let to = remote::join(&remote::parent(&from), &name);
                        self.send(Cmd::Rename(from, to), tr!("Переименование…"));
                    }
                }
                Some(Prompt::Move(moves)) => {
                    for (from, to) in moves {
                        self.send(Cmd::Rename(from, to), tr!("Перемещение…"));
                    }
                }
                Some(Prompt::Delete(items)) => {
                    for (path, dir) in items {
                        self.marked.remove(&path);
                        self.expanded.remove(&path);
                        self.send(Cmd::Delete { path, dir }, tr!("Удаление…"));
                    }
                }
                None => {}
            }
        } else if cancelled || modal.should_close() {
            self.prompt = None;
        }
    }

    pub(super) fn editor_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let mut test = false;
        let location = self
            .store
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let modal = dialog(("tror_profiles", self.id), pal).show(ctx, |ui| {
            ui.set_width(640.0);
            crate::widgets::title(ui, pal, tr!("Профили подключений"));
            ui.add_space(8.0);
            let limit = (ui.ctx().content_rect().height() - 200.0).max(200.0);
            ScrollArea::vertical()
                .id_salt("tror_profile_scroll")
                .max_height(limit)
                .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(180.0);
                        ScrollArea::vertical().max_height(270.0).show(ui, |ui| {
                            for (i, profile) in editor.drafts.iter().enumerate() {
                                let name = if profile.name.trim().is_empty() {
                                    tr!("без имени")
                                } else {
                                    profile.name.as_str()
                                };
                                if ui.selectable_label(i == editor.index, name).clicked() {
                                    editor.index = i;
                                    editor.result = None;
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.add(Button::new(tr!("Добавить"))).clicked() {
                                editor.drafts.push(Profile::new(tf!("Профиль {}", "Profile {}",
                                    editor.drafts.len() + 1
                                )));
                                editor.index = editor.drafts.len() - 1;
                                editor.result = None;
                            }
                            if ui
                                .add_enabled(
                                    !editor.drafts.is_empty(),
                                    Button::new(tr!("Удалить")),
                                )
                                .clicked()
                            {
                                editor.drafts.remove(editor.index);
                                editor.index = editor.index.saturating_sub(1);
                            }
                        });
                        ui.menu_button(tr!("Импорт…"), |ui| {
                            let mut found = None;
                            if let Some(path) = crate::remote::import::filezilla_file() {
                                if ui.button(tr!("Из FileZilla")).clicked() {
                                    found = Some(std::fs::read_to_string(path).map(|t| crate::remote::import::filezilla(&t)));
                                }
                            } else if ui.button(tr!("Из файла FileZilla (sitemanager.xml)…")).clicked()
                                && let Some(path) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file()
                            {
                                found = Some(std::fs::read_to_string(path).map(|t| crate::remote::import::filezilla(&t)));
                            }
                            if cfg!(windows) && ui.button(tr!("Из WinSCP")).clicked() {
                                found = Some(Ok(crate::remote::import::winscp(&crate::remote::import::winscp_sessions())));
                            }
                            if ui.button(tr!("Из файла WinSCP.ini…")).clicked()
                                && let Some(path) = rfd::FileDialog::new().add_filter("INI", &["ini"]).pick_file()
                            {
                                found = Some(std::fs::read_to_string(path).map(|t| crate::remote::import::winscp(&crate::remote::import::winscp_ini(&t))));
                            }
                            if let Some(found) = found {
                                ui.close();
                                editor.note = Some(match found {
                                    Ok(list) => {
                                        let added = merge_profiles(&mut editor.drafts, list);
                                        if added > 0 {
                                            editor.index = editor.drafts.len() - 1;
                                        }
                                        if added == 0 {
                                            Err(tr!("Новых подключений не найдено").to_owned())
                                        } else {
                                            Ok(tf!("Добавлено подключений: {added}. Проверьте их и нажмите «Сохранить».", "Connections added: {added}. Check them and press \"Save\"."))
                                        }
                                    }
                                    Err(e) => Err(e.to_string()),
                                });
                            }
                        });
                        if let Some(note) = &editor.note {
                            let (text, color) = match note {
                                Ok(text) => (text.as_str(), crate::widgets::ok_color(ui)),
                                Err(text) => (text.as_str(), pal.warn),
                            };
                            ui.add(egui::Label::new(RichText::new(text).small().color(color)).wrap());
                        }
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        let Some(p) = editor.drafts.get_mut(editor.index) else {
                            ui.label(RichText::new(tr!("Добавьте профиль.")).color(pal.subtext));
                            return;
                        };
                        egui::Grid::new("tror_profile_form")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                let wide = 320.0;
                                ui.label(tr!("Название"));
                                ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                                ui.end_row();
                                ui.label(tr!("Протокол"));
                                ui.horizontal_wrapped(|ui| {
                                    ui.set_max_width(wide);
                                    for protocol in Protocol::ALL {
                                        if ui
                                            .selectable_label(p.protocol == protocol, protocol.label())
                                            .clicked()
                                            && p.protocol != protocol
                                        {
                                            if p.port == p.protocol.default_port() {
                                                p.port = protocol.default_port();
                                            }
                                            p.protocol = protocol;
                                        }
                                    }
                                });
                                ui.end_row();
                                ui.label(tr!("Сервер"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        TextEdit::singleline(&mut p.host)
                                            .hint_text("example.org")
                                            .desired_width(wide - 70.0),
                                    );
                                    widgets::port_field(ui, &mut p.port, 60.0)
                                        .on_hover_text(tr!("Порт"));
                                });
                                ui.end_row();
                                ui.label(tr!("Пользователь"));
                                ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                                ui.end_row();
                                ui.label(tr!("Пароль"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        TextEdit::singleline(&mut p.password)
                                            .password(!editor.reveal)
                                            .desired_width(wide - 88.0),
                                    );
                                    let label = if editor.reveal { tr!("Скрыть") } else { tr!("Показать") };
                                    if ui
                                        .add_sized([80.0, 22.0], Button::new(label))
                                        .clicked()
                                    {
                                        editor.reveal = !editor.reveal;
                                    }
                                });
                                ui.end_row();
                                if p.protocol == Protocol::Sftp {
                                    ui.label(tr!("Файл ключа"));
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            TextEdit::singleline(&mut p.key_file)
                                                .hint_text(tr!("необязательно"))
                                                .desired_width(wide - 78.0),
                                        );
                                        if ui.button(tr!("Обзор…")).clicked()
                                            && let Some(file) = rfd::FileDialog::new().pick_file()
                                        {
                                            p.key_file = file.display().to_string();
                                        }
                                    });
                                    ui.end_row();
                                    if !p.key_file.trim().is_empty() {
                                        ui.label("");
                                        match remote::key_kind(&p.key_file) {
                                            Ok(kind) => ui.label(
                                                RichText::new(tf!("ключ: {kind}", "key: {kind}"))
                                                    .small()
                                                    .color(pal.subtext),
                                            ),
                                            Err(e) => {
                                                ui.label(RichText::new(e).small().color(pal.warn))
                                            }
                                        };
                                        ui.end_row();
                                        ui.label(tr!("Парольная фраза"));
                                        ui.add(
                                            TextEdit::singleline(&mut p.passphrase)
                                                .password(!editor.reveal)
                                                .hint_text(tr!("если ключ защищён"))
                                                .desired_width(wide),
                                        );
                                        ui.end_row();
                                    }
                                }
                                if p.protocol.tls() {
                                    ui.label(tr!("Сертификат"));
                                    ui.checkbox(&mut p.insecure_tls, tr!("не проверять (самоподписанный)"));
                                    ui.end_row();
                                }
                                ui.label("");
                                ui.vertical(|ui| {
                                    ui.set_max_width(wide);
                                    if editor.probe.is_some() {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(RichText::new(tr!("проверка…")).color(pal.subtext));
                                        });
                                    } else if ui
                                        .add(Button::new(tr!("Проверить подключение")))
                                        .clicked()
                                    {
                                        test = true;
                                    }
                                    if let Some(result) = &editor.result {
                                        let (text, color) = match result {
                                            Ok(text) => (tf!("Работает: {text}", "Works: {text}"), crate::widgets::ok_color(ui)),
                                            Err(e) => (tf!("Ошибка: {e}", "Error: {e}"), pal.warn),
                                        };
                                        ui.label(RichText::new(text).small().color(color));
                                    }
                                });
                                ui.end_row();
                                ui.label(tr!("Начальная папка"));
                                ui.add(
                                    TextEdit::singleline(&mut p.directory)
                                        .hint_text(tr!("домашняя папка"))
                                        .desired_width(wide),
                                );
                                ui.end_row();
                                ui.label(tr!("Адрес сайта"));
                                ui.add(
                                    TextEdit::singleline(&mut p.url)
                                        .hint_text(tr!("https://example.ru — для предпросмотра (F12)"))
                                        .desired_width(wide),
                                );
                                ui.end_row();
                                if p.protocol == Protocol::Sftp {
                                    ui.label("");
                                    ui.label(RichText::new(tr!("Консоль по этому профилю")).color(pal.subtext).small());
                                    ui.end_row();
                                    crate::widgets::appearance_rows(
                                        ui,
                                        pal,
                                        &mut p.term_theme,
                                        &mut p.term_font,
                                        &mut p.term_font_size,
                                        wide,
                                    );
                                }
                            });
                        ui.add_space(4.0);
                        let (text, color) = match p.protocol {
                            Protocol::Sftp => (
                                tr!("Вход по паролю, по ключу (RSA, ECDSA, Ed25519, PuTTY .ppk) или через ssh-agent, если не заданы ни пароль, ни ключ. Если у ключа нет отдельной парольной фразы, используется поле «Пароль»."),
                                pal.subtext,
                            ),
                            Protocol::Ftp => (tr!("FTP передаёт пароль и файлы без шифрования."), pal.warn),
                            _ if p.insecure_tls => (
                                tr!("Соединение шифруется, но подлинность сервера не проверяется — \
                                 включайте только для своего сервера с самоподписанным сертификатом."),
                                pal.warn,
                            ),
                            Protocol::Ftps => (
                                tr!("FTPS с явным TLS (AUTH TLS, обычно порт 21): шифруются и команды, и данные."),
                                pal.subtext,
                            ),
                            Protocol::FtpsImplicit => (
                                tr!("Неявный FTPS (обычно порт 990): TLS включается сразу при соединении. Старый способ — выбирайте его, только если хостинг не поддерживает явный FTPS."),
                                pal.subtext,
                            ),
                        };
                        ui.label(RichText::new(text).color(color).small());
                    });
                });
                });
            ui.add_space(10.0);
            ui.label(
                RichText::new(tf!("Профили хранятся в {location}. Пароли в файле скрыты от случайного взгляда, но не зашифрованы.", "Profiles are stored in {location}. Passwords in the file are hidden from a casual glance, but not encrypted."
                ))
                .color(pal.subtext)
                .small(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Сохранить"), true) {
                    save = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if test
            && let Some(editor) = &mut self.editor
            && let Some(profile) = editor.drafts.get(editor.index).cloned()
        {
            let key = format!("{}:{}", profile.host.trim(), profile.port);
            let known = self.store.known_hosts.get(&key).cloned();
            editor.result = None;
            editor.probe = Some(remote::probe(move || remote::test(&profile, known)));
        }
        if save
            && let Some(editor) = &mut self.editor
            && editor
                .drafts
                .get(editor.index)
                .is_some_and(|p| p.host.trim().is_empty() && !self.store.profiles.contains(p))
        {
            editor.result = Some(Err(tr!("Укажите сервер").into()));
            save = false;
        }
        if save {
            if let Some(mut editor) = self.editor.take() {
                let chosen = editor.drafts.get(editor.index).cloned();
                let known = &self.store.profiles;
                editor
                    .drafts
                    .retain(|p| !p.host.trim().is_empty() || known.contains(p));
                self.selected = chosen
                    .and_then(|c| editor.drafts.iter().position(|p| *p == c))
                    .unwrap_or(0);
                self.store.profiles = editor.drafts;
                if let Err(e) = self.store.save() {
                    self.busy = Some(tf!("Профили не сохранены: {e}", "Profiles not saved: {e}"));
                }
            }
        } else if cancel || modal.should_close() {
            self.editor = None;
        }
    }
}
