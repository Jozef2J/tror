use std::{ops::RangeInclusive, sync::Arc};

use egui::{
    Align2, CornerRadius, Galley, Id, NumExt, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2,
    Visuals, epaint::TextShape, vec2,
};

use crate::{
    OverlayType, Split, Style, TabDestination, TabStyle,
    utils::{rect_set_size_centered, rect_stroke_box},
};

/// Paints the parts of a [`DockArea`](crate::DockArea): its tabs, tab bars, separators and drop
/// overlay. Set one with [`DockArea::painter`](crate::DockArea::painter). It paints the bands and
/// boundaries of a [`Sections`](crate::sections::Sections) stack too
/// ([`Sections::painter`](crate::sections::Sections::painter)) and the tools of a
/// [`ToolRow`](crate::toolbar::ToolRow) ([`ToolRow::painter`](crate::toolbar::ToolRow::painter)),
/// so that one painter gives a dock, its section stacks and its toolbars one look.
///
/// Every method has a default, the painting of [`StylePainter`] from the [`Style`] the part
/// receives, so an implementation overrides only the parts it paints itself. A method overridden
/// can still hand some cases to the default by calling it on [`StylePainter`].
///
/// The dock lays the parts out and takes their input; the painter only paints, except
/// [`tab_width`](Self::tab_width), which measures a tab before it is laid out. The parameter
/// structs are `#[non_exhaustive]`: they may gain fields in minor releases.
///
/// The buttons of the tab bars (add, close all, collapse), the hidden tab bar's handle, the tab
/// bar's scroll bar and the tab bodies' background are still painted from [`Style`]; a body's
/// border is the painter's ([`tab_body`](Self::tab_body)).
pub trait DockPainter {
    /// The width of a tab, padding and close button included. A tab bar filling its width
    /// ([`TabBarStyle::fill_tab_bar`](crate::TabBarStyle::fill_tab_bar)) can still make it
    /// wider.
    fn tab_width(&self, tab: &TabMeasure<'_>) -> f32 {
        let tab_style = tab.tab_style;
        let text_width = tab.title.size().x + 2.0 * tab_style.padding.at_least(0.0);
        tab_style
            .minimum_width
            .unwrap_or(0.0)
            .at_least(text_width + tab.close_width)
    }

    /// A tab bar's background, painted before its tabs.
    ///
    /// Its bottom corners are square where its bottom edge joins the body below it, and
    /// rounded like its top ones when its leaf is folded ([`TabBarPaint::folded`]).
    fn tab_bar(&self, painter: &Painter, bar: &TabBarPaint<'_>) {
        let attached = if bar.folded {
            Attached::Nothing
        } else {
            Attached::Below
        };
        painter.rect_filled(
            bar.rect,
            attached.corner_radius(bar.style.tab_bar.corner_radius),
            bar.style.tab_bar.bg_fill,
        );
    }

    /// A tab, with its title and its close button if it has one.
    ///
    /// A dragged tab is painted on a layer of its own, moved with the pointer. Whatever its
    /// state (hovered, selected, focused), a tab in its bar has square bottom corners, where it
    /// joins the body below it; a dragged tab, or one in the bar of a folded leaf
    /// ([`TabPaint::folded`]), has nothing below it and its bottom corners are rounded like its
    /// top ones. The close button's highlight follows the tab's right corners.
    fn tab(&self, painter: &Painter, tab: &TabPaint<'_>) {
        let state = tab.state;
        let selected = state.contains(PartState::SELECTED) || state.contains(PartState::DRAGGED);
        let dragged = state.contains(PartState::DRAGGED);
        let kb_focus = state.contains(PartState::FOCUSED);
        let tab_style = tab.tab_style;
        let look = if (selected && state.contains(PartState::LEAF_FOCUSED)) || dragged {
            if kb_focus {
                &tab_style.focused_with_kb_focus
            } else {
                &tab_style.focused
            }
        } else if selected {
            if kb_focus {
                &tab_style.active_with_kb_focus
            } else {
                &tab_style.active
            }
        } else if state.contains(PartState::HOVERED) {
            &tab_style.hovered
        } else if kb_focus {
            &tab_style.inactive_with_kb_focus
        } else {
            &tab_style.inactive
        };
        let rect = tab.rect;
        let attached = if dragged || tab.folded {
            Attached::Nothing
        } else {
            Attached::Below
        };
        let corner_radius = attached.corner_radius(look.corner_radius);

        // The whole tab first, its close button's highlight, then its outline over them, so
        // that they do not mix.
        painter.rect_filled(rect, corner_radius, look.bg_fill);
        if let Some(close) = tab.close
            && (close.state.contains(PartState::HOVERED)
                || close.state.contains(PartState::FOCUSED))
        {
            painter.rect_filled(
                close.rect,
                corners_within(close.rect, rect, corner_radius),
                tab.style.buttons.close_tab_bg_fill,
            );
        }
        let stroke_rect = rect_stroke_box(rect, 1.0);
        painter.rect_stroke(
            stroke_rect,
            corner_radius,
            Stroke::new(1.0_f32, look.outline_color),
            StrokeKind::Inside,
        );
        if attached == Attached::Below {
            // Joins the tab to its body.
            painter.hline(
                RangeInclusive::new(
                    stroke_rect.min.x + f32::max(corner_radius.sw.into(), 1.5),
                    stroke_rect.max.x - f32::max(corner_radius.se.into(), 1.5),
                ),
                stroke_rect.bottom(),
                Stroke::new(2.0_f32, look.bg_fill),
            );
        }

        let close_width = if tab.close.is_some() {
            Style::TAB_CLOSE_BUTTON_SIZE.min(tab.style.tab_bar.height)
        } else {
            0.0
        };
        let mut text_rect = rect;
        text_rect.set_width(text_rect.width() - close_width);
        let x_spacing = tab_style.padding.at_least(0.0);
        let text_pos = Align2::CENTER_CENTER.pos_in_rect(&text_rect.shrink2(vec2(x_spacing, 0.0)))
            - tab.title.size() / 2.0;
        painter.add(TextShape::new(
            text_pos,
            Arc::clone(tab.title),
            look.text_color,
        ));

        if let Some(close) = tab.close {
            let buttons = &tab.style.buttons;
            let lit = close.state.contains(PartState::HOVERED)
                || close.state.contains(PartState::FOCUSED);
            let color = if lit {
                buttons.close_tab_active_color
            } else {
                buttons.close_tab_color
            };
            let mut x_rect = close.rect;
            rect_set_size_centered(&mut x_rect, Vec2::splat(Style::TAB_CLOSE_X_SIZE));
            painter.line_segment(
                [x_rect.left_top(), x_rect.right_bottom()],
                Stroke::new(1.0_f32, color),
            );
            painter.line_segment(
                [x_rect.right_top(), x_rect.left_bottom()],
                Stroke::new(1.0_f32, color),
            );
        }

        // The line under the bar, under every tab but the selected one: it parts the bar from
        // the body, so none without one.
        if attached == Attached::Below && (!selected || tab_style.hline_below_active_tab_name) {
            let px = painter.pixels_per_point().recip();
            painter.hline(
                rect.x_range(),
                tab.bar.bottom() - px,
                (px, tab.style.tab_bar.hline_color),
            );
        }
    }

