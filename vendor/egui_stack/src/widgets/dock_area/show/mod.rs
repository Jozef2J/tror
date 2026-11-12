use egui::{
    Context, CursorIcon, EventFilter, Key, Pos2, Rect, Sense, StrokeKind, Ui, Vec2, WidgetInfo,
    accesskit::{Action, ActionData, Orientation, Role},
    pos2,
};

use super::{
    drag_and_drop::{DragData, HoverData, TreeComponent},
    state::State,
    tab_removal::TabRemoval,
};
use crate::NodePath;
use crate::dock_area::tab_removal::ForcedRemoval;
use crate::dock_space::{NoDrop, dock_name, remove_empty_docks};
use crate::dock_state::tree::node::shares;
use crate::tab_viewer::{DockDrop, DropVerdict, OnCloseResponse};
use crate::{
    AllowedSplits, DockArea, DockPainter, DockRules, LinearDir, Node, NodeIndex, OverlayType,
    PartState, SeparatorAxis, SeparatorPaint, Style, SurfaceIndex, TabDestination, TabPath,
    TabViewer,
    utils::{expand_to_pixel, fade_dock_style, map_to_pixel},
};

mod leaf;
mod main_surface;
mod window_surface;

/// How far, in points, the arrow keys and assistive technology move a separator per step.
const SEPARATOR_KEY_STEP: f32 = 16.0;

/// How far, in points, the arrow keys move a separator per step with Shift held.
const SEPARATOR_LARGE_KEY_STEP: f32 = 64.0;

