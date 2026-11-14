use super::tabs::Viewer;
use super::*;

impl eframe::App for TrorApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(path) = self.update.setup.take() {
            let _ = crate::update::run_setup(&path);
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        self.apply_look(&ctx);
        let pal = self.palette();
        let mut actions = Vec::new();

        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.docs.iter().any(|d| d.dirty())
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        } else if ctx.input(|i| i.viewport().close_requested())
            && let Some(dir) = crate::recover::dir()
        {
            crate::recover::clear(&dir);
            self.recovery = Default::default();
            self.recovery.last = f64::INFINITY;
        }
        let detached = self.settings.detached.clone();
        for (index, panel) in self.remotes.iter_mut().enumerate() {
            let pane = Pane::hosts(index);
            panel.visible = dock::has(&self.dock, pane) || detached.contains(&pane);
            panel.tree = self.settings.remote_tree;
        }
        if let Ok(mut request) = VIEW_REQUEST.lock() {
            if let Some(tree) = request.0.take() {
                self.settings.remote_tree = tree;
            }
            if let Some(tree) = request.1.take() {
                self.settings.project_tree = tree;
            }
        }
        self.project.tree = self.settings.project_tree;
        self.project.upload_target = self.remotes.iter().find_map(|r| r.peer_label());
        crate::complete::set_custom(&self.settings.dictionaries);
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            if let Some(panel) = self.remotes.iter_mut().find(|r| r.can_upload()) {
                panel.offer_drop(dropped);
            } else if dropped.len() == 1 && dropped[0].is_dir() {
                self.project.set_root(dropped[0].clone());
                actions.push(Action::Show(Pane::Project));
            } else {
                for path in dropped {
                    actions.push(Action::OpenPath(path));
                }
            }
        }
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            ctx.set_zoom_factor((ctx.zoom_factor() * zoom).clamp(0.5, 3.0));
        }
        self.check_missing(&ctx);
        let modal_open = self.goto.is_some()
            || self.plugin_dialog.is_some()
            || self.macro_dialog.is_some()
            || self.rename.is_some()
            || self.confirm.is_some()
            || self.remotes.iter().any(|r| r.modal_open())
            || self.console.modal_open()
            || self.project.modal_open()
            || self.db.modal_open();
        if self
            .remotes
            .iter_mut()
            .any(|r| std::mem::take(&mut r.want_search))
        {
            dock::show(&mut self.dock, Pane::ServerSearch, &self.settings.floats);
        }
        self.log.extend(crate::trace::take());
        if self.log.len() > 5000 {
            self.log.drain(..1000);
        }
        let now = ctx.input(|i| i.time);
        for (text, ok) in crate::trace::take_popups() {
            self.popups.push((text, ok, now));
        }
        self.popups.retain(|(_, _, at)| now - at < 6.0);
        for index in 0..self.remotes.len() {
            let events = self.remotes[index].poll();
            self.remote_events(&ctx, events);
        }
        let labels: Vec<Option<String>> = self.remotes.iter().map(|r| r.peer_label()).collect();
        for (index, panel) in self.remotes.iter_mut().enumerate() {
            panel.peers = labels
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .filter_map(|(i, label)| Some((i as u8, label.clone()?)))
                .collect();
        }
        self.transfers();
        self.db.poll();
        self.plugin_results(now);
        self.plugin_finished(now);
        for note in self.console.poll() {
            self.notify(note, now);
        }
        for event in self.project.take_events() {
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
                    self.settings.recent_projects.retain(|p| *p != path);
                    self.settings.recent_projects.insert(0, path.clone());
                    self.settings.recent_projects.truncate(MAX_RECENT);
                    self.settings.project = Some(path);
                }
                ProjectEvent::View(tree) => self.settings.project_tree = tree,
            }
        }
        if self.project.root.is_none() {
            self.settings.project = None;
        }
        self.project.show_size = self.settings.project_size;
        self.project.show_date = self.settings.project_date;
        self.settings.columns = crate::widgets::columns();
        crate::document::set_indent(self.settings.edit.unit());
        crate::terminal::SCROLLBACK.store(
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
        match self
            .find_files
            .ui(&ctx, &pal, self.project.root.as_deref(), &unsaved)
        {
            Some(crate::findfiles::Out::Open(path, line)) => {
                self.open_path(&path, now);
                if let Some(doc) = self.docs.get_mut(self.active)
                    && doc.path.as_deref() == Some(path.as_path())
                {
                    doc.goto_line(line);
                }
            }
            Some(crate::findfiles::Out::Changed) => {
                for doc in self
                    .docs
                    .iter_mut()
                    .filter(|d| !d.dirty() && d.path.is_some())
                {
                    let _ = doc.reopen_as(doc.encoding);
                }
            }
            None => {}
        }
        self.update_ui(&ctx, &pal, now);
        if let Some(paths) = self.project.drag_out.take() {
            drag_out(frame, paths);
        }
        if self.settings.autosave && now - self.recovery.last > 10.0 {
            self.recovery.last = now;
            if let Some(dir) = crate::recover::dir() {
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

        self.settings.detached.retain(|pane| match pane {
            Pane::Doc(id) => self.docs.iter().any(|d| d.id == *id),
            _ => true,
        });
        let ids: Vec<u64> = self
            .docs
            .iter()
            .map(|d| d.id)
            .filter(|id| !self.settings.detached.contains(&Pane::Doc(*id)))
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
        let style = dock::style(&pal, ui.style(), self.settings.look.radius);
        self.console_shown = false;
        self.settings_shown = false;
        CentralPanel::no_frame().show(ui, |ui| {
            let mut viewer = Viewer {
                app: self,
                pal,
                actions: &mut actions,
                modal_open,
            };
            DockArea::new(&mut dock)
                .painter(&dock::TAB_PAINTER)
                .style(style)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .leaf_min_size(&|tabs: &[Pane]| dock::leaf_min(tabs) - egui::vec2(0.0, 34.0))
                .show_inside(ui, &mut viewer);
        });
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
        if !ctx.input(|i| i.pointer.any_down()) {
            dock::enforce_min(&mut dock);
        }
        dock::remember(&dock, &mut self.settings.floats);
        self.dock = dock;
        for pane in self.settings.detached.clone() {
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
            let (min_w, min_h) = pane.min_size();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("tror_window", pane)),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([860.0, 580.0])
                    .with_min_inner_size([min_w + 16.0, min_h + 16.0]),
                |ui, _class| {
                    closed = ui.input(|i| i.viewport().close_requested());
                    if !modal_open {
                        self.shortcuts(ui.ctx(), &mut actions);
                    }
                    CentralPanel::default()
                        .frame(Frame::new().fill(pal.mantle))
                        .show(ui, |ui| {
                            let mut viewer = Viewer {
                                app: self,
                                pal,
                                actions: &mut actions,
                                modal_open,
                            };
                            let mut tab = pane;
                            viewer.ui(ui, &mut tab);
                        });
                    self.pane_dialogs(pane, ui.ctx(), &pal);
                },
            );
            if closed {
                actions.push(Action::Attach(pane));
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

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
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
                "Отпустите, чтобы открыть файл",
                FontId::proportional(22.0),
                pal.text,
            );
        }

        self.goto_modal(&ctx, &pal);
        self.plugin_modal(&ctx, &pal);
        self.macro_modal(&ctx, &pal);
        self.rename_modal(&ctx, &pal);
        self.confirm_modal(&ctx);
        let mut panes = self.hosts_panes();
        panes.extend([Pane::Console, Pane::Db, Pane::Project]);
        for pane in panes {
            if !self.settings.detached.contains(&pane) {
                self.pane_dialogs(pane, &ctx, &pal);
            }
        }
        self.show_popups(&ctx, &pal);

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
        self.settings.open = self
            .docs
            .iter()
            .filter_map(|d| Some((d.id, d.path.clone()?)))
            .collect();
        self.settings.pinned = self
            .docs
            .iter()
            .filter(|d| d.pinned)
            .filter_map(|d| d.path.clone())
            .collect();
        self.settings.active = self.active;
        for doc in &self.docs {
            Self::remember_view(&mut self.settings, doc);
        }
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

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn drag_out(frame: &eframe::Frame, paths: Vec<PathBuf>) {
    let icon = drag_icon(paths.len());
    if let Err(e) = drag::start_drag(
        frame,
        drag::DragItem::Files(paths),
        drag::Image::Raw(icon),
        |_, _| {},
        drag::Options::default(),
    ) {
        crate::trace::popup(format!("Не удалось начать перетаскивание: {e}"), false);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn drag_out(_frame: &eframe::Frame, _paths: Vec<PathBuf>) {}

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
