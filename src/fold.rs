use crate::document::Document;

const OPEN: [char; 3] = ['{', '[', '('];
const CLOSE: [char; 3] = ['}', ']', ')'];
const LINE_LIMIT: usize = 4000;
const REACH: usize = 4_000_000;
const SCAN: usize = 20_000_000;

fn indent_of(text: &str) -> Option<usize> {
    let mut width = 0;
    for ch in text.chars() {
        match ch {
            ' ' => width += 1,
            '\t' => width += 4,
            '\r' | '\n' => return None,
            _ => return Some(width),
        }
    }
    None
}

fn closes_only(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text
            .chars()
            .all(|c| "}])>;,".contains(c) || c.is_whitespace())
}

impl Document {
    fn short_line(&self, line: usize) -> Option<String> {
        (line < self.line_count() && self.line_len(line) <= LINE_LIMIT)
            .then(|| self.rope.line(line).to_string())
    }

    fn opener(&self, line: usize) -> Option<(usize, usize)> {
        let text = self.short_line(line)?;
        let mut stack: Vec<(usize, usize)> = Vec::new();
        for (col, ch) in text.chars().enumerate() {
            if let Some(kind) = OPEN.iter().position(|o| *o == ch) {
                stack.push((kind, col));
            } else if let Some(kind) = CLOSE.iter().position(|c| *c == ch)
                && stack.last().is_some_and(|top| top.0 == kind)
            {
                stack.pop();
            }
        }
        stack
            .first()
            .map(|(kind, col)| (*kind, self.rope.line_to_char(line) + col))
    }

    fn indent_at(&self, line: usize) -> Option<usize> {
        indent_of(&self.short_line(line).unwrap_or_else(|| "x".into()))
    }

    pub fn foldable(&self, line: usize) -> bool {
        if line + 1 >= self.line_count() {
            return false;
        }
        if let Some((kind, _)) = self.opener(line) {
            let next = self.short_line(line + 1).unwrap_or_default();
            return !next.trim_start().starts_with(CLOSE[kind]);
        }
        let Some(base) = self.indent_at(line) else {
            return false;
        };
        (line + 1..(line + 50).min(self.line_count()))
            .find_map(|l| self.indent_at(l))
            .is_some_and(|i| i > base)
    }

    fn bracket_end(&self, line: usize, close: usize) -> Option<usize> {
        let end = if closes_only(&self.short_line(close).unwrap_or_default()) {
            close
        } else {
            close - 1
        };
        (end > line).then_some(end)
    }

    pub fn fold_end(&self, line: usize) -> Option<usize> {
        if let Some((kind, at)) = self.opener(line) {
            let mut depth = 0usize;
            for (step, ch) in self.rope.chars_at(at).enumerate().take(REACH) {
                if ch == OPEN[kind] {
                    depth += 1;
                } else if ch == CLOSE[kind] {
                    depth -= 1;
                    if depth == 0 {
                        let close = self.rope.char_to_line(at + step);
                        return self.bracket_end(line, close);
                    }
                }
            }
        }
        let base = self.indent_at(line)?;
        let mut end = line;
        for l in line + 1..self.line_count() {
            match self.indent_at(l) {
                None => continue,
                Some(i) if i > base => end = l,
                _ => break,
            }
        }
        (end > line).then_some(end)
    }

    pub fn block_end(&self, line: usize) -> Option<usize> {
        let mut cache = self.ends.borrow_mut();
        if cache.0 != self.version || cache.1.len() > 4096 {
            *cache = (self.version, Default::default());
        }
        if let Some(end) = cache.1.get(&line) {
            return *end;
        }
        let end = self.foldable(line).then(|| self.fold_end(line)).flatten();
        cache.1.insert(line, end);
        end
    }

