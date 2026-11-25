use super::model::{Attrs, decode};

const SKIP: &[&str] = &[
    "style", "script", "head", "title", "xml", "noscript", "template", "object", "svg",
];
const VOID: &[&str] = &[
    "br", "img", "hr", "meta", "link", "input", "col", "wbr", "area", "base", "source",
];
const KEEP: &[&str] = &[
    "p",
    "br",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "b",
    "strong",
    "i",
    "em",
    "s",
    "strike",
    "del",
    "sub",
    "sup",
    "a",
    "img",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "blockquote",
    "pre",
    "code",
    "hr",
    "span",
    "dl",
    "dt",
    "dd",
];
const BLOCK: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "blockquote",
    "pre",
    "hr",
    "dl",
    "dt",
    "dd",
    "div",
];

pub fn read_html() -> Option<String> {
    let html = arboard::Clipboard::new().ok()?.get().html().ok()?;
    let html = html.trim_matches('\0');
    (!html.trim().is_empty()).then(|| html.to_owned())
}

pub fn write_html(html: &str, text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set().html(html, Some(text)))
        .is_ok()
}

fn fragment(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    if let (Some(a), Some(b)) = (
        lower.find("<!--startfragment-->"),
        lower.rfind("<!--endfragment-->"),
    ) && a < b
    {
        return &html[a + 20..b];
    }
    if let Some(b) = lower.find("<body") {
        let start = html[b..].find('>').map_or(html.len(), |e| b + e + 1);
        let end = lower.rfind("</body").unwrap_or(html.len()).max(start);
        return &html[start..end];
    }
    html
}

fn clean_style(raw: Option<&str>) -> Vec<&'static str> {
    let mut wrap = Vec::new();
    for part in raw.unwrap_or_default().split(';') {
        let Some((k, v)) = part.split_once(':') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().trim_end_matches("!important").trim().to_owned();
        let lv = v.to_ascii_lowercase();
        match k.as_str() {
            "font-weight"
                if lv == "bold" || lv == "bolder" || lv.parse::<u32>().is_ok_and(|w| w >= 600) =>
            {
                wrap.push("strong")
            }
            "font-style" if lv == "italic" || lv == "oblique" => wrap.push("em"),
            "text-decoration" | "text-decoration-line" if lv.contains("line-through") => {
                wrap.push("s")
            }
            "vertical-align" if lv == "super" => wrap.push("sup"),
            "vertical-align" if lv == "sub" => wrap.push("sub"),
            _ => {}
        }
    }
    wrap
}

fn heading_level(attrs: &Attrs) -> Option<usize> {
    if attrs
        .get("role")
        .is_some_and(|r| r.eq_ignore_ascii_case("heading"))
    {
        return Some(
            attrs
                .get("aria-level")
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(2)
                .clamp(1, 6),
        );
    }
    let class = attrs.get("class")?.to_ascii_lowercase();
    class.split_whitespace().find_map(|c| {
        if c == "msotitle" {
            return Some(1);
        }
        let n = c
            .strip_prefix("msoheading")
            .or_else(|| c.strip_prefix("heading"))?;
        n.parse::<usize>().ok().filter(|n| (1..=6).contains(n))
    })
}

fn open_tag(tag: &str, attrs: &Attrs) -> String {
    let mut out = format!("<{tag}");
    for (name, value) in &attrs.0 {
        out.push(' ');
        out.push_str(name);
        if let Some(v) = value {
            out.push_str("=\"");
            out.push_str(
                &v.replace('&', "&amp;")
                    .replace('"', "&quot;")
                    .replace('<', "&lt;"),
            );
            out.push('"');
        }
    }
    out.push('>');
    out
}

fn list_marker(text: &str) -> bool {
    let t: String = decode(text)
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
        .collect();
    let Some(body) = t.strip_suffix(['.', ')']) else {
        return false;
    };
    !body.is_empty()
        && body.len() <= 6
        && (body.chars().all(|c| c.is_ascii_digit() || c == '.')
            || body.chars().all(|c| c.is_ascii_alphabetic()))
}

fn mso_list(attrs: &Attrs) -> Option<(String, usize)> {
    let style = attrs.get("style")?.to_ascii_lowercase();
    let at = style.find("mso-list:")?;
    let value = style[at + 9..].split(';').next()?.trim().to_owned();
    if value.starts_with("ignore") || value == "none" {
        return None;
    }
    let mut id = String::new();
    let mut level = 1;
    for word in value.split_whitespace() {
        if let Some(n) = word.strip_prefix("level") {
            level = n.parse().unwrap_or(1);
        } else if word.starts_with('l') && id.is_empty() {
            id = word.to_owned();
        }
    }
    Some((id, level.clamp(1, 9)))
}

