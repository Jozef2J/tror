use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui::{self, Button, Color32, ComboBox, RichText, ScrollArea, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};

use crate::highlight::{self, SYNTAX_PARTS};
use crate::theme::{self, PARTS, Palette};

type Colors = BTreeMap<String, [u8; 3]>;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Look {
    pub dark: bool,
    pub accent: usize,
    pub colors_dark: Colors,
    pub colors_light: Colors,
    pub syntax_dark: String,
    pub syntax_light: String,
    pub code_dark: Colors,
    pub code_light: Colors,
    pub ui_font: Option<PathBuf>,
    pub code_font: Option<PathBuf>,
    pub ui_size: f32,
    pub font_size: f32,
    pub line_spacing: f32,
    pub radius: u8,
    pub log_dark: Colors,
    pub log_light: Colors,
}

pub const LOG_PARTS: [(&str, &str, u32, u32); 4] = [
    ("server", "Ответы сервера", 0x5fcf80, 0x1e8a3c),
    ("client", "Команды клиента", 0x6fa8ff, 0x1f5fd1),
    ("error", "Ошибки", 0xff6b6b, 0xd02b2b),
    ("info", "Служебные сообщения", 0xa0a7b4, 0x5c6370),
];

impl Default for Look {
    fn default() -> Self {
        Self {
            dark: true,
            accent: 0,
            colors_dark: Colors::new(),
            colors_light: Colors::new(),
            syntax_dark: highlight::DEFAULT_DARK.into(),
            syntax_light: highlight::DEFAULT_LIGHT.into(),
            code_dark: Colors::new(),
            code_light: Colors::new(),
            ui_font: None,
            code_font: None,
            ui_size: 13.5,
            font_size: 14.0,
            line_spacing: 1.5,
            radius: 7,
            log_dark: Colors::new(),
            log_light: Colors::new(),
        }
    }
}

impl Look {
    pub fn palette(&self) -> Palette {
        let colors = if self.dark {
            &self.colors_dark
        } else {
            &self.colors_light
        };
        theme::palette(self.dark, self.accent, colors)
    }

    pub fn syntax(&self) -> (&str, &Colors) {
        if self.dark {
            (&self.syntax_dark, &self.code_dark)
        } else {
            (&self.syntax_light, &self.code_light)
        }
    }

    pub fn log_colors(&self) -> [Color32; 4] {
        let custom = if self.dark {
            &self.log_dark
        } else {
            &self.log_light
        };
        LOG_PARTS.map(|(key, _, dark, light)| match custom.get(key) {
            Some([r, g, b]) => Color32::from_rgb(*r, *g, *b),
            None => {
                let hex = if self.dark { dark } else { light };
                Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
            }
        })
    }

