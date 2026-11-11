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

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            base: rgb(0x1e1e2e),
            mantle: rgb(0x181825),
            crust: rgb(0x11111b),
            surface: rgb(0x313244),
            surface_hover: rgb(0x3b3c52),
            border: rgb(0x2a2b3c),
            text: rgb(0xcdd6f4),
            subtext: rgb(0xa6adc8),
            faint: rgb(0x585b70),
            accent: rgb(0x89b4fa),
            current_line: rgb(0x262738),
            selection: rgb(0x414668),
            match_bg: rgb(0x4f4a32),
            match_current: rgb(0x8a6a1f),
            warn: rgb(0xf9e2af),
        }
    } else {
        Palette {
            base: rgb(0xfbfbfd),
            mantle: rgb(0xf1f2f6),
            crust: rgb(0xe6e8ee),
            surface: rgb(0xdfe2ea),
            surface_hover: rgb(0xd3d7e1),
            border: rgb(0xdcdfe7),
            text: rgb(0x3b3e52),
            subtext: rgb(0x6c6f85),
            faint: rgb(0xa3a7b8),
            accent: rgb(0x1e66f5),
            current_line: rgb(0xf0f1f6),
            selection: rgb(0xc9d7fb),
            match_bg: rgb(0xfbe9a8),
            match_current: rgb(0xf5c542),
            warn: rgb(0xdf8e1d),
        }
    }
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let radius = CornerRadius::same(6);

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
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.menu_margin = egui::Margin::same(6);
    });
}