    /// The part of a tab bar right of its last tab, painted after the tabs. Its rect is empty
    /// when the tabs fill the bar. The default paints the line under the bar, which parts it
    /// from the body: none when the leaf is folded.
    fn tab_bar_tail(&self, painter: &Painter, tail: &TabBarTailPaint<'_>) {
        if tail.folded {
            return;
        }
        let px = painter.pixels_per_point().recip();
        painter.hline(
            tail.rect.x_range(),
            tail.rect.bottom() - px,
            (px, tail.style.tab_bar.hline_color),
        );
    }

    /// A leaf's body border, painted over the body's background and before its contents.
    ///
    /// The painter is clipped to the body ([`TabBodyPaint::rect`]); paint outside it with
    /// [`Painter::with_clip_rect`]. The default strokes
    /// [`TabBodyStyle::stroke`](crate::TabBodyStyle::stroke) inside the body, its top edge hidden
    /// under the tab bar, and paints nothing when the stroke is empty, as it is by default.
    fn tab_body(&self, painter: &Painter, body: &TabBodyPaint<'_>) {
        let stroke = body.tab_style.tab_body.stroke;
        if stroke.is_empty() {
            return;
        }
        // The top edge goes up behind the clip rect, out of sight. By an even number of
        // points, so that an odd stroke width does not leave an anti-aliased line.
        let effective_stroke_width = (stroke.width / 2.0).ceil() * 2.0;
        let rect = Rect::from_min_max(
            body.rect.min - vec2(0.0, effective_stroke_width),
            body.rect.max,
        );
        painter.rect_stroke(
            rect_stroke_box(rect, stroke.width),
            body.corner_radius,
            stroke,
            StrokeKind::Inside,
        );
    }

    /// A separator between two nodes.
    fn separator(&self, painter: &Painter, separator: &SeparatorPaint<'_>) {
        let style = &separator.style.separator;
        let state = separator.state;
        let color = if state.contains(PartState::DRAGGED) {
            style.color_dragged
        } else if state.contains(PartState::HOVERED) || state.contains(PartState::FOCUSED) {
            style.color_hovered
        } else {
            style.color_idle
        };
        painter.rect_filled(separator.rect, CornerRadius::ZERO, color);
    }

    /// While a tab is dragged, the node or surface under the pointer, painted first.
    fn drop_target(&self, painter: &Painter, target: &DropTargetPaint<'_>) {
        let highlight = &target.style.overlay.hovered_leaf_highlight;
        painter.rect(
            target.rect.expand(highlight.expansion),
            highlight.corner_radius,
            highlight.color,
            highlight.stroke,
            StrokeKind::Inside,
        );
    }