    pub fn clamp(&mut self) {
        self.ui_size = self.ui_size.clamp(10.0, 20.0);
        self.font_size = self.font_size.clamp(8.0, 40.0);
        self.line_spacing = self.line_spacing.clamp(1.0, 3.0);
        self.radius = self.radius.min(16);
        self.accent = self.accent.min(theme::ACCENTS.len() - 1);
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(default)]
pub struct Edit {
    pub spaces: bool,
    pub indent: u8,
    pub line_numbers: bool,
    pub current_line: bool,
    pub blink: bool,
    pub suggest: bool,
    pub hints: bool,
}

impl Default for Edit {
    fn default() -> Self {
        Self {
            spaces: false,
            indent: 4,
            line_numbers: true,
            current_line: true,
            blink: true,
            suggest: true,
            hints: true,
        }
    }
}

impl Edit {
    pub fn unit(&self) -> String {
        if self.spaces {
            " ".repeat(self.indent.clamp(1, 8) as usize)
        } else {
            "\t".to_owned()
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub look: Look,
    pub wrap: bool,
    pub recent: Vec<PathBuf>,
    pub open: Vec<(u64, PathBuf)>,
    pub active: usize,
    pub floats: crate::dock::Floats,
    pub keys: BTreeMap<String, String>,
    pub detached: Vec<crate::dock::Pane>,
    pub project: Option<PathBuf>,
    pub recent_projects: Vec<PathBuf>,
    pub project_tree: bool,
    pub remote_tree: bool,
    pub project_size: bool,
    pub project_date: bool,
    pub dictionaries: BTreeMap<String, String>,
    pub hosts: usize,
    pub tab_colors: BTreeMap<String, u8>,
    pub edit: Edit,
    pub restore: bool,
    pub scrollback: u32,
    pub db_rows: u32,
    pub columns: [f32; 3],
    pub pinned: Vec<PathBuf>,
    pub section: usize,
    migrated: bool,
    session: Vec<PathBuf>,
    dark: bool,
    accent: usize,
    font_size: f32,
    line_spacing: f32,
}

impl Default for Settings {
    fn default() -> Self {
        let look = Look::default();
        Self {
            wrap: false,
            recent: Vec::new(),
            open: Vec::new(),
            active: 0,
            floats: Vec::new(),
            keys: BTreeMap::new(),
            detached: Vec::new(),
            project: None,
            recent_projects: Vec::new(),
            project_tree: true,
            remote_tree: false,
            project_size: false,
            project_date: false,
            dictionaries: BTreeMap::new(),
            hosts: 1,
            tab_colors: BTreeMap::new(),
            edit: Edit::default(),
            restore: true,
            scrollback: 5000,
            db_rows: 5000,
            columns: crate::widgets::COLUMN_DEFAULTS,
            pinned: Vec::new(),
            section: 0,
            migrated: false,
            session: Vec::new(),
            dark: look.dark,
            accent: look.accent,
            font_size: look.font_size,
            line_spacing: look.line_spacing,
            look,
        }
    }
}

impl Settings {
    pub fn migrate(&mut self) {
        if !self.migrated {
            self.migrated = true;
            self.look.dark = self.dark;
            self.look.accent = self.accent;
            self.look.font_size = self.font_size;
            self.look.line_spacing = self.line_spacing;
            for (i, path) in std::mem::take(&mut self.session).into_iter().enumerate() {
                self.open.push((i as u64 + 1, path));
            }
        }
        self.look.clamp();
        self.scrollback = self.scrollback.clamp(500, 100_000);
        self.db_rows = self.db_rows.clamp(100, 100_000);
        self.edit.indent = self.edit.indent.clamp(1, 8);
    }
}

pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if let Some(body) = lower
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<u8> = body
            .split([',', ' '])
            .filter(|p| !p.is_empty())
            .map(|p| p.parse::<u8>())
            .collect::<Result<_, _>>()
            .ok()?;
        return <[u8; 3]>::try_from(parts).ok();
    }
    let hex = lower.strip_prefix('#').unwrap_or(&lower);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let mut out = [0; 3];
            for (i, c) in hex.chars().enumerate() {
                out[i] = value(&format!("{c}{c}"))?;
            }
            Some(out)
        }
        6 => Some([value(&hex[0..2])?, value(&hex[2..4])?, value(&hex[4..6])?]),
        _ => None,
    }
}

fn color_field(ui: &mut Ui, key: &str, color: &mut [u8; 3]) -> bool {
    let mut changed = ui.color_edit_button_srgb(color).changed();
    let id = ui.scope_id().with(("tror_hex", key));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]));
    let field = ui
        .add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(92.0)
                .font(egui::TextStyle::Monospace),
        )
        .on_hover_text("#rrggbb, #rgb или rgb(r, g, b)");
    if field.changed()
        && let Some(parsed) = parse_color(&text)
    {
        *color = parsed;
        changed = true;
    }
    if field.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    changed
}

fn heading(ui: &mut Ui, pal: &Palette, text: &str, hint: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(text).family(theme::medium()).size(15.5));
    if !hint.is_empty() {
        ui.label(RichText::new(hint).size(12.0).color(pal.faint));
    }
    ui.add_space(6.0);
}

fn font_row(ui: &mut Ui, pal: &Palette, label: &str, builtin: &str, font: &mut Option<PathBuf>) {
    ui.label(label);
    ui.horizontal(|ui| {
        let name = font
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{builtin} (встроенный)"));
        ui.label(RichText::new(name).color(pal.subtext));
        if ui
            .add(Button::new("Выбрать файл…").fill(pal.surface))
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Шрифты", &["ttf", "otf", "ttc"])
                .pick_file()
        {
            *font = Some(path);
        }
        if font.is_some() && ui.button("Сбросить").clicked() {
            *font = None;
        }
    });
    ui.end_row();
}

pub struct KeysEdit<'a> {
    pub custom: &'a mut BTreeMap<String, String>,
    pub capturing: &'a mut Option<usize>,
}

