use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use ropey::Rope;

use crate::highlight::Highlighter;

const MAX_UNDO: usize = 2000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Eol {
    Lf,
    Crlf,
    Cr,
}

impl Eol {
    pub fn label(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::Crlf => "CRLF",
            Eol::Cr => "CR",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::Crlf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    fn native() -> Self {
        if cfg!(windows) { Eol::Crlf } else { Eol::Lf }
    }
}

pub struct Decoded {
    pub text: String,
    pub encoding: &'static Encoding,
    pub bom: bool,
    pub eol: Eol,
}

pub fn decode(bytes: Vec<u8>, forced: Option<&'static Encoding>) -> Result<Decoded, String> {
    let (hint, bom_len) = match (forced, Encoding::for_bom(&bytes)) {
        (Some(enc), Some((bom_enc, len))) if enc == bom_enc => (Some(enc), len),
        (Some(enc), _) => (Some(enc), 0),
        (None, Some((enc, len))) => (Some(enc), len),
        (None, None) => {
            if memchr::memchr(0, &bytes).is_some() {
                return Err("похоже на бинарный файл".into());
            }
            (None, 0)
        }
    };
    let (mut text, encoding) = match hint {
        Some(encoding) if encoding != UTF_8 => (
            encoding
                .decode_without_bom_handling(&bytes[bom_len..])
                .0
                .into_owned(),
            encoding,
        ),
        _ => match String::from_utf8(bytes) {
            Ok(mut text) => {
                text.drain(..bom_len);
                (text, UTF_8)
            }
            Err(e) if hint.is_some() => (
                String::from_utf8_lossy(&e.as_bytes()[bom_len..]).into_owned(),
                UTF_8,
            ),
            Err(e) => {
                let bytes = e.into_bytes();
                let mut detector =
                    chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
                let sample = &bytes[..bytes.len().min(4 * 1024 * 1024)];
                detector.feed(sample, sample.len() == bytes.len());
                let encoding = detector.guess(Some(b"ru"), chardetng::Utf8Detection::Deny);
                (
                    encoding.decode_without_bom_handling(&bytes).0.into_owned(),
                    encoding,
                )
            }
        },
    };
    let eol = if memchr::memchr(b'\r', text.as_bytes()).is_some() {
        let eol = if text.contains("\r\n") {
            Eol::Crlf
        } else {
            Eol::Cr
        };
        text = text.replace("\r\n", "\n");
        if text.as_bytes().contains(&b'\r') {
            text = text.replace('\r', "\n");
        }
        eol
    } else if memchr::memchr(b'\n', text.as_bytes()).is_some() {
        Eol::Lf
    } else {
        Eol::native()
    };
    Ok(Decoded {
        text,
        encoding,
        bom: bom_len > 0,
        eol,
    })
}

pub fn encode(text: &str, encoding: &'static Encoding, bom: bool, eol: Eol) -> Vec<u8> {
    let converted;
    let text = if eol == Eol::Lf {
        text
    } else {
        converted = text.replace('\n', eol.as_str());
        &converted
    };
    if encoding == UTF_16LE || encoding == UTF_16BE {
        let mut out = Vec::with_capacity(text.len() * 2 + 2);
        let le = encoding == UTF_16LE;
        let mut push = |unit: u16| {
            out.extend_from_slice(&if le {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            })
        };
        if bom {
            push(0xFEFF);
        }
        text.encode_utf16().for_each(push);
        return out;
    }
    let mut out = Vec::with_capacity(text.len() + 3);
    if bom && encoding == UTF_8 {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(&encoding.encode(text).0);
    out
}

fn longest_line(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut longest = 0;
    let mut start = 0;
    let ends = memchr::memchr_iter(b'\n', bytes).chain(std::iter::once(bytes.len()));
    for end in ends {
        if end - start > longest {
            longest = longest.max(text[start..end].chars().count());
        }
        start = end + 1;
    }
    longest
}

fn comment_tokens(syntax: &str) -> (&'static str, &'static str) {
    let s = syntax.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| s.contains(w));
    if has(&["twig"]) {
        ("{#", "#}")
    } else if has(&["html", "xml", "markdown", "vue", "svg"]) {
        ("<!--", "-->")
    } else if s == "css" {
        ("/*", "*/")
    } else if has(&["sql", "lua", "haskell"]) {
        ("--", "")
    } else if has(&[
        "python",
        "yaml",
        "shell",
        "bash",
        "toml",
        "ruby",
        "perl",
        "nginx",
        "docker",
        "makefile",
        "ini",
        "apache",
        "plain text",
        "git",
        "properties",
    ]) {
        ("#", "")
    } else if has(&["batch"]) {
        ("REM", "")
    } else {
        ("//", "")
    }
}

fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum EditKind {
    Insert,
    Delete,
    Other,
}

struct Snapshot {
    rope: Rope,
    sel: (usize, usize),
    version: u64,
    max_line_chars: usize,
}

pub struct Document {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub remote: Option<(String, String)>,
    pub missing: bool,
    pub missing_ack: bool,
    pub untitled: u32,
    pub pinned: bool,
    pub rope: Rope,
    pub encoding: &'static Encoding,
    pub bom: bool,
    pub eol: Eol,
    pub syntax: String,
    pub hl: Highlighter,
    pub version: u64,
    saved_version: u64,
    next_version: u64,
    format_changed: bool,
    pub sel: (usize, usize),
    pub extra: Vec<(usize, usize)>,
    pub block: Option<((usize, f64), (usize, f64))>,
    pub goal_x: Option<f64>,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub reveal: bool,
    pub max_line_chars: usize,
    pub content_w: f64,
    pub activity: f64,
    pub folds: Vec<(usize, usize)>,
    pub marks: std::collections::BTreeSet<usize>,
    pub hidden: Vec<(usize, usize)>,
    pub(crate) skip: Vec<usize>,
    pub(crate) ends: std::cell::RefCell<(u64, std::collections::HashMap<usize, Option<usize>>)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(EditKind, Instant)>,
}

static INDENT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn set_indent(unit: String) {
    if let Ok(mut current) = INDENT.lock()
        && *current != unit
    {
        *current = unit;
    }
}

fn indent_unit() -> String {
    INDENT
        .lock()
        .ok()
        .filter(|unit| !unit.is_empty())
        .map_or_else(|| "\t".to_owned(), |unit| unit.clone())
}

impl Document {
    pub fn empty(id: u64, untitled: u32) -> Self {
        Self {
            id,
            path: None,
            remote: None,
            missing: false,
            missing_ack: false,
            pinned: false,
            untitled,
            rope: Rope::new(),
            encoding: UTF_8,
            bom: false,
            eol: Eol::native(),
            syntax: "Plain Text".into(),
            hl: Highlighter::default(),
            version: 0,
            saved_version: 0,
            next_version: 1,
            format_changed: false,
            sel: (0, 0),
            extra: Vec::new(),
            block: None,
            goal_x: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            reveal: false,
            max_line_chars: 0,
            content_w: 0.0,
            activity: 0.0,
            folds: Vec::new(),
            marks: Default::default(),
            hidden: Vec::new(),
            skip: vec![0],
            ends: Default::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        }
    }

