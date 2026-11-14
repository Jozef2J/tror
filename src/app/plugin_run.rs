use serde_json::json;

use super::*;

impl TrorApp {
    pub(super) fn run_plugin(&mut self, index: usize, ctx: &egui::Context) {
        use crate::plugins::{Context, Input};
        let now = ctx.input(|i| i.time);
        let Some(plugin) = self.plugins.list.get(index).cloned() else {
            return;
        };
        let doc = self.docs.get(self.active);
        if doc.is_none() && plugin.input != Input::None {
            self.notify(
                format!("Плагину «{}» нужен открытый файл", plugin.name),
                now,
            );
            return;
        }
        let mut context = Context {
            file: String::new(),
            dir: None,
            line: 1,
            column: 1,
            syntax: String::new(),
        };
        let mut input = String::new();
        let mut range = 0..0;
        let (mut id, mut version) = (0, 0);
        if let Some(doc) = doc {
            let (line, column) = doc.cursor_line_col();
            context = Context {
                file: doc.location().unwrap_or_default(),
                dir: doc
                    .path
                    .as_deref()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf()),
                line,
                column,
                syntax: doc.syntax.clone(),
            };
            range = match plugin.input {
                Input::Selection if doc.has_selection() => doc.selection(),
                Input::None => doc.selection(),
                _ => 0..doc.len(),
            };
            if plugin.input != Input::None {
                input = doc.rope.slice(range.clone()).to_string();
            }
            id = doc.id;
            version = doc.version;
        }
        let wake = ctx.clone();
        self.plugins
            .start(plugin, context, input, (id, version), range, move || {
                wake.request_repaint()
            });
    }

    pub(super) fn plugin_results(&mut self, now: f64) {
        use crate::plugins::Output;
        for done in self.plugins.poll() {
            let name = done.plugin.name.clone();
            for line in done.stderr.lines() {
                crate::trace::line(format!("[plugin {name}] {line}"), done.result.is_err());
            }
            let text = match done.result {
                Ok(text) => text,
                Err(e) => {
                    crate::trace::line(format!("[plugin {name}] {e}"), true);
                    self.notify(format!("Плагин «{name}»: {e}"), now);
                    continue;
                }
            };
            let doc = self.docs.iter_mut().find(|d| d.id == done.doc);
            match (done.plugin.output, doc) {
                (Output::None, _) => {}
                (Output::Message, _) => {
                    for line in text.lines() {
                        crate::trace::line(format!("[plugin {name}] {line}"), false);
                    }
                    let first = text.lines().next().unwrap_or("готово").to_owned();
                    self.notify(format!("{name}: {first}"), now);
                }
                (Output::NewTab, _) => {
                    self.new_doc();
                    if let Some(doc) = self.docs.last_mut() {
                        doc.insert_text(&text);
                        doc.move_to(0, false);
                    }
                }
                (Output::Replace | Output::Insert, Some(doc)) if doc.version == done.version => {
                    let source = doc.rope.slice(done.range.clone()).to_string();
                    let text = match text.strip_suffix('\n') {
                        Some(cut) if !source.ends_with('\n') => {
                            cut.strip_suffix('\r').unwrap_or(cut)
                        }
                        _ => &text,
                    };
                    if done.plugin.output == Output::Insert {
                        doc.insert_text(text);
                    } else if source != text {
                        doc.replace(done.range, text, crate::document::EditKind::Other);
                    }
                }
                _ => self.notify(
                    format!("Плагин «{name}»: файл изменился или закрыт, результат не применён"),
                    now,
                ),
            }
        }
    }
}

impl TrorApp {
    pub(super) fn plugin_command(&mut self, plugin: usize, command: usize, now: f64) {
        let Some(id) = self
            .plugins
            .wasm
            .get(plugin)
            .and_then(|p| p.manifest.commands.get(command))
            .map(|c| c.id.clone())
        else {
            return;
        };
        let target = (!self.docs.is_empty()).then_some(self.active);
        self.plugin_send(
            plugin,
            &json!({"method": "command", "params": {"id": id}}),
            target,
            now,
        );
    }

    pub(super) fn plugin_event(&mut self, event: &str, index: usize, now: f64) {
        let Some(doc) = self.docs.get(index) else {
            return;
        };
        let message = json!({"method": "event", "params": {"name": event, "path": doc.location()}});
        for plugin in 0..self.plugins.wasm.len() {
            let wasm = &self.plugins.wasm[plugin];
            if wasm.broken.is_none() && wasm.wants(event) {
                self.plugin_send(plugin, &message, Some(index), now);
            }
        }
    }

    fn plugin_send(
        &mut self,
        plugin: usize,
        message: &serde_json::Value,
        target: Option<usize>,
        now: f64,
    ) {
        use crate::wasm::{Effect, Snapshot};
        let root = self.project.root.clone();
        let snap = target.and_then(|i| self.docs.get(i)).map(|doc| Snapshot {
            rope: doc.rope.clone(),
            sel: doc.sel,
            path: doc.location(),
            syntax: doc.syntax.clone(),
            root: root.clone(),
        });
        let snap = snap.or_else(|| {
            root.clone().map(|root| Snapshot {
                rope: Default::default(),
                sel: (0, 0),
                path: None,
                syntax: String::new(),
                root: Some(root),
            })
        });
        let has_doc = target.is_some_and(|i| i < self.docs.len());
        let Some(loaded) = self.plugins.wasm.get_mut(plugin) else {
            return;
        };
        let name = loaded.manifest.name.clone();
        let started = std::time::Instant::now();
        let outcome = loaded.send(message, snap);
        if self.perf.is_some() {
            eprintln!(
                "плагин {name}: {:.1} мс",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        let mut edits = Vec::new();
        let mut moved = false;
        for effect in outcome.effects {
            match effect {
                Effect::Edit(range, text) => edits.push((range, text)),
                Effect::Select(..) => moved = true,
                Effect::Notify(text) => self.notify(format!("{name}: {text}"), now),
                Effect::Log(text, error) => {
                    crate::trace::line(format!("[plugin {name}] {text}"), error)
                }
                Effect::OpenTab(title, text) => {
                    self.new_doc();
                    let syntax = self.syntaxes.detect(Some(Path::new(&title)), &text);
                    if let Some(doc) = self.docs.last_mut() {
                        doc.insert_text(&text);
                        doc.move_to(0, false);
                        doc.syntax = syntax;
                    }
                }
            }
        }
        if has_doc
            && (!edits.is_empty() || moved)
            && let (Some(index), Some(sel)) = (target, outcome.sel)
            && let Some(doc) = self.docs.get_mut(index)
        {
            doc.apply_edits(&edits, sel);
        }
        if let Some(error) = outcome.error {
            crate::trace::line(format!("[plugin {name}] {error}"), true);
            self.notify(format!("Плагин «{name}»: {error}"), now);
        }
    }
}
