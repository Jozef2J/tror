use super::*;

impl TrorApp {
    pub(super) fn recover(&mut self) {
        let Some(dir) = crate::text::recover::dir() else {
            return;
        };
        let found = crate::text::recover::pending(&dir);
        if found.is_empty() {
            return;
        }
        let count = found.len();
        for (entry, text) in found {
            let open = entry
                .path
                .as_ref()
                .and_then(|p| self.docs.iter().position(|d| d.path.as_ref() == Some(p)));
            let mut fresh = false;
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
                            let mut doc = Document::recovered(
                                self.next_id,
                                entry.untitled.max(1),
                                entry.path.clone(),
                                entry.remote.clone(),
                                entry.name.clone(),
                            );
                            self.next_id += 1;
                            let name = entry
                                .path
                                .clone()
                                .or_else(|| entry.remote.as_ref().map(|r| PathBuf::from(&r.1)))
                                .or_else(|| entry.name.as_ref().map(PathBuf::from));
                            doc.set_syntax(self.syntaxes.detect(name.as_deref(), &text));
                            self.docs.push(doc);
                            fresh = true;
                            self.docs.len() - 1
                        }
                    }
                }
            };
            self.docs[index].set_text(&text);
            if fresh {
                self.docs[index].forget_history();
            }
            self.active = index;
        }
        crate::text::recover::clear(&dir);
        let _ = self.recovery.snapshot(&dir, &self.docs);
        self.toast = Some((
            tf!(
                "Восстановлены несохранённые изменения, вкладок: {count}",
                "Unsaved changes restored, tabs: {count}"
            ),
            0.0,
        ));
    }

    pub(super) fn open_arg(&mut self, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        if path.is_dir() {
            self.project.set_root(path);
            self.settings.show(&mut self.dock, Pane::Project);
        } else {
            self.open_path(&path, 0.0);
        }
    }

    pub(super) fn poll_instance(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.instance else {
            return;
        };
        let paths: Vec<PathBuf> = rx.try_iter().collect();
        if paths.is_empty() {
            return;
        }
        for path in paths {
            self.open_arg(path);
        }
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::Minimized(false),
        );
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
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
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("docx"))
        {
            match crate::visual::docx::to_html(path) {
                Ok(html) => {
                    self.new_doc();
                    let index = self.active;
                    let doc = &mut self.docs[index];
                    doc.replace(0..0, &html, crate::text::document::EditKind::Other);
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "document".into());
                    doc.present_as(format!("{stem}.html"), "HTML".into());
                    let editor = crate::visual::Visual::new(doc);
                    self.visual.insert(doc.id, editor);
                    self.notify(
                        tf!(
                            "Документ Word «{stem}» открыт как HTML — сохраните его как .html",
                            "Word document \"{stem}\" opened as HTML — save it as .html"
                        ),
                        now,
                    );
                }
                Err(e) => self.notify(
                    tf!(
                        "Не удалось открыть {}: {e}",
                        "Could not open {}: {e}",
                        path.display()
                    ),
                    now,
                ),
            }
            return;
        }
        let started = std::time::Instant::now();
        match Document::open(self.next_id, path) {
            Ok(mut doc) => {
                self.next_id += 1;
                doc.set_syntax(self.syntaxes.detect(Some(path), &doc.first_line()));
                if self.perf.is_some() {
                    eprintln!(
                        "открыт {} ({}) за {:.0} мс",
                        path.display(),
                        size_label(doc.len_bytes()),
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                let replace_blank = self.docs.len() == 1
                    && !self.docs[0].has_path()
                    && !self.docs[0].dirty()
                    && self.docs[0].len() == 0;
                if replace_blank {
                    self.docs.clear();
                }
                if let Some((_, marks, folds)) = self.settings.view_of(path) {
                    doc.restore_view(&marks, &folds);
                }
                self.docs.push(doc);
                self.active = self.docs.len() - 1;
                self.focus_editor = true;
                self.remember(path);
                self.plugin_event("opened", self.active);
            }
            Err(e) => self.notify(
                tf!(
                    "Не удалось открыть {}: {e}",
                    "Could not open {}: {e}",
                    path.display()
                ),
                now,
            ),
        }
    }

    pub(super) fn check_missing(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        if self.checked != (doc.id, focused) {
            self.checked = (doc.id, focused);
            if focused {
                doc.check_on_disk();
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
        self.settings.remember_file(path);
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
            let data = doc.encoded();
            let expect = doc.begin_upload(&data);
            let result = panel.upload(&ctx, &profile, &path, data, (doc.id, doc.version), expect);
            match result {
                Ok(text) => self.notify(text, now),
                Err(e) => self.notify(tf!("Не сохранено: {e}", "Not saved: {e}"), now),
            }
            return false;
        }
        if !doc.has_path() {
            return self.save_as(index, now);
        }
        match doc.save() {
            Ok(()) => {
                let title = doc.title();
                self.preview.touch();
                self.notify(tf!("Сохранено: {title}", "Saved: {title}"), now);
                self.plugin_event("saved", index);
                true
            }
            Err(e) => {
                self.notify(tf!("Ошибка сохранения: {e}", "Save error: {e}"), now);
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
            None => {
                dialog = dialog.set_file_name(match &doc.name {
                    Some(name) => name.clone(),
                    None => format!("{}.txt", doc.title()),
                })
            }
        }
        let Some(path) = dialog.save_file() else {
            return false;
        };
        let doc = &mut self.docs[index];
        match doc.save_as_path(path.clone()) {
            Ok(()) => {
                doc.set_syntax(self.syntaxes.detect(Some(&path), &doc.first_line()));
                self.preview.touch();
                self.remember(&path);
                self.notify(tf!("Сохранено: {}", "Saved: {}", path.display()), now);
                self.plugin_event("saved", index);
                true
            }
            Err(e) => {
                self.notify(tf!("Ошибка сохранения: {e}", "Save error: {e}"), now);
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

    pub(super) fn remove_doc(&mut self, index: usize) {
        if index >= self.docs.len() {
            return;
        }
        let doc = self.docs.remove(index);
        if let Some(path) = doc.path.clone() {
            let (marks, folds) = doc.view_state();
            self.settings.remember_view(path, marks, folds);
        }
        if !doc.is_new() {
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