    /// One of the drop overlay's buttons ([`OverlayType::Widgets`]):
    /// the middle one (no split) or a split's.
    fn drop_button(&self, painter: &Painter, button: &DropButtonPaint<'_>) {
        let visuals = &button.style.overlay;
        let button_stroke = Stroke::new(1.0_f32, visuals.button_color);
        painter.rect_stroke(
            button.rect,
            0.0,
            visuals.button_border_stroke,
            StrokeKind::Inside,
        );
        let rect = button.rect.shrink(button.rect.width() * 0.1);
        painter.rect_stroke(rect, 0.0, button_stroke, StrokeKind::Inside);
        let rim = Rect::from_two_pos(rect.min, rect.lerp_inside(vec2(1.0, 0.1)));
        painter.rect(
            rim,
            0.0,
            visuals.button_color,
            Stroke::NONE,
            StrokeKind::Inside,
        );

        if let Some(split) = button.split {
            for line in DASHED_LINE_ALPHAS.chunks(2) {
                let start = rect.lerp_inside(lerp_vec(split, line[0]));
                let end = rect.lerp_inside(lerp_vec(split, line[1]));
                painter.line_segment([start, end], button_stroke);
            }
        }
        if button.hovered {
            let vertical_alphas = vec2(1.0, 0.5);
            let horizontal_alphas = vec2(0.5, 1.0);
            let rect = match button.split {
                Some(Split::Above) => Rect::from_min_size(rect.min, rect.size() * vertical_alphas),
                Some(Split::Left) => Rect::from_min_size(rect.min, rect.size() * horizontal_alphas),
                Some(Split::Below) => {
                    let min = rect.lerp_inside(lerp_vec(Split::Below, 0.0));
                    Rect::from_min_size(min, rect.size() * vertical_alphas)
                }
                Some(Split::Right) => {
                    let min = rect.lerp_inside(lerp_vec(Split::Right, 0.0));
                    Rect::from_min_size(min, rect.size() * horizontal_alphas)
                }
                None => rect,
            };
            painter.rect_filled(rect, 0.0, visuals.selection_color);
        }
    }

    /// Where a dragged tab would land if released now, or why it may not
    /// ([`TabViewer::drop_allowed`](crate::TabViewer::drop_allowed)).
    ///
    /// With [`OverlayType::Zones`], the default tints the region the tab would take
    /// ([`OverlayStyle::zone_fill`](crate::OverlayStyle::zone_fill)) and borders it
    /// ([`OverlayStyle::zone_stroke`](crate::OverlayStyle::zone_stroke)).
    fn drop_preview(&self, painter: &Painter, drop: &DropPaint<'_>) {
        let overlay = &drop.style.overlay;
        if let TabDestination::Window(_) = drop.destination {
            painter.rect_stroke(
                drop.rect,
                0.0,
                Stroke::new(overlay.selection_stroke_width, overlay.selection_color),
                StrokeKind::Inside,
            );
        } else if overlay.overlay_type == OverlayType::Zones {
            painter.rect(
                drop.rect,
                overlay.zone_corner_radius,
                overlay.zone_fill,
                overlay.zone_stroke,
                StrokeKind::Inside,
            );
        } else if !drop.from_buttons {
            painter.rect_filled(drop.rect, 0.0, overlay.selection_color);
        }

        let Some(refusal) = drop.refusal else {
            return;
        };
        let visuals = drop.visuals;
        let color = visuals.error_fg_color;
        painter.rect(
            drop.rect,
            0.0,
            color.gamma_multiply(0.25),
            Stroke::new(overlay.selection_stroke_width.max(1.0), color),
            StrokeKind::Inside,
        );
        if let Some(reason) = &refusal.reason {
            let margin = vec2(6.0, 4.0);
            let min = refusal.pointer + vec2(16.0, 16.0);
            let frame = Rect::from_min_size(min, reason.size() + 2.0 * margin);
            painter.rect(
                frame,
                visuals.menu_corner_radius,
                visuals.window_fill(),
                Stroke::new(1.0, color),
                StrokeKind::Inside,
            );
            painter.galley(min + margin, Arc::clone(reason), visuals.text_color());
        }
    }

