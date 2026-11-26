use super::DbKind;
use super::engine::{Source, Table};

pub(super) fn json(table: &Table) -> String {
    let rows: Vec<serde_json::Value> = table
        .rows
        .iter()
        .map(|row| {
            serde_json::Value::Object(
                table
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(name, cell)| {
                        let value = match cell {
                            Some(text) => serde_json::Value::String(text.clone()),
                            None => serde_json::Value::Null,
                        };
                        (name.clone(), value)
                    })
                    .collect(),
            )
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

pub(super) fn inserts(table: &Table, kind: DbKind, name: &str) -> String {
    let columns: Vec<String> = table.columns.iter().map(|c| kind.quote(c)).collect();
    let mut out = String::new();
    for row in &table.rows {
        let values: Vec<String> = row.iter().map(|c| kind.literal(c.as_deref())).collect();
        out.push_str(&format!(
            "INSERT INTO {} ({}) VALUES ({});\n",
            kind.quote(name),
            columns.join(", "),
            values.join(", ")
        ));
    }
    out
}

pub fn split_sql(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut delimiter = ";".to_owned();
    let mut chars = text.chars().peekable();
    let mut quote: Option<char> = None;
    let mut line_start = true;
    while let Some(c) = chars.next() {
        if line_start && quote.is_none() && current.trim().is_empty() {
            let rest: String = std::iter::once(c).chain(chars.clone().take_while(|ch| *ch != '\n')).collect();
            if rest.trim_start().to_ascii_uppercase().starts_with("DELIMITER ") {
                delimiter = rest.trim_start()[10..].trim().to_owned();
                for _ in 0..rest.chars().count() - 1 {
                    chars.next();
                }
                current.clear();
                continue;
            }
        }
        line_start = c == '\n';
        if let Some(q) = quote {
            current.push(c);
            if c == '\\' && q != '`' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                quote = Some(c);
                current.push(c);
            }
            '-' if chars.peek() == Some(&'-') => {
                for ch in chars.by_ref() {
                    if ch == '\n' {
                        line_start = true;
                        current.push('\n');
                        break;
                    }
                }
            }
            '#' => {
                for ch in chars.by_ref() {
                    if ch == '\n' {
                        line_start = true;
                        current.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                let mut body = String::new();
                for ch in chars.by_ref() {
                    if prev == '*' && ch == '/' {
                        body.pop();
                        break;
                    }
                    body.push(ch);
                    prev = ch;
                }
                if body.starts_with('!') {
                    current.push_str("/*");
                    current.push_str(&body);
                    current.push_str("*/");
                }
            }
            _ => {
                current.push(c);
                if current.ends_with(delimiter.as_str()) {
                    current.truncate(current.len() - delimiter.len());
                    let statement = current.trim();
                    if !statement.is_empty() {
                        out.push(statement.to_owned());
                    }
                    current.clear();
                }
            }
        }
    }
    let statement = current.trim();
    if !statement.is_empty() {
        out.push(statement.to_owned());
    }
    out
}

pub fn parse_csv(text: &str) -> Vec<Vec<Option<String>>> {
    let first = text.lines().next().unwrap_or_default();
    let separator = [';', ',', '\t']
        .into_iter()
        .max_by_key(|s| first.matches(*s).count())
        .unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let push = |field: &mut String, was_quoted: &mut bool, row: &mut Vec<Option<String>>| {
        let value = std::mem::take(field);
        row.push(if value.is_empty() && !*was_quoted { None } else { Some(value) });
        *was_quoted = false;
    };
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                was_quoted = true;
            }
            '\r' => {}
            '\n' => {
                push(&mut field, &mut was_quoted, &mut row);
                rows.push(std::mem::take(&mut row));
            }
            c if c == separator => push(&mut field, &mut was_quoted, &mut row),
            c => field.push(c),
        }
    }
    if !field.is_empty() || was_quoted || !row.is_empty() {
        push(&mut field, &mut was_quoted, &mut row);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].is_none()));
    rows
}

pub(super) fn csv_inserts(rows: &[Vec<Option<String>>], kind: DbKind, table: &str) -> Result<Vec<String>, String> {
    let Some((header, body)) = rows.split_first() else {
        return Err(tr!("файл пустой").into());
    };
    let columns: Vec<String> = header
        .iter()
        .map(|c| kind.quote(c.as_deref().unwrap_or_default().trim()))
        .collect();
    let columns = columns.join(", ");
    body.iter()
        .enumerate()
        .map(|(index, row)| {
            if row.len() != header.len() {
                return Err(tf!(
                    "строка {}: полей {}, а в заголовке {}",
                    "line {}: {} fields, the header has {}",
                    index + 2,
                    row.len(),
                    header.len()
                ));
            }
            let values: Vec<String> = row.iter().map(|v| kind.literal(v.as_deref())).collect();
            Ok(format!("INSERT INTO {table} ({columns}) VALUES ({})", values.join(", ")))
        })
        .collect()
}

pub(super) fn csv(table: &Table, separator: char) -> String {
    let field = |text: &str| {
        if text.contains([separator, '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    };
    let mut out = String::new();
    let mut line = |cells: Vec<String>| {
        out.push_str(&cells.join(&separator.to_string()));
        out.push('\n');
    };
    line(table.columns.iter().map(|c| field(c)).collect());
    for row in &table.rows {
        line(
            row.iter()
                .map(|c| field(c.as_deref().unwrap_or("")))
                .collect(),
        );
    }
    out
}

pub fn source_of(sql: &str, current: &str) -> Option<Source> {
    let text = sql.trim().trim_end_matches(';').trim();
    let lower = text.to_lowercase();
    let rest = lower.strip_prefix("select * from ")?;
    let offset = text.len() - rest.len();
    let original = &text[offset..];
    let name: String = original
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    let tail = original[name.len()..].trim().to_lowercase();
    if !(tail.is_empty()
        || tail.starts_with("where ")
        || tail.starts_with("order by ")
        || tail.starts_with("limit "))
    {
        return None;
    }
    let clean = |part: &str| part.trim_matches(['`', '"', '[', ']']).to_owned();
    let mut parts = split_name(&name);
    let table = clean(&parts.pop()?);
    let schema = parts.pop().map(|p| clean(&p)).unwrap_or_else(|| current.to_owned());
    (!table.is_empty() && !schema.is_empty()).then_some(Source { schema, table })
}

pub(super) fn split_name(name: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for c in name.chars() {
        match (quote, c) {
            (None, '`' | '"') => {
                quote = Some(c);
                current.push(c);
            }
            (Some(q), c) if c == q => {
                quote = None;
                current.push(c);
            }
            (None, '.') => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}
