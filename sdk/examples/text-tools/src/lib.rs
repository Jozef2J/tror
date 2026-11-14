use trorpad_plugin::{Plugin, Value, host, text};

#[derive(Default)]
struct Tools;

trorpad_plugin::export!(Tools);

fn transform(op: &str, body: &str) -> String {
    let eol = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<&str> = body.split(eol).collect();
    match op {
        "sort" => lines.sort_by_key(|l| l.to_lowercase()),
        "unique" => {
            let mut seen = std::collections::HashSet::new();
            lines.retain(|l| seen.insert(*l));
        }
        _ => lines.reverse(),
    }
    lines.join(eol)
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
    let (all, from, to) = target()?;
    let (from, to) = text::lines_span(&all, from, to);
    let body = text::slice(&all, from, to);
    let count = body.lines().count();
    let changed = put(from, to, body, &transform(op, body))?;
    host::notify(&if changed {
        format!("Строк обработано: {count}")
    } else {
        "Изменений нет".to_owned()
    })
}

fn json() -> Result<(), String> {
    let (all, from, to) = target()?;
    let body = text::slice(&all, from, to);
    let value: Value = serde_json::from_str(body.trim()).map_err(|e| format!("не JSON: {e}"))?;
    let pretty = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
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

impl Plugin for Tools {
    fn command(&mut self, id: &str) -> Result<(), String> {
        match id {
            "sort" | "unique" | "reverse" => lines(id),
            "json" => json(),
            "stats" => stats(),
            other => Err(format!("нет команды «{other}»")),
        }
    }

    fn event(&mut self, name: &str, params: &Value) -> Result<(), String> {
        if name != "saved" {
            return Ok(());
        }
        let count = host::storage_get::<u64>("saves")?.unwrap_or(0) + 1;
        host::storage_set("saves", &count)?;
        let path = params.get("path").and_then(Value::as_str).unwrap_or("?");
        host::log(&format!("сохранён {path}, всего сохранений: {count}"))
    }
}

#[cfg(test)]
mod tests {
    use super::transform;

    #[test]
    fn line_operations() {
        assert_eq!(transform("sort", "b\nA\nc"), "A\nb\nc");
        assert_eq!(transform("unique", "a\r\nb\r\na"), "a\r\nb");
        assert_eq!(transform("reverse", "1\n2\n3"), "3\n2\n1");
    }
}
