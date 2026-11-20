pub mod model;
pub mod docx;
mod ops;
mod paste;

use std::sync::Arc;

use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, Event, FontFamily, FontId, Galley, Key, Pos2, Rect,
    RichText, Sense, Stroke, TextEdit, Ui,
    text::{CCursor, LayoutJob, TextFormat},
};

use crate::document::{Document, EditKind};
use crate::theme::Palette;
use model::*;
use ops::{TableOp, table_html};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Pos {
    pub para: usize,
    pub off: usize,
}

struct Seg {
    m: usize,
    ml: usize,
    g: usize,
    gl: usize,
}

struct View {
    para: usize,
    origin: Pos2,
    galley: Arc<Galley>,
    segs: Vec<Seg>,
    clip: Rect,
}

enum Deco {
    Bar(Rect),
    Marker(Pos2, String, f32),
    Cell(Rect, bool),
    Raw(Rect, String),
    Hr(Rect),
    Outline(Rect, String),
}

enum Dialog {
    Link { href: String, blank: bool },
    Image { src: String, alt: String, width: String, height: String, replace: Option<Pos> },
    Table { rows: String, cols: String, header: bool, width: String, border: String },
    Anchor(String),
}

pub struct Visual {
    html: Html,
    paras: Vec<Path>,
    version: u64,
    head: Pos,
    anchor: Pos,
    pending: Option<Vec<Mark>>,
    goal_x: Option<f32>,
    blocks: bool,
    views: Vec<View>,
    dialog: Option<Dialog>,
    reveal: bool,
    dragging: bool,
    menu: Option<Pos2>,
    color_menu: Option<(bool, Pos2)>,
    chars_menu: Option<Pos2>,
    pub notice: Option<String>,
    base_dir: Option<std::path::PathBuf>,
}

pub const LIMIT: usize = 3 * 1024 * 1024;

pub fn supports(doc: &Document) -> bool {
    let syntax = doc.syntax.to_ascii_lowercase();
    if ["html", "php", "twig", "xml"].iter().any(|s| syntax.contains(s)) {
        return !syntax.contains("xml") || syntax.contains("html");
    }
    let name = doc.title().to_ascii_lowercase();
    let bare = doc.remote.is_none()
        && doc
            .path
            .as_ref()
            .is_none_or(|p| p.extension().is_none());
    if bare && syntax == "plain text" {
        return true;
    }
    [".html", ".htm", ".xhtml", ".php", ".phtml", ".twig", ".tpl"]
        .iter()
        .any(|e| name.ends_with(e))
}

