use eframe::egui::{self, Align, Color32, ComboBox, FontId, Layout, Modal, Pos2, Rect, RichText, Sense, Stroke, TextEdit, Ui};

use crate::theme::Palette;


pub fn primary(text: impl Into<String>, pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(pal.base)).fill(pal.accent)
}

pub fn secondary(text: impl Into<String>, _pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(text.into())
}

pub const DANGER: Color32 = Color32::from_rgb(0xc8, 0x3c, 0x3c);

pub fn danger(text: impl Into<String>, _pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(Color32::WHITE)).fill(DANGER)
}

pub const ACTION: egui::Vec2 = egui::vec2(0.0, FORM_ROW);
pub const RESULT_ROW: f32 = 22.0;
pub const TITLE: f32 = 15.5;

pub fn ok_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0x3f, 0xc2, 0x6e)
    } else {
        Color32::from_rgb(0x1e, 0x88, 0x45)
    }
}

pub fn title(ui: &mut Ui, pal: &Palette, text: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(text.into()).size(TITLE).color(pal.text))
}

pub trait Lit {
    fn lit(self, ui: &Ui) -> Self;
}

impl Lit for egui::Response {
    fn lit(self, ui: &Ui) -> Self {
        if self.enabled() && (self.hovered() || self.is_pointer_button_down_on()) {
            let alpha = if self.is_pointer_button_down_on() {
                48
            } else {
                26
            };
            let color = if ui.visuals().dark_mode {
                Color32::from_white_alpha(alpha)
            } else {
                Color32::from_black_alpha(alpha)
            };
            ui.painter().rect_filled(
                self.rect,
                ui.visuals().widgets.inactive.corner_radius,
                color,
            );
        }
        self
    }
}

pub const FORM_LABEL: f32 = 86.0;
pub const FORM_ROW: f32 = 28.0;
pub const RESULTS: f32 = 320.0;

pub fn form_label(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(FORM_LABEL, FORM_ROW),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(FORM_LABEL, FORM_ROW));
            ui.label(RichText::new(text).color(pal.subtext))
        },
    );
}

pub fn field_width(ui: &Ui, reserve: f32) -> f32 {
    (ui.available_width() - reserve - 16.0).max(60.0)
}

pub fn form_row<R>(ui: &mut Ui, pal: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    if ui.available_width() < 380.0 {
        ui.label(RichText::new(label).color(pal.subtext));
        return ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                add(ui)
            })
            .inner;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        form_label(ui, pal, label);
        ui.spacing_mut().item_spacing.x = 6.0;
        add(ui)
    })
    .inner
}

pub fn form_field<'a>(text: &'a mut String, hint: impl Into<egui::WidgetText>) -> TextEdit<'a> {
    TextEdit::singleline(text)
        .hint_text(hint)
        .margin(egui::Margin::symmetric(8, 6))
}

pub fn toggle(ui: &mut Ui, pal: &Palette, value: &mut bool, label: &str, hint: &str) -> bool {
    let text =
        RichText::new(label)
            .monospace()
            .color(if *value { pal.accent } else { pal.subtext });
    let button = egui::Button::new(text).fill(if *value {
        pal.surface
    } else {
        Color32::TRANSPARENT
    });
    let clicked = ui
        .add_sized(egui::vec2(30.0, FORM_ROW), button)
        .on_hover_text(hint)
        .clicked();
    if clicked {
        *value = !*value;
    }
    clicked
}

pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    pal: &Palette,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(pal.crust)
        .corner_radius(7)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (option, label) in options {
                let active = *value == *option;
                let text = RichText::new(*label).color(if active { pal.text } else { pal.subtext });
                let button = egui::Button::new(text)
                    .min_size(egui::vec2(width, 24.0))
                    .corner_radius(5)
                    .fill(if active {
                        pal.surface
                    } else {
                        Color32::TRANSPARENT
                    });
                if ui.add(button).clicked() && !active {
                    *value = *option;
                    changed = true;
                }
            }
        });
    changed
}

pub fn window_frame(pal: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.mantle)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(18))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(70),
        })
}

pub fn wrapped_row<R>(ui: &mut Ui, margin: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(margin as i8, 0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                add(ui)
            })
            .inner
        })
        .inner
}

pub fn port_field(ui: &mut Ui, port: &mut u16, width: f32) -> egui::Response {
    let id = ui
        .scope_id()
        .with(("tror_port", port as *const u16 as usize));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| port.to_string());
    let response = ui.add(
        TextEdit::singleline(&mut text)
            .desired_width(width)
            .char_limit(5)
            .hint_text(tr!("порт")),
    );
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    if digits != text {
        text = digits;
    }
    if let Ok(value) = text.parse::<u32>()
        && (1..=65535).contains(&value)
    {
        *port = value as u16;
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    response
}

pub fn dots_button(ui: &mut Ui, pal: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Ещё")));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if response.hovered() || open {
        ui.painter().rect_filled(
            rect,
            5.0,
            if response.is_pointer_button_down_on() {
                pal.surface_hover
            } else {
                pal.surface
            },
        );
    }
    let color = if response.hovered() || open {
        pal.text
    } else {
        pal.subtext
    };
    for dx in [-5.0, 0.0, 5.0] {
        ui.painter()
            .circle_filled(rect.center() + egui::vec2(dx, 0.0), 1.6, color);
    }
    response
}

