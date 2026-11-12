use std::collections::BTreeMap;
use std::path::Path;

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy)]
pub struct Palette {
    pub base: Color32,
    pub mantle: Color32,
    pub crust: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub current_line: Color32,
    pub selection: Color32,
    pub match_bg: Color32,
    pub match_current: Color32,
    pub warn: Color32,
    pub smart: Color32,
}

pub const PARTS: [(&str, &str); 16] = [
    ("base", "Фон редактора"),
    ("mantle", "Фон панелей и меню"),
    ("crust", "Фон заголовков и наведения"),
    ("surface", "Кнопки и поля"),
    ("surface_hover", "Кнопки при наведении"),
    ("border", "Границы и разделители"),
    ("text", "Основной текст"),
    ("subtext", "Вторичный текст"),
    ("faint", "Номера строк и подсказки"),
    ("accent", "Акцент"),
    ("current_line", "Текущая строка"),
    ("selection", "Выделение"),
    ("match_bg", "Найденные совпадения"),
    ("match_current", "Текущее совпадение"),
    ("warn", "Предупреждения"),
    ("smart", "Совпадения с выделенным"),
];

impl Palette {
    pub fn part(&mut self, index: usize) -> &mut Color32 {
        match index {
            0 => &mut self.base,
            1 => &mut self.mantle,
            2 => &mut self.crust,
            3 => &mut self.surface,
            4 => &mut self.surface_hover,
            5 => &mut self.border,
            6 => &mut self.text,
            7 => &mut self.subtext,
            8 => &mut self.faint,
            9 => &mut self.accent,
            10 => &mut self.current_line,
            11 => &mut self.selection,
            12 => &mut self.match_bg,
            13 => &mut self.match_current,
            14 => &mut self.warn,
            _ => &mut self.smart,
        }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const ACCENTS: [(&str, u32, u32); 5] = [
    ("Сирень", 0x8ea6ff, 0x4f6bed),
    ("Бирюза", 0x5fd4c4, 0x0f9d8a),
    ("Янтарь", 0xf0b55a, 0xc27a0a),
    ("Роза", 0xf28fb0, 0xd6336c),
    ("Зелень", 0x8fd694, 0x2f9e44),
];

pub fn palette(dark: bool, accent: usize, colors: &BTreeMap<String, [u8; 3]>) -> Palette {
    let (_, on_dark, on_light) = ACCENTS[accent.min(ACCENTS.len() - 1)];
    let mut palette = base_palette(dark);
    palette.accent = rgb(if dark { on_dark } else { on_light });
    for (index, (key, _)) in PARTS.iter().enumerate() {
        if let Some([r, g, b]) = colors.get(*key) {
            *palette.part(index) = Color32::from_rgb(*r, *g, *b);
        }
    }
    palette
}

fn base_palette(dark: bool) -> Palette {
    if dark {
        Palette {
            base: rgb(0x16181d),
            mantle: rgb(0x101216),
            crust: rgb(0x1b1e25),
            surface: rgb(0x262a33),
            surface_hover: rgb(0x30353f),
            border: rgb(0x24272f),
            text: rgb(0xe4e7ec),
            subtext: rgb(0xa0a7b4),
            faint: rgb(0x596070),
            accent: rgb(0x8ea6ff),
            current_line: rgb(0x1c1f27),
            selection: rgb(0x2f3d66),
            match_bg: rgb(0x4a4326),
            match_current: rgb(0x8a6a1f),
            warn: rgb(0xf2c46d),
            smart: rgb(0x29473a),
        }
    } else {
        Palette {
            base: rgb(0xffffff),
            mantle: rgb(0xf4f5f8),
            crust: rgb(0xeaecf1),
            surface: rgb(0xe1e4eb),
            surface_hover: rgb(0xd5d9e2),
            border: rgb(0xdfe2e9),
            text: rgb(0x24272e),
            subtext: rgb(0x5c6370),
            faint: rgb(0xa2a8b3),
            accent: rgb(0x4f6bed),
            current_line: rgb(0xf3f5f9),
            selection: rgb(0xcfdaff),
            match_bg: rgb(0xfbe9a8),
            match_current: rgb(0xf5c542),
            warn: rgb(0xc9811a),
            smart: rgb(0xc9efcf),
        }
    }
}

pub fn medium() -> egui::FontFamily {
    egui::FontFamily::Name("medium".into())
}

static TERM_FONTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static PENDING_FONTS: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

pub fn commit_term_fonts() {
    if let Some(list) = PENDING_FONTS.lock().ok().and_then(|mut p| p.take())
        && let Ok(mut fonts) = TERM_FONTS.lock()
    {
        *fonts = list;
    }
}

pub fn term_family(path: &str) -> Option<egui::FontFamily> {
    if path.is_empty() {
        return None;
    }
    let fonts = TERM_FONTS.lock().ok()?;
    fonts
        .iter()
        .position(|p| p == path)
        .map(|i| egui::FontFamily::Name(format!("term{i}").into()))
}

pub fn install_fonts(
    ctx: &egui::Context,
    ui_font: Option<&Path>,
    code_font: Option<&Path>,
    term_fonts: &[std::path::PathBuf],
) -> Result<(), String> {
    use egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    let mut failed = None;
    let mut custom = |name: &str, path: Option<&Path>, fonts: &mut FontDefinitions| {
        let Some(path) = path else {
            return false;
        };
        match std::fs::read(path) {
            Ok(bytes)
                if bytes.len() > 4
                    && matches!(&bytes[..4], [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf") =>
            {
                fonts.font_data.insert(
                    name.to_owned(),
                    std::sync::Arc::new(FontData::from_owned(bytes)),
                );
                true
            }
            Ok(_) => {
                failed = Some(format!("{} — не файл шрифта TTF/OTF", path.display()));
                false
            }
            Err(e) => {
                failed = Some(format!("{}: {e}", path.display()));
                false
            }
        }
    };
    let own_ui = custom("user-ui", ui_font, &mut fonts);
    let own_code = custom("user-code", code_font, &mut fonts);
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    };
    add("inter", include_bytes!("../assets/fonts/Inter-Regular.ttf"));
    add(
        "inter-medium",
        include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    );
    add(
        "jbmono",
        include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    );
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut with = |family: FontFamily, first: &[&str]| {
        let mut list: Vec<String> = first.iter().map(|s| (*s).to_owned()).collect();
        list.extend(fallback.iter().cloned());
        fonts.families.insert(family, list);
    };
    if own_ui {
        with(FontFamily::Proportional, &["user-ui", "inter"]);
        with(medium(), &["user-ui", "inter"]);
    } else {
        with(FontFamily::Proportional, &["inter"]);
        with(medium(), &["inter-medium", "inter"]);
    }
    if own_code {
        with(FontFamily::Monospace, &["user-code", "jbmono", "Hack"]);
    } else {
        with(FontFamily::Monospace, &["jbmono", "Hack"]);
    }
    let mut registered = Vec::new();
    for path in term_fonts {
        let name = format!("term-font-{}", registered.len());
        if let Ok(bytes) = std::fs::read(path)
            && bytes.len() > 4
            && matches!(&bytes[..4], [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf")
        {
            fonts
                .font_data
                .insert(name.clone(), std::sync::Arc::new(FontData::from_owned(bytes)));
            let family = FontFamily::Name(format!("term{}", registered.len()).into());
            let mut list = vec![name, "jbmono".to_owned(), "Hack".to_owned()];
            list.extend(fallback.iter().cloned());
            fonts.families.insert(family, list);
            registered.push(path.display().to_string());
        }
    }
    ctx.set_fonts(fonts);
    if let Ok(mut list) = TERM_FONTS.lock() {
        list.clear();
    }
    if let Ok(mut pending) = PENDING_FONTS.lock() {
        *pending = Some(registered);
    }
    failed.map_or(Ok(()), Err)
}

pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

pub fn apply(ctx: &egui::Context, p: &Palette, dark: bool, radius: u8, ui_size: f32) {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let window_radius = CornerRadius::same(radius.saturating_add(3));
    let menu_radius = CornerRadius::same(radius.saturating_add(1));
    let radius = CornerRadius::same(radius);
    let grip = blend(p.border, p.accent, 0.45);

    v.panel_fill = p.mantle;
    v.window_fill = p.mantle;
    v.extreme_bg_color = p.base;
    v.faint_bg_color = p.crust;
    v.code_bg_color = p.crust;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = window_radius;
    v.menu_corner_radius = menu_radius;
    v.window_shadow.color = Color32::from_black_alpha(if dark { 110 } else { 45 });
    v.popup_shadow.color = Color32::from_black_alpha(if dark { 90 } else { 35 });
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);

    v.widgets.noninteractive.bg_fill = p.mantle;
    v.widgets.noninteractive.weak_bg_fill = p.mantle;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.subtext);
    v.widgets.noninteractive.corner_radius = radius;

    v.widgets.inactive.bg_fill = p.surface;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = radius;

    v.widgets.hovered.bg_fill = p.surface_hover;
    v.widgets.hovered.weak_bg_fill = p.surface;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, grip);
    v.widgets.hovered.corner_radius = radius;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = p.surface_hover;
    v.widgets.active.weak_bg_fill = p.surface_hover;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.fg_stroke = Stroke::new(1.5, p.accent);
    v.widgets.active.corner_radius = radius;
    v.widgets.active.expansion = 0.0;

    v.widgets.open.bg_fill = p.surface;
    v.widgets.open.weak_bg_fill = p.surface;
    v.widgets.open.bg_stroke = Stroke::NONE;
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.open.corner_radius = radius;

    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.interact_size.y = 26.0;
        for (style, size) in [
            (egui::TextStyle::Body, ui_size),
            (egui::TextStyle::Button, ui_size),
            (egui::TextStyle::Small, ui_size - 2.0),
            (egui::TextStyle::Heading, ui_size + 4.5),
            (egui::TextStyle::Monospace, ui_size - 0.5),
        ] {
            let family = if style == egui::TextStyle::Monospace {
                egui::FontFamily::Monospace
            } else {
                egui::FontFamily::Proportional
            };
            s.text_styles.insert(style, egui::FontId::new(size, family));
        }
    });
}
