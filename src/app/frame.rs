use super::*;
use crate::files::ProjectEvent;

impl eframe::App for TrorApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::system::instance::release();
        if let Some(path) = self.update.setup.take() {
            let _ = crate::system::update::run_setup(&path);
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        self.apply_look(&ctx);
        let pal = self.palette();
        let mut actions = Vec::new();
        self.poll_instance(&ctx);
        let scale = ctx.input(|i| i.viewport().native_pixels_per_point.unwrap_or(0.0));
        if scale != self.scale {
            if self.scale > 0.0 {
                let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
                crate::system::trace::line(
                    tf!(
                        "[UI] масштаб экрана {} → {scale}, окно {size:?}",
                        "[UI] screen scale {} → {scale}, window {size:?}",
                        self.scale
                    ),
                    false,
                );
            }
            self.scale = scale;
        }

        if ctx.input(|i| i.viewport().close_requested())
            && self.settings.keep_session
            && !self.allow_close
            && self
                .docs
                .iter()
                .all(|d| !d.dirty() || crate::text::recover::fits(d))
            && let Some(dir) = crate::text::recover::dir()
        {
            let kept = self.recovery.snapshot(&dir, &self.docs);
            if let Err(e) = kept {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.settings.end_session();
                let now = ctx.input(|i| i.time);
                self.notify(
                    tf!(
                        "Не удалось сохранить правки сеанса: {e}",
                        "Could not save session edits: {e}"
                    ),
                    now,
                );
            }
            self.recovery.stop();
        } else if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.docs.iter().any(|d| d.dirty())
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        } else if ctx.input(|i| i.viewport().close_requested())
            && let Some(dir) = crate::text::recover::dir()
        {
            crate::text::recover::clear(&dir);
            self.recovery = Default::default();
            self.recovery.stop();
        }
        let labels: Vec<Option<String>> = self.remotes.iter().map(|r| r.peer_label()).collect();
        for (index, panel) in self.remotes.iter_mut().enumerate() {
            let pane = Pane::hosts(index);
            let shown = dock::has(&self.dock, pane) || self.settings.is_detached(pane);
            panel.arrange(shown, self.settings.remote_tree, &labels, index);
        }
        self.project.arrange(
            self.settings.project_tree,
            &labels,
            self.settings.project_size,
            self.settings.project_date,
        );
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        let drop_pos = || {
            owner::cursor()
                .map(|(x, y)| egui::pos2(x, y) / ctx.pixels_per_point())
                .or_else(|| ctx.input(|i| i.pointer.latest_pos()))
        };
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let target = if hovering || !dropped.is_empty() {
            drop_pos().and_then(|pos| {
                self.remotes
                    .iter()
                    .enumerate()
                    .find_map(|(i, r)| Some((i, r.drop_target(pos)?)))
            })
        } else {
            None
        };
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        let released = ctx.input(|i| i.pointer.any_released());
        let held = owner::button_down().unwrap_or_else(|| ctx.input(|i| i.pointer.any_down()));
        let local_drag = self.project.dragging().is_some() || self.project2.dragging().is_some();
        let inner = if local_drag {
            pointer.and_then(|pos| {
                self.remotes
                    .iter()
                    .enumerate()
                    .find_map(|(i, r)| Some((i, r.drop_target(pos)?)))
            })
        } else {
            None
        };
        for (i, panel) in self.remotes.iter_mut().enumerate() {
            let hovered = target
                .as_ref()
                .filter(|(t, _)| hovering && *t == i)
                .or(inner.as_ref().filter(|(t, _)| *t == i));
            panel.hover_drop(hovered, i);
        }
        let outside = window_left(&ctx);
        if outside {
            self.project.start_drag_out();
            self.project2.start_drag_out();
        }
        self.project.clear_hover();
        self.project2.clear_hover();
        for index in 0..self.remotes.len() {
            let Some(items) = self.remotes[index].dragging(&ctx) else {
                continue;
            };
            if outside {
                if cfg!(any(target_os = "windows", target_os = "macos")) {
                    self.remotes[index].prepare_drag_out(items);
                }
                continue;
            }
            let Some(pos) = pointer else {
                continue;
            };
            for second in [false, true] {
                let panel = if second {
                    &mut self.project2
                } else {
                    &mut self.project
                };
                let Some(dir) = panel.drop_target(pos) else {
                    continue;
                };
                if released {
                    let now = ctx.input(|i| i.time);
                    self.remotes[index].download_into(items.clone(), &dir);
                    self.notify(
                        tf!("Скачивание в {}", "Downloading to {}", dir.display()),
                        now,
                    );
                } else {
                    panel.mark_hover(pos);
                }
                break;
            }
        }
        for panel in &mut self.remotes {
            if let Some(paths) = panel.finish_drag_out(held) {
                let done = panel.drag_done.clone();
                if held {
                    let ctx = ctx.clone();
                    drag_out(frame, paths, move |dropped| {
                        *done.lock().unwrap_or_else(|e| e.into_inner()) = Some(dropped);
                        ctx.request_repaint();
                    });
                } else {
                    *done.lock().unwrap_or_else(|e| e.into_inner()) = Some(false);
                }
            }
        }
        if hovering {
            ctx.request_repaint();
        }
        self.drop_hint = hovering.then(|| target.as_ref().map(|(i, dir)| (*i, dir.clone())));
        if inner.is_some() {
            self.drop_hint = Some(inner.clone());
            ctx.request_repaint();
        }
        if !dropped.is_empty() {
            if let Some((i, dir)) = target {
                self.remotes[i].upload_local_to(dropped, &dir);
            } else if dropped.len() == 1 && dropped[0].is_dir() {
                self.project.set_root(dropped[0].clone());
                actions.push(Action::Show(Pane::Project));
            } else {
                for path in dropped {
                    actions.push(Action::OpenPath(path));
                }
            }
        }
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let (zoom, reset, wheel) = ctx.input_mut(|i| {
            let reset = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0);
            let mut step = 0.0;
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Plus)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Equals)
            {
                step += 1.0;
            }
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Minus) {
                step -= 1.0;
            }
            (step, reset, i.zoom_delta())
        });
        let (mut zoom, reset, wheel) = (zoom, reset, wheel);
        if wheel != 1.0 && wheel > 0.0 {
            self.zoom_acc += wheel.ln();
        }
        while self.zoom_acc >= 0.19 {
            self.zoom_acc -= 0.2;
            zoom += 1.0;
        }
        while self.zoom_acc <= -0.19 {
            self.zoom_acc += 0.2;
            zoom -= 1.0;
        }
        if reset {
            self.settings.reset_zoom();
            let now = ctx.input(|i| i.time);
            self.notify(tr!("Размер шрифта редактора по умолчанию").to_owned(), now);
        }
        if zoom != 0.0 {
            let size = self.settings.zoom(zoom);
            let text = tf!(
                "Размер шрифта редактора: {}",
                "Editor font size: {}",
                size as i32
            );
            let now = ctx.input(|i| i.time);
            self.notify(text, now);
        }
        self.check_missing(&ctx);
        let modal_open = self.modal_open();
        self.log.extend(crate::system::trace::take());
        if self.log.len() > 5000 {
            self.log.drain(..1000);
        }
        let now = ctx.input(|i| i.time);
        for (text, ok) in crate::system::trace::take_popups() {
            self.popups.push((text, ok, now));
        }
        self.popups.retain(|(_, _, at)| now - at < 6.0);
        for index in 0..self.remotes.len() {
            let events = self.remotes[index].poll();
            self.remote_events(&ctx, index, events);
        }
        self.db.poll();
        self.plugin_results(now);
        self.plugin_finished(now);
        self.plugin_watch(now);
        self.console.follow(self.project.root());
        for note in self.console.poll() {
            self.notify(note, now);
        }
        for second in [false, true] {
            let events: Vec<ProjectEvent> = if second {
                self.project2.events().collect()
            } else {
                self.project.events().collect()
            };
            for event in events {
                self.project_event(event, second, now, &mut actions);
            }
        }
        if self.project.root().is_none() {
            self.settings.forget_project(false);
        }
        if self.project2.root().is_none() {
            self.settings.forget_project(true);
        }
        self.settings.remember_widths();
        self.settings.apply_editing();
        crate::console::terminal::SCROLLBACK.store(
            self.settings.scrollback as usize,
            std::sync::atomic::Ordering::Relaxed,
        );
        crate::db::MAX_ROWS.store(
            self.settings.db_rows as usize,
            std::sync::atomic::Ordering::Relaxed,
        );
        if !modal_open {
            self.shortcuts(&ctx, &mut actions);
        }
        let now = ctx.input(|i| i.time);
        let unsaved: Vec<PathBuf> = self
            .docs
            .iter()
            .filter(|d| d.dirty())
            .filter_map(|d| d.path.clone())
            .collect();
        self.find_files.note_unsaved(&unsaved);
        let found = self.find_files.poll().or(self.find_out.take());
        match found {
            Some(crate::remote::findfiles::Out::Open(path, line)) => {
                self.open_path(&path, now);
                if let Some(doc) = self.docs.get_mut(self.active)
                    && doc.path.as_deref() == Some(path.as_path())
                {
                    doc.goto_line(line);
                }
            }
            Some(crate::remote::findfiles::Out::OpenDoc(id, line)) => {
                if let Some(index) = self.docs.iter().position(|d| d.id == id) {
                    self.active = index;
                    self.settings.show(&mut self.dock, Pane::Doc(id));
                    self.docs[index].goto_line(line);
                }
            }
            Some(crate::remote::findfiles::Out::ReplaceTabs(re, with, literal)) => {
                let mut total = 0;
                for doc in &mut self.docs {
                    let text = doc.text();
                    let count = re.find_iter(&text).filter(|m| !m.is_empty()).count();
                    if count == 0 {
                        continue;
                    }
                    let changed = if literal {
                        re.replace_all(&text, regex::NoExpand(&with))
                    } else {
                        re.replace_all(&text, with.as_str())
                    };
                    let end = doc.len();
                    doc.replace(0..end, &changed, crate::text::document::EditKind::Other);
                    total += count;
                }
                self.notify(
                    tf!("Заменено во вкладках: {total}", "Replaced in tabs: {total}"),
                    now,
                );
            }
            Some(crate::remote::findfiles::Out::Changed) => {
                for doc in self
                    .docs
                    .iter_mut()
                    .filter(|d| !d.dirty() && d.has_path())
                {
                    let _ = doc.reload();
                }
            }
            None => {}
        }
        self.update_ui(&ctx, &pal, now);
        if let Some(paths) = self
            .project
            .take_drag_out()
            .or_else(|| self.project2.take_drag_out())
        {
            drag_out(frame, paths, |_| {});
        }
        if self.settings.autosave && self.recovery.due(now) {
            if let Some(dir) = crate::text::recover::dir() {
                let _ = self.recovery.snapshot(&dir, &self.docs);
            }
            if !unsaved.is_empty() || self.docs.iter().any(|d| d.dirty()) {
                ctx.request_repaint_after(std::time::Duration::from_secs(11));
            }
        }

        let bar = Frame::new().fill(pal.mantle);
        Panel::top("tror_menu")
            .frame(bar.inner_margin(Margin {
                left: 10,
                right: 8,
                top: 5,
                bottom: 3,
            }))
            .show_separator_line(true)
            .show(ui, |ui| self.menu(ui, &pal, &mut actions));
        Panel::bottom("tror_status")
            .frame(bar.inner_margin(Margin::symmetric(10, 4)))
            .show_separator_line(true)
            .show(ui, |ui| self.status_bar(ui, &pal));

        let docs = &self.docs;
        self.settings.keep_detached(|pane| match pane {
            Pane::Doc(id) => docs.iter().any(|d| d.id == *id),
            _ => true,
        });
        let ids: Vec<u64> = self
            .docs
            .iter()
            .map(|d| d.id)
            .filter(|id| !self.settings.is_detached(Pane::Doc(*id)))
            .collect();
        dock::sync_with(&mut self.dock, &ids, self.docs.is_empty());
        if self.focus_editor
            && let Some(doc) = self.docs.get(self.active)
            && let Some(path) = self.dock.find_tab(&Pane::Doc(doc.id))
        {
            let _ = self.dock.focus_tab(path);
        }
        self.refresh_tab_orders();
        let mut dock = std::mem::replace(&mut self.dock, DockState::new(Vec::new()));
        dock::keep_sizes(&mut dock, &mut self.sizes, false);
        let style = dock::style(&pal, ui.style(), self.settings.look.radius);
        self.console_shown = false;
        self.settings_shown = false;
        CentralPanel::no_frame().show(ui, |ui| {
            DockArea::new(&mut dock)
                .painter(&dock::TAB_PAINTER)
                .style(style)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .leaf_min_size(&|tabs: &[Pane]| dock::leaf_min(tabs) - egui::vec2(0.0, 34.0))
                .leaf_rigid_size(&dock::leaf_rigid)
                .show_inside(ui, self);
        });
        actions.append(&mut self.queued);
        for panel in &mut self.remotes {
            panel.adopt_window(owner::adopt);
        }
        if let Some((_, Pane::Doc(id))) = dock.find_active_focused()
            && let Some(index) = self.docs.iter().position(|d| d.id == *id)
            && index != self.active
        {
            self.active = index;
            self.complete.close();
        }
        let pinned: Vec<u64> = self
            .docs
            .iter()
            .filter(|d| d.pinned)
            .map(|d| d.id)
            .collect();
        dock::pins_first(&mut dock, &pinned);
        let pressed = ctx.input(|i| i.pointer.any_down());
        dock::keep_sizes(&mut dock, &mut self.sizes, pressed);
        if std::mem::take(&mut self.sizes.changed) {
            self.settings.remember_sizes(&self.sizes);
        }
        if !pressed {
            dock::enforce_min(&mut dock);
            let origin = ctx
                .input(|i| i.viewport().inner_rect)
                .map_or(egui::Pos2::ZERO, |r| r.min);
            let floating: Vec<(Pane, egui::Rect)> = dock
                .iter_all_tabs()
                .filter(|(path, _)| path.surface != egui_stack::SurfaceIndex::main())
                .filter_map(|(path, tab)| {
                    let rect = dock.get_window_state(path.surface)?.rect();
                    Some((*tab, rect))
                })
                .collect();
            for (pane, rect) in floating {
                dock::hide(&mut dock, pane);
                let min = origin + rect.min.to_vec2();
                let placed = (rect.is_finite() && rect.width() > 40.0)
                    .then_some([min.x, min.y, rect.width(), rect.height()]);
                self.settings.detach(pane, placed);
            }
        }
        dock::remember_homes(&dock);
        self.settings.remember_homes();
        self.dock = dock;
        if ctx.input(|i| i.viewport().focused) == Some(true) {
            owner::remember_main();
        }
        let detached = self.settings.detached_panes();
        self.floats.prune(&detached);
        for pane in detached {
            let title = match pane {
                Pane::Doc(id) => self
                    .docs
                    .iter()
                    .find(|d| d.id == id)
                    .map(|d| format!("{} — TrorPad", d.title()))
                    .unwrap_or_default(),
                _ => format!("{} — TrorPad", pane.title()),
            };
            let mut closed = false;
            let mut hidden = false;
            let (min_w, min_h) = pane.min_size();
            let mut builder = egui::ViewportBuilder::default()
                .with_title(title.clone())
                .with_decorations(false)
                .with_taskbar(false)
                .with_resizable(true)
                .with_inner_size([860.0, 580.0])
                .with_min_inner_size([min_w + 16.0, min_h + 48.0]);
            if !self.floats.spawn.contains_key(&pane)
                && let Some(rect) = self.settings.float_of(pane)
            {
                self.floats.spawn.insert(pane, rect);
            }
            if let Some([x, y, w, h]) = self.floats.spawn.get(&pane) {
                builder = builder
                    .with_position([*x, *y])
                    .with_inner_size([w.max(min_w + 16.0), h.max(min_h + 16.0)]);
            }
            let mut placed = None;
            let owned = self.floats.owned.contains(&pane);
            let title_text = title.clone();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("tror_window", pane)),
                builder,
                |ui, _class| {
                    closed = ui.input(|i| i.viewport().close_requested());
                    if !owned && owner::adopt(&title_text) {
                        self.floats.owned.insert(pane);
                    }
                    placed = ui.input(|i| {
                        let outer = i.viewport().outer_rect?;
                        let inner = i.viewport().inner_rect?;
                        Some([outer.min.x, outer.min.y, inner.width(), inner.height()])
                    });
                    if !modal_open {
                        self.shortcuts(ui.ctx(), &mut actions);
                    }
                    let ctx = ui.ctx().clone();
                    CentralPanel::default()
                        .frame(
                            Frame::new()
                                .fill(pal.mantle)
                                .stroke(Stroke::new(1.0, pal.border))
                                .inner_margin(Margin::same(1)),
                        )
                        .show(ui, |ui| {
                            let full = ui.max_rect();
                            let (bar, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 30.0),
                                Sense::hover(),
                            );
                            ui.painter().rect_filled(bar, 0.0, pal.crust);
                            let drag = ui.interact(
                                bar,
                                ui.scope_id().with("tror_float_bar"),
                                Sense::click_and_drag(),
                            );
                            if drag.drag_started() {
                                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                            }
                            if drag.double_clicked() {
                                closed = true;
                            }
                            ui.painter().text(
                                egui::pos2(bar.left() + 10.0, bar.center().y),
                                egui::Align2::LEFT_CENTER,
                                pane.title(),
                                FontId::proportional(13.0),
                                pal.text,
                            );
                            let mut buttons = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(bar.shrink2(egui::vec2(4.0, 3.0)))
                                    .layout(Layout::right_to_left(Align::Center)),
                            );
                            if !matches!(pane, Pane::Doc(_))
                                && icon_button(&mut buttons, &pal, Icon::Close)
                                    .on_hover_text(tr!("Скрыть панель"))
                                    .clicked()
                            {
                                hidden = true;
                            }
                            if icon_button(&mut buttons, &pal, Icon::Dock)
                                .on_hover_text(tr!(
                                    "Вернуть в окно TrorPad (двойной щелчок по заголовку)"
                                ))
                                .clicked()
                            {
                                closed = true;
                            }
                            let mut tab = pane;
                            TabViewer::ui(self, ui, &mut tab);
                            actions.append(&mut self.queued);
                            for (edge, dir) in [
                                (
                                    Rect::from_min_max(
                                        egui::pos2(full.right() - 6.0, full.top() + 30.0),
                                        egui::pos2(full.right(), full.bottom() - 10.0),
                                    ),
                                    egui::viewport::ResizeDirection::East,
                                ),
                                (
                                    Rect::from_min_max(
                                        egui::pos2(full.left(), full.top() + 30.0),
                                        egui::pos2(full.left() + 6.0, full.bottom() - 10.0),
                                    ),
                                    egui::viewport::ResizeDirection::West,
                                ),
                                (
                                    Rect::from_min_max(
                                        egui::pos2(full.left() + 10.0, full.bottom() - 6.0),
                                        egui::pos2(full.right() - 10.0, full.bottom()),
                                    ),
                                    egui::viewport::ResizeDirection::South,
                                ),
                                (
                                    Rect::from_min_max(
                                        egui::pos2(full.right() - 10.0, full.bottom() - 10.0),
                                        full.max,
                                    ),
                                    egui::viewport::ResizeDirection::SouthEast,
                                ),
                                (
                                    Rect::from_min_max(
                                        egui::pos2(full.left(), full.bottom() - 10.0),
                                        egui::pos2(full.left() + 10.0, full.bottom()),
                                    ),
                                    egui::viewport::ResizeDirection::SouthWest,
                                ),
                            ] {
                                let grip = ui.interact(
                                    edge,
                                    ui.scope_id().with((
                                        "tror_float_edge",
                                        edge.min.x as i32,
                                        edge.min.y as i32,
                                    )),
                                    Sense::drag(),
                                );
                                let icon = match dir {
                                    egui::viewport::ResizeDirection::East
                                    | egui::viewport::ResizeDirection::West => {
                                        egui::CursorIcon::ResizeHorizontal
                                    }
                                    egui::viewport::ResizeDirection::South => {
                                        egui::CursorIcon::ResizeVertical
                                    }
                                    egui::viewport::ResizeDirection::SouthEast => {
                                        egui::CursorIcon::ResizeNwSe
                                    }
                                    _ => egui::CursorIcon::ResizeNeSw,
                                };
                                let grip = grip.on_hover_cursor(icon);
                                if grip.drag_started() {
                                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
                                }
                            }
                        });
                    self.pane_dialogs(pane, ui.ctx(), &pal);
                },
            );
            if let Some(rect) = placed
                && rect.iter().all(|v| v.is_finite())
            {
                self.settings.remember_float(pane, rect);
            }
            if closed {
                actions.push(Action::Attach(pane));
            } else if hidden {
                actions.push(Action::Toggle(pane));
            }
        }
        self.find_window(&ctx, pal, &mut actions);
        self.focus_editor = false;
        if !self.console_shown {
            self.console.unfocus();
        }
        if !self.settings_shown {
            self.capturing = None;
        }

        if let Some(Some((i, dir))) = &self.drop_hint
            && let Some(zone) = self.remotes.get(*i).and_then(|r| r.drop_zone())
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("tror_drop"),
            ));
            painter.rect_stroke(
                zone.shrink(2.0),
                8.0,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
            let label = tf!("Загрузить в {dir}", "Upload to {dir}");
            let galley = painter.layout_no_wrap(label, FontId::proportional(14.0), pal.text);
            let at = egui::pos2(
                zone.center().x - galley.size().x / 2.0,
                zone.bottom() - galley.size().y - 18.0,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(10.0, 6.0)),
                6.0,
                pal.surface,
            );
            painter.galley(at, galley, pal.text);
        } else if self.drop_hint.is_some() {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("tror_drop"),
            ));
            painter.rect_filled(screen, 0.0, pal.crust.gamma_multiply(0.85));
            painter.rect_stroke(
                screen.shrink(14.0),
                14.0,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                tr!("Отпустите, чтобы открыть файл"),
                FontId::proportional(22.0),
                pal.text,
            );
        }

        self.goto_modal(&ctx, &pal);
        self.jump_modal(&ctx, &pal, &mut actions);
        self.plugin_modal(&ctx, &pal);
        self.plugin_manager(&ctx, &pal);
        self.macro_modal(&ctx, &pal);
        self.rename_modal(&ctx, &pal);
        self.confirm_modal(&ctx);
        self.conflict_modal(&ctx);
        let mut panes = self.hosts_panes();
        panes.extend([Pane::Console, Pane::Db, Pane::Project]);
        for pane in panes {
            if !self.settings.is_detached(pane) {
                self.pane_dialogs(pane, &ctx, &pal);
            }
        }
        self.show_popups(&ctx, &pal);

        actions.append(&mut self.queued);
        for action in actions {
            self.run(action, &ctx);
        }
        self.drain_close_queue();
        self.trim_hosts();

        let title = match self.docs.get(self.active) {
            Some(doc) => format!(
                "{}{} — TrorPad",
                if doc.dirty() { "● " } else { "" },
                doc.title()
            ),
            None => "TrorPad".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if let Some(perf) = &mut self.perf {
            perf.record(started.elapsed());
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.remember_open(&self.docs, self.active);
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        eframe::set_value(storage, DOCK_KEY, &self.dock);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c: Color32 = self.palette().mantle;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }
}

