use super::model::*;
use super::{Pos, Visual};

pub fn parent(path: &[usize]) -> &[usize] {
    &path[..path.len().saturating_sub(1)]
}

fn is_list(tag: &str) -> bool {
    tag == "ul" || tag == "ol"
}

impl Visual {
    pub(super) fn refresh(&mut self) {
        self.paras = self.html.paras();
        let last = self.paras.len().saturating_sub(1);
        for pos in [&mut self.head, &mut self.anchor] {
            pos.para = pos.para.min(last);
        }
        let head_len = self.len(self.head.para);
        let anchor_len = self.len(self.anchor.para);
        self.head.off = self.head.off.min(head_len);
        self.anchor.off = self.anchor.off.min(anchor_len);
    }

    pub(super) fn len(&self, para: usize) -> usize {
        self.paras
            .get(para)
            .and_then(|p| self.html.block(p))
            .and_then(Block::runs)
            .map_or(0, |r| text_len(r))
    }

    pub(super) fn runs(&self, para: usize) -> Option<&Vec<Inline>> {
        self.html.block(self.paras.get(para)?)?.runs()
    }

    pub(super) fn runs_mut(&mut self, para: usize) -> Option<&mut Vec<Inline>> {
        let path = self.paras.get(para)?.clone();
        self.html.block_mut(&path)?.runs_mut()
    }

    pub(super) fn index_of(&self, path: &[usize]) -> Option<usize> {
        self.paras.iter().position(|p| p == path)
    }

    pub(super) fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub(super) fn collapse_to(&mut self, pos: Pos) {
        self.head = pos;
        self.anchor = pos;
        self.pending = None;
    }

    fn remove_block(&mut self, path: &[usize]) {
        let Some((last, up)) = path.split_last() else {
            return;
        };
        if let Some(list) = self.html.list_mut(up) {
            if *last < list.len() {
                list.remove(*last);
            }
        } else {
            return;
        }
        let mut up = up.to_vec();
        while !up.is_empty() {
            let Some(block) = self.html.block(&up) else {
                break;
            };
            let empty = block.children().is_some_and(Vec::is_empty);
            if !empty {
                break;
            }
            if matches!(block.tag(), "td" | "th" | "li" | "blockquote" | "div") && up.len() > 1
                || matches!(block.tag(), "td" | "th")
            {
                if let Some(children) = self.html.block_mut(&up).and_then(Block::children_mut) {
                    children.push(Block::para("", Vec::new()));
                }
                break;
            }
            let (index, rest) = up
                .split_last()
                .map(|(a, b)| (*a, b.to_vec()))
                .unwrap_or((0, Vec::new()));
            if let Some(list) = self.html.list_mut(&rest) {
                list.remove(index);
            }
            up = rest;
        }
        if self.html.blocks.is_empty() {
            self.html.blocks.push(Block::para("p", Vec::new()));
        }
    }

    pub(super) fn delete_range(&mut self, a: Pos, b: Pos) {
        if a.para == b.para {
            if let Some(runs) = self.runs_mut(a.para) {
                cut(runs, a.off..b.off);
            }
        } else {
            let end = self.len(b.para);
            let tail = self
                .runs_mut(b.para)
                .map(|runs| cut(runs, b.off..end))
                .unwrap_or_default();
            let a_len = self.len(a.para);
            if let Some(runs) = self.runs_mut(a.para) {
                cut(runs, a.off..a_len);
                let at = text_len(runs);
                insert_runs(runs, at, tail);
            }
            let doomed: Vec<Path> = self.paras[a.para + 1..=b.para].to_vec();
            for path in doomed.iter().rev() {
                self.remove_block(path);
            }
            self.paras = self.html.paras();
        }
        self.collapse_to(a);
        self.refresh();
    }

    pub(super) fn delete_selection(&mut self) -> bool {
        if self.anchor == self.head {
            return false;
        }
        let (a, b) = self.ordered();
        self.delete_range(a, b);
        true
    }

    pub(super) fn typing_marks(&self) -> Vec<Mark> {
        if let Some(marks) = &self.pending {
            return marks.clone();
        }
        self.runs(self.head.para)
            .map(|r| marks_at(r, self.head.off))
            .unwrap_or_default()
    }

    pub(super) fn insert_text(&mut self, text: &str) {
        let marks = self.typing_marks();
        self.delete_selection();
        let pre = self
            .paras
            .get(self.head.para)
            .and_then(|p| self.html.block(p))
            .is_some_and(|b| b.tag() == "pre");
        let mut lines = text.split('\n');
        let first = lines.next().unwrap_or_default().replace('\r', "");
        let head = self.head;
        if let Some(runs) = self.runs_mut(head.para) {
            insert(runs, head.off, &first, &marks);
        }
        self.collapse_to(Pos {
            para: head.para,
            off: head.off + first.chars().count(),
        });
        for line in lines {
            if pre {
                let head = self.head;
                if let Some(runs) = self.runs_mut(head.para) {
                    insert(runs, head.off, "\n", &marks);
                }
                self.head.off += 1;
            } else {
                self.split();
            }
            let line = line.replace('\r', "");
            let head = self.head;
            if let Some(runs) = self.runs_mut(head.para) {
                insert(runs, head.off, &line, &marks);
            }
            self.collapse_to(Pos {
                para: head.para,
                off: head.off + line.chars().count(),
            });
        }
        self.pending = Some(marks);
    }

