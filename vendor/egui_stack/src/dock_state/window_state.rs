use egui::{Id, Pos2, Rect, Vec2};

/// The state of a [`Surface::Window`](crate::Surface::Window).
///
/// Doubles as a handle for the surface, allowing the user to set its size and position.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(default)
)]
pub struct WindowState {
    /// The [`Rect`] that this window was last taking up.
    screen_rect: Option<Rect>,

    /// Was this window dragged in the last frame?
    dragged: bool,

    /// The next position this window should be set to next frame.
    next_position: Option<Pos2>,

    /// The next size this window should be set to next frame.
    next_size: Option<Vec2>,

    /// The height of the window before it was fully collapsed
    expanded_height: Option<f32>,

    /// True the first frame this window is drawn.
    /// handles expanding after being fully collapsed, etc.
    new: bool,

    /// True if the window is minimized
    minimized: bool,

    /// The window's rect when the pointer was pressed on its left or top edge, and which of
    /// the two edges: a resize from there keeps the opposite edge where it was.
    #[cfg_attr(feature = "serde", serde(skip))]
    edge_drag: Option<EdgeDrag>,
}

#[derive(Clone, Copy, Debug)]
struct EdgeDrag {
    start: Rect,
    left: bool,
    top: bool,
    least_width: Option<f32>,
    least_height: Option<f32>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            screen_rect: None,
            dragged: false,
            next_position: None,
            next_size: None,
            expanded_height: None,
            new: true,
            minimized: false,
            edge_drag: None,
        }
    }
}

impl WindowState {
    /// Create a default window state.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Set the position for this window in screen coordinates.
    pub fn set_position(&mut self, position: Pos2) -> &mut Self {
        self.next_position = Some(position);
        self
    }

    /// Set the size of this window in egui points.
    pub fn set_size(&mut self, size: Vec2) -> &mut Self {
        self.next_size = Some(size);
        self
    }

    /// Get the [`Rect`] which this window occupied when last shown, in screen coordinates.
    /// If this window hasn't been shown before, this will be [`Rect::NOTHING`].
    pub fn rect(&self) -> Rect {
        // The reason why we're unwrapping an Option with a default value instead of
        // just storing Rect::NOTHING for the None variant is that deserializing Rect::NOTHING
        // with serde_json causes a panic, because f32::INFINITY serializes into null in JSON.
        self.screen_rect.unwrap_or(Rect::NOTHING)
    }

    /// Returns whether this window was being dragged when last shown.
    pub fn dragged(&self) -> bool {
        self.dragged
    }

    /// Records where the window was shown this frame and whether it is dragged.
    #[inline(always)]
    pub(crate) fn set_shown(&mut self, rect: Rect, dragged: bool) {
        self.screen_rect = Some(rect);
        self.dragged = dragged;
    }

    /// Keeps the right or bottom edge in place while the window is resized by its left or
    /// top edge: egui moves the window on with the pointer once it is at its least size.
    pub(crate) fn pin_far_edges(
        &mut self,
        rect: Rect,
        moved: bool,
        pressed: bool,
        press_origin: Option<Pos2>,
    ) {
        if !pressed || moved {
            self.edge_drag = None;
            return;
        }
        if self.edge_drag.is_none() {
            let grab = 8.0;
            let Some(origin) = press_origin else {
                return;
            };
            if !rect.expand(grab).contains(origin) {
                return;
            }
            self.edge_drag = Some(EdgeDrag {
                start: rect,
                left: (origin.x - rect.left()).abs() <= grab,
                top: (origin.y - rect.top()).abs() <= grab,
                least_width: None,
                least_height: None,
            });
        }
        let Some(drag) = &mut self.edge_drag else {
            return;
        };
        let pin = |active: bool, least: &mut Option<f32>, size: f32, far: f32, start_far: f32| {
            if !active {
                return None;
            }
            match *least {
                Some(limit) if size > limit + 0.5 => *least = None,
                Some(_) => {}
                None if far > start_far + 0.5 => *least = Some(size),
                None => {}
            }
            least.map(|_| start_far - size)
        };
        let x = pin(
            drag.left,
            &mut drag.least_width,
            rect.width(),
            rect.right(),
            drag.start.right(),
        );
        let y = pin(
            drag.top,
            &mut drag.least_height,
            rect.height(),
            rect.bottom(),
            drag.start.bottom(),
        );
        if x.is_some() || y.is_some() {
            self.next_position = Some(Pos2::new(
                x.unwrap_or(rect.left()),
                y.unwrap_or(rect.top()),
            ));
        }
    }

    /// Set the height of this window when it is expanded.
    #[inline(always)]
    pub(crate) fn set_expanded_height(&mut self, height: f32) -> &mut Self {
        self.expanded_height = Some(height);
        self
    }

    #[inline(always)]
    pub(crate) fn set_new(&mut self, new: bool) -> &mut Self {
        self.new = new;
        self
    }

    #[inline(always)]
    pub(crate) fn next_position(&mut self) -> Option<Pos2> {
        self.next_position.take()
    }

    #[inline(always)]
    pub(crate) fn next_size(&mut self) -> Option<Vec2> {
        self.next_size.take()
    }

    #[inline(always)]
    pub(crate) fn expanded_height(&mut self) -> Option<f32> {
        self.expanded_height.take()
    }

    #[inline(always)]
    pub(crate) fn toggle_minimized(&mut self) {
        self.minimized = !self.minimized;
    }

    #[inline(always)]
    pub(crate) fn is_minimized(&self) -> bool {
        self.minimized
    }

    /// Builds the window, and returns with it the outer height it gets back this frame, if it
    /// is unfolded or restored: the caller sets it as both its least and largest height after
    /// its own size constraints, which would otherwise override it.
    ///
    /// The `'static` means the `open` field is always `None`. `chrome` is the height the
    /// window's frame adds to its content: egui sizes a window by its outer size, and the
    /// expanded height kept is the content's.
    pub(crate) fn create_window(
        &mut self,
        id: Id,
        bounds: Rect,
        chrome: f32,
    ) -> (egui::Window<'static>, Option<f32>) {
        let new = self.new;
        let mut window_constructor = egui::Window::new("")
            .id(id)
            .constrain_to(bounds)
            .title_bar(false);

        if let Some(position) = self.next_position() {
            window_constructor = window_constructor.current_pos(position);
        }
        if let Some(size) = self.next_size() {
            window_constructor = window_constructor.fixed_size(size);
        }
        // The height the window had before it was folded or minimized, now it is open again.
        let restored_height = if new {
            self.expanded_height().map(|height| height + chrome)
        } else {
            None
        };
        self.new = false;
        (window_constructor, restored_height)
    }
}