    pub fn block_around(&self, line: usize) -> Option<(usize, usize)> {
        let mut head = line + 1;
        while head > 0 && line + 1 - head < 2000 {
            head -= 1;
            if let Some(end) = self.block_end(head)
                && end >= line
                && (head < line || end > line)
            {
                return Some((head, end));
            }
        }
        None
    }

    pub fn is_folded(&self, line: usize) -> bool {
        self.folds.binary_search_by_key(&line, |f| f.0).is_ok()
    }

    fn refold(&mut self) {
        self.folds.sort_unstable();
        self.folds.dedup_by_key(|f| f.0);
        self.hidden.clear();
        for &(head, end) in &self.folds {
            if let Some(last) = self.hidden.last_mut()
                && head <= last.1
            {
                if head >= last.0 {
                    last.1 = last.1.max(end);
                }
                continue;
            }
            self.hidden.push((head + 1, end));
        }
        self.skip.clear();
        self.skip.push(0);
        let mut total = 0;
        for (a, b) in &self.hidden {
            total += b - a + 1;
            self.skip.push(total);
        }
    }

    pub fn toggle_fold(&mut self, line: usize) -> bool {
        if self.is_folded(line) {
            self.folds.retain(|f| f.0 != line);
        } else if let Some(end) = self.fold_end(line) {
            self.folds.push((line, end));
        } else {
            return false;
        }
        self.refold();
        self.after_fold();
        true
    }

    fn after_fold(&mut self) {
        let line = self.rope.char_to_line(self.sel.1);
        if self.hidden_range(line).is_some() {
            let (head, _) = self
                .folds
                .iter()
                .filter(|f| f.0 < line && f.1 >= line)
                .min_by_key(|f| f.0)
                .copied()
                .unwrap_or((line, line));
            let pos = self.rope.line_to_char(head) + self.line_len(head);
            self.sel = (pos, pos);
            self.extra.clear();
        }
    }

    pub fn fold_here(&mut self) -> bool {
        let line = self.rope.char_to_line(self.sel.1);
        if !self.is_folded(line) && self.fold_end(line).is_some() {
            return self.toggle_fold(line);
        }
        let mut head = line;
        while head > 0 && line - head < 5000 {
            head -= 1;
            if !self.is_folded(head)
                && self.foldable(head)
                && self.fold_end(head).is_some_and(|end| end >= line)
            {
                return self.toggle_fold(head);
            }
        }
        false
    }

    pub fn unfold_here(&mut self) -> bool {
        let line = self.rope.char_to_line(self.sel.1);
        let before = self.folds.len();
        self.folds.retain(|f| !(f.0 <= line && f.1 >= line));
        self.refold();
        before != self.folds.len()
    }

    pub fn unfold_all(&mut self) {
        self.folds.clear();
        self.refold();
    }