    pub(super) fn insert_object(&mut self, html: String) {
        let marks = self.typing_marks();
        self.delete_selection();
        let head = self.head;
        if let Some(runs) = self.runs_mut(head.para) {
            insert_runs(runs, head.off, vec![Inline::Object { html, marks }]);
        }
        self.collapse_to(Pos {
            para: head.para,
            off: head.off + 1,
        });
    }

    pub(super) fn line_break(&mut self) {
        self.insert_object("<br>".into());
    }

    pub(super) fn split(&mut self) {
        self.delete_selection();
        let head = self.head;
        let Some(path) = self.paras.get(head.para).cloned() else {
            return;
        };
        let Some(block) = self.html.block(&path).cloned() else {
            return;
        };
        let Kind::Para { tag, attrs, .. } = &block.kind else {
            return;
        };
        if tag == "pre" {
            if let Some(runs) = self.runs_mut(head.para) {
                insert(runs, head.off, "\n", &[]);
            }
            self.collapse_to(Pos {
                para: head.para,
                off: head.off + 1,
            });
            return;
        }
        let len = self.len(head.para);
        let tail = self
            .runs_mut(head.para)
            .map(|r| cut(r, head.off..len))
            .unwrap_or_default();
        let up = parent(&path).to_vec();
        let index = *path.last().unwrap_or(&0);
        let parent_tag = self
            .html
            .block(&up)
            .map(|b| b.tag().to_owned())
            .unwrap_or_default();
        let new_path;
        if !tag.is_empty() {
            let heading = tag.starts_with('h') && tag.len() == 2;
            let new_tag = if heading && tail.is_empty() {
                "p"
            } else {
                tag.as_str()
            };
            let mut new_attrs = attrs.clone();
            new_attrs.set("id", None);
            let mut block = Block::para(new_tag, tail);
            if let Some(a) = block.attrs_mut() {
                *a = if new_tag == tag {
                    new_attrs
                } else {
                    Attrs::default()
                };
            }
            if let Some(list) = self.html.list_mut(&up) {
                list.insert(index + 1, block);
            }
            new_path = [up.as_slice(), &[index + 1]].concat();
        } else if parent_tag == "li" {
            let empty = len == 0;
            let li_path = up.clone();
            let list_path = parent(&li_path).to_vec();
            let li_index = *li_path.last().unwrap_or(&0);
            let only = self
                .html
                .block(&li_path)
                .and_then(Block::children)
                .is_some_and(|c| c.len() == 1);
            if empty && only {
                let list_index = *list_path.last().unwrap_or(&0);
                let outer = parent(&list_path).to_vec();
                let after = self
                    .html
                    .list_mut(&list_path)
                    .map(|items| items.split_off(li_index + 1))
                    .unwrap_or_default();
                if let Some(items) = self.html.list_mut(&list_path) {
                    items.pop();
                }
                let outer_tag = self
                    .html
                    .block(&outer)
                    .map(|b| b.tag().to_owned())
                    .unwrap_or_default();
                let p_tag = if outer_tag == "li" { "" } else { "p" };
                let mut at = list_index + 1;
                if let Some(list) = self.html.list_mut(&outer) {
                    list.insert(at, Block::para(p_tag, Vec::new()));
                    if !after.is_empty() {
                        let tag = list[list_index].tag().to_owned();
                        list.insert(at + 1, Block::boxed(&tag, after));
                    }
                }
                let list_empty = self
                    .html
                    .block(&list_path)
                    .and_then(Block::children)
                    .is_some_and(Vec::is_empty);
                if list_empty {
                    if let Some(list) = self.html.list_mut(&outer) {
                        list.remove(list_index);
                    }
                    at -= 1;
                }
                new_path = [outer.as_slice(), &[at]].concat();
            } else {
                let li = Block::boxed("li", vec![Block::para("", tail)]);
                if let Some(items) = self.html.list_mut(&list_path) {
                    items.insert(li_index + 1, li);
                }
                new_path = [list_path.as_slice(), &[li_index + 1, 0]].concat();
            }
        } else {
            if let Some(Kind::Para { tag, .. }) = self.html.block_mut(&path).map(|b| &mut b.kind) {
                *tag = "p".into();
            }
            if let Some(list) = self.html.list_mut(&up) {
                list.insert(index + 1, Block::para("p", tail));
            }
            new_path = [up.as_slice(), &[index + 1]].concat();
        }
        self.paras = self.html.paras();
        let para = self.index_of(&new_path).unwrap_or(head.para + 1);
        self.collapse_to(Pos { para, off: 0 });
        self.refresh();
    }