const SECTIONS: [&str; 9] = [
    "Тема и цвета",
    "Шрифты и размеры",
    "Подсветка кода",
    "Редактор",
    "Вкладки и панели",
    "Файлы и хосты",
    "Консоль и журнал",
    "База данных",
    "Горячие клавиши",
];

pub fn ui(
    ui: &mut Ui,
    pal: &Palette,
    settings: &mut Settings,
    capturing: &mut Option<usize>,
) -> Option<String> {
    let mut note = None;
    let ctx = ui.ctx().clone();
    settings.section = settings.section.min(SECTIONS.len() - 1);
    let wide = ui.available_width() >= 560.0;
    let mut page = |ui: &mut Ui, settings: &mut Settings| {
        ScrollArea::vertical()
            .id_salt(("tror_prefs", settings.section))
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(22, 4))
                    .show(ui, |ui| {
                        ui.set_max_width(680.0);
                        match settings.section {
                            0..=2 => body(
                                ui,
                                pal,
                                settings.section,
                                &mut settings.look,
                                &ctx,
                                &mut note,
                            ),
                            3 => editor_section(ui, pal, settings),
                            4 => tabs_section(ui, pal, settings),
                            5 => files_section(ui, pal, settings),
                            6 => {
                                console_section(ui, pal, settings);
                                log_section(ui, pal, &mut settings.look);
                            }
                            7 => db_section(ui, pal, settings),
                            _ => keys_section(
                                ui,
                                pal,
                                KeysEdit {
                                    custom: &mut settings.keys,
                                    capturing,
                                },
                                &mut note,
                            ),
                        }
                        ui.add_space(24.0);
                    });
            });
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(
                egui::vec2(190.0, ui.available_height()),
                egui::Layout::top_down_justified(egui::Align::Min),
                |ui| {
                    ui.add_space(14.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                    for (index, name) in SECTIONS.iter().enumerate() {
                        let on = settings.section == index;
                        let text =
                            RichText::new(*name).color(if on { pal.text } else { pal.subtext });
                        let button = Button::new(text).frame(true).fill(if on {
                            pal.surface
                        } else {
                            Color32::TRANSPARENT
                        });
                        if ui.add(button).clicked() {
                            settings.section = index;
                        }
                    }
                },
            );
            ui.vertical(|ui| page(ui, settings));
        });
    } else {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ComboBox::from_id_salt("tror_prefs_section")
                .width(220.0)
                .selected_text(SECTIONS[settings.section])
                .show_ui(ui, |ui| {
                    for (index, name) in SECTIONS.iter().enumerate() {
                        ui.selectable_value(&mut settings.section, index, *name);
                    }
                });
        });
        page(ui, settings);
    }
    note
}

fn toggle_row(ui: &mut Ui, pal: &Palette, value: &mut bool, label: &str, hint: &str) {
    ui.checkbox(value, label);
    if !hint.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            ui.label(RichText::new(hint).size(12.0).color(pal.faint));
        });
    }
    ui.add_space(4.0);
}

fn editor_section(ui: &mut Ui, pal: &Palette, settings: &mut Settings) {
    heading(ui, pal, "Редактор", "");
    toggle_row(
        ui,
        pal,
        &mut settings.wrap,
        "Переносить длинные строки (Alt+Z)",
        "",
    );
    let edit = &mut settings.edit;
    toggle_row(ui, pal, &mut edit.line_numbers, "Номера строк", "");
    toggle_row(
        ui,
        pal,
        &mut edit.current_line,
        "Подсвечивать строку с курсором",
        "",
    );
    toggle_row(
        ui,
        pal,
        &mut edit.blink,
        "Мигающий курсор",
        "Без мигания окно в покое не перерисовывается совсем.",
    );
    heading(ui, pal, "Отступы", "");
    toggle_row(
        ui,
        pal,
        &mut edit.spaces,
        "Клавиша Tab вставляет пробелы",
        "Иначе вставляется символ табуляции.",
    );
    if edit.spaces {
        ui.horizontal(|ui| {
            ui.label("Пробелов в отступе");
            let mut value = edit.indent as f32;
            if crate::widgets::stepper(ui, pal, "indent", &mut value, (1.0, 8.0), 1.0, "") {
                edit.indent = value as u8;
            }
        });
    }
    heading(ui, pal, "Подсказки", "");
    toggle_row(ui, pal, &mut edit.suggest, "Автодополнение при наборе", "");
    toggle_row(
        ui,
        pal,
        &mut edit.hints,
        "Подсказка параметров функции",
        "Появляется после открывающей скобки.",
    );
    dictionaries_section(ui, pal, &mut settings.dictionaries);
}