impl TrorApp {
    fn project_event(
        &mut self,
        event: ProjectEvent,
        second: bool,
        now: f64,
        actions: &mut Vec<Action>,
    ) {
        match event {
            ProjectEvent::Open(paths) => {
                for path in paths {
                    actions.push(Action::OpenPath(path));
                }
            }
            ProjectEvent::Upload(paths) => {
                if let Some(panel) = self.remotes.iter_mut().find(|r| r.connected()) {
                    panel.upload_local(paths);
                }
            }
            ProjectEvent::Toast(text) => self.notify(text, now),
            ProjectEvent::Opened(path) => {
                self.settings.remember_project(path, second);
            }
            ProjectEvent::View(tree) => {
                self.settings.view_as_tree(false, tree);
            }
            ProjectEvent::Transfer(paths, keep) => {
                self.move_between_projects(paths, second, keep, now)
            }
            ProjectEvent::Move(paths, dir, keep) => {
                match crate::files::transfer(&paths, &dir, keep) {
                    Ok(moved) => {
                        if !keep {
                            self.relocate(&moved);
                        }
                        self.notify(
                            tf!(
                                "{} {} в {}",
                                "{} {} to {}",
                                if keep {
                                    tr!("Скопировано:")
                                } else {
                                    tr!("Перемещено:")
                                },
                                moved.len(),
                                dir.display()
                            ),
                            now,
                        );
                    }
                    Err(e) => self.notify(tf!("Не удалось: {e}", "Failed: {e}"), now),
                }
                self.project.reload();
                self.project2.reload();
            }
            ProjectEvent::Dropped(paths, pos) => {
                let other = if second {
                    &self.project
                } else {
                    &self.project2
                };
                if other.contains(pos) {
                    self.move_between_projects(paths, second, true, now);
                } else if let Some(panel) = self
                    .remotes
                    .iter_mut()
                    .find(|r| r.connected() && r.area().contains(pos))
                {
                    match panel.drop_target(pos) {
                        Some(dir) => panel.upload_local_to(paths, &dir),
                        None => panel.upload_local(paths),
                    }
                }
            }
        }
    }