    /// The title band of a section in a [`Sections`](crate::sections::Sections) stack: its
    /// background, its disclosure mark, its title and its count. Painted from the band's
    /// [`egui::Style`], as a section stack has no dock [`Style`].
    fn section_band(&self, painter: &Painter, band: &SectionBandPaint<'_>) {
        let visuals = &band.style.visuals;
        let state = band.state;
        let rect = band.rect;
        let look = if state.contains(PartState::PRESSED) {
            &visuals.widgets.active
        } else if state.contains(PartState::HOVERED) {
            &visuals.widgets.hovered
        } else {
            &visuals.widgets.inactive
        };
        let fill = if state.contains(PartState::HOVERED) || state.contains(PartState::PRESSED) {
            look.weak_bg_fill
        } else {
            visuals.faint_bg_color
        };
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
        // The line over the band, which parts it from the body above.
        let px = painter.pixels_per_point().recip();
        painter.hline(
            rect.x_range(),
            rect.top() + px / 2.0,
            (px, visuals.widgets.noninteractive.bg_stroke.color),
        );

        let padding = band.style.spacing.item_spacing.x.max(4.0);
        let text_color = look.fg_stroke.color;

        // The disclosure mark: a triangle pointing right when folded, down when open.
        let side = (rect.height() * 0.4).clamp(4.0, 10.0);
        let mark = Rect::from_center_size(
            Pos2::new(rect.left() + padding + side / 2.0, rect.center().y),
            Vec2::splat(side),
        );
        let points = if band.open {
            vec![mark.left_top(), mark.right_top(), mark.center_bottom()]
        } else {
            vec![mark.left_top(), mark.right_center(), mark.left_bottom()]
        };
        painter.add(egui::Shape::convex_polygon(
            points,
            text_color,
            Stroke::NONE,
        ));

        // The count at the end, in a pill.
        let mut end = rect.right() - padding;
        if let Some(count) = band.count_text() {
            let font = egui::TextStyle::Small.resolve(band.style);
            let galley = painter.layout_no_wrap(count, font, text_color);
            let pill_size = galley.size() + vec2(8.0, 2.0);
            let pill = Rect::from_min_size(
                Pos2::new(end - pill_size.x, rect.center().y - pill_size.y / 2.0),
                pill_size,
            );
            painter.rect_filled(pill, pill.height() / 2.0, visuals.widgets.inactive.bg_fill);
            painter.galley(pill.center() - galley.size() / 2.0, galley, text_color);
            end = pill.left() - padding;
        }

        // The title between them, clipped to its room.
        let start = mark.right() + padding;
        let font = egui::TextStyle::Button.resolve(band.style);
        let galley = painter.layout_no_wrap(band.title.to_owned(), font, text_color);
        let room = Rect::from_x_y_ranges(start..=end.max(start), rect.y_range());
        painter
            .with_clip_rect(room.intersect(painter.clip_rect()))
            .galley(
                Pos2::new(start, rect.center().y - galley.size().y / 2.0),
                galley,
                text_color,
            );

        if state.contains(PartState::FOCUSED) {
            painter.rect_stroke(
                rect.shrink(1.0),
                CornerRadius::ZERO,
                visuals.selection.stroke,
                StrokeKind::Inside,
            );
        }
    }

    /// A boundary between two sections of a [`Sections`](crate::sections::Sections) stack, which
    /// resizes the open section above it. Painted after the bands, so that a focus ring shows
    /// over them. The default paints nothing at rest: the bands' own lines part the sections.
    fn section_boundary(&self, painter: &Painter, boundary: &SectionBoundaryPaint<'_>) {
        let visuals = &boundary.style.visuals;
        let state = boundary.state;
        if state.contains(PartState::FOCUSED) {
            painter.rect_stroke(
                boundary.rect,
                CornerRadius::ZERO,
                visuals.selection.stroke,
                StrokeKind::Inside,
            );
        } else if state.contains(PartState::DRAGGED)
            || state.contains(PartState::PRESSED)
            || state.contains(PartState::HOVERED)
        {
            painter.hline(
                boundary.rect.x_range(),
                boundary.line,
                visuals.widgets.hovered.bg_stroke,
            );
        }
    }

    /// A tool of a [`ToolRow`](crate::toolbar::ToolRow) (a button, a toggle, a menu), or a row
    /// of its More menu ([`ToolItemPaint::in_menu`]): its background, icon (or a spinner while
    /// busy), label and mark. The row lays it out: the painter paints at the places it is given.
    /// Painted from the row's [`egui::Style`].
    fn tool_item(&self, painter: &Painter, item: &ToolItemPaint<'_>) {
        let visuals = &item.style.visuals;
        let state = item.state;
        let enabled = !state.contains(PartState::DISABLED);
        let on = state.contains(PartState::SELECTED);
        let radius = visuals.widgets.inactive.corner_radius;
        let look = if state.contains(PartState::PRESSED) {
            &visuals.widgets.active
        } else {
            &visuals.widgets.hovered
        };
        let lit =
            enabled && (state.contains(PartState::HOVERED) || state.contains(PartState::PRESSED));
        // A toggle on is framed in the row; in a menu its tick says it.
        let framed = on && !item.in_menu;
        if framed {
            painter.rect(
                item.rect,
                radius,
                visuals.selection.bg_fill,
                Stroke::new(1.0, visuals.selection.stroke.color),
                StrokeKind::Inside,
            );
        } else if lit {
            painter.rect_filled(item.rect, radius, look.weak_bg_fill);
        }
        let color = if !enabled {
            visuals.weak_text_color()
        } else if framed {
            visuals.selection.stroke.color
        } else if lit {
            look.fg_stroke.color
        } else {
            visuals.widgets.inactive.fg_stroke.color
        };
        if item.busy {
            paint_spinner(painter, item.icon_rect, color);
        } else if let Some(icon) = item.icon {
            icon.paint(painter, item.icon_rect, color, enabled);
        }
        if let Some(label) = item.label {
            painter.galley(item.label_pos, Arc::clone(label), color);
        }
        if let Some(shortcut) = item.shortcut {
            let weak = if enabled {
                visuals.weak_text_color()
            } else {
                color
            };
            painter.galley(item.shortcut_pos, Arc::clone(shortcut), weak);
        }
        if let Some(mark) = item.mark {
            let stroke = Stroke::new(1.5, color);
            if item.kind == crate::toolbar::ToolKind::Menu {
                // A drop-down mark in the row, a submenu's arrow in a menu.
                let points = if item.in_menu {
                    vec![mark.left_top(), mark.right_center(), mark.left_bottom()]
                } else {
                    let m = mark.shrink2(vec2(0.0, mark.height() / 4.0));
                    vec![m.left_top(), m.right_top(), m.center_bottom()]
                };
                painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
            } else if on {
                // A tick.
                let m = mark.shrink(mark.width() / 8.0);
                painter.line(
                    vec![
                        m.lerp_inside(vec2(0.0, 0.55)),
                        m.lerp_inside(vec2(0.4, 0.9)),
                        m.lerp_inside(vec2(1.0, 0.1)),
                    ],
                    stroke,
                );
            }
        }
        if state.contains(PartState::FOCUSED) {
            painter.rect_stroke(
                item.rect.shrink(1.0),
                radius,
                visuals.selection.stroke,
                StrokeKind::Inside,
            );
        }
    }

