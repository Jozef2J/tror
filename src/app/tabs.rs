use super::*;

impl TrorApp {
    pub(super) fn doc_color(&self, doc: &Document) -> Option<Color32> {
        let key = doc.location()?;
        self.settings.tab_colors.get(&key).map(|c| tab_color(*c))
    }

    pub(super) fn leaf_docs(&self, id: u64) -> Vec<u64> {
        self.tab_orders
            .iter()
            .find(|order| order.contains(&id))
            .cloned()
            .unwrap_or_else(|| vec![id])
    }

    pub(super) fn refresh_tab_orders(&mut self) {
        self.tab_orders = self
            .dock
            .iter_leaves()
            .map(|(_, leaf)| {
                leaf.tabs()
                    .iter()
                    .filter_map(|t| match t {
                        Pane::Doc(d) => Some(*d),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
    }

    pub(super) fn tab_cmd(&mut self, id: u64, cmd: TabCmd, now: f64) {
        let Some(index) = self.docs.iter().position(|d| d.id == id) else {
            return;
        };
        match cmd {
            TabCmd::Close => self.close_queue.push_back(id),
            TabCmd::CloseOthers | TabCmd::CloseLeft | TabCmd::CloseRight => {
                let order = self.leaf_docs(id);
                let at = order.iter().position(|d| *d == id).unwrap_or(0);
                let ids: Vec<u64> = match cmd {
                    TabCmd::CloseLeft => order[..at].to_vec(),
                    TabCmd::CloseRight => order[at + 1..].to_vec(),
                    _ => order.into_iter().filter(|d| *d != id).collect(),
                };
                let pinned = |id: &u64| self.docs.iter().any(|d| d.id == *id && d.pinned);
                let ids: Vec<u64> = ids.into_iter().filter(|id| !pinned(id)).collect();
                self.close_queue.extend(ids);
            }
            TabCmd::Save => {
                self.save(index, now);
            }
            TabCmd::SaveAs => {
                self.save_as(index, now);
            }
            TabCmd::Rename => self.rename = Some((id, self.docs[index].title())),
            TabCmd::Color(color) => {
                if let Some(key) = self.docs[index].location() {
                    match color {
                        Some(c) => {
                            self.settings.tab_colors.insert(key, c);
                        }
                        None => {
                            self.settings.tab_colors.remove(&key);
                        }
                    }
                }
            }
            TabCmd::Pin => self.docs[index].pinned = !self.docs[index].pinned,
            TabCmd::NewGroup(split) => {
                if let Some(path) = self.dock.find_tab(&Pane::Doc(id)) {
                    let count = self.leaf_docs(id).len();
                    if count > 1 {
                        self.dock.move_tab(
                            path,
                            (path.node_path(), egui_stack::TabInsert::Split(split)),
                        );
                    }
                }
            }
            TabCmd::OtherGroup => {
                let Some(path) = self.dock.find_tab(&Pane::Doc(id)) else {
                    return;
                };
                let target = self
                    .dock
                    .iter_leaves()
                    .find(|(node, leaf)| {
                        *node != path.node_path()
                            && leaf
                                .tabs()
                                .iter()
                                .any(|t| matches!(t, Pane::Doc(_) | Pane::Welcome))
                    })
                    .map(|(node, _)| node);
                match target {
                    Some(node) => self
                        .dock
                        .move_tab(path, (node, egui_stack::TabInsert::Append)),
                    None if self.leaf_docs(id).len() > 1 => self.dock.move_tab(
                        path,
                        (
                            path.node_path(),
                            egui_stack::TabInsert::Split(egui_stack::Split::Right),
                        ),
                    ),
                    None => {}
                }
            }
            TabCmd::Reveal => {
                if let Some(dir) = self.docs[index].path.as_ref().and_then(|p| p.parent()) {
                    let _ = preview::open_folder(dir);
                }
            }
        }
    }

    pub(super) fn drain_close_queue(&mut self) {
        while self.confirm.is_none() {
            let Some(id) = self.close_queue.pop_front() else {
                break;
            };
            if let Some(index) = self.docs.iter().position(|d| d.id == id) {
                self.request_close(index);
            }
        }
    }

    pub(super) fn doc_pane(
        &mut self,
        ui: &mut Ui,
        id: u64,
        pal: &Palette,
        actions: &mut Vec<Action>,
        modal_open: bool,
    ) {
        let Some(index) = self.docs.iter().position(|d| d.id == id) else {
            return;
        };
        let active = index == self.active;
        let pane = ui.available_rect_before_wrap();
        let look = &self.settings.look;
        let (wrap, font_size, line_spacing) =
            (self.settings.wrap, look.font_size, look.line_spacing);
        if active {
            let idle = !self.search.open
                && !self.console.focused()
                && ui.ctx().memory(|m| m.focused().is_none());
            let focus = (std::mem::take(&mut self.focus_editor) || idle) && !modal_open;
            editor::show(
                ui,
                &mut self.docs[index],
                &mut self.search,
                editor::View {
                    syn: &self.syntaxes,
                    pal,
                    focus,
                    wrap,
                    font_size,
                    line_spacing,
                    edit: self.settings.edit,
                    macros: &mut self.macros,
                    complete: &mut self.complete,
                },
            );
            if self.search.open {
                let width = (pane.width() - 44.0).clamp(250.0, 430.0);
                egui::Area::new(egui::Id::unique("tror_find"))
                    .order(egui::Order::Middle)
                    .pivot(egui::Align2::RIGHT_TOP)
                    .fixed_pos(pane.right_top() + egui::vec2(-22.0, 8.0))
                    .show(ui.ctx(), |ui| {
                        Frame::new()
                            .fill(pal.mantle)
                            .stroke(Stroke::new(1.0, pal.border))
                            .corner_radius(self.settings.look.radius.saturating_add(2))
                            .shadow(ui.style().visuals.popup_shadow)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(width);
                                self.search_bar(ui, pal, actions);
                            });
                    });
            }
            return;
        }
        let mut search = SearchState::default();
        let mut complete = Completion::default();
        let mut macros = editor::Macro::default();
        editor::show(
            ui,
            &mut self.docs[index],
            &mut search,
            editor::View {
                syn: &self.syntaxes,
                pal,
                focus: false,
                wrap,
                font_size,
                line_spacing,
                edit: self.settings.edit,
                macros: &mut macros,
                complete: &mut complete,
            },
        );
        if ui.memory(|m| m.has_focus(editor::editor_id(id))) {
            actions.push(Action::Activate(index));
        }
    }

    pub(super) fn log_pane(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                if ui.add(Button::new("Очистить").fill(pal.surface)).clicked() {
                    self.log.clear();
                }
                if ui.add(Button::new("Копировать всё").fill(pal.surface)).clicked() {
                    let text: Vec<&str> = self.log.iter().map(|(t, _)| t.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("Сырой обмен с сервером: команды и ответы FTP, этапы SSH/SFTP, запросы к базе")
                                .small()
                                .color(pal.faint),
                        )
                        .truncate(),
                    );
                });
            });
        });
        ui.add_space(2.0);
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, pal.base);
        let font = FontId::monospace(12.5);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let [server, client, error, info] = self.settings.look.log_colors();
        ui.spacing_mut().item_spacing.y = 0.0;
        ScrollArea::both()
            .id_salt("tror_log")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_h, self.log.len(), |ui, range| {
                for (text, kind) in &self.log[range] {
                    let color = match kind {
                        crate::trace::Kind::Server => server,
                        crate::trace::Kind::Client => client,
                        crate::trace::Kind::Error => error,
                        crate::trace::Kind::Info => info,
                    };
                    let (stamp, body) = text.split_at(text.find("  ").unwrap_or(0));
                    let mut job = egui::text::LayoutJob::default();
                    job.append("  ", 0.0, egui::TextFormat::simple(font.clone(), pal.faint));
                    job.append(
                        stamp,
                        0.0,
                        egui::TextFormat::simple(font.clone(), pal.faint),
                    );
                    job.append(body, 0.0, egui::TextFormat::simple(font.clone(), color));
                    ui.add(egui::Label::new(job).extend());
                }
            });
    }
}