    fn move_between_projects(
        &mut self,
        paths: Vec<PathBuf>,
        from_second: bool,
        keep: bool,
        now: f64,
    ) {
        let target = if from_second {
            self.project.target_dir()
        } else {
            self.project2.target_dir()
        };
        let Some(dir) = target else {
            return;
        };
        match crate::files::transfer(&paths, &dir, keep) {
            Ok(moved) => {
                if !keep {
                    self.relocate(&moved);
                }
                let n = moved.len();
                self.notify(
                    tf!(
                        "{} {n} в {}",
                        "{} {n} to {}",
                        if keep {
                            tr!("Скопировано:")
                        } else {
                            tr!("Перемещено:")
                        },
                        dir.display()
                    ),
                    now,
                )
            }
            Err(e) => self.notify(tf!("Не удалось: {e}", "Failed: {e}"), now),
        }
        self.project.reload();
        self.project2.reload();
    }

    fn relocate(&mut self, moved: &[(PathBuf, PathBuf)]) {
        for doc in &mut self.docs {
            doc.move_path(moved);
        }
    }

    fn apply_look(&mut self, ctx: &egui::Context) {
        theme::commit_term_fonts();
        let wanted_fonts = self.console.fonts();
        let fonts_changed = self.term_fonts.as_ref() != Some(&wanted_fonts);
        if self.settings.same_look(self.applied.as_ref()) && !fonts_changed {
            return;
        }
        let look = self.settings.look();
        let old = self.applied.take();
        let fonts = |l: &Look| (l.ui_font.clone(), l.code_font.clone(), l.tab_width);
        if (old.as_ref().map(fonts) != Some(fonts(&look)) || fonts_changed)
            && let Err(e) = theme::install_fonts(
                ctx,
                look.ui_font.as_deref(),
                look.code_font.as_deref(),
                &wanted_fonts,
                look.tab_width,
            )
        {
            self.notify(
                tf!("Шрифт не загружен: {e}", "Font not loaded: {e}"),
                ctx.input(|i| i.time),
            );
        }
        self.term_fonts = Some(wanted_fonts);
        theme::apply(ctx, &look.palette(), look.dark, look.radius, look.ui_size);
        let code = |l: &Look| {
            let (scheme, colors) = l.syntax();
            (scheme.to_owned(), colors.clone())
        };
        if old.as_ref().map(code) != Some(code(&look)) {
            let (scheme, colors) = look.syntax();
            self.syntaxes.set_theme(scheme, colors);
        }
        self.applied = Some(look);
    }
}