    /// The More button at the end of a [`ToolRow`](crate::toolbar::ToolRow), shown when some
    /// tools do not fit: it opens them as a menu. The default paints three dots, along the row.
    fn tool_more(&self, painter: &Painter, more: &ToolMorePaint<'_>) {
        let visuals = &more.style.visuals;
        let state = more.state;
        let radius = visuals.widgets.inactive.corner_radius;
        let open = state.contains(PartState::SELECTED);
        let look = if state.contains(PartState::PRESSED) || open {
            &visuals.widgets.active
        } else {
            &visuals.widgets.hovered
        };
        let lit = open || state.contains(PartState::HOVERED) || state.contains(PartState::PRESSED);
        if lit {
            painter.rect_filled(more.rect, radius, look.weak_bg_fill);
        }
        let color = if lit {
            look.fg_stroke.color
        } else {
            visuals.widgets.inactive.fg_stroke.color
        };
        let r = (more.rect.height().min(more.rect.width()) / 14.0).max(1.0);
        let step = if more.vertical {
            vec2(0.0, 3.0 * r)
        } else {
            vec2(3.0 * r, 0.0)
        };
        let c = more.rect.center();
        for at in [c - step, c, c + step] {
            painter.circle_filled(at, r, color);
        }
        if state.contains(PartState::FOCUSED) {
            painter.rect_stroke(
                more.rect.shrink(1.0),
                radius,
                visuals.selection.stroke,
                StrokeKind::Inside,
            );
        }
    }

    /// The line between two groups of a [`ToolRow`](crate::toolbar::ToolRow): across the row,
    /// in the middle of its room.
    fn tool_separator(&self, painter: &Painter, separator: &ToolSeparatorPaint<'_>) {
        let rect = separator.rect;
        let stroke = separator.style.visuals.widgets.noninteractive.bg_stroke;
        if separator.vertical {
            let inset = rect.width() / 8.0;
            painter.hline(
                rect.left() + inset..=rect.right() - inset,
                rect.center().y,
                stroke,
            );
        } else {
            let inset = rect.height() / 8.0;
            painter.vline(
                rect.center().x,
                rect.top() + inset..=rect.bottom() - inset,
                stroke,
            );
        }
    }
}

/// A busy tool's spinner in `rect`, as egui's: an arc turning with time.
fn paint_spinner(painter: &Painter, rect: Rect, color: egui::Color32) {
    let ctx = painter.ctx();
    ctx.request_repaint();
    let time = ctx.input(|i| i.time);
    let radius = rect.width().min(rect.height()) / 2.0 - 1.0;
    let start = time * std::f64::consts::TAU;
    let end = start + 240f64.to_radians() * time.sin();
    let n = 20;
    let points: Vec<Pos2> = (0..n)
        .map(|i| {
            let angle = start + (end - start) * i as f64 / n as f64;
            let (sin, cos) = angle.sin_cos();
            rect.center() + radius * vec2(cos as f32, sin as f32)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(2.0, color)));
}

/// The painter a [`DockArea`](crate::DockArea) uses unless given another: every part painted
/// from the [`Style`], as before painters existed.
#[derive(Clone, Copy, Debug, Default)]
pub struct StylePainter;

impl DockPainter for StylePainter {}

const DASHED_LINE_ALPHAS: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];

#[inline(always)]
const fn lerp_vec(split: Split, alpha: f32) -> Vec2 {
    if split.is_top_bottom() {
        vec2(alpha, 0.5)
    } else {
        vec2(0.5, alpha)
    }
}