impl Visual {
    pub fn new(doc: &Document) -> Visual {
        let text = doc.rope.to_string();
        let html = parse(&text);
        let mut visual = Visual {
            paras: html.paras(),
            html,
            version: doc.version,
            head: Pos::default(),
            anchor: Pos::default(),
            pending: None,
            goal_x: None,
            blocks: false,
            views: Vec::new(),
            dialog: None,
            reveal: false,
            dragging: false,
            menu: None,
            color_menu: None,
            chars_menu: None,
            notice: None,
            base_dir: doc.path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf()),
        };
        if visual.paras.is_empty() {
            visual.html.blocks.push(Block::para("p", Vec::new()));
            visual.paras = visual.html.paras();
        }
        visual
    }

    fn sync_from(&mut self, doc: &Document) {
        if doc.version == self.version {
            return;
        }
        let (head, anchor) = (self.head, self.anchor);
        let dialog = self.dialog.take();
        let blocks = self.blocks;
        *self = Visual::new(doc);
        self.blocks = blocks;
        self.dialog = dialog;
        self.head = head;
        self.anchor = anchor;
        self.refresh();
    }

    fn commit(&mut self, doc: &mut Document, kind: EditKind) {
        let new = serialize(&self.html);
        let old = doc.rope.to_string();
        if new == old {
            self.version = doc.version;
            return;
        }
        let prefix = old
            .chars()
            .zip(new.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let old_len = old.chars().count();
        let new_len = new.chars().count();
        let suffix = old
            .chars()
            .rev()
            .zip(new.chars().rev())
            .take(old_len.min(new_len) - prefix)
            .take_while(|(a, b)| a == b)
            .count();
        let middle: String = new.chars().skip(prefix).take(new_len - prefix - suffix).collect();
        doc.replace(prefix..old_len - suffix, &middle, kind);
        self.version = doc.version;
    }

    pub fn ui(&mut self, ui: &mut Ui, doc: &mut Document, pal: &Palette, base: f32, focus: bool) {
        if doc.rope.len_bytes() > LIMIT {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Файл слишком большой для визуального режима (больше 3 МБ).").color(pal.subtext));
            });
            return;
        }
        self.sync_from(doc);
        let id = egui::Id::unique(("tror_visual", doc.id));
        let mut changed: Option<EditKind> = None;
        self.toolbar(ui, pal, doc, &mut changed);
        let status_h = 24.0;
        let height = (ui.available_height() - status_h).max(60.0);
        let size = base + 2.0;
        let frame_rect = ui.available_rect_before_wrap();
        let frame_rect = Rect::from_min_size(frame_rect.min, egui::vec2(frame_rect.width(), height));
        ui.painter().rect_filled(frame_rect, 0.0, pal.base);
        let scroll = egui::ScrollArea::vertical()
            .id_salt(("tror_visual_scroll", doc.id))
            .max_height(height)
            .auto_shrink(false)
            .show(ui, |ui| {
                let width = ui.available_width().max(120.0);
                let origin = ui.cursor().min + egui::vec2(28.0, 18.0);
                let content_w = (width - 56.0).max(80.0);
                let mut decos = Vec::new();
                self.views.clear();
                let clip = ui.clip_rect();
                let bottom = {
                    let mut lay = Lay {
                        ui,
                        pal,
                        size,
                        atoms: &self.html.atoms,
                        views: &mut self.views,
                        decos: &mut decos,
                        blocks: self.blocks,
                        para: 0,
                        clip,
                        align: None,
                    };
                    lay.blocks(&self.html.blocks, origin.x, content_w, origin.y, &mut Vec::new(), 0)
                };
                let total = (bottom - ui.cursor().min.y + 40.0).max(height - 4.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, total), Sense::hover());
                let response = ui.interact(rect, id, Sense::click_and_drag());
                self.paint(ui, pal, &decos, ui.memory(|m| m.has_focus(id)));
                (response, rect)
            });
        let (response, _rect) = scroll.inner;
        if focus && !ui.memory(|m| m.has_focus(id)) && ui.memory(|m| m.focused().is_none()) {
            ui.memory_mut(|m| m.request_focus(id));
        }
        if response.clicked_by(egui::PointerButton::Primary) || response.drag_started() || response.secondary_clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            )
        });
        self.pointer(ui, &response);
        if response.secondary_clicked() && self.table_context().is_some() {
            self.menu = ui.input(|i| i.pointer.interact_pos());
        }
        if ui.memory(|m| m.has_focus(id)) && self.dialog.is_none() {
            self.keyboard(ui, doc, &mut changed);
        }
        self.status(ui, pal);
        self.table_menu(ui, pal, &mut changed);
        self.dialogs(ui, pal, &mut changed);
        if let Some(kind) = changed {
            self.paras = self.html.paras();
            self.refresh();
            self.commit(doc, kind);
            self.reveal = true;
            ui.ctx().request_repaint();
        }
    }

    fn pointer(&mut self, ui: &Ui, response: &egui::Response) {
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let shift = ui.input(|i| i.modifiers.shift);
        if response.double_clicked_by(egui::PointerButton::Primary) {
            if let Some(hit) = self.hit(pos) {
                self.select_word(hit);
            }
            return;
        }
        let primary = response.clicked_by(egui::PointerButton::Primary);
        if response.drag_started_by(egui::PointerButton::Primary) || primary || response.secondary_clicked() && self.anchor == self.head {
            if let Some(hit) = self.hit(pos) {
                self.head = hit;
                if !shift {
                    self.anchor = hit;
                }
                self.pending = None;
                self.goal_x = None;
            }
            self.dragging = response.drag_started();
        }
        if self.dragging && response.dragged() {
            if let Some(hit) = self.hit(pos) {
                self.head = hit;
            }
            self.reveal = true;
        }
        if response.drag_stopped() {
            self.dragging = false;
        }
    }

    fn select_word(&mut self, at: Pos) {
        let Some(runs) = self.runs(at.para) else {
            return;
        };
        let text: Vec<char> = plain(runs).chars().collect();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut start = at.off.min(text.len());
        while start > 0 && word(text[start - 1]) {
            start -= 1;
        }
        let mut end = at.off.min(text.len());
        while end < text.len() && word(text[end]) {
            end += 1;
        }
        self.anchor = Pos { para: at.para, off: start };
        self.head = Pos { para: at.para, off: end };
    }

    fn hit(&self, pos: Pos2) -> Option<Pos> {
        let mut best: Option<(f32, &View)> = None;
        for view in &self.views {
            let rect = view.clip;
            let inside_x = pos.x >= view.clip.left() && pos.x <= view.clip.right();
            let dy = if pos.y < rect.top() {
                rect.top() - pos.y
            } else if pos.y > rect.bottom() {
                pos.y - rect.bottom()
            } else {
                0.0
            };
            let dx = if inside_x {
                0.0
            } else {
                (pos.x - view.clip.center().x).abs() / 4.0
            };
            let score = dy + dx;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, view));
            }
        }
        let (_, view) = best?;
        let cursor = view.galley.cursor_from_pos(pos - view.origin);
        Some(Pos {
            para: view.para,
            off: g2m(&view.segs, cursor.index.0),
        })
    }

    fn view(&self, para: usize) -> Option<&View> {
        self.views.iter().find(|v| v.para == para)
    }

    fn caret_rect(&self, pos: Pos) -> Option<Rect> {
        let view = self.view(pos.para)?;
        let g = m2g(&view.segs, pos.off);
        let mut rect = view.galley.pos_from_cursor(CCursor::new(g));
        if rect.height() < 4.0 {
            rect.max.y = rect.min.y + 18.0;
        }
        Some(rect.translate(view.origin.to_vec2()))
    }

    fn paint(&mut self, ui: &Ui, pal: &Palette, decos: &[Deco], focused: bool) {
        let painter = ui.painter();
        for deco in decos {
            match deco {
                Deco::Bar(rect) => {
                    painter.rect_filled(*rect, 1.0, pal.border);
                }
                Deco::Marker(pos, text, size) => {
                    painter.text(*pos, egui::Align2::RIGHT_TOP, text, FontId::proportional(*size), pal.subtext);
                }
                Deco::Cell(rect, header) => {
                    if *header {
                        painter.rect_filled(*rect, 0.0, pal.mantle);
                    }
                    painter.rect_stroke(*rect, 0.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                }
                Deco::Raw(rect, text) => {
                    painter.rect_filled(*rect, 4.0, pal.mantle);
                    painter.rect_stroke(*rect, 4.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                    painter.text(
                        rect.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        text,
                        FontId::monospace(12.0),
                        pal.faint,
                    );
                }
                Deco::Hr(rect) => {
                    painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, pal.subtext));
                }
                Deco::Outline(rect, tag) => {
                    painter.rect_stroke(
                        *rect,
                        2.0,
                        Stroke::new(1.0, pal.faint.gamma_multiply(0.7)),
                        egui::StrokeKind::Outside,
                    );
                    painter.text(
                        rect.left_top() + egui::vec2(2.0, -1.0),
                        egui::Align2::LEFT_BOTTOM,
                        tag,
                        FontId::monospace(9.0),
                        pal.faint,
                    );
                }
            }
        }
        let (a, b) = self.ordered();
        if a != b {
            for view in &self.views {
                if view.para < a.para || view.para > b.para {
                    continue;
                }
                let from = if view.para == a.para { m2g(&view.segs, a.off) } else { 0 };
                let to = if view.para == b.para {
                    m2g(&view.segs, b.off)
                } else {
                    view.galley.text().chars().count()
                };
                paint_selection(painter, view, from, to, pal.selection);
            }
        }
        for view in &self.views {
            painter.with_clip_rect(view.clip.expand(2.0)).galley(view.origin, view.galley.clone(), pal.text);
        }
        if focused && let Some(rect) = self.caret_rect(self.head) {
            painter.vline(rect.left(), rect.y_range(), Stroke::new(1.6, pal.accent));
            if self.reveal {
                ui.scroll_to_rect(rect.expand(30.0), None);
                self.reveal = false;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, pal: &Palette, doc: &mut Document, changed: &mut Option<EditKind>) {
        let frame = egui::Frame::new()
            .fill(pal.mantle)
            .inner_margin(egui::Margin::symmetric(8, 5));
        frame.show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 4.0);
                let sep = |ui: &mut Ui| {
                    ui.add_space(4.0);
                    let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, 20.0), Sense::hover());
                    ui.painter().rect_filled(r, 0.0, pal.border);
                    ui.add_space(4.0);
                };
                if icon_tool(ui, pal, Icon::Undo, "Отменить (Ctrl+Z)", false) && doc.can_undo() {
                    doc.undo();
                }
                if icon_tool(ui, pal, Icon::Redo, "Повторить (Ctrl+Y)", false) && doc.can_redo() {
                    doc.redo();
                }
                sep(ui);
                let styles = [
                    (Style::Bold, "B", "Полужирный (Ctrl+B)"),
                    (Style::Italic, "I", "Курсив (Ctrl+I)"),
                    (Style::Underline, "U", "Подчёркнутый (Ctrl+U)"),
                    (Style::Strike, "S", "Зачёркнутый"),
                    (Style::Sub, "x₂", "Подстрочный индекс"),
                    (Style::Sup, "x²", "Надстрочный индекс"),
                ];
                for (style, label, hint) in styles {
                    if style_tool(ui, pal, label, hint, style, self.style_active(style)) {
                        self.toggle_style(style);
                        *changed = Some(EditKind::Other);
                    }
                }
                if tool(ui, pal, "Tx", "Убрать форматирование", false) {
                    self.clear_format();
                    *changed = Some(EditKind::Other);
                }
                sep(ui);
                let list = self
                    .list_context(self.head.para)
                    .and_then(|(l, _)| self.html.block(&l).map(|b| b.tag().to_owned()));
                if icon_tool(ui, pal, Icon::Ordered, "Нумерованный список", list.as_deref() == Some("ol")) {
                    self.toggle_list(true);
                    *changed = Some(EditKind::Other);
                }
                if icon_tool(ui, pal, Icon::Bullets, "Маркированный список", list.as_deref() == Some("ul")) {
                    self.toggle_list(false);
                    *changed = Some(EditKind::Other);
                }
                if tool(ui, pal, "⇤", "Уменьшить отступ (Shift+Tab)", false) {
                    self.indent(false);
                    *changed = Some(EditKind::Other);
                }
                if tool(ui, pal, "⇥", "Увеличить отступ (Tab)", false) {
                    self.indent(true);
                    *changed = Some(EditKind::Other);
                }
                if icon_tool(ui, pal, Icon::Quote, "Цитата", self.quote_context().is_some()) {
                    self.toggle_quote();
                    *changed = Some(EditKind::Other);
                }
                sep(ui);
                let align = self.align();
                for (value, label, hint) in [
                    (None, Icon::Left, "По левому краю"),
                    (Some("center"), Icon::Center, "По центру"),
                    (Some("right"), Icon::Right, "По правому краю"),
                    (Some("justify"), Icon::Justify, "По ширине"),
                ] {
                    let active = match value {
                        None => align.is_empty() || align == "left",
                        Some(v) => align == v,
                    };
                    if icon_tool(ui, pal, label, hint, active) {
                        self.set_align(value);
                        *changed = Some(EditKind::Other);
                    }
                }
                sep(ui);
                let link = self.link_at_cursor();
                if icon_tool(ui, pal, Icon::Link, "Ссылка (Ctrl+K)", link.is_some()) {
                    self.open_link();
                }
                if icon_tool(ui, pal, Icon::Unlink, "Убрать ссылку", false) {
                    self.unlink();
                    *changed = Some(EditKind::Other);
                }
                if icon_tool(ui, pal, Icon::Anchor, "Якорь", false) {
                    self.dialog = Some(Dialog::Anchor(String::new()));
                }
                sep(ui);
                if icon_tool(ui, pal, Icon::Image, "Изображение", false) {
                    self.dialog = Some(Dialog::Image {
                        src: String::new(),
                        alt: String::new(),
                        width: String::new(),
                        height: String::new(),
                        replace: None,
                    });
                }
                if icon_tool(ui, pal, Icon::Table, "Таблица", self.table_context().is_some()) {
                    self.dialog = Some(Dialog::Table {
                        rows: "3".into(),
                        cols: "3".into(),
                        header: true,
                        width: "100%".into(),
                        border: "1".into(),
                    });
                }
                if tool(ui, pal, "―", "Горизонтальная линия", false) {
                    self.insert_block_after(Block::new(Kind::Raw("<hr>".into())), true);
                    *changed = Some(EditKind::Other);
                }
                if tool(ui, pal, "W", "Вставить документ Word (.docx)…", false)
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Документ Word", &["docx"])
                        .pick_file()
                {
                    match docx::to_html(&path) {
                        Ok(html) => {
                            if self.insert_html(&html) {
                                *changed = Some(EditKind::Other);
                            }
                        }
                        Err(e) => self.notice = Some(format!("Не удалось открыть {}: {e}", path.display())),
                    }
                }
                let chars = tool(ui, pal, "Ω", "Специальный символ", false);
                if chars {
                    self.chars_menu = ui.ctx().pointer_latest_pos();
                }
                sep(ui);
                self.format_combo(ui, changed);
                self.size_combo(ui, changed);
                self.font_combo(ui, changed);
                let text_color = icon_tool(ui, pal, Icon::Color, "Цвет текста", false);
                if text_color {
                    self.color_menu = ui.ctx().pointer_latest_pos().map(|p| (false, p));
                }
                let bg_color = icon_tool(ui, pal, Icon::Fill, "Цвет фона", false);
                if bg_color {
                    self.color_menu = ui.ctx().pointer_latest_pos().map(|p| (true, p));
                }
                sep(ui);
                if icon_tool(ui, pal, Icon::Blocks, "Показать блоки", self.blocks) {
                    self.blocks = !self.blocks;
                }
            });
        });
        self.color_popup(ui, pal, changed);
        self.chars_popup(ui, pal, changed);
    }

    fn format_combo(&mut self, ui: &mut Ui, changed: &mut Option<EditKind>) {
        let current = self.current_block_tag();
        let formats = [
            ("p", "Обычный"),
            ("h1", "Заголовок 1"),
            ("h2", "Заголовок 2"),
            ("h3", "Заголовок 3"),
            ("h4", "Заголовок 4"),
            ("h5", "Заголовок 5"),
            ("h6", "Заголовок 6"),
            ("pre", "Форматированный"),
            ("address", "Адрес"),
        ];
        let label = formats
            .iter()
            .find(|(t, _)| *t == current)
            .map_or("Обычный", |(_, l)| *l);
        egui::ComboBox::from_id_salt("tror_visual_format")
            .width(130.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (tag, name) in formats {
                    if ui.selectable_label(current == tag, name).clicked() {
                        self.set_format(tag);
                        *changed = Some(EditKind::Other);
                    }
                }
            });
    }

    fn size_combo(&mut self, ui: &mut Ui, changed: &mut Option<EditKind>) {
        let current = self
            .typing_marks()
            .iter()
            .find_map(|m| (m.tag == "span").then(|| m.attrs.style("font-size")).flatten())
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("tror_visual_size")
            .width(62.0)
            .selected_text(if current.is_empty() { "Размер".to_owned() } else { current.clone() })
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_empty(), "Обычный").clicked() {
                    self.set_span_style("font-size", None);
                    *changed = Some(EditKind::Other);
                }
                for px in [8, 10, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 72] {
                    let value = format!("{px}px");
                    if ui.selectable_label(current == value, &value).clicked() {
                        self.set_span_style("font-size", Some(&value));
                        *changed = Some(EditKind::Other);
                    }
                }
            });
    }

    fn font_combo(&mut self, ui: &mut Ui, changed: &mut Option<EditKind>) {
        let current = self
            .typing_marks()
            .iter()
            .find_map(|m| (m.tag == "span").then(|| m.attrs.style("font-family")).flatten())
            .unwrap_or_default();
        let shown = current.split(',').next().unwrap_or("").trim_matches(['\'', '"']).to_owned();
        egui::ComboBox::from_id_salt("tror_visual_font")
            .width(96.0)
            .selected_text(if shown.is_empty() { "Шрифт".to_owned() } else { shown })
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_empty(), "Обычный").clicked() {
                    self.set_span_style("font-family", None);
                    *changed = Some(EditKind::Other);
                }
                for (name, value) in [
                    ("Arial", "Arial, Helvetica, sans-serif"),
                    ("Georgia", "Georgia, serif"),
                    ("Tahoma", "Tahoma, Geneva, sans-serif"),
                    ("Times New Roman", "'Times New Roman', Times, serif"),
                    ("Trebuchet MS", "'Trebuchet MS', Helvetica, sans-serif"),
                    ("Verdana", "Verdana, Geneva, sans-serif"),
                    ("Courier New", "'Courier New', Courier, monospace"),
                ] {
                    if ui.selectable_label(current == value, name).clicked() {
                        self.set_span_style("font-family", Some(value));
                        *changed = Some(EditKind::Other);
                    }
                }
            });
    }

    fn color_popup(&mut self, ui: &mut Ui, pal: &Palette, changed: &mut Option<EditKind>) {
        let Some((background, pos)) = self.color_menu else {
            return;
        };
        let prop = if background { "background-color" } else { "color" };
        let colors = [
            "#000000", "#444444", "#666666", "#999999", "#cccccc", "#eeeeee", "#ffffff", "#c0392b",
            "#e74c3c", "#d35400", "#e67e22", "#f1c40f", "#27ae60", "#2ecc71", "#16a085", "#1abc9c",
            "#2980b9", "#3498db", "#8e44ad", "#9b59b6", "#2c3e50", "#7f8c8d", "#f39c12", "#ff00ff",
        ];
        let mut close = false;
        let area = egui::Area::new(egui::Id::unique("tror_visual_colors"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos + egui::vec2(-10.0, 14.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    if ui.button("Автоматически").clicked() {
                        self.set_span_style(prop, None);
                        *changed = Some(EditKind::Other);
                        close = true;
                    }
                    egui::Grid::new("tror_visual_color_grid").spacing([3.0, 3.0]).show(ui, |ui| {
                        for (i, hex) in colors.iter().enumerate() {
                            let color = parse_color(hex).unwrap_or(pal.text);
                            let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::click());
                            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                            ui.painter().rect_filled(rect, 3.0, color);
                            ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                            if response.on_hover_text(*hex).clicked() {
                                self.set_span_style(prop, Some(hex));
                                *changed = Some(EditKind::Other);
                                close = true;
                            }
                            if i % 8 == 7 {
                                ui.end_row();
                            }
                        }
                    });
                });
            });
        let outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && (p - pos).length() > 12.0);
        if close || outside || ui.input(|i| i.key_pressed(Key::Escape)) {
            self.color_menu = None;
        }
    }

    fn chars_popup(&mut self, ui: &mut Ui, pal: &Palette, changed: &mut Option<EditKind>) {
        let Some(pos) = self.chars_menu else {
            return;
        };
        let chars = "©®™°±×÷€£¥§¶•…—–«»„“”‘’←→↑↓½¼¾¹²³µ‰∞≈≠≤≥√∑№";
        let mut close = false;
        let area = egui::Area::new(egui::Id::unique("tror_visual_chars"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos + egui::vec2(-10.0, 14.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    egui::Grid::new("tror_visual_char_grid").spacing([2.0, 2.0]).show(ui, |ui| {
                        for (i, c) in chars.chars().enumerate() {
                            let button = Button::new(RichText::new(c.to_string()).size(15.0).color(pal.text))
                                .min_size(egui::vec2(26.0, 24.0));
                            if ui.add(button).clicked() {
                                self.insert_text(&c.to_string());
                                *changed = Some(EditKind::Other);
                                close = true;
                            }
                            if i % 10 == 9 {
                                ui.end_row();
                            }
                        }
                    });
                    if ui.button("Неразрывный пробел").clicked() {
                        self.insert_text("\u{a0}");
                        *changed = Some(EditKind::Other);
                        close = true;
                    }
                });
            });
        let outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && (p - pos).length() > 12.0);
        if close || outside || ui.input(|i| i.key_pressed(Key::Escape)) {
            self.chars_menu = None;
        }
    }

    fn open_link(&mut self) {
        let (href, blank) = match self.link_at_cursor() {
            Some((_, _, mark)) => (
                mark.attrs.get("href").unwrap_or_default().to_owned(),
                mark.attrs.get("target") == Some("_blank"),
            ),
            None => ("https://".to_owned(), false),
        };
        self.dialog = Some(Dialog::Link { href, blank });
    }

    fn table_menu(&mut self, ui: &mut Ui, _pal: &Palette, changed: &mut Option<EditKind>) {
        let Some(pos) = self.menu else {
            return;
        };
        let mut op = None;
        let area = egui::Area::new(egui::Id::unique("tror_visual_table_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    for (label, value) in [
                        ("Вставить строку сверху", TableOp::RowAbove),
                        ("Вставить строку снизу", TableOp::RowBelow),
                        ("Вставить столбец слева", TableOp::ColLeft),
                        ("Вставить столбец справа", TableOp::ColRight),
                    ] {
                        if ui.button(label).clicked() {
                            op = Some(value);
                        }
                    }
                    ui.separator();
                    for (label, value) in [
                        ("Удалить строку", TableOp::DeleteRow),
                        ("Удалить столбец", TableOp::DeleteCol),
                        ("Удалить таблицу", TableOp::DeleteTable),
                    ] {
                        if ui.button(label).clicked() {
                            op = Some(value);
                        }
                    }
                });
            });
        if let Some(op) = op {
            self.table_op(op);
            *changed = Some(EditKind::Other);
            self.menu = None;
            return;
        }
        let outside = ui.input(|i| i.pointer.any_pressed() && !i.pointer.secondary_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p));
        if outside || ui.input(|i| i.key_pressed(Key::Escape)) {
            self.menu = None;
        }
    }

    fn dialogs(&mut self, ui: &mut Ui, pal: &Palette, changed: &mut Option<EditKind>) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let base = self.base_dir.clone();
        let modal = crate::app::dialog("tror_visual_dialog", pal).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            let row = |ui: &mut Ui, label: &str, value: &mut String| {
                ui.horizontal(|ui| {
                    ui.add_sized([110.0, 24.0], egui::Label::new(label));
                    ui.add(TextEdit::singleline(value).desired_width(f32::INFINITY));
                });
            };
            match dialog {
                Dialog::Link { href, blank } => {
                    ui.label(RichText::new("Ссылка").strong());
                    ui.add_space(6.0);
                    row(ui, "Адрес", href);
                    ui.checkbox(blank, "Открывать в новом окне");
                }
                Dialog::Image { src, alt, width, height, .. } => {
                    ui.label(RichText::new("Изображение").strong());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_sized([110.0, 24.0], egui::Label::new("Адрес"));
                        ui.add(TextEdit::singleline(src).desired_width(220.0));
                        if ui.button("Выбрать…").clicked() {
                            let mut picker = rfd::FileDialog::new().add_filter(
                                "Изображения",
                                &["png", "jpg", "jpeg", "gif", "webp", "svg"],
                            );
                            if let Some(dir) = &base {
                                picker = picker.set_directory(dir);
                            }
                            if let Some(file) = picker.pick_file() {
                                *src = relative(base.as_deref(), &file);
                            }
                        }
                    });
                    row(ui, "Описание (alt)", alt);
                    row(ui, "Ширина", width);
                    row(ui, "Высота", height);
                }
                Dialog::Table { rows, cols, header, width, border } => {
                    ui.label(RichText::new("Таблица").strong());
                    ui.add_space(6.0);
                    row(ui, "Строк", rows);
                    row(ui, "Столбцов", cols);
                    row(ui, "Ширина", width);
                    row(ui, "Рамка", border);
                    ui.checkbox(header, "Первая строка — заголовок");
                }
                Dialog::Anchor(name) => {
                    ui.label(RichText::new("Якорь").strong());
                    ui.add_space(6.0);
                    row(ui, "Имя", name);
                }
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                ok = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if crate::app::dialog_button(ui, pal, "OK", true) {
                    ok = true;
                }
                if crate::app::dialog_button(ui, pal, "Отмена", false) {
                    cancel = true;
                }
            });
        });
        if ok {
            match self.dialog.take() {
                Some(Dialog::Link { href, blank }) => {
                    if !href.trim().is_empty() && href.trim() != "https://" {
                        self.apply_link(href.trim(), blank);
                        *changed = Some(EditKind::Other);
                    }
                }
                Some(Dialog::Image { src, alt, width, height, replace }) => {
                    if !src.trim().is_empty() {
                        let mut attrs = Attrs::default();
                        attrs.set("src", Some(src.trim().to_owned()));
                        attrs.set("alt", Some(alt.trim().to_owned()));
                        if !width.trim().is_empty() {
                            attrs.set("width", Some(width.trim().to_owned()));
                        }
                        if !height.trim().is_empty() {
                            attrs.set("height", Some(height.trim().to_owned()));
                        }
                        let mut html = String::from("<img");
                        for (k, v) in &attrs.0 {
                            html.push_str(&format!(" {k}=\"{}\"", v.clone().unwrap_or_default().replace('"', "&quot;")));
                        }
                        html.push('>');
                        if let Some(at) = replace {
                            self.anchor = at;
                            self.head = Pos { para: at.para, off: at.off + 1 };
                        }
                        self.insert_object(html);
                        *changed = Some(EditKind::Other);
                    }
                }
                Some(Dialog::Table { rows, cols, header, width, border }) => {
                    let rows = rows.trim().parse().unwrap_or(3).clamp(1, 100);
                    let cols = cols.trim().parse().unwrap_or(3).clamp(1, 30);
                    self.insert_block_after(table_html(rows, cols, header, &width, &border), true);
                    *changed = Some(EditKind::Other);
                }
                Some(Dialog::Anchor(name)) => {
                    let name = name.trim().replace([' ', '"', '<', '>'], "-");
                    if !name.is_empty() {
                        self.insert_object(format!("<a id=\"{name}\" name=\"{name}\"></a>"));
                        *changed = Some(EditKind::Other);
                    }
                }
                None => {}
            }
        } else if cancel || modal.should_close() {
            self.dialog = None;
        }
    }

    fn status(&self, ui: &mut Ui, pal: &Palette) {
        let mut parts = vec!["body".to_owned()];
        if let Some(path) = self.paras.get(self.head.para) {
            for depth in 1..=path.len() {
                if let Some(block) = self.html.block(&path[..depth])
                    && !block.tag().is_empty()
                {
                    parts.push(block.tag().to_owned());
                }
            }
            if let Some(runs) = self.runs(self.head.para) {
                for mark in marks_at(runs, self.head.off) {
                    parts.push(mark.tag.clone());
                }
            }
        }
        egui::Frame::new()
            .fill(pal.mantle)
            .inner_margin(egui::Margin::symmetric(10, 3))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(parts.join(" › ")).small().color(pal.subtext));
            });
    }

    fn keyboard(&mut self, ui: &Ui, doc: &mut Document, changed: &mut Option<EditKind>) {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                Event::Text(text) => {
                    if text.chars().all(|c| !c.is_control()) {
                        self.insert_text(&text);
                        *changed = Some(EditKind::Insert);
                    }
                }
                Event::Copy | Event::Cut => {
                    let text = self.selected_text();
                    if !text.is_empty() {
                        if !paste::write_html(&self.selected_html(), &text) {
                            ui.ctx().copy_text(text);
                        }
                        if matches!(event, Event::Cut) {
                            self.delete_selection();
                            *changed = Some(EditKind::Other);
                        }
                    }
                }
                Event::Paste(text) => {
                    let rich = paste::read_html().is_some_and(|html| self.insert_html(&paste::clean(&html)));
                    if !rich {
                        self.insert_text(&text);
                    }
                    *changed = Some(EditKind::Other);
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => self.key(ui, doc, key, modifiers, changed),
                _ => {}
            }
        }
    }

    fn key(&mut self, ui: &Ui, doc: &mut Document, key: Key, mods: egui::Modifiers, changed: &mut Option<EditKind>) {
        let shift = mods.shift;
        let word = mods.command;
        match key {
            Key::Enter => {
                if shift {
                    self.line_break();
                } else {
                    self.split();
                }
                *changed = Some(EditKind::Other);
            }
            Key::Backspace => {
                self.backspace();
                *changed = Some(EditKind::Other);
            }
            Key::Delete => {
                self.delete_forward();
                *changed = Some(EditKind::Other);
            }
            Key::Tab => {
                self.indent(!shift);
                *changed = Some(EditKind::Other);
            }
            Key::B if word => {
                self.toggle_style(Style::Bold);
                *changed = Some(EditKind::Other);
            }
            Key::I if word => {
                self.toggle_style(Style::Italic);
                *changed = Some(EditKind::Other);
            }
            Key::U if word => {
                self.toggle_style(Style::Underline);
                *changed = Some(EditKind::Other);
            }
            Key::K if word => self.open_link(),
            Key::A if word => {
                self.anchor = Pos::default();
                let last = self.paras.len().saturating_sub(1);
                self.head = Pos { para: last, off: self.len(last) };
            }
            Key::Z if word => {
                if shift {
                    doc.redo();
                } else {
                    doc.undo();
                }
            }
            Key::Y if word => doc.redo(),
            Key::ArrowLeft | Key::ArrowRight => {
                let right = key == Key::ArrowRight;
                let (a, b) = self.ordered();
                if !shift && a != b {
                    self.collapse_to(if right { b } else { a });
                    return;
                }
                let mut pos = self.head;
                if right {
                    if pos.off < self.len(pos.para) {
                        pos.off = if word { self.word_edge(pos, true) } else { pos.off + 1 };
                    } else if pos.para + 1 < self.paras.len() {
                        pos = Pos { para: pos.para + 1, off: 0 };
                    }
                } else if pos.off > 0 {
                    pos.off = if word { self.word_edge(pos, false) } else { pos.off - 1 };
                } else if pos.para > 0 {
                    pos = Pos { para: pos.para - 1, off: self.len(pos.para - 1) };
                }
                self.move_head(pos, shift);
            }
            Key::ArrowUp | Key::ArrowDown => {
                if let Some(pos) = self.vertical(key == Key::ArrowDown) {
                    let goal = self.goal_x;
                    self.move_head(pos, shift);
                    self.goal_x = goal;
                }
            }
            Key::Home | Key::End => {
                let end = key == Key::End;
                let pos = if word {
                    if end {
                        let last = self.paras.len().saturating_sub(1);
                        Pos { para: last, off: self.len(last) }
                    } else {
                        Pos::default()
                    }
                } else {
                    self.row_edge(end)
                };
                self.move_head(pos, shift);
            }
            Key::PageUp | Key::PageDown => {
                let mut pos = self.head;
                for _ in 0..15 {
                    if let Some(next) = self.vertical_from(pos, key == Key::PageDown) {
                        pos = next;
                    }
                }
                self.move_head(pos, shift);
            }
            _ => {}
        }
        let _ = ui;
    }

    fn move_head(&mut self, pos: Pos, extend: bool) {
        self.head = pos;
        if !extend {
            self.anchor = pos;
        }
        self.pending = None;
        self.goal_x = None;
        self.reveal = true;
    }

    fn word_edge(&self, pos: Pos, right: bool) -> usize {
        let text: Vec<char> = self.runs(pos.para).map(|r| plain(r)).unwrap_or_default().chars().collect();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut i = pos.off.min(text.len());
        if right {
            while i < text.len() && !word(text[i]) {
                i += 1;
            }
            while i < text.len() && word(text[i]) {
                i += 1;
            }
        } else {
            while i > 0 && !word(text[i - 1]) {
                i -= 1;
            }
            while i > 0 && word(text[i - 1]) {
                i -= 1;
            }
        }
        i
    }

    fn row_edge(&self, end: bool) -> Pos {
        let pos = self.head;
        let Some(view) = self.view(pos.para) else {
            return pos;
        };
        let cursor = CCursor::new(m2g(&view.segs, pos.off));
        let target = if end {
            view.galley.cursor_end_of_row(&cursor)
        } else {
            view.galley.cursor_begin_of_row(&cursor)
        };
        Pos { para: pos.para, off: g2m(&view.segs, target.index.0) }
    }

    fn vertical(&mut self, down: bool) -> Option<Pos> {
        if self.goal_x.is_none() {
            self.goal_x = self.caret_rect(self.head).map(|r| r.left());
        }
        self.vertical_from(self.head, down)
    }

    fn vertical_from(&self, pos: Pos, down: bool) -> Option<Pos> {
        let view = self.view(pos.para)?;
        let caret = self.caret_rect(pos)?;
        let x = self.goal_x.unwrap_or(caret.left());
        let local_y = if down { caret.bottom() + 4.0 } else { caret.top() - 4.0 } - view.origin.y;
        if local_y >= 0.0 && local_y <= view.galley.size().y {
            let c = view.galley.cursor_from_pos(egui::vec2(x - view.origin.x, local_y));
            return Some(Pos { para: pos.para, off: g2m(&view.segs, c.index.0) });
        }
        let next = if down {
            self.views.iter().filter(|v| v.origin.y > view.origin.y + 1.0).min_by(|a, b| {
                let da = (a.origin.y, (a.clip.center().x - x).abs());
                let db = (b.origin.y, (b.clip.center().x - x).abs());
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
        } else {
            self.views
                .iter()
                .filter(|v| v.origin.y + v.galley.size().y < view.origin.y - 1.0)
                .max_by(|a, b| {
                    let da = (a.origin.y + a.galley.size().y, -(a.clip.center().x - x).abs());
                    let db = (b.origin.y + b.galley.size().y, -(b.clip.center().x - x).abs());
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
        };
        let Some(next) = next else {
            return Some(if down {
                Pos { para: pos.para, off: self.len(pos.para) }
            } else {
                Pos { para: pos.para, off: 0 }
            });
        };
        let y = if down { 2.0 } else { next.galley.size().y - 2.0 };
        let c = next.galley.cursor_from_pos(egui::vec2(x - next.origin.x, y.max(0.0)));
        Some(Pos { para: next.para, off: g2m(&next.segs, c.index.0) })
    }
}

fn paint_selection(painter: &egui::Painter, view: &View, from: usize, to: usize, color: Color32) {
    if to <= from {
        let rect = view.galley.pos_from_cursor(CCursor::new(from));
        let r = Rect::from_min_size(rect.min, egui::vec2(6.0, rect.height().max(14.0)));
        painter.rect_filled(r.translate(view.origin.to_vec2()), 0.0, color);
        return;
    }
    let mut start = from;
    while start < to {
        let a = view.galley.pos_from_cursor(CCursor::new(start));
        let row_end = view.galley.cursor_end_of_row(&CCursor::new(start)).index.0;
        let end = row_end.min(to).max(start + 1).min(to);
        let b = view.galley.pos_from_cursor(CCursor { index: egui::text::CharIndex(end), prefer_next_row: false });
        let mut right = b.left();
        if end == row_end && end < to {
            right += 6.0;
        }
        let rect = Rect::from_min_max(a.min, egui::pos2(right.max(a.min.x + 2.0), a.max.y));
        painter.rect_filled(rect.translate(view.origin.to_vec2()), 0.0, color);
        if end <= start {
            break;
        }
        start = end;
        if end == row_end {
            let next = view.galley.cursor_from_pos(egui::vec2(0.0, a.max.y + 2.0)).index.0;
            if next <= start {
                break;
            }
            start = next;
        }
    }
}

fn m2g(segs: &[Seg], off: usize) -> usize {
    for seg in segs {
        if off <= seg.m {
            return seg.g;
        }
        if off < seg.m + seg.ml {
            return if seg.ml == seg.gl { seg.g + (off - seg.m) } else { seg.g + seg.gl };
        }
    }
    segs.last().map_or(off, |s| s.g + s.gl + off.saturating_sub(s.m + s.ml))
}

fn g2m(segs: &[Seg], g: usize) -> usize {
    for seg in segs {
        if g <= seg.g {
            return seg.m;
        }
        if g < seg.g + seg.gl {
            return if seg.ml == seg.gl {
                seg.m + (g - seg.g)
            } else if g - seg.g < seg.gl.div_ceil(2) {
                seg.m
            } else {
                seg.m + seg.ml
            };
        }
    }
    segs.last().map_or(g, |s| s.m + s.ml)
}

fn relative(base: Option<&std::path::Path>, file: &std::path::Path) -> String {
    if let Some(base) = base
        && let Ok(rest) = file.strip_prefix(base)
    {
        return rest.to_string_lossy().replace('\\', "/");
    }
    file.to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Undo,
    Redo,
    Ordered,
    Bullets,
    Quote,
    Left,
    Center,
    Right,
    Justify,
    Link,
    Unlink,
    Anchor,
    Image,
    Table,
    Blocks,
    Color,
    Fill,
}

fn icon_tool(ui: &mut Ui, pal: &Palette, icon: Icon, hint: &str, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect.expand(1.0));
    if active {
        painter.rect_filled(rect, 5.0, pal.surface);
    } else if response.hovered() {
        painter.rect_filled(rect, 5.0, pal.surface_hover.gamma_multiply(0.6));
    }
    let color = if active { pal.accent } else { pal.text };
    let stroke = Stroke::new(1.5, color);
    let c = rect.center();
    let p = |x: f32, y: f32| c + egui::vec2(x, y);
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
    match icon {
        Icon::Undo | Icon::Redo => {
            let dir = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let points: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = std::f32::consts::PI * (i as f32 / 12.0);
                    p(dir * (1.0 + 5.0 * t.cos()), 2.5 - 5.0 * t.sin())
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            let tip = p(dir * -4.0, 2.5);
            line(tip, tip + egui::vec2(-3.0, -3.0));
            line(tip, tip + egui::vec2(3.0, -3.0));
        }
        Icon::Ordered | Icon::Bullets => {
            for (k, y) in [-4.0f32, 0.0, 4.0].iter().enumerate() {
                line(p(-1.0, *y), p(7.0, *y));
                if icon == Icon::Bullets {
                    painter.circle_filled(p(-5.0, *y), 1.4, color);
                } else {
                    painter.text(
                        p(-5.0, *y),
                        egui::Align2::CENTER_CENTER,
                        (k + 1).to_string(),
                        FontId::proportional(7.5),
                        color,
                    );
                }
            }
        }
        Icon::Quote => {
            for x in [-3.5f32, 3.5] {
                painter.circle_filled(p(x, -1.5), 2.6, color);
                line(p(x + 2.4, -1.0), p(x - 1.0, 5.0));
            }
        }
        Icon::Left | Icon::Center | Icon::Right | Icon::Justify => {
            for (k, y) in [-6.0f32, -2.0, 2.0, 6.0].iter().enumerate() {
                let w = if icon == Icon::Justify || k % 2 == 0 { 14.0 } else { 9.0 };
                let x0 = match icon {
                    Icon::Center => -w / 2.0,
                    Icon::Right => 7.0 - w,
                    _ => -7.0,
                };
                line(p(x0, *y), p(x0 + w, *y));
            }
        }
        Icon::Link | Icon::Unlink => {
            let r1 = Rect::from_center_size(p(-3.0, 0.0), egui::vec2(9.0, 5.5));
            let r2 = Rect::from_center_size(p(3.0, 0.0), egui::vec2(9.0, 5.5));
            painter.rect_stroke(r1, 2.8, stroke, egui::StrokeKind::Middle);
            painter.rect_stroke(r2, 2.8, stroke, egui::StrokeKind::Middle);
            if icon == Icon::Unlink {
                painter.line_segment([p(-6.0, 6.0), p(6.0, -6.0)], Stroke::new(2.0, pal.warn));
            }
        }
        Icon::Anchor => {
            painter.circle_stroke(p(0.0, -5.0), 1.8, stroke);
            line(p(0.0, -3.2), p(0.0, 6.0));
            line(p(-3.5, -1.5), p(3.5, -1.5));
            let points: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let t = std::f32::consts::PI * (i as f32 / 10.0);
                    p(-6.0 * t.cos(), 2.0 + 4.0 * t.sin())
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }
        Icon::Image => {
            let frame = Rect::from_center_size(c, egui::vec2(15.0, 12.0));
            painter.rect_stroke(frame, 1.5, stroke, egui::StrokeKind::Middle);
            painter.add(egui::Shape::line(
                vec![p(-6.0, 5.0), p(-2.0, 0.0), p(1.0, 3.0), p(3.0, 1.0), p(6.5, 5.0)],
                stroke,
            ));
            painter.circle_filled(p(3.0, -2.5), 1.4, color);
        }
        Icon::Table => {
            let frame = Rect::from_center_size(c, egui::vec2(15.0, 12.0));
            painter.rect_stroke(frame, 1.0, stroke, egui::StrokeKind::Middle);
            line(p(-7.5, -2.0), p(7.5, -2.0));
            line(p(-7.5, 2.0), p(7.5, 2.0));
            line(p(-2.5, -6.0), p(-2.5, 6.0));
            line(p(2.5, -6.0), p(2.5, 6.0));
        }
        Icon::Blocks => {
            let frame = Rect::from_center_size(c, egui::vec2(15.0, 13.0));
            for (a, b) in [
                (frame.left_top(), frame.right_top()),
                (frame.right_top(), frame.right_bottom()),
                (frame.right_bottom(), frame.left_bottom()),
                (frame.left_bottom(), frame.left_top()),
            ] {
                painter.add(egui::Shape::dashed_line(&[a, b], Stroke::new(1.2, color), 2.0, 2.0));
            }
            painter.text(c, egui::Align2::CENTER_CENTER, "P", FontId::proportional(9.0), color);
        }
        Icon::Color | Icon::Fill => {
            if icon == Icon::Fill {
                painter.rect_filled(Rect::from_center_size(p(0.0, -1.5), egui::vec2(13.0, 11.0)), 2.0, pal.surface_hover);
            }
            painter.text(p(0.0, -1.5), egui::Align2::CENTER_CENTER, "A", FontId::new(13.0, crate::theme::medium()), color);
            painter.rect_filled(Rect::from_center_size(p(0.0, 6.5), egui::vec2(13.0, 2.5)), 1.0, if icon == Icon::Fill { Color32::from_rgb(241, 196, 15) } else { Color32::from_rgb(231, 76, 60) });
        }
    }
    response.on_hover_text(hint).clicked()
}

fn tool(ui: &mut Ui, pal: &Palette, label: &str, hint: &str, active: bool) -> bool {
    let text = RichText::new(label).size(14.0).color(if active { pal.accent } else { pal.text });
    let button = Button::new(text)
        .min_size(egui::vec2(28.0, 26.0))
        .corner_radius(CornerRadius::same(5))
        .frame_when_inactive(active);
    ui.add(button).on_hover_text(hint).clicked()
}

fn style_tool(ui: &mut Ui, pal: &Palette, label: &str, hint: &str, style: Style, active: bool) -> bool {
    let mut text = RichText::new(label).size(14.0).color(if active { pal.accent } else { pal.text });
    text = match style {
        Style::Bold => text.family(crate::theme::medium()).strong(),
        Style::Italic => text.italics(),
        Style::Underline => text.underline(),
        Style::Strike => text.strikethrough(),
        _ => text,
    };
    let button = Button::new(text)
        .min_size(egui::vec2(28.0, 26.0))
        .corner_radius(CornerRadius::same(5))
        .frame_when_inactive(active);
    ui.add(button).on_hover_text(hint).clicked()
}

pub fn parse_color(value: &str) -> Option<Color32> {
    let v = value.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let expand = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            3 => {
                let c: Vec<u8> = hex.chars().filter_map(|c| expand(&format!("{c}{c}"))).collect();
                (c.len() == 3).then(|| Color32::from_rgb(c[0], c[1], c[2]))
            }
            6 => Some(Color32::from_rgb(expand(&hex[0..2])?, expand(&hex[2..4])?, expand(&hex[4..6])?)),
            _ => None,
        };
    }
    if let Some(inner) = v.strip_prefix("rgb(").or_else(|| v.strip_prefix("rgba(")) {
        let n: Vec<u8> = inner
            .trim_end_matches(')')
            .split(',')
            .take(3)
            .filter_map(|p| p.trim().parse::<f32>().ok().map(|f| f.clamp(0.0, 255.0) as u8))
            .collect();
        return (n.len() == 3).then(|| Color32::from_rgb(n[0], n[1], n[2]));
    }
    Some(match v.as_str() {
        "red" => Color32::from_rgb(255, 0, 0),
        "green" => Color32::from_rgb(0, 128, 0),
        "blue" => Color32::from_rgb(0, 0, 255),
        "black" => Color32::BLACK,
        "white" => Color32::WHITE,
        "gray" | "grey" => Color32::GRAY,
        "orange" => Color32::from_rgb(255, 165, 0),
        "yellow" => Color32::from_rgb(255, 255, 0),
        "purple" => Color32::from_rgb(128, 0, 128),
        _ => return None,
    })
}

