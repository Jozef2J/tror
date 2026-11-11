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

pub fn palette(dark: bool, accent: usize) -> Palette {
    let (_, on_dark, on_light) = ACCENTS[accent.min(ACCENTS.len() - 1)];
    let mut palette = base_palette(dark);
    palette.accent = rgb(if dark { on_dark } else { on_light });
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
        }
    }
}

pub fn medium() -> egui::FontFamily {
    egui::FontFamily::Name("medium".into())
}

pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
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
    with(FontFamily::Proportional, &["inter"]);
    with(medium(), &["inter-medium", "inter"]);
    with(FontFamily::Monospace, &["jbmono", "Hack"]);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, dark: bool, accent: usize) {
    let p = palette(dark, accent);
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let radius = CornerRadius::same(7);

    v.panel_fill = p.mantle;
    v.window_fill = p.mantle;
    v.extreme_bg_color = p.base;
    v.faint_bg_color = p.crust;
    v.code_bg_color = p.crust;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
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
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = radius;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = p.surface_hover;
    v.widgets.active.weak_bg_fill = p.surface_hover;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
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
            (egui::TextStyle::Body, 13.5),
            (egui::TextStyle::Button, 13.5),
            (egui::TextStyle::Small, 11.5),
            (egui::TextStyle::Heading, 18.0),
            (egui::TextStyle::Monospace, 13.0),
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