/// The state of a painted part, as flags: a tab uses all of them, a separator
/// [`HOVERED`](Self::HOVERED), [`PRESSED`](Self::PRESSED), [`FOCUSED`](Self::FOCUSED) and
/// [`DRAGGED`](Self::DRAGGED), a tab's close button [`HOVERED`](Self::HOVERED),
/// [`PRESSED`](Self::PRESSED) and [`FOCUSED`](Self::FOCUSED). A section's band and a boundary
/// between sections use the same four as a separator. A tool of a
/// [`ToolRow`](crate::toolbar::ToolRow) uses [`HOVERED`](Self::HOVERED),
/// [`PRESSED`](Self::PRESSED), [`FOCUSED`](Self::FOCUSED), [`SELECTED`](Self::SELECTED) (a toggle
/// on, a menu open) and [`DISABLED`](Self::DISABLED); its More button the first four. A tool of a
/// [`ToolRow`](crate::toolbar::ToolRow) uses [`HOVERED`](Self::HOVERED),
/// [`PRESSED`](Self::PRESSED), [`FOCUSED`](Self::FOCUSED), [`SELECTED`](Self::SELECTED) (a toggle
/// on, a menu open) and [`DISABLED`](Self::DISABLED); the More button the first four. A leaf's
/// body ([`TabBodyPaint`]) uses [`HOVERED`](Self::HOVERED), and [`FOCUSED`](Self::FOCUSED) with
/// [`LEAF_FOCUSED`](Self::LEAF_FOCUSED) when its leaf is the dock's focused one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartState(u8);

impl PartState {
    /// No flag.
    pub const NONE: Self = Self(0);
    /// The tab is its leaf's active one, whose body shows.
    pub const SELECTED: Self = Self(1);
    /// The pointer is over the part.
    pub const HOVERED: Self = Self(1 << 1);
    /// A pointer button is held down on the part.
    pub const PRESSED: Self = Self(1 << 2);
    /// The part has the keyboard focus.
    pub const FOCUSED: Self = Self(1 << 3);
    /// The part is being dragged.
    pub const DRAGGED: Self = Self(1 << 4);
    /// The tab's leaf is the dock's focused one, the last one interacted with
    /// ([`DockState::focused_leaf`](crate::DockState::focused_leaf)).
    pub const LEAF_FOCUSED: Self = Self(1 << 5);
    /// The part takes no input: a disabled tool of a [`ToolRow`](crate::toolbar::ToolRow).
    pub const DISABLED: Self = Self(1 << 6);

    /// Whether every flag of `other` is set.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// These flags with `flag` set or cleared.
    #[inline]
    #[must_use]
    pub const fn with(self, flag: Self, on: bool) -> Self {
        if on {
            Self(self.0 | flag.0)
        } else {
            Self(self.0 & !flag.0)
        }
    }
}

impl std::ops::BitOr for PartState {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for PartState {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A tab to measure, for [`DockPainter::tab_width`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TabMeasure<'a> {
    /// The tab's title, laid out.
    pub title: &'a Arc<Galley>,
    /// The flags known before the tab is laid out: [`PartState::SELECTED`],
    /// [`PartState::DRAGGED`] and [`PartState::LEAF_FOCUSED`].
    pub state: PartState,
    /// The width of the tab's close button, `0.0` without one.
    pub close_width: f32,
    /// The dock's style.
    pub style: &'a Style,
    /// The tab's style, after [`TabViewer::tab_style_override`](crate::TabViewer::tab_style_override).
    pub tab_style: &'a TabStyle,
}

/// A tab bar, for [`DockPainter::tab_bar`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TabBarPaint<'a> {
    /// The whole bar.
    pub rect: Rect,
    /// How far its tabs are scrolled to the left, in points: `0.0` when not scrolled.
    pub scrolled: f32,
    /// Whether its leaf is folded: only the bar shows, with no body below it.
    pub folded: bool,
    /// The dock's style.
    pub style: &'a Style,
}

/// The end of a tab bar, for [`DockPainter::tab_bar_tail`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TabBarTailPaint<'a> {
    /// From the right edge of the last tab to the right of the bar.
    pub rect: Rect,
    /// The bar, inside its margins.
    pub bar: Rect,
    /// Whether its leaf is folded: only the bar shows, with no body below it.
    pub folded: bool,
    /// The dock's style.
    pub style: &'a Style,
}

/// A tab, for [`DockPainter::tab`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TabPaint<'a> {
    /// The tab, as wide as [`DockPainter::tab_width`] measured (or wider in a filled bar).
    pub rect: Rect,
    /// Its bar, inside the bar's margins.
    pub bar: Rect,
    /// Its title, laid out.
    pub title: &'a Arc<Galley>,
    /// Its state.
    pub state: PartState,
    /// Its close button, if it shows one.
    pub close: Option<TabClose>,
    /// Whether its leaf is folded: only the tab bar shows, with no body below it.
    pub folded: bool,
    /// The dock's style.
    pub style: &'a Style,
    /// The tab's style, after [`TabViewer::tab_style_override`](crate::TabViewer::tab_style_override).
    pub tab_style: &'a TabStyle,
}

/// A tab's close button, in [`TabPaint`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TabClose {
    /// Where it is, at the tab's right end.
    pub rect: Rect,
    /// Its state.
    pub state: PartState,
}

/// Which way a separator runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeparatorAxis {
    /// A vertical separator, between a left and a right node; it moves sideways.
    Vertical,
    /// A horizontal separator, between a node above and one below; it moves up and down.
    Horizontal,
}

