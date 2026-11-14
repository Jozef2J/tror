use serde_json::{Map, Value, json};

use super::*;

pub(super) enum Ask {
    Text(String),
    Pick(Vec<String>, String, usize),
    Settings(Map<String, Value>),
}

pub(super) struct PluginDialog {
    pub plugin: usize,
    pub id: String,
    pub title: String,
    pub ask: Ask,
}

pub(super) struct PluginPanel {
    pub plugin: usize,
    pub panel: String,
    pub tree: Value,
}

fn text_of(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn render(ui: &mut Ui, pal: &Palette, salt: u64, node: &mut Value, out: &mut Vec<(String, Value)>) {
    let kind = text_of(node, "type");
    let id = text_of(node, "id");
    match kind.as_str() {
        "column" | "row" => {
            let mut body = |ui: &mut Ui| {
                if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
                    for child in children {
                        render(ui, pal, salt, child, out);
                    }
                }
            };
            if kind == "row" {
                ui.horizontal_wrapped(|ui| body(ui));
            } else {
                ui.vertical(|ui| body(ui));
            }
        }
        "heading" => {
            ui.label(
                RichText::new(text_of(node, "text"))
                    .size(15.5)
                    .color(pal.text),
            );
        }
        "text" => {
            let mut text = RichText::new(text_of(node, "text"));
            if node.get("small").and_then(Value::as_bool).unwrap_or(false) {
                text = text.small().color(pal.subtext);
            }
            if node.get("strong").and_then(Value::as_bool).unwrap_or(false) {
                text = text.strong();
            }
            ui.add(egui::Label::new(text).wrap());
        }
        "code" => {
            let text = text_of(node, "text");
            egui::Frame::new()
                .fill(pal.surface)
                .corner_radius(4)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).monospace()).selectable(true));
                });
        }
        "button" => {
            let primary = node
                .get("primary")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if dialog_button(ui, pal, &text_of(node, "text"), primary) {
                out.push((id, Value::Null));
            }
        }
        "input" => {
            let mut value = text_of(node, "value");
            let multiline = node
                .get("multiline")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let edit = if multiline {
                TextEdit::multiline(&mut value).desired_rows(4)
            } else {
                TextEdit::singleline(&mut value)
            };
            let response = ui.add(
                edit.hint_text(text_of(node, "hint"))
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                node["value"] = json!(value);
                node["dirty"] = json!(true);
            }
            if response.lost_focus() && node.get("dirty").and_then(Value::as_bool).unwrap_or(false)
            {
                node["dirty"] = json!(false);
                out.push((id, json!(value)));
            }
        }
        "checkbox" => {
            let mut value = node.get("value").and_then(Value::as_bool).unwrap_or(false);
            if ui.checkbox(&mut value, text_of(node, "text")).changed() {
                node["value"] = json!(value);
                out.push((id, json!(value)));
            }
        }
        "select" => {
            let current = text_of(node, "value");
            let options: Vec<String> = node
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let mut chosen = None;
            egui::ComboBox::from_id_salt(("tror_plugin_select", salt, &id))
                .selected_text(current.clone())
                .width(ui.available_width().min(260.0))
                .show_ui(ui, |ui| {
                    for option in &options {
                        if ui.selectable_label(*option == current, option).clicked() {
                            chosen = Some(option.clone());
                        }
                    }
                });
            if let Some(option) = chosen {
                node["value"] = json!(option);
                out.push((id, json!(option)));
            }
        }
        "list" => {
            let selected = node
                .get("selected")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let items: Vec<String> = node
                .get("items")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|v| v.as_str().unwrap_or_default().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            let mut picked = None;
            for (index, item) in items.iter().enumerate() {
                let response = ui.add(
                    egui::Button::selectable(selected == Some(index), item)
                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                );
                if response.clicked() {
                    picked = Some((index, false));
                }
                if response.double_clicked() {
                    picked = Some((index, true));
                }
            }
            if let Some((index, open)) = picked {
                node["selected"] = json!(index);
                out.push((
                    id,
                    json!({"index": index, "item": items[index], "open": open}),
                ));
            }
        }
        "progress" => {
            let value = node.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            ui.add(egui::ProgressBar::new(value.clamp(0.0, 1.0)).show_percentage());
        }
        "separator" => {
            ui.separator();
        }
        "space" => ui.add_space(node.get("size").and_then(Value::as_f64).unwrap_or(8.0) as f32),
        "" => {}
        other => {
            ui.label(RichText::new(format!("неизвестный элемент «{other}»")).color(pal.subtext));
        }
    }
}