fn tabs_section(ui: &mut Ui, pal: &Palette, settings: &mut Settings) {
    heading(ui, pal, "Вкладки", "");
    toggle_row(
        ui,
        pal,
        &mut settings.restore,
        "Открывать при запуске файлы прошлого сеанса",
        "Закреплённые вкладки и их порядок тоже восстанавливаются.",
    );
    ui.label(
        RichText::new(
            "Вкладку можно перетащить по панели вкладок — соседние сразу расступаются. \
             Закреплённые вкладки стоят в начале и меняются местами только между собой. \
             Закрепить, покрасить или вынести вкладку в новую область можно из её меню по правому щелчку.",
        )
        .size(12.0)
        .color(pal.faint),
    );
    heading(
        ui,
        pal,
        "Панели",
        "Любую панель можно перетащить за заголовок: к краю окна или другой панели — она прилипнет и разделит место, \
         на заголовок — станет соседней вкладкой, в сторону — превратится в отдельное окно. Расположение запоминается.",
    );
}

fn console_section(ui: &mut Ui, pal: &Palette, settings: &mut Settings) {
    heading(
        ui,
        pal,
        "Консоль",
        "Цветовая схема и шрифт задаются в профиле подключения и в меню сеанса «⋯ → Оформление сеанса».",
    );
    ui.horizontal(|ui| {
        ui.label("Строк истории в сеансе");
        let mut value = settings.scrollback as f32;
        if crate::widgets::stepper(
            ui,
            pal,
            "scrollback",
            &mut value,
            (500.0, 100_000.0),
            500.0,
            "",
        ) {
            settings.scrollback = value as u32;
        }
    });
    ui.label(
        RichText::new("Действует для сеансов, открытых после изменения.")
            .size(12.0)
            .color(pal.faint),
    );
}

fn db_section(ui: &mut Ui, pal: &Palette, settings: &mut Settings) {
    heading(
        ui,
        pal,
        "База данных",
        "Сколько строк результата запроса загружать в таблицу. Чем больше, тем больше памяти.",
    );
    ui.horizontal(|ui| {
        ui.label("Строк результата");
        let mut value = settings.db_rows as f32;
        if crate::widgets::stepper(
            ui,
            pal,
            "db_rows",
            &mut value,
            (100.0, 100_000.0),
            100.0,
            "",
        ) {
            settings.db_rows = value as u32;
        }
    });
}

fn files_section(ui: &mut Ui, pal: &Palette, settings: &mut Settings) {
    heading(
        ui,
        pal,
        "Структура файлов",
        "Как показывать папки: деревом с раскрывающимися папками или списком одной папки, как в обычном файловом менеджере.",
    );
    egui::Grid::new("tror_prefs_files")
        .num_columns(2)
        .spacing([22.0, 8.0])
        .show(ui, |ui| {
            for (label, value) in [
                ("Папка проекта", &mut settings.project_tree),
                ("Удалённые хосты", &mut settings.remote_tree),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    for (tree, text) in [(true, "Дерево"), (false, "Список")] {
                        let mut button = Button::new(text);
                        if *value == tree {
                            button = button.fill(pal.surface);
                        }
                        if ui.add(button).clicked() {
                            *value = tree;
                        }
                    }
                });
                ui.end_row();
            }
        });
    heading(
        ui,
        pal,
        "Колонки",
        "Ширину колонки меняют перетаскиванием границы в заголовке списка; двойной щелчок по границе возвращает исходную. Колонки удалённых хостов включаются в меню «⋯» панели.",
    );
    toggle_row(
        ui,
        pal,
        &mut settings.project_size,
        "Проект: показывать размер",
        "",
    );
    toggle_row(
        ui,
        pal,
        &mut settings.project_date,
        "Проект: показывать дату изменения",
        "",
    );
    if settings.columns != crate::widgets::COLUMN_DEFAULTS
        && ui
            .add(Button::new("Сбросить ширину колонок").fill(pal.surface))
            .clicked()
    {
        settings.columns = crate::widgets::COLUMN_DEFAULTS;
        crate::widgets::set_columns(settings.columns);
    }
}

