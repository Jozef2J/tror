use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use regex::{Captures, Regex, RegexBuilder};
use ropey::Rope;

use crate::document::{Document, EditKind};

pub const MAX_MATCHES: usize = 100_000;
const SYNC_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy, PartialEq)]
pub enum Pending {
    FromAnchor,
    Next,
    Prev,
    Replace,
    ReplaceAll,
}

#[derive(Clone, PartialEq)]
struct Key {
    doc: u64,
    version: u64,
    query: String,
    case: bool,
    word: bool,
    regex: bool,
}

struct Found {
    matches: Vec<(usize, usize)>,
    truncated: bool,
}

struct Job {
    rx: Receiver<Found>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub replace_mode: bool,
    pub query: String,
    pub replacement: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    pub focus_query: bool,
    pub pending: Option<Pending>,
    pub matches: Vec<(usize, usize)>,
    pub truncated: bool,
    pub error: Option<String>,
    pub last_replaced: Option<usize>,
    re: Option<Regex>,
    key: Option<Key>,
    job: Option<Job>,
}

fn find_all(rope: &Rope, re: &Regex, cancel: &AtomicBool) -> Option<Found> {
    let text = String::from(rope);
    let mut matches = Vec::new();
    let mut truncated = false;
    for (n, m) in re.find_iter(&text).enumerate() {
        if n % 512 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        if m.is_empty() {
            continue;
        }
        if matches.len() >= MAX_MATCHES {
            truncated = true;
            break;
        }
        matches.push((rope.byte_to_char(m.start()), rope.byte_to_char(m.end())));
    }
    Some(Found { matches, truncated })
}

impl SearchState {
    pub fn searching(&self) -> bool {
        self.job.is_some()
    }