pub fn plus_button(ui: &mut Ui, pal: &Palette, size: egui::Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Добавить")));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if response.hovered() || open {
        ui.painter().rect_filled(
            rect,
            6.0,
            if response.is_pointer_button_down_on() {
                pal.surface_hover
            } else {
                pal.surface
            },
        );
    }
    let color = if response.hovered() || open {
        pal.text
    } else {
        pal.subtext
    };
    let scale = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * scale).round() / scale;
    let thick = (1.5 * scale).round().max(1.0) / scale;
    let arm = snap(5.0);
    let x = snap(rect.center().x - thick / 2.0);
    let y = snap(rect.center().y - thick / 2.0);
    let painter = ui.painter();
    painter.rect_filled(
        Rect::from_min_size(egui::pos2(x - arm, y), egui::vec2(arm * 2.0 + thick, thick)),
        0.0,
        color,
    );
    painter.rect_filled(
        Rect::from_min_size(egui::pos2(x, y - arm), egui::vec2(thick, arm * 2.0 + thick)),
        0.0,
        color,
    );
    response
}

pub fn scroll_arrow(ui: &mut Ui, pal: &Palette, forward: bool, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            if forward {
                tr!("Вперёд")
            } else {
                tr!("Назад")
            },
        )
    });
    let color = if !enabled {
        pal.faint.gamma_multiply(0.5)
    } else if response.hovered() {
        pal.text
    } else {
        pal.subtext
    };
    let direction = if forward { -1.0 } else { 1.0 };
    let center = rect.center();
    let tip = center + egui::vec2(-3.0 * direction, 0.0);
    let stroke = Stroke::new(1.4, color);
    ui.painter()
        .line_segment([center + egui::vec2(2.0 * direction, -4.5), tip], stroke);
    ui.painter()
        .line_segment([center + egui::vec2(2.0 * direction, 4.5), tip], stroke);
    response
}

pub fn chevron(painter: &egui::Painter, center: Pos2, open: bool, color: Color32) {
    let d = 3.2;
    let points = if open {
        vec![
            center + egui::vec2(-d, -d * 0.55),
            center + egui::vec2(0.0, d * 0.55),
            center + egui::vec2(d, -d * 0.55),
        ]
    } else {
        vec![
            center + egui::vec2(-d * 0.55, -d),
            center + egui::vec2(d * 0.55, 0.0),
            center + egui::vec2(-d * 0.55, d),
        ]
    };
    painter.add(egui::Shape::line(points, Stroke::new(1.3, color)));
}

fn kind_color(name: &str, pal: &Palette) -> Color32 {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "php" | "phtml" | "twig" | "tpl" => Color32::from_rgb(0x9d, 0x8c, 0xf2),
        "js" | "mjs" | "ts" | "json" => Color32::from_rgb(0xe5, 0xc0, 0x5a),
        "css" | "scss" | "less" => Color32::from_rgb(0x5a, 0xa9, 0xf2),
        "html" | "htm" | "xml" | "svg" => Color32::from_rgb(0xf0, 0x8a, 0x5d),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => Color32::from_rgb(0x6c, 0xc7, 0x8a),
        "zip" | "gz" | "tar" | "rar" | "7z" => Color32::from_rgb(0xc9, 0x8f, 0x6b),
        "rs" => Color32::from_rgb(0xde, 0x86, 0x5a),
        "py" => Color32::from_rgb(0x5f, 0x9c, 0xd8),
        "sql" => Color32::from_rgb(0xd6, 0x7c, 0xb6),
        _ => pal.faint,
    }
}

pub fn paint_icon(painter: &egui::Painter, center: Pos2, is_dir: bool, name: &str, pal: &Palette) {
    if is_dir {
        let color = pal.accent.gamma_multiply(0.9);
        let body = Rect::from_center_size(center + egui::vec2(0.0, 1.0), egui::vec2(14.0, 10.0));
        let tab = Rect::from_min_size(body.left_top() - egui::vec2(0.0, 2.5), egui::vec2(6.5, 4.0));
        painter.rect_filled(tab, 1.5, color.gamma_multiply(0.75));
        painter.rect_filled(body, 2.0, color);
        painter.line_segment(
            [
                body.left_top() + egui::vec2(1.0, 2.5),
                body.right_top() + egui::vec2(-1.0, 2.5),
            ],
            Stroke::new(1.0, pal.mantle.gamma_multiply(0.35)),
        );
    } else {
        let color = kind_color(name, pal);
        let page = Rect::from_center_size(center, egui::vec2(10.5, 13.5));
        let fold = 3.5;
        let outline = vec![
            page.left_top(),
            page.right_top() - egui::vec2(fold, 0.0),
            page.right_top() + egui::vec2(0.0, fold),
            page.right_bottom(),
            page.left_bottom(),
        ];
        painter.add(egui::Shape::closed_line(outline, Stroke::new(1.1, color)));
        painter.line_segment(
            [
                page.right_top() - egui::vec2(fold, 0.0),
                page.right_top() + egui::vec2(-fold, fold),
            ],
            Stroke::new(1.0, color),
        );
        painter.line_segment(
            [
                page.right_top() + egui::vec2(-fold, fold),
                page.right_top() + egui::vec2(0.0, fold),
            ],
            Stroke::new(1.0, color),
        );
        for (i, w) in [5.5, 4.0].into_iter().enumerate() {
            let y = page.center().y + 1.0 + i as f32 * 3.0;
            painter.line_segment(
                [
                    egui::pos2(page.left() + 2.5, y),
                    egui::pos2(page.left() + 2.5 + w, y),
                ],
                Stroke::new(1.0, color.gamma_multiply(0.7)),
            );
        }
    }
}