fn log_section(ui: &mut Ui, pal: &Palette, look: &mut Look) {
    let mode = if look.dark {
        "тёмной"
    } else {
        "светлой"
    };
    heading(
        ui,
        pal,
        "Цвета журнала",
        &format!("Подсветка строк в журнале обмена с сервером, как в NppFTP. Для {mode} темы."),
    );
    let current = look.log_colors();
    let colors = if look.dark {
        &mut look.log_dark
    } else {
        &mut look.log_light
    };
    egui::Grid::new("tror_prefs_log")
        .num_columns(2)
        .spacing([22.0, 7.0])
        .show(ui, |ui| {
            for (index, (key, label, _, _)) in LOG_PARTS.iter().enumerate() {
                ui.label(*label);
                ui.horizontal(|ui| {
                    let c = current[index];
                    let mut value = [c.r(), c.g(), c.b()];
                    if color_field(ui, &format!("log:{key}"), &mut value) {
                        colors.insert((*key).to_owned(), value);
                    }
                    if colors.contains_key(*key)
                        && ui.button("↺").on_hover_text("Цвет по умолчанию").clicked()
                    {
                        colors.remove(*key);
                    }
                });
                ui.end_row();
            }
        });
}

const DICTIONARY_LANGS: [(&str, &str); 8] = [
    ("all", "Все языки"),
    ("php", "PHP"),
    ("js", "JavaScript"),
    ("css", "CSS"),
    ("html", "HTML и Twig"),
    ("python", "Python"),
    ("rust", "Rust"),
    ("sql", "SQL"),
];

fn dictionaries_section(ui: &mut Ui, pal: &Palette, dictionaries: &mut BTreeMap<String, String>) {
    heading(
        ui,
        pal,
        "Словари подсказок",
        "Свои слова для автодополнения — по одному в строке. Строка вида «имя(параметры)» добавляет и сигнатуру: она появится в подсказке, когда вы наберёте «имя(». После сигнатуры через « — » можно дописать описание.",
    );
    let id = ui.scope_id().with("tror_dict_lang");
    let mut lang: String = ui
        .data(|d| d.get_temp(id))
        .unwrap_or_else(|| "php".to_owned());
    ui.horizontal(|ui| {
        ui.label("Язык");
        ComboBox::from_id_salt("tror_dict_combo")
            .width(180.0)
            .selected_text(
                DICTIONARY_LANGS
                    .iter()
                    .find(|(key, _)| *key == lang)
                    .map_or("", |(_, label)| *label),
            )
            .show_ui(ui, |ui| {
                for (key, label) in DICTIONARY_LANGS {
                    let count = dictionaries
                        .get(key)
                        .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count());
                    let text = if count > 0 {
                        format!("{label} ({count})")
                    } else {
                        label.to_owned()
                    };
                    ui.selectable_value(&mut lang, key.to_owned(), text);
                }
            });
    });
    ui.data_mut(|d| d.insert_temp(id, lang.clone()));
    let text = dictionaries.entry(lang.clone()).or_default();
    ui.add(
        egui::TextEdit::multiline(text)
            .font(egui::TextStyle::Monospace)
            .desired_rows(6)
            .desired_width(f32::INFINITY)
            .hint_text("CIBlockElement::GetList($arOrder, $arFilter, $arGroupBy, $arNavStartParams, $arSelectFields) — элементы инфоблока\nLocalRedirect($url)\nmyHelper"),
    );
    if dictionaries.get(&lang).is_some_and(|t| t.is_empty()) {
        dictionaries.remove(&lang);
    }
}

