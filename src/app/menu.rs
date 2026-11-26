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
            ui.menu_button(tr!("Файл"), |ui| {
                ui.set_min_width(240.0);
                if item(ui, tr!("Новый"), &key("new"), true) {
                    actions.push(Action::New);
                }
                if item(ui, tr!("Открыть…"), &key("open"), true) {
                    actions.push(Action::Open);
                }
                if item(ui, tr!("Быстрое открытие…"), &key("quick_open"), true) {
                    actions.push(Action::Jump(""));
                }
                if item(ui, tr!("Открыть папку…"), &key("open_folder"), true) {
                    actions.push(Action::OpenFolder);
                }
                if item(ui, tr!("Открыть вторую папку…"), "", true) {
                    actions.push(Action::OpenSecondFolder);
                }
                ui.menu_button(tr!("Недавние"), |ui| {
                    if self.settings.recent.is_empty() {
                        ui.weak(tr!("пусто"));
                    }
                    for path in &self.settings.recent {
                        if ui.button(path.display().to_string()).clicked() {
                            actions.push(Action::OpenPath(path.clone()));
                        }
                    }
                });
                if !self.settings.recent_projects.is_empty() {
                    ui.menu_button(tr!("Недавние папки"), |ui| {
                        for path in self.settings.recent_projects.clone() {
                            if ui.button(path.display().to_string()).clicked() && path.is_dir() {
                                self.project.set_root(path);
                                actions.push(Action::Show(Pane::Project));
                            }
                        }
                    });
                }
                ui.separator();
                let current = self.docs.get(self.active);
                let savable =
                    current.is_some_and(|d| d.dirty() || d.path.is_none() && d.remote.is_none());
                if item(ui, tr!("Сохранить"), &key("save"), savable) {
                    actions.push(Action::Save);
                }
                if item(ui, tr!("Сохранить как…"), &key("save_as"), has_doc) {
                    actions.push(Action::SaveAs);
                }
                if item(
                    ui,
                    tr!("Сохранить все"),
                    "",
                    self.docs.iter().any(|d| d.dirty()),
                ) {
                    actions.push(Action::SaveAll);
                }
                if item(ui, tr!("Печать…"), &key("print"), has_doc) {
                    actions.push(Action::Print);
                }
                let backups = crate::remote::backups_dir().filter(|d| d.is_dir());
                if item(ui, tr!("Резервные копии с серверов"), "", backups.is_some())
                    && let Some(dir) = backups
                    && let Err(e) = crate::preview::open_folder(&dir)
                {
                    let now = ui.input(|i| i.time);
                    self.notify(
                        tf!(
                            "Не удалось открыть папку: {e}",
                            "Could not open the folder: {e}"
                        ),
                        now,
                    );
                }
                ui.separator();
                if item(ui, tr!("Закрыть вкладку"), &key("close_tab"), has_doc) {
                    actions.push(Action::CloseTab(self.active));
                }
                if item(ui, tr!("Закрыть все"), "", has_doc) {
                    actions.push(Action::CloseAll);
                }
                if item(
                    ui,
                    tr!("Открыть закрытую вкладку"),
                    &key("reopen"),
                    !self.closed.is_empty(),
                ) {
                    actions.push(Action::Reopen);
                }
                ui.separator();
                if item(ui, tr!("Выход"), "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button(tr!("Правка"), |ui| {
                ui.set_min_width(280.0);
                let doc = self.docs.get(self.active);
                let selected = doc.is_some_and(|d| d.has_selection());
                let filled = doc.is_some_and(|d| d.len() > 0);
                let line = doc.map_or(0, |d| d.rope.char_to_line(d.sel.1.min(d.len())));
                let last = doc.map_or(0, |d| d.line_count().saturating_sub(1));
                let marked = doc.is_some_and(|d| !d.marks.is_empty());
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
                    tr!("Отменить"),
                    &fixed("Z"),
                    doc.is_some_and(|d| d.can_undo()),
                    "undo",
                );
                edit(
                    ui,
                    tr!("Повторить"),
                    &fixed("Y"),
                    doc.is_some_and(|d| d.can_redo()),
                    "redo",
                );
                ui.separator();
                edit(ui, tr!("Вырезать"), &fixed("X"), selected, "cut");
                edit(ui, tr!("Копировать"), &fixed("C"), selected, "copy");
                edit(ui, tr!("Вставить"), &fixed("V"), true, "paste");
                edit(ui, tr!("Удалить"), "Delete", filled, "delete");
                edit(ui, tr!("Выделить всё"), &fixed("A"), filled, "select_all");
                ui.separator();
                edit(
                    ui,
                    tr!("Дублировать строку"),
                    &key("duplicate_line"),
                    filled,
                    "duplicate_line",
                );
                edit(
                    ui,
                    tr!("Удалить строку"),
                    &key("delete_line"),
                    filled,
                    "delete_line",
                );
                edit(
                    ui,
                    tr!("Переместить строку вверх"),
                    &key("line_up"),
                    line > 0,
                    "line_up",
                );
                edit(
                    ui,
                    tr!("Переместить строку вниз"),
                    &key("line_down"),
                    line < last,
                    "line_down",
                );
                ui.separator();
                edit(
                    ui,
                    tr!("Закомментировать / раскомментировать"),
                    &fixed("/"),
                    filled,
                    "comment",
                );
                edit(ui, tr!("Увеличить отступ"), "Tab", true, "indent");
                edit(ui, tr!("Уменьшить отступ"), "Shift+Tab", filled, "outdent");
                edit(
                    ui,
                    tr!("Удалить пробелы в конце строк"),
                    &key("trim"),
                    filled,
                    "trim",
                );
                ui.menu_button(tr!("Регистр"), |ui| {
                    ui.set_min_width(220.0);
                    edit(ui, tr!("ПРОПИСНЫЕ"), &key("upper"), filled, "upper");
                    edit(ui, tr!("строчные"), &key("lower"), filled, "lower");
                });
                ui.menu_button(tr!("Закладки"), |ui| {
                    ui.set_min_width(240.0);
                    edit(
                        ui,
                        tr!("Поставить или снять"),
                        &key("bookmark"),
                        true,
                        "bookmark",
                    );
                    edit(
                        ui,
                        tr!("Следующая"),
                        &key("bookmark_next"),
                        marked,
                        "bookmark_next",
                    );
                    edit(
                        ui,
                        tr!("Предыдущая"),
                        &key("bookmark_prev"),
                        marked,
                        "bookmark_prev",
                    );
                    edit(
                        ui,
                        tr!("Убрать все"),
                        "",
                        doc.is_some_and(|d| !d.marks.is_empty()),
                        "bookmark_clear",
                    );
                });
            });
            ui.menu_button(tr!("Поиск"), |ui| {
                ui.set_min_width(240.0);
                if item(ui, tr!("Найти…"), &key("find"), has_doc) {
                    actions.push(Action::Find);
                }
                if item(ui, tr!("Заменить…"), &key("replace"), has_doc) {
                    actions.push(Action::Replace);
                }
                let queried = has_doc && self.search.has_query();
                if item(ui, tr!("Найти далее"), "F3", queried) {
                    actions.push(Action::FindNext);
                }
                if item(ui, tr!("Найти ранее"), "Shift+F3", queried) {
                    actions.push(Action::FindPrev);
                }
                ui.separator();
                if item(ui, tr!("Перейти к строке…"), &key("goto"), has_doc) {
                    actions.push(Action::GotoLine);
                }
                ui.separator();
                let doc = self.docs.get(self.active);
                let line = doc.map_or(0, |d| d.rope.char_to_line(d.sel.1.min(d.len())));
                let foldable =
                    doc.is_some_and(|d| d.block_around(line).is_some_and(|(h, _)| !d.is_folded(h)));
                let unfoldable =
                    doc.is_some_and(|d| d.folds.iter().any(|f| f.0 <= line && f.1 >= line));
                let any_folds = doc.is_some_and(|d| !d.folds.is_empty());
                let blocks = doc.is_some_and(|d| d.has_blocks());
                for (kind, label, id, enabled) in [
                    (0, tr!("Свернуть блок"), "fold", foldable),
                    (1, tr!("Развернуть блок"), "unfold", unfoldable),
                    (2, tr!("Свернуть всё"), "fold_all", blocks),
                    (3, tr!("Развернуть всё"), "unfold_all", any_folds),
                ] {
                    if item(ui, label, &key(id), enabled) {
                        actions.push(Action::Fold(kind));
                    }
                }
                ui.separator();
                if item(ui, tr!("Найти в файлах проекта…"), &key("find_files"), true)
                {
                    actions.push(Action::FindFiles);
                }
            });
            ui.menu_button(tr!("Вид"), |ui| {
                ui.set_min_width(260.0);
                if item(ui, tr!("Палитра команд…"), &key("palette"), true) {
                    actions.push(Action::Jump(">"));
                }
                ui.separator();
                let theme = if self.settings.look.dark {
                    tr!("Светлая тема")
                } else {
                    tr!("Тёмная тема")
                };
                if item(ui, theme, "", true) {
                    actions.push(Action::ToggleTheme);
                }
                let wrap = if self.settings.wrap {
                    tr!("Выключить перенос строк")
                } else {
                    tr!("Перенос строк")
                };
                if item(ui, wrap, &key("wrap"), true) {
                    actions.push(Action::ToggleWrap);
                }
                let current = self.docs.get(self.active);
                let visual_ok = current.is_some_and(crate::visual::supports);
                let visual_on = current.is_some_and(|d| self.visual.contains_key(&d.id));
                let visual = if visual_on {
                    tr!("Вернуться к коду")
                } else {
                    tr!("Визуальный редактор")
                };
                if item(ui, visual, &key("visual"), visual_ok) {
                    actions.push(Action::ToggleVisual);
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
                            tf!("Подключение {n} (SFTP/FTP)", "Connection {n} (SFTP/FTP)"),
                            tf!("Скрыть подключение {n}", "Hide connection {n}"),
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
                if self.remotes.len() < 16 && item(ui, tr!("Ещё одно подключение"), "", true)
                {
                    actions.push(Action::AddHosts);
                }
                ui.separator();
                let panes = [
                    (
                        Pane::Project,
                        tr!("Папка проекта"),
                        tr!("Скрыть папку проекта"),
                        "project",
                    ),
                    (
                        Pane::Project2,
                        tr!("Вторая папка проекта"),
                        tr!("Скрыть вторую папку"),
                        "",
                    ),
                    (
                        Pane::Hosts,
                        tr!("Удалённые хосты (SFTP/FTP)"),
                        tr!("Скрыть удалённые хосты"),
                        "hosts",
                    ),
                    (
                        Pane::Console,
                        tr!("Консоль (SSH, Telnet, Mosh)"),
                        tr!("Скрыть консоль"),
                        "console",
                    ),
                    (Pane::Log, tr!("Журнал"), tr!("Скрыть журнал"), "log"),
                    (
                        Pane::Db,
                        tr!("База данных"),
                        tr!("Скрыть базу данных"),
                        "db",
                    ),
                    (
                        Pane::Diff,
                        tr!("Сравнение файлов"),
                        tr!("Скрыть сравнение файлов"),
                        "",
                    ),
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
                if item(ui, tr!("Сбросить расположение панелей"), "", true)
                {
                    actions.push(Action::ResetLayout);
                }
                ui.separator();
                ui.weak(tr!(
                    "Шрифт редактора: Ctrl + / Ctrl − / Ctrl 0, Ctrl + колесо"
                ));
            });
            ui.menu_button(tr!("Инструменты"), |ui| {
                ui.set_min_width(260.0);
                if item(
                    ui,
                    tr!("Сравнить с другим файлом"),
                    &key("compare"),
                    has_doc,
                ) {
                    actions.push(Action::Compare);
                }
                if item(ui, tr!("Предпросмотр в браузере"), &key("preview"), has_doc)
                {
                    actions.push(Action::Preview);
                }
                ui.separator();
                if item(ui, tr!("Проверить обновления"), "", true) {
                    actions.push(Action::CheckUpdates);
                }
                ui.separator();
                let record = if self.macros.recording {
                    tr!("Остановить запись макроса")
                } else {
                    tr!("Записать макрос")
                };
                if item(ui, record, &key("macro_record"), has_doc) {
                    actions.push(Action::MacroRecord);
                }
                let ready = has_doc && !self.macros.recording && !self.macros.events.is_empty();
                if item(ui, tr!("Выполнить макрос"), &key("macro_play"), ready) {
                    actions.push(Action::MacroPlay);
                }
                if item(ui, tr!("Выполнить несколько раз…"), "", ready) {
                    actions.push(Action::MacroDialog(false));
                }
                if item(ui, tr!("Сохранить макрос…"), "", ready) {
                    actions.push(Action::MacroDialog(true));
                }
                if !self.settings.macros.is_empty() {
                    ui.menu_button(tr!("Сохранённые макросы"), |ui| {
                        ui.set_min_width(220.0);
                        for (index, (name, _)) in self.settings.macros.iter().enumerate() {
                            ui.menu_button(name, |ui| {
                                ui.set_min_width(180.0);
                                if item(ui, tr!("Выполнить"), "", has_doc) {
                                    actions.push(Action::MacroLoad(index, true));
                                }
                                if item(ui, tr!("Сделать текущим"), "", true) {
                                    actions.push(Action::MacroLoad(index, false));
                                }
                                if item(ui, tr!("Удалить"), "", true) {
                                    actions.push(Action::MacroDelete(index));
                                }
                            });
                        }
                    });
                }
                ui.separator();
                if item(ui, tr!("База данных"), "", true) {
                    actions.push(Action::Show(Pane::Db));
                }
                if item(ui, tr!("Поиск на сервере"), "", true) {
                    actions.push(Action::FindFiles);
                }
            });
            if ui.button(tr!("Настройки")).clicked() {
                actions.push(Action::Show(Pane::Settings));
            }
            ui.menu_button(tr!("Плагины"), |ui| {
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
                        if !manifest.settings.is_empty() && item(ui, tr!("Настройки…"), "", true)
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
                    ui.weak(tr!("Плагинов пока нет"));
                }
                ui.separator();
                if item(ui, tr!("Управление плагинами…"), "", true) {
                    actions.push(Action::PluginsManage);
                }
                if item(ui, tr!("Открыть папку плагинов"), "", true) {
                    actions.push(Action::PluginsFolder);
                }
                if item(ui, tr!("Перечитать"), "", true) {
                    actions.push(Action::PluginsReload);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let theme_hint = if self.settings.look.dark {
                    tr!("Светлая тема")
                } else {
                    tr!("Тёмная тема")
                };
                if Self::tool_button(ui, pal, 3, false, theme_hint) {
                    actions.push(Action::ToggleTheme);
                }
                if Self::tool_button(ui, pal, 2, self.settings.wrap, tr!("Перенос строк"))
                {
                    actions.push(Action::ToggleWrap);
                }
                if let Some(doc) = self.docs.get(self.active)
                    && crate::visual::supports(doc)
                {
                    let on = self.visual.contains_key(&doc.id);
                    let hint = if on {
                        tr!("Вернуться к коду")
                    } else {
                        tr!("Визуальный редактор")
                    };
                    if Self::tool_button(ui, pal, 5, on, hint) {
                        actions.push(Action::ToggleVisual);
                    }
                }
                if Self::tool_button(
                    ui,
                    pal,
                    1,
                    dock::has(&self.dock, Pane::Console),
                    tr!("Консоль"),
                ) {
                    actions.push(Action::Toggle(Pane::Console));
                }
                if Self::tool_button(
                    ui,
                    pal,
                    0,
                    dock::has(&self.dock, Pane::Hosts),
                    tr!("Удалённые хосты"),
                ) {
                    actions.push(Action::Toggle(Pane::Hosts));
                }
                if crate::widgets::plus_button(ui, pal, egui::vec2(30.0, 26.0))
                    .on_hover_text(tr!("Новая вкладка"))
                    .clicked()
                {
                    actions.push(Action::New);
                }
            });
        });
    }
}