impl<Tab> DockArea<'_, Tab> {
    /// Shows the docking hierarchy inside a [`Ui`].
    pub fn show_inside(mut self, ui: &mut Ui, tab_viewer: &mut impl TabViewer<Tab = Tab>) {
        self.style
            .get_or_insert(Style::from_egui(ui.style().as_ref()));
        self.window_bounds.get_or_insert(ui.ctx().content_rect());

        let mut state = State::load(ui.ctx(), self.id);
        // The docks of a space share one state: the first of them shown in a pass resolves the
        // drag for all of them, the others only draw.
        let pass = ui.ctx().cumulative_pass_nr();
        let first_in_pass = state.pass != Some(pass);
        state.pass = Some(pass);

        let dock_rect = ui.available_rect_before_wrap();
        if let Some(space) = &mut self.space
            && let Some(dock) = space.dock
        {
            space.docks[dock].rect = Some(dock_rect);
        }

        if first_in_pass {
            self.resolve_drag(ui, &mut state, tab_viewer, dock_rect);
        }

        let style = self.style.as_ref().unwrap();
        let fade_surface =
            self.hovered_window_surface(&mut state, style.overlay.feel.fade_hold_time, ui.ctx());
        let fade_style = {
            fade_surface.is_some().then(|| {
                let mut fade_style = style.clone();
                fade_dock_style(&mut fade_style, style.overlay.surface_fade_opacity);
                (fade_style, style.overlay.surface_fade_opacity)
            })
        };

        for surface_index in self.surfaces_to_show() {
            self.show_surface_inside(
                surface_index,
                ui,
                tab_viewer,
                &mut state,
                fade_style.as_ref().map(|(style, factor)| {
                    (style, *factor, fade_surface.unwrap_or(SurfaceIndex::main()))
                }),
            );
        }

        for removal in self.to_remove.drain(..).rev() {
            match removal {
                TabRemoval::Tab(path, ForcedRemoval(is_forced)) => {
                    if is_forced {
                        self.dock_state.remove_tab(path);
                    } else {
                        let leaf = &mut self.dock_state.leaf_mut(path.node_path()).unwrap();
                        match tab_viewer.on_close(&mut leaf.tabs[path.tab.0]) {
                            OnCloseResponse::Close => {
                                self.dock_state.remove_tab(path);
                            }
                            OnCloseResponse::Focus => {
                                leaf.active = path.tab;
                                self.new_focused = Some(path.node_path());
                            }
                            OnCloseResponse::Ignore => {
                                // no-op
                            }
                        }
                    }
                }
                TabRemoval::Node(path) => {
                    let mut all_tabs_are_closable = true;
                    for tab in self.dock_state[path].iter_tabs_mut() {
                        if !(tab_viewer.is_closeable(tab)
                            && matches!(tab_viewer.on_close(tab), OnCloseResponse::Close))
                        {
                            all_tabs_are_closable = false;
                        }
                    }
                    if all_tabs_are_closable {
                        self.dock_state.remove_leaf(path);
                    }
                }
                TabRemoval::Window(surface) => {
                    let mut all_tabs_are_closable = true;
                    for node in self.dock_state[surface].iter_mut() {
                        for tab in node.iter_tabs_mut() {
                            if !(tab_viewer.is_closeable(tab)
                                && matches!(tab_viewer.on_close(tab), OnCloseResponse::Close))
                            {
                                all_tabs_are_closable = false;
                            }
                        }
                    }
                    if all_tabs_are_closable {
                        self.dock_state.remove_surface(surface);
                    }
                }
            }
        }

        for path in self.to_detach.drain(..).rev() {
            let mouse_pos = state.last_hover_pos;
            self.dock_state
                .eject_tab(path, mouse_pos.unwrap_or(Pos2::ZERO));
        }

        if let Some(focused) = self.new_focused {
            self.dock_state.set_focused_node_and_surface(focused);
        }

        if let Some(space) = &mut self.space {
            remove_empty_docks(self.dock_state, space.docks);
        }

        state.store(ui.ctx(), self.id);
    }

    /// Reads what the leaves recorded during the last pass: the dragged tab and what it hovers.
    /// Paints the drop preview and, when the tab is released, moves it.
    fn resolve_drag(
        &mut self,
        ui: &Ui,
        state: &mut State,
        tab_viewer: &impl TabViewer<Tab = Tab>,
        dock_rect: Rect,
    ) {
        state.dragged = None;

        // Delay hover position one frame. On touch screens hover_pos() is None when any_released()
        if !ui.input(|i| i.pointer.any_released()) {
            state.last_hover_pos = ui.input(|i| i.pointer.hover_pos());
        }

        let (drag_data, hover_data) = ui.memory_mut(|mem| {
            (
                mem.data.remove_temp(self.id.with("drag_data")).flatten(),
                mem.data.remove_temp(self.id.with("hover_data")).flatten(),
            )
        });

        // A folded zone's strip taking drops wins over whatever lies under it.
        let hover_data = drag_data
            .as_ref()
            .and_then(|_| self.hover_strip(state))
            .or(hover_data)
            .or_else(|| self.hover_outside(dock_rect, state, &drag_data));
        // Where no tab may land, the drag hovers nothing: released, the tab stays where it was.
        let refused = hover_data
            .as_ref()
            .is_some_and(|hover| self.refuses_drop(ui.ctx(), state, hover));
        let hover_data = if refused {
            state.dnd = None;
            None
        } else {
            hover_data
        };
        if let Some(space) = &mut self.space {
            *space.no_drop = NoDrop::default();
        }

        if let (Some(source), Some(hover)) = (drag_data, hover_data) {
            let style = self.style.as_ref().unwrap();
            state.set_drag_and_drop(source, hover, ui.ctx(), style);
            let tab_dst = self.show_drag_drop_overlay(ui, state, tab_viewer);
            if ui.input(|i| i.pointer.primary_released())
                && let Some(destination) = tab_dst
            {
                let source = {
                    match state.dnd.as_ref().unwrap().drag.src {
                        TreeComponent::Tab(src) => src,
                        _ => todo!(
                            "collections of tabs, like nodes and surfaces can't be docked (yet)"
                        ),
                    }
                };
                let src_tab_count = self.dock_state[source.node_path()].tabs_count();
                self.dock_state.move_tab(source, destination);
                // unhide tab bar on the source leaf when a tab is dragged out
                if self.hidable_tab_bars
                    && let Ok(leaf) = self.dock_state.leaf_mut(source.node_path())
                    && leaf.tabs.len() < src_tab_count
                    && leaf.tab_bar_hidden
                {
                    leaf.tab_bar_hidden = false;
                }
            }
        }

        if ui.input(|i| i.pointer.primary_released()) {
            state.reset_drag();
        }
    }

    /// The surfaces this area draws: every surface of its [`DockState`], or its dock of a space,
    /// with the floating windows if it is the space's windows dock.
    fn surfaces_to_show(&self) -> Vec<SurfaceIndex> {
        match &self.space {
            None => self.dock_state.valid_surface_indices().to_vec(),
            Some(space) => {
                let dock = space.dock.map(|dock| space.docks[dock].surface);
                let windows = self
                    .dock_state
                    .valid_surface_indices()
                    .into_iter()
                    .filter(|&surface| space.shows_windows && self.dock_state.is_window(surface));
                dock.into_iter().chain(windows).collect()
            }
        }
    }

    /// The rules of the space's dock at `surface`, if it is one.
    fn dock_rules(&self, surface: SurfaceIndex) -> Option<DockRules> {
        let space = self.space.as_ref()?;
        space
            .docks
            .iter()
            .find(|dock| dock.surface == surface)
            .map(|dock| dock.rules)
    }

    /// Whether a drag hovering `hover` may not land there: the pointer is in one of the space's
    /// no-drop zones, unless over a floating window, or the dock under it refuses drops.
    fn refuses_drop(&self, ctx: &Context, state: &State, hover: &HoverData) -> bool {
        let Some(space) = &self.space else {
            return false;
        };
        let surface = hover.dst.surface_address();
        if !hover.outside && self.dock_state.is_window(surface) {
            return false;
        }
        if !hover.outside && self.dock_rules(surface).is_some_and(|r| !r.accept_drops) {
            return true;
        }
        let Some(pointer) = state.last_hover_pos else {
            return false;
        };
        let widgets = space
            .no_drop
            .ids
            .iter()
            .filter_map(|&id| ctx.read_response(id).map(|r| r.rect));
        space
            .no_drop
            .rects
            .iter()
            .copied()
            .chain(widgets)
            .any(|rect| rect.contains(pointer))
    }

    /// The name of the space's dock at `surface`, if it is one.
    fn dock_name(&self, surface: SurfaceIndex) -> Option<&str> {
        dock_name(self.space.as_ref()?.docks, surface)
    }

    /// Whether the tab at `path` may become a floating window: the viewer allows it, and the
    /// dock it is in, if any, too.
    pub(super) fn may_float(
        &mut self,
        path: TabPath,
        tab_viewer: &impl TabViewer<Tab = Tab>,
    ) -> bool {
        let dock_allows = self
            .dock_rules(path.surface)
            .is_none_or(|rules| rules.windows);
        let Node::Leaf(leaf) = &mut self.dock_state[path.node_path()] else {
            unreachable!("tab drags can only come from leaf nodes")
        };
        dock_allows && tab_viewer.allowed_in_windows(&mut leaf.tabs[path.tab.0])
    }

    /// Whether one of this dock's floating windows, not the one `ui` draws, is the topmost
    /// layer under `pointer`. What lies under it, a docked leaf or a window further down, is
    /// not what a dragged tab hovers, whatever the order the surfaces are drawn in.
    pub(super) fn covered_by_window(&self, ui: &Ui, pointer: Pos2) -> bool {
        let Some(top) = ui.ctx().layer_id_at(pointer) else {
            return false;
        };
        top != ui.layer_id()
            && self
                .dock_state
                .valid_surface_indices()
                .iter()
                .any(|&surface| {
                    self.dock_state.is_window(surface)
                        && window_surface::window_layer_id(surface) == top
                })
    }

    /// A strip of a folded zone that takes drops for one of the space's docks
    /// ([`DockSpace::bind`](crate::DockSpace::bind)), under the pointer and under no floating
    /// window: hovered as that dock's tab bar, so a tab released there joins the dock's focused
    /// leaf, else its first, or the dock itself when empty.
    fn hover_strip(&self, state: &State) -> Option<HoverData> {
        let space = self.space.as_ref()?;
        let pointer = state.last_hover_pos?;
        let (strip, name) = space
            .no_drop
            .strips
            .iter()
            .find(|(strip, _)| strip.contains(pointer))?;
        let over_a_window = self
            .dock_state
            .valid_surface_indices()
            .iter()
            .filter_map(|&surface| self.dock_state.get_window_state(surface))
            .any(|window| window.rect().contains(pointer));
        if over_a_window {
            return None;
        }
        let surface = space.docks.iter().find(|dock| &dock.name == name)?.surface;
        let tree = self.dock_state.get_surface(surface)?.node_tree()?;
        let leaf = tree
            .focused_leaf()
            .filter(|&node| tree.iter().nth(node.0).is_some_and(Node::is_leaf))
            .or_else(|| tree.iter().position(Node::is_leaf).map(NodeIndex));
        let dst = match leaf {
            Some(node) => TreeComponent::Node(NodePath::new(surface, node)),
            None => TreeComponent::Surface(surface),
        };
        Some(HoverData {
            rect: *strip,
            dst,
            // As a tab bar: the tab joins the leaf, with no split.
            tab: Some(*strip),
            outside: false,
        })
    }

    /// With [`OverlayType::Zones`], a tab dragged outside the dock, where none of its surfaces
    /// is under the pointer, hovers "outside": dropped there, it opens in a new window.
    fn hover_outside(
        &self,
        dock_rect: Rect,
        state: &State,
        drag_data: &Option<DragData>,
    ) -> Option<HoverData> {
        if drag_data.is_none()
            || self.style.as_ref().unwrap().overlay.overlay_type != OverlayType::Zones
        {
            return None;
        }
        let pointer = state.last_hover_pos?;
        let in_a_dock = match &self.space {
            None => dock_rect.contains(pointer),
            Some(space) => space
                .docks
                .iter()
                .filter_map(|dock| dock.rect)
                .any(|rect| rect.contains(pointer)),
        };
        let over_a_window = self
            .dock_state
            .valid_surface_indices()
            .iter()
            .filter_map(|&surface| self.dock_state.get_window_state(surface))
            .any(|window| window.rect().contains(pointer));
        (!in_a_dock && !over_a_window).then_some(HoverData {
            rect: Rect::NOTHING,
            dst: TreeComponent::Surface(SurfaceIndex::main()),
            tab: None,
            outside: true,
        })
    }

    /// Returns some when windows are fading, and what surface index is being hovered over
    #[inline(always)]
    fn hovered_window_surface(
        &self,
        state: &mut State,
        hold_time: f32,
        ctx: &Context,
    ) -> Option<SurfaceIndex> {
        if let Some(dnd_state) = &state.dnd
            && dnd_state.is_locked(self.style.as_ref().unwrap(), ctx)
        {
            state.window_fade =
                Some((ctx.input(|i| i.time), dnd_state.hover.dst.surface_address()));
        }

        state.window_fade.and_then(|(time, surface)| {
            ctx.request_repaint();
            (hold_time > (ctx.input(|i| i.time) - time) as f32).then_some(surface)
        })
    }

    /// Resolve where a dragged tab would land given it's dropped this frame, returns `None` when the resulting drop is an invalid move.
    fn show_drag_drop_overlay(
        &mut self,
        ui: &Ui,
        state: &mut State,
        tab_viewer: &impl TabViewer<Tab = Tab>,
    ) -> Option<TabDestination> {
        let drag_state = state.dnd.as_mut().unwrap();

        let deserted_node = {
            match (
                drag_state.drag.src.node_address(),
                drag_state.hover.dst.node_address(),
            ) {
                ((src_surf, Some(src_node)), (dst_surf, Some(dst_node))) => {
                    src_surf == dst_surf
                        && src_node == dst_node
                        && self.dock_state[src_surf][src_node].tabs_count() == 1
                }
                _ => false,
            }
        };

        // Not all scenarios can house all splits.
        let restricted_splits = if drag_state.hover.dst.is_surface() || deserted_node {
            AllowedSplits::None
        } else {
            AllowedSplits::All
        };
        let (src_surface, dst_surface) = (
            drag_state.drag.src.surface_address(),
            drag_state.hover.dst.surface_address(),
        );
        // The dock under the pointer may refuse splits.
        let dock_splits = if drag_state.hover.outside {
            AllowedSplits::All
        } else {
            self.dock_rules(dst_surface)
                .map_or(AllowedSplits::All, |rules| rules.splits)
        };
        let allowed_splits = self.allowed_splits & restricted_splits & dock_splits;

        let allowed_in_window = match drag_state.drag.src {
            TreeComponent::Tab(path) => self.may_float(path, tab_viewer),
            _ => todo!("collections of tabs, like nodes or surfaces, can't be dragged! (yet)"),
        };
        let drag_state = state.dnd.as_mut().unwrap();
        drag_state.src_in_window = self.dock_state.is_window(src_surface);
        drag_state.dst_in_window =
            !drag_state.hover.outside && self.dock_state.is_window(dst_surface);
        let style = self.style.as_ref().unwrap();

        if let Some(pointer) = state.last_hover_pos {
            drag_state.pointer = pointer;
        }

        let window_bounds = self.window_bounds.unwrap();
        let painter: &dyn DockPainter = self.painter.0;
        let (destination, from_buttons) =
            match (style.overlay.overlay_type, drag_state.is_on_title_bar()) {
                (OverlayType::HighlightedAreas, _) | (_, true) => (
                    drag_state.resolve_traditional(
                        ui,
                        style,
                        painter,
                        allowed_splits,
                        allowed_in_window,
                    ),
                    false,
                ),
                (OverlayType::Widgets, false) => (
                    drag_state.resolve_icon_based(
                        ui,
                        style,
                        painter,
                        allowed_splits,
                        allowed_in_window,
                    ),
                    true,
                ),
                (OverlayType::Zones, false) => (
                    drag_state.resolve_zones(ui, style, painter, allowed_splits, allowed_in_window),
                    false,
                ),
            };
        let destination = destination?;
        let from_buttons = from_buttons && !matches!(destination, TabDestination::Window(_));

        // Last, the viewer's veto for this tab at this destination.
        let TreeComponent::Tab(path) = drag_state.drag.src else {
            unreachable!("only tabs can be dragged")
        };
        let tab = &self.dock_state[path.node_path()]
            .tabs()
            .expect("tab drags can only come from leaf nodes")[path.tab.0];
        let to = match destination {
            TabDestination::Node(path, _) => self.dock_name(path.surface),
            TabDestination::EmptySurface(surface) => self.dock_name(surface),
            TabDestination::Window(_) => None,
        };
        let drop = DockDrop {
            destination: &destination,
            from: self.dock_name(path.surface),
            to,
        };
        let (allowed, refusal) = match tab_viewer.dock_drop_allowed(tab, &drop) {
            DropVerdict::Allow => (true, None),
            DropVerdict::Refuse(reason) => (false, Some(reason)),
        };
        drag_state.paint_preview(
            ui,
            style,
            painter,
            &destination,
            from_buttons,
            refusal,
            window_bounds,
        );
        allowed.then_some(destination)
    }

    /// Show a single surface of a [`DockState`].
    fn show_surface_inside(
        &mut self,
        surf_index: SurfaceIndex,
        ui: &mut Ui,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        state: &mut State,
        fade_style: Option<(&Style, f32, SurfaceIndex)>,
    ) {
        if !self.dock_state.is_window(surf_index) {
            self.show_root_surface_inside(ui, tab_viewer, state, surf_index);
        } else {
            self.show_window_surface(ui, surf_index, tab_viewer, state, fade_style);
        }
    }

    fn render_nodes(
        &mut self,
        ui: &mut Ui,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        state: &mut State,
        surf_index: SurfaceIndex,
        fade_style: Option<(&Style, f32)>,
    ) {
        // First compute all rect sizes in the node graph, parents before their children.
        let max_rect = self.allocate_area_for_root_node(ui, surf_index);
        for node_index in self.dock_state[surf_index].breadth_first_index_iter() {
            let path = NodePath {
                surface: surf_index,
                node: node_index,
            };
            if self.dock_state[path].is_parent() {
                self.compute_rect_sizes(ui, path, max_rect);
            }
        }

        // Then, draw the bodies of each leaves.
        for node_index in self.dock_state[surf_index].breadth_first_index_iter() {
            let path = NodePath {
                surface: surf_index,
                node: node_index,
            };
            if self.dock_state[path].is_leaf() {
                self.show_leaf(ui, state, path, tab_viewer, fade_style);
            }
        }

        // Finally, draw separators so that their "interaction zone" is above
        // bodies (see `SeparatorStyle::extra_interact_width`).
        let fade_style = fade_style.map(|(style, _)| style);
        for node_index in self.dock_state[surf_index].breadth_first_index_iter() {
            let path = NodePath {
                surface: surf_index,
                node: node_index,
            };
            for separator in separators(&self.dock_state[path]) {
                self.show_separator(ui, path, separator, fade_style);
            }
        }
    }

    fn allocate_area_for_root_node(&mut self, ui: &mut Ui, surface: SurfaceIndex) -> Rect {
        let style = self.style.as_ref().unwrap();
        let mut rect = ui.available_rect_before_wrap();

        if let Some(margin) = style.dock_area_padding {
            rect.min += margin.left_top();
            rect.max -= margin.right_bottom();
        }

        ui.painter().rect_stroke(
            rect,
            style.main_surface_border_rounding,
            style.main_surface_border_stroke,
            StrokeKind::Inside,
        );
        if !self.dock_state.is_window(surface) {
            rect = rect.expand(-style.main_surface_border_stroke.width / 2.0);
        }
        ui.allocate_rect(rect, Sense::hover());

        if self.dock_state[surface].is_empty() {
            return rect;
        }
        self.dock_state[surface][NodeIndex::root()].set_rect(rect);
        rect
    }

    fn compute_rect_sizes(&mut self, ui: &Ui, path: NodePath, max_rect: Rect) {
        let style = self.style.as_ref().unwrap();
        let pixels_per_point = ui.ctx().pixels_per_point();
        let tree = &self.dock_state[path.surface];
        let linear = match &tree[path.node] {
            Node::Linear(linear) => linear,
            Node::Grid(_) => return self.compute_grid_rect_sizes(ui, path, max_rect),
            _ => unreachable!("only containers have children"),
        };
        let rect = linear.rect;
        debug_assert!(!rect.any_nan() && rect.is_finite());
        let rect = expand_to_pixel(rect, pixels_per_point);

        // Collapsed children of a column keep the height of their tab bars.
        let collapsed: Vec<Option<i32>> = linear
            .children()
            .iter()
            .map(|&child| {
                (linear.dir == LinearDir::Vertical && tree[child].is_collapsed())
                    .then(|| tree[child].collapsed_leaf_count())
            })
            .collect();
        let axis = linear.dir;
        let (start, size) = match axis {
            LinearDir::Horizontal => (rect.min.x, rect.width()),
            LinearDir::Vertical => (rect.min.y, rect.height()),
        };
        let boundaries = boundaries(linear, &collapsed, size, style.tab_bar.height);
        let children = linear.children().to_vec();

        let half = style.separator.width * 0.5;
        let last = children.len() - 1;
        for (index, child) in children.into_iter().enumerate() {
            let mut child_rect = rect;
            let (min, max) = match axis {
                LinearDir::Horizontal => (&mut child_rect.min.x, &mut child_rect.max.x),
                LinearDir::Vertical => (&mut child_rect.min.y, &mut child_rect.max.y),
            };
            if index > 0 {
                *min = map_to_pixel(
                    start + boundaries[index - 1] + half,
                    pixels_per_point,
                    f32::round,
                );
            }
            if index < last {
                *max = map_to_pixel(
                    start + boundaries[index] - half,
                    pixels_per_point,
                    f32::round,
                );
            }
            let child_rect = rect.intersect(child_rect).intersect(max_rect);
            self.dock_state[path.surface][child].set_rect(child_rect);
        }
    }

    /// Lays out the children of the grid at `path` in its cells, row by row.
    fn compute_grid_rect_sizes(&mut self, ui: &Ui, path: NodePath, max_rect: Rect) {
        let style = self.style.as_ref().unwrap();
        let pixels_per_point = ui.ctx().pixels_per_point();
        let half = style.separator.width * 0.5;
        let Node::Grid(grid) = &mut self.dock_state[path.surface][path.node] else {
            unreachable!("a grid node")
        };
        grid.fit_shares();
        let grid = grid.clone();
        debug_assert!(!grid.rect.any_nan() && grid.rect.is_finite());
        let rect = expand_to_pixel(grid.rect, pixels_per_point);

        // The span of the cell `index` of `count` along one axis, between its lines.
        let span = |shares: &[f32], index: usize, count: usize, start: f32, size: f32| {
            let min = if index > 0 {
                map_to_pixel(
                    start + size * shares::boundary(shares, index - 1) + half,
                    pixels_per_point,
                    f32::round,
                )
            } else {
                start
            };
            let max = if index + 1 < count {
                map_to_pixel(
                    start + size * shares::boundary(shares, index) - half,
                    pixels_per_point,
                    f32::round,
                )
            } else {
                start + size
            };
            (min, max)
        };
        let (columns, rows) = (grid.columns(), grid.rows());
        for (index, &child) in grid.children().iter().enumerate() {
            let (column, row) = grid.cell(index);
            let (min_x, max_x) = span(grid.col_shares(), column, columns, rect.min.x, rect.width());
            let (min_y, max_y) = span(grid.row_shares(), row, rows, rect.min.y, rect.height());
            let child_rect = Rect::from_min_max(pos2(min_x, min_y), pos2(max_x, max_y));
            let child_rect = rect.intersect(child_rect).intersect(max_rect);
            self.dock_state[path.surface][child].set_rect(child_rect);
        }
    }

    /// Shows a separator of the container at `path`.
    fn show_separator(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        separator_of: SeparatorOf,
        fade_style: Option<&Style>,
    ) {
        let tree = &self.dock_state[path.surface];
        // If either of the children is collapsed, we don't want the user to interact with the separator
        if let (Node::Linear(linear), SeparatorOf::Linear(index)) = (&tree[path.node], separator_of)
        {
            let pair = [linear.children()[index], linear.children()[index + 1]];
            if linear.dir == LinearDir::Vertical && pair.iter().any(|&c| tree[c].is_collapsed()) {
                return;
            }
        }

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let pixels_per_point = ui.ctx().pixels_per_point();
        let painter = self.painter.0;
        let label = &self.labels.separator;

        // The separator stands after the share `index`, moving it and the next one.
        let (horizontal, rect, shares, index) =
            match (&mut self.dock_state[path.surface][path.node], separator_of) {
                (Node::Linear(linear), SeparatorOf::Linear(index)) => (
                    linear.dir == LinearDir::Horizontal,
                    linear.rect,
                    &mut linear.shares,
                    index,
                ),
                (Node::Grid(grid), SeparatorOf::Column(index)) => {
                    (true, grid.rect, &mut grid.col_shares, index)
                }
                (Node::Grid(grid), SeparatorOf::Row(index)) => {
                    (false, grid.rect, &mut grid.row_shares, index)
                }
                _ => unreachable!("a separator of this container"),
            };
        let dim = |v: Vec2| if horizontal { v.x } else { v.y };
        let along = |r: Rect| {
            if horizontal {
                (r.min.x, r.max.x)
            } else {
                (r.min.y, r.max.y)
            }
        };
        let set_along = |r: &mut Rect, min: f32, max: f32| {
            if horizontal {
                (r.min.x, r.max.x) = (min, max);
            } else {
                (r.min.y, r.max.y) = (min, max);
            }
        };

        let (start, end) = along(rect);
        let range = end - start;
        let mut separator = rect;
        let midpoint = start + range * shares::boundary(shares, index);
        set_along(
            &mut separator,
            midpoint - style.separator.width * 0.5,
            midpoint + style.separator.width * 0.5,
        );

        let mut expand = Vec2::ZERO;
        if horizontal {
            expand.x += style.separator.extra_interact_width / 2.0;
        } else {
            expand.y += style.separator.extra_interact_width / 2.0;
        }
        let interact_rect = separator.expand2(expand);

        let cursor = if horizontal {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        };
        let response = ui
            .allocate_rect(interact_rect, Sense::click_and_drag())
            .on_hover_and_drag_cursor(cursor);

        // A focused separator moves with the arrow keys along its axis, by a larger step
        // with Shift, and Enter resets it. The arrows across its axis still move the focus.
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    EventFilter {
                        horizontal_arrows: horizontal,
                        vertical_arrows: !horizontal,
                        tab: false,
                        escape: false,
                    },
                )
            });
        }

        let (back_key, forward_key) = if horizontal {
            (Key::ArrowLeft, Key::ArrowRight)
        } else {
            (Key::ArrowUp, Key::ArrowDown)
        };
        let (arrow_key_offset, reset) = if response.has_focus() {
            ui.input(|i| {
                let step = if i.modifiers.shift {
                    SEPARATOR_LARGE_KEY_STEP
                } else {
                    SEPARATOR_KEY_STEP
                };
                let offset = if i.key_pressed(back_key) {
                    Some(-step)
                } else if i.key_pressed(forward_key) {
                    Some(step)
                } else {
                    None
                };
                (offset, i.key_pressed(Key::Enter))
            })
        } else {
            (None, false)
        };

        set_along(
            &mut separator,
            map_to_pixel(
                midpoint - style.separator.width * 0.5,
                pixels_per_point,
                f32::round,
            ),
            map_to_pixel(
                midpoint + style.separator.width * 0.5,
                pixels_per_point,
                f32::round,
            ),
        );

        painter.separator(
            ui.painter(),
            &SeparatorPaint {
                rect: separator,
                axis: if horizontal {
                    SeparatorAxis::Vertical
                } else {
                    SeparatorAxis::Horizontal
                },
                state: PartState::NONE
                    .with(PartState::HOVERED, response.hovered())
                    .with(PartState::PRESSED, response.is_pointer_button_down_on())
                    .with(PartState::FOCUSED, response.has_focus())
                    .with(PartState::DRAGGED, response.dragged()),
                style,
            },
        );

        // Update the shares after drawing the separator, otherwise it may overlap on other
        // separators / bodies when shrunk fast.
        // Assistive technology moves the separator by the arrow keys' step.
        let accesskit_step = if range > 0.0 {
            (SEPARATOR_KEY_STEP / range).min(1.0)
        } else {
            0.0
        };
        let (accesskit_steps, accesskit_value) = ui.input(|i| {
            let steps = i.num_accesskit_action_requests(response.id, Action::Increment) as f32
                - i.num_accesskit_action_requests(response.id, Action::Decrement) as f32;
            let value = i
                .accesskit_action_requests(response.id, Action::SetValue)
                .filter_map(|request| match request.data {
                    Some(ActionData::NumericValue(value)) => Some(value as f32),
                    _ => None,
                })
                .last();
            (steps, value)
        });
        // Each child of the pair keeps at least `separator.extra` points.
        let min_part = if range > 0.0 {
            (style.separator.extra / range).min(1.0)
        } else {
            0.0
        };
        if range > 0.0 {
            let delta = arrow_key_offset.unwrap_or(dim(response.drag_delta()));
            let boundary = shares::boundary(shares, index);
            let target = accesskit_value.unwrap_or(boundary)
                + delta / range
                + accesskit_steps * accesskit_step;
            if target != boundary {
                shares::move_boundary(shares, index, target - boundary, min_part);
            }
        }

        if response.double_clicked() || reset {
            shares::even_pair(shares, index);
        }

        // Expose the separator as a splitter whose value is its position in the node, from 0
        // to 1: for two children, the split's fraction.
        let value = shares::boundary(shares, index);
        let pair_start = if index == 0 {
            0.0
        } else {
            shares::boundary(shares, index - 1)
        };
        let pair_end = shares::boundary(shares, index + 1);
        let (min, max) = if range > 0.0 {
            let min = pair_start + min_part;
            let max = pair_end - min_part;
            (min.min(max), max.max(min))
        } else {
            (value, value)
        };
        let enabled = ui.is_enabled();
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(Role::Splitter, enabled, label);
            info.value = Some(f64::from(value));
            info
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_orientation(if horizontal {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            });
            node.set_min_numeric_value(f64::from(min));
            node.set_max_numeric_value(f64::from(max));
            node.set_numeric_value_step(f64::from(accesskit_step));
            if range > 0.0 {
                node.add_action(Action::SetValue);
                if value < max {
                    node.add_action(Action::Increment);
                }
                if min < value {
                    node.add_action(Action::Decrement);
                }
            }
        });
    }
}

