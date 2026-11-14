use super::*;

impl TrorApp {
    pub(super) fn menu(&mut self, ui: &mut Ui, pal: &Palette, actions: &mut Vec<Action>) {
        let has_doc = !self.docs.is_empty();
        let item = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool| {
            ui.add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
                .clicked()
        };
        let keys = crate::keys::Keys::new(&self.settings.keys);
        let key = |id: &str| keys.label(id);
        MenuBar::new().ui(ui, |ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(18.0, 18.0)));
                ui.add_space(2.0);
            }
            ui.menu_button("Файл", |ui| {
                ui.set_min_width(240.0);
                if item(ui, "Новый", &key("new"), true) {
                    actions.push(Action::New);
                }
                if item(ui, "Открыть…", &key("open"), true) {
                    actions.push(Action::Open);
                }
                if item(ui, "Открыть папку…", &key("open_folder"), true) {
                    actions.push(Action::OpenFolder);
                }
                ui.menu_button("Недавние", |ui| {
                    if self.settings.recent.is_empty() {
                        ui.weak("пусто");
                    }
                    for path in &self.settings.recent {
                        if ui.button(path.display().to_string()).clicked() {
                            actions.push(Action::OpenPath(path.clone()));
                        }
                    }
                });
                if !self.settings.recent_projects.is_empty() {
                    ui.menu_button("Недавние папки", |ui| {
                        for path in self.settings.recent_projects.clone() {
                            if ui.button(path.display().to_string()).clicked() && path.is_dir() {
                                self.project.set_root(path);
                                actions.push(Action::Show(Pane::Project));
                            }
                        }
                    });
                }
                ui.separator();
                if item(ui, "Сохранить", &key("save"), has_doc) {
                    actions.push(Action::Save);
                }
                if item(ui, "Сохранить как…", &key("save_as"), has_doc) {
                    actions.push(Action::SaveAs);
                }
                if item(ui, "Сохранить все", "", has_doc) {
                    actions.push(Action::SaveAll);
                }
                ui.separator();
                if item(ui, "Закрыть вкладку", &key("close_tab"), has_doc) {
                    actions.push(Action::CloseTab(self.active));
                }
                if item(ui, "Закрыть все", "", has_doc) {
                    actions.push(Action::CloseAll);
                }
                if item(
                    ui,
                    "Открыть закрытую вкладку",
                    &key("reopen"),
                    !self.closed.is_empty(),
                ) {
                    actions.push(Action::Reopen);
                }
                ui.separator();
                if item(ui, "Настройки", &key("settings"), true) {
                    actions.push(Action::Show(Pane::Settings));
                }
                ui.separator();
                if item(ui, "Выход", "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button("Правка", |ui| {
                ui.set_min_width(280.0);
                let doc = self.docs.get(self.active);
                let selected = doc.is_some_and(|d| d.has_selection());
                let cmd = if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl+"
                };
                let fixed = |k: &str| format!("{cmd}{k}");
                let mut edit =
                    |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, id: &'static str| {
                        if item(ui, label, shortcut, enabled && has_doc) {
                            actions.push(Action::Edit(id));
                        }
                    };
                edit(
                    ui,
                    "Отменить",
                    &fixed("Z"),
                    doc.is_some_and(|d| d.can_undo()),
                    "undo",
                );
                edit(
                    ui,
                    "Повторить",
                    &fixed("Y"),
                    doc.is_some_and(|d| d.can_redo()),
                    "redo",
                );
                ui.separator();
                edit(ui, "Вырезать", &fixed("X"), selected, "cut");
                edit(ui, "Копировать", &fixed("C"), selected, "copy");
                edit(ui, "Вставить", &fixed("V"), true, "paste");
                edit(ui, "Удалить", "Delete", true, "delete");
                edit(ui, "Выделить всё", &fixed("A"), true, "select_all");
                ui.separator();
                edit(
                    ui,
                    "Дублировать строку",
                    &key("duplicate_line"),
                    true,
                    "duplicate_line",
                );
                edit(
                    ui,
                    "Удалить строку",
                    &key("delete_line"),
                    true,
                    "delete_line",
                );
                edit(
                    ui,
                    "Переместить строку вверх",
                    &key("line_up"),
                    true,
                    "line_up",
                );
                edit(
                    ui,
                    "Переместить строку вниз",
                    &key("line_down"),
                    true,
                    "line_down",
                );
                ui.separator();
                edit(
                    ui,
                    "Закомментировать / раскомментировать",
                    &fixed("/"),
                    true,
                    "comment",
                );
                edit(ui, "Увеличить отступ", "Tab", true, "indent");
                edit(ui, "Уменьшить отступ", "Shift+Tab", true, "outdent");
                edit(
                    ui,
                    "Удалить пробелы в конце строк",
                    &key("trim"),
                    true,
                    "trim",
                );
                ui.menu_button("Регистр", |ui| {
                    ui.set_min_width(220.0);
                    edit(ui, "ПРОПИСНЫЕ", &key("upper"), true, "upper");
                    edit(ui, "строчные", &key("lower"), true, "lower");
                });
                ui.menu_button("Закладки", |ui| {
                    ui.set_min_width(240.0);
                    edit(
                        ui,
                        "Поставить или снять",
                        &key("bookmark"),
                        true,
                        "bookmark",
                    );
                    edit(
                        ui,
                        "Следующая",
                        &key("bookmark_next"),
                        true,
                        "bookmark_next",
                    );
                    edit(
                        ui,
                        "Предыдущая",
                        &key("bookmark_prev"),
                        true,
                        "bookmark_prev",
                    );
                    edit(
                        ui,
                        "Убрать все",
                        "",
                        doc.is_some_and(|d| !d.marks.is_empty()),
                        "bookmark_clear",
                    );
                });
            });
            ui.menu_button("Поиск", |ui| {
                ui.set_min_width(240.0);
                if item(ui, "Найти…", &key("find"), has_doc) {
                    actions.push(Action::Find);
                }
                if item(ui, "Заменить…", &key("replace"), has_doc) {
                    actions.push(Action::Replace);
                }
                if item(ui, "Найти далее", "F3", has_doc) {
                    actions.push(Action::FindNext);
                }
                if item(ui, "Найти ранее", "Shift+F3", has_doc) {
                    actions.push(Action::FindPrev);
                }
                ui.separator();
                if item(ui, "Перейти к строке…", &key("goto"), has_doc) {
                    actions.push(Action::GotoLine);
                }
                ui.separator();
                for (kind, label, id) in [
                    (0, "Свернуть блок", "fold"),
                    (1, "Развернуть блок", "unfold"),
                    (2, "Свернуть всё", "fold_all"),
                    (3, "Развернуть всё", "unfold_all"),
                ] {
                    if item(ui, label, &key(id), has_doc) {
                        actions.push(Action::Fold(kind));
                    }
                }
                ui.separator();
                if item(ui, "Найти в файлах проекта…", &key("find_files"), true)
                {
                    actions.push(Action::FindFiles);
                }
            });
            ui.menu_button("Вид", |ui| {
                ui.set_min_width(260.0);
                let theme = if self.settings.look.dark {
                    "Светлая тема"
                } else {
                    "Тёмная тема"
                };
                if item(ui, theme, "", true) {
                    actions.push(Action::ToggleTheme);
                }
                let wrap = if self.settings.wrap {
                    "Выключить перенос строк"
                } else {
                    "Перенос строк"
                };
                if item(ui, wrap, &key("wrap"), true) {
                    actions.push(Action::ToggleWrap);
                }
                ui.separator();
                let extra: Vec<(Pane, String, String)> = self
                    .hosts_panes()
                    .into_iter()
                    .skip(1)
                    .filter(|pane| {
                        let i = pane.host_index().unwrap_or(0);
                        self.host_shown(i) || self.remotes[i].in_use()
                    })
                    .map(|pane| {
                        let n = pane.host_index().unwrap_or(0) + 1;
                        (
                            pane,
                            format!("Подключение {n} (SFTP/FTP)"),
                            format!("Скрыть подключение {n}"),
                        )
                    })
                    .collect();
                for (pane, show, hide) in &extra {
                    let label =
                        if dock::has(&self.dock, *pane) || self.settings.detached.contains(pane) {
                            hide
                        } else {
                            show
                        };
                    if item(ui, label, "", true) {
                        actions.push(Action::Toggle(*pane));
                    }
                }
                if self.remotes.len() < 16 && item(ui, "Ещё одно подключение", "", true)
                {
                    actions.push(Action::AddHosts);
                }
                ui.separator();
                let panes = [
                    (
                        Pane::Project,
                        "Папка проекта",
                        "Скрыть папку проекта",
                        "project",
                    ),
                    (
                        Pane::Hosts,
                        "Удалённые хосты (SFTP/FTP)",
                        "Скрыть удалённые хосты",
                        "hosts",
                    ),
                    (
                        Pane::Console,
                        "Консоль (SSH, Telnet, Mosh)",
                        "Скрыть консоль",
                        "console",
                    ),
                    (Pane::Log, "Журнал", "Скрыть журнал", "log"),
                    (
                        Pane::ServerSearch,
                        "Поиск на сервере",
                        "Скрыть поиск на сервере",
                        "server_search",
                    ),
                    (Pane::Db, "База данных", "Скрыть базу данных", "db"),
                    (
                        Pane::Diff,
                        "Сравнение файлов",
                        "Скрыть сравнение файлов",
                        "",
                    ),
                    (Pane::Settings, "Настройки", "Скрыть настройки", ""),
                ];
                for (pane, show, hide, shortcut) in panes {
                    let label =
                        if dock::has(&self.dock, pane) || self.settings.detached.contains(&pane) {
                            hide
                        } else {
                            show
                        };
                    if item(ui, label, &key(shortcut), true) {
                        actions.push(Action::Toggle(pane));
                    }
                }
                ui.separator();
                if item(ui, "Сбросить расположение панелей", "", true) {
                    actions.push(Action::ResetLayout);
                }
                ui.separator();
                ui.weak("Масштаб: Ctrl + / Ctrl − / Ctrl 0");
            });
            ui.menu_button("Инструменты", |ui| {
                ui.set_min_width(260.0);
                if item(ui, "Сравнить с другим файлом", &key("compare"), has_doc)
                {
                    actions.push(Action::Compare);
                }
                if item(ui, "Предпросмотр в браузере", &key("preview"), has_doc)
                {
                    actions.push(Action::Preview);
                }
                ui.separator();
                if item(ui, "Проверить обновления", "", true) {
                    actions.push(Action::CheckUpdates);
                }
                ui.separator();
                let record = if self.macros.recording {
                    "Остановить запись макроса"
                } else {
                    "Записать макрос"
                };
                if item(ui, record, &key("macro_record"), has_doc) {
                    actions.push(Action::MacroRecord);
                }
                let ready = has_doc && !self.macros.recording && !self.macros.events.is_empty();
                if item(ui, "Выполнить макрос", &key("macro_play"), ready) {
                    actions.push(Action::MacroPlay);
                }
                if item(ui, "Выполнить несколько раз…", "", ready) {
                    actions.push(Action::MacroDialog(false));
                }
                if item(ui, "Сохранить макрос…", "", ready) {
                    actions.push(Action::MacroDialog(true));
                }
                if !self.settings.macros.is_empty() {
                    ui.menu_button("Сохранённые макросы", |ui| {
                        ui.set_min_width(220.0);
                        for (index, (name, _)) in self.settings.macros.iter().enumerate() {
                            ui.menu_button(name, |ui| {
                                ui.set_min_width(180.0);
                                if item(ui, "Выполнить", "", has_doc) {
                                    actions.push(Action::MacroLoad(index, true));
                                }
                                if item(ui, "Сделать текущим", "", true) {
                                    actions.push(Action::MacroLoad(index, false));
                                }
                                if item(ui, "Удалить", "", true) {
                                    actions.push(Action::MacroDelete(index));
                                }
                            });
                        }
                    });
                }
                ui.separator();
                if item(ui, "База данных", "", true) {
                    actions.push(Action::Show(Pane::Db));
                }
                if item(ui, "Поиск на сервере", "", true) {
                    actions.push(Action::Show(Pane::ServerSearch));
                }
            });
            ui.menu_button("Плагины", |ui| {
                ui.set_min_width(260.0);
                for (index, plugin) in self.plugins.list.iter().enumerate() {
                    let shortcut = crate::keys::parse(&plugin.shortcut)
                        .map(|s| crate::keys::format(&s))
                        .unwrap_or_default();
                    if item(ui, &plugin.name, &shortcut, true) {
                        actions.push(Action::Plugin(index));
                    }
                }
                for (index, plugin) in self.plugins.wasm.iter().enumerate() {
                    let enabled = plugin.broken.is_none();
                    let manifest = &plugin.manifest;
                    let mut entries = |ui: &mut Ui| {
                        for (command, def) in manifest.commands.iter().enumerate() {
                            let shortcut = crate::keys::parse(&def.key)
                                .map(|s| crate::keys::format(&s))
                                .unwrap_or_default();
                            if item(ui, &def.title, &shortcut, enabled) {
                                actions.push(Action::PluginCommand(index, command));
                            }
                        }
                        for (panel, def) in manifest.panels.iter().enumerate() {
                            if item(ui, &def.title, "", enabled) {
                                actions.push(Action::PluginPanel(index, panel));
                            }
                        }
                        if !manifest.settings.is_empty() && item(ui, "Настройки…", "", true)
                        {
                            actions.push(Action::PluginSettings(index));
                        }
                    };
                    let count = manifest.commands.len()
                        + manifest.panels.len()
                        + usize::from(!manifest.settings.is_empty());
                    match count {
                        0 => {}
                        1 if manifest.settings.is_empty() => entries(ui),
                        _ => {
                            ui.menu_button(&manifest.name, |ui| {
                                ui.set_min_width(240.0);
                                entries(ui);
                            });
                        }
                    }
                }
                if self.plugins.count() == 0 {
                    ui.weak("Плагинов пока нет");
                }
                ui.separator();
                if item(ui, "Открыть папку плагинов", "", true) {
                    actions.push(Action::PluginsFolder);
                }
                if item(ui, "Перечитать", "", true) {
                    actions.push(Action::PluginsReload);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let theme_hint = if self.settings.look.dark {
                    "Светлая тема"
                } else {
                    "Тёмная тема"
                };
                if Self::tool_button(ui, pal, 3, false, theme_hint) {
                    actions.push(Action::ToggleTheme);
                }
                if Self::tool_button(ui, pal, 2, self.settings.wrap, "Перенос строк") {
                    actions.push(Action::ToggleWrap);
                }
                if Self::tool_button(ui, pal, 1, dock::has(&self.dock, Pane::Console), "Консоль")
                {
                    actions.push(Action::Toggle(Pane::Console));
                }
                if Self::tool_button(
                    ui,
                    pal,
                    0,
                    dock::has(&self.dock, Pane::Hosts),
                    "Удалённые хосты",
                ) {
                    actions.push(Action::Toggle(Pane::Hosts));
                }
                if crate::widgets::plus_button(ui, pal, egui::vec2(30.0, 26.0))
                    .on_hover_text("Новая вкладка")
                    .clicked()
                {
                    actions.push(Action::New);
                }
            });
        });
    }
}
