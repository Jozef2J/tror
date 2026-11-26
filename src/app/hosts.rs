use super::*;

impl TrorApp {
    pub(super) fn pane_dialogs(&mut self, pane: Pane, ctx: &egui::Context, pal: &Palette) {
        match pane {
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) => {
                let Some(index) = pane.host_index() else {
                    return;
                };
                if let Some(panel) = self.remotes.get_mut(index) {
                    let events = panel.dialogs(ctx, pal);
                    self.remote_events(ctx, index, events);
                }
            }
            Pane::Console => self.console.dialogs(ctx, pal),
            Pane::Project => self.project.dialogs(ctx, pal),
            Pane::Db => self.db.dialogs(ctx, pal),
            _ => {}
        }
    }

    pub(super) fn hosts_panes(&self) -> Vec<Pane> {
        (0..self.remotes.len()).map(Pane::hosts).collect()
    }

    pub(super) fn host_shown(&self, index: usize) -> bool {
        let pane = Pane::hosts(index);
        dock::has(&self.dock, pane) || self.settings.is_detached(pane)
    }

    pub(super) fn trim_hosts(&mut self) {
        while self.remotes.len() > 1 {
            let last = self.remotes.len() - 1;
            if self.host_shown(last) || self.remotes[last].in_use() {
                break;
            }
            self.remotes.pop();
        }
        self.settings.remember_hosts(self.remotes.len());
    }

    pub(super) fn remote_events(&mut self, ctx: &egui::Context, from: usize, events: Vec<PanelEvent>) {
        let now = ctx.input(|i| i.time);
        for event in events {
            match event {
                PanelEvent::Tree(tree) => {
                    self.settings.view_as_tree(true, tree);
                }
                PanelEvent::FindFiles => self.queued.push(Action::SearchServer(from)),
                PanelEvent::Send { target, files } => {
                    if let Some(dir) = self
                        .remotes
                        .get(target as usize)
                        .filter(|p| p.peer_label().is_some())
                        .map(|p| p.cwd().to_owned())
                        && let Some(panel) = self.remotes.get_mut(from)
                    {
                        panel.relay(files, target, &dir);
                    }
                }
                PanelEvent::Receive(dragged) => {
                    let Some(dir) = self.remotes.get(from).map(|p| p.cwd().to_owned()) else {
                        continue;
                    };
                    if let Some(source) = self.remotes.get_mut(dragged.panel as usize) {
                        source.relay(dragged.paths, from as u8, &dir);
                    }
                }
                PanelEvent::Relay { target, path, data } => {
                    match self
                        .remotes
                        .get_mut(target as usize)
                        .filter(|p| p.peer_label().is_some())
                    {
                        Some(panel) => panel.receive(path, data),
                        None => self.notify(tr!("Передача отменена: второй сервер отключён"), now),
                    }
                }
                PanelEvent::Toast(text) => self.notify(text, now),
                PanelEvent::Conflict {
                    profile,
                    doc,
                    path,
                    server,
                } => {
                    self.conflict = Some(Conflict {
                        profile,
                        doc: doc.0,
                        path,
                        server,
                    });
                }
                PanelEvent::Compare {
                    profile,
                    path,
                    server,
                } => {
                    let now = ctx.input(|i| i.time);
                    if let Some(index) = self.docs.iter().position(|d| {
                        d.remote
                            .as_ref()
                            .is_some_and(|(p, r)| *p == profile && *r == path)
                    }) {
                        self.show_server_diff(index, &profile, &path, server, now);
                    }
                }
                PanelEvent::Console(profile) => {
                    self.console.open(*profile, ctx);
                    self.settings.show(&mut self.dock, Pane::Console);
                }
                PanelEvent::Listed {
                    profile,
                    dir,
                    names,
                } => {
                    for doc in &mut self.docs {
                        if let Some((p, path)) = &doc.remote
                            && *p == profile
                            && crate::remote::parent(path) == dir
                        {
                            let exists = names.iter().any(|n| n == crate::remote::file_name(path));
                            if exists {
                                doc.missing = false;
                                doc.missing_ack = false;
                            } else {
                                doc.missing = true;
                            }
                        }
                    }
                }
                PanelEvent::Uploaded { doc, path } => {
                    if let Some(d) = self.docs.iter_mut().find(|d| d.id == doc.0) {
                        d.mark_saved(doc.1);
                        if d.pending_hash.is_some() {
                            d.server_hash = d.pending_hash.take();
                        }
                    }
                    self.notify(
                        tf!(
                            "Сохранено на сервере: {path}",
                            "Saved on the server: {path}"
                        ),
                        now,
                    );
                }
                PanelEvent::Open {
                    profile,
                    path,
                    data,
                    line,
                } => {
                    let target = Some((profile.clone(), path.clone()));
                    if let Some(i) = self.docs.iter().position(|d| d.remote == target) {
                        self.active = i;
                        self.focus_editor = true;
                        match line {
                            Some(line) => self.docs[i].goto_line(line),
                            None => {
                                self.notify(tr!("Файл уже открыт — показана открытая вкладка"), now)
                            }
                        }
                        continue;
                    }
                    match Document::open_remote(self.next_id, &profile, &path, data) {
                        Ok(mut doc) => {
                            self.next_id += 1;
                            doc.syntax = self
                                .syntaxes
                                .detect(Some(Path::new(&path)), &doc.first_line());
                            if let Some(line) = line {
                                doc.goto_line(line);
                            }
                            self.docs.push(doc);
                            self.active = self.docs.len() - 1;
                            self.focus_editor = true;
                        }
                        Err(e) => self.notify(
                            tf!(
                                "Не удалось открыть {path}: {e}",
                                "Could not open {path}: {e}"
                            ),
                            now,
                        ),
                    }
                }
            }
        }
    }
}