pub fn refresh_button(ui: &mut Ui, pal: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Обновить")));
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            5.0,
            if response.is_pointer_button_down_on() {
                pal.surface_hover
            } else {
                pal.surface
            },
        );
    }
    let color = if response.hovered() {
        pal.text
    } else {
        pal.subtext
    };
    let c = rect.center();
    let r = 5.5;
    let points: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = -0.4 + i as f32 / 20.0 * 5.1;
            c + egui::vec2(a.cos() * r, a.sin() * r)
        })
        .collect();
    let tip = points[0];
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.5, color)));
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            tip + egui::vec2(-3.2, -3.6),
            tip + egui::vec2(2.6, -2.6),
            tip + egui::vec2(-0.6, 2.2),
        ],
        color,
        Stroke::NONE,
    ));
    response
}

pub fn date_label(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn parse_date(text: &str) -> Option<u64> {
    let text = text.trim();
    for format in [
        "%d.%m.%Y %H:%M:%S",
        "%d.%m.%Y %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return naive
                .and_local_timezone(chrono::Local)
                .single()
                .map(|d| d.timestamp().max(0) as u64);
        }
    }
    None
}

fn parse_mode(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || text.len() > 4 || !text.chars().all(|c| ('0'..='7').contains(&c)) {
        return None;
    }
    u32::from_str_radix(text, 8).ok()
}

pub fn perm_grid(ui: &mut Ui, pal: &Palette, mode: &mut u32, octal: &mut String) -> bool {
    let mut changed = false;
    egui::Grid::new(ui.scope_id().with("tror_perm_grid"))
        .num_columns(4)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            ui.label("");
            for label in [tr!("чтение"), tr!("запись"), tr!("выполнение")] {
                ui.label(RichText::new(label).small().color(pal.subtext));
            }
            ui.end_row();
            for (shift, who) in [
                (6, tr!("Владелец")),
                (3, tr!("Группа")),
                (0, tr!("Остальные")),
            ] {
                ui.label(who);
                for bit in [4u32, 2, 1] {
                    let mask = bit << shift;
                    let mut on = *mode & mask != 0;
                    if ui.checkbox(&mut on, "").changed() {
                        if on {
                            *mode |= mask;
                        } else {
                            *mode &= !mask;
                        }
                        changed = true;
                    }
                }
                ui.end_row();
            }
        });
    if changed {
        *octal = format!("{:03o}", *mode & 0o7777);
    }
    ui.horizontal(|ui| {
        ui.label(tr!("Числом"));
        let field = ui.add(
            TextEdit::singleline(octal)
                .desired_width(60.0)
                .font(egui::TextStyle::Monospace),
        );
        if field.changed()
            && let Some(value) = parse_mode(octal)
        {
            *mode = value;
            changed = true;
        }
        ui.label(
            RichText::new(perm_string(*mode, false)[1..].to_owned())
                .monospace()
                .color(pal.subtext),
        );
    });
    changed
}

