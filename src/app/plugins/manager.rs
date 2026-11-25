use super::super::*;
use crate::widgets::Lit;

#[derive(Default)]
pub(in crate::app) struct Manager {
    install: Option<(PathBuf, crate::plugins::wasm::Manifest)>,
    remove: Option<(PathBuf, String)>,
    log: Option<String>,
    error: Option<String>,
}

fn permission(name: &str) -> &'static str {
    match name {
        "fs" => tr!("чтение и запись файлов в открытой папке проекта"),
        "network" => tr!("запросы в интернет (HTTP)"),
        "hosts" => {
            tr!("файлы на удалённых хостах из ваших профилей (пароли плагину не передаются)")
        }
        "db" => tr!("запросы к базам данных из ваших профилей (пароли плагину не передаются)"),
        _ => tr!("неизвестное разрешение"),
    }
}

impl TrorApp {
    pub(in crate::app) fn plugin_manager(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(manager) = &mut self.manager else {
            return;
        };
        let dir = self.plugins.dir.clone();
        let installed = dir
            .as_deref()
            .map(crate::plugins::wasm::manifests)
            .unwrap_or_default();
        let mut close = false;
        let mut reload = false;
        let mut toggle: Option<(String, bool)> = None;
        let mut pick_zip = false;
        let mut open_dir: Option<PathBuf> = None;
        let mut do_install = None;
        let mut do_remove = None;
        let log_lines: Vec<String> = match &manager.log {
            Some(name) => {
                let tag = format!("[plugin {name}]");
                self.log
                    .iter()
                    .filter(|(line, _)| line.contains(&tag))
                    .map(|(line, _)| line.clone())
                    .rev()
                    .take(200)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            }
            None => Vec::new(),
        };
        let modal = dialog("tror_plugin_manager", pal).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.horizontal(|ui| {
                crate::widgets::title(ui, pal, tr!("Плагины"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Button::new(tr!("Перечитать"))).clicked() {
                        reload = true;
                    }
                    if ui.add(Button::new(tr!("Открыть папку"))).clicked() {
                        open_dir = dir.clone();
                    }
                    if ui.add(crate::widgets::primary(tr!("Установить из zip…"), pal)).lit(ui).clicked() {
                        pick_zip = true;
                    }
                });
            });
            ui.add_space(8.0);
            if let Some(error) = &manager.error {
                ui.label(RichText::new(error).color(pal.warn));
                ui.add_space(6.0);
            }
            if let Some((_, manifest)) = &manager.install {
                egui::Frame::new()
                    .fill(pal.surface)
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let exists = installed
                            .iter()
                            .any(|(_, m)| m.as_ref().is_ok_and(|m| m.id == manifest.id));
                        ui.label(
                            RichText::new(format!(
                                "{} «{}» {}",
                                if exists { tr!("Обновить плагин") } else { tr!("Установить плагин") },
                                manifest.name,
                                manifest.version
                            ))
                            .strong(),
                        );
                        ui.add_space(4.0);
                        if manifest.permissions.is_empty() {
                            ui.label(RichText::new(tr!("Плагин не просит особых разрешений: только текст открытого файла и свои настройки.")).color(pal.subtext));
                        } else {
                            ui.label(tr!("Плагин просит разрешения:"));
                            for p in &manifest.permissions {
                                ui.label(RichText::new(format!("• {}", permission(p))).color(pal.text));
                            }
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if dialog_button(ui, pal, if exists { tr!("Обновить") } else { tr!("Установить") }, true) {
                                do_install = Some(true);
                            }
                            if dialog_button(ui, pal, tr!("Отмена"), false) {
                                do_install = Some(false);
                            }
                        });
                    });
                ui.add_space(8.0);
            }
            if let Some((_, name)) = &manager.remove {
                egui::Frame::new()
                    .fill(pal.surface)
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(tf!("Удалить плагин «{name}» вместе с его настройками и данными?", "Delete plugin \"{name}\" together with its settings and data?"));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if dialog_button(ui, pal, tr!("Удалить"), true) {
                                do_remove = Some(true);
                            }
                            if dialog_button(ui, pal, tr!("Отмена"), false) {
                                do_remove = Some(false);
                            }
                        });
                    });
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                if installed.is_empty() && self.plugins.list.is_empty() {
                    ui.label(RichText::new(tr!("Плагинов пока нет.")).color(pal.subtext));
                }
                for (folder, manifest) in &installed {
                    egui::Frame::new()
                        .stroke(Stroke::new(1.0, pal.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            let folder_name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            match manifest {
                                Err(e) => {
                                    ui.label(RichText::new(&folder_name).strong());
                                    ui.label(RichText::new(e).color(pal.warn));
                                }
                                Ok(m) => {
                                    let enabled = !self.plugins.off.contains(&m.id);
                                    let worker = self.plugins.wasm.iter().find(|w| w.manifest.id == m.id);
                                    ui.horizontal(|ui| {
                                        let mut on = enabled;
                                        if ui.checkbox(&mut on, "").on_hover_text(tr!("Включён")).changed() {
                                            toggle = Some((m.id.clone(), on));
                                        }
                                        ui.label(RichText::new(&m.name).strong());
                                        ui.label(RichText::new(format!("{} · {}", m.version, m.id)).small().color(pal.subtext));
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if ui.small_button(tr!("Удалить…")).clicked() {
                                                manager.remove = Some((folder.clone(), m.name.clone()));
                                            }
                                            if ui.small_button(tr!("Папка")).clicked() {
                                                open_dir = Some(folder.clone());
                                            }
                                            let shown = manager.log.as_deref() == Some(m.name.as_str());
                                            if ui.small_button(if shown { tr!("Скрыть журнал") } else { tr!("Журнал") }).clicked() {
                                                manager.log = if shown { None } else { Some(m.name.clone()) };
                                            }
                                        });
                                    });
                                    let status = match (enabled, worker) {
                                        (false, _) => RichText::new(tr!("Выключен")).color(pal.subtext),
                                        (true, Some(w)) => match &w.broken {
                                            Some(reason) => RichText::new(tf!("Остановлен: {reason}", "Stopped: {reason}")).color(pal.warn),
                                            None => RichText::new(tr!("Работает")).color(crate::widgets::ok_color(ui)),
                                        },
                                        (true, None) => RichText::new(tr!("Не загружен — см. журнал программы")).color(pal.warn),
                                    };
                                    ui.label(status.small());
                                    if !m.permissions.is_empty() {
                                        let list: Vec<&str> = m.permissions.iter().map(|p| permission(p)).collect();
                                        ui.label(RichText::new(tf!("Разрешения: {}", "Permissions: {}", list.join("; "))).small().color(pal.subtext));
                                    }
                                    if manager.log.as_deref() == Some(m.name.as_str()) {
                                        egui::Frame::new().fill(pal.crust).corner_radius(4).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                                            ui.set_min_width(ui.available_width());
                                            if log_lines.is_empty() {
                                                ui.label(RichText::new(tr!("Записей нет")).small().color(pal.subtext));
                                            }
                                            for line in &log_lines {
                                                ui.label(RichText::new(line).monospace().small());
                                            }
                                        });
                                    }
                                }
                            }
                        });
                    ui.add_space(6.0);
                }
                if !self.plugins.list.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(tr!("Внешние программы (.json)")).small().color(pal.subtext));
                    for plugin in &self.plugins.list {
                        ui.label(format!("• {}", plugin.name));
                    }
                }
            });
            ui.add_space(10.0);
            if dialog_button(ui, pal, tr!("Закрыть"), false) {
                close = true;
            }
        });
        if pick_zip
            && let Some(file) = rfd::FileDialog::new()
                .add_filter(tr!("Плагин"), &["zip"])
                .pick_file()
        {
            match crate::plugins::peek_manifest(&file) {
                Ok(m) => {
                    manager.install = Some((file, m));
                    manager.error = None;
                }
                Err(e) => {
                    manager.error = Some(tf!(
                        "Не удалось прочитать архив: {e}",
                        "Could not read the archive: {e}"
                    ))
                }
            }
        }
        if let Some(path) = open_dir {
            let _ = std::fs::create_dir_all(&path);
            let _ = crate::preview::open_folder(&path);
        }
        let mut changed = false;
        if let Some(yes) = do_install
            && let Some((file, _)) = manager.install.take()
            && yes
        {
            match self.plugins.install(&file) {
                Ok(m) => {
                    manager.error = None;
                    crate::trace::line(
                        tf!(
                            "[plugin {}] установлен {}",
                            "[plugin {}] installed {}",
                            m.name,
                            m.version
                        ),
                        false,
                    );
                    changed = true;
                }
                Err(e) => {
                    manager.error =
                        Some(tf!("Не удалось установить: {e}", "Could not install: {e}"))
                }
            }
        }
        if let Some(yes) = do_remove
            && let Some((folder, name)) = manager.remove.take()
            && yes
        {
            match self.plugins.remove(&folder) {
                Ok(()) => {
                    crate::trace::line(
                        tf!("[plugin {name}] удалён", "[plugin {name}] removed"),
                        false,
                    );
                    changed = true;
                }
                Err(e) => {
                    manager.error = Some(tf!("Не удалось удалить: {e}", "Could not delete: {e}"))
                }
            }
        }
        if let Some((id, on)) = toggle {
            self.settings.plugins_off.retain(|x| *x != id);
            if !on {
                self.settings.plugins_off.push(id);
            }
            self.plugins.off = self.settings.plugins_off.clone();
            reload = true;
        }
        if reload {
            self.plugins.reload();
            changed = true;
        }
        if changed {
            self.plugin_decos.clear();
            self.decorations.clear();
            self.plugin_active = None;
            self.sync_plugin_panels();
        }
        if close || modal.should_close() {
            self.manager = None;
        }
    }
}
