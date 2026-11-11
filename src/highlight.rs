use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui::Color32;
use ropey::Rope;
use syntect::highlighting::{HighlightIterator, HighlightState, Highlighter as Styler, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};
use two_face::theme::EmbeddedThemeName;

pub const FRAME_BUDGET: Duration = Duration::from_millis(8);
const FULL_LIMIT: usize = 2 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 10_000;
const CHECKPOINT: usize = 16;
const WINDOW_WARMUP: usize = 200;
const WINDOW_MARGIN: usize = 250;
const WINDOW_SLACK: usize = 60;

pub struct Syntaxes {
    pub set: SyntaxSet,
    dark: Theme,
    light: Theme,
    pub names: Vec<String>,
}

impl Syntaxes {
    pub fn new() -> Self {
        let set = two_face::syntax::extra_newlines();
        let themes = two_face::theme::extra();
        let mut names: Vec<String> = set
            .syntaxes()
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| s.name.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        Self {
            set,
            dark: themes.get(EmbeddedThemeName::CatppuccinMocha).clone(),
            light: themes.get(EmbeddedThemeName::CatppuccinLatte).clone(),
            names,
        }
    }

    fn theme(&self, dark: bool) -> &Theme {
        if dark { &self.dark } else { &self.light }
    }

    pub fn detect(&self, path: Option<&Path>, text: &str) -> String {
        let by_path = path.and_then(|p| {
            let name = p.file_name()?.to_str()?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            let lower = ext.to_lowercase();
            let alias = match lower.as_str() {
                "twig" => Some("HTML (Twig)"),
                "tpl" | "htm" | "xhtml" => Some("HTML"),
                "phtml" | "php5" | "php7" | "php8" | "inc" | "module" => Some("PHP"),
                "mjs" | "cjs" => Some("JavaScript (Babel)"),
                _ => None,
            };
            alias
                .and_then(|n| self.set.find_syntax_by_name(n))
                .or_else(|| self.set.find_syntax_by_extension(name))
                .or_else(|| self.set.find_syntax_by_extension(ext))
                .or_else(|| self.set.find_syntax_by_extension(&lower))
        });
        by_path
            .or_else(|| {
                self.set
                    .find_syntax_by_first_line(text.lines().next().unwrap_or(""))
            })
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Plain Text".into())
    }

