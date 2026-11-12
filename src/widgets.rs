use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextEdit, Ui};

use crate::theme::Palette;

pub fn port_field(ui: &mut Ui, port: &mut u16, width: f32) -> egui::Response {
    let id = ui.scope_id().with(("tror_port", port as *const u16 as usize));
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
    for format in ["%d.%m.%Y %H:%M:%S", "%d.%m.%Y %H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
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
                ui.label(RichText::new(label).size(12.0).color(pal.subtext));
            }
            ui.end_row();
            for (shift, who) in [(6, "Владелец"), (3, "Группа"), (0, "Остальные")] {
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
}
