use std::ops::Range;

use eframe::egui::Color32;
use ropey::{Rope, RopeSlice};

use crate::text::document::Document;
use crate::text::highlight::{Span, Syntaxes};

impl Document {
    pub fn highlight(
        &mut self,
        syn: &Syntaxes,
        visible: Range<usize>,
        budget: Option<std::time::Duration>,
    ) -> bool {
        self.hl.process(syn, &self.rope, &self.syntax, self.version, visible, budget)
    }

    pub fn spans(&self, line: usize) -> &[Span] {
        self.hl.spans(line)
    }

    pub fn text_color(&self) -> Color32 {
        self.hl.default_color
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn rope(&self) -> Rope {
        self.rope.clone()
    }

    pub fn char_to_line(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos)
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn char_at(&self, pos: usize) -> char {
        self.rope.char(pos)
    }

    pub fn line(&self, line: usize) -> RopeSlice<'_> {
        self.rope.line(line)
    }

    pub fn slice(&self, range: impl std::ops::RangeBounds<usize>) -> RopeSlice<'_> {
        self.rope.slice(range)
    }

    pub fn chars_at(&self, pos: usize) -> ropey::iter::Chars<'_> {
        self.rope.chars_at(pos)
    }

    pub fn select(&mut self, anchor: usize, head: usize) {
        let len = self.len();
        self.sel = (anchor.min(len), head.min(len));
        self.goal_x = None;
    }

    pub fn jump_to(&mut self, anchor: usize, head: usize) {
        self.select(anchor, head);
        self.reveal = true;
    }

    pub fn move_to_goal(&mut self, pos: usize, extend: bool, goal: f64) {
        self.move_to(pos, extend);
        self.goal_x = Some(goal);
    }

    pub fn collapse_to_primary(&mut self) {
        let sel = self.sel;
        self.clear_extra();
        self.sel = sel;
    }

    pub fn extras(&self) -> Vec<(usize, usize)> {
        self.extra.clone()
    }

    pub fn stop_reveal(&mut self) {
        self.reveal = false;
    }

    pub fn take_reveal(&mut self) -> Option<usize> {
        std::mem::take(&mut self.reveal).then_some(self.sel.1)
    }

    pub fn reveal_cursor(&mut self) {
        let line = self.char_to_line(self.sel.1);
        if self.hidden_range(line).is_some() {
            self.reveal_line(line);
            self.reveal = true;
        }
    }

    pub fn scroll_by(&mut self, dx: f64, dy: f64) {
        self.scroll_x += dx;
        self.scroll_y += dy;
    }

    pub fn scroll_to(&mut self, x: f64, y: f64, max_x: f64, max_y: f64) {
        self.scroll_x = x.clamp(0.0, max_x.max(0.0));
        self.scroll_y = y.clamp(0.0, max_y.max(0.0));
    }

    pub fn widen(&mut self, width: f64) {
        self.content_w = self.content_w.max(width);
    }

    pub fn note_activity(&mut self, now: f64, before: (u64, (usize, usize))) {
        if before != (self.version, self.sel) {
            self.activity = now;
        }
    }

    pub fn set_block(&mut self, anchor: (usize, f64), target: (usize, f64)) {
        self.block = Some(((anchor.0, anchor.1.max(0.0)), (target.0, target.1.max(0.0))));
    }

    pub fn has_block(&self) -> bool {
        self.block.is_some()
    }

    pub fn anchor_line(&self) -> usize {
        self.char_to_line(self.sel.0.min(self.len()))
    }

    pub fn clear_block(&mut self) {
        self.block = None;
    }

    pub fn set_block_cursors(&mut self, cursors: Vec<(usize, usize)>) {
        let Some(&sel) = cursors.last() else {
            return;
        };
        let primary = cursors.len() - 1;
        let block = self.block;
        self.set_cursors(cursors.clone(), primary);
        if self.cursors().len() != cursors.len() {
            self.sel = sel;
            self.extra = cursors[..primary].to_vec();
        }
        self.block = block;
        self.reveal = true;
    }

    pub fn has_mark(&self, line: usize) -> bool {
        self.marks.contains(&line)
    }

    pub fn toggle_mark_at(&mut self, line: usize) {
        if !self.marks.remove(&line) {
            self.marks.insert(line);
        }
    }

    pub fn cursor_line(&self) -> usize {
        self.char_to_line(self.sel.1.min(self.len()))
    }

    pub fn has_marks(&self) -> bool {
        !self.marks.is_empty()
    }

    pub fn clear_marks(&mut self) {
        self.marks.clear();
    }

    pub fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    pub fn folded_at(&self, line: usize) -> bool {
        self.folds.iter().any(|f| f.0 <= line && f.1 >= line)
    }

    pub fn request_reveal(&mut self) {
        self.reveal = true;
    }

    pub fn view_state(&self) -> (Vec<usize>, Vec<(usize, usize)>) {
        (self.marks.iter().copied().collect(), self.folds.clone())
    }
}
