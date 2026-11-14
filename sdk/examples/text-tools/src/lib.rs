use trorpad_plugin::{Answer, Plugin, Value, host, text, ui};

const CASES: [&str; 6] = [
    "ВЕРХНИЙ РЕГИСТР",
    "нижний регистр",
    "Каждое Слово С Заглавной",
    "camelCase",
    "snake_case",
    "kebab-case",
];

#[derive(Default)]
struct Tools {
    words: Vec<String>,
    watching: bool,
}

trorpad_plugin::export!(Tools);

fn order(a: &str, b: &str, numeric: bool) -> std::cmp::Ordering {
    if numeric && let (Ok(x), Ok(y)) = (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        return x.total_cmp(&y);
    }
    a.to_lowercase().cmp(&b.to_lowercase())
}

fn transform(op: &str, body: &str, numeric: bool) -> String {
    let eol = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<&str> = body.split(eol).collect();
    match op {
        "sort" => lines.sort_by(|a, b| order(a, b, numeric)),
        "unique" => {
            let mut seen = std::collections::HashSet::new();
            lines.retain(|l| seen.insert(*l));
        }
        _ => lines.reverse(),
    }
    lines.join(eol)
}

fn tokens(source: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut word = String::new();
    let mut count = 0;
    for (index, c) in source.chars().enumerate() {
        count = index + 1;
        if c.is_alphanumeric() || c == '_' {
            start.get_or_insert(index);
            word.push(c);
        } else if let Some(from) = start.take() {
            out.push((from, index, std::mem::take(&mut word)));
        }
    }
    if let Some(from) = start {
        out.push((from, count, word));
    }
    out
}