struct Out {
    html: String,
    stack: Vec<(String, String)>,
    lists: Vec<&'static str>,
    list_id: String,
    pending: Option<(String, usize)>,
    marker: Option<String>,
    skip: Option<String>,
    loose: bool,
}

impl Out {
    fn in_block(&self) -> bool {
        self.pending.is_some()
            || !self.lists.is_empty()
            || self
                .stack
                .iter()
                .any(|(n, c)| BLOCK.contains(&n.as_str()) && !c.is_empty())
    }

    fn in_heading(&self) -> bool {
        self.stack
            .iter()
            .any(|(_, c)| c.len() == 5 && c.starts_with("</h") && c.as_bytes()[3].is_ascii_digit())
    }

    fn open_loose(&mut self) {
        if !self.loose && !self.in_block() {
            self.html.push_str("<p>");
            self.loose = true;
        }
    }

    fn close_loose(&mut self) {
        if self.loose {
            self.html.push_str("</p>");
            self.loose = false;
        }
    }

    fn close_lists(&mut self) {
        while let Some(tag) = self.lists.pop() {
            self.html.push_str("</li></");
            self.html.push_str(tag);
            self.html.push('>');
        }
    }

    fn flush_li(&mut self, ordered: bool) {
        let Some((id, level)) = self.pending.take() else {
            return;
        };
        if level == 1 && !self.lists.is_empty() && id != self.list_id {
            self.close_lists();
        }
        self.list_id = id;
        while self.lists.len() > level {
            let tag = self.lists.pop().unwrap_or("ul");
            self.html.push_str(&format!("</li></{tag}>"));
        }
        if self.lists.len() == level {
            self.html.push_str("</li>");
        }
        while self.lists.len() < level {
            let tag = if ordered { "ol" } else { "ul" };
            self.html.push_str(&format!("<{tag}>"));
            self.lists.push(tag);
        }
        self.html.push_str("<li>");
    }

    fn before_content(&mut self) {
        if self.pending.is_some() {
            self.flush_li(false);
        }
    }

    fn block_boundary(&mut self) {
        if self.pending.is_none() && !self.lists.is_empty() {
            self.close_lists();
        }
    }
}

