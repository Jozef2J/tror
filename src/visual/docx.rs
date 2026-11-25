use std::collections::HashMap;
use std::path::Path;

use super::model::decode;

enum Tok<'a> {
    Open(&'a str, &'a str, bool),
    Close(&'a str),
    Text(&'a str),
}

fn tokens(xml: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < xml.len() {
        let rest = &xml[i..];
        if let Some(body) = rest.strip_prefix('<') {
            let end = if rest.starts_with("<!--") {
                rest.find("-->").map(|e| e + 3)
            } else {
                rest.find('>').map(|e| e + 1)
            };
            let Some(end) = end else {
                break;
            };
            i += end;
            if body.starts_with('?') || body.starts_with('!') {
                continue;
            }
            let inner = &rest[1..end - 1];
            if let Some(name) = inner.strip_prefix('/') {
                out.push(Tok::Close(name.trim()));
                continue;
            }
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let split = inner.find(char::is_whitespace).unwrap_or(inner.len());
            out.push(Tok::Open(&inner[..split], &inner[split..], empty));
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            out.push(Tok::Text(&rest[..end]));
            i += end;
        }
    }
    out
}

fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = attrs[from..].find(key) {
        let start = from + at;
        let before = attrs[..start].chars().last();
        let after = &attrs[start + key.len()..];
        if before.is_none_or(char::is_whitespace) && after.trim_start().starts_with('=') {
            let value = after.trim_start()[1..].trim_start();
            let quote = value.chars().next()?;
            let body = &value[1..];
            let end = body.find(quote)?;
            return Some(decode(&body[..end]));
        }
        from = start + key.len();
    }
    None
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\u{a0}', "&nbsp;")
}

fn escape_attr(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

fn on(attrs: &str) -> bool {
    !matches!(
        attr(attrs, "w:val").as_deref(),
        Some("0" | "false" | "none" | "off")
    )
}

#[derive(Clone, Default, PartialEq)]
struct Format {
    bold: bool,
    italic: bool,
    strike: bool,
    sup: bool,
    sub: bool,
    link: Option<String>,
}

impl Format {
    fn wrap(&self, text: &str) -> String {
        let mut open = String::new();
        let mut close = String::new();
        let mut tag = |o: &str, c: &str| {
            open.push_str(o);
            close.insert_str(0, c);
        };
        if let Some(link) = &self.link {
            tag(&format!("<a href=\"{}\">", escape_attr(link)), "</a>");
        }
        if self.bold {
            tag("<strong>", "</strong>");
        }
        if self.italic {
            tag("<em>", "</em>");
        }
        if self.strike {
            tag("<s>", "</s>");
        }
        if self.sup {
            tag("<sup>", "</sup>");
        }
        if self.sub {
            tag("<sub>", "</sub>");
        }
        format!("{open}{text}{close}")
    }
}

struct Ctx {
    rels: HashMap<String, String>,
    styles: HashMap<String, String>,
    based: HashMap<String, String>,
    outline: HashMap<String, usize>,
    style_lists: HashMap<String, (String, Option<usize>)>,
    ordered: HashMap<(String, usize), bool>,
    media: HashMap<String, Vec<u8>>,
}

enum Block {
    Para {
        tag: String,
        align: Option<String>,
        list: Option<(String, usize)>,
        html: String,
    },
    Table(String),
}

fn skip(t: &[Tok], i: &mut usize, name: &str) {
    let mut depth = 0;
    while *i < t.len() {
        match &t[*i] {
            Tok::Open(n, _, false) if *n == name => depth += 1,
            Tok::Close(n) if *n == name => {
                depth -= 1;
                if depth == 0 {
                    *i += 1;
                    return;
                }
            }
            _ => {}
        }
        *i += 1;
    }
}

fn image(ctx: &Ctx, id: &str) -> Option<String> {
    let target = ctx.rels.get(id)?;
    let name = target.trim_start_matches("../").trim_start_matches('/');
    let path = if name.starts_with("word/") {
        name.to_owned()
    } else {
        format!("word/{name}")
    };
    let data = ctx.media.get(&path)?;
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or("png")
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    };
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    let file = path.rsplit('/').next().unwrap_or("image");
    Some(format!(
        "<img src=\"data:{mime};base64,{encoded}\" alt=\"{}\">",
        escape_attr(file)
    ))
}