fn perm_string(mode: u32, is_dir: bool) -> String {
    let mut out = String::with_capacity(10);
    out.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

pub fn appearance_rows(
    ui: &mut Ui,
    pal: &Palette,
    theme: &mut String,
    font: &mut String,
    font_size: &mut f32,
    wide: f32,
) {
    ui.label(tr!("Цветовая схема"));
    ComboBox::from_id_salt("tror_term_theme")
        .width(wide)
        .selected_text(if theme.is_empty() {
            tr!("Как в редакторе")
        } else {
            crate::theme::SCHEMES
                .iter()
                .find(|s| s.name == theme.as_str())
                .map_or(theme.as_str(), |s| tr!(s.name))
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(theme, String::new(), tr!("Как в редакторе"));
            for scheme in crate::theme::SCHEMES {
                ui.selectable_value(theme, scheme.name.to_owned(), tr!(scheme.name));
            }
        });
    ui.end_row();
    ui.label("");
    let colors = crate::theme::colors(theme, pal, true);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(wide, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, colors.bg);
    for (i, color) in colors.ansi.iter().take(8).enumerate() {
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 14.0 + i as f32 * 18.0, rect.center().y),
            5.5,
            *color,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 162.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "user@host:~$ ls",
        FontId::monospace(12.0),
        colors.fg,
    );
    ui.end_row();
    ui.label(tr!("Шрифт"));
    ui.horizontal(|ui| {
        let name = if font.trim().is_empty() {
            tr!("как в редакторе").to_owned()
        } else {
            std::path::Path::new(font.trim())
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| font.clone())
        };
        ui.label(RichText::new(name).color(pal.subtext));
        if ui.button(tr!("Выбрать…")).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter(tr!("Шрифты"), &["ttf", "otf", "ttc"])
                .pick_file()
        {
            *font = path.display().to_string();
        }
        if !font.is_empty() && ui.button(tr!("Сбросить")).clicked() {
            font.clear();
        }
    });
    ui.end_row();
    ui.label(tr!("Размер шрифта"));
    ui.horizontal(|ui| {
        let mut custom = *font_size > 0.0;
        if ui.checkbox(&mut custom, tr!("свой")).changed() {
            *font_size = if custom { 14.0 } else { 0.0 };
        }
        if custom {
            crate::widgets::stepper(ui, pal, "term_font", font_size, (8.0, 28.0), 1.0, "px");
        }
    });
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_dates_parse() {
        assert_eq!(parse_mode("755"), Some(0o755));
        assert_eq!(parse_mode("0644"), Some(0o644));
        assert_eq!(parse_mode("788"), None);
        assert_eq!(parse_mode(""), None);
        assert_eq!(perm_string(0o754, true), "drwxr-xr--");
        let secs = parse_date("02.01.2026 03:04").unwrap();
        assert_eq!(date_label(secs), "02.01.2026 03:04");
        assert!(parse_date("вчера").is_none());
    }

    #[test]
    fn sort_keeps_folders_first_and_cycles() {
        let mut items = vec![
            ("b", false, 5u64),
            ("A", true, 1),
            ("c", false, 1),
            ("z", true, 9),
        ];
        fn fields<'a>(i: &'a (&'static str, bool, u64)) -> (bool, &'a str, u64, u32, u64) {
            (i.1, i.0, i.2, 0, i.2)
        }
        let mut sort = Sort::default();
        sort.click(SortKey::Size);
        sort.apply(&mut items, fields);
        assert_eq!(
            items.iter().map(|i| i.0).collect::<Vec<_>>(),
            ["A", "z", "c", "b"]
        );
        sort.click(SortKey::Size);
        sort.apply(&mut items, fields);
        assert_eq!(
            items.iter().map(|i| i.0).collect::<Vec<_>>(),
            ["z", "A", "b", "c"]
        );
        sort.click(SortKey::Size);
        assert_eq!(sort, Sort::default());
    }

    #[test]
    fn columns_shrink_to_fit_and_never_overlap() {
        set_columns([260.0, 34.0, 260.0]);
        for width in [200.0, 260.0, 320.0, 480.0, 900.0] {
            let fit = fit_columns(width, [true, true, true]);
            let used: f32 = fit
                .iter()
                .filter(|w| **w > 0.0)
                .map(|w| w + COLUMN_GAP)
                .sum();
            assert!(
                used <= (width - 6.0 - NAME_MIN).max(0.0) + 0.01,
                "{width}: {fit:?}"
            );
            for i in 0..3 {
                assert!(fit[i] == 0.0 || fit[i] >= COLUMN_MIN[i], "{width}: {fit:?}");
            }
        }
        assert!(
            fit_columns(320.0, [true, false, true])
                .iter()
                .filter(|w| **w > 0.0)
                .count()
                == 2
        );
        assert_eq!(fit_columns(900.0, [true, true, true]), [260.0, 34.0, 260.0]);
        set_columns(COLUMN_DEFAULTS);
    }
}

fn step_button(ui: &mut Ui, pal: &Palette, plus: bool, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let lit = enabled && response.hovered();
    ui.painter()
        .rect_filled(rect, 6.0, if lit { pal.surface_hover } else { pal.surface });
    let color = if !enabled {
        pal.faint.gamma_multiply(0.5)
    } else if lit {
        pal.text
    } else {
        pal.subtext
    };
    let scale = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * scale).round() / scale;
    let thick = (1.5 * scale).round().max(1.0) / scale;
    let arm = snap(4.0);
    let x = snap(rect.center().x - thick / 2.0);
    let y = snap(rect.center().y - thick / 2.0);
    ui.painter().rect_filled(
        Rect::from_min_size(egui::pos2(x - arm, y), egui::vec2(arm * 2.0 + thick, thick)),
        0.0,
        color,
    );
    if plus {
        ui.painter().rect_filled(
            Rect::from_min_size(egui::pos2(x, y - arm), egui::vec2(thick, arm * 2.0 + thick)),
            0.0,
            color,
        );
    }
    enabled && response.clicked()
}

pub fn stepper(
    ui: &mut Ui,
    pal: &Palette,
    key: &str,
    value: &mut f32,
    (min, max): (f32, f32),
    step: f32,
    suffix: &str,
) -> bool {
    let decimals = if step.fract() == 0.0 {
        0
    } else if (step * 10.0).fract().abs() < 1e-4 {
        1
    } else {
        2
    };
    let snap = |v: f32| ((v / step).round() * step).clamp(min, max);
    let before = *value;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if step_button(ui, pal, false, *value > min + step * 0.01) {
            *value = snap(*value - step);
        }
        let id = ui.scope_id().with(("tror_stepper", key));
        let shown = format!("{:.*}", decimals, *value);
        let mut text = ui
            .data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| shown.clone());
        let field = ui.add(
            TextEdit::singleline(&mut text)
                .desired_width(
                    (format!("{:.*}", decimals, max).len() as f32 * 8.0 + 14.0).max(46.0),
                )
                .align(egui::Align2::CENTER_CENTER),
        );
        if field.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text));
        } else if field.lost_focus() {
            if let Ok(parsed) = text.trim().replace(',', ".").parse::<f32>()
                && parsed.is_finite()
            {
                *value = snap(parsed);
            }
            ui.data_mut(|d| d.remove::<String>(id));
        }
        if step_button(ui, pal, true, *value < max - step * 0.01) {
            *value = snap(*value + step);
        }
        if !suffix.is_empty() {
            ui.label(egui::RichText::new(suffix).color(pal.faint));
        }
    });
    (*value - before).abs() > f32::EPSILON
}

