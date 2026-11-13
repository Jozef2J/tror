use egui::{
    Align, Color32, CornerRadius, CursorIcon, Frame, Id, LayerId, Layout, Order, Rect, Response,
    RichText, Sense, Shape, Stroke, Ui, UiBuilder, Vec2, WidgetText, vec2,
};

use crate::{
    DockArea, LinearDir, Node, NodeIndex, Style, SurfaceIndex, TabViewer,
    dock_area::{state::State, tab_removal::TabRemoval},
    utils::{fade_visuals, rect_set_size_centered},
    widgets::painter::{Attached, corners_within},
};

/// The layer of the floating window showing `surf_index`: an [`egui::Window`] is an area of
/// [`Order::Middle`].
pub(super) fn window_layer_id(surf_index: SurfaceIndex) -> LayerId {
    LayerId::new(Order::Middle, Id::unique(format!("window {surf_index:?}")))
}

impl<Tab> DockArea<'_, Tab> {
    pub(super) fn show_window_surface(
        &mut self,
        ui: &Ui,
        surf_index: SurfaceIndex,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        state: &mut State,
        fade_style: Option<(&Style, f32, SurfaceIndex)>,
    ) {
        // Construct egui window
        let id = window_layer_id(surf_index).id;
        let bounds = self.window_bounds.unwrap();
        let open = true;
        // egui sizes a window by its outer size: the frame's margins and stroke come on top of
        // what the content takes.
        let chrome = Frame::window(ui.style()).total_margin().sum();
        let (window, restored_height) = self
            .dock_state
            .get_window_state_mut(surf_index)
            .unwrap()
            .create_window(id, bounds, chrome.y);

        // Calculate fading of the window (if any)
        let (fade_factor, fade_style) = match fade_style {
            Some((style, factor, surface_index)) => {
                if surface_index == surf_index {
                    (1.0, None)
                } else {
                    (factor, Some((style, factor)))
                }
            }
            None => (1.0, None),
        };

        // Get galley of currently selected node as a window title
        let title = {
            let node_id = self.dock_state[surf_index]
                .focused_leaf()
                .unwrap_or_else(|| {
                    for node_index in self.dock_state[surf_index].breadth_first_index_iter() {
                        if self.dock_state[surf_index][node_index].is_leaf() {
                            return node_index;
                        }
                    }
                    unreachable!("a window surface should never be empty")
                });
            let leaf = self.dock_state[surf_index][node_id].get_leaf_mut().unwrap();
            tab_viewer
                .title(&mut leaf.tabs[leaf.active.0])
                .color(ui.visuals().widgets.noninteractive.fg_stroke.color)
        };

        // Iterate through every node in dock_state[surf_index], and sum up the number of tabs in them
        let mut tab_count = 0;
        for node_index in self.dock_state[surf_index].breadth_first_index_iter() {
            if self.dock_state[surf_index][node_index].is_leaf() {
                tab_count += self.dock_state[surf_index][node_index].tabs_count();
            }
        }

        // Fade window frame (if necessary)
        let mut frame = Frame::window(ui.style());
        if fade_factor != 1.0 {
            frame.fill = frame.fill.linear_multiply(fade_factor);
            frame.stroke.color = frame.stroke.color.linear_multiply(fade_factor);
            frame.shadow.color = frame.shadow.color.linear_multiply(fade_factor);
        }

        let tab_bar_height = self.style.as_ref().unwrap().tab_bar.height;
        let minimized = self
            .dock_state
            .get_window_state(surf_index)
            .unwrap()
            .is_minimized();
        // The least size the window's content is laid out in: egui's resize stops there, so the
        // frame, the tab bars and their buttons keep one width.
        let min_size = self.window_min_size(surf_index) + chrome;
        let window = if minimized {
            let height = tab_bar_height + chrome.y;
            window
                .resizable([true, false])
                .max_height(height)
                .min_height(height)
        } else if self.dock_state[surf_index].is_collapsed() {
            // Folded, the window is as tall as its folded bands, with the separators between
            // them: the least height of its content.
            let height = min_size.y;
            window
                .resizable([true, false])
                .max_height(height)
                .min_height(height)
        } else if let Some(height) = restored_height {
            // Unfolded or restored: the window gets back the height it had, set after its least
            // size so that does not override it; egui keeps it from the next frame on.
            let height = height.max(min_size.y);
            window
                .min_size(min_size)
                .min_height(height)
                .max_height(height)
        } else {
            window.min_size(min_size)
        }
        .min_width(min_size.x)
        .frame(frame);
        let shown = window.show(ui.ctx(), |ui| {
            // Fade inner ui (if necessary)
            if fade_factor != 1.0 {
                fade_visuals(ui.visuals_mut(), fade_factor);
            }
            if minimized {
                self.minimized_body(
                    ui,
                    surf_index,
                    fade_style.map(|(style, _)| style),
                    title,
                    tab_count,
                )
            } else {
                self.render_nodes(ui, tab_viewer, state, surf_index, fade_style);
            }
        });

        // Remember where the window is and whether it is dragged, for `WindowState::rect` and
        // `WindowState::dragged`.
        if let Some(shown) = shown
            && let Some(window_state) = self.dock_state.get_window_state_mut(surf_index)
        {
            let (pressed, origin) =
                ui.input(|i| (i.pointer.primary_down(), i.pointer.press_origin()));
            let shift = window_state.pin_far_edges(
                shown.response.rect,
                shown.response.dragged(),
                pressed,
                origin,
            );
            if shift.is_some() {
                ui.ctx().request_repaint();
            }
            window_state.set_shown(shown.response.rect, shown.response.dragged());
        }

        if !open {
            self.to_remove.push(TabRemoval::Window(surf_index));
        }
    }

    /// The least size of the content of the window `surface`: each leaf keeps its tab bar's
    /// buttons and some of its tabs, and a body below its bar unless it is folded; the
    /// separators and the dock's padding come on top.
    fn window_min_size(&self, surface: SurfaceIndex) -> Vec2 {
        let style = self.style.as_ref().unwrap();
        let padding = style
            .dock_area_padding
            .map_or(Vec2::ZERO, |margin| margin.sum());
        if self.dock_state[surface].is_empty() {
            return padding;
        }
        self.node_min_size(surface, NodeIndex::root()) + padding
    }

    pub(super) fn node_min_size(&self, surface: SurfaceIndex, node: NodeIndex) -> Vec2 {
        let style = self.style.as_ref().unwrap();
        let gap = style.separator.width;
        let along = |sizes: &mut dyn Iterator<Item = f32>| {
            let (sum, count) = sizes.fold((0.0, 0), |(sum, count), size| (sum + size, count + 1));
            sum + gap * (count as f32 - 1.0).max(0.0)
        };
        let tree = &self.dock_state[surface];
        match &tree[node] {
            Node::Empty => Vec2::ZERO,
            Node::Leaf(leaf) => {
                let buttons = [
                    (self.show_add_buttons, Style::TAB_ADD_BUTTON_SIZE),
                    (
                        self.show_leaf_close_all_buttons,
                        Style::TAB_CLOSE_ALL_BUTTON_SIZE,
                    ),
                    (
                        self.show_leaf_collapse_buttons,
                        Style::TAB_COLLAPSE_BUTTON_SIZE,
                    ),
                ]
                .iter()
                .filter(|(shown, _)| *shown)
                .map(|(_, size)| size)
                .sum::<f32>();
                // Room for a part of a tab, which scrolls when the bar is narrower.
                let width = buttons + 2.0 * Style::TAB_CLOSE_BUTTON_SIZE;
                let bar = if self.hidable_tab_bars && leaf.tab_bar_hidden {
                    0.0
                } else {
                    style.tab_bar.height
                };
                let (width, body) = if leaf.collapsed {
                    (width, 0.0)
                } else {
                    let least = self.leaf_min_size.map_or(Vec2::ZERO, |f| f(&leaf.tabs));
                    (width.max(least.x), style.tab_bar.height.max(least.y))
                };
                vec2(width, bar + body)
            }
            Node::Linear(linear) => {
                let sizes: Vec<Vec2> = linear
                    .children()
                    .iter()
                    .map(|&child| self.node_min_size(surface, child))
                    .collect();
                let widest = sizes.iter().fold(Vec2::ZERO, |max, size| max.max(*size));
                match linear.dir {
                    LinearDir::Horizontal => {
                        vec2(along(&mut sizes.iter().map(|size| size.x)), widest.y)
                    }
                    LinearDir::Vertical => {
                        vec2(widest.x, along(&mut sizes.iter().map(|size| size.y)))
                    }
                }
            }
            Node::Grid(grid) => {
                let widest = grid
                    .children()
                    .iter()
                    .map(|&child| self.node_min_size(surface, child))
                    .fold(Vec2::ZERO, Vec2::max);
                vec2(
                    along(&mut std::iter::repeat_n(widest.x, grid.columns())),
                    along(&mut std::iter::repeat_n(widest.y, grid.rows())),
                )
            }
        }
    }

    fn minimized_body(
        &mut self,
        ui: &mut Ui,
        surface_index: SurfaceIndex,
        fade_style: Option<&Style>,
        title: WidgetText,
        tab_count: usize,
    ) {
        ui.horizontal(|ui| {
            let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
            let (tabbar_outer_rect, _) = ui.allocate_exact_size(
                vec2(Style::TAB_EXPAND_BUTTON_SIZE, style.tab_bar.height),
                Sense::hover(),
            );
            // A minimized window shows only this bar: nothing is below it.
            let bar_radius = Attached::Nothing.corner_radius(style.tab_bar.corner_radius);
            ui.painter()
                .rect_filled(tabbar_outer_rect, bar_radius, style.tab_bar.bg_fill);
            self.window_expand(ui, surface_index, tabbar_outer_rect, fade_style, bar_radius);
            ui.label(title);
            if tab_count > 1 {
                ui.label(
                    RichText::new(format!("+{}", tab_count - 1))
                        .color(ui.visuals().weak_text_color()),
                );
            }
            ui.allocate_space(ui.available_size());
        });
    }

    /// Draws the expand window button.
    fn window_expand(
        &mut self,
        ui: &mut Ui,
        surface_index: SurfaceIndex,
        tabbar_outer_rect: Rect,
        fade_style: Option<&Style>,
        bar_radius: CornerRadius,
    ) {
        let rect = tabbar_outer_rect;

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((surface_index, "window_expand")),
        );

        let (rect, mut response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                enabled,
                &self.labels.restore_window,
            )
        });

        response = response.on_hover_cursor(CursorIcon::PointingHand);

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let color = if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(
                rect,
                corners_within(rect, tabbar_outer_rect, bar_radius),
                style.buttons.minimize_window_bg_fill,
            );
            style.buttons.minimize_window_active_color
        } else {
            style.buttons.minimize_window_color
        };

        let mut arrow_rect = rect;

        rect_set_size_centered(&mut arrow_rect, Vec2::splat(Style::TAB_EXPAND_ARROW_SIZE));

        Self::draw_chevron_right(ui, &mut response, style, color, arrow_rect);

        // Draw button right border.
        ui.painter().vline(
            rect.right(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.minimize_window_border_color,
            ),
        );

        if response.clicked() {
            self.window_toggle_minimized(surface_index);
        }
    }

    fn draw_chevron_right(
        ui: &mut Ui,
        response: &mut Response,
        style: &Style,
        color: Color32,
        arrow_rect: Rect,
    ) {
        ui.painter().add(Shape::convex_polygon(
            // Arrow pointing rightwards.
            vec![
                arrow_rect.left_top(),
                arrow_rect.center(),
                arrow_rect.left_bottom(),
            ],
            color,
            Stroke::NONE,
        ));

        // Chevron pointing rightwards.
        ui.painter().add(Shape::convex_polygon(
            vec![
                arrow_rect.center_top(),
                arrow_rect.right_center(),
                arrow_rect.center_bottom(),
            ],
            color,
            Stroke::NONE,
        ));
        let color = if response.hovered() || response.has_focus() {
            style.buttons.minimize_window_bg_fill
        } else {
            style.tab_bar.bg_fill
        };
        ui.painter().add(Shape::convex_polygon(
            vec![
                arrow_rect
                    .center_top()
                    .lerp(arrow_rect.center_bottom(), 0.25),
                arrow_rect.center().lerp(arrow_rect.right_center(), 0.5),
                arrow_rect
                    .center_top()
                    .lerp(arrow_rect.center_bottom(), 0.75),
            ],
            color,
            Stroke::NONE,
        ));
    }

    pub(super) fn window_toggle_minimized(&mut self, surf_index: SurfaceIndex) {
        let minimized = self
            .dock_state
            .get_window_state(surf_index)
            .unwrap()
            .is_minimized();
        let surface = &mut self.dock_state[surf_index];

        if surface.root_node().is_some_and(|node| node.is_collapsed()) {
            // The window is already fully collapsed,
            // so `expanded_height` has already been set.
            // We don't need to set `new` either.
            if let Some(window_state) = self.dock_state.get_window_state_mut(surf_index) {
                window_state.toggle_minimized();
            }
        } else if minimized {
            if let Some(window_state) = self.dock_state.get_window_state_mut(surf_index) {
                window_state.set_new(true);
                window_state.toggle_minimized();
            }
        } else {
            let root_index = NodeIndex::root();
            let surface_height = if surface.root_node().is_some() {
                surface[root_index].rect().unwrap().height()
            } else {
                0.0
            };
            if let Some(window_state) = self.dock_state.get_window_state_mut(surf_index) {
                window_state.set_expanded_height(surface_height);
                window_state.toggle_minimized();
            }
        }
    }
}