/// A separator of a container: the one after the child `index` of a linear node, or the line
/// after the column or the row `index` of a grid.
#[derive(Clone, Copy, Debug)]
enum SeparatorOf {
    Linear(usize),
    Column(usize),
    Row(usize),
}

/// The separators of `node`: one between each pair of neighbours of a linear node, one on each
/// line between two columns then between two rows of a grid.
fn separators<Tab>(node: &Node<Tab>) -> Vec<SeparatorOf> {
    match node {
        Node::Linear(linear) => (0..linear.len().saturating_sub(1))
            .map(SeparatorOf::Linear)
            .collect(),
        Node::Grid(grid) => (0..grid.columns() - 1)
            .map(SeparatorOf::Column)
            .chain((0..grid.rows() - 1).map(SeparatorOf::Row))
            .collect(),
        _ => Vec::new(),
    }
}

/// Where the separators of `linear` stand, in points from its start along its axis: one per
/// pair of neighbours. Collapsed children of a column (`Some(number of collapsed leaves)`)
/// keep the height of their tab bars; the others share the rest. If all are collapsed, the
/// last one takes the rest.
fn boundaries(
    linear: &crate::LinearNode,
    collapsed: &[Option<i32>],
    size: f32,
    tab_bar_height: f32,
) -> Vec<f32> {
    let n = linear.len();
    let sizes: Vec<f32> = if collapsed.iter().all(Option::is_none) {
        (0..n).map(|i| size * linear.fraction(i)).collect()
    } else {
        let fixed: f32 = collapsed
            .iter()
            .flatten()
            .map(|&count| count as f32 * tab_bar_height)
            .sum();
        let open_total: f32 = (0..n)
            .filter(|&i| collapsed[i].is_none())
            .map(|i| linear.shares()[i])
            .sum();
        let open_count = collapsed.iter().filter(|c| c.is_none()).count() as f32;
        let rest = (size - fixed).max(0.0);
        let mut sizes: Vec<f32> = (0..n)
            .map(|i| match collapsed[i] {
                Some(count) => count as f32 * tab_bar_height,
                None if open_total > 0.0 => rest * linear.shares()[i] / open_total,
                None => rest / open_count,
            })
            .collect();
        if open_count == 0.0 {
            let others: f32 = sizes[..n - 1].iter().sum();
            sizes[n - 1] = size - others;
        }
        sizes
    };
    sizes
        .iter()
        .take(n - 1)
        .scan(0.0, |acc, s| {
            *acc += s;
            Some(*acc)
        })
        .collect()
}
