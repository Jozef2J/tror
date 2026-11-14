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
                    self.remote_events(ctx, events);
                }
            }
            Pane::Console => self.console.dialogs(ctx, pal),
            Pane::Project => self.project.dialogs(ctx, pal),
            Pane::Db => self.db.dialogs(ctx, pal, &mut self.remotes[0].store),
            _ => {}
        }
    }

    pub(super) fn hosts_panes(&self) -> Vec<Pane> {
        (0..self.remotes.len()).map(Pane::hosts).collect()
    }

    pub(super) fn host_shown(&self, index: usize) -> bool {
        let pane = Pane::hosts(index);
        dock::has(&self.dock, pane) || self.settings.detached.contains(&pane)
    }

    pub(super) fn trim_hosts(&mut self) {
        while self.remotes.len() > 1 {
            let last = self.remotes.len() - 1;
            if self.host_shown(last) || self.remotes[last].in_use() {
                break;
            }
            self.remotes.pop();
        }
        self.settings.hosts = self.remotes.len();
    }

    pub(super) fn transfers(&mut self) {
        let count = self.remotes.len();
        for from in 0..count {
            if let Some((target, files)) = self.remotes[from].outgoing.take()
                && let Some(dir) = self
                    .remotes
                    .get(target as usize)
                    .filter(|p| p.peer_label().is_some())
                    .map(|p| p.cwd().to_owned())
            {
                self.remotes[from].relay(files, target, &dir);
            }
            if let Some(dragged) = self.remotes[from].incoming.take() {
                let dir = self.remotes[from].cwd().to_owned();
                if let Some(source) = self.remotes.get_mut(dragged.panel as usize) {
                    source.relay(dragged.paths, from as u8, &dir);
                }
            }
        }
    }

    pub(super) fn remote_events(&mut self, ctx: &egui::Context, events: Vec<PanelEvent>) {
        let now = ctx.input(|i| i.time);
        for event in events {
            match event {
                PanelEvent::Relay { target, path, data } => {
                    match self
                        .remotes
                        .get_mut(target as usize)
                        .filter(|p| p.peer_label().is_some())
                    {
                        Some(panel) => panel.receive(path, data),
                        None => self.notify("Передача отменена: второй сервер отключён", now),
                    }
                }
                PanelEvent::Toast(text) => self.notify(text, now),
                PanelEvent::OpenLocal(paths) => {
                    for path in paths {
                        self.open_path(&path, now);
                    }
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
                    }
                    self.notify(format!("Сохранено на сервере: {path}"), now);
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
                            None => self.notify("Файл уже открыт — показана открытая вкладка", now),
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
                        Err(e) => self.notify(format!("Не удалось открыть {path}: {e}"), now),
                    }
                }
            }
        }
    }
}
