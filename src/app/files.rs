use super::*;

impl TrorApp {
    pub(super) fn recover(&mut self) {
        let Some(dir) = crate::recover::dir() else {
            return;
        };
        let found = crate::recover::pending(&dir);
        if found.is_empty() {
            return;
        }
        let count = found.len();
        for (entry, text) in found {
            let open = entry
                .path
                .as_ref()
                .and_then(|p| self.docs.iter().position(|d| d.path.as_ref() == Some(p)));
            let index = match open {
                Some(index) => index,
                None => {
                    if let Some(path) = entry.path.as_ref().filter(|p| p.is_file()) {
                        self.open_path(path, 0.0);
                    }
                    match entry
                        .path
                        .as_ref()
                        .and_then(|p| self.docs.iter().position(|d| d.path.as_ref() == Some(p)))
                    {
                        Some(index) => index,
                        None => {
                            let mut doc = Document::empty(self.next_id, entry.untitled.max(1));
                            self.next_id += 1;
                            doc.path = entry.path.clone();
                            doc.remote = entry.remote.clone();
                            let name = entry
                                .path
                                .clone()
                                .or_else(|| entry.remote.as_ref().map(|r| PathBuf::from(&r.1)));
                            doc.syntax = self.syntaxes.detect(name.as_deref(), &text);
                            self.docs.push(doc);
                            self.docs.len() - 1
                        }
                    }
                }
            };
            self.docs[index].set_text(&text);
            self.active = index;
        }
        crate::recover::clear(&dir);
        let _ = self.recovery.snapshot(&dir, &self.docs);
        self.toast = Some((
            format!("Восстановлены несохранённые правки, вкладок: {count}"),
            0.0,
        ));
    }

