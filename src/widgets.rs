use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextEdit, Ui};

use crate::theme::Palette;

pub fn primary(text: impl Into<String>, pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(pal.base)).fill(pal.accent)
}

pub fn secondary(text: impl Into<String>, pal: &Palette) -> egui::Button<'static> {
    egui::Button::new(text.into()).fill(pal.surface)
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
            .hint_text("порт"),
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
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if response.hovered() || open {
        ui.painter().rect_filled(rect, 5.0, pal.surface);
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
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if response.hovered() || open {
        ui.painter().rect_filled(rect, 6.0, pal.surface);
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

pub fn kind_color(name: &str, pal: &Palette) -> Color32 {
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
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, pal.surface);
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

pub fn parse_mode(text: &str) -> Option<u32> {
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
            for label in ["чтение", "запись", "выполнение"] {
                ui.label(RichText::new(label).small().color(pal.subtext));
            }
            ui.end_row();
            for (shift, who) in [(6, "Владелец"), (3, "Группа"), (0, "Остальные")]
            {
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
        ui.label("Числом");
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
            RichText::new(crate::remote::perm_string(*mode, false)[1..].to_owned())
                .monospace()
                .color(pal.subtext),
        );
    });
    changed
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
        assert_eq!(crate::remote::perm_string(0o754, true), "drwxr-xr--");
        let secs = parse_date("02.01.2026 03:04").unwrap();
        assert_eq!(date_label(secs), "02.01.2026 03:04");
        assert!(parse_date("вчера").is_none());
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
                .desired_width(46.0)
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

pub fn fit_columns(width: f32, shown: [bool; 3]) -> [f32; 3] {
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

pub fn column_header(ui: &mut Ui, pal: &Palette, shown: [bool; 3]) -> [f32; 3] {
    if !shown.contains(&true) {
        return [0.0; 3];
    }
    let mut stored = columns();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), Sense::hover());
    let fit = fit_columns(rect.width(), shown);
    let painter = ui.painter_at(rect);
    let font = egui::FontId::proportional(11.0);
    let room = rect.width() - 6.0 - NAME_MIN;
    let mut right = rect.right() - 6.0;
    for (index, label) in [(2, "Изменён"), (1, "Права"), (0, "Размер")] {
        let width = fit[index];
        if width <= 0.0 {
            continue;
        }
        let left = right - width;
        painter
            .with_clip_rect(Rect::from_min_max(
                egui::pos2(left, rect.top()),
                egui::pos2(right, rect.bottom()),
            ))
            .text(
                egui::pos2(right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                label,
                font.clone(),
                pal.faint,
            );
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
    }
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Имя",
        font,
        pal.faint,
    );
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