fn heading_of(ctx: &Ctx, style: &str) -> Option<usize> {
    let mut id = style.to_owned();
    for _ in 0..8 {
        let name = ctx
            .styles
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_ascii_lowercase())
            .replace(' ', "");
        if name == "title" {
            return Some(1);
        }
        if let Some(n) = name
            .strip_prefix("heading")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| (1..=6).contains(n))
        {
            return Some(n);
        }
        if let Some(level) = ctx.outline.get(&id) {
            return Some(*level);
        }
        id = ctx.based.get(&id)?.clone();
    }
    None
}

fn paragraph(t: &[Tok], i: &mut usize, ctx: &Ctx) -> Block {
    let mut style = String::new();
    let mut outline: Option<usize> = None;
    let mut align = None;
    let mut num_id = None;
    let mut level: Option<usize> = None;
    let mut pieces: Vec<(Format, String)> = Vec::new();
    let mut format = Format::default();
    let mut link: Option<String> = None;
    let mut field: Option<String> = None;
    let mut field_link: Option<String> = None;
    let mut in_text = false;
    let mut in_instr = false;
    let mut in_props = 0;
    let mut in_run_props = false;
    *i += 1;
    let push = |pieces: &mut Vec<(Format, String)>, format: &Format, text: String| {
        if let Some((last, body)) = pieces.last_mut()
            && last == format
        {
            body.push_str(&text);
        } else {
            pieces.push((format.clone(), text));
        }
    };
    while *i < t.len() {
        match &t[*i] {
            Tok::Close("w:p") => {
                *i += 1;
                break;
            }
            Tok::Open("w:pPr", _, false) => in_props += 1,
            Tok::Close("w:pPr") => in_props -= 1,
            Tok::Open("w:pStyle", a, _) if in_props > 0 => {
                style = attr(a, "w:val").unwrap_or_default()
            }
            Tok::Open("w:outlineLvl", a, _) if in_props > 0 && !in_run_props => {
                outline = attr(a, "w:val")
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|v| *v < 6)
                    .map(|v| v + 1)
            }
            Tok::Open("w:jc", a, _) if in_props > 0 => {
                align = match attr(a, "w:val").as_deref() {
                    Some("center") => Some("center".into()),
                    Some("right" | "end") => Some("right".into()),
                    Some("both" | "distribute") => Some("justify".into()),
                    _ => None,
                }
            }
            Tok::Open("w:ilvl", a, _) if in_props > 0 => {
                level = attr(a, "w:val").and_then(|v| v.parse().ok())
            }
            Tok::Open("w:numId", a, _) if in_props > 0 => num_id = attr(a, "w:val"),
            _ if in_props > 0 => {}
            Tok::Open(name @ ("w:del" | "mc:Fallback"), _, false) => {
                let name = name.to_string();
                skip(t, i, &name);
                continue;
            }
            Tok::Open("w:hyperlink", a, false) => {
                link = attr(a, "r:id")
                    .and_then(|id| ctx.rels.get(&id).cloned())
                    .or_else(|| attr(a, "w:anchor").map(|a| format!("#{a}")));
            }
            Tok::Close("w:hyperlink") => link = None,
            Tok::Open("w:r", _, false) => format = Format::default(),
            Tok::Open("w:rPr", _, false) => in_run_props = true,
            Tok::Close("w:rPr") => in_run_props = false,
            Tok::Open(name, a, _) if in_run_props => match *name {
                "w:b" => format.bold = on(a),
                "w:i" => format.italic = on(a),
                "w:strike" | "w:dstrike" => format.strike = on(a),
                "w:vertAlign" => {
                    let v = attr(a, "w:val").unwrap_or_default();
                    format.sup = v == "superscript";
                    format.sub = v == "subscript";
                }
                _ => {}
            },
            Tok::Open("w:t", _, false) => in_text = true,
            Tok::Close("w:t") => in_text = false,
            Tok::Open("w:instrText", _, false) => in_instr = true,
            Tok::Close("w:instrText") => in_instr = false,
            Tok::Open("w:fldChar", a, _) => match attr(a, "w:fldCharType").as_deref() {
                Some("begin") => field = Some(String::new()),
                Some("separate") => {
                    field_link = field.take().and_then(|code| {
                        let code = code.trim();
                        let rest = code.strip_prefix("HYPERLINK")?.trim();
                        let anchor = rest.starts_with("\\l");
                        let rest = rest.trim_start_matches("\\l").trim();
                        let url = rest.trim_start_matches('"');
                        let url = &url[..url.find('"').unwrap_or(url.len())];
                        Some(if anchor {
                            format!("#{url}")
                        } else {
                            url.to_owned()
                        })
                    })
                }
                Some("end") => {
                    field = None;
                    field_link = None;
                }
                _ => {}
            },
            Tok::Text(text) if in_instr => {
                if let Some(code) = &mut field {
                    code.push_str(&decode(text));
                }
            }
            Tok::Text(text) if in_text => {
                let mut current = format.clone();
                current.link = link.clone().or_else(|| field_link.clone());
                push(&mut pieces, &current, escape(&decode(text)));
            }
            Tok::Open("w:tab", _, _) => {
                let mut current = format.clone();
                current.link = link.clone();
                push(&mut pieces, &current, "&emsp;".into());
            }
            Tok::Open("w:br", a, _) if attr(a, "w:type").is_none_or(|v| v == "textWrapping") => {
                pieces.push((Format::default(), "<br>".into()));
            }
            Tok::Open("a:blip", a, _) => {
                if let Some(img) = attr(a, "r:embed").and_then(|id| image(ctx, &id)) {
                    pieces.push((Format::default(), img));
                }
            }
            Tok::Open("v:imagedata", a, _) => {
                if let Some(img) = attr(a, "r:id").and_then(|id| image(ctx, &id)) {
                    pieces.push((Format::default(), img));
                }
            }
            _ => {}
        }
        *i += 1;
    }
    let name = ctx
        .styles
        .get(&style)
        .cloned()
        .unwrap_or_else(|| style.to_ascii_lowercase());
    let compact = name.replace(' ', "");
    let tag = if let Some(n) = outline.or_else(|| heading_of(ctx, &style)) {
        format!("h{n}")
    } else if compact.contains("quote") {
        "blockquote".into()
    } else {
        "p".into()
    };
    let heading = tag.starts_with('h');
    let html: String = pieces
        .iter()
        .map(|(f, text)| {
            if heading && f.bold {
                Format {
                    bold: false,
                    ..f.clone()
                }
                .wrap(text)
            } else {
                f.wrap(text)
            }
        })
        .collect();
    let from_style = ctx.style_lists.get(&style);
    let num_id = num_id.or_else(|| from_style.map(|(n, _)| n.clone()));
    let level = level
        .or_else(|| from_style.and_then(|(_, l)| *l))
        .or_else(|| {
            let rest = compact
                .strip_prefix("listbullet")
                .or_else(|| compact.strip_prefix("listnumber"))?;
            Some(rest.parse::<usize>().map_or(0, |n| n.saturating_sub(1)))
        })
        .unwrap_or(0);
    let list = num_id.filter(|n| n != "0").map(|n| (n, level));
    Block::Para {
        tag,
        align,
        list,
        html,
    }
}