pub const COLUMN_DEFAULTS: [f32; 3] = [52.0, 34.0, 106.0];
pub const COLUMN_GAP: f32 = 8.0;
static COLUMNS: std::sync::Mutex<[f32; 3]> = std::sync::Mutex::new(COLUMN_DEFAULTS);

pub fn columns() -> [f32; 3] {
    COLUMNS.lock().map(|c| *c).unwrap_or(COLUMN_DEFAULTS)
}

pub fn set_columns(widths: [f32; 3]) {
    if let Ok(mut columns) = COLUMNS.lock() {
        *columns = widths.map(|w| {
            if w.is_finite() {
                w.clamp(30.0, 260.0)
            } else {
                52.0
            }
        });
    }
}

const COLUMN_MIN: [f32; 3] = [40.0, 30.0, 56.0];
const NAME_MIN: f32 = 110.0;

fn fit_columns(width: f32, shown: [bool; 3]) -> [f32; 3] {
    let stored = columns();
    let room = width - 6.0 - NAME_MIN;
    let mut on = shown;
    let least = |on: &[bool; 3]| -> f32 {
        (0..3)
            .filter(|i| on[*i])
            .map(|i| COLUMN_MIN[i] + COLUMN_GAP)
            .sum()
    };
    for drop in [1, 2, 0] {
        if least(&on) > room {
            on[drop] = false;
        }
    }
    let mut fit = [0.0; 3];
    for i in 0..3 {
        if on[i] {
            fit[i] = stored[i].max(COLUMN_MIN[i]);
        }
    }
    let total: f32 = (0..3).filter(|i| on[*i]).map(|i| fit[i] + COLUMN_GAP).sum();
    let slack: f32 = (0..3)
        .filter(|i| on[*i])
        .map(|i| fit[i] - COLUMN_MIN[i])
        .sum();
    if total > room && slack > 0.0 {
        let cut = ((total - room) / slack).min(1.0);
        for i in 0..3 {
            if on[i] {
                fit[i] -= (fit[i] - COLUMN_MIN[i]) * cut;
            }
        }
    }
    fit
}

pub fn rows_width<'a>(
    ui: &Ui,
    rows: impl Iterator<Item = (f32, &'a str)>,
    widths: [f32; 3],
) -> f32 {
    let view = ui.available_width();
    let guess = |(indent, name): (f32, &str)| indent + name.chars().count() as f32 * 7.0;
    let Some((indent, name)) = rows.max_by(|a, b| guess(*a).total_cmp(&guess(*b))) else {
        return view;
    };
    let text = ui.ctx().fonts_mut(|f| {
        f.layout_no_wrap(
            name.to_owned(),
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        )
        .size()
        .x
    });
    let columns: f32 = widths
        .iter()
        .filter(|w| **w > 0.0)
        .map(|w| w + COLUMN_GAP)
        .sum();
    view.max(indent + 28.0 + text + 12.0 + columns + 6.0)
}

