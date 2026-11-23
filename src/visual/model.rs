use std::ops::Range;

pub const OBJ: char = '\u{FFFC}';
const ATOM_OPEN: char = '\u{E000}';
const ATOM_CLOSE: char = '\u{E001}';

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Attrs(pub Vec<(String, Option<String>)>);

impl Attrs {
    pub fn parse(raw: &str) -> Attrs {
        let mut out = Vec::new();
        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/'
            {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '=' {
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let value = if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                    let quote = chars[i];
                    i += 1;
                    let s = i;
                    while i < chars.len() && chars[i] != quote {
                        i += 1;
                    }
                    let v: String = chars[s..i].iter().collect();
                    i += 1;
                    v
                } else {
                    let s = i;
                    while i < chars.len() && !chars[i].is_whitespace() {
                        i += 1;
                    }
                    chars[s..i].iter().collect()
                };
                if !name.is_empty() {
                    out.push((name, Some(decode(&value))));
                }
            } else if !name.is_empty() {
                out.push((name, None));
            }
        }
        Attrs(out)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn set(&mut self, name: &str, value: Option<String>) {
        match value {
            Some(v) => match self
                .0
                .iter_mut()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
            {
                Some(entry) => entry.1 = Some(v),
                None => self.0.push((name.to_owned(), Some(v))),
            },
            None => self.0.retain(|(n, _)| !n.eq_ignore_ascii_case(name)),
        }
    }

    pub fn style(&self, prop: &str) -> Option<String> {
        let style = self.get("style")?;
        style.split(';').find_map(|part| {
            let (k, v) = part.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(prop)
                .then(|| v.trim().to_owned())
        })
    }