struct Cell {
    html: String,
    span: usize,
    merge: Option<bool>,
    header: bool,
}

fn table(t: &[Tok], i: &mut usize, ctx: &Ctx) -> String {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut header_rows = Vec::new();
    *i += 1;
    while *i < t.len() {
        match &t[*i] {
            Tok::Close("w:tbl") => {
                *i += 1;
                break;
            }
            Tok::Open("w:tr", _, false) => {
                rows.push(Vec::new());
                header_rows.push(false);
            }
            Tok::Open("w:tblHeader", a, _) if on(a) => {
                if let Some(last) = header_rows.last_mut() {
                    *last = true;
                }
            }
            Tok::Open("w:tc", _, false) => {
                let mut span = 1;
                let mut merge = None;
                let mut blocks = Vec::new();
                *i += 1;
                while *i < t.len() {
                    match &t[*i] {
                        Tok::Close("w:tc") => break,
                        Tok::Open("w:gridSpan", a, _) => {
                            span = attr(a, "w:val").and_then(|v| v.parse().ok()).unwrap_or(1)
                        }
                        Tok::Open("w:vMerge", a, _) => {
                            merge = Some(attr(a, "w:val").as_deref() == Some("restart"))
                        }
                        Tok::Open("w:p", _, false) => {
                            blocks.push(paragraph(t, i, ctx));
                            continue;
                        }
                        Tok::Open("w:p", _, true) => blocks.push(Block::Para {
                            tag: "p".into(),
                            align: None,
                            list: None,
                            html: String::new(),
                        }),
                        Tok::Open("w:tbl", _, false) => {
                            blocks.push(Block::Table(table(t, i, ctx)));
                            continue;
                        }
                        _ => {}
                    }
                    *i += 1;
                }
                let header = header_rows.last().copied().unwrap_or(false);
                if let Some(row) = rows.last_mut() {
                    row.push(Cell {
                        html: render(&blocks, ctx, true),
                        span,
                        merge,
                        header,
                    });
                }
            }
            _ => {}
        }
        *i += 1;
    }
    let grid: Vec<Vec<usize>> = rows
        .iter()
        .map(|row| {
            let mut col = 0;
            row.iter()
                .map(|cell| {
                    let at = col;
                    col += cell.span;
                    at
                })
                .collect()
        })
        .collect();
    let mut out = String::from("<table border=\"1\" cellpadding=\"4\" cellspacing=\"0\">\n");
    for (r, row) in rows.iter().enumerate() {
        out.push_str("\t<tr>\n");
        for (c, cell) in row.iter().enumerate() {
            if cell.merge == Some(false) {
                continue;
            }
            let mut rowspan = 1;
            if cell.merge == Some(true) {
                let col = grid[r][c];
                for below in r + 1..rows.len() {
                    let continues = grid[below]
                        .iter()
                        .position(|&x| x == col)
                        .is_some_and(|k| rows[below][k].merge == Some(false));
                    if !continues {
                        break;
                    }
                    rowspan += 1;
                }
            }
            let tag = if cell.header { "th" } else { "td" };
            out.push_str(&format!("\t\t<{tag}"));
            if cell.span > 1 {
                out.push_str(&format!(" colspan=\"{}\"", cell.span));
            }
            if rowspan > 1 {
                out.push_str(&format!(" rowspan=\"{rowspan}\""));
            }
            out.push('>');
            out.push_str(&cell.html);
            out.push_str(&format!("</{tag}>\n"));
        }
        out.push_str("\t</tr>\n");
    }
    out.push_str("</table>");
    out
}

