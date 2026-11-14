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