/// A separator, for [`DockPainter::separator`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SeparatorPaint<'a> {
    /// The separator, [`SeparatorStyle::width`](crate::SeparatorStyle::width) thick.
    pub rect: Rect,
    /// Which way it runs.
    pub axis: SeparatorAxis,
    /// Its state.
    pub state: PartState,
    /// The dock's style.
    pub style: &'a Style,
}

/// A leaf's body, for [`DockPainter::tab_body`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TabBodyPaint<'a> {
    /// The body, below its tab bar (or under the hidden bar's handle). The painter is clipped
    /// to it.
    pub rect: Rect,
    /// Its corners: [`TabBodyStyle::corner_radius`](crate::TabBodyStyle::corner_radius), with
    /// the top ones square when a tab bar is above it, to join it.
    pub corner_radius: CornerRadius,
    /// Its state: [`PartState::HOVERED`] while the pointer is over it, [`PartState::FOCUSED`]
    /// and [`PartState::LEAF_FOCUSED`] when its leaf is the dock's focused one
    /// ([`DockState::focused_leaf`](crate::DockState::focused_leaf)).
    pub state: PartState,
    /// The dock's style.
    pub style: &'a Style,
    /// The style of the leaf's active tab, after
    /// [`TabViewer::tab_style_override`](crate::TabViewer::tab_style_override): its
    /// [`tab_body`](TabStyle::tab_body) is the body's.
    pub tab_style: &'a TabStyle,
}

/// The node or surface under a dragged tab, for [`DockPainter::drop_target`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DropTargetPaint<'a> {
    /// The node or surface.
    pub rect: Rect,
    /// The dock's style.
    pub style: &'a Style,
}

/// A drop overlay button, for [`DockPainter::drop_button`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DropButtonPaint<'a> {
    /// The button.
    pub rect: Rect,
    /// The split it makes, `None` for the middle button, which adds the tab to the node.
    pub split: Option<Split>,
    /// Whether the pointer picks this button.
    pub hovered: bool,
    /// The dock's style.
    pub style: &'a Style,
}

/// Where a dragged tab would land, for [`DockPainter::drop_preview`].
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct DropPaint<'a> {
    /// Where the tab would go: the hovered tab or tab bar, the node, the half of it a split
    /// would give, or the new window.
    pub rect: Rect,
    /// The destination.
    pub destination: &'a TabDestination,
    /// Whether the destination was picked on the overlay's buttons, which then already show
    /// it: the default paints no fill but a new window's outline.
    pub from_buttons: bool,
    /// Why the tab may not go there, if refused.
    pub refusal: Option<&'a DropRefusal>,
    /// The dock's style.
    pub style: &'a Style,
    /// The visuals of the [`Ui`](egui::Ui) the dock is in.
    pub visuals: &'a Visuals,
}

/// A refused drop, in [`DropPaint`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DropRefusal {
    /// The reason [`TabViewer::drop_allowed`](crate::TabViewer::drop_allowed) gave, laid out.
    pub reason: Option<Arc<Galley>>,
    /// The pointer's position.
    pub pointer: Pos2,
}

/// A section's title band, for [`DockPainter::section_band`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SectionBandPaint<'a> {
    /// The whole band, as wide as the stack.
    pub rect: Rect,
    /// The band's widget id: what its response, its focus and its animations go by.
    pub id: Id,
    /// The title as shown, in capitals unless the stack keeps its case
    /// ([`Sections::capitals`](crate::sections::Sections::capitals)).
    pub title: &'a str,
    /// The count to show on the band, if any.
    pub count: Option<u64>,
    /// Whether the section is open.
    pub open: bool,
    /// Its state: [`PartState::HOVERED`], [`PartState::PRESSED`], [`PartState::FOCUSED`] and,
    /// for a title that drags ([`Sections::grab_titles`](crate::sections::Sections::grab_titles)),
    /// [`PartState::DRAGGED`].
    pub state: PartState,
    /// The style of the [`Ui`](egui::Ui) the stack is in.
    pub style: &'a egui::Style,
}

impl SectionBandPaint<'_> {
    /// The count as shown: `99+` past 99.
    pub fn count_text(&self) -> Option<String> {
        self.count.map(|count| {
            if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            }
        })
    }
}

/// A boundary between two sections, for [`DockPainter::section_boundary`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SectionBoundaryPaint<'a> {
    /// Where the boundary takes the pointer: a few points on either side of the line, or above
    /// it alone when the titles drag.
    pub rect: Rect,
    /// The line between the body above and the band below, as a `y`.
    pub line: f32,
    /// The boundary's widget id.
    pub id: Id,
    /// Its state: [`PartState::HOVERED`], [`PartState::PRESSED`], [`PartState::FOCUSED`] and
    /// [`PartState::DRAGGED`].
    pub state: PartState,
    /// The style of the [`Ui`](egui::Ui) the stack is in.
    pub style: &'a egui::Style,
}

