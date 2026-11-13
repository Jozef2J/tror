use crate::document::{Document, EditKind};

impl Document {
    pub fn toggle_mark(&mut self) {
        let line = self.rope.char_to_line(self.sel.1);
        if !self.marks.remove(&line) {
            self.marks.insert(line);
        }
    }

    pub fn jump_mark(&mut self, forward: bool) -> bool {
        let line = self.rope.char_to_line(self.sel.1);
        let target = if forward {
            self.marks
                .range(line + 1..)
                .next()
                .or_else(|| self.marks.iter().next())
        } else {
            self.marks
                .range(..line)
                .next_back()
                .or_else(|| self.marks.iter().next_back())
        };
        let Some(&target) = target else {
            return false;
        };
        let at = self.rope.line_to_char(target.min(self.line_count() - 1));
        self.move_to(at, false);
        self.reveal = true;
        true
    }

    pub(crate) fn shift_marks(&mut self, start: usize, old_end: usize, new_end: usize) {
        if self.marks.is_empty() || old_end == new_end {
            return;
        }
        let moved: Vec<usize> = self.marks.range(start + 1..).copied().collect();
        for line in &moved {
            self.marks.remove(line);
        }
        for line in moved {
            let shifted = if line > old_end {
                line + new_end - old_end
            } else {
                line.min(new_end)
            };
            self.marks.insert(shifted);
        }
    }

    pub(crate) fn clamp_marks(&mut self) {
        let count = self.line_count();
        self.marks.retain(|line| *line < count);
    }

    fn line_bounds(&self) -> (usize, usize) {
        let range = self.selection();
        let first = self.rope.char_to_line(range.start);
        let mut last = self.rope.char_to_line(range.end);
        if range.end > range.start && last > first && self.rope.line_to_char(last) == range.end {
            last -= 1;
        }
        (first, last)
    }

    fn line_start(&self, line: usize) -> usize {
        if line < self.rope.len_lines() {
            self.rope.line_to_char(line)
        } else {
            self.rope.len_chars()
        }
    }

    pub fn move_lines(&mut self, up: bool) {
        self.clear_extra();
        let (first, last) = self.line_bounds();
        let lines = self.rope.len_lines();
        let (from, to, block_start) = if up {
            if first == 0 {
                return;
            }
            (first - 1, last, self.line_start(first))
        } else {
            if last + 1 >= lines || (last + 2 == lines && self.line_len(last + 1) == 0) {
                return;
            }
            (first, last + 1, self.line_start(first))
        };
        let start = self.line_start(from);
        let end = self.line_start(to + 1);
        let split = if up {
            block_start
        } else {
            self.line_start(last + 1)
        };
        let head = self.rope.slice(start..split).to_string();
        let tail = self.rope.slice(split..end).to_string();
        let (moved, other) = if up { (tail, head) } else { (head, tail) };
        let (moved, other) = if moved.ends_with('\n') {
            (moved, other)
        } else {
            let other = other.strip_suffix('\n').unwrap_or(&other).to_owned();
            (format!("{moved}\n"), other)
        };
        let text = if up {
            format!("{moved}{other}")
        } else if other.ends_with('\n') {
            format!("{other}{moved}")
        } else {
            format!("{other}\n{}", moved.strip_suffix('\n').unwrap_or(&moved))
        };
        let new_start = if up {
            start
        } else {
            start + other.chars().count() + usize::from(!other.ends_with('\n'))
        };
        let sel = self.sel;
        let marks: Vec<usize> = self.marks.range(first..=last).copied().collect();
        let other_line = if up { from } else { to };
        let other_mark = self.marks.contains(&other_line);
        self.replace(start..end, &text, EditKind::Other);
        let delta = new_start as isize - block_start as isize;
        let shift = |p: usize| (p as isize + delta).max(0) as usize;
        self.sel = (shift(sel.0).min(self.len()), shift(sel.1).min(self.len()));
        for line in self.marks.range(from..=to).copied().collect::<Vec<_>>() {
            self.marks.remove(&line);
        }
        let (moved_to, other_to) = if up { (from, last) } else { (first + 1, first) };
        for line in marks {
            self.marks.insert(line - first + moved_to);
        }
        if other_mark {
            self.marks.insert(other_to);
        }
        self.reveal = true;
    }

