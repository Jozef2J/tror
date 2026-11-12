use egui::{Context, Id, Pos2};

use super::{
    DraggedTab,
    drag_and_drop::{DragData, DragDropState, HoverData},
};
use crate::{Style, SurfaceIndex};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub drag_start: Option<Pos2>,
    pub last_hover_pos: Option<Pos2>,
    pub dnd: Option<DragDropState>,
    pub window_fade: Option<(f64, SurfaceIndex)>,
    /// The tab dragged during the last frame, if any.
    pub dragged: Option<DraggedTab>,
    /// The pass the drag was last resolved in: the docks of a space resolve it once a pass.
    pub pass: Option<u64>,
}

impl State {
    #[inline(always)]
    pub(super) fn load(ctx: &Context, id: Id) -> Self {
        ctx.data_mut(|d| d.get_temp(id)).unwrap_or(Self {
            drag_start: None,
            last_hover_pos: None,
            dnd: None,
            window_fade: None,
            dragged: None,
            pass: None,
        })
    }

    #[inline(always)]
    pub(super) fn store(self, ctx: &Context, id: Id) {
        ctx.data_mut(|d| d.insert_temp(id, self));
    }

    pub(super) fn reset_drag(&mut self) {
        self.dnd = None;
        self.window_fade = None;
        self.drag_start = None;
    }

    pub(super) fn set_drag_and_drop(
        &mut self,
        drag: DragData,
        drop: HoverData,
        ctx: &Context,
        style: &Style,
    ) {
        if !self.is_drag_drop_locked(ctx, style) {
            self.dnd = Some(DragDropState {
                hover: drop,
                drag,
                pointer: ctx.pointer_hover_pos().unwrap_or(Pos2::ZERO),
                locked: None,
                src_in_window: false,
                dst_in_window: false,
            })
        }
    }

    #[inline(always)]
    fn is_drag_drop_locked(&self, ctx: &Context, style: &Style) -> bool {
        self.dnd
            .as_ref()
            .is_some_and(|drag_drop_state| drag_drop_state.is_locked(style, ctx))
    }
}