    fn syntax(&self, name: &str) -> &SyntaxReference {
        self.set
            .find_syntax_by_name(name)
            .unwrap_or_else(|| self.set.find_syntax_plain_text())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Span {
    pub len: u32,
    pub color: Color32,
}

type State = (ParseState, HighlightState);

#[derive(Default)]
struct Entry {
    spans: Vec<Span>,
    state: Option<Box<State>>,
}

struct Window {
    version: u64,
    start: usize,
    entries: Vec<Vec<Span>>,
}

struct Builder {
    version: u64,
    start: usize,
    end: usize,
    state: State,
    entries: Vec<Vec<Span>>,
}

#[derive(Default)]
pub struct Highlighter {
    style: Option<(String, bool, bool)>,
    entries: Vec<Entry>,
    dirty: Option<(usize, usize)>,
    shown: Option<Window>,
    building: Option<Builder>,
    buf: String,
    pub default_color: Color32,
}

fn fresh_state(syntax: &SyntaxReference, styler: &Styler<'_>) -> State {
    (
        ParseState::new(syntax),
        HighlightState::new(styler, ScopeStack::new()),
    )
}

fn highlight_line(
    rope: &Rope,
    line: usize,
    state: &mut State,
    set: &SyntaxSet,
    styler: &Styler<'_>,
    buf: &mut String,
) -> Vec<Span> {
    let slice = rope.line(line);
    if slice.len_bytes() > MAX_LINE_BYTES {
        return Vec::new();
    }
    buf.clear();
    for chunk in slice.chunks() {
        buf.push_str(chunk);
    }
    let Ok(ops) = state.0.parse_line(buf, set) else {
        return Vec::new();
    };
    let mut spans: Vec<Span> = Vec::new();
    for (style, piece) in HighlightIterator::new(&mut state.1, &ops, buf, styler) {
        if piece.is_empty() {
            continue;
        }
        let c = style.foreground;
        let color = Color32::from_rgb(c.r, c.g, c.b);
        let len = piece.len() as u32;
        match spans.last_mut() {
            Some(last) if last.color == color => last.len += len,
            _ => spans.push(Span { len, color }),
        }
    }
    spans
}

impl Highlighter {
    pub fn reset(&mut self) {
        self.style = None;
    }

    pub fn edit(&mut self, start_line: usize, old_lines: usize, new_lines: usize) {
        if self.entries.is_empty() {
            return;
        }
        if start_line + old_lines > self.entries.len() || old_lines == 0 || new_lines == 0 {
            self.reset();
            return;
        }
        let state = self.entries[start_line].state.take();
        self.entries.splice(
            start_line..start_line + old_lines,
            (0..new_lines).map(|_| Entry::default()),
        );
        self.entries[start_line].state = state;
        let last = start_line + new_lines - 1;
        self.dirty = Some(match self.dirty {
            None => (start_line, last),
            Some((from, to)) => {
                let to = if to >= start_line + old_lines {
                    to + new_lines - old_lines
                } else {
                    to
                };
                (from.min(start_line), to.max(last))
            }
        });
    }

    pub fn replace_all(&mut self, old: &Rope, new: &Rope) {
        if self.entries.is_empty() {
            return;
        }
        let a = String::from(old);
        let b = String::from(new);
        let (a, b) = (a.as_bytes(), b.as_bytes());
        let limit = a.len().min(b.len());
        let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        let suffix = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take(limit - prefix)
            .take_while(|(x, y)| x == y)
            .count();
        let start = old.byte_to_line(prefix);
        let old_end = old.byte_to_line(a.len() - suffix);
        let new_end = new.byte_to_line(b.len() - suffix);
        self.edit(start, old_end - start + 1, new_end.max(start) - start + 1);
    }

    pub fn spans(&self, line: usize) -> &[Span] {
        if let Some(entry) = self.entries.get(line) {
            return &entry.spans;
        }
        match &self.shown {
            Some(w) if line >= w.start => w
                .entries
                .get(line - w.start)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            _ => &[],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        syn: &Syntaxes,
        rope: &Rope,
        syntax: &str,
        dark: bool,
        version: u64,
        visible: Range<usize>,
        budget: Option<Duration>,
    ) -> bool {
        let theme = syn.theme(dark);
        let c = theme
            .settings
            .foreground
            .unwrap_or(syntect::highlighting::Color::WHITE);
        self.default_color = Color32::from_rgb(c.r, c.g, c.b);
        let full = rope.len_bytes() <= FULL_LIMIT;
        let lines = rope.len_lines();
        let stale = !matches!(&self.style, Some((s, d, f)) if s == syntax && *d == dark && *f == full)
            || (full && syntax != "Plain Text" && self.entries.len() != lines);
        if stale {
            self.style = Some((syntax.to_owned(), dark, full));
            self.entries.clear();
            self.dirty = None;
            self.shown = None;
            self.building = None;
            if full && syntax != "Plain Text" {
                self.entries.resize_with(lines, Entry::default);
                self.dirty = Some((0, lines - 1));
            }
        }
        if syntax == "Plain Text" {
            return false;
        }
        let styler = Styler::new(theme);
        let deadline = budget.map(|b| Instant::now() + b);
        if full {
            self.process_full(syn, rope, syntax, &styler, deadline)
        } else {
            self.process_window(syn, rope, syntax, &styler, version, visible, deadline)
        }
    }

    fn process_full(
        &mut self,
        syn: &Syntaxes,
        rope: &Rope,
        syntax: &str,
        styler: &Styler<'_>,
        deadline: Option<Instant>,
    ) -> bool {
        let Some((from, mut to)) = self.dirty else {
            return false;
        };
        let n = self.entries.len();
        let mut i = from.min(n - 1);
        while i > 0 && self.entries[i].state.is_none() {
            i -= 1;
        }
        let first = i;
        let mut state = match self.entries[i].state.as_deref() {
            Some(state) => state.clone(),
            None => fresh_state(syn.syntax(syntax), styler),
        };
        loop {
            if i >= n {
                self.dirty = None;
                return false;
            }
            let checkpoint = self.entries[i].state.is_some() || i % CHECKPOINT == 0;
            if checkpoint {
                if i > to && self.entries[i].state.as_deref() == Some(&state) {
                    self.dirty = None;
                    return false;
                }
                self.entries[i].state = Some(Box::new(state.clone()));
            }
            if i > first + CHECKPOINT && deadline.is_some_and(|d| Instant::now() > d) {
                if !checkpoint {
                    self.entries[i].state = Some(Box::new(state.clone()));
                }
                to = to.max(i);
                self.dirty = Some((i, to));
                return true;
            }
            self.entries[i].spans =
                highlight_line(rope, i, &mut state, &syn.set, styler, &mut self.buf);
            i += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_window(
        &mut self,
        syn: &Syntaxes,
        rope: &Rope,
        syntax: &str,
        styler: &Styler<'_>,
        version: u64,
        visible: Range<usize>,
        deadline: Option<Instant>,
    ) -> bool {
        let lines = rope.len_lines();
        let want =
            visible.start.saturating_sub(WINDOW_SLACK)..(visible.end + WINDOW_SLACK).min(lines);
        let covers = |start: usize, end: usize| start <= want.start && end >= want.end;
        if let Some(w) = &self.shown
            && w.version == version
            && covers(w.start, w.start + w.entries.len())
        {
            self.building = None;
            return false;
        }
        let reusable = self
            .building
            .as_ref()
            .is_some_and(|b| b.version == version && covers(b.start, b.end));
        if !reusable {
            let name = if syntax == "PHP" {
                "PHP Source"
            } else {
                syntax
            };
            self.building = Some(Builder {
                version,
                start: visible.start.saturating_sub(WINDOW_WARMUP),
                end: (visible.end + WINDOW_MARGIN).min(lines),
                state: fresh_state(syn.syntax(name), styler),
                entries: Vec::new(),
            });
        }
        let Some(b) = &mut self.building else {
            return false;
        };
        let mut done = 0;
        loop {
            let line = b.start + b.entries.len();
            if line >= b.end {
                break;
            }
            if done > CHECKPOINT && deadline.is_some_and(|d| Instant::now() > d) {
                return true;
            }
            b.entries.push(highlight_line(
                rope,
                line,
                &mut b.state,
                &syn.set,
                styler,
                &mut self.buf,
            ));
            done += 1;
        }
        let b = self.building.take().unwrap();
        self.shown = Some(Window {
            version: b.version,
            start: b.start,
            entries: b.entries,
        });
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(h: &mut Highlighter, syn: &Syntaxes, rope: &Rope, syntax: &str) {
        h.process(syn, rope, syntax, true, 0, 0..50, None);
    }

    fn all(h: &Highlighter, rope: &Rope) -> Vec<Vec<Span>> {
        (0..rope.len_lines()).map(|l| h.spans(l).to_vec()).collect()
    }

    fn reference(syn: &Syntaxes, rope: &Rope, syntax: &str) -> Vec<Vec<Span>> {
        let mut h = Highlighter::default();
        run(&mut h, syn, rope, syntax);
        all(&h, rope)
    }

    #[test]
    fn required_languages_present() {
        let syn = Syntaxes::new();
        for (file, expected) in [
            ("a.php", "PHP"),
            ("a.html", "HTML"),
            ("a.js", "JavaScript (Babel)"),
            ("a.twig", "HTML (Twig)"),
            ("a.css", "CSS"),
            ("a.py", "Python"),
            ("a.rs", "Rust"),
            ("a.ts", "TypeScript"),
            ("a.json", "JSON"),
            ("a.scss", "SCSS"),
            ("a.sql", "SQL"),
            ("a.toml", "TOML"),
            ("a.yml", "YAML"),
            ("a.xml", "XML"),
            ("a.md", "Markdown"),
        ] {
            assert_eq!(syn.detect(Some(Path::new(file)), ""), expected, "{file}");
        }
        assert_eq!(syn.detect(None, "#!/usr/bin/env python3"), "Python");
        assert!(syn.set.find_syntax_by_name("PHP Source").is_some());
    }

    #[test]
    fn spans_cover_each_line() {
        let syn = Syntaxes::new();
        let rope = Rope::from_str(
            "<?php\n$a = 'привет';\nfunction f() { return 1; }\n?>\n<div class=\"x\">ok</div>",
        );
        let lines = reference(&syn, &rope, "PHP");
        assert_eq!(lines.len(), 5);
        for (i, spans) in lines.iter().enumerate() {
            let total: usize = spans.iter().map(|s| s.len as usize).sum();
            assert_eq!(total, rope.line(i).len_bytes());
        }
        assert!(lines[1].len() > 2);
    }

    fn find(doc: &crate::document::Document, needle: &str) -> usize {
        let text = doc.rope.to_string();
        doc.rope.byte_to_char(text.find(needle).unwrap())
    }

    #[test]
    fn edits_match_full_rehighlight() {
        let syn = Syntaxes::new();
        let mut base = String::new();
        for i in 0..200 {
            base.push_str(&format!("fn f{i}() -> u32 {{ let s = \"v{i}\"; {i} }}\n"));
        }
        let mut doc = crate::document::Document::from_text(&base);
        run(&mut doc.hl, &syn, &doc.rope, "Rust");
        type Edit = Box<dyn Fn(&mut crate::document::Document)>;
        let edits: Vec<Edit> = vec![
            Box::new(|d| {
                let at = find(d, "fn f100()");
                d.replace(at..at, "/* ", crate::document::EditKind::Other)
            }),
            Box::new(|d| {
                let at = find(d, "fn f150()");
                d.replace(at..at, "*/ ", crate::document::EditKind::Other)
            }),
            Box::new(|d| {
                let at = find(d, "fn f20()");
                let end = find(d, "fn f23()");
                d.replace(at..end, "", crate::document::EditKind::Other)
            }),
            Box::new(|d| {
                let at = find(d, "fn f60()");
                d.replace(at..at, "\n\n// note\n", crate::document::EditKind::Other)
            }),
            Box::new(|d| d.undo()),
            Box::new(|d| d.undo()),
            Box::new(|d| d.redo()),
            Box::new(|d| {
                let end = d.len();
                d.replace(end..end, "// tail", crate::document::EditKind::Other)
            }),
            Box::new(|d| {
                let end = d.len();
                d.replace(end / 2..end, "", crate::document::EditKind::Other)
            }),
            Box::new(|d| {
                let end = d.len();
                d.replace(0..end, "", crate::document::EditKind::Other)
            }),
            Box::new(|d| d.replace(0..0, "let x = 1;", crate::document::EditKind::Other)),
        ];
        for (step, edit) in edits.iter().enumerate() {
            edit(&mut doc);
            run(&mut doc.hl, &syn, &doc.rope, "Rust");
            assert_eq!(
                all(&doc.hl, &doc.rope),
                reference(&syn, &doc.rope, "Rust"),
                "step {step}"
            );
        }
    }

    #[test]
    fn budgeted_passes_converge_with_edits_in_between() {
        let syn = Syntaxes::new();
        let mut text = String::from("<?php\n");
        for i in 0..300 {
            text.push_str(&format!("/* c{i} */ $a{i} = ['k' => \"v{i}\"];\n"));
        }
        let mut doc = crate::document::Document::from_text(&text);
        let mut passes = 0;
        while doc
            .hl
            .process(&syn, &doc.rope, "PHP", true, 0, 0..50, Some(Duration::ZERO))
        {
            passes += 1;
            if passes == 3 {
                doc.replace(8..8, "x\ny", crate::document::EditKind::Other);
            }
            assert!(passes < 1000);
        }
        assert!(passes > 5);
        assert_eq!(all(&doc.hl, &doc.rope), reference(&syn, &doc.rope, "PHP"));
    }

    #[test]
    fn window_mode_highlights_only_near_viewport() {
        let syn = Syntaxes::new();
        let mut text = String::new();
        let mut i = 0;
        while text.len() <= FULL_LIMIT {
            text.push_str(&format!(
                "INSERT INTO t (id, name) VALUES ({i}, 'row {i}');\n"
            ));
            i += 1;
        }
        let rope = Rope::from_str(&text);
        let mut h = Highlighter::default();
        h.process(&syn, &rope, "SQL", true, 1, 20_000..20_050, None);
        assert!(h.spans(20_010).len() > 3);
        assert!(h.spans(100).is_empty());
        assert!(h.entries.is_empty());
        assert!(!h.process(&syn, &rope, "SQL", true, 1, 20_020..20_070, None));
        assert!(h.shown.as_ref().unwrap().start == 20_000 - WINDOW_WARMUP);
        h.process(&syn, &rope, "SQL", true, 2, 0..50, None);
        assert!(h.spans(10).len() > 3);
    }

    #[test]
    #[ignore]
    fn perf_large_file() {
        let syn = Syntaxes::new();
        let mut text = String::from("<?php\n");
        let mut i = 0;
        while text.len() < 1024 * 1024 {
            text.push_str(&format!(
                "function handler{i}(array $items, int $limit = 20): array {{\n    $result = [];\n    foreach ($items as $key => $item) {{\n        if ($key > $limit) {{ break; }}\n        $result[] = htmlspecialchars($item['NAME']) . ' #{i}';\n    }}\n    return $result;\n}}\n\n"
            ));
            i += 1;
        }
        let mut doc = crate::document::Document::from_text(&text);
        let t = Instant::now();
        run(&mut doc.hl, &syn, &doc.rope, "PHP");
        let full = t.elapsed();
        let middle = doc.len() / 2;
        let t = Instant::now();
        doc.replace(middle..middle, "$x = 1; ", crate::document::EditKind::Other);
        run(&mut doc.hl, &syn, &doc.rope, "PHP");
        let edit = t.elapsed();
        println!("lines={} full={full:?} edit={edit:?}", doc.line_count());
    }

    #[test]
    fn random_edits_undo_redo_never_panic_and_stay_consistent() {
        let syn = Syntaxes::new();
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };
        let pieces = [
            "x",
            "\n",
            "<?php\n$a = 1;\n",
            "/* ",
            " */",
            "'str",
            "\n\n\n",
            "function f() {\n\treturn 1;\n}\n",
            "привет",
            "<div class=\"a\">\n</div>",
            "?>\n",
            "\t",
        ];
        for round in 0..40 {
            let mut doc = crate::document::Document::from_text("<?php\necho 1;\n");
            doc.syntax = "PHP".into();
            for step in 0..120 {
                match next(10) {
                    0..=3 => {
                        let at = next(doc.len() + 1);
                        doc.move_to(at, false);
                        doc.insert_text(pieces[next(pieces.len())]);
                    }
                    4 => {
                        let a = next(doc.len() + 1);
                        let b = next(doc.len() + 1);
                        doc.move_to(a, false);
                        doc.move_to(b, true);
                        doc.insert_text(pieces[next(pieces.len())]);
                    }
                    5 => doc.backspace(next(2) == 0),
                    6 => doc.delete(next(2) == 0),
                    7 | 8 => doc.undo(),
                    _ => doc.redo(),
                }
                assert!(
                    doc.sel.0 <= doc.len() && doc.sel.1 <= doc.len(),
                    "{round}/{step}"
                );
                let budget = (next(3) == 0).then_some(Duration::ZERO);
                if next(4) != 0 {
                    doc.hl
                        .process(&syn, &doc.rope, "PHP", true, doc.version, 0..50, budget);
                }
            }
            run(&mut doc.hl, &syn, &doc.rope, "PHP");
            assert_eq!(
                all(&doc.hl, &doc.rope),
                reference(&syn, &doc.rope, "PHP"),
                "round {round}"
            );
        }
    }
}