fn render(blocks: &[Block], ctx: &Ctx, cell: bool) -> String {
    let mut out = String::new();
    let mut lists: Vec<&'static str> = Vec::new();
    let mut current = String::new();
    fn close(out: &mut String, lists: &mut Vec<&'static str>) {
        let tag = lists.pop().unwrap_or("ul");
        out.push_str(&format!("</li>\n{}</{tag}>", "\t".repeat(lists.len() * 2)));
        if lists.is_empty() {
            out.push('\n');
        }
    }
    let close_all = |out: &mut String, lists: &mut Vec<&'static str>| {
        while !lists.is_empty() {
            close(out, lists);
        }
    };
    let single = cell
        && blocks.len() == 1
        && matches!(&blocks[0], Block::Para { tag, align: None, list: None, .. } if tag == "p");
    for block in blocks {
        match block {
            Block::Table(html) => {
                close_all(&mut out, &mut lists);
                out.push_str(html);
                out.push('\n');
            }
            Block::Para {
                tag,
                align,
                list,
                html,
            } => {
                let style = align
                    .as_ref()
                    .map(|a| format!(" style=\"text-align: {a};\""))
                    .unwrap_or_default();
                let body = if html.is_empty() {
                    "&nbsp;"
                } else {
                    html.as_str()
                };
                if let Some((id, level)) = list {
                    let depth = level + 1;
                    let ordered = ctx
                        .ordered
                        .get(&(id.clone(), *level))
                        .copied()
                        .unwrap_or(false);
                    let tag_here = if ordered { "ol" } else { "ul" };
                    let keep = if lists.len() >= depth
                        && (lists[depth - 1] != tag_here || (depth == 1 && current != *id))
                    {
                        depth - 1
                    } else {
                        depth
                    };
                    while lists.len() > keep {
                        close(&mut out, &mut lists);
                    }
                    if depth == 1 {
                        current = id.clone();
                    }
                    if lists.len() == depth {
                        out.push_str("</li>");
                    }
                    while lists.len() < depth {
                        let tag = if ordered { "ol" } else { "ul" };
                        if !lists.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&"\t".repeat(lists.len() * 2));
                        out.push_str(&format!("<{tag}>"));
                        lists.push(tag);
                    }
                    out.push('\n');
                    out.push_str(&"\t".repeat(lists.len() * 2 - 1));
                    out.push_str(&format!("<li{style}>{html}"));
                    continue;
                }
                close_all(&mut out, &mut lists);
                if single {
                    out.push_str(body);
                } else if tag == "blockquote" {
                    out.push_str(&format!("<blockquote><p{style}>{body}</p></blockquote>\n"));
                } else {
                    out.push_str(&format!("<{tag}{style}>{body}</{tag}>\n"));
                }
            }
        }
    }
    close_all(&mut out, &mut lists);
    out
}

