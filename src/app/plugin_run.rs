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
    pub(super) fn plugin_command(&mut self, plugin: usize, command: usize) {
        let Some(id) = self
            .plugins
            .wasm
            .get(plugin)
            .and_then(|p| p.manifest.commands.get(command))
            .map(|c| c.id.clone())
        else {
            return;
        };
        let target = self.active_target();
        self.plugin_send(
            plugin,
            json!({"method": "command", "params": {"id": id}}),
            target,
        );
    }

    pub(super) fn plugin_event(&mut self, event: &str, index: usize) {
        let Some(doc) = self.docs.get(index) else {
            return;
        };
        let message = json!({"method": "event", "params": {"name": event, "path": doc.location()}});
        for plugin in 0..self.plugins.wasm.len() {
            let wasm = &self.plugins.wasm[plugin];
            if wasm.broken.is_none() && wasm.wants(event) {
                self.plugin_send(plugin, message.clone(), Some(index));
            }
        }
    }

    pub(super) fn plugin_send(
        &mut self,
        plugin: usize,
        message: serde_json::Value,
        target: Option<usize>,
    ) {
        use crate::wasm::{Job, Snapshot};
        let root = self.project.root.clone();
        let doc = target.and_then(|i| self.docs.get(i));
        let snap = match doc {
            Some(doc) => Some(Snapshot {
                rope: doc.rope.clone(),
                sel: doc.sel,
                path: doc.location(),
                syntax: doc.syntax.clone(),
                root,
            }),
            None => root.map(|root| Snapshot {
                rope: Default::default(),
                sel: (0, 0),
                path: None,
                syntax: String::new(),
                root: Some(root),
            }),
        };
        let doc = doc.map(|d| (d.id, d.version));
        let ctx = self.ctx.clone();
        if let Some(worker) = self.plugins.wasm.get_mut(plugin) {
            worker.run(Job {
                message,
                snap,
                doc,
                wake: Box::new(move || ctx.request_repaint()),
            });
        }
    }

    pub(super) fn plugin_finished(&mut self, now: f64) {
        use crate::wasm::Effect;
        for done in self.plugins.finished() {
            let Some(name) = self
                .plugins
                .wasm
                .get(done.plugin)
                .map(|w| w.manifest.name.clone())
            else {
                continue;
            };
            if self.perf.is_some() {
                eprintln!("плагин {name}: {:.1} мс", done.took.as_secs_f64() * 1000.0);
            }
            let outcome = done.outcome;
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
                    Effect::Prompt(..)
                    | Effect::Pick(..)
                    | Effect::Panel(..)
                    | Effect::ShowPanel(..) => self.plugin_effect_ui(done.plugin, effect),
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
            if (!edits.is_empty() || moved)
                && let (Some((id, version)), Some(sel)) = (done.doc, outcome.sel)
                && let Some(doc) = self.docs.iter_mut().find(|d| d.id == id)
            {
                if doc.version == version {
                    doc.apply_edits(&edits, sel);
                } else if !edits.is_empty() {
                    self.notify(
                        format!("Плагин «{name}»: файл изменился, правки не применены"),
                        now,
                    );
                }
            }
            if let Some(error) = outcome.error {
                crate::trace::line(format!("[plugin {name}] {error}"), true);
                self.notify(format!("Плагин «{name}»: {error}"), now);
            }
        }
    }
}