fn split_words(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in source.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_lowercase() || c.is_numeric();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

fn recase(source: &str, style: usize) -> String {
    match style {
        0 => source.to_uppercase(),
        1 => source.to_lowercase(),
        2 => {
            let mut out = String::new();
            let mut start = true;
            for c in source.chars() {
                if start && c.is_alphanumeric() {
                    out.extend(c.to_uppercase());
                    start = false;
                } else {
                    out.extend(c.to_lowercase());
                    if !c.is_alphanumeric() {
                        start = true;
                    }
                }
            }
            out
        }
        3 => split_words(source)
            .iter()
            .enumerate()
            .map(|(i, w)| if i == 0 { w.clone() } else { capital(w) })
            .collect(),
        4 => split_words(source).join("_"),
        _ => split_words(source).join("-"),
    }
}

fn target() -> Result<(String, usize, usize), String> {
    let all = host::text()?;
    let sel = host::selection()?;
    let (from, to) = if sel.is_empty() {
        (0, text::chars(&all))
    } else {
        (sel.start, sel.end)
    };
    Ok((all, from, to))
}

fn put(from: usize, to: usize, old: &str, new: &str) -> Result<bool, String> {
    if old == new {
        return Ok(false);
    }
    host::replace(from, to, new)?;
    host::select(from, from + text::chars(new))?;
    Ok(true)
}

fn lines(op: &str) -> Result<(), String> {
    let numeric = host::setting::<bool>("numeric").unwrap_or(true);
    let (all, from, to) = target()?;
    let (from, to) = text::lines_span(&all, from, to);
    let body = text::slice(&all, from, to);
    let count = body.lines().count();
    let changed = put(from, to, body, &transform(op, body, numeric))?;
    host::notify(&if changed {
        format!("Строк обработано: {count}")
    } else {
        "Изменений нет".to_owned()
    })
}

fn json() -> Result<(), String> {
    use serde::Serialize;
    let (all, from, to) = target()?;
    let body = text::slice(&all, from, to);
    let value: Value = serde_json::from_str(body.trim()).map_err(|e| format!("не JSON: {e}"))?;
    let indent = match host::setting::<String>("indent")
        .unwrap_or_default()
        .as_str()
    {
        "2" => b"  ".to_vec(),
        "tab" => b"\t".to_vec(),
        _ => b"    ".to_vec(),
    };
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(&indent);
    let mut writer = serde_json::Serializer::with_formatter(&mut out, formatter);
    value.serialize(&mut writer).map_err(|e| e.to_string())?;
    let pretty = String::from_utf8(out).map_err(|e| e.to_string())?;
    put(from, to, body, &pretty)?;
    Ok(())
}

fn stats() -> Result<(), String> {
    let (all, from, to) = target()?;
    let body = text::slice(&all, from, to);
    host::notify(&format!(
        "Строк: {} · слов: {} · символов: {}",
        body.lines().count(),
        body.split_whitespace().count(),
        text::chars(body)
    ))
}

fn selected() -> Result<String, String> {
    let chosen = host::selected_text()?;
    if chosen.is_empty() {
        return Err("выделите текст".into());
    }
    Ok(chosen)
}

impl Tools {
    fn refresh(&mut self) -> Result<(), String> {
        let top = host::setting::<f64>("top")
            .unwrap_or(15.0)
            .clamp(1.0, 200.0) as usize;
        let (source, path) = match host::text() {
            Ok(source) => (
                source,
                host::info()?.path.unwrap_or_else(|| "без имени".into()),
            ),
            Err(_) => (String::new(), "нет открытого файла".into()),
        };
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (_, _, word) in tokens(&source) {
            if word.chars().count() >= 3 {
                *counts.entry(word.to_lowercase()).or_default() += 1;
            }
        }
        let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(top);
        self.words = ranked.iter().map(|(w, _)| w.clone()).collect();
        let items: Vec<String> = ranked.iter().map(|(w, n)| format!("{w} — {n}")).collect();
        let mut body = vec![
            ui::heading("Частые слова"),
            ui::small(&path),
            ui::space(4.0),
        ];
        if items.is_empty() {
            body.push(ui::text("Слов не найдено"));
        } else {
            body.push(ui::list("word", &items, None));
        }
        body.extend([
            ui::space(6.0),
            ui::row(vec![ui::primary("refresh", "Обновить")]),
            ui::checkbox("watch", "Обновлять при сохранении", self.watching),
            ui::small("Щелчок по слову выделяет первое вхождение"),
        ]);
        host::panel("words", ui::column(body))
    }
}

impl Plugin for Tools {
    fn command(&mut self, id: &str) -> Result<(), String> {
        match id {
            "sort" | "unique" | "reverse" => lines(id),
            "json" => json(),
            "stats" => stats(),
            "wrap" => {
                selected()?;
                let last = host::storage_get::<String>("tag")?.unwrap_or_else(|| "div".into());
                host::prompt("wrap", "Обернуть выделение в тег", &last)
            }
            "case" => {
                selected()?;
                host::pick("case", "Регистр выделенного текста", &CASES)
            }
            other => Err(format!("нет команды «{other}»")),
        }
    }

    fn answer(&mut self, id: &str, answer: Answer) -> Result<(), String> {
        let chosen = selected()?;
        match id {
            "wrap" => {
                let open = answer.value.trim();
                let name = open.split_whitespace().next().unwrap_or_default();
                if name.is_empty() {
                    return Err("пустой тег".into());
                }
                host::storage_set("tag", &open)?;
                host::replace_selection(&format!("<{open}>{chosen}</{name}>"))
            }
            "case" => {
                let style = answer.index.unwrap_or(0);
                let sel = host::selection()?;
                put(sel.start, sel.end, &chosen, &recase(&chosen, style)).map(|_| ())
            }
            _ => Ok(()),
        }
    }

    fn panel_open(&mut self, _panel: &str) -> Result<(), String> {
        self.refresh()
    }

    fn panel_event(&mut self, _panel: &str, id: &str, value: &Value) -> Result<(), String> {
        match id {
            "refresh" => self.refresh(),
            "watch" => {
                self.watching = value.as_bool().unwrap_or(false);
                Ok(())
            }
            "word" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let Some(word) = self.words.get(index) else {
                    return Ok(());
                };
                let source = host::text()?;
                match tokens(&source)
                    .into_iter()
                    .find(|t| t.2.to_lowercase() == *word)
                {
                    Some((from, to, _)) => host::select(from, to),
                    None => host::notify("слово больше не встречается"),
                }
            }
            _ => Ok(()),
        }
    }

    fn event(&mut self, name: &str, params: &Value) -> Result<(), String> {
        match name {
            "saved" => {
                let count = host::storage_get::<u64>("saves")?.unwrap_or(0) + 1;
                host::storage_set("saves", &count)?;
                let path = params.get("path").and_then(Value::as_str).unwrap_or("?");
                host::log(&format!("сохранён {path}, всего сохранений: {count}"))?;
                if self.watching {
                    self.refresh()?;
                }
                Ok(())
            }
            "settings" if !self.words.is_empty() => self.refresh(),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{recase, tokens, transform};

    #[test]
    fn line_operations() {
        assert_eq!(transform("sort", "b\nA\nc", false), "A\nb\nc");
        assert_eq!(transform("sort", "10\n9\nx", true), "9\n10\nx");
        assert_eq!(transform("sort", "10\n9", false), "10\n9");
        assert_eq!(transform("unique", "a\r\nb\r\na", false), "a\r\nb");
        assert_eq!(transform("reverse", "1\n2\n3", false), "3\n2\n1");
    }

    #[test]
    fn cases() {
        assert_eq!(recase("hello World", 0), "HELLO WORLD");
        assert_eq!(recase("привет мир", 2), "Привет Мир");
        assert_eq!(recase("user_name id", 3), "userNameId");
        assert_eq!(recase("userName", 4), "user_name");
        assert_eq!(recase("Привет мир", 5), "привет-мир");
    }

    #[test]
    fn tokens_are_char_positions() {
        assert_eq!(
            tokens("ёж, cat"),
            vec![(0, 2, "ёж".into()), (4, 7, "cat".into())]
        );
    }
}