fn keys_section(ui: &mut Ui, pal: &Palette, keys: KeysEdit, note: &mut Option<String>) {
    use crate::keys::{self, COMMANDS};
    heading(
        ui,
        pal,
        "Горячие клавиши",
        "Нажмите на сочетание и введите новое. Esc — отмена. Подходят сочетания с Ctrl или Alt и клавиши F1–F12.",
    );
    let current: Vec<String> = COMMANDS
        .iter()
        .map(|(id, _, default, _)| {
            keys.custom
                .get(*id)
                .cloned()
                .unwrap_or_else(|| (*default).to_owned())
        })
        .collect();
    if let Some(index) = *keys.capturing {
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(egui::KeyboardShortcut::new(*modifiers, *key)),
                _ => None,
            })
        });
        if let Some(shortcut) = pressed {
            *keys.capturing = None;
            if shortcut.logical_key != egui::Key::Escape {
                let text = keys::format(&shortcut).replace('⌘', "Ctrl");
                let taken = current
                    .iter()
                    .position(|c| keys::parse(c) == keys::parse(&text))
                    .filter(|other| *other != index);
                if !keys::usable(&shortcut) {
                    *note = Some(format!(
                        "{text}: нужно сочетание с Ctrl или Alt либо F1–F12"
                    ));
                } else if let Some(other) = taken {
                    *note = Some(format!("{text} уже занято: {}", COMMANDS[other].1));
                } else {
                    keys.custom.insert(COMMANDS[index].0.to_owned(), text);
                }
            }
        }
    }
    egui::Grid::new("tror_prefs_keys")
        .num_columns(2)
        .spacing([22.0, 6.0])
        .show(ui, |ui| {
            for (index, (id, label, default, _)) in COMMANDS.iter().enumerate() {
                ui.label(*label);
                ui.horizontal(|ui| {
                    let capturing = *keys.capturing == Some(index);
                    let text = if capturing {
                        "нажмите сочетание…".to_owned()
                    } else if current[index].is_empty() {
                        "не задано".to_owned()
                    } else {
                        keys::parse(&current[index])
                            .map(|s| keys::format(&s))
                            .unwrap_or_else(|| current[index].clone())
                    };
                    let color = if capturing {
                        pal.accent
                    } else if current[index].is_empty() {
                        pal.faint
                    } else {
                        pal.text
                    };
                    let button = Button::new(RichText::new(text).color(color))
                        .fill(pal.surface)
                        .min_size(egui::vec2(150.0, 0.0));
                    if ui.add(button).clicked() {
                        *keys.capturing = if capturing { None } else { Some(index) };
                    }
                    if !current[index].is_empty() && ui.button("Убрать").clicked() {
                        keys.custom.insert((*id).to_owned(), String::new());
                    }
                    if keys.custom.contains_key(*id)
                        && ui
                            .button("↺")
                            .on_hover_text(if default.is_empty() {
                                "По умолчанию: не задано".to_owned()
                            } else {
                                format!("По умолчанию: {default}")
                            })
                            .clicked()
                    {
                        keys.custom.remove(*id);
                    }
                });
                ui.end_row();
            }
        });
}