    pub fn fold_all(&mut self) {
        let count = self.line_count();
        let mut ends: Vec<Option<usize>> = vec![None; count];
        let mut open_at = vec![false; count];
        let mut closer = vec![false; count];
        let mut indent: Vec<Option<usize>> = vec![None; count];
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let scan = self.len() <= SCAN;
        let mut local: Vec<usize> = Vec::new();
        for (line, text) in self.rope.lines().enumerate() {
            local.clear();
            let short = text.len_chars() <= LINE_LIMIT;
            let mut width = 0;
            let mut lead = true;
            let mut only = true;
            let mut any = false;
            for ch in text.chars() {
                if lead {
                    match ch {
                        ' ' => width += 1,
                        '\t' => width += 4,
                        '\r' | '\n' => {}
                        _ => {
                            lead = false;
                            indent[line] = Some(width);
                        }
                    }
                }
                if !ch.is_whitespace() {
                    any = true;
                    if !"}])>;,".contains(ch) {
                        only = false;
                    }
                }
                if let Some(kind) = OPEN.iter().position(|o| *o == ch) {
                    local.push(kind);
                    if scan {
                        stack.push((kind, line));
                    }
                } else if let Some(kind) = CLOSE.iter().position(|c| *c == ch) {
                    if local.last() == Some(&kind) {
                        local.pop();
                    }
                    if scan
                        && stack.last().is_some_and(|top| top.0 == kind)
                        && let Some((_, from)) = stack.pop()
                        && line > from
                        && ends[from].is_none_or(|e| e < line)
                    {
                        ends[from] = Some(line);
                    }
                }
            }
            if !short {
                indent[line] = Some(0);
            }
            open_at[line] = short && !local.is_empty();
            closer[line] = short && any && only;
        }
        let mut by_indent: Vec<Option<usize>> = vec![None; count];
        let mut open: Vec<(usize, usize)> = Vec::new();
        let mut last = 0;
        let widths = indent
            .iter()
            .enumerate()
            .filter_map(|(l, w)| w.map(|w| (l, Some(w))));
        for (line, width) in widths.chain(std::iter::once((count, None))) {
            while let Some(&(w, head)) = open.last() {
                if width.is_some_and(|i| w < i) {
                    break;
                }
                open.pop();
                if last > head {
                    by_indent[head] = Some(last);
                }
            }
            if let Some(w) = width {
                open.push((w, line));
                last = line;
            }
        }
        self.folds = (0..count)
            .filter_map(|line| {
                let end = if open_at[line] {
                    ends[line]
                        .map(|close| if closer[close] { close } else { close - 1 })
                        .filter(|end| *end > line)
                        .or(by_indent[line].filter(|_| ends[line].is_none()))
                } else {
                    by_indent[line]
                };
                end.map(|end| (line, end))
            })
            .collect();
        self.refold();
        self.after_fold();
    }

    pub fn hidden_range(&self, line: usize) -> Option<(usize, usize)> {
        let at = self.hidden.partition_point(|r| r.1 < line);
        self.hidden.get(at).filter(|r| r.0 <= line).copied()
    }

    pub fn reveal_line(&mut self, line: usize) {
        if self.hidden_range(line).is_some() {
            self.folds.retain(|f| !(f.0 < line && f.1 >= line));
            self.refold();
        }
    }

    pub fn shown_count(&self) -> usize {
        self.line_count() - self.skip.last().copied().unwrap_or(0)
    }

    pub fn to_visual(&self, line: usize) -> usize {
        let at = self.hidden.partition_point(|r| r.1 < line);
        let mut visual = line - self.skip[at];
        if let Some((a, _)) = self.hidden.get(at)
            && *a <= line
        {
            visual -= line - a + 1;
        }
        visual
    }

    pub fn line_of_visual(&self, visual: usize) -> usize {
        let mut lo = 0;
        let mut hi = self.hidden.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if visual + self.skip[mid] >= self.hidden[mid].0 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (visual + self.skip[lo]).min(self.line_count() - 1)
    }

    pub fn next_shown(&self, line: usize) -> Option<usize> {
        let mut next = line + 1;
        if let Some((_, b)) = self.hidden_range(next) {
            next = b + 1;
        }
        (next < self.line_count()).then_some(next)
    }

    pub fn prev_shown(&self, line: usize) -> Option<usize> {
        let prev = line.checked_sub(1)?;
        Some(self.hidden_range(prev).map_or(prev, |(a, _)| a - 1))
    }

    pub(crate) fn shift_folds(&mut self, start: usize, old_end: usize, new_end: usize) {
        if self.folds.is_empty() {
            return;
        }
        let same = old_end == new_end && start == old_end;
        self.folds.retain_mut(|f| {
            if f.1 < start {
                true
            } else if f.0 > old_end {
                f.0 = f.0 + new_end - old_end;
                f.1 = f.1 + new_end - old_end;
                true
            } else {
                same && f.0 == start
            }
        });
        self.refold();
    }