impl TrorApp {
    pub(super) fn sync_plugin_panels(&mut self) {
        let mut panels = std::collections::HashMap::new();
        let mut titles = Vec::new();
        for (plugin, worker) in self.plugins.wasm.iter().enumerate() {
            for def in &worker.manifest.panels {
                let key = dock::plugin_key(&worker.manifest.id, &def.id);
                titles.push((key, def.title.clone(), def.dock.clone()));
                panels.insert(
                    key,
                    PluginPanel {
                        plugin,
                        panel: def.id.clone(),
                        tree: Value::Null,
                    },
                );
            }
        }
        dock::set_plugin_panes(titles);
        self.plugin_panels = panels;
        self.plugin_dialog = None;
        let open: Vec<u64> = self
            .plugin_panels
            .keys()
            .copied()
            .filter(|key| {
                dock::has(&self.dock, Pane::Plugin(*key))
                    || self.settings.detached.contains(&Pane::Plugin(*key))
            })
            .collect();
        for key in open {
            self.plugin_panel_opened(key);
        }
    }

    pub(super) fn plugin_panel_opened(&mut self, key: u64) {
        if let Some(panel) = self.plugin_panels.get(&key) {
            let message =
                json!({"method": "event", "params": {"name": "panel_open", "panel": panel.panel}});
            let (plugin, target) = (panel.plugin, self.active_target());
            self.plugin_send(plugin, message, target);
        }
    }

    pub(super) fn open_plugin_panel(&mut self, plugin: usize, panel: usize) {
        let Some(worker) = self.plugins.wasm.get(plugin) else {
            return;
        };
        let Some(def) = worker.manifest.panels.get(panel) else {
            return;
        };
        let key = dock::plugin_key(&worker.manifest.id, &def.id);
        let pane = Pane::Plugin(key);
        let shown = dock::has(&self.dock, pane);
        dock::show(&mut self.dock, pane, &self.settings.floats);
        if !shown {
            self.plugin_panel_opened(key);
        }
    }

    pub(super) fn open_plugin_settings(&mut self, plugin: usize) {
        if let Some(worker) = self.plugins.wasm.get(plugin) {
            self.plugin_dialog = Some(PluginDialog {
                plugin,
                id: String::new(),
                title: format!("Настройки: {}", worker.manifest.name),
                ask: Ask::Settings(crate::wasm::settings_of(&worker.dir, &worker.manifest)),
            });
        }
    }

    pub(super) fn active_target(&self) -> Option<usize> {
        (!self.docs.is_empty()).then_some(self.active)
    }