pub(super) struct Viewer<'a> {
    pub(super) app: &'a mut TrorApp,
    pub(super) pal: Palette,
    pub(super) actions: &'a mut Vec<Action>,
    pub(super) modal_open: bool,
}

impl Viewer<'_> {
    fn tab_menu(&mut self, ui: &mut Ui, id: u64) {
        let Some(doc) = self.app.docs.iter().find(|d| d.id == id) else {
            return;
        };
        ui.set_min_width(250.0);
        let keys = crate::keys::Keys::new(&self.app.settings.keys);
        let key = |name: &str| keys.label(name);
        let order = self.app.leaf_docs(id);
        let at = order.iter().position(|d| *d == id).unwrap_or(0);
        let location = doc.location();
        let path = doc.path.clone();
        let remote = doc.remote.clone();
        let saved = path.is_some() || remote.is_some();
        let color = location
            .as_ref()
            .and_then(|k| self.app.settings.tab_colors.get(k))
            .copied();
        let pinned = doc.pinned;
        let mut cmd = None;
        let row = |ui: &mut Ui, text: &str, shortcut: String, enabled: bool| {
            ui.add_enabled(enabled, Button::new(text).shortcut_text(shortcut))
                .clicked()
        };
        if row(ui, "Закрыть", key("close_tab"), true) {
            cmd = Some(TabCmd::Close);
        }
        if row(ui, "Закрыть остальные", String::new(), order.len() > 1) {
            cmd = Some(TabCmd::CloseOthers);
        }
        if row(ui, "Закрыть вкладки слева", String::new(), at > 0) {
            cmd = Some(TabCmd::CloseLeft);
        }
        if row(
            ui,
            "Закрыть вкладки справа",
            String::new(),
            at + 1 < order.len(),
        ) {
            cmd = Some(TabCmd::CloseRight);
        }
        ui.separator();
        let pin = if pinned {
            "Открепить вкладку"
        } else {
            "Закрепить вкладку"
        };
        if row(ui, pin, String::new(), true) {
            cmd = Some(TabCmd::Pin);
        }
        ui.separator();
        if row(ui, "Сохранить", key("save"), true) {
            cmd = Some(TabCmd::Save);
        }
        if row(ui, "Сохранить как…", key("save_as"), true) {
            cmd = Some(TabCmd::SaveAs);
        }
        if row(ui, "Переименовать…", String::new(), saved) {
            cmd = Some(TabCmd::Rename);
        }
        ui.separator();
        let full = match (&remote, &path) {
            (Some((_, p)), _) => Some(p.clone()),
            (None, Some(p)) => Some(p.display().to_string()),
            _ => None,
        };
        let name = doc.title();
        let dir = match (&remote, &path) {
            (Some((_, p)), _) => Some(crate::remote::parent(p)),
            (None, Some(p)) => p.parent().map(|d| d.display().to_string()),
            _ => None,
        };
        if row(ui, "Копировать полный путь", String::new(), full.is_some())
            && let Some(text) = &full
        {
            ui.ctx().copy_text(text.clone());
        }
        if row(ui, "Копировать имя файла", String::new(), true) {
            ui.ctx().copy_text(name.clone());
        }
        if row(ui, "Копировать путь к папке", String::new(), dir.is_some())
            && let Some(text) = &dir
        {
            ui.ctx().copy_text(text.clone());
        }
        if path.is_some() && remote.is_none() && row(ui, "Показать в папке", String::new(), true)
        {
            cmd = Some(TabCmd::Reveal);
        }
        ui.separator();
        ui.add_enabled_ui(location.is_some(), |ui| {
            ui.menu_button("Цвет вкладки", |ui| {
                ui.set_min_width(170.0);
                for (i, (label, _)) in TAB_COLORS.iter().enumerate() {
                    let swatch = tab_color(i as u8);
                    let response =
                        ui.selectable_label(color == Some(i as u8), format!("     {label}"));
                    let r = response.rect;
                    ui.painter().circle_filled(
                        egui::pos2(r.left() + 12.0, r.center().y),
                        5.5,
                        swatch,
                    );
                    if response.clicked() {
                        cmd = Some(TabCmd::Color(Some(i as u8)));
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(color.is_some(), Button::new("Без цвета"))
                    .clicked()
                {
                    cmd = Some(TabCmd::Color(None));
                }
            });
        });
        ui.menu_button("Область вкладок", |ui| {
            ui.set_min_width(230.0);
            let alone = order.len() < 2;
            if ui
                .add_enabled(!alone, Button::new("Новая область справа"))
                .clicked()
            {
                cmd = Some(TabCmd::NewGroup(egui_stack::Split::Right));
            }
            if ui
                .add_enabled(!alone, Button::new("Новая область снизу"))
                .clicked()
            {
                cmd = Some(TabCmd::NewGroup(egui_stack::Split::Below));
            }
            if ui.button("Переместить в другую область").clicked() {
                cmd = Some(TabCmd::OtherGroup);
            }
        });
        if let Some(cmd) = cmd {
            self.actions.push(Action::TabCmd(id, cmd));
            ui.close();
        }
    }
}

impl TabViewer for Viewer<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::unique(*tab)
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        dock::set_mark(dock::Mark::default());
        let Pane::Doc(id) = *tab else {
            return tab.title().into();
        };
        let Some(index) = self.app.docs.iter().position(|d| d.id == id) else {
            return "".into();
        };
        let doc = &self.app.docs[index];
        let pal = &self.pal;
        let active = index == self.app.active;
        dock::set_mark(dock::Mark {
            dirty: doc.dirty(),
            pinned: doc.pinned,
            accent: pal.accent,
        });
        let color = if doc.missing {
            pal.warn
        } else if active {
            pal.text
        } else {
            pal.subtext
        };
        let family = if active {
            theme::medium()
        } else {
            egui::FontFamily::Proportional
        };
        let size = self.app.settings.look.ui_size - 0.5;
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &doc.title(),
            0.0,
            egui::TextFormat {
                font_id: FontId::new(size, family),
                color,
                valign: Align::Center,
                strikethrough: if doc.missing {
                    Stroke::new(1.0, pal.warn)
                } else {
                    Stroke::NONE
                },
                ..Default::default()
            },
        );
        job.into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Pane) {
        let pal = self.pal;
        let app = &mut *self.app;
        let look = &app.settings.look;
        let (dark, font_size) = (look.dark, look.font_size);
        let side = Frame::new().inner_margin(Margin::symmetric(8, 0));
        let pane = *tab;
        let actions = &mut *self.actions;
        let modal_open = self.modal_open;
        let mut body = |ui: &mut Ui| match pane {
            Pane::Welcome => app.welcome(ui, &pal, actions),
            Pane::Doc(id) => app.doc_pane(ui, id, &pal, actions, modal_open),
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) => {
                let index = pane.host_index().unwrap_or(0);
                if let Some(panel) = app.remotes.get_mut(index) {
                    side.show(ui, |ui| panel.ui(ui, &pal));
                }
            }
            Pane::Project => {
                let recent = app.settings.recent_projects.clone();
                app.project.show_size = app.settings.project_size;
                app.project.show_date = app.settings.project_date;
                side.show(ui, |ui| app.project.ui(ui, &pal, &recent));
                app.settings.project_size = app.project.show_size;
                app.settings.project_date = app.project.show_date;
            }
            Pane::Console => {
                app.console_shown = true;
                side.show(ui, |ui| app.console.ui(ui, &pal, dark, font_size));
            }
            Pane::Log => app.log_pane(ui, &pal),
            Pane::ServerSearch => {
                side.show(ui, |ui| app.remotes[0].search_ui(ui, &pal));
            }
            Pane::Settings => {
                app.settings_shown = true;
                let note = prefs::ui(ui, &pal, &mut app.settings, &mut app.capturing);
                if let Some(text) = note {
                    let now = ui.input(|i| i.time);
                    app.notify(text, now);
                }
            }
            Pane::Diff => {
                if let Some((id, line)) = app.diff.ui(ui, &pal, &app.docs, dark, font_size) {
                    actions.push(Action::Goto(id, line));
                }
            }
            Pane::Db => app.db.ui(ui, &pal, &mut app.remotes[0].store, font_size),
            Pane::Preview => {}
        };
        let (min_w, min_h) = pane.min_size();
        let room = ui.available_size();
        let fixed = matches!(pane, Pane::Doc(_) | Pane::Welcome | Pane::Preview);
        if !fixed && (room.x + 0.5 < min_w || room.y + 0.5 < min_h) {
            let size = egui::vec2(room.x.max(min_w), room.y.max(min_h));
            ScrollArea::both()
                .id_salt(("tror_narrow", pane))
                .auto_shrink(false)
                .show(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let mut inner = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    inner.set_clip_rect(inner.clip_rect().intersect(ui.clip_rect()));
                    body(&mut inner);
                });
        } else {
            body(ui);
        }
    }

    fn context_menu(&mut self, ui: &mut Ui, tab: &mut Pane, _path: egui_stack::TabPath) {
        if let Pane::Doc(id) = *tab {
            self.tab_menu(ui, id);
            ui.separator();
        }
        if !matches!(*tab, Pane::Welcome | Pane::Preview)
            && ui.button("В отдельное окно системы").clicked()
        {
            self.actions.push(Action::Detach(*tab));
            ui.close();
        }
    }

    fn on_tab_button(&mut self, tab: &mut Pane, response: &egui::Response) {
        let Pane::Doc(id) = *tab else {
            return;
        };
        let Some(index) = self.app.docs.iter().position(|d| d.id == id) else {
            return;
        };
        if response.middle_clicked() && !self.app.docs[index].pinned {
            self.actions.push(Action::CloseTab(index));
        }
        if let Some(location) = self.app.docs[index].location() {
            response.clone().on_hover_text(location);
        }
    }

    fn on_close(&mut self, tab: &mut Pane) -> egui_stack::tab_viewer::OnCloseResponse {
        if let Pane::Doc(id) = *tab {
            if let Some(index) = self.app.docs.iter().position(|d| d.id == id) {
                if self.app.docs[index].pinned {
                    self.actions.push(Action::TabCmd(id, TabCmd::Pin));
                } else {
                    self.actions.push(Action::CloseTab(index));
                }
            }
            return egui_stack::tab_viewer::OnCloseResponse::Ignore;
        }
        egui_stack::tab_viewer::OnCloseResponse::Close
    }

    fn tab_style_override(
        &self,
        tab: &Pane,
        global: &egui_stack::TabStyle,
    ) -> Option<egui_stack::TabStyle> {
        let Pane::Doc(id) = *tab else {
            return None;
        };
        let doc = self.app.docs.iter().find(|d| d.id == id)?;
        let color = self.app.doc_color(doc)?;
        let mut style = global.clone();
        let mix = |base: Color32, amount: f32| theme::blend(base, color, amount);
        style.inactive.bg_fill = mix(self.pal.mantle, 0.16);
        style.hovered.bg_fill = mix(self.pal.crust, 0.24);
        style.inactive_with_kb_focus.bg_fill = style.inactive.bg_fill;
        for state in [
            &mut style.active,
            &mut style.focused,
            &mut style.active_with_kb_focus,
            &mut style.focused_with_kb_focus,
        ] {
            state.bg_fill = mix(state.bg_fill, 0.22);
            state.outline_color = mix(state.outline_color, 0.5);
        }
        Some(style)
    }

    fn tab_group(&self, tab: &Pane) -> u8 {
        match tab {
            Pane::Doc(id) => self.app.docs.iter().any(|d| d.id == *id && d.pinned) as u8,
            _ => 0,
        }
    }

    fn close_in_tab_menu(&self, tab: &Pane) -> bool {
        !matches!(tab, Pane::Doc(_))
    }

    fn is_closeable(&self, tab: &Pane) -> bool {
        *tab != Pane::Welcome
    }

    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] {
        [false, false]
    }
}