pub fn column_cell(
    painter: &egui::Painter,
    row: Rect,
    right: &mut f32,
    width: f32,
    text: Option<String>,
    color: Color32,
) {
    if width <= 0.0 {
        return;
    }
    if let Some(text) = text {
        let clip = Rect::from_min_max(
            egui::pos2(*right - width, row.top()),
            egui::pos2(*right, row.bottom()),
        );
        painter
            .with_clip_rect(clip.intersect(painter.clip_rect()))
            .text(
                egui::pos2(*right, row.center().y),
                egui::Align2::RIGHT_CENTER,
                text,
                egui::FontId::proportional(11.5),
                color,
            );
    }
    *right -= width + COLUMN_GAP;
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum SortKey {
    #[default]
    None,
    Name,
    Size,
    Perm,
    Date,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub desc: bool,
}

impl Sort {
    pub fn apply<T>(&self, items: &mut [T], fields: impl Fn(&T) -> (bool, &str, u64, u32, u64)) {
        if self.key == SortKey::None {
            return;
        }
        items.sort_by(|a, b| {
            let (a, b) = (fields(a), fields(b));
            let by_name = || a.1.to_lowercase().cmp(&b.1.to_lowercase());
            let main = match self.key {
                SortKey::Size => a.2.cmp(&b.2).then_with(by_name),
                SortKey::Perm => a.3.cmp(&b.3).then_with(by_name),
                SortKey::Date => a.4.cmp(&b.4).then_with(by_name),
                _ => by_name(),
            };
            let main = if self.desc { main.reverse() } else { main };
            (!a.0).cmp(&!b.0).then(main)
        });
    }

    fn click(&mut self, key: SortKey) {
        *self = if self.key != key {
            Sort { key, desc: false }
        } else if !self.desc {
            Sort { key, desc: true }
        } else {
            Sort::default()
        };
    }
}

fn sort_arrow(painter: &egui::Painter, at: Pos2, desc: bool, color: Color32) {
    let (tip, base) = if desc { (3.0, -2.0) } else { (-3.0, 2.0) };
    painter.add(egui::Shape::convex_polygon(
        vec![
            at + egui::vec2(0.0, tip),
            at + egui::vec2(-3.5, base),
            at + egui::vec2(3.5, base),
        ],
        color,
        Stroke::NONE,
    ));
}

pub fn column_header(ui: &mut Ui, pal: &Palette, shown: [bool; 3], sort: &mut Sort) -> [f32; 3] {
    let mut stored = columns();
    let (rect, header) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), Sense::hover());
    header.context_menu(|ui| {
        if ui
            .add_enabled(
                sort.key != SortKey::None,
                egui::Button::new(tr!("Сбросить сортировку")),
            )
            .clicked()
        {
            *sort = Sort::default();
            ui.close();
        }
    });
    let fit = fit_columns(rect.width(), shown);
    let painter = ui.painter_at(rect);
    let font = egui::FontId::proportional(11.0);
    let room = rect.width() - 6.0 - NAME_MIN;
    let mut right = rect.right() - 6.0;
    let mut name_right = rect.right();
    for (index, label, key) in [
        (2, tr!("Изменён"), SortKey::Date),
        (1, tr!("Права"), SortKey::Perm),
        (0, tr!("Размер"), SortKey::Size),
    ] {
        let width = fit[index];
        if width <= 0.0 {
            continue;
        }
        let left = right - width;
        let cell = Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        let click = ui
            .interact(
                cell,
                ui.scope_id().with(("tror_sort", index)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if click.clicked() {
            sort.click(key);
        }
        click.context_menu(|ui| {
            if ui
                .add_enabled(
                    sort.key != SortKey::None,
                    egui::Button::new(tr!("Сбросить сортировку")),
                )
                .clicked()
            {
                *sort = Sort::default();
                ui.close();
            }
        });
        let active = sort.key == key;
        let color = if active || click.hovered() {
            pal.text
        } else {
            pal.faint
        };
        let galley = painter.layout_no_wrap(label.to_owned(), font.clone(), color);
        let text_left = right - galley.size().x;
        painter.with_clip_rect(cell).galley(
            egui::pos2(text_left, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        if active {
            sort_arrow(
                &painter,
                egui::pos2(text_left - 7.0, rect.center().y),
                sort.desc,
                pal.accent,
            );
        }
        let grip = Rect::from_min_max(
            egui::pos2(left - COLUMN_GAP, rect.top()),
            egui::pos2(left, rect.bottom()),
        );
        let response = ui
            .interact(
                grip,
                ui.scope_id().with(("tror_column", index)),
                Sense::drag(),
            )
            .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
        let lit = response.hovered() || response.dragged();
        painter.line_segment(
            [
                egui::pos2(left - 4.0, rect.top() + 4.0),
                egui::pos2(left - 4.0, rect.bottom() - 4.0),
            ],
            Stroke::new(1.0, if lit { pal.accent } else { pal.border }),
        );
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let others: f32 = (0..3)
                .filter(|i| *i != index && fit[*i] > 0.0)
                .map(|i| fit[i] + COLUMN_GAP)
                .sum();
            let most = (room - others - COLUMN_GAP).clamp(COLUMN_MIN[index], 260.0);
            stored[index] = (right - pointer.x - 4.0).clamp(COLUMN_MIN[index], most);
            for i in 0..3 {
                if i != index && fit[i] > 0.0 {
                    stored[i] = fit[i];
                }
            }
        }
        if response.double_clicked() {
            stored[index] = COLUMN_DEFAULTS[index];
        }
        right = left - COLUMN_GAP;
        name_right = left - COLUMN_GAP;
    }
    let name_cell = Rect::from_min_max(rect.min, egui::pos2(name_right, rect.bottom()));
    let click = ui
        .interact(
            name_cell,
            ui.scope_id().with("tror_sort_name"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if click.clicked() {
        sort.click(SortKey::Name);
    }
    let active = sort.key == SortKey::Name;
    let color = if active || click.hovered() {
        pal.text
    } else {
        pal.faint
    };
    let galley = painter.layout_no_wrap(tr!("Имя").to_owned(), font, color);
    let width = galley.size().x;
    painter.galley(
        egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if active {
        sort_arrow(
            &painter,
            egui::pos2(rect.left() + 13.0 + width, rect.center().y),
            sort.desc,
            pal.accent,
        );
    }
    click.context_menu(|ui| {
        if ui
            .add_enabled(
                sort.key != SortKey::None,
                egui::Button::new(tr!("Сбросить сортировку")),
            )
            .clicked()
        {
            *sort = Sort::default();
            ui.close();
        }
    });
    set_columns(stored);
    fit
}

pub fn paint_pin(painter: &egui::Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    painter.rect_filled(
        Rect::from_center_size(center + egui::vec2(0.0, -2.5), egui::vec2(5.0, 5.0)),
        1.0,
        color,
    );
    painter.line_segment(
        [
            center + egui::vec2(-4.0, 0.5),
            center + egui::vec2(4.0, 0.5),
        ],
        stroke,
    );
    painter.line_segment(
        [center + egui::vec2(0.0, 0.5), center + egui::vec2(0.0, 5.5)],
        stroke,
    );
}

pub enum ProfileAction {
    Select(usize),
    Edit(usize),
    Rename(usize, String),
    Delete(usize),
    Add,
}

#[derive(Clone)]
enum ProfileAsk {
    Rename(usize, String),
    Delete(usize),
}

pub fn profile_list(
    ui: &mut Ui,
    id: egui::Id,
    names: &[(String, String)],
    selected: Option<usize>,
    add: &str,
) -> Option<ProfileAction> {
    let mut action = None;
    ui.set_min_width(240.0);
    ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
    let filter_id = id.with("filter");
    let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    let field = ui.add(
        TextEdit::singleline(&mut filter)
            .hint_text(tr!("Поиск…"))
            .margin(egui::Margin::symmetric(8, 4))
            .min_size(egui::vec2(0.0, 26.0))
            .align(egui::Align2::LEFT_CENTER)
            .desired_width(f32::INFINITY),
    );
    if ui
        .data(|d| d.get_temp::<bool>(id.with("focused")))
        .is_none()
    {
        field.request_focus();
        ui.data_mut(|d| {
            d.insert_temp(id.with("focused"), true);
            d.remove::<(usize, egui::Pos2)>(id.with("menu"));
        });
    }
    let needle = filter.to_lowercase();
    let mut shown: Vec<usize> = (0..names.len())
        .filter(|i| {
            needle.is_empty()
                || names[*i].0.to_lowercase().contains(&needle)
                || names[*i].1.to_lowercase().contains(&needle)
        })
        .collect();
    shown.sort_by_cached_key(|i| names[*i].0.to_lowercase());
    if field.lost_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && let Some(first) = shown.first()
    {
        action = Some(ProfileAction::Select(*first));
    }
    ui.data_mut(|d| d.insert_temp(filter_id, filter));
    ui.add_space(2.0);
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.weak(tr!("Ничего не найдено"));
            }
            for index in shown {
                let (name, kind) = &names[index];
                let text = if kind.is_empty() {
                    name.clone()
                } else {
                    format!("{name}  ·  {kind}")
                };
                let response = ui.add(
                    egui::Button::selectable(selected == Some(index), text)
                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                );
                if response.clicked() {
                    action = Some(ProfileAction::Select(index));
                }
                if response.secondary_clicked() {
                    let pos = ui
                        .input(|i| i.pointer.interact_pos())
                        .unwrap_or(response.rect.center());
                    ui.data_mut(|d| d.insert_temp(id.with("menu"), (index, pos)));
                }
            }
        });
    if let Some((index, pos)) = ui.data(|d| d.get_temp::<(usize, egui::Pos2)>(id.with("menu"))) {
        let name = names.get(index).map(|n| n.0.clone()).unwrap_or_default();
        let area = egui::Area::new(id.with("menu_area"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(170.0);
                    if let Some(a) = profile_menu(ui, id, index, &name) {
                        action = Some(a);
                    }
                });
            });
        let clicked_outside = ui.input(|i| i.pointer.any_pressed())
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| !area.response.rect.contains(p));
        if clicked_outside && !ui.input(|i| i.pointer.secondary_pressed()) {
            ui.data_mut(|d| d.remove::<(usize, egui::Pos2)>(id.with("menu")));
        }
    }
    ui.separator();
    if ui.button(add).clicked() {
        action = Some(ProfileAction::Add);
    }
    if action.is_some()
        || ui
            .data(|d| d.get_temp::<ProfileAsk>(id.with("ask")))
            .is_some()
    {
        ui.data_mut(|d| {
            d.remove::<String>(filter_id);
            d.remove::<bool>(id.with("focused"));
            d.remove::<(usize, egui::Pos2)>(id.with("menu"));
        });
        ui.close();
    }
    action
}