    pub(super) fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some((text.into(), now));
    }

    pub(super) fn new_doc(&mut self) {
        let taken: Vec<u32> = self
            .docs
            .iter()
            .filter(|d| d.path.is_none() && d.remote.is_none())
            .map(|d| d.untitled)
            .collect();
        let number = (1..).find(|n| !taken.contains(n)).unwrap_or(1);
        let doc = Document::empty(self.next_id, number);
        self.next_id += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
        self.focus_editor = true;
    }

    pub(super) fn open_path(&mut self, path: &Path, now: f64) {
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(path))
        {
            self.active = i;
            self.focus_editor = true;
            return;
        }
        let started = std::time::Instant::now();
        match Document::open(self.next_id, path) {
            Ok(mut doc) => {
                self.next_id += 1;
                doc.syntax = self.syntaxes.detect(Some(path), &doc.first_line());
                if self.perf.is_some() {
                    eprintln!(
                        "открыт {} ({}) за {:.0} мс",
                        path.display(),
                        size_label(doc.rope.len_bytes()),
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                let replace_blank = self.docs.len() == 1
                    && self.docs[0].path.is_none()
                    && !self.docs[0].dirty()
                    && self.docs[0].len() == 0;
                if replace_blank {
                    self.docs.clear();
                }
                if let Some((_, marks, folds)) = self.settings.views.iter().find(|v| v.0 == path) {
                    doc.restore_view(marks, folds);
                }
                self.docs.push(doc);
                self.active = self.docs.len() - 1;
                self.focus_editor = true;
                self.remember(path);
                self.plugin_event("opened", self.active, now);
            }
            Err(e) => self.notify(format!("Не удалось открыть {}: {e}", path.display()), now),
        }
    }

    pub(super) fn check_missing(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        if self.checked != (doc.id, focused) {
            self.checked = (doc.id, focused);
            if focused && let Some(path) = &doc.path {
                if path.exists() {
                    doc.missing = false;
                    doc.missing_ack = false;
                } else {
                    doc.missing = true;
                }
            }
        }
        if doc.missing
            && !doc.missing_ack
            && self.confirm.is_none()
            && self.goto.is_none()
            && !self.remotes.iter().any(|r| r.modal_open())
        {
            self.confirm = Some(Confirm::Missing(doc.id));
        }
    }

    pub(super) fn remember(&mut self, path: &Path) {
        self.settings.recent.retain(|p| p != path);
        self.settings.recent.insert(0, path.to_path_buf());
        self.settings.recent.truncate(MAX_RECENT);
    }

    pub(super) fn open_dialog(&mut self, now: f64) {
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = self
            .docs
            .get(self.active)
            .and_then(|d| d.path.as_deref())
            .and_then(Path::parent)
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(paths) = dialog.pick_files() {
            for path in paths {
                self.open_path(&path, now);
            }
        }
    }

    pub(super) fn save(&mut self, index: usize, now: f64) -> bool {
        let Some(doc) = self.docs.get_mut(index) else {
            return false;
        };
        if let Some((profile, path)) = doc.remote.clone() {
            let ctx = self.ctx.clone();
            let index = self
                .remotes
                .iter()
                .position(|r| r.serves(&profile))
                .unwrap_or(0);
            let panel = &mut self.remotes[index];
            let result = panel.upload(&ctx, &profile, &path, doc.encoded(), (doc.id, doc.version));
            match result {
                Ok(text) => self.notify(text, now),
                Err(e) => self.notify(format!("Не сохранено: {e}"), now),
            }
            return false;
        }
        if doc.path.is_none() {
            return self.save_as(index, now);
        }
        match doc.save() {
            Ok(()) => {
                let title = doc.title();
                self.preview.touch();
                self.notify(format!("Сохранено: {title}"), now);
                self.plugin_event("saved", index, now);
                true
            }
            Err(e) => {
                self.notify(format!("Ошибка сохранения: {e}"), now);
                false
            }
        }
    }

    pub(super) fn save_as(&mut self, index: usize, now: f64) -> bool {
        let Some(doc) = self.docs.get(index) else {
            return false;
        };
        let mut dialog = rfd::FileDialog::new();
        match &doc.path {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    dialog = dialog.set_directory(dir);
                }
                dialog = dialog.set_file_name(doc.title());
            }
            None => dialog = dialog.set_file_name(format!("{}.txt", doc.title())),
        }
        let Some(path) = dialog.save_file() else {
            return false;
        };
        let doc = &mut self.docs[index];
        let previous = doc.path.replace(path.clone());
        match doc.save() {
            Ok(()) => {
                doc.syntax = self.syntaxes.detect(Some(&path), &doc.first_line());
                self.preview.touch();
                self.remember(&path);
                self.notify(format!("Сохранено: {}", path.display()), now);
                self.plugin_event("saved", index, now);
                true
            }
            Err(e) => {
                self.docs[index].path = previous;
                self.notify(format!("Ошибка сохранения: {e}"), now);
                false
            }
        }
    }

    pub(super) fn save_all(&mut self, now: f64) -> bool {
        for i in 0..self.docs.len() {
            if self.docs[i].dirty() && !self.save(i, now) {
                return false;
            }
        }
        true
    }

    pub(super) fn remember_view(settings: &mut Settings, doc: &Document) {
        let Some(path) = &doc.path else {
            return;
        };
        settings.views.retain(|v| &v.0 != path);
        if !doc.marks.is_empty() || !doc.folds.is_empty() {
            settings.views.push((
                path.clone(),
                doc.marks.iter().copied().collect(),
                doc.folds.clone(),
            ));
        }
        let extra = settings.views.len().saturating_sub(300);
        settings.views.drain(..extra);
    }

    pub(super) fn remove_doc(&mut self, index: usize) {
        if index >= self.docs.len() {
            return;
        }
        let doc = self.docs.remove(index);
        Self::remember_view(&mut self.settings, &doc);
        if doc.path.is_some() || doc.remote.is_some() {
            let entry = (doc.path.clone(), doc.remote.clone());
            self.closed.retain(|c| *c != entry);
            self.closed.push(entry);
            if self.closed.len() > 30 {
                self.closed.remove(0);
            }
        }
        if self.active > index || self.active >= self.docs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.focus_editor = true;
    }

    pub(super) fn request_close(&mut self, index: usize) {
        match self.docs.get(index) {
            Some(doc) if doc.dirty() => {
                self.active = index;
                self.confirm = Some(Confirm::Tab(doc.id));
            }
            Some(_) => self.remove_doc(index),
            None => {}
        }
    }
}