fn css_size(value: &str, base: f32) -> Option<f32> {
    let v = value.trim().to_ascii_lowercase();
    if let Some(px) = v.strip_suffix("px") {
        return px.trim().parse::<f32>().ok().map(|p| p * base / 15.0);
    }
    if let Some(pt) = v.strip_suffix("pt") {
        return pt.trim().parse::<f32>().ok().map(|p| p * 1.333 * base / 15.0);
    }
    if let Some(em) = v.strip_suffix("em").or_else(|| v.strip_suffix("rem")) {
        return em.trim().parse::<f32>().ok().map(|e| e * base);
    }
    if let Some(pc) = v.strip_suffix('%') {
        return pc.trim().parse::<f32>().ok().map(|p| p / 100.0 * base);
    }
    None
}

struct Lay<'a> {
    ui: &'a Ui,
    pal: &'a Palette,
    size: f32,
    atoms: &'a [String],
    views: &'a mut Vec<View>,
    decos: &'a mut Vec<Deco>,
    blocks: bool,
    para: usize,
    clip: Rect,
    align: Option<String>,
}

impl Lay<'_> {
    fn blocks(&mut self, list: &[Block], x: f32, width: f32, mut y: f32, path: &mut Path, depth: usize) -> f32 {
        for (i, block) in list.iter().enumerate() {
            path.push(i);
            y = self.block(block, x, width, y, path, depth);
            path.pop();
        }
        y
    }

    fn block(&mut self, block: &Block, x: f32, width: f32, y: f32, path: &mut Path, depth: usize) -> f32 {
        let margin = block
            .attrs()
            .and_then(|a| a.style("margin-left"))
            .and_then(|v| css_size(&v, self.size))
            .unwrap_or(0.0)
            .min(width * 0.5);
        let x = x + margin;
        let width = width - margin;
        let start_y = y;
        let end = match &block.kind {
            Kind::Raw(raw) => {
                let label: String = restore(raw, self.atoms)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(90)
                    .collect();
                if raw.to_ascii_lowercase().starts_with("<hr") {
                    self.decos.push(Deco::Hr(Rect::from_min_size(egui::pos2(x, y + 6.0), egui::vec2(width, 4.0))));
                    y + 16.0
                } else {
                    let rect = Rect::from_min_size(egui::pos2(x, y + 2.0), egui::vec2(width, 26.0));
                    self.decos.push(Deco::Raw(rect, label));
                    y + 34.0
                }
            }
            Kind::Para { tag, attrs, runs } => {
                let top = y + if tag.is_empty() { 0.0 } else { 2.0 };
                let align = parent_align(attrs).or_else(|| self.align.clone());
                let bottom = self.para(tag, runs, x, width, top, align);
                bottom + para_gap(tag, self.size)
            }
            Kind::Box { tag, attrs, children } => {
                let saved = self.align.clone();
                if let Some(a) = parent_align(attrs) {
                    self.align = Some(a);
                }
                let end = match tag.as_str() {
                "ul" | "ol" => {
                    let start = attrs.get("start").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
                    let mut y = y + 2.0;
                    let mut n = start;
                    for (i, child) in children.iter().enumerate() {
                        path.push(i);
                        let label = if tag == "ol" {
                            format!("{n}.")
                        } else if depth.is_multiple_of(2) {
                            "•".into()
                        } else {
                            "◦".into()
                        };
                        if child.tag() == "li" {
                            self.decos.push(Deco::Marker(egui::pos2(x + 20.0, y + 1.0), label, self.size));
                            n += 1;
                        }
                        y = self.block(child, x + 26.0, width - 26.0, y, path, depth + 1);
                        path.pop();
                    }
                    y + 4.0
                }
                "blockquote" => {
                    let bottom = self.blocks(children, x + 18.0, width - 18.0, y + 4.0, path, depth);
                    self.decos.push(Deco::Bar(Rect::from_min_max(egui::pos2(x + 2.0, y + 4.0), egui::pos2(x + 5.0, bottom))));
                    bottom + 6.0
                }
                "table" => self.table(block, x, width, y + 4.0, path, depth) + 8.0,
                _ => self.blocks(children, x, width, y, path, depth),
                };
                self.align = saved;
                end
            }
        };
        if self.blocks && !block.tag().is_empty() {
            let rect = Rect::from_min_max(egui::pos2(x - 3.0, start_y), egui::pos2(x + width + 3.0, end.max(start_y + 8.0) - 2.0));
            self.decos.push(Deco::Outline(rect, block.tag().to_owned()));
        }
        end
    }

    fn table(&mut self, block: &Block, x: f32, width: f32, y: f32, path: &mut Path, depth: usize) -> f32 {
        let Some(children) = block.children() else {
            return y;
        };
        let mut rows: Vec<(Path, &Block)> = Vec::new();
        for (i, child) in children.iter().enumerate() {
            match child.tag() {
                "tr" => rows.push(([path.as_slice(), &[i]].concat(), child)),
                "thead" | "tbody" | "tfoot" => {
                    for (k, row) in child.children().into_iter().flatten().enumerate() {
                        if row.tag() == "tr" {
                            rows.push(([path.as_slice(), &[i, k]].concat(), row));
                        }
                    }
                }
                _ => {}
            }
        }
        let cols = rows
            .iter()
            .map(|(_, r)| {
                r.children()
                    .map(|c| {
                        c.iter()
                            .map(|cell| cell.attrs().and_then(|a| a.get("colspan")).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1))
                            .sum::<usize>()
                    })
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(1)
            .max(1);
        let col_w = width / cols as f32;
        let mut y = y;
        for (row_path, row) in rows {
            let mut cx = x;
            let mut bottom = y + self.size + 12.0;
            let mut cells = Vec::new();
            for (k, cell) in row.children().into_iter().flatten().enumerate() {
                let span = cell.attrs().and_then(|a| a.get("colspan")).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1);
                let w = col_w * span as f32;
                let mut cell_path = [row_path.as_slice(), &[k]].concat();
                let inner = match cell.children() {
                    Some(kids) => self.blocks(kids, cx + 6.0, w - 12.0, y + 5.0, &mut cell_path, depth),
                    None => y + 5.0,
                };
                bottom = bottom.max(inner + 5.0);
                cells.push((cx, w, cell.tag() == "th"));
                cx += w;
            }
            for (cx, w, header) in cells {
                self.decos.insert(0, Deco::Cell(Rect::from_min_size(egui::pos2(cx, y), egui::vec2(w, bottom - y)), header));
            }
            y = bottom;
        }
        y
    }

    fn para(&mut self, tag: &str, runs: &[Inline], x: f32, width: f32, y: f32, align: Option<String>) -> f32 {
        let index = self.para;
        self.para += 1;
        let base = match tag {
            "h1" => self.size * 1.75,
            "h2" => self.size * 1.5,
            "h3" => self.size * 1.3,
            "h4" => self.size * 1.15,
            "h5" => self.size,
            "h6" => self.size * 0.9,
            _ => self.size,
        };
        let heading = tag.len() == 2 && tag.starts_with('h');
        let mono = tag == "pre";
        let mut job = LayoutJob {
            wrap: egui::text::TextWrapping::wrap_at_width(width.max(20.0)),
            halign: match align.as_deref() {
                Some("center") => Align::Center,
                Some("right") => Align::RIGHT,
                _ => Align::LEFT,
            },
            ..Default::default()
        };
        let mut segs = Vec::new();
        let mut m = 0;
        let mut g = 0;
        for run in runs {
            match run {
                Inline::Text { text, marks } => {
                    let format = self.format(marks, base, heading, mono, tag == "address");
                    job.append(text, 0.0, format);
                    let n = text.chars().count();
                    segs.push(Seg { m, ml: n, g, gl: n });
                    m += n;
                    g += n;
                }
                Inline::Object { html, marks } => {
                    let (label, kind) = object_label(html, self.atoms);
                    let mut format = self.format(marks, base, heading, mono, false);
                    if kind != ObjectKind::Break {
                        format.font_id = FontId::monospace((base * 0.82).max(9.0));
                        format.background = self.pal.surface;
                        format.color = match kind {
                            ObjectKind::Image => self.pal.text,
                            ObjectKind::Entity => self.pal.subtext,
                            _ => self.pal.accent,
                        };
                        format.underline = Stroke::NONE;
                        format.strikethrough = Stroke::NONE;
                        format.italics = false;
                    }
                    let shown = if kind == ObjectKind::Break { label } else { format!("\u{2009}{label}\u{2009}") };
                    let n = shown.chars().count();
                    job.append(&shown, 0.0, format);
                    segs.push(Seg { m, ml: 1, g, gl: n });
                    m += 1;
                    g += n;
                }
            }
        }
        if runs.is_empty() {
            let format = self.format(&[], base, heading, mono, false);
            job.append("", 0.0, format);
        }
        let galley = self.ui.fonts_mut(|f| f.layout_job(job));
        let height = galley.size().y.max(base * 1.25);
        let shift = match align.as_deref() {
            Some("center") => width / 2.0,
            Some("right") => width,
            _ => 0.0,
        };
        let origin = egui::pos2(x + shift, y);
        let clip = Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height)).intersect(self.clip.expand(4000.0));
        self.views.push(View { para: index, origin, galley, segs, clip });
        y + height
    }

    fn format(&self, marks: &[Mark], base: f32, heading: bool, mono: bool, italic: bool) -> TextFormat {
        let mut size = base;
        let mut bold = heading;
        let mut italics = italic;
        let mut underline = false;
        let mut strike = false;
        let mut color = self.pal.text;
        let mut background = Color32::TRANSPARENT;
        let mut code = mono;
        let mut valign = Align::BOTTOM;
        let mut small = 1.0;
        for mark in marks {
            match mark.tag.as_str() {
                "strong" | "b" => bold = true,
                "em" | "i" | "cite" | "dfn" => italics = true,
                "u" | "ins" => underline = true,
                "s" | "strike" | "del" => strike = true,
                "code" | "kbd" | "samp" | "tt" => code = true,
                "sub" => {
                    small = 0.72;
                    valign = Align::BOTTOM;
                }
                "sup" => {
                    small = 0.72;
                    valign = Align::TOP;
                }
                "small" => small = 0.85,
                "big" => small = 1.2,
                "a" => {
                    color = self.pal.accent;
                    underline = true;
                }
                "mark" => background = Color32::from_rgba_unmultiplied(255, 215, 0, 90),
                _ => {}
            }
            if let Some(c) = mark.attrs.style("color").and_then(|c| parse_color(&c)) {
                color = c;
            }
            if let Some(c) = mark.attrs.style("background-color").or_else(|| mark.attrs.style("background")).and_then(|c| parse_color(&c)) {
                background = c;
            }
            if let Some(s) = mark.attrs.style("font-size").and_then(|s| css_size(&s, base)) {
                size = s.clamp(6.0, 120.0);
            }
            if mark.tag == "font"
                && let Some(c) = mark.attrs.get("color").and_then(parse_color)
            {
                color = c;
            }
        }
        let family = if code {
            FontFamily::Monospace
        } else if bold {
            crate::theme::medium()
        } else {
            FontFamily::Proportional
        };
        TextFormat {
            font_id: FontId::new(size * small, family),
            color,
            background,
            italics,
            underline: if underline { Stroke::new(1.0, color) } else { Stroke::NONE },
            strikethrough: if strike { Stroke::new(1.0, color) } else { Stroke::NONE },
            valign,
            line_height: Some(size * 1.45),
            ..Default::default()
        }
    }
}

fn parent_align(attrs: &Attrs) -> Option<String> {
    attrs.style("text-align").or_else(|| attrs.get("align").map(str::to_owned))
}

fn para_gap(tag: &str, size: f32) -> f32 {
    match tag {
        "" => 2.0,
        "h1" | "h2" | "h3" => size * 0.7,
        _ => size * 0.6,
    }
}