    pub(super) fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let head = self.head;
        if head.off > 0 {
            self.delete_range(
                Pos {
                    para: head.para,
                    off: head.off - 1,
                },
                head,
            );
        } else if head.para > 0 {
            if self.unlist_first(head.para) {
                return;
            }
            let prev = Pos {
                para: head.para - 1,
                off: self.len(head.para - 1),
            };
            self.delete_range(prev, head);
        }
    }

    fn unlist_first(&mut self, para: usize) -> bool {
        let path = self.paras[para].clone();
        let up = parent(&path);
        let in_li = self.html.block(up).is_some_and(|b| b.tag() == "li");
        if !in_li || path.last() != Some(&0) {
            return false;
        }
        self.outdent_li(up.to_vec());
        true
    }

    pub(super) fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let head = self.head;
        let len = self.len(head.para);
        if head.off < len {
            self.delete_range(
                head,
                Pos {
                    para: head.para,
                    off: head.off + 1,
                },
            );
        } else if head.para + 1 < self.paras.len() {
            self.delete_range(
                head,
                Pos {
                    para: head.para + 1,
                    off: 0,
                },
            );
        }
    }

    pub(super) fn selected_html(&self) -> String {
        let (a, b) = self.ordered();
        let mut out = String::new();
        for para in a.para..=b.para {
            let Some(runs) = self.runs(para) else {
                continue;
            };
            let mut runs = runs.clone();
            let len = text_len(&runs);
            let from = if para == a.para { a.off.min(len) } else { 0 };
            let to = if para == b.para { b.off.min(len) } else { len };
            let piece = cut(&mut runs, from..to.max(from));
            if a.para == b.para {
                write_runs(&piece, &mut out);
                break;
            }
            let tag = self
                .paras
                .get(para)
                .and_then(|p| self.html.block(p))
                .map(|b| b.tag().to_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "p".into());
            out.push_str(&format!("<{tag}>"));
            write_runs(&piece, &mut out);
            out.push_str(&format!("</{tag}>\n"));
        }
        restore(&out, &self.html.atoms)
    }

    pub(super) fn insert_html(&mut self, html: &str) -> bool {
        let fragment = parse(html);
        let mut blocks = fragment.blocks;
        blocks.retain(|b| match &b.kind {
            Kind::Raw(raw) => !raw.trim().is_empty(),
            _ => true,
        });
        let has_content = |b: &Block| -> bool {
            fn any(b: &Block) -> bool {
                match &b.kind {
                    Kind::Para { runs, .. } => !runs.is_empty(),
                    Kind::Box { children, .. } => children.iter().any(any) || !children.is_empty(),
                    Kind::Raw(_) => true,
                }
            }
            any(b)
        };
        if !blocks.iter().any(has_content) {
            return false;
        }
        adopt(&mut blocks, &fragment.atoms, &mut self.html.atoms);
        self.insert_blocks(blocks);
        true
    }

    fn insert_blocks(&mut self, mut blocks: Vec<Block>) {
        self.delete_selection();
        self.pending = None;
        let simple =
            |b: &Block| matches!(&b.kind, Kind::Para { tag, .. } if tag.is_empty() || tag == "p");
        let in_pre = self
            .paras
            .get(self.head.para)
            .and_then(|p| self.html.block(p))
            .is_some_and(|b| b.tag() == "pre");
        if in_pre || (blocks.len() == 1 && simple(&blocks[0])) {
            let mut runs = Vec::new();
            for (i, block) in blocks.into_iter().enumerate() {
                if let Kind::Para { runs: r, .. } = block.kind {
                    if i > 0 {
                        runs.push(Inline::Text {
                            text: "\n".into(),
                            marks: Vec::new(),
                        });
                    }
                    runs.extend(r);
                }
            }
            let count = text_len(&runs);
            let head = self.head;
            if let Some(target) = self.runs_mut(head.para) {
                insert_runs(target, head.off, runs);
            }
            self.collapse_to(Pos {
                para: head.para,
                off: head.off + count,
            });
            return;
        }
        if simple(&blocks[0]) {
            let first = blocks.remove(0);
            if let Kind::Para { runs, .. } = first.kind {
                let count = text_len(&runs);
                let head = self.head;
                if let Some(target) = self.runs_mut(head.para) {
                    insert_runs(target, head.off, runs);
                }
                self.collapse_to(Pos {
                    para: head.para,
                    off: head.off + count,
                });
            }
        }
        if blocks.is_empty() {
            return;
        }
        let head = self.head;
        let len = self.len(head.para);
        let mut path = self.paras.get(head.para).cloned().unwrap_or_default();
        let mut at = *path.last().unwrap_or(&0);
        let mut replace = false;
        if head.off >= len && len > 0 {
            at += 1;
        } else if len == 0 {
            replace = true;
        } else if head.off > 0 {
            self.split();
            path = self.paras.get(self.head.para).cloned().unwrap_or_default();
            at = *path.last().unwrap_or(&0);
        }
        let up = parent(&path).to_vec();
        fn count(blocks: &[Block]) -> usize {
            blocks
                .iter()
                .map(|b| match &b.kind {
                    Kind::Para { .. } => 1,
                    Kind::Box { children, .. } => count(children),
                    Kind::Raw(_) => 0,
                })
                .sum()
        }
        let added = count(&blocks);
        let first_path = [up.as_slice(), &[at]].concat();
        if let Some(list) = self.html.list_mut(&up) {
            if replace && at < list.len() {
                list.remove(at);
            }
            let at = at.min(list.len());
            for (k, block) in blocks.into_iter().enumerate() {
                list.insert(at + k, block);
            }
        }
        self.paras = self.html.paras();
        if self.paras.is_empty() {
            self.html.blocks.push(Block::para("p", Vec::new()));
            self.paras = self.html.paras();
        }
        let first = self
            .paras
            .iter()
            .position(|p| p.as_slice() >= first_path.as_slice())
            .unwrap_or(0);
        let last = (first + added)
            .saturating_sub(1)
            .min(self.paras.len().saturating_sub(1));
        let off = self.len(last);
        self.collapse_to(Pos { para: last, off });
        self.refresh();
    }

    pub(super) fn selected_text(&self) -> String {
        let (a, b) = self.ordered();
        let mut out = String::new();
        for para in a.para..=b.para {
            let Some(runs) = self.runs(para) else {
                continue;
            };
            let text: Vec<char> = plain(runs).chars().collect();
            let from = if para == a.para { a.off } else { 0 };
            let to = if para == b.para { b.off } else { text.len() };
            let piece: String = text[from.min(text.len())..to.min(text.len())]
                .iter()
                .map(|c| if *c == OBJ { ' ' } else { *c })
                .collect();
            if para > a.para {
                out.push('\n');
            }
            out.push_str(&piece);
        }
        out
    }

    pub(super) fn each_run(&mut self, mut f: impl FnMut(&mut Inline)) {
        let (a, b) = self.ordered();
        for para in a.para..=b.para {
            let len = self.len(para);
            let from = if para == a.para { a.off } else { 0 };
            let to = if para == b.para { b.off } else { len };
            let Some(runs) = self.runs_mut(para) else {
                continue;
            };
            let start = split_at(runs, from);
            let end = split_at(runs, to);
            for run in &mut runs[start..end] {
                f(run);
            }
            normalize(runs);
        }
    }

    pub(super) fn style_active(&self, style: Style) -> bool {
        if self.anchor == self.head {
            return self.typing_marks().iter().any(|m| m.is(style));
        }
        let (a, b) = self.ordered();
        let mut all = true;
        let mut any = false;
        for para in a.para..=b.para {
            let Some(runs) = self.runs(para) else {
                continue;
            };
            let from = if para == a.para { a.off } else { 0 };
            let to = if para == b.para {
                b.off
            } else {
                text_len(runs)
            };
            let mut pos = 0;
            for run in runs {
                let len = run.len();
                if pos + len > from && pos < to {
                    any = true;
                    all &= run.marks().iter().any(|m| m.is(style));
                }
                pos += len;
            }
        }
        any && all
    }

    pub(super) fn toggle_style(&mut self, style: Style) {
        let on = !self.style_active(style);
        let tag = style.tags()[0];
        let apply = |marks: &mut Vec<Mark>| {
            marks.retain(|m| !m.is(style));
            if on {
                marks.push(Mark::new(tag));
                if style == Style::Sub {
                    marks.retain(|m| !m.is(Style::Sup));
                }
                if style == Style::Sup {
                    marks.retain(|m| !m.is(Style::Sub));
                }
            }
        };
        if self.anchor == self.head {
            let mut marks = self.typing_marks();
            apply(&mut marks);
            self.pending = Some(marks);
            return;
        }
        self.each_run(|run| apply(run.marks_mut()));
    }

    pub(super) fn set_span_style(&mut self, prop: &str, value: Option<&str>) {
        let apply = |marks: &mut Vec<Mark>| {
            let mut found = false;
            for mark in marks.iter_mut() {
                if mark.tag == "span" && mark.attrs.style(prop).is_some() {
                    mark.attrs.set_style(prop, value);
                    found = true;
                }
            }
            if !found && let Some(v) = value {
                let mut mark = Mark::new("span");
                mark.attrs.set_style(prop, Some(v));
                marks.push(mark);
            }
            marks.retain(|m| !(m.tag == "span" && m.attrs.is_empty()));
        };
        if self.anchor == self.head {
            let mut marks = self.typing_marks();
            apply(&mut marks);
            self.pending = Some(marks);
            return;
        }
        self.each_run(|run| apply(run.marks_mut()));
    }

    pub(super) fn clear_format(&mut self) {
        if self.anchor == self.head {
            self.pending = Some(Vec::new());
            return;
        }
        self.each_run(|run| run.marks_mut().retain(|m| m.tag == "a"));
    }

    pub(super) fn link_at_cursor(&self) -> Option<(Pos, Pos, Mark)> {
        let head = self.head;
        let runs = self.runs(head.para)?;
        let mut pos = 0;
        let mut found: Option<(usize, usize, Mark)> = None;
        for run in runs {
            let len = run.len();
            let link = run.marks().iter().find(|m| m.tag == "a").cloned();
            match (&mut found, link) {
                (Some((_, end, mark)), Some(l)) if *end == pos && *mark == l => *end = pos + len,
                (Some((s, e, _)), _) if head.off >= *s && head.off <= *e => break,
                (_, Some(l)) => found = Some((pos, pos + len, l)),
                (_, None) => found = None,
            }
            pos += len;
        }
        let (s, e, mark) = found?;
        (head.off >= s && head.off <= e).then_some((
            Pos {
                para: head.para,
                off: s,
            },
            Pos {
                para: head.para,
                off: e,
            },
            mark,
        ))
    }

    pub(super) fn apply_link(&mut self, href: &str, blank: bool) {
        let mut mark = Mark::new("a");
        mark.attrs.set("href", Some(href.to_owned()));
        if blank {
            mark.attrs.set("target", Some("_blank".into()));
        }
        if self.anchor == self.head {
            if let Some((a, b, _)) = self.link_at_cursor() {
                self.anchor = a;
                self.head = b;
            } else {
                let marks = self.typing_marks();
                let head = self.head;
                let mut with = marks.clone();
                with.retain(|m| m.tag != "a");
                with.push(mark);
                if let Some(runs) = self.runs_mut(head.para) {
                    insert(runs, head.off, href, &with);
                }
                let end = Pos {
                    para: head.para,
                    off: head.off + href.chars().count(),
                };
                self.collapse_to(end);
                self.pending = Some(marks.into_iter().filter(|m| m.tag != "a").collect());
                return;
            }
        }
        self.each_run(|run| {
            let marks = run.marks_mut();
            marks.retain(|m| m.tag != "a");
            marks.push(mark.clone());
        });
    }

    pub(super) fn unlink(&mut self) {
        if self.anchor == self.head
            && let Some((a, b, _)) = self.link_at_cursor()
        {
            let head = self.head;
            self.anchor = a;
            self.head = b;
            self.each_run(|run| run.marks_mut().retain(|m| m.tag != "a"));
            self.collapse_to(head);
            return;
        }
        self.each_run(|run| run.marks_mut().retain(|m| m.tag != "a"));
    }

    pub(super) fn current_block_tag(&self) -> String {
        self.paras
            .get(self.head.para)
            .and_then(|p| self.html.block(p))
            .map(|b| b.tag().to_owned())
            .unwrap_or_default()
    }

    fn selected_paras(&self) -> std::ops::RangeInclusive<usize> {
        let (a, b) = self.ordered();
        a.para..=b.para
    }

    pub(super) fn set_format(&mut self, new_tag: &str) {
        for para in self.selected_paras() {
            let path = self.paras[para].clone();
            if let Some(Kind::Para { tag, .. }) = self.html.block_mut(&path).map(|b| &mut b.kind) {
                if new_tag == "p" && tag.is_empty() {
                    continue;
                }
                *tag = new_tag.to_owned();
            }
        }
    }

    fn style_target(&mut self, para: usize) -> Option<Path> {
        let path = self.paras.get(para)?.clone();
        let block = self.html.block(&path)?;
        if !block.tag().is_empty() {
            return Some(path);
        }
        let up = parent(&path).to_vec();
        let parent_tag = self
            .html
            .block(&up)
            .map(|b| b.tag().to_owned())
            .unwrap_or_default();
        if !up.is_empty()
            && matches!(
                parent_tag.as_str(),
                "li" | "td" | "th" | "div" | "blockquote"
            )
        {
            return Some(up);
        }
        if let Some(Kind::Para { tag, .. }) = self.html.block_mut(&path).map(|b| &mut b.kind) {
            *tag = "p".into();
        }
        Some(path)
    }

    pub(super) fn set_align(&mut self, align: Option<&str>) {
        for para in self.selected_paras() {
            if let Some(path) = self.style_target(para)
                && let Some(attrs) = self.html.block_mut(&path).and_then(Block::attrs_mut)
            {
                attrs.set_style("text-align", align);
            }
        }
    }

    pub(super) fn align(&self) -> String {
        let Some(path) = self.paras.get(self.head.para) else {
            return String::new();
        };
        let mut path = path.clone();
        while !path.is_empty() {
            if let Some(a) = self
                .html
                .block(&path)
                .and_then(Block::attrs)
                .and_then(|a| a.style("text-align"))
            {
                return a;
            }
            path.pop();
        }
        String::new()
    }

    pub(super) fn list_context(&self, para: usize) -> Option<(Path, Path)> {
        let mut path = self.paras.get(para)?.clone();
        while path.len() >= 2 {
            path.pop();
            if self.html.block(&path).is_some_and(|b| b.tag() == "li") {
                let list = parent(&path).to_vec();
                return Some((list, path));
            }
        }
        None
    }

    fn sibling_span(&self) -> Option<(Path, usize, usize)> {
        let (a, b) = self.ordered();
        let pa = self.paras.get(a.para)?.clone();
        let pb = self.paras.get(b.para)?.clone();
        let mut common = 0;
        while common < pa.len().min(pb.len()) - 1 && pa[common] == pb[common] {
            common += 1;
        }
        Some((pa[..common].to_vec(), pa[common], pb[common]))
    }

    pub(super) fn toggle_list(&mut self, ordered: bool) {
        let want = if ordered { "ol" } else { "ul" };
        if let Some((list, _)) = self.list_context(self.head.para) {
            let tag = self
                .html
                .block(&list)
                .map(|b| b.tag().to_owned())
                .unwrap_or_default();
            if tag == want {
                self.unwrap_list(&list);
            } else if let Some(Kind::Box { tag, .. }) =
                self.html.block_mut(&list).map(|b| &mut b.kind)
            {
                *tag = want.to_owned();
            }
            self.paras = self.html.paras();
            self.refresh();
            return;
        }
        let Some((up, from, to)) = self.sibling_span() else {
            return;
        };
        let taken: Vec<Block> = match self.html.list_mut(&up) {
            Some(list) => list.drain(from..=to).collect(),
            None => return,
        };
        let items: Vec<Block> = taken
            .into_iter()
            .map(|block| {
                let content = match block.kind {
                    Kind::Para { runs, .. } => Block::para("", runs),
                    kind => Block::new(kind),
                };
                Block::boxed("li", vec![content])
            })
            .collect();
        if let Some(list) = self.html.list_mut(&up) {
            list.insert(from, Block::boxed(want, items));
        }
        let first = [up.as_slice(), &[from, 0, 0]].concat();
        let (a, b) = self.ordered();
        let forward = self.head >= self.anchor;
        self.paras = self.html.paras();
        let para = self.index_of(&first).unwrap_or(0);
        let start = Pos { para, off: a.off };
        let end = Pos {
            para: para + (b.para - a.para),
            off: b.off,
        };
        (self.anchor, self.head) = if forward { (start, end) } else { (end, start) };
        self.refresh();
    }

    fn unwrap_list(&mut self, list: &[usize]) {
        let Some(block) = self.html.block(list).cloned() else {
            return;
        };
        let Some(items) = block.children() else {
            return;
        };
        let outer = parent(list).to_vec();
        let in_li = self.html.block(&outer).is_some_and(|b| b.tag() == "li");
        let mut out = Vec::new();
        for item in items {
            for child in item.children().cloned().unwrap_or_default() {
                out.push(match child.kind {
                    Kind::Para { tag, runs, .. } if tag.is_empty() => {
                        Block::para(if in_li { "" } else { "p" }, runs)
                    }
                    kind => Block::new(kind),
                });
            }
        }
        let index = *list.last().unwrap_or(&0);
        if let Some(parent_list) = self.html.list_mut(&outer) {
            parent_list.remove(index);
            for (k, b) in out.into_iter().enumerate() {
                parent_list.insert(index + k, b);
            }
        }
    }

    pub(super) fn outdent_li(&mut self, li: Path) {
        let list = parent(&li).to_vec();
        let li_index = *li.last().unwrap_or(&0);
        let outer = parent(&list).to_vec();
        let list_index = *list.last().unwrap_or(&0);
        let Some(list_block) = self.html.block(&list).cloned() else {
            return;
        };
        let tag = list_block.tag().to_owned();
        let mut items = list_block.children().cloned().unwrap_or_default();
        if li_index >= items.len() {
            return;
        }
        let after = items.split_off(li_index + 1);
        let item = items
            .pop()
            .unwrap_or_else(|| Block::boxed("li", Vec::new()));
        let outer_is_li = self.html.block(&outer).is_some_and(|b| b.tag() == "li");
        let mut replacement = Vec::new();
        if !items.is_empty() {
            replacement.push(Block::boxed(&tag, items));
        }
        let moved_path_offset = replacement.len();
        if outer_is_li {
            let mut item = item;
            if !after.is_empty()
                && let Some(children) = item.children_mut()
            {
                children.push(Block::boxed(&tag, after));
            }
            if let Some(list) = self.html.list_mut(&outer) {
                list.remove(list_index);
                for (k, b) in replacement.into_iter().enumerate() {
                    list.insert(list_index + k, b);
                }
            }
            let grand_list = parent(&outer).to_vec();
            let outer_index = *outer.last().unwrap_or(&0);
            if let Some(list) = self.html.list_mut(&grand_list) {
                list.insert(outer_index + 1, item);
            }
            let target = [grand_list.as_slice(), &[outer_index + 1, 0]].concat();
            self.finish_move(target);
            return;
        }
        for child in item.children().cloned().unwrap_or_default() {
            replacement.push(match child.kind {
                Kind::Para { tag, runs, .. } if tag.is_empty() => Block::para("p", runs),
                kind => Block::new(kind),
            });
        }
        if !after.is_empty() {
            replacement.push(Block::boxed(&tag, after));
        }
        if let Some(list) = self.html.list_mut(&outer) {
            list.remove(list_index);
            for (k, b) in replacement.into_iter().enumerate() {
                list.insert(list_index + k, b);
            }
        }
        let target = [outer.as_slice(), &[list_index + moved_path_offset]].concat();
        self.finish_move(target);
    }

    fn finish_move(&mut self, target: Path) {
        let off = self.head.off;
        self.paras = self.html.paras();
        let para = self
            .paras
            .iter()
            .position(|p| p.starts_with(&target))
            .unwrap_or(0);
        self.collapse_to(Pos { para, off });
        self.refresh();
    }

    fn indent_li(&mut self, li: Path) -> bool {
        let li_index = *li.last().unwrap_or(&0);
        if li_index == 0 {
            return false;
        }
        let list = parent(&li).to_vec();
        let tag = self
            .html
            .block(&list)
            .map(|b| b.tag().to_owned())
            .unwrap_or_default();
        let item = match self.html.list_mut(&list) {
            Some(items) => items.remove(li_index),
            None => return false,
        };
        let prev = [list.as_slice(), &[li_index - 1]].concat();
        let Some(children) = self.html.block_mut(&prev).and_then(Block::children_mut) else {
            return false;
        };
        let nested = children.last().is_some_and(|b| is_list(b.tag()));
        if nested {
            if let Some(items) = children.last_mut().and_then(Block::children_mut) {
                items.push(item);
            }
        } else {
            children.push(Block::boxed(&tag, vec![item]));
        }
        let child_index = children.len() - 1;
        let count = children[child_index].children().map_or(1, Vec::len);
        let target = [prev.as_slice(), &[child_index, count - 1]].concat();
        self.finish_move(target);
        true
    }

    pub(super) fn indent(&mut self, more: bool) {
        if let Some((_, li)) = self.list_context(self.head.para) {
            if more {
                if self.indent_li(li) {
                    return;
                }
            } else {
                self.outdent_li(li);
                return;
            }
        }
        for para in self.selected_paras() {
            if let Some(path) = self.style_target(para)
                && let Some(attrs) = self.html.block_mut(&path).and_then(Block::attrs_mut)
            {
                let now = attrs
                    .style("margin-left")
                    .and_then(|v| v.trim_end_matches("px").trim().parse::<i32>().ok())
                    .unwrap_or(0);
                let next = (now + if more { 40 } else { -40 }).max(0);
                let value = format!("{next}px");
                attrs.set_style("margin-left", (next > 0).then_some(value.as_str()));
            }
        }
    }

    pub(super) fn quote_context(&self) -> Option<Path> {
        let mut path = self.paras.get(self.head.para)?.clone();
        while path.len() >= 2 {
            path.pop();
            if self
                .html
                .block(&path)
                .is_some_and(|b| b.tag() == "blockquote")
            {
                return Some(path);
            }
        }
        None
    }

    pub(super) fn toggle_quote(&mut self) {
        if let Some(quote) = self.quote_context() {
            let children = self
                .html
                .block(&quote)
                .and_then(Block::children)
                .cloned()
                .unwrap_or_default();
            let index = *quote.last().unwrap_or(&0);
            let outer = parent(&quote).to_vec();
            if let Some(list) = self.html.list_mut(&outer) {
                list.remove(index);
                for (k, b) in children.into_iter().enumerate() {
                    let b = match b.kind {
                        Kind::Para { tag, runs, .. } if tag.is_empty() => Block::para("p", runs),
                        kind => Block::new(kind),
                    };
                    list.insert(index + k, b);
                }
            }
            let target = [outer.as_slice(), &[index]].concat();
            self.finish_move(target);
            return;
        }
        let Some((up, from, to)) = self.sibling_span() else {
            return;
        };
        let taken: Vec<Block> = match self.html.list_mut(&up) {
            Some(list) => list.drain(from..=to).collect(),
            None => return,
        };
        let taken: Vec<Block> = taken
            .into_iter()
            .map(|b| match b.kind {
                Kind::Para { tag, runs, .. } if tag.is_empty() => Block::para("p", runs),
                kind => Block::new(kind),
            })
            .collect();
        if let Some(list) = self.html.list_mut(&up) {
            list.insert(from, Block::boxed("blockquote", taken));
        }
        let target = [up.as_slice(), &[from]].concat();
        self.finish_move(target);
    }

    pub(super) fn insert_block_after(&mut self, block: Block, then_para: bool) {
        self.collapse_to(self.head);
        let head = self.head;
        let len = self.len(head.para);
        let before = if head.off > 0 && head.off < len {
            self.split();
            true
        } else {
            head.off == 0 && len > 0
        };
        let path = self.paras.get(self.head.para).cloned().unwrap_or_default();
        let up = parent(&path).to_vec();
        let index = *path.last().unwrap_or(&0);
        let at = if before { index } else { index + 1 };
        let filler = if up.is_empty() { "p" } else { "" };
        if let Some(list) = self.html.list_mut(&up) {
            let at = at.min(list.len());
            list.insert(at, block);
            let next_is_para = list
                .get(at + 1)
                .is_some_and(|b| matches!(b.kind, Kind::Para { .. }));
            if then_para && !next_is_para {
                list.insert(at + 1, Block::para(filler, Vec::new()));
            }
        }
        let inserted = [up.as_slice(), &[at]].concat();
        self.paras = self.html.paras();
        let para = self
            .paras
            .iter()
            .position(|p| p.starts_with(&inserted) || p.as_slice() > inserted.as_slice())
            .unwrap_or(self.paras.len().saturating_sub(1));
        self.collapse_to(Pos { para, off: 0 });
        self.refresh();
    }

    pub(super) fn table_context(&self) -> Option<(Path, Vec<Path>, usize, usize)> {
        let mut path = self.paras.get(self.head.para)?.clone();
        let mut cell = None;
        while !path.is_empty() {
            let tag = self
                .html
                .block(&path)
                .map(|b| b.tag().to_owned())
                .unwrap_or_default();
            if (tag == "td" || tag == "th") && cell.is_none() {
                cell = Some(path.clone());
            }
            if tag == "table" {
                break;
            }
            path.pop();
        }
        let cell = cell?;
        if path.is_empty() {
            return None;
        }
        let rows = self.rows(&path);
        let tr = parent(&cell).to_vec();
        let row = rows.iter().position(|r| *r == tr)?;
        Some((path, rows, row, *cell.last().unwrap_or(&0)))
    }

    pub(super) fn rows(&self, table: &[usize]) -> Vec<Path> {
        let mut out = Vec::new();
        let Some(children) = self.html.block(table).and_then(Block::children) else {
            return out;
        };
        for (i, child) in children.iter().enumerate() {
            match child.tag() {
                "tr" => out.push([table, &[i]].concat()),
                "thead" | "tbody" | "tfoot" => {
                    for (k, row) in child.children().into_iter().flatten().enumerate() {
                        if row.tag() == "tr" {
                            out.push([table, &[i, k]].concat());
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub(super) fn table_op(&mut self, op: TableOp) {
        let Some((table, rows, row, col)) = self.table_context() else {
            return;
        };
        let cell = |tag: &str| Block::boxed(tag, vec![Block::para("", Vec::new())]);
        match op {
            TableOp::RowAbove | TableOp::RowBelow => {
                let count = self
                    .html
                    .block(&rows[row])
                    .and_then(Block::children)
                    .map_or(1, Vec::len);
                let new_row = Block::boxed("tr", (0..count).map(|_| cell("td")).collect());
                let at = *rows[row].last().unwrap_or(&0) + usize::from(op == TableOp::RowBelow);
                if let Some(list) = self.html.list_mut(parent(&rows[row])) {
                    list.insert(at, new_row);
                }
            }
            TableOp::ColLeft | TableOp::ColRight => {
                let at = col + usize::from(op == TableOp::ColRight);
                for r in &rows {
                    let header = self
                        .html
                        .block(r)
                        .and_then(Block::children)
                        .and_then(|c| c.first())
                        .is_some_and(|c| c.tag() == "th");
                    if let Some(cells) = self.html.block_mut(r).and_then(Block::children_mut) {
                        let at = at.min(cells.len());
                        cells.insert(at, cell(if header { "th" } else { "td" }));
                    }
                }
            }
            TableOp::DeleteRow => {
                if rows.len() <= 1 {
                    self.delete_table(&table);
                    return;
                }
                if let Some(list) = self.html.list_mut(parent(&rows[row])) {
                    list.remove(*rows[row].last().unwrap_or(&0));
                }
            }
            TableOp::DeleteCol => {
                let mut empty = true;
                for r in &rows {
                    if let Some(cells) = self.html.block_mut(r).and_then(Block::children_mut) {
                        if col < cells.len() {
                            cells.remove(col);
                        }
                        empty &= cells.is_empty();
                    }
                }
                if empty {
                    self.delete_table(&table);
                    return;
                }
            }
            TableOp::DeleteTable => {
                self.delete_table(&table);
                return;
            }
        }
        let off = self.head.off;
        let para = self.head.para;
        self.paras = self.html.paras();
        self.collapse_to(Pos { para, off });
        self.refresh();
    }

    fn delete_table(&mut self, table: &[usize]) {
        let index = *table.last().unwrap_or(&0);
        let up = parent(table).to_vec();
        if let Some(list) = self.html.list_mut(&up) {
            list.remove(index);
            if list.is_empty() {
                list.push(Block::para(
                    if up.is_empty() { "p" } else { "" },
                    Vec::new(),
                ));
            }
        }
        self.paras = self.html.paras();
        let para = self
            .paras
            .iter()
            .position(|p| p.as_slice() >= table)
            .unwrap_or(self.paras.len().saturating_sub(1));
        self.collapse_to(Pos { para, off: 0 });
        self.refresh();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TableOp {
    RowAbove,
    RowBelow,
    ColLeft,
    ColRight,
    DeleteRow,
    DeleteCol,
    DeleteTable,
}

pub fn table_html(rows: usize, cols: usize, header: bool, width: &str, border: &str) -> Block {
    let mut attrs = Attrs::default();
    if !border.trim().is_empty() && border.trim() != "0" {
        attrs.set("border", Some(border.trim().to_owned()));
    }
    attrs.set("cellpadding", Some("4".into()));
    if !width.trim().is_empty() {
        attrs.set_style("width", Some(width.trim()));
        attrs.set_style("border-collapse", Some("collapse"));
    }
    let cell = |tag: &str| Block::boxed(tag, vec![Block::para("", Vec::new())]);
    let mut children = Vec::new();
    if header {
        children.push(Block::boxed(
            "thead",
            vec![Block::boxed("tr", (0..cols).map(|_| cell("th")).collect())],
        ));
    }
    let body_rows = rows.saturating_sub(usize::from(header)).max(1);
    children.push(Block::boxed(
        "tbody",
        (0..body_rows)
            .map(|_| Block::boxed("tr", (0..cols).map(|_| cell("td")).collect()))
            .collect(),
    ));
    Block::new(Kind::Box {
        tag: "table".into(),
        attrs,
        children,
    })
}