/// A tool of a [`ToolRow`](crate::toolbar::ToolRow), or a row of its More menu, for
/// [`DockPainter::tool_item`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ToolItemPaint<'a> {
    /// The whole tool.
    pub rect: Rect,
    /// The tool's widget id: what its response, its focus and its animations go by.
    pub id: Id,
    /// What the tool is: a button, a toggle, a radio choice or a menu.
    pub kind: crate::toolbar::ToolKind,
    /// Its icon, if it shows one.
    pub icon: Option<&'a crate::toolbar::ToolIcon<'a>>,
    /// Where its icon goes, or its spinner while busy: a square, also given when no icon shows.
    pub icon_rect: Rect,
    /// Its label laid out, if it shows it, in [`Color32::PLACEHOLDER`](egui::Color32::PLACEHOLDER)
    /// so that the painter gives it its colour.
    pub label: Option<&'a Arc<Galley>>,
    /// Where the label's top left goes.
    pub label_pos: Pos2,
    /// Its key binding laid out, in [`Color32::PLACEHOLDER`](egui::Color32::PLACEHOLDER): given
    /// in the More menu only ([`Tool::shortcut`](crate::toolbar::Tool::shortcut)).
    pub shortcut: Option<&'a Arc<Galley>>,
    /// Where the key binding's top left goes.
    pub shortcut_pos: Pos2,
    /// Where its mark goes, if it has one: a menu's drop-down mark (or a submenu's arrow, in a
    /// menu), or, in a menu, a toggle's tick.
    pub mark: Option<Rect>,
    /// The tool is waiting for the result of what it started: the default paints a spinner in
    /// its icon's place.
    pub busy: bool,
    /// It is a row of the More menu, as wide as the menu, rather than a tool of the row.
    pub in_menu: bool,
    /// Its state: [`PartState::HOVERED`], [`PartState::PRESSED`], [`PartState::FOCUSED`],
    /// [`PartState::SELECTED`] (a toggle on, a menu open) and [`PartState::DISABLED`].
    pub state: PartState,
    /// The style of the [`Ui`](egui::Ui) the row is in.
    pub style: &'a egui::Style,
}

/// The More button of a [`ToolRow`](crate::toolbar::ToolRow), for [`DockPainter::tool_more`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ToolMorePaint<'a> {
    /// The button, at the row's end.
    pub rect: Rect,
    /// Its widget id.
    pub id: Id,
    /// Whether the row runs down, a column, rather than across.
    pub vertical: bool,
    /// Its state: [`PartState::HOVERED`], [`PartState::PRESSED`], [`PartState::FOCUSED`] and
    /// [`PartState::SELECTED`] while its menu is open.
    pub state: PartState,
    /// The style of the [`Ui`](egui::Ui) the row is in.
    pub style: &'a egui::Style,
}

/// A separator between two groups of a [`ToolRow`](crate::toolbar::ToolRow), for
/// [`DockPainter::tool_separator`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ToolSeparatorPaint<'a> {
    /// Its room in the row: as tall as the row in a row, as wide as the column in a column.
    pub rect: Rect,
    /// Whether the row runs down, a column, so the line runs across it.
    pub vertical: bool,
    /// The style of the [`Ui`](egui::Ui) the row is in.
    pub style: &'a egui::Style,
}

/// What a part of a leaf (its tab bar, a tab, a close button, its body) is attached to, which
/// says which of its corners are square: square only where something is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attached {
    /// The leaf's body is below it: its bottom corners are square, to join it.
    Below,
    /// A tab bar is above it, as above a leaf's body: its top corners are square, to join it.
    Above,
    /// Nothing is below it (a folded leaf, a dragged tab): its bottom corners are rounded like
    /// its top ones.
    Nothing,
}

impl Attached {
    /// The corners of a part styled with `radius` and attached so. A style gives the parts of
    /// a tab bar their top corners (their bottom ones are usually square): a part with nothing
    /// below it takes its top corners at the bottom too, unless its style rounds them more.
    pub(crate) fn corner_radius(self, radius: CornerRadius) -> CornerRadius {
        match self {
            Self::Below => CornerRadius {
                sw: 0,
                se: 0,
                ..radius
            },
            Self::Above => CornerRadius {
                nw: 0,
                ne: 0,
                ..radius
            },
            Self::Nothing => CornerRadius {
                sw: radius.sw.max(radius.nw),
                se: radius.se.max(radius.ne),
                ..radius
            },
        }
    }
}

/// The corners of `part`, a piece of `outer` painted over it (a tab's close button, a tab
/// bar's button) whose corners are `outer_radius`: a corner of `part` that lies on a corner
/// of `outer` is rounded like it, so that it does not stick out of it; the others are square.
pub(crate) fn corners_within(part: Rect, outer: Rect, outer_radius: CornerRadius) -> CornerRadius {
    let near = |a: f32, b: f32| (a - b).abs() < 0.5;
    let (top, bottom) = (
        near(part.top(), outer.top()),
        near(part.bottom(), outer.bottom()),
    );
    let (left, right) = (
        near(part.left(), outer.left()),
        near(part.right(), outer.right()),
    );
    let pick = |on: bool, radius: u8| if on { radius } else { 0 };
    CornerRadius {
        nw: pick(top && left, outer_radius.nw),
        ne: pick(top && right, outer_radius.ne),
        sw: pick(bottom && left, outer_radius.sw),
        se: pick(bottom && right, outer_radius.se),
    }
}