    fn build(&self) -> Result<Regex, String> {
        let mut pattern = if self.regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        if self.word {
            pattern = format!(r"\b(?:{pattern})\b");
        }
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case)
            .multi_line(true)
            .build()
            .map_err(|_| "некорректное регулярное выражение".to_owned())
    }

    pub fn update(&mut self, doc: &Document, ctx: Option<&egui::Context>) {
        if let Some(job) = &self.job
            && let Ok(found) = job.rx.try_recv()
        {
            self.matches = found.matches;
            self.truncated = found.truncated;
            self.job = None;
        }
        let key = Key {
            doc: doc.id,
            version: doc.version,
            query: self.query.clone(),
            case: self.case,
            word: self.word,
            regex: self.regex,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.key = Some(key);
        self.matches.clear();
        self.truncated = false;
        self.error = None;
        self.re = None;
        if self.query.is_empty() {
            return;
        }
        let re = match self.build() {
            Ok(re) => re,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.re = Some(re.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        match ctx {
            Some(ctx) if doc.rope.len_bytes() > SYNC_LIMIT => {
                let (tx, rx) = channel();
                let rope = doc.rope.clone();
                let flag = cancel.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    if let Some(found) = find_all(&rope, &re, &flag) {
                        let _ = tx.send(found);
                        ctx.request_repaint();
                    }
                });
                self.job = Some(Job { rx, cancel });
            }
            _ => {
                if let Some(found) = find_all(&doc.rope, &re, &cancel) {
                    self.matches = found.matches;
                    self.truncated = found.truncated;
                }
            }
        }
    }

    pub fn current(&self, selection: (usize, usize)) -> Option<usize> {
        let i = self.matches.partition_point(|m| m.0 < selection.0);
        (self.matches.get(i)? == &selection).then_some(i)
    }

    pub fn apply_pending(&mut self, doc: &mut Document, ctx: Option<&egui::Context>) {
        let Some(pending) = self.pending else {
            return;
        };
        self.update(doc, ctx);
        match pending {
            Pending::Replace => {
                self.pending = Some(Pending::Next);
                let range = doc.selection();
                if let Some(re) = &self.re
                    && !range.is_empty()
                {
                    let selected = doc.selected_text();
                    let whole = re
                        .find(&selected)
                        .is_some_and(|m| m.start() == 0 && m.end() == selected.len());
                    if whole {
                        let replaced = if self.regex {
                            re.replace(&selected, self.replacement.as_str())
                                .into_owned()
                        } else {
                            self.replacement.clone()
                        };
                        doc.replace(range, &replaced, EditKind::Other);
                    }
                }
                self.apply_pending(doc, ctx);
            }
            Pending::ReplaceAll => {
                self.pending = None;
                if let Some(re) = &self.re {
                    let text = String::from(&doc.rope);
                    let mut count = 0;
                    let replacement = self.replacement.as_str();
                    let regex_mode = self.regex;
                    let replaced = re.replace_all(&text, |caps: &Captures<'_>| {
                        count += 1;
                        let mut out = String::new();
                        if regex_mode {
                            caps.expand(replacement, &mut out);
                        } else {
                            out.push_str(replacement);
                        }
                        out
                    });
                    if count > 0 {
                        doc.set_text(&replaced);
                    }
                    self.last_replaced = Some(count);
                    self.update(doc, ctx);
                }
            }
            Pending::FromAnchor | Pending::Next | Pending::Prev => {
                if self.searching() {
                    return;
                }
                self.pending = None;
                if self.matches.is_empty() {
                    return;
                }
                let range = doc.selection();
                let index = match pending {
                    Pending::Prev => {
                        let i = self.matches.partition_point(|m| m.1 <= range.start);
                        if i == 0 {
                            self.matches.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    _ => {
                        let anchor = if pending == Pending::Next {
                            range.end
                        } else {
                            range.start
                        };
                        let i = self.matches.partition_point(|m| m.0 < anchor);
                        if i == self.matches.len() { 0 } else { i }
                    }
                };
                let (start, end) = self.matches[index];
                doc.sel = (start, end);
                doc.goal_x = None;
                doc.reveal = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(s: &mut SearchState, d: &mut Document, pending: Pending) {
        s.pending = Some(pending);
        s.apply_pending(d, None);
    }

    #[test]
    fn finds_in_char_indices_and_wraps() {
        let mut d = Document::from_text("тест Тест\nтест");
        let mut s = SearchState {
            query: "тест".into(),
            ..Default::default()
        };
        s.update(&d, None);
        assert_eq!(s.matches, vec![(0, 4), (5, 9), (10, 14)]);
        s.case = true;
        s.update(&d, None);
        assert_eq!(s.matches, vec![(0, 4), (10, 14)]);

        step(&mut s, &mut d, Pending::FromAnchor);
        assert_eq!(d.sel, (0, 4));
        assert_eq!(s.current(d.sel), Some(0));
        step(&mut s, &mut d, Pending::Next);
        assert_eq!(d.sel, (10, 14));
        step(&mut s, &mut d, Pending::Next);
        assert_eq!(d.sel, (0, 4));
        step(&mut s, &mut d, Pending::Prev);
        assert_eq!(d.sel, (10, 14));
    }

    #[test]
    fn whole_word_and_invalid_regex() {
        let d = Document::from_text("cat concat cat_ cat");
        let mut s = SearchState {
            query: "cat".into(),
            word: true,
            ..Default::default()
        };
        s.update(&d, None);
        assert_eq!(s.matches, vec![(0, 3), (16, 19)]);
        s.regex = true;
        s.query = "(".into();
        s.update(&d, None);
        assert!(s.error.is_some());
        assert!(s.matches.is_empty());
    }

    #[test]
    fn replace_one_then_all_with_groups_and_undo() {
        let mut d = Document::from_text("width: 10px; height: 20px; top: 5px");
        let mut s = SearchState {
            query: r"(\d+)px".into(),
            replacement: "${1}rem".into(),
            regex: true,
            ..Default::default()
        };
        step(&mut s, &mut d, Pending::FromAnchor);
        step(&mut s, &mut d, Pending::Replace);
        assert_eq!(d.rope.to_string(), "width: 10rem; height: 20px; top: 5px");
        assert_eq!(d.sel, (22, 26));
        step(&mut s, &mut d, Pending::ReplaceAll);
        assert_eq!(d.rope.to_string(), "width: 10rem; height: 20rem; top: 5rem");
        assert_eq!(s.last_replaced, Some(2));
        assert!(d.dirty());
        d.undo();
        assert_eq!(d.rope.to_string(), "width: 10rem; height: 20px; top: 5px");
    }

    #[test]
    fn plain_replacement_is_literal() {
        let mut d = Document::from_text("a.b a.b");
        let mut s = SearchState {
            query: "a.b".into(),
            replacement: "$1".into(),
            ..Default::default()
        };
        step(&mut s, &mut d, Pending::ReplaceAll);
        assert_eq!(d.rope.to_string(), "$1 $1");
    }

    #[test]
    fn background_search_delivers_results() {
        let line = "SELECT * FROM orders WHERE status = 'new';\n";
        let d = Document::from_text(&line.repeat(SYNC_LIMIT / line.len() + 10));
        let mut s = SearchState {
            query: "orders".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        s.update(&d, Some(&ctx));
        assert!(s.searching());
        for _ in 0..500 {
            s.update(&d, Some(&ctx));
            if !s.searching() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(s.matches.len(), d.line_count() - 1);
        assert_eq!(s.matches[1], (line.len() + 14, line.len() + 20));
    }
}