pub fn to_html(path: &Path) -> Result<String, String> {
    let files = crate::plugins::unzip(path)?;
    let text = |name: &str| {
        files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, data)| String::from_utf8_lossy(data).into_owned())
    };
    let document = text("word/document.xml").ok_or(tr!("это не документ Word (.docx)"))?;
    let mut rels = HashMap::new();
    if let Some(xml) = text("word/_rels/document.xml.rels") {
        for tok in tokens(&xml) {
            if let Tok::Open("Relationship", a, _) = tok
                && let (Some(id), Some(target)) = (attr(a, "Id"), attr(a, "Target"))
            {
                rels.insert(id, target);
            }
        }
    }
    let mut styles = HashMap::new();
    let mut based = HashMap::new();
    let mut outline = HashMap::new();
    let mut style_lists: HashMap<String, (String, Option<usize>)> = HashMap::new();
    if let Some(xml) = text("word/styles.xml") {
        let mut current: Option<String> = None;
        for tok in tokens(&xml) {
            match tok {
                Tok::Open("w:style", a, _) => current = attr(a, "w:styleId"),
                Tok::Close("w:style") => current = None,
                Tok::Open("w:name", a, _) => {
                    if let (Some(id), Some(name)) = (current.clone(), attr(a, "w:val")) {
                        styles.insert(id, name.to_ascii_lowercase());
                    }
                }
                Tok::Open("w:basedOn", a, _) => {
                    if let (Some(id), Some(base)) = (current.clone(), attr(a, "w:val")) {
                        based.insert(id, base);
                    }
                }
                Tok::Open("w:outlineLvl", a, _) => {
                    if let (Some(id), Some(level)) = (
                        current.clone(),
                        attr(a, "w:val").and_then(|v| v.parse::<usize>().ok()),
                    ) && level < 6
                    {
                        outline.insert(id, level + 1);
                    }
                }
                Tok::Open("w:numId", a, _) => {
                    if let (Some(id), Some(num)) = (current.clone(), attr(a, "w:val")) {
                        style_lists.entry(id).or_insert((String::new(), None)).0 = num;
                    }
                }
                Tok::Open("w:ilvl", a, _) => {
                    if let Some(id) = current.clone() {
                        style_lists.entry(id).or_insert((String::new(), None)).1 =
                            attr(a, "w:val").and_then(|v| v.parse().ok());
                    }
                }
                _ => {}
            }
        }
        style_lists.retain(|_, (num, _)| !num.is_empty());
    }
    let mut ordered = HashMap::new();
    if let Some(xml) = text("word/numbering.xml") {
        let mut abstracts: HashMap<String, HashMap<usize, bool>> = HashMap::new();
        let mut links: Vec<(String, String)> = Vec::new();
        let mut current_abstract = None;
        let mut current_level = 0;
        let mut current_num = None;
        for tok in tokens(&xml) {
            match tok {
                Tok::Open("w:abstractNum", a, _) => current_abstract = attr(a, "w:abstractNumId"),
                Tok::Open("w:lvl", a, _) => {
                    current_level = attr(a, "w:ilvl").and_then(|v| v.parse().ok()).unwrap_or(0)
                }
                Tok::Open("w:numFmt", a, _) => {
                    if let Some(id) = &current_abstract {
                        let fmt = attr(a, "w:val").unwrap_or_default();
                        abstracts
                            .entry(id.clone())
                            .or_default()
                            .insert(current_level, fmt != "bullet" && fmt != "none");
                    }
                }
                Tok::Close("w:abstractNum") => current_abstract = None,
                Tok::Open("w:num", a, _) => current_num = attr(a, "w:numId"),
                Tok::Open("w:abstractNumId", a, _) => {
                    if let (Some(num), Some(abs)) = (current_num.clone(), attr(a, "w:val")) {
                        links.push((num, abs));
                    }
                }
                _ => {}
            }
        }
        for (num, abs) in links {
            if let Some(levels) = abstracts.get(&abs) {
                for (level, is_ordered) in levels {
                    ordered.insert((num.clone(), *level), *is_ordered);
                }
            }
        }
    }
    let media = files
        .iter()
        .filter(|(n, _)| n.starts_with("word/media/"))
        .map(|(n, d)| (n.clone(), d.clone()))
        .collect();
    let ctx = Ctx {
        rels,
        styles,
        based,
        outline,
        style_lists,
        ordered,
        media,
    };
    let t = tokens(&document);
    let mut i = t
        .iter()
        .position(|tok| matches!(tok, Tok::Open("w:body", _, _)))
        .map_or(0, |p| p + 1);
    let mut blocks = Vec::new();
    while i < t.len() {
        match &t[i] {
            Tok::Close("w:body") => break,
            Tok::Open("w:p", _, false) => {
                blocks.push(paragraph(&t, &mut i, &ctx));
                continue;
            }
            Tok::Open("w:tbl", _, false) => {
                blocks.push(Block::Table(table(&t, &mut i, &ctx)));
                continue;
            }
            Tok::Open("w:sectPr", _, false) => {
                skip(&t, &mut i, "w:sectPr");
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    Ok(render(&blocks, &ctx, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docx(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::io::Write;
        let mut zip = Vec::new();
        let mut central = Vec::new();
        for (name, body) in files {
            let offset = zip.len() as u32;
            let crc = crc32(body.as_bytes());
            let head = |sig: u32, out: &mut Vec<u8>, central: bool| {
                out.extend_from_slice(&sig.to_le_bytes());
                if central {
                    out.extend_from_slice(&20u16.to_le_bytes());
                }
                out.extend_from_slice(&20u16.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&crc.to_le_bytes());
                out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
            };
            head(0x0403_4b50, &mut zip, false);
            zip.extend_from_slice(name.as_bytes());
            zip.extend_from_slice(body.as_bytes());
            head(0x0201_4b50, &mut central, true);
            central.extend_from_slice(&[0; 6]);
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let start = zip.len() as u32;
        let size = central.len() as u32;
        zip.extend_from_slice(&central);
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&start.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("tror-docx-{}-{n}.docx", std::process::id()));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&zip)
            .unwrap();
        path
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn custom_heading_styles_become_headings() {
        let document = r#"<w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:pStyle w:val="a5"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>Свой стиль</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="a7"/></w:pPr><w:r><w:t>Уровень</w:t></w:r></w:p>
<w:p><w:pPr><w:outlineLvl w:val="2"/></w:pPr><w:r><w:t>Прямо</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Жирный абзац</w:t></w:r></w:p>
</w:body></w:document>"#;
        let styles = r#"<w:styles><w:style w:type="paragraph" w:styleId="2"><w:name w:val="heading 2"/></w:style><w:style w:type="paragraph" w:styleId="a5"><w:name w:val="Мой заголовок"/><w:basedOn w:val="2"/></w:style><w:style w:type="paragraph" w:styleId="a7"><w:name w:val="Раздел"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style></w:styles>"#;
        let path = docx(&[("word/document.xml", document), ("word/styles.xml", styles)]);
        let html = to_html(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let html = html.replace('\n', "");
        assert!(html.contains("<h2>Свой стиль</h2>"), "{html}");
        assert!(html.contains("<h1>Уровень</h1>"), "{html}");
        assert!(html.contains("<h3>Прямо</h3>"), "{html}");
        assert!(
            html.contains("<p><b>Жирный абзац</b></p>")
                || html.contains("<p><strong>Жирный абзац</strong></p>"),
            "{html}"
        );
    }

    #[test]
    fn word_document_becomes_html() {
        let document = r#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:r="r"><w:body>
<w:p><w:pPr><w:pStyle w:val="1"/></w:pPr><w:r><w:t>Заголовок</w:t></w:r></w:p>
<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Жирный </w:t></w:r><w:r><w:t>и </w:t></w:r><w:hyperlink r:id="rId5"><w:r><w:t>ссылка</w:t></w:r></w:hyperlink><w:r><w:rPr><w:i/><w:b w:val="0"/></w:rPr><w:t>&amp; курсив</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Один</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Вложенный</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>Номер</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>AB</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>C</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>D</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p><w:r><w:t>E</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> HYPERLINK "https://x.ru/" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>поле</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:sectPr><w:pgSz/></w:sectPr></w:body></w:document>"#;
        let rels = r#"<Relationships><Relationship Id="rId5" Type="hyperlink" Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/></Relationships>"#;
        let styles = r#"<w:styles><w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/></w:style></w:styles>"#;
        let numbering = r#"<w:numbering><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        let path = docx(&[
            ("word/document.xml", document),
            ("word/_rels/document.xml.rels", rels),
            ("word/styles.xml", styles),
            ("word/numbering.xml", numbering),
        ]);
        let html = to_html(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let flat = html.replace(['\n', '\t'], "");
        assert!(html.contains("<h1>Заголовок</h1>"), "{html}");
        assert!(html.contains(r#"<p style="text-align: center;"><strong>Жирный </strong>и <a href="https://example.com/?a=1&amp;b=2">ссылка</a><em>&amp; курсив</em></p>"#), "{html}");
        assert!(
            flat.contains(
                "<ul><li>Один<ul><li>Вложенный</li></ul></li></ul><ol><li>Номер</li></ol>"
            ),
            "{html}"
        );
        assert!(html.contains(r#"<td colspan="2">AB</td>"#), "{html}");
        assert!(
            html.contains(r#"<td rowspan="2">C</td>"#) && !html.contains("<td>&nbsp;</td>"),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="https://x.ru/">поле</a>"#),
            "{html}"
        );
    }
}