pub fn clean(source: &str) -> String {
    let src = fragment(source);
    let mut out = Out {
        html: String::with_capacity(src.len() / 2),
        stack: Vec::new(),
        lists: Vec::new(),
        list_id: String::new(),
        pending: None,
        marker: None,
        skip: None,
        loose: false,
    };
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(src.len(), |e| i + e + 3);
            let body = src[i..end].to_ascii_lowercase();
            if body.contains("[if !supportlists]") {
                out.marker = Some(String::new());
            } else if body.contains("[endif]") && out.marker.is_some() {
                let marker = out.marker.take().unwrap_or_default();
                out.flush_li(list_marker(&marker));
            }
            i = end;
            continue;
        }
        if rest.starts_with("<![") {
            let end = rest.find('>').map_or(src.len(), |e| i + e + 1);
            let body = src[i..end].to_ascii_lowercase();
            if body.contains("supportlists") && !body.contains("endif") {
                out.marker = Some(String::new());
            } else if body.contains("endif") && out.marker.is_some() {
                let marker = out.marker.take().unwrap_or_default();
                out.flush_li(list_marker(&marker));
            }
            i = end;
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i = rest.find('>').map_or(src.len(), |e| i + e + 1);
            continue;
        }
        if rest.starts_with('<')
            && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/')
        {
            let Some(close) = rest.find('>') else {
                break;
            };
            let end = i + close + 1;
            let inner = src[i + 1..i + close].trim();
            i = end;
            let closing = inner.starts_with('/');
            let inner = inner.trim_start_matches('/');
            let name_end = inner
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(inner.len());
            let name = inner[..name_end].to_ascii_lowercase();
            let raw_attrs = &inner[name_end..];
            if let Some(skip) = &out.skip {
                if closing && name == *skip {
                    out.skip = None;
                }
                continue;
            }
            if out.marker.is_some() {
                continue;
            }
            if closing {
                if BLOCK.contains(&name.as_str()) {
                    out.close_loose();
                }
                if let Some(pos) = out.stack.iter().rposition(|(n, _)| *n == name) {
                    if BLOCK.contains(&name.as_str()) {
                        out.before_content();
                    }
                    while out.stack.len() > pos {
                        let (n, close) = out.stack.pop().unwrap_or_default();
                        if n != "#li" && BLOCK.contains(&n.as_str()) && !close.is_empty() {
                            out.block_boundary();
                        }
                        out.html.push_str(&close);
                    }
                }
                continue;
            }
            if SKIP.contains(&name.as_str()) {
                if !raw_attrs.trim_end().ends_with('/') {
                    out.skip = Some(name);
                }
                continue;
            }
            let attrs = Attrs::parse(raw_attrs);
            let void = VOID.contains(&name.as_str()) || raw_attrs.trim_end().ends_with('/');
            if BLOCK.contains(&name.as_str()) {
                out.close_loose();
            }
            if name == "p"
                && let Some(list) = mso_list(&attrs)
            {
                out.before_content();
                out.pending = Some(list);
                out.stack.push(("p".into(), String::new()));
                continue;
            }
            if name.contains(':') || !KEEP.contains(&name.as_str()) {
                if BLOCK.contains(&name.as_str()) {
                    out.before_content();
                    out.block_boundary();
                }
                if !void {
                    out.stack.push((name, String::new()));
                }
                continue;
            }
            let clean = clean_style(attrs.get("style"));
            let mut kept = Attrs::default();
            let allowed: &[&str] = match name.as_str() {
                "a" => &["href", "target", "title", "name"],
                "img" => &["src", "alt", "width", "height", "title"],
                "td" | "th" => &["colspan", "rowspan"],
                "table" => &["border", "cellpadding", "cellspacing"],
                "ol" => &["start", "type"],
                "li" => &["value"],
                _ => &[],
            };
            for key in allowed {
                if let Some(v) = attrs.get(key) {
                    let v = v.trim();
                    if v.is_empty() || (*key == "name" && v.starts_with('_')) {
                        continue;
                    }
                    if (*key == "colspan" || *key == "rowspan") && v == "1" {
                        continue;
                    }
                    kept.set(key, Some(v.to_owned()));
                }
            }
            let mut wrap = clean;
            let mut tag = name.clone();
            let heading = out.in_heading();
            if heading {
                wrap.retain(|w| *w != "strong");
            }
            match name.as_str() {
                "p" if let Some(level) = heading_level(&attrs) => {
                    tag = format!("h{level}");
                    wrap.clear();
                }
                "b" | "strong" if heading => tag.clear(),
                "span" => tag.clear(),
                "b" | "strong"
                    if attrs
                        .style("font-weight")
                        .is_some_and(|w| w == "normal" || w == "400") =>
                {
                    tag.clear();
                }
                "a" if kept.get("href").is_none() && kept.get("name").is_none() => tag.clear(),
                "b" | "strong" | "i" | "em" | "s" | "strike" | "del" | "sub" | "sup" => {}
                _ => wrap.clear(),
            }
            if BLOCK.contains(&name.as_str()) {
                out.before_content();
                if name != "li" || out.lists.is_empty() {
                    out.block_boundary();
                }
            } else {
                out.before_content();
            }
            let mut open = String::new();
            let mut close = String::new();
            if !tag.is_empty() {
                open.push_str(&open_tag(&tag, &kept));
            }
            for w in &wrap {
                open.push_str(&format!("<{w}>"));
            }
            for w in wrap.iter().rev() {
                close.push_str(&format!("</{w}>"));
            }
            if !tag.is_empty() && !void {
                close.push_str(&format!("</{tag}>"));
            }
            if !open.is_empty() && name != "br" && !BLOCK.contains(&name.as_str()) {
                out.open_loose();
            }
            out.html.push_str(&open);
            if !void {
                out.stack.push((name, close));
            }
            continue;
        }
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let next = rest[first..].find('<').map_or(src.len(), |e| i + first + e);
        let text = &src[i..next];
        i = next;
        if out.skip.is_some() {
            continue;
        }
        if let Some(marker) = &mut out.marker {
            marker.push_str(text);
            continue;
        }
        if !text.trim().is_empty() {
            out.before_content();
            out.open_loose();
        } else if !out.loose && !out.in_block() {
            continue;
        }
        out.html.push_str(text);
    }
    out.before_content();
    out.close_loose();
    while let Some((_, close)) = out.stack.pop() {
        out.html.push_str(&close);
    }
    out.close_lists();
    out.html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_lists_links_and_junk() {
        let word = r#"<html xmlns:o="urn:schemas-microsoft-com:office:office"><head><style>p.MsoNormal{margin:0}</style></head><body lang=RU>
<!--StartFragment--><p class=MsoNormal style='margin-bottom:0cm;mso-line-height-alt:12pt'><b><span style='font-size:14.0pt;font-family:"Arial",sans-serif;mso-bidi-font-family:Arial'>Заголовок<o:p></o:p></span></b></p>
<p class=MsoNormal>Текст со <a href="https://example.com/x?a=1&amp;b=2">ссылкой</a> и <i>курсивом</i>.<o:p></o:p></p>
<p class=MsoListParagraphCxSpFirst style='text-indent:-18.0pt;mso-list:l0 level1 lfo1'><![if !supportLists]><span style='font-family:Symbol'>·<span style='font:7.0pt "Times New Roman"'>&nbsp;&nbsp; </span></span><![endif]>Первый<o:p></o:p></p>
<p class=MsoListParagraphCxSpMiddle style='mso-list:l0 level2 lfo1'><![if !supportLists]><span>o<span>&nbsp; </span></span><![endif]>Вложенный<o:p></o:p></p>
<p class=MsoListParagraphCxSpLast style='mso-list:l0 level1 lfo1'><![if !supportLists]><span>·<span>&nbsp; </span></span><![endif]>Второй<o:p></o:p></p>
<p class=MsoListParagraph style='mso-list:l1 level1 lfo2'><![if !supportLists]><span>1.<span>&nbsp; </span></span><![endif]>Нумерованный<o:p></o:p></p>
<table class=MsoTableGrid border=1 cellspacing=0 cellpadding=0 style='border-collapse:collapse;mso-yfti-tbllook:1184'><tr><td width=300 valign=top style='width:225pt'><p class=MsoNormal>A<o:p></o:p></p></td></tr></table>
<!--EndFragment--></body></html>"#;
        let html = clean(word).replace('\n', "");
        assert!(
            !html.contains("Mso") && !html.contains("o:p") && !html.contains("mso-"),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="https://example.com/x?a=1&amp;b=2">ссылкой</a>"#),
            "{html}"
        );
        assert!(
            html.contains("<ul><li>Первый<ul><li>Вложенный</li></ul></li><li>Второй</li></ul>"),
            "{html}"
        );
        assert!(html.contains("<ol><li>Нумерованный</li></ol>"), "{html}");
        assert!(html.contains(r#"<table border="1" cellpadding="0" cellspacing="0"><tr><td><p>A</p></td></tr></table>"#), "{html}");
        assert!(html.contains("<b>Заголовок</b>"), "{html}");
    }

    #[test]
    fn word_online_headings_and_loose_text() {
        let w = r#"<div class="OutlineElement Ltr SCXW1"><p class="Paragraph SCXW1 BCX0" role="heading" aria-level="2" paraid="1"><span lang="RU-RU" class="TextRun" style="font-size:16pt;font-weight:bold"><span class="NormalTextRun">Заголовок</span></span><span class="EOP">&nbsp;</span></p></div><div class="OutlineElement Ltr"><p class="Paragraph" paraid="2"><span class="TextRun"><span>Обычный текст</span></span></p></div>"#;
        assert_eq!(clean(w), "<h2>Заголовок&nbsp;</h2><p>Обычный текст</p>");
        let desktop = r#"<p class=MsoTitle>Титул</p><h3><b><span lang=RU>Третий</span></b></h3><p class="Heading4">Четвёртый</p>"#;
        assert_eq!(
            clean(desktop),
            "<h1>Титул</h1><h3>Третий</h3><h4>Четвёртый</h4>"
        );
        let loose =
            "<div>Первая строка<br>вторая</div>\n<div>Ещё <b>текст</b></div><h2>Заг</h2>хвост";
        assert_eq!(
            clean(loose),
            "<p>Первая строка<br>вторая</p><p>Ещё <b>текст</b></p><h2>Заг</h2><p>хвост</p>"
        );
        let cut = "<html><body><!--StartFragment-->конец абзаца</p><p>Целый</p><p>начало<!--EndFragment--></body></html>";
        assert_eq!(clean(cut), "<p>конец абзаца</p><p>Целый</p><p>начало</p>");
    }

    #[test]
    fn google_docs_spans_become_tags() {
        let g = r#"<meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-1"><p dir="ltr" style="line-height:1.38;margin-top:0pt"><span style="font-size:11pt;font-family:Arial;color:#000000;font-weight:700;font-style:normal">Жирный</span><span style="font-weight:400;font-style:italic"> курсив</span></p></b>"#;
        let html = clean(g);
        assert_eq!(html, "<p><strong>Жирный</strong><em> курсив</em></p>");
    }

    #[test]
    fn text_formatting_is_dropped() {
        let w = r#"<h2 style="color:red;text-align:center">Заголовок</h2><p style="text-align:center"><span style="color:#ff0000;background:yellow;font-family:Arial;font-size:20pt;text-decoration:underline">Красный</span> <u>подчёркнутый</u> <font color="blue">синий</font> <a href="https://x.y" style="color:green">ссылка</a></p><ul><li style="color:red">пункт</li></ul>"#;
        assert_eq!(
            clean(w),
            r#"<h2>Заголовок</h2><p>Красный подчёркнутый синий <a href="https://x.y">ссылка</a></p><ul><li>пункт</li></ul>"#
        );
    }
}
