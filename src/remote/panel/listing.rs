use super::*;

impl RemotePanel {
    pub(super) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.cwd != "/" {
            rows.push(Row {
                path: remote::parent(&self.cwd),
                name: "..".into(),
                depth: 0,
                entry: None,
            });
        }
        let sorted = |entries: &[Entry]| {
            let mut list = entries.to_vec();
            self.sort.apply(&mut list, |e| {
                (e.is_dir, e.name.as_str(), e.size, e.perm.unwrap_or(0), e.mtime.unwrap_or(0))
            });
            list
        };
        if !self.tree {
            for entry in sorted(&self.entries) {
                rows.push(Row {
                    path: remote::join(&self.cwd, &entry.name),
                    name: entry.name.clone(),
                    depth: 0,
                    entry: Some(entry.clone()),
                });
            }
            return rows;
        }
        if self.children.contains_key(&self.cwd) {
            self.walk(&self.cwd, 0, &mut rows);
        } else {
            for entry in sorted(&self.entries) {
                rows.push(Row {
                    path: remote::join(&self.cwd, &entry.name),
                    name: entry.name.clone(),
                    depth: 0,
                    entry: Some(entry.clone()),
                });
            }
        }
        rows
    }

    fn walk(&self, dir: &str, depth: usize, rows: &mut Vec<Row>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        let mut entries = entries.clone();
        self.sort.apply(&mut entries, |e| {
            (e.is_dir, e.name.as_str(), e.size, e.perm.unwrap_or(0), e.mtime.unwrap_or(0))
        });
        for entry in &entries {
            let path = remote::join(dir, &entry.name);
            let open = entry.is_dir && self.expanded.contains(&path);
            rows.push(Row {
                path: path.clone(),
                name: entry.name.clone(),
                depth,
                entry: Some(entry.clone()),
            });
            if open && depth < 40 {
                self.walk(&path, depth + 1, rows);
            }
        }
    }

    pub(super) fn selection_paths(&self, rows: &[Row], clicked: &str) -> Vec<(String, bool)> {
        if self.marked.contains(clicked) {
            rows.iter()
                .filter(|r| r.entry.is_some() && self.marked.contains(&r.path))
                .map(|r| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir)))
                .collect()
        } else {
            rows.iter()
                .filter(|r| r.entry.is_some() && r.path == clicked)
                .map(|r| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir)))
                .collect()
        }
    }

    pub(super) fn listing(&mut self, ui: &mut Ui, pal: &Palette) {
        let row_h = 22.0;
        let rows = self.rows();
        let total = rows.len();
        let show_size = !self.store.hide_size;
        let show_date = self.store.show_date;
        let show_perm = self.store.show_perm;
        let mut go = None;
        let mut download: Vec<String> = Vec::new();
        let mut fetch: Option<(Vec<(String, bool)>, PathBuf)> = None;
        let mut fetch_as: Option<(String, PathBuf)> = None;
        let mut moves: Vec<(String, String)> = Vec::new();
        let mut walk: Vec<String> = Vec::new();
        let mut toggle: Option<String> = None;
        let mut attrs: Option<Vec<(String, bool)>> = None;
        let waiting = self
            .transfers
            .iter()
            .any(|t| !matches!(t.cmd, Cmd::Touch { .. }));
        let errors = if self.failed.is_empty() {
            0.0
        } else {
            30.0 + 18.0 * (self.failed.len().min(4) + usize::from(self.failed.len() > 4)) as f32
        };
        let reserve = if self.busy.is_some() { 34.0 } else { 0.0 }
            + if waiting { 34.0 } else { 0.0 }
            + errors;
        let tree = self.tree;
        let widths = crate::widgets::column_header(ui, pal, [show_size, show_perm, show_date], &mut self.sort);
        ui.spacing_mut().item_spacing.y = 0.0;
        let outer = ui.available_rect_before_wrap();
        let height = (ui.available_height() - reserve).max(40.0);
        let blank = ui.interact(
            Rect::from_min_size(outer.min, egui::vec2(outer.width(), height)),
            ui.scope_id().with(("tror_remote_blank", self.id)),
            Sense::click(),
        );
        let root = ui.ctx().viewport_id() == egui::ViewportId::ROOT;
        self.drop_zone = root.then_some(blank.rect);
        self.drop_dirs.clear();
        let mut dirs = Vec::new();
        let wide = crate::widgets::rows_width(
            ui,
            rows.iter().map(|r| {
                (r.depth as f32 * 14.0 + if tree { 14.0 } else { 0.0 }, r.name.as_str())
            }),
            widths,
        );
        let area = ScrollArea::both()
            .id_salt(("tror_remote_rows", self.id))
            .auto_shrink(false)
            .max_height((ui.available_height() - reserve).max(40.0))
            .show_rows(ui, row_h, total, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let view = ui.clip_rect();
                for index in range {
                    let row = &rows[index];
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(wide, row_h),
                        Sense::click_and_drag(),
                    );
                    let entry = row.entry.clone();
                    let name = row.name.clone();
                    let path = row.path.clone();
                    let is_dir = entry.as_ref().is_none_or(|e| e.is_dir);
                    let marked = entry.is_some() && self.marked.contains(&path);
                    let drop_target = is_dir
                        && response
                            .dnd_hover_payload::<Dragged>()
                            .is_some_and(|dragged| {
                                dragged.panel == self.id
                                    && !dragged.paths.iter().any(|(p, _)| *p == path)
                            });
                    if is_dir && root {
                        dirs.push((rect.intersect(view), path.clone()));
                    }
                    let external = is_dir && self.external_hover.as_deref() == Some(path.as_str());
                    let painter = ui.painter_at(rect);
                    if drop_target || external {
                        painter.rect_filled(rect, 5.0, pal.selection);
                    } else if marked {
                        painter.rect_filled(rect, 5.0, pal.surface);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 5.0, pal.crust);
                    }
                    let indent = row.depth as f32 * 14.0 + if tree { 14.0 } else { 0.0 };
                    if tree && is_dir && entry.is_some() {
                        let open = self.expanded.contains(&path);
                        chevron(
                            &painter,
                            egui::pos2(rect.left() + indent - 6.0, rect.center().y),
                            open,
                            pal.faint,
                        );
                        if self.loading.contains(&path) {
                            painter.circle_filled(
                                egui::pos2(rect.left() + indent - 6.0, rect.center().y + 7.0),
                                1.5,
                                pal.accent,
                            );
                        }
                    }
                    paint_icon(
                        &painter,
                        egui::pos2(rect.left() + indent + 14.0, rect.center().y),
                        is_dir,
                        &name,
                        pal,
                    );
                    let mut right = view.right().min(rect.right()) - 6.0;
                    let info = entry.as_ref();
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[2],
                        info.and_then(|e| e.mtime).map(date_label),
                        pal.faint,
                    );
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[1],
                        info.and_then(|e| e.perm).map(|mode| format!("{mode:03o}")),
                        pal.faint,
                    );
                    crate::widgets::column_cell(
                        &painter,
                        rect,
                        &mut right,
                        widths[0],
                        info.filter(|e| !e.is_dir)
                            .map(|e| size_label(e.size as usize)),
                        pal.faint,
                    );
                    let name_clip = Rect::from_min_max(
                        egui::pos2(rect.left() + indent + 28.0, rect.top()),
                        egui::pos2(right.max(rect.left() + indent + 28.0), rect.bottom()),
                    );
                    ui.painter_at(name_clip).text(
                        egui::pos2(name_clip.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        egui::FontId::proportional(13.0),
                        if name.starts_with('.') && name != ".." {
                            pal.subtext
                        } else {
                            pal.text
                        },
                    );

                    if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        let on_chevron = tree
                            && is_dir
                            && entry.is_some()
                            && response
                                .interact_pointer_pos()
                                .is_some_and(|p| p.x < rect.left() + indent + 4.0);
                        if on_chevron {
                            toggle = Some(path.clone());
                        } else if entry.is_none() {
                            self.marked.clear();
                        } else if modifiers.shift && self.anchor.is_some() {
                            let anchor = self.anchor.clone().unwrap_or_default();
                            let a = rows.iter().position(|r| r.path == anchor).unwrap_or(index);
                            let (from, to) = (a.min(index), a.max(index));
                            if !modifiers.command {
                                self.marked.clear();
                            }
                            for r in &rows[from..=to] {
                                if r.entry.is_some() {
                                    self.marked.insert(r.path.clone());
                                }
                            }
                        } else if modifiers.command {
                            if !self.marked.remove(&path) {
                                self.marked.insert(path.clone());
                            }
                            self.anchor = Some(path.clone());
                        } else {
                            self.marked.clear();
                            self.marked.insert(path.clone());
                            self.anchor = Some(path.clone());
                            if tree && is_dir {
                                toggle = Some(path.clone());
                            }
                        }
                    }
                    if response.double_clicked() {
                        if is_dir && tree && entry.is_some() {
                        } else if is_dir {
                            go = Some(path.clone());
                        } else {
                            download.push(path.clone());
                        }
                    }
                    if entry.is_some() && response.drag_started() {
                        response.dnd_set_drag_payload(Dragged {
                            panel: self.id,
                            paths: self.selection_paths(&rows, &path),
                        });
                    }
                    if is_dir
                        && response
                            .dnd_hover_payload::<Dragged>()
                            .is_some_and(|d| d.panel == self.id)
                        && let Some(dragged) = response.dnd_release_payload::<Dragged>()
                    {
                        for (from, _) in dragged.paths.iter() {
                            let to = remote::join(&path, remote::file_name(from));
                            if *from != path && remote::parent(from) != path {
                                moves.push((from.clone(), to));
                            }
                        }
                    }
                    if entry.is_some() {
                        response.context_menu(|ui| {
                            ui.set_min_width(220.0);
                            let chosen = self.selection_paths(&rows, &path);
                            let files: Vec<String> = chosen
                                .iter()
                                .filter(|(_, dir)| !dir)
                                .map(|(p, _)| p.clone())
                                .collect();
                            let dirs: Vec<String> = chosen
                                .iter()
                                .filter(|(_, dir)| *dir)
                                .map(|(p, _)| p.clone())
                                .collect();
                            if !files.is_empty() && ui.button(tr!("Открыть")).clicked() {
                                download.extend(files.iter().cloned());
                            }
                            let archives: Vec<String> = files
                                .iter()
                                .filter(|f| remote::is_archive(f))
                                .cloned()
                                .collect();
                            if !archives.is_empty() && self.sftp() {
                                let mut extract: Option<Vec<(String, Option<String>)>> = None;
                                if ui.button(tr!("Распаковать здесь")).clicked() {
                                    extract = Some(archives.iter().map(|a| (a.clone(), None)).collect());
                                }
                                let label = match archives.as_slice() {
                                    [one] => tf!("Распаковать в «{}/»", "Extract to \"{}/\"",
                                        remote::archive_stem(one).unwrap_or_default()
                                    ),
                                    _ => tr!("Распаковать каждый в свою папку").to_owned(),
                                };
                                if ui.button(label).clicked() {
                                    extract = Some(
                                        archives
                                            .iter()
                                            .map(|a| {
                                                let stem = remote::archive_stem(a).unwrap_or("archive");
                                                (a.clone(), Some(remote::join(&remote::parent(a), stem)))
                                            })
                                            .collect(),
                                    );
                                }
                                if ui.button(tr!("Распаковать в…")).clicked() {
                                    let first = &archives[0];
                                    let suggest = remote::join(
                                        &remote::parent(first),
                                        remote::archive_stem(first).unwrap_or_default(),
                                    );
                                    self.prompt = Some(Prompt::Extract(archives.clone(), suggest));
                                }
                                for (archive, dest) in extract.into_iter().flatten() {
                                    let busy = tf!("Распаковка {}…", "Extracting {}…", remote::file_name(&archive));
                                    self.send(Cmd::Extract { path: archive, dest }, busy);
                                }
                            }
                            let picked: Vec<String> = chosen.iter().map(|(p, _)| p.clone()).collect();
                            self.clip_items(
                                ui,
                                &picked,
                                (chosen.len() == 1 && is_dir).then_some(path.as_str()),
                            );
                            let parent = remote::parent(&picked[0]);
                            if picked.iter().all(|p| remote::parent(p) == parent) {
                                ui.menu_button(tr!("Архивировать"), |ui| {
                                    for (label, zip) in [("TAR.GZ", false), ("ZIP", true)] {
                                        if ui.button(label).clicked() {
                                            let names = picked
                                                .iter()
                                                .map(|p| remote::file_name(p).to_owned())
                                                .collect();
                                            self.send(
                                                Cmd::Archive {
                                                    dir: parent.clone(),
                                                    names,
                                                    zip,
                                                },
                                                tr!("Архивирование…"),
                                            );
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            ui.separator();
                            if chosen.len() == 1 && is_dir {
                                if ui.button(tr!("Открыть консоль здесь")).clicked() {
                                    self.console_here(&path);
                                }
                                if ui.button(tr!("Новый файл здесь…")).clicked() {
                                    self.prompt = Some(Prompt::NewFile(path.clone(), String::new()));
                                }
                                if ui.button(tr!("Новая папка здесь…")).clicked() {
                                    self.prompt =
                                        Some(Prompt::NewFolder(path.clone(), String::new()));
                                }
                                if ui.button(tr!("Загрузить сюда файлы…")).clicked()
                                    && let Some(files) = rfd::FileDialog::new().pick_files()
                                {
                                    self.upload_local_to(files, &path);
                                }
                                ui.separator();
                            }
                            if files.len() == 1
                                && ui.button(tr!("Скачать как…")).clicked()
                                && let Some(local) = rfd::FileDialog::new()
                                    .set_file_name(remote::file_name(&files[0]))
                                    .save_file()
                            {
                                fetch_as = Some((files[0].clone(), local));
                            }
                            if (files.len() > 1 || !dirs.is_empty())
                                && ui.button(tr!("Скачать в папку…")).clicked()
                                && let Some(folder) = std::env::var_os("TROR_PICK_FOLDER")
                                    .map(PathBuf::from)
                                    .or_else(|| rfd::FileDialog::new().pick_folder())
                            {
                                let items = files
                                    .iter()
                                    .map(|f| (f.clone(), false))
                                    .chain(dirs.iter().map(|d| (d.clone(), true)))
                                    .collect();
                                fetch = Some((items, folder));
                            }
                            if dirs.len() == 1
                                && files.is_empty()
                                && ui.button(tr!("Синхронизировать с папкой на компьютере…")).clicked()
                                && let Some(folder) = std::env::var_os("TROR_PICK_FOLDER")
                                    .map(PathBuf::from)
                                    .or_else(|| rfd::FileDialog::new().pick_folder())
                            {
                                self.walks.insert(dirs[0].clone(), Walk::Sync(folder));
                                walk.push(dirs[0].clone());
                            }
                            if !dirs.is_empty() && ui.button(tr!("Размер папки")).clicked() {
                                for dir in &dirs {
                                    self.walks.insert(dir.clone(), Walk::Size);
                                    walk.push(dir.clone());
                                }
                            }
                            for (peer, label) in self.peers.clone() {
                                if ui.button(tf!("Передать на «{label}»", "Transfer to \"{label}\"")).clicked() {
                                    self.notes.push(PanelEvent::Send {
                                        target: peer,
                                        files: chosen.clone(),
                                    });
                                }
                            }
                            ui.separator();
                            if ui.button(tr!("Права и атрибуты…")).clicked() {
                                attrs = Some(chosen.clone());
                            }
                            if ui.button(tr!("Копировать путь")).clicked() {
                                let text: Vec<&str> =
                                    chosen.iter().map(|(p, _)| p.as_str()).collect();
                                ui.ctx().copy_text(text.join("\n"));
                            }
                            if chosen.len() == 1 && ui.button(tr!("Переименовать…")).clicked()
                            {
                                self.prompt = Some(Prompt::Rename {
                                    from: path.clone(),
                                    name: name.clone(),
                                });
                            }
                            let label = if chosen.len() > 1 {
                                tf!("Удалить выбранное ({})…", "Delete selected ({})…", chosen.len())
                            } else {
                                tr!("Удалить…").to_owned()
                            };
                            if ui.button(label).clicked() {
                                self.prompt = Some(Prompt::Delete(chosen));
                            }
                        });
                    }
                }
            });
        let inner = area.inner_rect;
        self.drop_dirs = dirs;
        if blank.clicked() {
            self.marked.clear();
        }
        let keys_area = Rect::from_min_size(outer.min, egui::vec2(outer.width(), height));
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(keys_area) {
            ui.memory_mut(|m| m.request_focus(blank.id));
        }
        if ui.memory(|m| m.has_focus(blank.id)) && self.prompt.is_none() {
            let chosen: Vec<(String, bool)> = rows
                .iter()
                .filter(|r| r.entry.is_some() && self.marked.contains(&r.path))
                .map(|r| (r.path.clone(), r.entry.as_ref().is_some_and(|e| e.is_dir)))
                .collect();
            let (delete, rename) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F2),
                )
            });
            let (cut, copy, paste) = ui.input(|i| {
                let mut found = (false, false, false);
                for event in &i.events {
                    match event {
                        egui::Event::Cut => found.0 = true,
                        egui::Event::Copy => found.1 = true,
                        egui::Event::Paste(_) => found.2 = true,
                        _ => {}
                    }
                }
                found
            });
            if (cut || copy) && !chosen.is_empty() {
                self.clip = Some((chosen.iter().map(|(p, _)| p.clone()).collect(), copy));
            }
            if paste && self.clip.is_some() {
                let dir = match chosen.as_slice() {
                    [(one, true)] => one.clone(),
                    _ => self.cwd.clone(),
                };
                self.paste(&dir);
            }
            if delete && !chosen.is_empty() {
                self.prompt = Some(Prompt::Delete(chosen.clone()));
            } else if rename && chosen.len() == 1 {
                let from = chosen[0].0.clone();
                self.prompt = Some(Prompt::Rename {
                    name: remote::file_name(&from).to_owned(),
                    from,
                });
            }
        }
        let cwd = self.cwd.clone();
        egui::Popup::context_menu(&blank).show(|ui| {
            ui.set_min_width(220.0);
            self.clip_items(ui, &[], Some(&cwd));
            self.menu_items(ui, &cwd);
            if ui.button(tr!("Открыть консоль здесь")).clicked() {
                self.console_here(&cwd);
            }
            ui.separator();
            if ui.button(tr!("Обновить")).clicked() {
                self.list(cwd.clone());
                ui.close();
            }
            if ui.button(tr!("Копировать путь папки")).clicked() {
                ui.ctx().copy_text(cwd.clone());
                ui.close();
            }
        });
        if let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx())
            && dragged.panel != self.id
            && ui.rect_contains_pointer(inner)
        {
            ui.painter().rect_stroke(
                inner.shrink(1.0),
                6.0,
                egui::Stroke::new(1.5, pal.accent),
                egui::StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released())
                && let Some(dragged) = egui::DragAndDrop::take_payload::<Dragged>(ui.ctx())
            {
                self.notes.push(PanelEvent::Receive((*dragged).clone()));
            }
        }
        if ui.ctx().dragged_id().is_some()
            && let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx())
            && dragged.panel == self.id
            && let Some(pos) = ui.ctx().pointer_interact_pos()
        {
            let text = if dragged.paths.len() == 1 {
                remote::file_name(&dragged.paths[0].0).to_owned()
            } else {
                tf!("объектов: {}", "items: {}", dragged.paths.len())
            };
            let _ = pal;
            egui::Area::new(egui::Id::unique("tror_drag"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(14.0, 10.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(text).small());
                    });
                });
        }
        if let Some(path) = toggle {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path.clone());
                if !self.children.contains_key(&path) {
                    self.load(path);
                }
            }
        }
        if let Some(path) = go {
            self.list(path);
        }
        if let Some(items) = attrs {
            self.open_attrs(items);
        }
        if let Some((items, folder)) = fetch {
            self.download_into(items, &folder);
        }
        if let Some((path, local)) = fetch_as
            && let Some(folder) = local.parent()
        {
            self.download_into(vec![(path.clone(), false)], folder);
            self.save_to.insert(path, local);
        }
        for path in download {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Download(path), tf!("Скачивание {name}…", "Downloading {name}…"));
        }
        if !moves.is_empty() {
            self.prompt = Some(Prompt::Move(moves));
        }
        for path in walk {
            let name = remote::file_name(&path).to_owned();
            self.send(Cmd::Walk(path), tf!("Чтение папки {name}…", "Reading folder {name}…"));
        }
    }
}