fn window_left(ctx: &egui::Context) -> bool {
    let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
    if let (Some((x, y)), Some(size)) = (owner::cursor(), size) {
        let ppp = ctx.pixels_per_point();
        return x < -2.0 || y < -2.0 || x > size.x * ppp + 2.0 || y > size.y * ppp + 2.0;
    }
    ctx.input(|i| i.pointer.any_down())
        && ctx
            .input(|i| i.pointer.latest_pos())
            .is_none_or(|p| !ctx.content_rect().contains(p))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn drag_out(frame: &eframe::Frame, paths: Vec<PathBuf>, done: impl Fn(bool) + Send + 'static) {
    let icon = drag_icon(paths.len());
    if let Err(e) = drag::start_drag(
        frame,
        drag::DragItem::Files(paths),
        drag::Image::Raw(icon),
        move |result, _| done(matches!(result, drag::DragResult::Dropped)),
        drag::Options::default(),
    ) {
        crate::system::trace::popup(
            tf!(
                "Не удалось начать перетаскивание: {e}",
                "Could not start dragging: {e}"
            ),
            false,
        );
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn drag_out(_frame: &eframe::Frame, _paths: Vec<PathBuf>, done: impl Fn(bool) + Send + 'static) {
    done(false);
}

#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
fn drag_icon(count: usize) -> Vec<u8> {
    const W: usize = 52;
    const H: usize = 60;
    let mut pixels = vec![0u8; W * H * 4];
    let mut put = |x: usize, y: usize, c: [u8; 4]| {
        if x < W && y < H {
            pixels[(y * W + x) * 4..(y * W + x) * 4 + 4].copy_from_slice(&c);
        }
    };
    let border = [0x7a, 0x84, 0x99, 0xff];
    let paper = [0xf4, 0xf6, 0xfa, 0xff];
    let fold = [0xd5, 0xda, 0xe4, 0xff];
    let ink = [0xb3, 0xba, 0xc8, 0xff];
    let mut page = |ox: usize, oy: usize, lines: bool| {
        let (w, h, corner) = (40usize, 48usize, 12usize);
        for y in 0..h {
            for x in 0..w {
                let from_right = w - 1 - x;
                if from_right + y < corner - 1 {
                    continue;
                }
                let in_fold = from_right < corner && y < corner;
                let edge = x == 0 || y == h - 1 || from_right == 0 || y == 0;
                let color = if edge
                    || from_right + y == corner - 1
                    || (in_fold && (from_right == corner - 1 || y == corner - 1))
                {
                    border
                } else if in_fold {
                    fold
                } else {
                    paper
                };
                put(ox + x, oy + y, color);
            }
        }
        if lines {
            for (row, len) in [(18, 26), (24, 22), (30, 26), (36, 16)] {
                for x in 7..7 + len {
                    put(ox + x, oy + row, ink);
                }
            }
        }
    };
    if count > 1 {
        page(10, 10, false);
        page(5, 5, false);
    }
    page(0, 0, true);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, W as u32, H as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(&pixels);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn drag_icon_is_small_png() {
        for count in [1, 3] {
            let bytes = super::drag_icon(count);
            assert_eq!(&bytes[1..4], b"PNG");
            let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
            let info = decoder.read_info().unwrap().info().clone();
            assert_eq!((info.width, info.height), (52, 60));
            std::fs::write(format!("/tmp/drag_icon_{count}.png"), &bytes).ok();
        }
    }
}
