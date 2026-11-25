use super::*;

impl DbPanel {
    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette, store: &mut Store) {
        self.confirm_dialog(ctx, pal);
        self.import_dialog(ctx, pal);
        self.structure_dialogs(ctx, pal);
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut done = false;
        let mut save = false;
        let mut test = false;
        if let Some(rx) = &editor.probe
            && let Ok(result) = rx.try_recv()
        {
            crate::trace::popup(
                match &result {
                    Ok(text) => tf!("Подключение работает: {text}", "Connection works: {text}"),
                    Err(e) => tf!("Не удалось подключиться: {e}", "Could not connect: {e}"),
                },
                result.is_ok(),
            );
            editor.result = Some(result);
            editor.probe = None;
        }
        let modal = dialog("tror_db_editor", pal).show(ctx, |ui| {
            ui.set_width(640.0);
            crate::widgets::title(ui, pal, tr!("Профили баз данных"));
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(180.0);
                    ScrollArea::vertical()
                        .id_salt("tror_db_profiles")
                        .max_height(270.0)
                        .show(ui, |ui| {
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
                            editor.drafts.push(DbProfile {
                                name: tf!("База {}", "Database {}", editor.drafts.len() + 1),
                                ..Default::default()
                            });
                            editor.index = editor.drafts.len() - 1;
                            editor.result = None;
                        }
                        if ui
                            .add_enabled(!editor.drafts.is_empty(), Button::new(tr!("Удалить")))
                            .clicked()
                        {
                            editor.drafts.remove(editor.index);
                            editor.index = editor.index.saturating_sub(1);
                            editor.result = None;
                        }
                    });
                });
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    let Some(p) = editor.drafts.get_mut(editor.index) else {
                        ui.label(RichText::new(tr!("Добавьте профиль.")).color(pal.subtext));
                        return;
                    };
                    egui::Grid::new("tror_db_editor_grid")
                        .num_columns(2)
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            let wide = 300.0;
                            ui.label(tr!("Название"));
                            ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                            ui.end_row();
                            ui.label(tr!("Тип"));
                            ui.horizontal(|ui| {
                                for kind in [DbKind::MySql, DbKind::Postgres, DbKind::Sqlite] {
                                    if ui.selectable_label(p.kind == kind, kind.label()).clicked() && p.kind != kind {
                                        if p.port == p.kind.default_port() {
                                            p.port = kind.default_port();
                                        }
                                        p.kind = kind;
                                    }
                                }
                            });
                            ui.end_row();
                            if p.kind == DbKind::Sqlite {
                                ui.label(tr!("Файл базы"));
                                ui.horizontal(|ui| {
                                    ui.add(TextEdit::singleline(&mut p.file).desired_width(wide - 78.0));
                                    if ui.button(tr!("Обзор…")).clicked()
                                        && let Some(file) = rfd::FileDialog::new()
                                            .add_filter("SQLite", &["db", "sqlite", "sqlite3", "db3"])
                                            .add_filter(tr!("Все файлы"), &["*"])
                                            .pick_file()
                                    {
                                        p.file = file.display().to_string();
                                    }
                                });
                                ui.end_row();
                                return;
                            }
                            ui.label(tr!("Туннель через SSH"));
                            ComboBox::from_id_salt("tror_db_tunnel")
                                .width(wide)
                                .selected_text(if p.tunnel.is_empty() {
                                    tr!("нет, подключаться напрямую")
                                } else {
                                    p.tunnel.as_str()
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut p.tunnel,
                                        String::new(),
                                        tr!("нет, подключаться напрямую"),
                                    );
                                    for host in &store.profiles {
                                        if host.protocol == remote::Protocol::Sftp {
                                            ui.selectable_value(
                                                &mut p.tunnel,
                                                host.name.clone(),
                                                host.name.as_str(),
                                            );
                                        }
                                    }
                                });
                            ui.end_row();
                            ui.label(tr!("Шифрование TLS"));
                            ui.vertical(|ui| {
                                ui.checkbox(&mut p.tls, tr!("Подключаться по TLS"));
                                if p.tls {
                                    ui.checkbox(
                                        &mut p.tls_any_cert,
                                        tr!("Принимать любой сертификат (самоподписанный)"),
                                    );
                                }
                            });
                            ui.end_row();
                            ui.label(if p.tunnel.is_empty() {
                                tr!("Сервер")
                            } else {
                                tr!("Сервер (со стороны хоста)")
                            });
                            ui.horizontal(|ui| {
                                ui.add(TextEdit::singleline(&mut p.host).desired_width(wide - 70.0));
                                crate::widgets::port_field(ui, &mut p.port, 60.0);
                            });
                            ui.end_row();
                            ui.label(tr!("Пользователь"));
                            ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                            ui.end_row();
                            ui.label(tr!("Пароль"));
                            ui.horizontal(|ui| {
                                ui.add(
                                    TextEdit::singleline(&mut p.password)
                                        .password(!editor.show_password)
                                        .desired_width(wide - 96.0),
                                );
                                let label = if editor.show_password {
                                    tr!("Скрыть")
                                } else {
                                    tr!("Показать")
                                };
                                if ui.button(label).clicked() {
                                    editor.show_password = !editor.show_password;
                                }
                            });
                            ui.end_row();
                            ui.label(tr!("База (необязательно)"));
                            ui.add(TextEdit::singleline(&mut p.database).desired_width(wide));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    if editor.probe.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(tr!("проверка…")).color(pal.subtext));
                        });
                    } else if ui.add(Button::new(tr!("Проверить подключение"))).clicked() {
                        test = true;
                    }
                    if let Some(result) = &editor.result {
                        let (text, color) = match result {
                            Ok(text) => (tf!("Работает: {text}", "Works: {text}"), crate::widgets::ok_color(ui)),
                            Err(e) => (tf!("Ошибка: {e}", "Error: {e}"), pal.warn),
                        };
                        ui.label(RichText::new(text).small().color(color));
                    }
                    ui.add_space(6.0);
                    if p.kind == DbKind::Sqlite {
                        ui.label(
                            RichText::new(tr!("Файл SQLite открывается напрямую с этого компьютера. Если файла нет, он будет создан."))
                                .small()
                                .color(pal.subtext),
                        );
                    } else {
                        let sftp_hosts = store
                            .profiles
                            .iter()
                            .filter(|h| h.protocol == remote::Protocol::Sftp)
                            .count();
                        let default_port = p.kind.default_port();
                        let tunnel_note = if sftp_hosts == 0 {
                            tr!("Туннель через SSH нужен, когда база принимает подключения только с самого сервера — \
                             так устроен почти любой хостинг. В списке туннелей появляются ваши SFTP-профили из панели \
                             «Удалённые хосты» (F8); сейчас там нет ни одного SFTP-профиля, поэтому выбрать нечего.")
                                .to_owned()
                        } else if p.tunnel.is_empty() {
                            tr!("Туннель через SSH нужен, когда база принимает подключения только с самого сервера — \
                             так устроен почти любой хостинг. Выберите в списке SFTP-профиль этого сервера: программа \
                             зайдёт на него по SSH и уже оттуда подключится к базе.")
                                .to_owned()
                        } else {
                            tf!("Программа зайдёт на выбранный хост по SSH и подключится к базе оттуда. В поле «Сервер» \
                                 укажите адрес базы так, как его видит сам хост — обычно 127.0.0.1 или localhost, порт {default_port}.", "The program will log in to the chosen host over SSH and connect to the database from there. In the \"Server\" field \
                                 enter the database address as the host itself sees it — usually 127.0.0.1 or localhost, port {default_port}."
                            )
                        };
                        ui.label(RichText::new(tunnel_note).small().color(pal.subtext));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                tr!("Пароль хранится в файле профилей в замаскированном, но не зашифрованном виде. \
                                 Без TLS и без туннеля пароль и данные идут по сети открыто."),
                            )
                            .small()
                            .color(pal.faint),
                        );
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Сохранить"), true) {
                    save = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    done = true;
                }
            });
        });
        if test {
            let profile = editor.drafts.get(editor.index).cloned().unwrap_or_default();
            let ssh = (!profile.tunnel.is_empty() && profile.kind != DbKind::Sqlite)
                .then(|| store.profiles.iter().find(|p| p.name == profile.tunnel))
                .flatten()
                .map(|host| {
                    let key = format!("{}:{}", host.host.trim(), host.port);
                    (host.clone(), store.known_hosts.get(&key).cloned())
                });
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(test_profile(profile, ssh));
                ctx.request_repaint();
            });
            editor.probe = Some(rx);
            editor.result = None;
        }
        if editor.probe.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if save && let Some(editor) = self.editor.take() {
            store.databases = editor.drafts;
            self.selected = editor.index.min(store.databases.len().saturating_sub(1));
            if let Err(e) = store.save() {
                self.status = tf!("Не удалось сохранить профили: {e}", "Could not save profiles: {e}");
                self.error = true;
            }
            return;
        }
        if done || modal.should_close() {
            self.editor = None;
        }
    }

    pub(super) fn import_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some((name, statements)) = &self.import else {
            return;
        };
        let mut run = false;
        let mut cancel = false;
        let modal = dialog("tror_db_import", pal).show(ctx, |ui| {
            ui.set_width(560.0);
            crate::widgets::title(ui, pal, tf!("Импорт: {name}", "Import: {name}"));
            ui.label(
                RichText::new(tf!(
                    "Запросов: {}. Они выполнятся в одной транзакции: при ошибке ничего не изменится.",
                    "Statements: {}. They run in one transaction: on an error nothing changes.",
                    statements.len()
                ))
                .color(pal.subtext)
                .small(),
            );
            ui.add_space(6.0);
            ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for statement in statements.iter().take(20) {
                    let short: String = statement.chars().take(300).collect();
                    ui.add(egui::Label::new(RichText::new(short).monospace().size(12.0)).wrap());
                    ui.add_space(3.0);
                }
                if statements.len() > 20 {
                    ui.label(
                        RichText::new(tf!("…и ещё {}", "…and {} more", statements.len() - 20)).color(pal.faint),
                    );
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if !statements.is_empty() && dialog_button(ui, pal, tr!("Выполнить"), true) {
                    run = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if run && let Some((_, statements)) = self.import.take() {
            self.status = tr!("Идёт импорт…").into();
            self.error = false;
            self.send(Cmd::Import(statements));
        } else if cancel || modal.should_close() {
            self.import = None;
        }
    }

    pub(super) fn confirm_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(statements) = &self.edits.confirm else {
            return;
        };
        let mut run = false;
        let mut cancel = false;
        let modal = dialog("tror_db_apply", pal).show(ctx, |ui| {
            ui.set_width(560.0);
            crate::widgets::title(ui, pal, tr!("Применить изменения"));
            ui.label(
                RichText::new(tr!("Будут выполнены запросы, в одной транзакции:"))
                    .color(pal.subtext)
                    .small(),
            );
            ui.add_space(6.0);
            ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for statement in statements {
                    ui.add(
                        egui::Label::new(RichText::new(statement).monospace().size(12.0)).wrap(),
                    );
                    ui.add_space(3.0);
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, tr!("Выполнить"), true) {
                    run = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if run && let Some(statements) = self.edits.confirm.take() {
            let refresh = self.last_sql.clone();
            let source = self.source.clone();
            self.send(Cmd::Batch(statements, refresh, source));
        } else if cancel || modal.should_close() {
            self.edits.confirm = None;
        }
    }
}