    pub(super) fn plugin_pane(&mut self, ui: &mut Ui, key: u64, pal: &Palette) {
        let Some(panel) = self.plugin_panels.get_mut(&key) else {
            ui.add_space(12.0);
            ui.label(RichText::new("Плагин этой панели не загружен").color(pal.subtext));
            return;
        };
        let mut out = Vec::new();
        ScrollArea::vertical()
            .id_salt(("tror_plugin_pane", key))
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        if panel.tree.is_null() {
                            ui.label(RichText::new("Загрузка…").color(pal.subtext));
                        } else {
                            render(ui, pal, key, &mut panel.tree, &mut out);
                        }
                    });
            });
        let (plugin, name) = (panel.plugin, panel.panel.clone());
        let target = self.active_target();
        for (id, value) in out {
            let message = json!({"method": "event", "params": {"name": "panel", "panel": name, "id": id, "value": value}});
            self.plugin_send(plugin, message, target);
        }
    }

    pub(super) fn plugin_effect_ui(&mut self, plugin: usize, effect: crate::wasm::Effect) {
        use crate::wasm::Effect;
        let Some(worker) = self.plugins.wasm.get(plugin) else {
            return;
        };
        let plugin_id = worker.manifest.id.clone();
        match effect {
            Effect::Prompt(id, title, value) => {
                self.plugin_dialog = Some(PluginDialog {
                    plugin,
                    id,
                    title,
                    ask: Ask::Text(value),
                });
            }
            Effect::Pick(id, title, items) => {
                self.plugin_dialog = Some(PluginDialog {
                    plugin,
                    id,
                    title,
                    ask: Ask::Pick(items, String::new(), 0),
                });
            }
            Effect::Panel(id, tree) => {
                if let Some(panel) = self
                    .plugin_panels
                    .get_mut(&dock::plugin_key(&plugin_id, &id))
                {
                    panel.tree = tree;
                }
            }
            Effect::ShowPanel(id) => {
                dock::show(
                    &mut self.dock,
                    Pane::Plugin(dock::plugin_key(&plugin_id, &id)),
                    &self.settings.floats,
                );
            }
            _ => {}
        }
    }

    pub(super) fn plugin_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(dialog) = &mut self.plugin_dialog else {
            return;
        };
        let mut done = false;
        let mut answer = None;
        let settings = self
            .plugins
            .wasm
            .get(dialog.plugin)
            .map(|w| w.manifest.settings.clone())
            .unwrap_or_default();
        let modal = super::dialog(("tror_plugin_dialog", dialog.plugin), pal).show(ctx, |ui| {
            ui.set_width(380.0);
            if window_header(ui, pal, &dialog.title, true) {
                done = true;
            }
            let mut confirm = false;
            match &mut dialog.ask {
                Ask::Text(value) => {
                    let field = ui.add(TextEdit::singleline(value).desired_width(f32::INFINITY));
                    field.request_focus();
                    confirm = ui.input(|i| i.key_pressed(Key::Enter));
                }
                Ask::Pick(items, filter, selected) => {
                    let field = ui.add(
                        TextEdit::singleline(filter)
                            .hint_text("фильтр")
                            .desired_width(f32::INFINITY),
                    );
                    field.request_focus();
                    let needle = filter.to_lowercase();
                    let shown: Vec<usize> = (0..items.len())
                        .filter(|i| items[*i].to_lowercase().contains(&needle))
                        .collect();
                    if field.changed() {
                        *selected = 0;
                    }
                    if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                        *selected = (*selected + 1).min(shown.len().saturating_sub(1));
                    }
                    if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
                        *selected = selected.saturating_sub(1);
                    }
                    ui.add_space(4.0);
                    ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for (row, index) in shown.iter().enumerate() {
                            let response = ui.add(
                                egui::Button::selectable(row == *selected, &items[*index])
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            );
                            if row == *selected {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                *selected = row;
                                confirm = true;
                            }
                        }
                    });
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        confirm = true;
                    }
                    if confirm {
                        match shown.get(*selected) {
                            Some(index) => {
                                answer = Some(json!({"index": index, "value": items[*index]}))
                            }
                            None => confirm = false,
                        }
                    }
                }
                Ask::Settings(values) => {
                    egui::Grid::new("tror_plugin_settings")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            for def in &settings {
                                ui.label(&def.title);
                                let value = values.entry(def.key.clone()).or_insert(Value::Null);
                                match def.kind.as_str() {
                                    "bool" => {
                                        let mut on = value.as_bool().unwrap_or(false);
                                        if ui.checkbox(&mut on, "").changed() {
                                            *value = json!(on);
                                        }
                                    }
                                    "number" => {
                                        let mut n = value.as_f64().unwrap_or(0.0);
                                        if ui.add(egui::DragValue::new(&mut n)).changed() {
                                            *value = json!(n);
                                        }
                                    }
                                    "choice" => {
                                        let current = value.as_str().unwrap_or_default().to_owned();
                                        egui::ComboBox::from_id_salt((
                                            "tror_plugin_choice",
                                            &def.key,
                                        ))
                                        .selected_text(current.clone())
                                        .show_ui(
                                            ui,
                                            |ui| {
                                                for option in &def.options {
                                                    if ui
                                                        .selectable_label(
                                                            *option == current,
                                                            option,
                                                        )
                                                        .clicked()
                                                    {
                                                        *value = json!(option);
                                                    }
                                                }
                                            },
                                        );
                                    }
                                    _ => {
                                        let mut text =
                                            value.as_str().unwrap_or_default().to_owned();
                                        if ui
                                            .add(
                                                TextEdit::singleline(&mut text)
                                                    .desired_width(200.0),
                                            )
                                            .changed()
                                        {
                                            *value = json!(text);
                                        }
                                    }
                                }
                                ui.end_row();
                            }
                        });
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "OK", true) {
                    confirm = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
                    done = true;
                }
            });
            if confirm && answer.is_none() {
                answer = match &dialog.ask {
                    Ask::Text(value) => Some(json!({"value": value})),
                    Ask::Settings(values) => Some(Value::Object(values.clone())),
                    Ask::Pick(..) => None,
                };
            }
        });
        let Some(dialog) = self.plugin_dialog.take() else {
            return;
        };
        let target = self.active_target();
        match answer {
            Some(values) if matches!(dialog.ask, Ask::Settings(_)) => {
                let now = ctx.input(|i| i.time);
                let saved = match (&values, self.plugins.wasm.get(dialog.plugin)) {
                    (Value::Object(map), Some(worker)) => {
                        crate::wasm::save_settings(&worker.dir, map)
                    }
                    _ => Err("плагин не загружен".into()),
                };
                match saved {
                    Ok(()) => self.plugin_send(
                        dialog.plugin,
                        json!({"method": "event", "params": {"name": "settings"}}),
                        target,
                    ),
                    Err(e) => self.notify(format!("Настройки плагина не сохранены: {e}"), now),
                }
                self.focus_editor = true;
            }
            Some(mut reply) => {
                reply["name"] = json!("answer");
                reply["id"] = json!(dialog.id);
                self.plugin_send(
                    dialog.plugin,
                    json!({"method": "event", "params": reply}),
                    target,
                );
                self.focus_editor = true;
            }
            None if done || modal.should_close() => self.focus_editor = true,
            None => self.plugin_dialog = Some(dialog),
        }
    }
}