    #[cfg(test)]
    pub fn from_text(text: &str) -> Self {
        let mut doc = Self::empty(1, 1);
        doc.load_text(text);
        doc.saved_version = doc.version;
        doc
    }

    pub fn open(id: u64, path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let decoded = decode(bytes, None)?;
        let mut doc = Self::empty(id, 0);
        doc.path = Some(path.to_path_buf());
        doc.apply_decoded(decoded);
        Ok(doc)
    }

    pub fn open_remote(id: u64, profile: &str, path: &str, bytes: Vec<u8>) -> Result<Self, String> {
        let decoded = decode(bytes, None)?;
        let mut doc = Self::empty(id, 0);
        doc.remote = Some((profile.to_owned(), path.to_owned()));
        doc.apply_decoded(decoded);
        Ok(doc)
    }

    pub fn encoded(&self) -> Vec<u8> {
        encode(&String::from(&self.rope), self.encoding, self.bom, self.eol)
    }

    pub fn mark_saved(&mut self, version: u64) {
        self.saved_version = version;
        self.format_changed = false;
        self.missing = false;
        self.missing_ack = false;
    }

    pub fn toggle_comment(&mut self) {
        let (open, close) = comment_tokens(&self.syntax);
        let lines = self.selected_lines();
        let single = lines.len() < 2;
        let text_of = |doc: &Self, line: usize| -> String {
            let start = doc.rope.line_to_char(line);
            doc.rope
                .slice(start..start + doc.line_len(line))
                .to_string()
        };
        let busy: Vec<usize> = lines
            .clone()
            .filter(|l| !text_of(self, *l).trim().is_empty())
            .collect();
        if busy.is_empty() {
            return;
        }
        let commented = busy.iter().all(|l| {
            let text = text_of(self, *l);
            let body = text.trim();
            body.starts_with(open)
                && body.ends_with(close)
                && body.len() >= open.len() + close.len()
        });
        let indent = busy
            .iter()
            .map(|l| {
                text_of(self, *l)
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .count()
            })
            .min()
            .unwrap_or(0);
        let head = self.sel.1;
        let head_line = self.rope.char_to_line(head);
        let mut head_shift: isize = 0;
        self.checkpoint(EditKind::Other);
        for line in busy {
            let text = text_of(self, line);
            let start = self.rope.line_to_char(line);
            if commented {
                let lead = text.chars().take_while(|c| c.is_whitespace()).count();
                let mut remove = open.chars().count();
                if text.chars().nth(lead + remove) == Some(' ') {
                    remove += 1;
                }
                if !close.is_empty() {
                    let trimmed = text.trim_end();
                    let mut tail = close.chars().count();
                    let body_end = trimmed.chars().count();
                    if trimmed.chars().nth(body_end - tail - 1) == Some(' ') {
                        tail += 1;
                    }
                    self.apply(start + body_end - tail..start + body_end, "");
                }
                self.apply(start + lead..start + lead + remove, "");
                if line == head_line && head >= start + lead {
                    head_shift = -(remove.min(head - start - lead) as isize);
                }
            } else {
                if !close.is_empty() {
                    let end = start + text.trim_end().chars().count();
                    self.apply(end..end, &format!(" {close}"));
                }
                let insert = format!("{open} ");
                self.apply(start + indent..start + indent, &insert);
                if line == head_line && head >= start + indent {
                    head_shift = insert.chars().count() as isize;
                }
            }
        }
        if single {
            let pos = ((head as isize + head_shift).max(0) as usize).min(self.len());
            self.sel = (pos, pos);
        } else {
            self.select_lines(lines);
        }
        self.reveal = true;
    }

    fn load_text(&mut self, text: &str) {
        let (rope, longest) = if text.len() > 4 * 1024 * 1024 {
            std::thread::scope(|scope| {
                let longest = scope.spawn(|| longest_line(text));
                let rope = Rope::from_str(text);
                (rope, longest.join().unwrap_or(0))
            })
        } else {
            (Rope::from_str(text), longest_line(text))
        };
        self.rope = rope;
        self.drop_folds();
        self.max_line_chars = longest;
        self.hl = Highlighter::default();
        self.sel = (0, 0);
        self.content_w = 0.0;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        self.bump();
    }

    fn apply_decoded(&mut self, d: Decoded) {
        self.load_text(&d.text);
        self.encoding = d.encoding;
        self.bom = d.bom;
        self.eol = d.eol;
        self.saved_version = self.version;
        self.format_changed = false;
    }

    pub fn location(&self) -> Option<String> {
        match (&self.remote, &self.path) {
            (Some((profile, path)), _) => Some(format!("{profile}: {path}")),
            (None, Some(path)) => Some(path.display().to_string()),
            (None, None) => None,
        }
    }