fn body(
    ui: &mut Ui,
    pal: &Palette,
    section: usize,
    look: &mut Look,
    ctx: &egui::Context,
    note: &mut Option<String>,
) {
    let dark = look.dark;
    let mode = if dark {
        "тёмной"
    } else {
        "светлой"
    };

    if section == 0 {
        heading(ui, pal, "Тема", "");
        egui::Grid::new("tror_prefs_theme")
            .num_columns(2)
            .spacing([22.0, 10.0])
            .show(ui, |ui| {
                ui.label("Режим");
                ui.horizontal(|ui| {
                    for (value, label) in [(true, "Тёмная"), (false, "Светлая")] {
                        let mut button = Button::new(label);
                        if look.dark == value {
                            button = button.fill(pal.surface);
                        }
                        if ui.add(button).clicked() {
                            look.dark = value;
                        }
                    }
                });
                ui.end_row();
                ui.label("Готовые акценты");
                ui.horizontal(|ui| {
                    for (i, (name, on_dark, on_light)) in theme::ACCENTS.iter().enumerate() {
                        let hex = if dark { *on_dark } else { *on_light };
                        let color =
                            Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::click());
                        ui.painter().circle_filled(rect.center(), 9.0, color);
                        let colors = if dark {
                            &mut look.colors_dark
                        } else {
                            &mut look.colors_light
                        };
                        if look.accent == i && !colors.contains_key("accent") {
                            ui.painter().circle_stroke(
                                rect.center(),
                                12.0,
                                Stroke::new(1.5, pal.text),
                            );
                        }
                        if response.on_hover_text(*name).clicked() {
                            look.accent = i;
                            colors.remove("accent");
                        }
                    }
                });
                ui.end_row();
            });

        heading(
            ui,
            pal,
            "Цвета интерфейса",
            &format!(
                "Настраиваются отдельно для {mode} темы. Цвет можно выбрать в палитре или ввести: #rrggbb, #rgb, rgb(r, g, b)."
            ),
        );
        let mut current = *pal;
        let colors = if dark {
            &mut look.colors_dark
        } else {
            &mut look.colors_light
        };
        egui::Grid::new("tror_prefs_colors")
            .num_columns(2)
            .spacing([22.0, 7.0])
            .show(ui, |ui| {
                for (index, (key, label)) in PARTS.iter().enumerate() {
                    ui.label(*label);
                    ui.horizontal(|ui| {
                        let c = *current.part(index);
                        let mut value = [c.r(), c.g(), c.b()];
                        if color_field(ui, key, &mut value) {
                            colors.insert((*key).to_owned(), value);
                        }
                        if colors.contains_key(*key)
                            && ui.button("↺").on_hover_text("Вернуть цвет темы").clicked()
                        {
                            colors.remove(*key);
                        }
                    });
                    ui.end_row();
                }
            });
        if !colors.is_empty() && ui.button("Сбросить все цвета интерфейса").clicked()
        {
            colors.clear();
        }
    }
    if section == 2 {
        heading(
            ui,
            pal,
            "Подсветка кода",
            &format!("Схема и собственные цвета для {mode} темы."),
        );
        let (scheme, code) = if dark {
            (&mut look.syntax_dark, &mut look.code_dark)
        } else {
            (&mut look.syntax_light, &mut look.code_light)
        };
        egui::Grid::new("tror_prefs_code")
            .num_columns(2)
            .spacing([22.0, 7.0])
            .show(ui, |ui| {
                ui.label("Цветовая схема");
                ComboBox::from_id_salt("tror_prefs_scheme")
                    .width(240.0)
                    .selected_text(scheme.as_str())
                    .show_ui(ui, |ui| {
                        for name in highlight::theme_names() {
                            ui.selectable_value(scheme, name.to_owned(), name);
                        }
                    });
                ui.end_row();
                for (key, label) in SYNTAX_PARTS {
                    ui.label(label);
                    ui.horizontal(|ui| match code.get(key).copied() {
                        Some(mut value) => {
                            if color_field(ui, &format!("code:{key}"), &mut value) {
                                code.insert(key.to_owned(), value);
                            }
                            if ui.button("↺").on_hover_text("Как в схеме").clicked() {
                                code.remove(key);
                            }
                        }
                        None => {
                            ui.label(RichText::new("как в схеме").color(pal.faint));
                            if ui.button("Задать цвет").clicked() {
                                code.insert(
                                    key.to_owned(),
                                    [pal.text.r(), pal.text.g(), pal.text.b()],
                                );
                            }
                        }
                    });
                    ui.end_row();
                }
            });
    }
    if section == 1 {
        heading(
            ui,
            pal,
            "Шрифты и размеры",
            "Подойдёт любой файл TTF или OTF. Для кода лучше моноширинный шрифт.",
        );
        egui::Grid::new("tror_prefs_fonts")
            .num_columns(2)
            .spacing([22.0, 10.0])
            .show(ui, |ui| {
                font_row(ui, pal, "Шрифт интерфейса", "Inter", &mut look.ui_font);
                font_row(ui, pal, "Шрифт кода", "JetBrains Mono", &mut look.code_font);
                ui.label("Размер текста интерфейса");
                crate::widgets::stepper(
                    ui,
                    pal,
                    "ui_size",
                    &mut look.ui_size,
                    (11.0, 18.0),
                    0.5,
                    "px",
                );
                ui.end_row();
                ui.label("Размер шрифта кода");
                crate::widgets::stepper(
                    ui,
                    pal,
                    "font_size",
                    &mut look.font_size,
                    (9.0, 28.0),
                    1.0,
                    "px",
                );
                ui.end_row();
                ui.label("Межстрочный интервал");
                crate::widgets::stepper(
                    ui,
                    pal,
                    "line_spacing",
                    &mut look.line_spacing,
                    (1.1, 2.2),
                    0.05,
                    "",
                );
                ui.end_row();
                ui.label("Масштаб всего окна");
                let mut zoom = ctx.zoom_factor();
                if crate::widgets::stepper(ui, pal, "zoom", &mut zoom, (0.7, 2.0), 0.05, "") {
                    ctx.set_zoom_factor(zoom);
                }
                ui.end_row();
                ui.label("Скругление углов");
                let mut radius = look.radius as f32;
                if crate::widgets::stepper(ui, pal, "radius", &mut radius, (0.0, 14.0), 1.0, "px") {
                    look.radius = radius as u8;
                }
                ui.end_row();
            });
    }
    if section == 0 {
        heading(ui, pal, "Оформление целиком", "");
        ui.horizontal(|ui| {
            if ui
                .add(Button::new("Экспорт в файл…").fill(pal.surface))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("tror-theme.json")
                    .save_file()
            {
                let result = serde_json::to_string_pretty(look)
                    .map_err(|e| e.to_string())
                    .and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string()));
                *note = Some(match result {
                    Ok(()) => format!("Оформление сохранено: {}", path.display()),
                    Err(e) => format!("Не удалось сохранить оформление: {e}"),
                });
            }
            if ui
                .add(Button::new("Импорт из файла…").fill(pal.surface))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Оформление Tror", &["json"])
                    .pick_file()
            {
                let loaded = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| {
                        serde_json::from_str::<Look>(&text).map_err(|e| e.to_string())
                    });
                *note = Some(match loaded {
                    Ok(mut loaded) => {
                        loaded.clamp();
                        *look = loaded;
                        "Оформление загружено".to_owned()
                    }
                    Err(e) => format!("Не удалось прочитать оформление: {e}"),
                });
            }
            if ui
                .add(Button::new("Сбросить всё оформление").fill(pal.surface))
                .clicked()
            {
                *look = Look {
                    dark: look.dark,
                    ..Look::default()
                };
                ctx.set_zoom_factor(1.0);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_from_hex_and_rgb() {
        assert_eq!(parse_color("#8ea6ff"), Some([0x8e, 0xa6, 0xff]));
        assert_eq!(parse_color("8EA6FF"), Some([0x8e, 0xa6, 0xff]));
        assert_eq!(parse_color("#fa0"), Some([0xff, 0xaa, 0x00]));
        assert_eq!(parse_color("rgb(12, 200,7)"), Some([12, 200, 7]));
        assert_eq!(parse_color("RGB(1 2 3)"), Some([1, 2, 3]));
        assert_eq!(parse_color("rgb(1,2)"), None);
        assert_eq!(parse_color("rgb(1,2,300)"), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#ggg"), None);
        assert_eq!(parse_color("#ёёё"), None);
    }

    #[test]
    fn overrides_change_palette_and_look_roundtrips() {
        let mut look = Look::default();
        let before = look.palette();
        look.colors_dark.insert("base".into(), [1, 2, 3]);
        look.colors_dark.insert("accent".into(), [9, 8, 7]);
        look.colors_light.insert("base".into(), [200, 200, 200]);
        let after = look.palette();
        assert_eq!(after.base, Color32::from_rgb(1, 2, 3));
        assert_eq!(after.accent, Color32::from_rgb(9, 8, 7));
        assert_eq!(after.text, before.text);
        look.dark = false;
        assert_eq!(look.palette().base, Color32::from_rgb(200, 200, 200));
        let text = serde_json::to_string(&look).unwrap();
        assert!(serde_json::from_str::<Look>(&text).unwrap() == look);
    }

    #[test]
    fn old_flat_settings_are_migrated() {
        let old = r#"(dark:false,accent:3,font_size:17.0,line_spacing:1.8,wrap:true,remote_panel:true,terminal_panel:false,recent:["/tmp/r.css"],session:["/tmp/a.php","/tmp/b.js"],active:1)"#;
        let mut settings: Settings = ron::from_str(old).unwrap();
        settings.migrate();
        assert!(!settings.look.dark && settings.wrap);
        assert_eq!(settings.look.accent, 3);
        assert_eq!(settings.look.font_size, 17.0);
        assert_eq!(settings.look.line_spacing, 1.8);
        assert_eq!(settings.recent, [PathBuf::from("/tmp/r.css")]);
        assert_eq!(settings.open.len(), 2);
        assert_eq!(settings.open[1], (2, PathBuf::from("/tmp/b.js")));

        settings.look.dark = true;
        settings.look.colors_dark.insert("base".into(), [1, 2, 3]);
        settings.look.ui_font = Some(PathBuf::from("/fonts/My.ttf"));
        let saved = ron::ser::to_string(&settings).unwrap();
        let mut again: Settings = ron::from_str(&saved).unwrap();
        again.migrate();
        assert!(again.look == settings.look);
        assert_eq!(again.open, settings.open);

        let mut fresh = Settings::default();
        fresh.migrate();
        assert!(fresh.look == Look::default());
    }
}