pub fn profile_context(
    ui: &mut Ui,
    id: egui::Id,
    response: &egui::Response,
    index: usize,
    name: &str,
) -> Option<ProfileAction> {
    let key = id.with("context");
    if response.secondary_clicked() {
        let pos = ui
            .input(|i| i.pointer.interact_pos())
            .unwrap_or(response.rect.left_bottom());
        ui.data_mut(|d| d.insert_temp(key, pos));
        return None;
    }
    let pos = ui.data(|d| d.get_temp::<egui::Pos2>(key))?;
    let mut action = None;
    let area = egui::Area::new(key.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(170.0);
                action = profile_menu(ui, id, index, name);
            });
        });
    let outside = ui.input(|i| i.pointer.any_pressed())
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| !area.response.rect.contains(p));
    let asked = ui.data(|d| d.get_temp::<ProfileAsk>(id.with("ask")).is_some());
    if action.is_some() || asked || outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.remove::<egui::Pos2>(key));
    }
    action
}

pub fn profile_menu(ui: &mut Ui, id: egui::Id, index: usize, name: &str) -> Option<ProfileAction> {
    let mut action = None;
    if ui.button(tr!("Редактировать…")).clicked() {
        action = Some(ProfileAction::Edit(index));
        ui.close();
    }
    if ui.button(tr!("Переименовать…")).clicked() {
        let ask = ProfileAsk::Rename(index, name.to_owned());
        ui.data_mut(|d| d.insert_temp(id.with("ask"), ask));
        ui.close();
    }
    if ui.button(tr!("Удалить…")).clicked() {
        ui.data_mut(|d| d.insert_temp(id.with("ask"), ProfileAsk::Delete(index)));
        ui.close();
    }
    action
}