    pub fn reopen_as(&mut self, encoding: &'static Encoding) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or("файл не сохранён на этом компьютере")?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let decoded = decode(bytes, Some(encoding))?;
        self.apply_decoded(decoded);
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self.path.clone().ok_or("не задан путь")?;
        std::fs::write(&path, self.encoded()).map_err(|e| e.to_string())?;
        self.remote = None;
        self.mark_saved(self.version);
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        self.version != self.saved_version || self.format_changed
    }

    pub fn mark_modified(&mut self) {
        self.format_changed = true;
    }

    pub fn title(&self) -> String {
        if let Some((_, path)) = &self.remote {
            return crate::remote::file_name(path).to_owned();
        }
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            None => format!("Новый {}", self.untitled),
        }
    }

    pub fn first_line(&self) -> String {
        self.rope.line(0).chars().take(200).collect()
    }

    pub fn len(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len - 1
        } else {
            len
        }
    }

    pub fn selection(&self) -> Range<usize> {
        self.sel.0.min(self.sel.1)..self.sel.0.max(self.sel.1)
    }

    pub fn has_selection(&self) -> bool {
        self.sel.0 != self.sel.1
    }

    pub fn selected_text(&self) -> String {
        self.rope.slice(self.selection()).to_string()
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.rope.char_to_line(self.sel.1);
        (line + 1, self.sel.1 - self.rope.line_to_char(line) + 1)
    }

    fn bump(&mut self) {
        self.version = self.next_version;
        self.next_version += 1;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            rope: self.rope.clone(),
            sel: self.sel,
            version: self.version,
            max_line_chars: self.max_line_chars,
        }
    }

    fn checkpoint(&mut self, kind: EditKind) {
        let now = Instant::now();
        let coalesce = kind != EditKind::Other
            && self
                .last_edit
                .is_some_and(|(k, t)| k == kind && now.duration_since(t).as_millis() < 1000);
        if !coalesce {
            self.undo.push(self.snapshot());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some((kind, now));
    }

    fn apply(&mut self, range: Range<usize>, text: &str) -> usize {
        let start_line = self.rope.char_to_line(range.start);
        let old_end_line = self.rope.char_to_line(range.end);
        if !range.is_empty() {
            self.rope.remove(range.clone());
        }
        if !text.is_empty() {
            self.rope.insert(range.start, text);
        }
        let end = range.start + text.chars().count();
        let new_end_line = self.rope.char_to_line(end);
        self.hl.edit(
            start_line,
            old_end_line - start_line + 1,
            new_end_line - start_line + 1,
        );
        self.shift_folds(start_line, old_end_line, new_end_line);
        self.shift_marks(start_line, old_end_line, new_end_line);
        if new_end_line - start_line < 64 {
            for line in start_line..=new_end_line {
                self.max_line_chars = self.max_line_chars.max(self.line_len(line));
            }
        } else {
            let bound =
                longest_line(text) + self.line_len(start_line) + self.line_len(new_end_line);
            self.max_line_chars = self.max_line_chars.max(bound);
        }
        self.bump();
        end
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str, kind: EditKind) {
        self.extra.clear();
        self.block = None;
        self.checkpoint(kind);
        let end = self.apply(range, text);
        self.sel = (end, end);
        self.goal_x = None;
        self.reveal = true;
    }

    pub fn apply_edits(&mut self, edits: &[(Range<usize>, String)], sel: (usize, usize)) {
        self.extra.clear();
        self.block = None;
        if !edits.is_empty() {
            self.checkpoint(EditKind::Other);
            for (range, text) in edits {
                let len = self.len();
                self.apply(range.start.min(len)..range.end.min(len), text);
            }
        }
        let len = self.len();
        self.sel = (sel.0.min(len), sel.1.min(len));
        self.last_edit = None;
        self.goal_x = None;
        self.reveal = true;
    }

    pub fn set_text(&mut self, text: &str) {
        self.checkpoint(EditKind::Other);
        self.rope = Rope::from_str(text);
        self.drop_folds();
        self.max_line_chars = longest_line(text);
        self.hl.reset();
        let pos = self.sel.1.min(self.len());
        self.sel = (pos, pos);
        self.bump();
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.extra.clear();
        self.block = None;
        self.hl.replace_all(&self.rope, &snapshot.rope);
        self.rope = snapshot.rope;
        self.drop_folds();
        self.sel = snapshot.sel;
        self.version = snapshot.version;
        self.max_line_chars = self.max_line_chars.max(snapshot.max_line_chars);
        self.last_edit = None;
        self.goal_x = None;
        self.reveal = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(snapshot);
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(snapshot);
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        if self.multi() {
            self.insert_each(text);
            return;
        }
        let kind = if text.contains('\n') || self.has_selection() {
            EditKind::Other
        } else {
            EditKind::Insert
        };
        self.replace(self.selection(), text, kind);
    }

    pub fn newline(&mut self) {
        if self.multi() {
            self.edit_each(EditKind::Other, |doc, c| {
                let start = c.0.min(c.1);
                let line = doc.rope.char_to_line(start);
                let line_start = doc.rope.line_to_char(line);
                let indent: String = doc
                    .rope
                    .slice(line_start..start)
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                (start..c.0.max(c.1), format!("\n{indent}"))
            });
            return;
        }
        let start = self.selection().start;
        let line = self.rope.char_to_line(start);
        let line_start = self.rope.line_to_char(line);
        let indent: String = self
            .rope
            .slice(line_start..start)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.replace(self.selection(), &format!("\n{indent}"), EditKind::Other);
    }

    pub fn backspace(&mut self, word: bool) {
        if self.multi() {
            self.edit_each(EditKind::Delete, |doc, c| {
                if c.0 != c.1 {
                    (c.0.min(c.1)..c.0.max(c.1), String::new())
                } else if c.1 == 0 {
                    (0..0, String::new())
                } else if word {
                    (doc.word_left(c.1)..c.1, String::new())
                } else {
                    (c.1 - 1..c.1, String::new())
                }
            });
            return;
        }
        if self.has_selection() {
            self.replace(self.selection(), "", EditKind::Other);
        } else if self.sel.1 > 0 {
            let start = if word {
                self.word_left(self.sel.1)
            } else {
                self.sel.1 - 1
            };
            self.replace(start..self.sel.1, "", EditKind::Delete);
        }
    }

    pub fn delete(&mut self, word: bool) {
        if self.multi() {
            let len = self.len();
            self.edit_each(EditKind::Delete, |doc, c| {
                if c.0 != c.1 {
                    (c.0.min(c.1)..c.0.max(c.1), String::new())
                } else if c.1 >= len {
                    (len..len, String::new())
                } else if word {
                    (c.1..doc.word_right(c.1), String::new())
                } else {
                    (c.1..c.1 + 1, String::new())
                }
            });
            return;
        }
        if self.has_selection() {
            self.replace(self.selection(), "", EditKind::Other);
        } else if self.sel.1 < self.len() {
            let end = if word {
                self.word_right(self.sel.1)
            } else {
                self.sel.1 + 1
            };
            self.replace(self.sel.1..end, "", EditKind::Delete);
        }
    }

    fn selected_lines(&self) -> Range<usize> {
        let range = self.selection();
        let first = self.rope.char_to_line(range.start);
        let mut last = self.rope.char_to_line(range.end);
        if last > first && self.rope.line_to_char(last) == range.end {
            last -= 1;
        }
        first..last + 1
    }

    pub fn indent(&mut self) {
        let unit = indent_unit();
        if self.multi() {
            self.insert_each(&unit);
            return;
        }
        let lines = self.selected_lines();
        if lines.len() < 2 {
            self.insert_text(&unit);
            return;
        }
        self.checkpoint(EditKind::Other);
        for line in lines.clone() {
            if self.line_len(line) > 0 {
                let at = self.rope.line_to_char(line);
                self.apply(at..at, &unit);
            }
        }
        self.select_lines(lines);
    }

    pub fn outdent(&mut self) {
        let lines = self.selected_lines();
        let single = lines.len() < 2;
        let head = self.sel.1;
        self.checkpoint(EditKind::Other);
        let mut removed_before_head = 0;
        for line in lines.clone() {
            let at = self.rope.line_to_char(line);
            let mut remove = 0;
            for c in self.rope.line(line).chars().take(4) {
                match c {
                    '\t' if remove == 0 => {
                        remove = 1;
                        break;
                    }
                    ' ' => remove += 1,
                    _ => break,
                }
            }
            if remove > 0 {
                self.apply(at..at + remove, "");
                removed_before_head = remove.min(head.saturating_sub(at));
            }
        }
        if single {
            let pos = head - removed_before_head;
            self.sel = (pos, pos);
        } else {
            self.select_lines(lines);
        }
        self.reveal = true;
    }

    fn select_lines(&mut self, lines: Range<usize>) {
        let start = self.rope.line_to_char(lines.start);
        let end = if lines.end < self.line_count() {
            self.rope.line_to_char(lines.end)
        } else {
            self.len()
        };
        self.sel = (start, end);
        self.reveal = true;
    }

    pub fn multi(&self) -> bool {
        !self.extra.is_empty()
    }

    pub fn cursors(&self) -> Vec<(usize, usize)> {
        let mut all = Vec::with_capacity(self.extra.len() + 1);
        all.push(self.sel);
        all.extend(self.extra.iter().copied());
        all
    }

    pub fn set_cursors(&mut self, cursors: Vec<(usize, usize)>, primary: usize) {
        let len = self.len();
        let primary_sel = cursors.get(primary).copied();
        let mut list: Vec<(usize, usize)> = cursors
            .into_iter()
            .map(|(a, h)| (a.min(len), h.min(len)))
            .collect();
        list.sort_by_key(|(a, h)| (*a.min(h), *a.max(h)));
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(list.len());
        for cur in list {
            let (lo, hi) = (cur.0.min(cur.1), cur.0.max(cur.1));
            if let Some(last) = merged.last_mut() {
                let (llo, lhi) = (last.0.min(last.1), last.0.max(last.1));
                if lo < lhi || (lo == llo && hi == lhi) || (lo == lhi && lo == hi) {
                    let start = llo.min(lo);
                    let end = lhi.max(hi);
                    *last = if last.1 >= last.0 {
                        (start, end)
                    } else {
                        (end, start)
                    };
                    continue;
                }
            }
            merged.push(cur);
        }
        let primary = primary_sel
            .and_then(|p| {
                merged.iter().position(|c| {
                    let (lo, hi) = (c.0.min(c.1), c.0.max(c.1));
                    lo <= p.1.min(len) && p.1.min(len) <= hi
                })
            })
            .unwrap_or(merged.len().saturating_sub(1));
        if merged.is_empty() {
            return;
        }
        self.sel = merged.remove(primary);
        self.extra = merged;
        self.last_edit = None;
        self.goal_x = None;
    }

    pub fn clear_extra(&mut self) {
        self.extra.clear();
        self.block = None;
    }

    pub fn edit_each(
        &mut self,
        kind: EditKind,
        f: impl Fn(&Self, (usize, usize)) -> (Range<usize>, String),
    ) {
        let cursors = self.cursors();
        let mut edits: Vec<(usize, Range<usize>, String)> = cursors
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (range, text) = f(self, *c);
                (i, range, text)
            })
            .collect();
        edits.sort_by_key(|(_, r, _)| (r.start, r.end));
        let mut last_end = 0;
        edits.retain(|(_, r, _)| {
            let keep = r.start >= last_end || r.is_empty() && r.start == last_end;
            if keep {
                last_end = r.end;
            }
            keep
        });
        self.checkpoint(kind);
        let mut shift: isize = 0;
        let mut carets = vec![None; cursors.len()];
        for (i, range, text) in edits {
            let start = (range.start as isize + shift) as usize;
            let end = (range.end as isize + shift) as usize;
            let caret = self.apply(start..end, &text);
            shift += text.chars().count() as isize - (range.end - range.start) as isize;
            carets[i] = Some(caret);
        }
        let primary = 0;
        let list: Vec<(usize, usize)> = carets.into_iter().flatten().map(|c| (c, c)).collect();
        self.block = None;
        self.set_cursors(list, primary);
        self.reveal = true;
    }

    pub fn transform(&mut self, f: impl Fn(&str) -> String) {
        if !self.multi() && !self.has_selection() {
            self.select_word_at(self.sel.1);
            if !self.has_selection() {
                return;
            }
        }
        let mut ranges: Vec<(usize, usize, bool)> = self
            .cursors()
            .into_iter()
            .map(|(a, h)| (a.min(h), a.max(h), h >= a))
            .filter(|(lo, hi, _)| hi > lo)
            .collect();
        if ranges.is_empty() {
            return;
        }
        ranges.sort();
        self.checkpoint(EditKind::Other);
        let mut shift: isize = 0;
        let mut sels = Vec::with_capacity(ranges.len());
        for (lo, hi, forward) in ranges {
            let start = (lo as isize + shift) as usize;
            let end = (hi as isize + shift) as usize;
            let old = self.rope.slice(start..end).to_string();
            let new = f(&old);
            let n = new.chars().count();
            if new != old {
                self.apply(start..end, &new);
            }
            shift += n as isize - (end - start) as isize;
            sels.push(if forward {
                (start, start + n)
            } else {
                (start + n, start)
            });
        }
        self.block = None;
        self.set_cursors(sels, 0);
        self.reveal = true;
    }

    fn line_span(&self) -> (usize, usize) {
        let range = self.selection();
        let first = self.rope.char_to_line(range.start);
        let mut last = self.rope.char_to_line(range.end);
        if range.end > range.start && last > first && self.rope.line_to_char(last) == range.end {
            last -= 1;
        }
        let start = self.rope.line_to_char(first);
        let end = if last + 1 < self.rope.len_lines() {
            self.rope.line_to_char(last + 1)
        } else {
            self.rope.len_chars()
        };
        (start, end)
    }

    pub fn duplicate_lines(&mut self) {
        self.clear_extra();
        let (start, end) = self.line_span();
        let text = self.rope.slice(start..end).to_string();
        let insert = if text.ends_with('\n') {
            text
        } else {
            format!("\n{text}")
        };
        let sel = self.sel;
        self.checkpoint(EditKind::Other);
        self.apply(end..end, &insert);
        self.sel = sel;
        self.reveal = true;
    }

    pub fn delete_lines(&mut self) {
        self.clear_extra();
        let (mut start, end) = self.line_span();
        let text = self.rope.slice(start..end).to_string();
        if !text.ends_with('\n') && start > 0 {
            start -= 1;
        }
        self.replace(start..end, "", EditKind::Other);
        let line = self.rope.char_to_line(self.sel.1.min(self.len()));
        let at = self.rope.line_to_char(line);
        self.sel = (at, at);
    }

    pub fn move_each(&mut self, extend: bool, f: impl Fn(&Self, (usize, usize)) -> usize) {
        let cursors: Vec<(usize, usize)> = self
            .cursors()
            .into_iter()
            .map(|c| {
                let head = f(self, c).min(self.len());
                if extend { (c.0, head) } else { (head, head) }
            })
            .collect();
        self.block = None;
        self.set_cursors(cursors, 0);
        self.reveal = true;
    }

    pub fn texts(&self) -> Vec<String> {
        let mut cursors = self.cursors();
        cursors.sort_by_key(|(a, h)| *a.min(h));
        cursors
            .iter()
            .map(|(a, h)| self.rope.slice(*a.min(h)..*a.max(h)).to_string())
            .collect()
    }

    pub fn insert_each(&mut self, text: &str) {
        let count = self.extra.len() + 1;
        let normalized = text.strip_suffix('\n').unwrap_or(text);
        let parts: Vec<&str> = normalized.split('\n').collect();
        let mut order: Vec<usize> = (0..count).collect();
        let cursors = self.cursors();
        order.sort_by_key(|i| cursors[*i].0.min(cursors[*i].1));
        let mut rank = vec![0; count];
        for (r, i) in order.iter().enumerate() {
            rank[*i] = r;
        }
        let distribute = parts.len() == count && count > 1;
        let kind = if text.contains('\n') {
            EditKind::Other
        } else {
            EditKind::Insert
        };
        let cursors_rank: Vec<((usize, usize), usize)> =
            cursors.iter().copied().zip(rank).collect();
        self.edit_each(kind, |_, c| {
            let r = cursors_rank
                .iter()
                .find(|(cc, _)| *cc == c)
                .map_or(0, |(_, r)| *r);
            let range = c.0.min(c.1)..c.0.max(c.1);
            let piece = if distribute {
                parts[r].to_owned()
            } else {
                text.to_owned()
            };
            (range, piece)
        });
    }

    pub fn block_paste(&mut self, text: &str) {
        let head = self.sel.1;
        let line = self.rope.char_to_line(head);
        let col = head - self.rope.line_to_char(line);
        let lines: Vec<&str> = text
            .strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .collect();
        self.checkpoint(EditKind::Other);
        let mut carets = Vec::new();
        for (i, piece) in lines.iter().enumerate() {
            let target = line + i;
            if target >= self.line_count() {
                let end = self.len();
                self.apply(end..end, "\n");
            }
            let start = self.rope.line_to_char(target);
            let len = self.line_len(target);
            let pad = col.saturating_sub(len);
            let at = start + col.min(len);
            let insert = format!("{}{piece}", " ".repeat(pad));
            let caret = self.apply(at..at, &insert);
            carets.push((caret, caret));
        }
        self.set_cursors(carets, 0);
        self.reveal = true;
    }

    pub fn add_cursor(&mut self, pos: usize) {
        let pos = pos.min(self.len());
        let mut cursors = self.cursors();
        if let Some(i) = cursors.iter().position(|c| c.1 == pos && c.0 == pos)
            && cursors.len() > 1
        {
            cursors.remove(i);
            self.set_cursors(cursors, 0);
            return;
        }
        cursors.push((pos, pos));
        let primary = cursors.len() - 1;
        self.block = None;
        self.set_cursors(cursors, primary);
    }

    pub fn add_cursor_vertical(&mut self, down: bool) {
        let source = if down {
            self.cursors().into_iter().max_by_key(|c| c.1)
        } else {
            self.cursors().into_iter().min_by_key(|c| c.1)
        };
        let Some((_, head)) = source else {
            return;
        };
        let line = self.rope.char_to_line(head);
        let col = head - self.rope.line_to_char(line);
        let target = if down {
            if line + 1 >= self.line_count() {
                return;
            }
            line + 1
        } else {
            if line == 0 {
                return;
            }
            line - 1
        };
        let pos = self.rope.line_to_char(target) + col.min(self.line_len(target));
        let mut cursors = self.cursors();
        cursors.push((pos, pos));
        let primary = cursors.len() - 1;
        self.block = None;
        self.set_cursors(cursors, primary);
        self.reveal = true;
    }

    pub fn select_next_match(&mut self) {
        if !self.has_selection() {
            let head = self.sel.1;
            self.select_word_at(head);
            return;
        }
        let needle = self.selected_text();
        if needle.is_empty() {
            return;
        }
        let text = self.rope.to_string();
        let mut from = self
            .cursors()
            .iter()
            .map(|c| c.0.max(c.1))
            .max()
            .unwrap_or(0);
        let byte = |chars: usize| self.rope.char_to_byte(chars.min(self.len()));
        let mut found = None;
        for _ in 0..2 {
            if let Some(at) = text[byte(from)..].find(&needle) {
                let start = self.rope.byte_to_char(byte(from) + at);
                let end = start + needle.chars().count();
                if !self.cursors().iter().any(|c| c.0.min(c.1) == start) {
                    found = Some((start, end));
                }
                break;
            }
            from = 0;
        }
        if let Some(sel) = found {
            let mut cursors = self.cursors();
            cursors.push(sel);
            let primary = cursors.len() - 1;
            self.set_cursors(cursors, primary);
            self.reveal = true;
        }
    }

    pub fn move_to(&mut self, pos: usize, extend: bool) {
        self.extra.clear();
        self.block = None;
        let pos = pos.min(self.len());
        self.sel = if extend {
            (self.sel.0, pos)
        } else {
            (pos, pos)
        };
        self.last_edit = None;
        self.goal_x = None;
        self.reveal = true;
    }

    pub fn select_all(&mut self) {
        self.clear_extra();
        self.sel = (0, self.len());
        self.last_edit = None;
    }

    pub fn word_left(&self, pos: usize) -> usize {
        let mut i = pos;
        let mut chars = self.rope.chars_at(pos);
        let mut kind = None;
        while let Some(c) = chars.prev() {
            let k = class(c);
            match kind {
                None if k == 0 => {}
                None => kind = Some(k),
                Some(expected) if expected != k => break,
                Some(_) => {}
            }
            i -= 1;
        }
        i
    }

    pub fn word_right(&self, pos: usize) -> usize {
        let mut i = pos;
        let mut kind = None;
        for c in self.rope.chars_at(pos) {
            let k = class(c);
            match kind {
                None if k == 0 => {}
                None => kind = Some(k),
                Some(expected) if expected != k => break,
                Some(_) => {}
            }
            i += 1;
        }
        i
    }

    pub fn line_home(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        let start = self.rope.line_to_char(line);
        let indent = self
            .rope
            .line(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count();
        if pos == start + indent {
            start
        } else {
            start + indent
        }
    }

    pub fn line_end(&self, pos: usize) -> usize {
        let line = self.rope.char_to_line(pos);
        self.rope.line_to_char(line) + self.line_len(line)
    }

    pub fn select_word_at(&mut self, pos: usize) {
        let pos = pos.min(self.len());
        let at = |i: usize| (i < self.len()).then(|| self.rope.char(i));
        let kind = match at(pos) {
            Some(c) if c != '\n' => class(c),
            _ if pos > 0 && at(pos - 1) != Some('\n') => class(self.rope.char(pos - 1)),
            _ => {
                self.sel = (pos, pos);
                return;
            }
        };
        let same = |c: char| c != '\n' && class(c) == kind;
        let mut start = pos;
        let mut back = self.rope.chars_at(pos);
        while back.prev().is_some_and(same) {
            start -= 1;
        }
        let end = pos + self.rope.chars_at(pos).take_while(|c| same(*c)).count();
        self.sel = (start, end);
        self.last_edit = None;
    }

    pub fn select_line_at(&mut self, pos: usize) {
        let line = self.rope.char_to_line(pos.min(self.len()));
        self.select_lines(line..line + 1);
        self.last_edit = None;
    }

    pub fn bracket_pair(&self, head: usize) -> [Option<usize>; 2] {
        const OPEN: [char; 3] = ['(', '[', '{'];
        const CLOSE: [char; 3] = [')', ']', '}'];
        const REACH: usize = 20_000;
        let len = self.len();
        let at = [head, head.wrapping_sub(1)].into_iter().find(|i| {
            *i < len && {
                let c = self.rope.char(*i);
                OPEN.contains(&c) || CLOSE.contains(&c)
            }
        });
        let Some(at) = at else {
            return [None, None];
        };
        let c = self.rope.char(at);
        let mut depth = 0usize;
        if let Some(kind) = OPEN.iter().position(|o| *o == c) {
            for (step, ch) in self.rope.chars_at(at).enumerate().take(REACH) {
                if ch == OPEN[kind] {
                    depth += 1;
                } else if ch == CLOSE[kind] {
                    depth -= 1;
                    if depth == 0 {
                        return [Some(at), Some(at + step)];
                    }
                }
            }
        } else if let Some(kind) = CLOSE.iter().position(|o| *o == c) {
            let mut chars = self.rope.chars_at(at + 1);
            for step in 0..REACH.min(at + 1) {
                let Some(ch) = chars.prev() else {
                    break;
                };
                if ch == CLOSE[kind] {
                    depth += 1;
                } else if ch == OPEN[kind] {
                    depth -= 1;
                    if depth == 0 {
                        return [Some(at), Some(at - step)];
                    }
                }
            }
        }
        [Some(at), None]
    }

    pub fn goto_line(&mut self, line: usize) {
        let line = line.clamp(1, self.line_count()) - 1;
        let pos = self.rope.line_to_char(line);
        self.move_to(pos, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::WINDOWS_1251;

    #[test]
    fn bracket_pair_respects_nesting_in_both_directions() {
        let mut doc = Document::empty(1, 1);
        doc.insert_text("a(b[c]{d}(e))f");
        assert_eq!(doc.bracket_pair(1), [Some(1), Some(12)]);
        assert_eq!(doc.bracket_pair(13), [Some(12), Some(1)]);
        assert_eq!(doc.bracket_pair(3), [Some(3), Some(5)]);
        assert_eq!(doc.bracket_pair(0), [None, None]);
        let mut open = Document::empty(2, 2);
        open.insert_text("(x");
        assert_eq!(open.bracket_pair(0), [Some(0), None]);
    }

    fn text(doc: &Document) -> String {
        doc.rope.to_string()
    }

    #[test]
    fn utf8_roundtrip_keeps_crlf_and_bom() {
        let src = b"\xEF\xBB\xBFline1\r\nline2\r\n";
        let d = decode(src.to_vec(), None).unwrap();
        assert_eq!(d.text, "line1\nline2\n");
        assert_eq!(d.eol, Eol::Crlf);
        assert!(d.bom);
        assert_eq!(encode(&d.text, d.encoding, d.bom, d.eol), src);
    }

    #[test]
    fn cp1251_detected_and_roundtrips() {
        let original = "<?php\n// Привет, мир! Это проверка кодировки для старого шаблона сайта.\necho 'Каталог товаров';\n";
        let (bytes, _, _) = WINDOWS_1251.encode(original);
        let d = decode(bytes.to_vec(), None).unwrap();
        assert_eq!(d.encoding, WINDOWS_1251);
        assert_eq!(d.text, original);
        assert_eq!(encode(&d.text, d.encoding, d.bom, d.eol), bytes.as_ref());
    }

    #[test]
    fn utf16_roundtrip() {
        let src: Vec<u8> = [0xFFu8, 0xFE]
            .into_iter()
            .chain("aб\n".encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        let d = decode(src.clone(), None).unwrap();
        assert_eq!(d.text, "aб\n");
        assert_eq!(encode(&d.text, d.encoding, d.bom, d.eol), src);
    }

    #[test]
    fn binary_rejected() {
        assert!(decode(vec![0x7f, b'E', b'L', b'F', 0, 0, 1], None).is_err());
    }

    #[test]
    fn cursor_lines_and_longest_line() {
        let mut doc = Document::from_text("ab\nпривет\nx");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.max_line_chars, 6);
        doc.move_to(5, false);
        assert_eq!(doc.cursor_line_col(), (2, 3));
        doc.goto_line(3);
        assert_eq!(doc.sel, (10, 10));
        assert_eq!(doc.line_len(1), 6);
        assert!(!doc.dirty());
    }

    #[test]
    fn typing_undo_redo_and_dirty_state() {
        let mut doc = Document::from_text("abc");
        doc.move_to(3, false);
        doc.insert_text("d");
        doc.insert_text("e");
        assert_eq!(text(&doc), "abcde");
        assert!(doc.dirty());
        doc.undo();
        assert_eq!(text(&doc), "abc");
        assert!(!doc.dirty());
        doc.redo();
        assert_eq!(text(&doc), "abcde");
        assert_eq!(doc.sel, (5, 5));
        doc.move_to(0, false);
        doc.move_to(2, true);
        doc.insert_text("X");
        assert_eq!(text(&doc), "Xcde");
        doc.undo();
        assert_eq!(text(&doc), "abcde");
        assert_eq!(doc.selection(), 0..2);
    }

    #[test]
    fn newline_keeps_indent_and_backspace_joins() {
        let mut doc = Document::from_text("\tif (x) {");
        doc.move_to(doc.len(), false);
        doc.newline();
        doc.insert_text("y");
        assert_eq!(text(&doc), "\tif (x) {\n\ty");
        doc.backspace(false);
        doc.backspace(false);
        doc.backspace(false);
        assert_eq!(text(&doc), "\tif (x) {");
        doc.backspace(true);
        assert_eq!(text(&doc), "\tif (x) ");
        doc.move_to(1, false);
        doc.delete(true);
        assert_eq!(text(&doc), "\t (x) ");
    }

    #[test]
    fn indent_and_outdent_selected_lines() {
        let mut doc = Document::from_text("a\nb\n\nc\nd");
        doc.move_to(0, false);
        doc.move_to(6, true);
        doc.indent();
        assert_eq!(text(&doc), "\ta\n\tb\n\n\tc\nd");
        doc.outdent();
        assert_eq!(text(&doc), "a\nb\n\nc\nd");
        let mut spaces = Document::from_text("      x");
        spaces.move_to(6, false);
        spaces.outdent();
        assert_eq!(text(&spaces), "  x");
        assert_eq!(spaces.sel, (2, 2));
    }

    #[test]
    fn word_motion_and_selection() {
        let mut doc = Document::from_text("foo_bar = $baz->qux;\nnext");
        assert_eq!(doc.word_right(0), 7);
        assert_eq!(doc.word_right(7), 9);
        assert_eq!(doc.word_left(14), 11);
        assert_eq!(doc.word_left(10), 8);
        doc.select_word_at(12);
        assert_eq!(doc.selected_text(), "baz");
        doc.select_word_at(20);
        assert_eq!(doc.selected_text(), ";");
        doc.select_line_at(3);
        assert_eq!(doc.selected_text(), "foo_bar = $baz->qux;\n");
        assert_eq!(doc.line_home(5), 0);
        assert_eq!(doc.line_end(5), 20);
    }

    #[test]
    fn multiple_cursors_type_delete_and_paste() {
        let mut doc = Document::from_text("ab\ncd\nef");
        doc.set_cursors(vec![(1, 1), (4, 4), (7, 7)], 0);
        doc.insert_text("X");
        assert_eq!(text(&doc), "aXb\ncXd\neXf");
        assert_eq!(doc.cursors().len(), 3);
        doc.backspace(false);
        assert_eq!(text(&doc), "ab\ncd\nef");
        doc.insert_text("1\n2\n3");
        assert_eq!(text(&doc), "a1b\nc2d\ne3f");
        doc.undo();
        assert_eq!(text(&doc), "ab\ncd\nef");
        assert!(!doc.multi());
        doc.set_cursors(vec![(0, 2), (3, 5)], 1);
        assert_eq!(doc.texts(), vec!["ab", "cd"]);
        doc.insert_text("Z");
        assert_eq!(text(&doc), "Z\nZ\nef");
        doc.move_each(false, |_, c| c.1 - 1);
        doc.delete(false);
        assert_eq!(text(&doc), "\n\nef");
    }

    #[test]
    fn cursors_merge_and_vertical_add() {
        let mut doc = Document::from_text("hello\nhi\nworld");
        doc.set_cursors(vec![(0, 3), (2, 5), (8, 8)], 0);
        assert_eq!(doc.cursors(), vec![(0, 5), (8, 8)]);
        doc.move_to(4, false);
        doc.add_cursor_vertical(true);
        doc.add_cursor_vertical(true);
        assert_eq!(doc.cursors().len(), 3);
        let mut heads: Vec<usize> = doc.cursors().iter().map(|c| c.1).collect();
        heads.sort();
        assert_eq!(heads, vec![4, 8, 11]);
        doc.add_cursor(8);
        assert_eq!(doc.cursors().len(), 2);
        let mut words = Document::from_text("foo bar foo baz foo");
        words.move_to(1, false);
        words.select_next_match();
        assert_eq!(words.selected_text(), "foo");
        words.select_next_match();
        words.select_next_match();
        assert_eq!(words.cursors().len(), 3);
        words.insert_text("x");
        assert_eq!(text(&words), "x bar x baz x");
    }

    #[test]
    fn block_paste_pads_short_lines() {
        let mut doc = Document::from_text("abcd\nx\n");
        doc.move_to(2, false);
        doc.block_paste("12\n34\n56");
        assert_eq!(text(&doc), "ab12cd\nx 34\n  56");
    }

    #[test]
    fn case_and_line_tools() {
        let mut doc = Document::from_text("one two\nthree\n");
        doc.sel = (0, 3);
        doc.transform(|t| t.to_uppercase());
        assert_eq!(doc.rope.to_string(), "ONE two\nthree\n");
        assert_eq!(doc.sel, (0, 3));
        doc.sel = (5, 5);
        doc.transform(|t| t.to_uppercase());
        assert_eq!(doc.rope.to_string(), "ONE TWO\nthree\n");
        doc.sel = (9, 9);
        doc.duplicate_lines();
        assert_eq!(doc.rope.to_string(), "ONE TWO\nthree\nthree\n");
        doc.sel = (0, 0);
        doc.delete_lines();
        assert_eq!(doc.rope.to_string(), "three\nthree\n");
        let mut tail = Document::from_text("a\nb");
        tail.sel = (3, 3);
        tail.duplicate_lines();
        assert_eq!(tail.rope.to_string(), "a\nb\nb");
        tail.delete_lines();
        assert_eq!(tail.rope.to_string(), "a\nb");
    }

    #[test]
    fn toggle_comment_lines_and_blocks() {
        let mut doc = Document::from_text("    $a = 1;\n\n        $b = 2;\nend");
        doc.syntax = "PHP".into();
        doc.move_to(0, false);
        doc.move_to(26, true);
        doc.toggle_comment();
        assert_eq!(text(&doc), "    // $a = 1;\n\n    //     $b = 2;\nend");
        doc.toggle_comment();
        assert_eq!(text(&doc), "    $a = 1;\n\n        $b = 2;\nend");
        doc.undo();
        doc.undo();
        assert_eq!(text(&doc), "    $a = 1;\n\n        $b = 2;\nend");

        let mut css = Document::from_text("a { color: red; }");
        css.syntax = "CSS".into();
        css.move_to(4, false);
        css.toggle_comment();
        assert_eq!(text(&css), "/* a { color: red; } */");
        assert_eq!(css.sel, (7, 7));
        css.toggle_comment();
        assert_eq!(text(&css), "a { color: red; }");
        assert_eq!(css.sel, (4, 4));

        let mut py = Document::from_text("x = 1");
        py.syntax = "Python".into();
        py.toggle_comment();
        assert_eq!(text(&py), "# x = 1");
        let mut twig = Document::from_text("{{ a }}");
        twig.syntax = "HTML (Twig)".into();
        twig.toggle_comment();
        assert_eq!(text(&twig), "{# {{ a }} #}");
    }

    #[test]
    #[ignore]
    fn perf_open_phases() {
        let path = std::path::Path::new("/tmp/demo/dump.sql");
        for round in 0..3 {
            let t = std::time::Instant::now();
            let bytes = std::fs::read(path).unwrap();
            let read = t.elapsed();
            let t = std::time::Instant::now();
            let decoded = decode(bytes, None).unwrap();
            let dec = t.elapsed();
            let t = std::time::Instant::now();
            let rope = Rope::from_str(&decoded.text);
            let build = t.elapsed();
            let t = std::time::Instant::now();
            let longest = longest_line(&decoded.text);
            let scan = t.elapsed();
            let slow = decoded
                .text
                .split('\n')
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0);
            assert_eq!(longest, slow);
            eprintln!(
                "#{round} read {:.0} decode {:.0} rope {:.0} longest {:.0} мс ({} строк, {longest})",
                read.as_secs_f64() * 1e3,
                dec.as_secs_f64() * 1e3,
                build.as_secs_f64() * 1e3,
                scan.as_secs_f64() * 1e3,
                rope.len_lines()
            );
        }
    }
}
