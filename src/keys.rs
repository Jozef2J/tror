use std::collections::BTreeMap;

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

pub const COMMANDS: [(&str, &str, &str, bool); 43] = [
    ("new", "Новый файл", "Ctrl+N", true),
    ("open", "Открыть файл", "Ctrl+O", true),
    ("open_folder", "Открыть папку", "Ctrl+Shift+O", true),
    ("save", "Сохранить", "Ctrl+S", true),
    ("save_as", "Сохранить как", "Ctrl+Shift+S", true),
    ("save_all", "Сохранить все", "", true),
    ("close_tab", "Закрыть вкладку", "Ctrl+W", true),
    ("reopen", "Открыть закрытую вкладку", "Ctrl+Shift+T", true),
    ("next_tab", "Следующая вкладка", "Ctrl+Tab", true),
    ("prev_tab", "Предыдущая вкладка", "Ctrl+Shift+Tab", true),
    ("duplicate_line", "Дублировать строку", "Ctrl+Shift+D", true),
    ("delete_line", "Удалить строку", "Ctrl+Shift+K", true),
    ("line_up", "Переместить строку вверх", "Alt+Up", true),
    ("line_down", "Переместить строку вниз", "Alt+Down", true),
    ("upper", "Регистр: ПРОПИСНЫЕ", "Ctrl+Shift+U", true),
    ("lower", "Регистр: строчные", "Ctrl+U", true),
    ("trim", "Удалить пробелы в конце строк", "", true),
    ("bookmark", "Закладка: поставить или снять", "Ctrl+F2", true),
    ("bookmark_next", "Следующая закладка", "F2", true),
    ("bookmark_prev", "Предыдущая закладка", "Shift+F2", true),
    ("find", "Найти", "Ctrl+F", true),
    ("replace", "Заменить", "Ctrl+H", true),
    ("find_next", "Найти далее", "F3", true),
    ("find_prev", "Найти ранее", "Shift+F3", true),
    ("goto", "Перейти к строке", "Ctrl+G", true),
    ("fold", "Свернуть блок", "Ctrl+Alt+F", true),
    ("unfold", "Развернуть блок", "Ctrl+Alt+Shift+F", true),
    ("fold_all", "Свернуть всё", "Alt+0", true),
    ("unfold_all", "Развернуть всё", "", true),
    ("find_files", "Найти в файлах проекта", "Ctrl+Shift+F", true),
    ("macro_record", "Запись макроса: начать или остановить", "Ctrl+Shift+R", true),
    ("macro_play", "Выполнить макрос", "Ctrl+Shift+P", true),
    ("wrap", "Перенос строк", "Alt+Z", false),
    ("theme", "Сменить тему", "", false),
    ("settings", "Настройки", "Ctrl+Comma", false),
    ("project", "Папка проекта", "F7", false),
    ("hosts", "Удалённые хосты", "F8", false),
    ("console", "Консоль", "F9", false),
    ("log", "Журнал", "", false),
    ("server_search", "Поиск на сервере", "", false),
    ("db", "База данных", "", false),
    ("compare", "Сравнить файлы", "", false),
    ("preview", "Предпросмотр в браузере", "F12", false),
];

pub fn parse(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "⌘" => modifiers |= Modifiers::COMMAND,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            _ => key = Some(Key::from_name(part)?),
        }
    }
    Some(KeyboardShortcut::new(modifiers, key?))
}

pub fn format(shortcut: &KeyboardShortcut) -> String {
    let mut parts = Vec::new();
    let m = shortcut.modifiers;
    if m.command || m.ctrl || m.mac_cmd {
        parts.push(if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        });
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    parts.push(shortcut.logical_key.name());
    parts.join("+")
}

pub fn usable(shortcut: &KeyboardShortcut) -> bool {
    let m = shortcut.modifiers;
    let plain = !(m.command || m.ctrl || m.mac_cmd || m.alt);
    let function =
        shortcut.logical_key.name().starts_with('F') && shortcut.logical_key.name().len() > 1;
    !plain || function
}

pub fn weight(s: &KeyboardShortcut) -> u8 {
    let m = s.modifiers;
    m.shift as u8 + m.alt as u8 + (m.command || m.ctrl || m.mac_cmd) as u8
}

pub struct Keys {
    bound: Vec<(usize, KeyboardShortcut)>,
}

impl Keys {
    pub fn new(custom: &BTreeMap<String, String>) -> Self {
        let mut bound: Vec<(usize, KeyboardShortcut)> = COMMANDS
            .iter()
            .enumerate()
            .filter_map(|(index, (id, _, default, _))| {
                let text = custom.get(*id).map(String::as_str).unwrap_or(default);
                Some((index, parse(text)?))
            })
            .collect();
        bound.sort_by_key(|(_, shortcut)| std::cmp::Reverse(weight(shortcut)));
        Self { bound }
    }

    pub fn bound(&self) -> &[(usize, KeyboardShortcut)] {
        &self.bound
    }

    pub fn label(&self, id: &str) -> String {
        self.bound
            .iter()
            .find(|(index, _)| COMMANDS[*index].0 == id)
            .map(|(_, shortcut)| format(shortcut))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_format_and_override() {
        let s = parse("Ctrl+Shift+S").unwrap();
        assert!(s.modifiers.command && s.modifiers.shift && !s.modifiers.alt);
        assert_eq!(s.logical_key, Key::S);
        assert_eq!(parse(&format(&s).replace('⌘', "Ctrl")), Some(s));
        assert_eq!(parse("F8").unwrap().logical_key, Key::F8);
        assert_eq!(parse("Ctrl+Comma").unwrap().logical_key, Key::Comma);
        assert_eq!(parse(""), None);
        assert_eq!(parse("Ctrl+"), None);
        assert_eq!(parse("Ctrl+Nonsense"), None);
        assert!(usable(&parse("F5").unwrap()) && usable(&parse("Alt+Z").unwrap()));
        assert!(!usable(&parse("A").unwrap()) && !usable(&parse("Shift+A").unwrap()));

        for (id, _, default, _) in COMMANDS {
            assert!(default.is_empty() || parse(default).is_some(), "{id}");
        }
        let mut custom = BTreeMap::new();
        custom.insert("save".to_owned(), "Ctrl+Alt+S".to_owned());
        custom.insert("hosts".to_owned(), String::new());
        custom.insert("db".to_owned(), "F6".to_owned());
        let keys = Keys::new(&custom);
        assert!(keys.label("save").ends_with("+Alt+S"));
        assert_eq!(keys.label("hosts"), "");
        assert_eq!(keys.label("db"), "F6");
        assert_eq!(keys.label("console"), "F9");
        let first = keys.bound()[0].1;
        assert!(first.modifiers.shift || first.modifiers.alt);
    }
}
