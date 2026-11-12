use egui::{Sense, Ui};

use crate::{
    DockArea, SurfaceIndex, TabViewer,
    dock_area::{
        drag_and_drop::{HoverData, TreeComponent},
        state::State,
    },
};

impl<Tab> DockArea<'_, Tab> {
    /// Shows a docked surface: the main surface, or a dock of a space.
    pub(super) fn show_root_surface_inside(
        &mut self,
        ui: &mut Ui,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        state: &mut State,
        surf_index: SurfaceIndex,
    ) {
        if self.dock_state[surf_index].is_empty() {
            let rect = ui.available_rect_before_wrap();
            let response = ui.allocate_rect(rect, Sense::hover());
            // The dragged tab, on a layer of its own, may cover the surface: test the pointer.
            let hovered_by_drag = state.drag_start.is_some()
                && state.last_hover_pos.is_some_and(|pos| rect.contains(pos));
            // A floating window over the surface takes the hover.
            let covered = state
                .last_hover_pos
                .is_some_and(|pos| self.covered_by_window(ui, pos));
            if (response.contains_pointer() || hovered_by_drag) && !covered {
                ui.memory_mut(|mem| {
                    mem.data.insert_temp(
                        self.id.with("hover_data"),
                        Some(HoverData {
                            rect,
                            dst: TreeComponent::Surface(surf_index),
                            tab: None,
                            outside: false,
                        }),
                    );
                });
            }
            return;
        }

        self.render_nodes(ui, tab_viewer, state, surf_index, None);
    }
}