    pub fn set_style(&mut self, prop: &str, value: Option<&str>) {
        let style = self.get("style").unwrap_or_default().to_owned();
        let mut parts: Vec<(String, String)> = style
            .split(';')
            .filter_map(|p| {
                let (k, v) = p.split_once(':')?;
                Some((k.trim().to_owned(), v.trim().to_owned()))
            })
            .filter(|(k, _)| !k.eq_ignore_ascii_case(prop))
            .collect();
        if let Some(v) = value {
            parts.push((prop.to_owned(), v.to_owned()));
        }
        let joined: Vec<String> = parts.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        self.set(
            "style",
            (!joined.is_empty()).then(|| format!("{};", joined.join("; "))),
        );
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn write(&self, out: &mut String) {
        for (name, value) in &self.0 {
            out.push(' ');
            out.push_str(name);
            if let Some(v) = value {
                out.push_str("=\"");
                out.push_str(&escape_attr(v));
                out.push('"');
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub tag: String,
    pub attrs: Attrs,
}

impl Mark {
    pub fn new(tag: &str) -> Mark {
        Mark {
            tag: tag.to_owned(),
            attrs: Attrs::default(),
        }
    }

    pub fn is(&self, kind: Style) -> bool {
        kind.tags().contains(&self.tag.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Bold,
    Italic,
    Underline,
    Strike,
    Sub,
    Sup,
}

impl Style {
    pub fn tags(self) -> &'static [&'static str] {
        match self {
            Style::Bold => &["strong", "b"],
            Style::Italic => &["em", "i"],
            Style::Underline => &["u", "ins"],
            Style::Strike => &["s", "strike", "del"],
            Style::Sub => &["sub"],
            Style::Sup => &["sup"],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text { text: String, marks: Vec<Mark> },
    Object { html: String, marks: Vec<Mark> },
}

impl Inline {
    pub fn len(&self) -> usize {
        match self {
            Inline::Text { text, .. } => text.chars().count(),
            Inline::Object { .. } => 1,
        }
    }

    pub fn marks(&self) -> &Vec<Mark> {
        match self {
            Inline::Text { marks, .. } | Inline::Object { marks, .. } => marks,
        }
    }

    pub fn marks_mut(&mut self) -> &mut Vec<Mark> {
        match self {
            Inline::Text { marks, .. } | Inline::Object { marks, .. } => marks,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Para {
        tag: String,
        attrs: Attrs,
        runs: Vec<Inline>,
    },
    Box {
        tag: String,
        attrs: Attrs,
        children: Vec<Block>,
    },
    Raw(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: Kind,
    pub gap: String,
    pub src: Option<String>,
}

impl Block {
    pub fn new(kind: Kind) -> Block {
        Block {
            kind,
            gap: String::new(),
            src: None,
        }
    }

    pub fn para(tag: &str, runs: Vec<Inline>) -> Block {
        Block::new(Kind::Para {
            tag: tag.to_owned(),
            attrs: Attrs::default(),
            runs,
        })
    }

    pub fn boxed(tag: &str, children: Vec<Block>) -> Block {
        Block::new(Kind::Box {
            tag: tag.to_owned(),
            attrs: Attrs::default(),
            children,
        })
    }

    pub fn tag(&self) -> &str {
        match &self.kind {
            Kind::Para { tag, .. } | Kind::Box { tag, .. } => tag,
            Kind::Raw(_) => "",
        }
    }

    pub fn attrs_mut(&mut self) -> Option<&mut Attrs> {
        match &mut self.kind {
            Kind::Para { attrs, .. } | Kind::Box { attrs, .. } => Some(attrs),
            Kind::Raw(_) => None,
        }
    }

    pub fn attrs(&self) -> Option<&Attrs> {
        match &self.kind {
            Kind::Para { attrs, .. } | Kind::Box { attrs, .. } => Some(attrs),
            Kind::Raw(_) => None,
        }
    }

    pub fn children(&self) -> Option<&Vec<Block>> {
        match &self.kind {
            Kind::Box { children, .. } => Some(children),
            _ => None,
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Block>> {
        match &mut self.kind {
            Kind::Box { children, .. } => Some(children),
            _ => None,
        }
    }

    pub fn runs(&self) -> Option<&Vec<Inline>> {
        match &self.kind {
            Kind::Para { runs, .. } => Some(runs),
            _ => None,
        }
    }

    pub fn runs_mut(&mut self) -> Option<&mut Vec<Inline>> {
        match &mut self.kind {
            Kind::Para { runs, .. } => Some(runs),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Html {
    pub prefix: String,
    pub blocks: Vec<Block>,
    pub tail: String,
    pub suffix: String,
    pub atoms: Vec<String>,
}

pub type Path = Vec<usize>;

impl Html {
    pub fn block(&self, path: &[usize]) -> Option<&Block> {
        let mut list = &self.blocks;
        let mut found = None;
        for (i, index) in path.iter().enumerate() {
            let block = list.get(*index)?;
            if i + 1 == path.len() {
                found = Some(block);
            } else {
                list = block.children()?;
            }
        }
        found
    }

    pub fn block_mut(&mut self, path: &[usize]) -> Option<&mut Block> {
        if let Some(first) = path.first()
            && let Some(top) = self.blocks.get_mut(*first)
        {
            top.src = None;
        }
        let mut list = &mut self.blocks;
        for (i, index) in path.iter().enumerate() {
            if i + 1 == path.len() {
                return list.get_mut(*index);
            }
            list = list.get_mut(*index)?.children_mut()?;
        }
        None
    }

    pub fn list_mut(&mut self, parent: &[usize]) -> Option<&mut Vec<Block>> {
        if parent.is_empty() {
            return Some(&mut self.blocks);
        }
        self.block_mut(parent)?.children_mut()
    }

    pub fn paras(&self) -> Vec<Path> {
        let mut out = Vec::new();
        fn walk(list: &[Block], path: &mut Path, out: &mut Vec<Path>) {
            for (i, block) in list.iter().enumerate() {
                path.push(i);
                match &block.kind {
                    Kind::Para { .. } => out.push(path.clone()),
                    Kind::Box { children, .. } => walk(children, path, out),
                    Kind::Raw(_) => {}
                }
                path.pop();
            }
        }
        walk(&self.blocks, &mut Vec::new(), &mut out);
        out
    }
}

pub fn text_len(runs: &[Inline]) -> usize {
    runs.iter().map(Inline::len).sum()
}

pub fn plain(runs: &[Inline]) -> String {
    runs.iter()
        .map(|r| match r {
            Inline::Text { text, .. } => text.clone(),
            Inline::Object { .. } => OBJ.to_string(),
        })
        .collect()
}

pub fn split_at(runs: &mut Vec<Inline>, at: usize) -> usize {
    let mut pos = 0;
    for i in 0..runs.len() {
        let len = runs[i].len();
        if at == pos {
            return i;
        }
        if at < pos + len {
            if let Inline::Text { text, marks } = &runs[i] {
                let byte = text
                    .char_indices()
                    .nth(at - pos)
                    .map_or(text.len(), |(b, _)| b);
                let tail = Inline::Text {
                    text: text[byte..].to_owned(),
                    marks: marks.clone(),
                };
                if let Inline::Text { text, .. } = &mut runs[i] {
                    text.truncate(byte);
                }
                runs.insert(i + 1, tail);
                return i + 1;
            }
            return i + 1;
        }
        pos += len;
    }
    runs.len()
}

pub fn cut(runs: &mut Vec<Inline>, range: Range<usize>) -> Vec<Inline> {
    let start = split_at(runs, range.start);
    let end = split_at(runs, range.end);
    let taken: Vec<Inline> = runs.drain(start..end).collect();
    normalize(runs);
    taken
}

pub fn normalize(runs: &mut Vec<Inline>) {
    runs.retain(|r| !matches!(r, Inline::Text { text, .. } if text.is_empty()));
    let mut i = 1;
    while i < runs.len() {
        let merge = matches!(
            (&runs[i - 1], &runs[i]),
            (Inline::Text { marks: a, .. }, Inline::Text { marks: b, .. }) if a == b
        );
        if merge {
            if let Inline::Text { text: tail, .. } = runs.remove(i)
                && let Inline::Text { text, .. } = &mut runs[i - 1]
            {
                text.push_str(&tail);
            }
        } else {
            i += 1;
        }
    }
}

pub fn marks_at(runs: &[Inline], at: usize) -> Vec<Mark> {
    let mut pos = 0;
    let mut last = Vec::new();
    for run in runs {
        let len = run.len();
        if at > pos && at <= pos + len {
            return run.marks().clone();
        }
        if at == pos && len > 0 && pos == 0 {
            last = run.marks().clone();
        }
        pos += len;
    }
    last
}

pub fn insert(runs: &mut Vec<Inline>, at: usize, text: &str, marks: &[Mark]) {
    let i = split_at(runs, at);
    runs.insert(
        i,
        Inline::Text {
            text: text.to_owned(),
            marks: marks.to_vec(),
        },
    );
    normalize(runs);
}

pub fn insert_runs(runs: &mut Vec<Inline>, at: usize, new: Vec<Inline>) {
    let i = split_at(runs, at);
    for (k, r) in new.into_iter().enumerate() {
        runs.insert(i + k, r);
    }
    normalize(runs);
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr", "param",
];
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title", "xmp"];
const OPAQUE: &[&str] = &[
    "script", "style", "iframe", "object", "video", "audio", "svg", "canvas", "noscript", "select",
    "textarea", "template", "math", "head", "map", "picture", "button", "input", "embed",
];
const OPAQUE_BLOCK: &[&str] = &[
    "script", "style", "noscript", "template", "head", "iframe", "video", "audio", "object",
    "canvas",
];
pub const BLOCK: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "div",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "legend",
    "dl",
    "dt",
    "dd",
    "address",
    "hr",
    "details",
    "summary",
    "center",
    "body",
    "html",
    "colgroup",
];
const PARA: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "address",
    "dt",
    "dd",
    "caption",
    "figcaption",
    "summary",
    "legend",
];

fn is_block(tag: &str) -> bool {
    BLOCK.contains(&tag) || OPAQUE_BLOCK.contains(&tag)
}

#[derive(Debug)]
enum Node {
    Element {
        tag: String,
        attrs: Attrs,
        children: Vec<Node>,
        span: Range<usize>,
    },
    Text(String, Range<usize>),
    Raw(String, Range<usize>, bool),
}

impl Node {
    fn span(&self) -> Range<usize> {
        match self {
            Node::Element { span, .. } | Node::Text(_, span) | Node::Raw(_, span, _) => {
                span.clone()
            }
        }
    }

    fn blockish(&self) -> bool {
        match self {
            Node::Element { tag, .. } => is_block(tag),
            Node::Raw(_, _, block) => *block,
            Node::Text(..) => false,
        }
    }
}

fn protect(src: &str) -> (String, Vec<String>) {
    let mut atoms = Vec::new();
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        let end = if bytes[i] == b'&' {
            let name: String = src[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .take(32)
                .collect();
            let known = ENTITIES.iter().any(|(n, _)| *n == name);
            (!name.is_empty()
                && !known
                && name.starts_with(|c: char| c.is_ascii_alphabetic())
                && src[i + 1 + name.len()..].starts_with(';'))
            .then(|| i + 2 + name.len())
        } else if bytes[i] == b'<' && bytes.get(i + 1) == Some(&b'?') {
            Some(src[i..].find("?>").map_or(src.len(), |e| i + e + 2))
        } else if bytes[i] == b'{' && matches!(bytes.get(i + 1), Some(b'{' | b'%' | b'#')) {
            let close = match bytes[i + 1] {
                b'{' => "}}",
                b'%' => "%}",
                _ => "#}",
            };
            src[i + 2..].find(close).map(|e| i + 2 + e + 2)
        } else {
            None
        };
        match end {
            Some(end) => {
                out.push_str(&src[last..i]);
                out.push(ATOM_OPEN);
                out.push_str(&atoms.len().to_string());
                out.push(ATOM_CLOSE);
                atoms.push(src[i..end].to_owned());
                i = end;
                last = end;
            }
            None => i += 1,
        }
    }
    out.push_str(&src[last..]);
    (out, atoms)
}

pub fn restore(text: &str, atoms: &[String]) -> String {
    if !text.contains(ATOM_OPEN) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(ATOM_OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + ATOM_OPEN.len_utf8()..];
        match after.find(ATOM_CLOSE) {
            Some(end) => {
                let index: usize = after[..end].parse().unwrap_or(usize::MAX);
                out.push_str(atoms.get(index).map_or("", |s| s.as_str()));
                rest = &after[end + ATOM_CLOSE.len_utf8()..];
            }
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

const ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("laquo", '«'),
    ("raquo", '»'),
    ("mdash", '—'),
    ("ndash", '–'),
    ("hellip", '…'),
    ("copy", '©'),
    ("reg", '®'),
    ("trade", '™'),
    ("deg", '°'),
    ("times", '×'),
    ("euro", '€'),
    ("bull", '•'),
    ("middot", '·'),
    ("rsquo", '’'),
    ("lsquo", '‘'),
    ("rdquo", '”'),
    ("ldquo", '“'),
    ("bdquo", '„'),
    ("sect", '§'),
    ("plusmn", '±'),
    ("frac12", '½'),
    ("larr", '←'),
    ("rarr", '→'),
    ("uarr", '↑'),
    ("darr", '↓'),
];

pub fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| entity(&after[..e]));
        match (end, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let value = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(value);
    }
    ENTITIES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_attr(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    open: Vec<String>,
}

impl Parser<'_> {
    fn nodes(&mut self, stop: Option<&str>) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        while self.pos < self.src.len() {
            let rest = &self.src[self.pos..];
            if let Some(after) = rest.strip_prefix("</") {
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect::<String>()
                    .to_ascii_lowercase();
                if stop.is_some() && self.open.contains(&name) {
                    return out;
                }
                let end = rest.find('>').map_or(self.src.len(), |e| self.pos + e + 1);
                self.pos = end;
                continue;
            }
            if rest.starts_with("<!--") {
                let end = rest
                    .find("-->")
                    .map_or(self.src.len(), |e| self.pos + e + 3);
                let start = self.pos;
                out.push(Node::Raw(
                    self.src[start..end].to_owned(),
                    start..end,
                    false,
                ));
                self.pos = end;
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") {
                let end = rest.find('>').map_or(self.src.len(), |e| self.pos + e + 1);
                let start = self.pos;
                out.push(Node::Raw(self.src[start..end].to_owned(), start..end, true));
                self.pos = end;
                continue;
            }
            if rest.starts_with('<') && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                let start = self.pos;
                let tag_end = find_tag_end(rest).map_or(self.src.len(), |e| self.pos + e + 1);
                let inner = &self.src[self.pos + 1..tag_end.saturating_sub(1).max(self.pos + 1)];
                let name_len = inner
                    .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .unwrap_or(inner.len());
                let tag = inner[..name_len].to_ascii_lowercase();
                let self_closing = inner.trim_end().ends_with('/');
                let attrs = Attrs::parse(&inner[name_len..]);
                self.pos = tag_end;
                if let Some(s) = stop
                    && closes(s, &tag)
                {
                    self.pos = start;
                    return out;
                }
                if VOID.contains(&tag.as_str()) || self_closing {
                    let span = start..self.pos;
                    out.push(Node::Element {
                        tag,
                        attrs,
                        children: Vec::new(),
                        span,
                    });
                    continue;
                }
                if RAW_TEXT.contains(&tag.as_str()) || OPAQUE.contains(&tag.as_str()) {
                    let close = format!("</{tag}");
                    let lower = self.src[self.pos..].to_ascii_lowercase();
                    let end = lower
                        .find(&close)
                        .map(|e| {
                            let e = self.pos + e;
                            self.src[e..]
                                .find('>')
                                .map_or(self.src.len(), |g| e + g + 1)
                        })
                        .unwrap_or(self.src.len());
                    self.pos = end;
                    let block = is_block(&tag);
                    out.push(Node::Raw(
                        self.src[start..end].to_owned(),
                        start..end,
                        block,
                    ));
                    continue;
                }
                self.open.push(tag.clone());
                let children = self.nodes(Some(&tag));
                self.open.pop();
                let rest = &self.src[self.pos..];
                if let Some(after) = rest.strip_prefix("</") {
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                        .collect::<String>()
                        .to_ascii_lowercase();
                    if name == tag {
                        self.pos = rest.find('>').map_or(self.src.len(), |e| self.pos + e + 1);
                    }
                }
                let span = start..self.pos;
                out.push(Node::Element {
                    tag,
                    attrs,
                    children,
                    span,
                });
                continue;
            }
            let start = self.pos;
            let first = rest.chars().next().map_or(1, char::len_utf8);
            let end = rest[first..]
                .find('<')
                .map_or(self.src.len(), |e| self.pos + first + e);
            self.pos = end;
            out.push(Node::Text(decode(&self.src[start..end]), start..end));
        }
        out
    }
}

fn find_tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in rest.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn closes(open: &str, next: &str) -> bool {
    match open {
        "p" => is_block(next),
        "li" => next == "li",
        "dt" | "dd" => next == "dt" || next == "dd",
        "td" | "th" => matches!(next, "td" | "th" | "tr" | "tbody" | "thead" | "tfoot"),
        "tr" => matches!(next, "tr" | "tbody" | "thead" | "tfoot"),
        "thead" | "tbody" => matches!(next, "tbody" | "tfoot"),
        "option" => next == "option",
        _ => false,
    }
}

fn collapse(text: &str, pre: bool) -> String {
    if pre {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

fn inline_runs(nodes: &[Node], src: &str, marks: &mut Vec<Mark>, pre: bool, out: &mut Vec<Inline>) {
    for node in nodes {
        match node {
            Node::Text(text, _) => push_text(out, &collapse(text, pre), marks),
            Node::Raw(raw, _, _) => out.push(Inline::Object {
                html: raw.clone(),
                marks: marks.clone(),
            }),
            Node::Element {
                tag,
                attrs,
                children,
                span,
            } => {
                if tag == "br"
                    || tag == "img"
                    || children.is_empty()
                    || OPAQUE.contains(&tag.as_str())
                {
                    out.push(Inline::Object {
                        html: src[span.clone()].to_owned(),
                        marks: marks.clone(),
                    });
                    continue;
                }
                marks.push(Mark {
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                });
                inline_runs(children, src, marks, pre || tag == "pre", out);
                marks.pop();
            }
        }
    }
}

fn push_text(out: &mut Vec<Inline>, text: &str, marks: &[Mark]) {
    if text.is_empty() {
        return;
    }
    let mut rest = text;
    while let Some(start) = rest.find(ATOM_OPEN) {
        if start > 0 {
            out.push(Inline::Text {
                text: rest[..start].to_owned(),
                marks: marks.to_vec(),
            });
        }
        let after = &rest[start..];
        let end = after
            .find(ATOM_CLOSE)
            .map_or(after.len(), |e| e + ATOM_CLOSE.len_utf8());
        out.push(Inline::Object {
            html: after[..end].to_owned(),
            marks: marks.to_vec(),
        });
        rest = &after[end..];
    }
    if !rest.is_empty() {
        out.push(Inline::Text {
            text: rest.to_owned(),
            marks: marks.to_vec(),
        });
    }
}

fn trim_runs(runs: &mut Vec<Inline>, pre: bool) {
    normalize(runs);
    if pre {
        if let Some(Inline::Text { text, .. }) = runs.first_mut()
            && text.starts_with('\n')
        {
            text.remove(0);
        }
        return;
    }
    if let Some(Inline::Text { text, .. }) = runs.first_mut() {
        *text = text.trim_start_matches(' ').to_owned();
    }
    let last_br = runs.len() >= 2
        && matches!(runs.last(), Some(Inline::Text { text, .. }) if text.trim().is_empty())
        && matches!(&runs[runs.len() - 2], Inline::Object { html, .. } if html.starts_with("<br"));
    if last_br {
        runs.pop();
    }
    if let Some(Inline::Text { text, .. }) = runs.last_mut() {
        *text = text.trim_end_matches(' ').to_owned();
    }
    normalize(runs);
}

fn blocks_of(nodes: &[Node], src: &str, top: bool) -> (Vec<Block>, usize) {
    let mut out: Vec<Block> = Vec::new();
    let mut pending: Vec<&Node> = Vec::new();
    let mut last_end: Option<usize> = None;
    let flush = |pending: &mut Vec<&Node>, out: &mut Vec<Block>, last_end: &mut Option<usize>| {
        if pending.is_empty() {
            return;
        }
        let only_space = pending
            .iter()
            .all(|n| matches!(n, Node::Text(t, _) if t.trim().is_empty()));
        if only_space {
            pending.clear();
            return;
        }
        let owned: Vec<&Node> = std::mem::take(pending);
        let start = owned.first().map(|n| n.span().start).unwrap_or(0);
        let end = owned.last().map(|n| n.span().end).unwrap_or(0);
        let mut runs = Vec::new();
        let mut marks = Vec::new();
        for node in &owned {
            inline_runs(
                std::slice::from_ref(*node),
                src,
                &mut marks,
                false,
                &mut runs,
            );
        }
        trim_runs(&mut runs, false);
        let (gap, s, e) = trimmed_span(src, start, end, if top { *last_end } else { Some(start) });
        let mut block = Block::para("", runs);
        if top {
            block.gap = gap;
            block.src = Some(src[s..e].to_owned());
        }
        *last_end = Some(e);
        out.push(block);
    };
    for node in nodes {
        if !node.blockish() {
            pending.push(node);
            continue;
        }
        flush(&mut pending, &mut out, &mut last_end);
        let span = node.span();
        let mut block = match node {
            Node::Raw(raw, _, _) => Block::new(Kind::Raw(raw.clone())),
            Node::Element {
                tag,
                attrs,
                children,
                ..
            } => element_block(tag, attrs, children, src),
            Node::Text(..) => continue,
        };
        if top {
            let from = last_end.unwrap_or(0);
            block.gap = src[from.min(span.start)..span.start].to_owned();
            block.src = Some(src[span.clone()].to_owned());
        }
        last_end = Some(span.end);
        out.push(block);
    }
    flush(&mut pending, &mut out, &mut last_end);
    (out, last_end.unwrap_or(0))
}

fn trimmed_span(
    src: &str,
    start: usize,
    end: usize,
    last_end: Option<usize>,
) -> (String, usize, usize) {
    let slice = &src[start..end];
    let s = start + (slice.len() - slice.trim_start().len());
    let e = end - (slice.len() - slice.trim_end().len());
    let from = last_end.unwrap_or(0).min(s);
    (src[from..s].to_owned(), s, e.max(s))
}

fn element_block(tag: &str, attrs: &Attrs, children: &[Node], src: &str) -> Block {
    let inline_only = children.iter().all(|c| !c.blockish());
    if PARA.contains(&tag) && inline_only || tag == "pre" {
        let pre = tag == "pre";
        let mut runs = Vec::new();
        inline_runs(children, src, &mut Vec::new(), pre, &mut runs);
        trim_runs(&mut runs, pre);
        return Block::new(Kind::Para {
            tag: tag.to_owned(),
            attrs: attrs.clone(),
            runs,
        });
    }
    if tag == "hr" {
        return Block::new(Kind::Raw(format!("<hr{}>", {
            let mut s = String::new();
            attrs.write(&mut s);
            s
        })));
    }
    let mut kids = blocks_of(children, src, false).0;
    let cell = matches!(tag, "li" | "td" | "th" | "dd" | "dt" | "blockquote" | "div");
    if kids.is_empty() && (cell || PARA.contains(&tag)) {
        kids.push(Block::para("", Vec::new()));
    }
    Block::new(Kind::Box {
        tag: tag.to_owned(),
        attrs: attrs.clone(),
        children: kids,
    })
}

pub fn parse(source: &str) -> Html {
    let (text, atoms) = protect(source);
    let lower = text.to_ascii_lowercase();
    let (body_start, body_end) = match lower.find("<body") {
        Some(b) => {
            let open_end = text[b..].find('>').map_or(text.len(), |e| b + e + 1);
            let close = lower.rfind("</body").unwrap_or(text.len()).max(open_end);
            (open_end, close)
        }
        None => (0, text.len()),
    };
    let body = &text[body_start..body_end];
    let mut parser = Parser {
        src: body,
        pos: 0,
        open: Vec::new(),
    };
    let nodes = parser.nodes(None);
    let (blocks, tail_start) = blocks_of(&nodes, body, true);
    Html {
        prefix: text[..body_start].to_owned(),
        blocks,
        tail: body[tail_start.min(body.len())..].to_owned(),
        suffix: text[body_end..].to_owned(),
        atoms,
    }
}

pub fn write_runs(runs: &[Inline], out: &mut String) {
    let mut open: Vec<&Mark> = Vec::new();
    for run in runs {
        let marks = run.marks();
        let common = open
            .iter()
            .zip(marks.iter())
            .take_while(|(a, b)| **a == *b)
            .count();
        while open.len() > common {
            let mark = open.pop().unwrap_or_else(|| unreachable!());
            out.push_str(&format!("</{}>", mark.tag));
        }
        for mark in &marks[common..] {
            out.push('<');
            out.push_str(&mark.tag);
            mark.attrs.write(out);
            out.push('>');
            open.push(mark);
        }
        match run {
            Inline::Text { text, .. } => out.push_str(&escape_text(text)),
            Inline::Object { html, .. } => out.push_str(html),
        }
    }
    while let Some(mark) = open.pop() {
        out.push_str(&format!("</{}>", mark.tag));
    }
}

fn write_block(block: &Block, depth: usize, out: &mut String) {
    let indent = "\t".repeat(depth);
    match &block.kind {
        Kind::Raw(raw) => {
            out.push_str(&indent);
            out.push_str(raw);
        }
        Kind::Para { tag, attrs, runs } => {
            out.push_str(&indent);
            if tag.is_empty() {
                write_runs(runs, out);
                if runs.is_empty() {
                    out.push_str("&nbsp;");
                }
                return;
            }
            out.push('<');
            out.push_str(tag);
            attrs.write(out);
            out.push('>');
            write_runs(runs, out);
            if runs.is_empty() && tag != "pre" {
                out.push_str("&nbsp;");
            }
            out.push_str(&format!("</{tag}>"));
        }
        Kind::Box {
            tag,
            attrs,
            children,
        } => {
            out.push_str(&indent);
            out.push('<');
            out.push_str(tag);
            attrs.write(out);
            out.push('>');
            let simple = children.len() == 1
                && matches!(&children[0].kind, Kind::Para { tag, .. } if tag.is_empty());
            if simple {
                if let Kind::Para { runs, .. } = &children[0].kind {
                    write_runs(runs, out);
                }
            } else {
                for child in children {
                    out.push('\n');
                    write_block(child, depth + 1, out);
                }
                out.push('\n');
                out.push_str(&indent);
            }
            out.push_str(&format!("</{tag}>"));
        }
    }
}

fn shift_atoms(text: &mut String, base: usize) {
    if base == 0 || !text.contains(ATOM_OPEN) {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find(ATOM_OPEN) {
        out.push_str(&rest[..start + ATOM_OPEN.len_utf8()]);
        let after = &rest[start + ATOM_OPEN.len_utf8()..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        match after[..digits].parse::<usize>() {
            Ok(n) if after[digits..].starts_with(ATOM_CLOSE) => {
                out.push_str(&(n + base).to_string())
            }
            _ => out.push_str(&after[..digits]),
        }
        rest = &after[digits..];
    }
    out.push_str(rest);
    *text = out;
}

fn shift_attrs(attrs: &mut Attrs, base: usize) {
    for (_, value) in &mut attrs.0 {
        if let Some(v) = value {
            shift_atoms(v, base);
        }
    }
}

fn shift_runs(runs: &mut [Inline], base: usize) {
    for run in runs {
        match run {
            Inline::Text { text, .. } => shift_atoms(text, base),
            Inline::Object { html, .. } => shift_atoms(html, base),
        }
        for mark in run.marks_mut() {
            shift_attrs(&mut mark.attrs, base);
        }
    }
}

fn shift_blocks(blocks: &mut [Block], base: usize) {
    for block in blocks {
        block.src = None;
        block.gap.clear();
        match &mut block.kind {
            Kind::Raw(raw) => shift_atoms(raw, base),
            Kind::Para { attrs, runs, .. } => {
                shift_attrs(attrs, base);
                shift_runs(runs, base);
            }
            Kind::Box {
                attrs, children, ..
            } => {
                shift_attrs(attrs, base);
                shift_blocks(children, base);
            }
        }
    }
}

pub fn adopt(blocks: &mut [Block], atoms: &[String], into: &mut Vec<String>) {
    let base = into.len();
    into.extend(atoms.iter().cloned());
    shift_blocks(blocks, base);
}

pub fn serialize(html: &Html) -> String {
    let mut out = html.prefix.clone();
    for (i, block) in html.blocks.iter().enumerate() {
        match &block.src {
            Some(src) => {
                out.push_str(&block.gap);
                out.push_str(src);
            }
            None => {
                let gap = if block.gap.is_empty() && i > 0 {
                    "\n"
                } else {
                    block.gap.as_str()
                };
                out.push_str(gap);
                let start = out.len();
                write_block(block, 0, &mut out);
                let _ = start;
            }
        }
    }
    out.push_str(&html.tail);
    out.push_str(&html.suffix);
    restore(&out, &html.atoms)
}

pub fn object_label(html: &str, atoms: &[String]) -> (String, ObjectKind) {
    let full = restore(html, atoms);
    let lower = full.to_ascii_lowercase();
    if lower.starts_with("<br") {
        return ("\n".into(), ObjectKind::Break);
    }
    if lower.starts_with("<img") {
        let attrs = Attrs::parse(
            full.trim_start_matches(['<', 'i', 'm', 'g', 'I', 'M', 'G'])
                .trim_end_matches(['>', '/']),
        );
        let src = attrs.get("src").unwrap_or_default();
        let name = attrs
            .get("alt")
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| src.rsplit('/').next().unwrap_or(src).to_owned());
        return (format!("img · {name}"), ObjectKind::Image);
    }
    if lower.starts_with("<a") {
        let attrs = Attrs::parse(
            full.get(2..)
                .unwrap_or_default()
                .split('>')
                .next()
                .unwrap_or(""),
        );
        let name = attrs
            .get("name")
            .or(attrs.get("id"))
            .unwrap_or("")
            .to_owned();
        return (format!("#{name}"), ObjectKind::Anchor);
    }
    if lower.starts_with("<!--") {
        return ("<!-- -->".into(), ObjectKind::Other);
    }
    if full.starts_with('&') {
        return (full, ObjectKind::Entity);
    }
    let one: String = full.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = one.chars().take(40).collect();
    let short = if one.chars().count() > 40 {
        format!("{short}…")
    } else {
        short
    };
    (short, ObjectKind::Code)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectKind {
    Break,
    Image,
    Anchor,
    Entity,
    Code,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_round_trip_is_exact() {
        let src = "<h1 class=\"t\">Заголовок</h1>\n\n<p>Текст <b>жирный</b> &laquo;к&raquo; <?= $x ?></p>\n<ul>\n  <li>один</li>\n  <li>два</li>\n</ul>\n<script>var a = 1 < 2;</script>\n{% if a %}<div>б</div>{% endif %}\n";
        let html = parse(src);
        assert_eq!(serialize(&html), src);
        let page = "<!DOCTYPE html>\n<html><head><title>x</title></head>\n<body class=\"a\">\n<p>Hi</p>\n</body></html>\n";
        assert_eq!(serialize(&parse(page)), page);
    }

    #[test]
    fn edited_block_is_regenerated() {
        let src = "<p>a <strong>b</strong> c</p>\n<p>keep  me</p>";
        let mut html = parse(src);
        let paths = html.paras();
        let runs = html.block_mut(&paths[0]).unwrap().runs_mut().unwrap();
        insert(runs, 1, "X", &[]);
        assert_eq!(
            serialize(&html),
            "<p>aX <strong>b</strong> c</p>\n<p>keep  me</p>"
        );
    }

    #[test]
    fn atoms_and_lists() {
        let src = "<ul><li>a<li>b <?php echo 1; ?></ul>";
        let html = parse(src);
        let paras = html.paras();
        assert_eq!(paras.len(), 2);
        let b = html.block(&paras[1]).unwrap();
        assert_eq!(text_len(b.runs().unwrap()), 3);
        let mut edited = html.clone();
        edited.block_mut(&paras[0]);
        assert_eq!(
            serialize(&edited),
            "<ul>\n\t<li>a</li>\n\t<li>b <?php echo 1; ?></li>\n</ul>"
        );
    }

    #[test]
    fn styles_and_attrs() {
        let mut attrs =
            Attrs::parse(" class=\"x\" style=\"color: red; margin-left: 40px\" data-a='1' hidden");
        assert_eq!(attrs.style("margin-left").as_deref(), Some("40px"));
        attrs.set_style("color", None);
        attrs.set_style("text-align", Some("center"));
        let mut out = String::new();
        attrs.write(&mut out);
        assert_eq!(
            out,
            " class=\"x\" style=\"margin-left: 40px; text-align: center;\" data-a=\"1\" hidden"
        );
    }
}
