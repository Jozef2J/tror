use super::*;

impl TrorApp {
    pub(super) fn open_search(&mut self, replace: bool) {
        self.search.open = true;
        self.find.focus = true;
        self.search.replace_mode = replace;
        self.search.focus_query = true;
        self.search.last_replaced = None;
        if let Some(doc) = self.docs.get(self.active) {
            let range = doc.selection();
            if !range.is_empty() && range.len() < 200 {
                let selected = doc.selected_text();
                if !selected.contains('\n') {
                    self.search.query = selected;
                }
            }
        }
    }

    pub(super) fn run(&mut self, action: Action, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.complete.close();
        match action {
            Action::New => self.new_doc(),
            Action::Open => self.open_dialog(now),
            Action::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.project.set_root(dir);
                    if !self.settings.detached.contains(&Pane::Project) {
                        dock::show(&mut self.dock, Pane::Project, &self.settings.floats);
                    }
                }
            }
            Action::AddHosts => {
                let free = (1..self.remotes.len())
                    .find(|i| !self.host_shown(*i) && !self.remotes[*i].in_use());
                let index = match free {
                    Some(index) => Some(index),
                    None if self.remotes.len() < 16 => {
                        self.remotes
                            .push(RemotePanel::new(self.remotes.len() as u8));
                        Some(self.remotes.len() - 1)
                    }
                    None => None,
                };
                if let Some(index) = index {
                    self.settings.hosts = self.remotes.len();
                    dock::show(&mut self.dock, Pane::hosts(index), &self.settings.floats);
                }
            }
            Action::OpenPath(path) => self.open_path(&path, now),
            Action::Save => {
                self.save(self.active, now);
            }
            Action::SaveAs => {
                self.save_as(self.active, now);
            }
            Action::SaveAll => {
                self.save_all(now);
            }
            Action::Activate(i) => {
                if i < self.docs.len() {
                    self.active = i;
                    self.focus_editor = true;
                }
            }
            Action::CloseTab(i) => self.request_close(i),
            Action::TabCmd(id, cmd) => self.tab_cmd(id, cmd, now),
            Action::Reopen => {
                if let Some((path, remote)) = self.closed.pop() {
                    match remote {
                        Some((profile, file)) => {
                            let index = self
                                .remotes
                                .iter()
                                .position(|r| r.serves(&profile))
                                .or_else(|| self.remotes.iter().position(|r| !r.in_use()))
                                .unwrap_or(0);
                            if let Err(e) = self.remotes[index].reopen(ctx, &profile, &file) {
                                self.notify(format!("Не удалось открыть {file}: {e}"), now);
                            } else if !self.host_shown(index) {
                                dock::show(
                                    &mut self.dock,
                                    Pane::hosts(index),
                                    &self.settings.floats,
                                );
                            }
                        }
                        None => {
                            if let Some(path) = path {
                                self.open_path(&path, now);
                            }
                        }
                    }
                } else {
                    self.notify("Закрытых вкладок нет", now);
                }
            }
            Action::CloseAll => {
                if self.docs.iter().any(|d| d.dirty()) {
                    self.notify(
                        "Есть несохранённые вкладки — закройте их по отдельности",
                        now,
                    );
                    self.docs.retain(|d| d.dirty());
                    self.active = 0;
                } else {
                    self.docs.clear();
                    self.active = 0;
                }
            }
            Action::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Action::Find => self.open_search(false),
            Action::Replace => self.open_search(true),
            Action::FindNext => {
                if self.search.query.is_empty() {
                    self.open_search(false);
                } else {
                    self.search.open = true;
                    self.search.pending = Some(Pending::Next);
                }
            }
            Action::FindPrev => {
                if !self.search.query.is_empty() {
                    self.search.open = true;
                    self.search.pending = Some(Pending::Prev);
                }
            }
            Action::CloseSearch => {
                self.search.open = false;
                self.focus_editor = true;
            }
            Action::GotoLine => {
                if !self.docs.is_empty() {
                    self.goto = Some(String::new());
                }
            }
            Action::Edit(command) => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    match command {
                        "cut" => crate::editor::copy_selection(ctx, doc, true),
                        "copy" => crate::editor::copy_selection(ctx, doc, false),
                        "paste" => {
                            ctx.memory_mut(|m| m.request_focus(crate::editor::editor_id(doc.id)));
                            ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
                        }
                        "delete" => {
                            if doc.has_selection() {
                                doc.insert_text("");
                            } else {
                                doc.delete(false);
                            }
                        }
                        "undo" => doc.undo(),
                        "redo" => doc.redo(),
                        "select_all" => doc.select_all(),
                        "comment" => doc.toggle_comment(),
                        "indent" => doc.indent(),
                        "outdent" => doc.outdent(),
                        "duplicate_line" => doc.duplicate_lines(),
                        "delete_line" => doc.delete_lines(),
                        "line_up" => doc.move_lines(true),
                        "line_down" => doc.move_lines(false),
                        "upper" => doc.transform(|t| t.to_uppercase()),
                        "lower" => doc.transform(|t| t.to_lowercase()),
                        "trim" => {
                            let count = doc.trim_trailing();
                            self.notify(format!("Пробелы в конце строк убраны: {count}"), now);
                        }
                        "bookmark" => doc.toggle_mark(),
                        "bookmark_next" | "bookmark_prev" => {
                            if !doc.jump_mark(command == "bookmark_next") {
                                self.notify("Закладок нет: Ctrl+F2 ставит закладку на строку", now);
                            }
                        }
                        "bookmark_clear" => doc.marks.clear(),
                        _ => {}
                    }
                }
                self.focus_editor = true;
            }
            Action::Fold(kind) => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    match kind {
                        0 => {
                            doc.fold_here();
                        }
                        1 => {
                            doc.unfold_here();
                        }
                        2 => doc.fold_all(),
                        _ => doc.unfold_all(),
                    }
                    doc.reveal = true;
                }
                self.focus_editor = true;
            }
            Action::ToggleTheme => self.settings.look.dark = !self.settings.look.dark,
            Action::ToggleWrap => self.settings.wrap = !self.settings.wrap,
            Action::Toggle(pane) => {
                if self.settings.detached.contains(&pane) {
                    self.settings.detached.retain(|p| *p != pane);
                } else {
                    dock::toggle(&mut self.dock, pane, &self.settings.floats);
                }
            }
            Action::Detach(pane) => {
                dock::hide(&mut self.dock, pane);
                if !self.settings.detached.contains(&pane) {
                    self.settings.detached.push(pane);
                }
            }
            Action::Attach(pane) => {
                self.settings.detached.retain(|p| *p != pane);
                if !matches!(pane, Pane::Doc(_)) {
                    dock::show(&mut self.dock, pane, &self.settings.floats);
                }
            }
            Action::Show(pane) => {
                if !self.settings.detached.contains(&pane) {
                    dock::show(&mut self.dock, pane, &self.settings.floats);
                }
            }
            Action::ResetLayout => {
                self.dock = dock::new();
                self.focus_editor = true;
            }
            Action::Preview => {
                let url = match self.docs.get(self.active) {
                    None => Err("Нет открытого файла".to_owned()),
                    Some(doc) => match (&doc.remote, &doc.path) {
                        (Some((profile, path)), _) => self.remotes[0].site_url(profile, path).ok_or(
                            "В профиле хоста не указан адрес сайта — добавьте его в настройках профиля"
                                .to_owned(),
                        ),
                        (None, Some(path)) => self.preview.url(path),
                        _ => Err("Сначала сохраните файл на диск".to_owned()),
                    },
                };
                match url {
                    Ok(url) => match preview::open_url(&url) {
                        Ok(()) => self.notify(format!("Открыто в браузере: {url}"), now),
                        Err(e) => self.notify(format!("Предпросмотр: {e}"), now),
                    },
                    Err(e) => self.notify(format!("Предпросмотр: {e}"), now),
                }
            }
            Action::CheckUpdates => {
                self.update.manual = true;
                self.update.rx = Some(crate::update::check());
                self.notify("Проверяю обновления…", now);
            }
            Action::FindFiles => self.find_files.show(),
            Action::MacroRecord => {
                if self.macros.recording {
                    self.macros.recording = false;
                    let count = self.macros.events.len();
                    self.notify(format!("Макрос записан, действий: {count}"), now);
                } else {
                    self.macros.events.clear();
                    self.macros.recording = true;
                    self.notify(
                        "Идёт запись макроса — нажмите сочетание ещё раз, чтобы остановить",
                        now,
                    );
                }
                self.focus_editor = true;
            }
            Action::MacroPlay => {
                if self.macros.recording {
                    self.notify("Сначала остановите запись макроса", now);
                } else if self.macros.events.is_empty() {
                    self.notify("Макрос ещё не записан", now);
                } else {
                    self.macros.play += 1;
                    self.focus_editor = true;
                }
            }
            Action::MacroDialog(save) => {
                if self.macros.recording || self.macros.events.is_empty() {
                    self.notify("Макрос ещё не записан", now);
                } else {
                    self.macro_dialog = Some(if save {
                        MacroDialog::Save(String::new())
                    } else {
                        MacroDialog::Run(10, false)
                    });
                }
            }
            Action::MacroLoad(index, run) => {
                if let Some((name, events)) = self.settings.macros.get(index) {
                    self.macros.events = events.clone();
                    if run {
                        self.macros.play += 1;
                    } else {
                        self.notify(format!("Текущий макрос: {name}"), now);
                    }
                    self.focus_editor = true;
                }
            }
            Action::MacroDelete(index) => {
                if index < self.settings.macros.len() {
                    self.settings.macros.remove(index);
                }
            }
            Action::Compare => {
                if let Some(doc) = self.docs.get(self.active) {
                    self.diff.left = Some(doc.id);
                    if self.diff.right.is_none() || self.diff.right == self.diff.left {
                        self.diff.right = self
                            .docs
                            .iter()
                            .cycle()
                            .skip(self.active + 1)
                            .take(self.docs.len().saturating_sub(1))
                            .next()
                            .map(|d| d.id);
                    }
                }
                dock::show(&mut self.dock, Pane::Diff, &self.settings.floats);
            }
            Action::Plugin(index) => self.run_plugin(index, ctx),
            Action::PluginCommand(plugin, command) => self.plugin_command(plugin, command, now),
            Action::PluginsFolder => {
                let opened = self
                    .plugins
                    .prepare()
                    .and_then(|dir| preview::open_folder(&dir).map(|()| dir));
                match opened {
                    Ok(dir) => self.notify(format!("Папка плагинов: {}", dir.display()), now),
                    Err(e) => self.notify(format!("Папка плагинов: {e}"), now),
                }
            }
            Action::PluginsReload => {
                self.plugins.reload();
                let errors = self.plugins.errors.clone();
                for plugin in &self.plugins.wasm {
                    let m = &plugin.manifest;
                    crate::trace::line(format!("[plugin] {} {} ({})", m.name, m.version, m.id), false);
                }
                for error in &errors {
                    crate::trace::line(format!("[plugin] {error}"), true);
                }
                self.notify(
                    if errors.is_empty() {
                        format!("Плагинов загружено: {}", self.plugins.count())
                    } else {
                        format!(
                            "Плагинов загружено: {}, с ошибками: {} (подробности в журнале)",
                            self.plugins.count(),
                            errors.len()
                        )
                    },
                    now,
                );
            }
            Action::Goto(id, line) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    self.active = i;
                    self.focus_editor = true;
                    self.docs[i].goto_line(line);
                }
            }
            Action::NextTab => {
                if !self.docs.is_empty() {
                    self.active = (self.active + 1) % self.docs.len();
                    self.focus_editor = true;
                }
            }
            Action::PrevTab => {
                if !self.docs.is_empty() {
                    self.active = (self.active + self.docs.len() - 1) % self.docs.len();
                    self.focus_editor = true;
                }
            }
        }
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.capturing.is_some() {
            return;
        }
        let keys = crate::keys::Keys::new(&self.settings.keys);
        for (index, shortcut) in keys.bound() {
            let (id, _, _, editing) = crate::keys::COMMANDS[*index];
            if self.console.focused() && editing {
                continue;
            }
            if !ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
                continue;
            }
            if shortcut.modifiers.alt {
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
            }
            actions.push(match id {
                "new" => Action::New,
                "open" => Action::Open,
                "open_folder" => Action::OpenFolder,
                "project" => Action::Toggle(Pane::Project),
                "save" => Action::Save,
                "save_as" => Action::SaveAs,
                "save_all" => Action::SaveAll,
                "close_tab" => Action::CloseTab(self.active),
                "reopen" => Action::Reopen,
                "next_tab" => Action::NextTab,
                "prev_tab" => Action::PrevTab,
                "find" => Action::Find,
                "replace" => Action::Replace,
                "find_next" => Action::FindNext,
                "find_prev" => Action::FindPrev,
                "goto" => Action::GotoLine,
                "duplicate_line" => Action::Edit("duplicate_line"),
                "delete_line" => Action::Edit("delete_line"),
                "line_up" => Action::Edit("line_up"),
                "line_down" => Action::Edit("line_down"),
                "upper" => Action::Edit("upper"),
                "lower" => Action::Edit("lower"),
                "trim" => Action::Edit("trim"),
                "bookmark" => Action::Edit("bookmark"),
                "bookmark_next" => Action::Edit("bookmark_next"),
                "bookmark_prev" => Action::Edit("bookmark_prev"),
                "fold" => Action::Fold(0),
                "unfold" => Action::Fold(1),
                "fold_all" => Action::Fold(2),
                "unfold_all" => Action::Fold(3),
                "wrap" => Action::ToggleWrap,
                "theme" => Action::ToggleTheme,
                "settings" => Action::Show(Pane::Settings),
                "hosts" => Action::Toggle(Pane::Hosts),
                "console" => Action::Toggle(Pane::Console),
                "log" => Action::Toggle(Pane::Log),
                "server_search" => Action::Toggle(Pane::ServerSearch),
                "db" => Action::Toggle(Pane::Db),
                "compare" => Action::Compare,
                "find_files" => Action::FindFiles,
                "macro_record" => Action::MacroRecord,
                "macro_play" => Action::MacroPlay,
                _ => Action::Preview,
            });
        }
        for (index, plugin) in self.plugins.list.iter().enumerate() {
            if let Some(shortcut) = crate::keys::parse(&plugin.shortcut)
                && crate::keys::usable(&shortcut)
                && ctx.input_mut(|i| i.consume_shortcut(&shortcut))
            {
                actions.push(Action::Plugin(index));
            }
        }
        for (index, plugin) in self.plugins.wasm.iter().enumerate() {
            for (command, def) in plugin.manifest.commands.iter().enumerate() {
                if let Some(shortcut) = crate::keys::parse(&def.key)
                    && crate::keys::usable(&shortcut)
                    && ctx.input_mut(|i| i.consume_shortcut(&shortcut))
                {
                    actions.push(Action::PluginCommand(index, command));
                }
            }
        }
        if self.search.open
            && self.goto.is_none()
            && self.confirm.is_none()
            && !self.complete.open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            actions.push(Action::CloseSearch);
        }
    }
}