pub fn profile_dialogs(
    ctx: &egui::Context,
    pal: &Palette,
    id: egui::Id,
    names: &[(String, String)],
) -> Option<ProfileAction> {
    let mut ask: ProfileAsk = ctx.data(|d| d.get_temp(id.with("ask")))?;
    let mut action = None;
    let mut done = false;
    let modal = egui::Modal::new(id.with("modal"))
        .frame(window_frame(pal))
        .show(ctx, |ui| {
            ui.set_width(340.0);
            match &mut ask {
                ProfileAsk::Rename(index, name) => {
                    title(ui, pal, tr!("Переименовать профиль"));
                    ui.add_space(8.0);
                    let field = ui.add(TextEdit::singleline(name).desired_width(f32::INFINITY));
                    field.request_focus();
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let ok = ui
                            .add(primary(tr!("Переименовать"), pal).min_size(ACTION))
                            .lit(ui)
                            .clicked()
                            || enter;
                        if ok && !name.trim().is_empty() {
                            action = Some(ProfileAction::Rename(*index, name.trim().to_owned()));
                        }
                        if ui
                            .add(secondary(tr!("Отмена"), pal).min_size(ACTION))
                            .clicked()
                        {
                            done = true;
                        }
                    });
                }
                ProfileAsk::Delete(index) => {
                    let title = names.get(*index).map(|n| n.0.clone()).unwrap_or_default();
                    ui.label(
                        RichText::new(tr!("Удалить профиль?"))
                            .size(15.5)
                            .color(pal.text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(tf!(
                            "«{title}» будет удалён вместе с паролем.",
                            "\"{title}\" will be deleted together with its password."
                        ))
                        .color(pal.subtext),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(danger(tr!("Удалить"), pal).min_size(ACTION))
                            .lit(ui)
                            .clicked()
                        {
                            action = Some(ProfileAction::Delete(*index));
                        }
                        if ui
                            .add(secondary(tr!("Отмена"), pal).min_size(ACTION))
                            .clicked()
                        {
                            done = true;
                        }
                    });
                }
            }
        });
    if done || action.is_some() || modal.should_close() {
        ctx.data_mut(|d| d.remove::<ProfileAsk>(id.with("ask")));
    } else {
        ctx.data_mut(|d| d.insert_temp(id.with("ask"), ask));
    }
    action
}

pub fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        tf!("{bytes} Б", "{bytes} B")
    } else if bytes < 1024 * 1024 {
        tf!("{:.1} КБ", "{:.1} KB", bytes as f64 / 1024.0)
    } else {
        tf!("{:.1} МБ", "{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn danger_button(ui: &mut Ui, pal: &Palette, text: &str) -> bool {
    let response = ui.add(danger(text, pal).min_size(ACTION));
    Lit::lit(response, ui).clicked()
}

pub fn dialog_button(ui: &mut Ui, pal: &Palette, text: &str, primary: bool) -> bool {
    let button = if primary {
        self::primary(text, pal)
    } else {
        self::secondary(text, pal)
    };
    let response = ui.add(button.min_size(ACTION));
    if primary {
        Lit::lit(response, ui).clicked()
    } else {
        response.clicked()
    }
}

pub fn window_header(ui: &mut Ui, pal: &Palette, title: &str, closable: bool) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        self::title(ui, pal, title);
        if closable {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = icon_button(ui, pal, Icon::Close).clicked();
            });
        }
    });
    ui.add_space(4.0);
    close
}

pub fn dialog(id: impl std::hash::Hash + std::fmt::Debug, pal: &Palette) -> Modal {
    Modal::new(egui::Id::unique(id)).frame(window_frame(pal))
}

#[derive(Clone, Copy)]
pub enum Icon {
    Up,
    Down,
    Close,
    Dock,
}

pub fn icon_button(ui: &mut Ui, pal: &Palette, icon: Icon) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            match icon {
                Icon::Up => tr!("Вверх"),
                Icon::Down => tr!("Вниз"),
                Icon::Close => tr!("Закрыть"),
                Icon::Dock => tr!("Закрепить"),
            },
        )
    });
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            6.0,
            if response.is_pointer_button_down_on() {
                pal.surface_hover
            } else {
                pal.surface
            },
        );
    }
    let color = if response.hovered() {
        pal.text
    } else {
        pal.subtext
    };
    let stroke = Stroke::new(1.5, color);
    let c = rect.center();
    let d = 4.0;
    if matches!(icon, Icon::Dock) {
        let frame = Rect::from_center_size(c, egui::vec2(11.0, 9.0));
        ui.painter().rect_stroke(
            frame,
            1.5,
            Stroke::new(1.2, color),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_filled(
            Rect::from_min_max(frame.min, egui::pos2(frame.min.x + 4.0, frame.max.y)),
            1.5,
            color,
        );
        return response;
    }
    let lines = match icon {
        Icon::Up => [
            [c + egui::vec2(-d, 2.0), c + egui::vec2(0.0, -2.5)],
            [c + egui::vec2(0.0, -2.5), c + egui::vec2(d, 2.0)],
        ],
        Icon::Down => [
            [c + egui::vec2(-d, -2.0), c + egui::vec2(0.0, 2.5)],
            [c + egui::vec2(0.0, 2.5), c + egui::vec2(d, -2.0)],
        ],
        Icon::Close | Icon::Dock => [
            [c + egui::vec2(-d, -d), c + egui::vec2(d, d)],
            [c + egui::vec2(d, -d), c + egui::vec2(-d, d)],
        ],
    };
    for line in lines {
        ui.painter().line_segment(line, stroke);
    }
    response
}