    pub fn trim_trailing(&mut self) -> usize {
        let mut changed = 0;
        let mut out = String::with_capacity(self.len());
        for line in self.rope.lines() {
            let text = line.to_string();
            let body = text.trim_end_matches(['\n', '\r']);
            let trimmed = body.trim_end_matches([' ', '\t']);
            if trimmed.len() != body.len() {
                changed += 1;
            }
            out.push_str(trimmed);
            out.push_str(&text[body.len()..]);
        }
        if changed > 0 {
            let sel = self.sel;
            let len = self.len();
            self.replace(0..len, &out, EditKind::Other);
            let len = self.len();
            self.sel = (sel.0.min(len), sel.1.min(len));
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text)
    }

    fn select_line(d: &mut Document, line: usize) {
        let at = d.rope.line_to_char(line) + 1;
        d.move_to(at, false);
    }

    #[test]
    fn move_lines_up_and_down() {
        let mut d = doc("a\nb\nc\n");
        select_line(&mut d, 1);
        d.move_lines(true);
        assert_eq!(d.rope.to_string(), "b\na\nc\n");
        assert_eq!(d.rope.char_to_line(d.sel.1), 0);
        d.move_lines(true);
        assert_eq!(d.rope.to_string(), "b\na\nc\n");
        d.move_lines(false);
        d.move_lines(false);
        assert_eq!(d.rope.to_string(), "a\nc\nb\n");
        assert_eq!(d.rope.char_to_line(d.sel.1), 2);
        d.move_lines(false);
        assert_eq!(d.rope.to_string(), "a\nc\nb\n");
    }

    #[test]
    fn move_lines_without_final_newline() {
        let mut d = doc("a\nb\nc");
        select_line(&mut d, 2);
        d.move_lines(true);
        assert_eq!(d.rope.to_string(), "a\nc\nb");
        assert_eq!(d.rope.char_to_line(d.sel.1), 1);
        d.move_lines(false);
        assert_eq!(d.rope.to_string(), "a\nb\nc");
        assert_eq!(d.rope.char_to_line(d.sel.1), 2);
        d.undo();
        assert_eq!(d.rope.to_string(), "a\nc\nb");
    }

    #[test]
    fn move_selected_block_keeps_selection() {
        let mut d = doc("1\n2\n3\n4\n");
        let a = d.rope.line_to_char(1);
        let b = d.rope.line_to_char(3);
        d.move_to(a, false);
        d.move_to(b, true);
        d.move_lines(false);
        assert_eq!(d.rope.to_string(), "1\n4\n2\n3\n");
        assert_eq!(d.selected_text(), "2\n3\n");
    }

    #[test]
    fn marks_follow_edits_and_moves() {
        let mut d = doc("a\nb\nc\nd\n");
        select_line(&mut d, 2);
        d.toggle_mark();
        d.move_to(0, false);
        d.insert_text("x\ny\n");
        assert!(d.marks.contains(&4));
        select_line(&mut d, 4);
        d.move_lines(true);
        assert_eq!(d.marks.iter().copied().collect::<Vec<_>>(), vec![3]);
        d.move_to(0, false);
        assert!(d.jump_mark(true));
        assert_eq!(d.rope.char_to_line(d.sel.1), 3);
        d.toggle_mark();
        assert!(d.marks.is_empty());
        assert!(!d.jump_mark(true));
    }

    #[test]
    fn trim_trailing_spaces() {
        let mut d = doc("a  \nb\t\r\nc\n  \n");
        assert_eq!(d.trim_trailing(), 3);
        assert_eq!(d.rope.to_string(), "a\nb\r\nc\n\n");
        assert_eq!(d.trim_trailing(), 0);
    }
}