    pub(crate) fn drop_folds(&mut self) {
        self.folds.clear();
        self.hidden.clear();
        self.skip.clear();
        self.skip.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text)
    }

    #[test]
    fn braces_fold_to_the_closing_line() {
        let mut d = doc("fn a() {\n    x();\n    y();\n}\nz\n");
        assert!(d.foldable(0));
        assert_eq!(d.fold_end(0), Some(3));
        assert!(d.toggle_fold(0));
        assert_eq!(d.hidden, vec![(1, 3)]);
        assert_eq!(d.shown_count(), 3);
        assert_eq!(d.line_of_visual(1), 4);
        assert_eq!(d.to_visual(4), 1);
        assert_eq!(d.to_visual(2), 0);
        assert_eq!(d.next_shown(0), Some(4));
        assert_eq!(d.prev_shown(4), Some(0));
    }

    #[test]
    fn else_line_stays_visible() {
        let d = doc("if (a) {\n  b;\n} else {\n  c;\n}\n");
        assert_eq!(d.fold_end(0), Some(1));
        assert_eq!(d.fold_end(2), Some(4));
        assert!(!doc("x {\n}\n").foldable(0));
    }

    #[test]
    fn indentation_folds_without_brackets() {
        let d = doc("def f():\n    a = 1\n\n    return a\nprint(f())\n");
        assert!(d.foldable(0));
        assert_eq!(d.fold_end(0), Some(3));
        assert!(!d.foldable(4));
    }

    #[test]
    fn edits_shift_or_drop_folds() {
        let mut d = doc("a\nb {\n  c\n}\nd\n");
        d.toggle_fold(1);
        d.move_to(0, false);
        d.insert_text("x\n");
        assert_eq!(d.folds, vec![(2, 4)]);
        let pos = d.rope.line_to_char(2) + 1;
        d.move_to(pos, false);
        d.insert_text("y");
        assert_eq!(d.folds, vec![(2, 4)]);
        let pos = d.rope.line_to_char(3);
        d.move_to(pos, false);
        d.insert_text("z");
        assert!(d.folds.is_empty());
    }

    #[test]
    fn fold_all_matches_single_folds_and_nests() {
        let text = "a {\n  b {\n    c\n  }\n  d\n}\ne\n";
        let mut d = doc(text);
        d.fold_all();
        assert_eq!(d.folds, vec![(0, 5), (1, 3)]);
        assert_eq!(d.hidden, vec![(1, 5)]);
        d.toggle_fold(0);
        assert_eq!(d.hidden, vec![(2, 3)]);
        assert_eq!(d.line_of_visual(2), 4);
    }

    #[test]
    fn fold_all_agrees_with_single_folds() {
        let text = "<?php\nclass A {\n    function a() {\n        if ($x) {\n            f(1, [\n                2,\n            ]);\n        } else {\n            b();\n        }\n    }\n\n    function c() { return 1; }\n}\ndef g():\n    pass\n\nx\n";
        let mut d = doc(text);
        d.fold_all();
        let all = d.folds.clone();
        let single: Vec<(usize, usize)> = (0..d.line_count())
            .filter_map(|l| d.fold_end(l).map(|e| (l, e)))
            .collect();
        assert_eq!(all, single);
    }

    #[test]
    fn visual_mapping_round_trips() {
        let mut d = doc(&"a {\n  b\n  c\n}\nx\n".repeat(50));
        d.fold_all();
        for line in 0..d.line_count() {
            if d.hidden_range(line).is_none() {
                assert_eq!(d.line_of_visual(d.to_visual(line)), line);
            }
        }
        assert_eq!(d.shown_count(), 101);
    }

    #[test]
    fn cursor_inside_a_new_fold_moves_to_its_header() {
        let mut d = doc("a {\n  b\n}\n");
        let pos = d.rope.line_to_char(1) + 2;
        d.move_to(pos, false);
        assert!(d.fold_here());
        assert_eq!(d.rope.char_to_line(d.sel.1), 0);
        d.reveal_line(1);
        assert!(d.folds.is_empty());
    }
}
