/// Due to there being a lot of code to show a dock in a ui every complementing
/// method to ``show`` and ``show_inside`` is put in ``show_extra``.
/// Otherwise ``mod.rs`` would be humongous.
mod show;

// Various components of the `DockArea` which is used when rendering
mod allowed_splits;
mod drag_and_drop;
mod drop_zone;
mod labels;
mod state;
mod tab_removal;

pub use allowed_splits::AllowedSplits;
pub use drop_zone::DropZone;
use std::panic::AssertUnwindSafe;

use egui::{Context, Id, Modifiers, emath::*};
pub use labels::Labels;
use tab_removal::TabRemoval;

use crate::{
    DockPainter, DockSpace, NodePath, Style, StylePainter, TabIndex, TabPath,
    dock_space::{Dock, NoDrop},
    dock_state::DockState,
};

/// Displays a [`DockState`] in `egui`.
pub struct DockArea<'tree, Tab> {
    id: Id,
    dock_state: &'tree mut DockState<Tab>,
    style: Option<Style>,
    // Asserted unwind safe so that `DockArea` keeps its auto traits: the painter only paints.
    painter: AssertUnwindSafe<&'tree (dyn DockPainter + Sync)>,
    labels: Labels,
    show_add_popup: bool,
    show_add_buttons: bool,
    show_close_buttons: bool,
    tab_context_menus: bool,
    show_builtin_tab_menu: bool,
    draggable_tabs: bool,
    tab_focus: TabFocus,
    hidable_tab_bars: bool,
    show_tab_name_on_hover: bool,
    show_window_close_buttons: bool,
    show_window_collapse_buttons: bool,
    show_leaf_close_all_buttons: bool,
    show_leaf_collapse_buttons: bool,
    show_secondary_button_hint: bool,
    secondary_button_modifiers: Modifiers,
    secondary_button_on_modifier: bool,
    secondary_button_context_menu: bool,
    allowed_splits: AllowedSplits,
    window_bounds: Option<Rect>,
    leaf_min_size: Option<&'tree dyn Fn(&[Tab]) -> Vec2>,

    /// Set when the area shows one dock of a [`DockSpace`](crate::DockSpace).
    space: Option<SpaceDock<'tree>>,

    to_remove: Vec<TabRemoval>,
    to_detach: Vec<TabPath>,
    new_focused: Option<NodePath>,
    tab_hover_rect: Option<(Rect, TabIndex)>,
}

/// The dock of a [`DockSpace`](crate::DockSpace) a [`DockArea`] shows, and the space's docks.
struct SpaceDock<'a> {
    docks: &'a mut Vec<Dock>,
    /// Where no tab is dropped, this frame.
    no_drop: &'a mut NoDrop,
    /// The dock shown, as an index in `docks`; `None` if the space has no dock of that name.
    dock: Option<usize>,
    /// Whether this area also shows the floating windows.
    shows_windows: bool,
}

/// A tab being dragged in a [`DockArea`], from [`DockArea::dragged_tab`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DraggedTab {
    /// Where the dragged tab is in the [`DockState`].
    pub path: TabPath,
    /// The pointer's position.
    pub pointer: Pos2,
}

// Not generic over the tab type, so that `DockArea::dragged_tab(ctx, id)` needs no annotation.
impl DockArea<'_, ()> {
    /// The id of a [`DockArea`] whose [`id`](Self::id) was not set.
    pub fn default_id() -> Id {
        Id::unique("egui_stack::DockArea")
    }

    /// The tab being dragged in the [`DockArea`] with this `id`, as of its last
    /// [`show_inside`](DockArea::show_inside).
    ///
    /// A tab counts as dragged once it has moved far enough to leave its tab bar, until it is
    /// released. Use [`default_id`](Self::default_id) if the [`DockArea`]'s id was not set.
    pub fn dragged_tab(ctx: &Context, id: Id) -> Option<DraggedTab> {
        ctx.data(|d| d.get_temp::<state::State>(id))
            .and_then(|state| state.dragged)
    }
}

/// How the keyboard focus reaches the tabs of a tab bar, set with [`DockArea::tab_focus`], or
/// the tools of a [`ToolRow`](crate::toolbar::ToolRow), set with
/// [`ToolRow::tab_focus`](crate::toolbar::ToolRow::tab_focus) (where [`Roving`](Self::Roving)
/// is the default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabFocus {
    /// Every tab, and every tab's close button, is a Tab stop.
    #[default]
    Each,
    /// One Tab stop per tab bar, the active tab, as in WAI-ARIA's tab list: once it has the
    /// focus, Left and Right select and focus the previous and next tab (wrapping around), Home
    /// and End the first and last. Close buttons are not Tab stops; they stay clickable.
    Roving,
}

// Builder
impl<'tree, Tab> DockArea<'tree, Tab> {
    /// Creates a new [`DockArea`] from the provided [`DockState`].
    #[inline(always)]
    pub fn new(tree: &'tree mut DockState<Tab>) -> DockArea<'tree, Tab> {
        Self {
            id: DockArea::default_id(),
            dock_state: tree,
            style: None,
            painter: AssertUnwindSafe(&StylePainter),
            labels: Labels::ENGLISH,
            show_add_popup: false,
            show_add_buttons: false,
            show_close_buttons: true,
            tab_context_menus: true,
            show_builtin_tab_menu: true,
            draggable_tabs: true,
            tab_focus: TabFocus::Each,
            hidable_tab_bars: false,
            show_tab_name_on_hover: false,
            allowed_splits: AllowedSplits::default(),
            space: None,
            to_remove: Vec::new(),
            to_detach: Vec::new(),
            new_focused: None,
            tab_hover_rect: None,
            window_bounds: None,
            leaf_min_size: None,
            show_window_close_buttons: true,
            show_window_collapse_buttons: true,
            show_leaf_close_all_buttons: true,
            show_leaf_collapse_buttons: true,
            show_secondary_button_hint: true,
            secondary_button_modifiers: Modifiers::SHIFT,
            secondary_button_on_modifier: true,
            secondary_button_context_menu: true,
        }
    }

    /// Creates a [`DockArea`] showing the dock named `dock` of `space`, drawn in the [`Ui`] given
    /// to [`show_inside`](Self::show_inside); if the dock is the space's
    /// [windows dock](crate::DockSpace::windows_dock), the floating windows too.
    ///
    /// Each dock of a space is shown by its own call, in any order and anywhere in the frame;
    /// a tab is dragged from any of them to any other as inside one dock. The calls share the
    /// space's [`id`](crate::DockSpace::id): set it on the space, not with [`id`](Self::id). A
    /// dock the space does not have shows nothing.
    ///
    /// [`Ui`]: egui::Ui
    pub fn dock(space: &'tree mut DockSpace<Tab>, dock: &str) -> DockArea<'tree, Tab> {
        let (dock_state, docks, no_drop, dock, shows_windows, id) = space.split_for_area(dock);
        let mut area = Self::new(dock_state).id(id);
        area.space = Some(SpaceDock {
            docks,
            no_drop,
            dock,
            shows_windows,
        });
        area
    }

    /// Sets the [`DockArea`] ID. Useful if you have more than one [`DockArea`].
    #[inline(always)]
    pub fn id(mut self, id: Id) -> Self {
        self.id = id;
        self
    }

    /// Sets the look and feel of the [`DockArea`].
    #[inline(always)]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Sets what paints the tabs, tab bars, separators and drop overlay. By default it's
    /// [`StylePainter`], which paints them from the [`Style`].
    ///
    /// The painter is `Sync`, as the rest of a [`DockArea`] can be: one that records what it
    /// paints keeps it behind a `Mutex`.
    ///
    /// ```
    /// # use egui_stack::{DockArea, DockPainter, DockState, SeparatorPaint};
    /// /// Separators as thin lines, everything else as the style says.
    /// struct ThinSeparators;
    ///
    /// impl DockPainter for ThinSeparators {
    ///     fn separator(&self, painter: &egui::Painter, separator: &SeparatorPaint<'_>) {
    ///         let color = separator.style.separator.color_hovered;
    ///         painter.rect_filled(separator.rect.shrink(1.0), 0.0, color);
    ///     }
    /// }
    ///
    /// # egui::__run_test_ui(|ui| {
    /// # struct Viewer;
    /// # impl egui_stack::TabViewer for Viewer {
    /// #     type Tab = String;
    /// #     fn id(&mut self, tab: &mut String) -> egui::Id { egui::Id::new(tab.as_str()) }
    /// #     fn title(&mut self, tab: &mut String) -> egui::WidgetText { tab.as_str().into() }
    /// #     fn ui(&mut self, ui: &mut egui::Ui, tab: &mut String) { ui.label(tab.as_str()); }
    /// # }
    /// let mut dock_state = DockState::new(vec!["Tab".to_owned()]);
    /// DockArea::new(&mut dock_state)
    ///     .painter(&ThinSeparators)
    ///     .show_inside(ui, &mut Viewer);
    /// # });
    /// ```
    ///
    /// A painter can draw what no style field gives, such as a border on the focused leaf
    /// alone, glowing:
    ///
    /// ```
    /// # use egui_stack::{DockArea, DockPainter, DockState, PartState, TabBodyPaint};
    /// /// A glow inside the focused leaf's body, and no border on the others.
    /// struct FocusGlow;
    ///
    /// impl DockPainter for FocusGlow {
    ///     fn tab_body(&self, painter: &egui::Painter, body: &TabBodyPaint<'_>) {
    ///         if !body.state.contains(PartState::FOCUSED) {
    ///             return;
    ///         }
    ///         let color = egui::Color32::from_rgb(0, 160, 255);
    ///         for (inset, alpha) in [(0.5, 1.0), (1.5, 0.4), (2.5, 0.15)] {
    ///             painter.rect_stroke(
    ///                 body.rect.shrink(inset),
    ///                 body.corner_radius,
    ///                 egui::Stroke::new(1.0, color.gamma_multiply(alpha)),
    ///                 egui::StrokeKind::Middle,
    ///             );
    ///         }
    ///     }
    /// }
    ///
    /// # egui::__run_test_ui(|ui| {
    /// # struct Viewer;
    /// # impl egui_stack::TabViewer for Viewer {
    /// #     type Tab = String;
    /// #     fn id(&mut self, tab: &mut String) -> egui::Id { egui::Id::new(tab.as_str()) }
    /// #     fn title(&mut self, tab: &mut String) -> egui::WidgetText { tab.as_str().into() }
    /// #     fn ui(&mut self, ui: &mut egui::Ui, tab: &mut String) { ui.label(tab.as_str()); }
    /// # }
    /// let mut dock_state = DockState::new(vec!["Tab".to_owned()]);
    /// DockArea::new(&mut dock_state)
    ///     .painter(&FocusGlow)
    ///     .show_inside(ui, &mut Viewer);
    /// # });
    /// ```
    #[inline(always)]
    pub fn painter(mut self, painter: &'tree (dyn DockPainter + Sync)) -> Self {
        self.painter = AssertUnwindSafe(painter);
        self
    }

    /// Sets the accessible names of the buttons and separators, read by screen readers through
    /// AccessKit. By default they are [`Labels::ENGLISH`].
    #[inline(always)]
    pub fn labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }

    /// Sets how the keyboard focus reaches the tabs. By default it's [`TabFocus::Each`].
    #[inline(always)]
    pub fn tab_focus(mut self, tab_focus: TabFocus) -> Self {
        self.tab_focus = tab_focus;
        self
    }

    /// Shows or hides the add button popup.
    /// By default it's `false`.
    pub fn show_add_popup(mut self, show_add_popup: bool) -> Self {
        self.show_add_popup = show_add_popup;
        self
    }

    /// Shows or hides the tab add buttons.
    /// By default it's `false`.
    pub fn show_add_buttons(mut self, show_add_buttons: bool) -> Self {
        self.show_add_buttons = show_add_buttons;
        self
    }

    /// Shows or hides the tab close buttons.
    /// By default it's `true`.
    pub fn show_close_buttons(mut self, show_close_buttons: bool) -> Self {
        self.show_close_buttons = show_close_buttons;
        self
    }

    /// Whether tabs show a context menu when right-clicked.
    /// By default it's `true`.
    pub fn tab_context_menus(mut self, tab_context_menus: bool) -> Self {
        self.tab_context_menus = tab_context_menus;
        self
    }

    /// Shows or hides the built-in items of the tab context menus: Eject, Close, and Hide or Show
    /// tab bar. Hidden, a tab's menu holds only what [`TabViewer::context_menu`] adds, for
    /// applications that offer those actions as their own commands.
    /// By default it's `true`.
    ///
    /// [`TabViewer::context_menu`]: crate::TabViewer::context_menu
    #[inline(always)]
    pub fn show_builtin_tab_menu(mut self, show_builtin_tab_menu: bool) -> Self {
        self.show_builtin_tab_menu = show_builtin_tab_menu;
        self
    }

    /// Whether tabs can be dragged between nodes and reordered on the tab bar.
    /// By default it's `true`.
    pub fn draggable_tabs(mut self, draggable_tabs: bool) -> Self {
        self.draggable_tabs = draggable_tabs;
        self
    }

    /// Whether tab bars can be hidden with the context menu.
    /// By default it's `false`.
    pub fn hidable_tab_bars(mut self, hidable_tab_bars: bool) -> Self {
        self.hidable_tab_bars = hidable_tab_bars;
        self
    }

    /// Whether tabs show their name when hovered over them.
    /// By default it's `false`.
    pub fn show_tab_name_on_hover(mut self, show_tab_name_on_hover: bool) -> Self {
        self.show_tab_name_on_hover = show_tab_name_on_hover;
        self
    }

    /// What directions can a node be split in: left-right, top-bottom, all, or none.
    /// By default it's all.
    pub fn allowed_splits(mut self, allowed_splits: AllowedSplits) -> Self {
        self.allowed_splits = allowed_splits;
        self
    }

    /// Whether tooltip hints are shown for secondary buttons on tab bars.
    /// By default it's `true`.
    pub fn show_secondary_button_hint(mut self, show_secondary_button_hint: bool) -> Self {
        self.show_secondary_button_hint = show_secondary_button_hint;
        self
    }

    /// The key combination used to activate secondary buttons on tab bars.
    /// By default it's [`Modifiers::SHIFT`].
    pub fn secondary_button_modifiers(mut self, secondary_button_modifiers: Modifiers) -> Self {
        self.secondary_button_modifiers = secondary_button_modifiers;
        self
    }

    /// Whether the secondary buttons on tab bars are activated by the modifier key.
    /// By default it's `true`.
    pub fn secondary_button_on_modifier(mut self, secondary_button_on_modifier: bool) -> Self {
        self.secondary_button_on_modifier = secondary_button_on_modifier;
        self
    }

    /// Whether the secondary buttons on tab bars are activated from a context value by right-clicking primary buttons.
    /// By default it's `true`.
    pub fn secondary_button_context_menu(mut self, secondary_button_context_menu: bool) -> Self {
        self.secondary_button_context_menu = secondary_button_context_menu;
        self
    }

    /// The bounds for any windows inside the [`DockArea`]. Defaults to the screen rect.
    /// By default it's set to [`egui::Context::content_rect`].
    #[inline(always)]
    pub fn window_bounds(mut self, bounds: Rect) -> Self {
        self.window_bounds = Some(bounds);
        self
    }

    /// The least size of a leaf's body for its tabs, kept by floating windows.
    pub fn leaf_min_size(mut self, min: &'tree dyn Fn(&[Tab]) -> Vec2) -> Self {
        self.leaf_min_size = Some(min);
        self
    }

    /// Enables or disables the close button on windows.
    /// By default it's `true`.
    #[inline(always)]
    #[deprecated = "consider using `show_leaf_close_buttons` instead."]
    pub fn show_window_close_buttons(mut self, show_window_close_buttons: bool) -> Self {
        self.show_window_close_buttons = show_window_close_buttons;
        self
    }

    /// Enables or disables the collapsing header on windows.
    /// By default it's `true`.
    #[inline(always)]
    #[deprecated = "consider using `show_leaf_collapse_buttons` instead."]
    pub fn show_window_collapse_buttons(mut self, show_window_collapse_buttons: bool) -> Self {
        self.show_window_collapse_buttons = show_window_collapse_buttons;
        self
    }

    /// Enables or disables the close all tabs button on tab bars.
    /// By default it's `true`.
    #[inline(always)]
    pub fn show_leaf_close_all_buttons(mut self, show_leaf_close_all_buttons: bool) -> Self {
        self.show_leaf_close_all_buttons = show_leaf_close_all_buttons;
        self
    }

    /// Enables or disables the collapse tabs button on tab bars.
    /// By default it's `true`.
    #[inline(always)]
    pub fn show_leaf_collapse_buttons(mut self, show_leaf_collapse_buttons: bool) -> Self {
        self.show_leaf_collapse_buttons = show_leaf_collapse_buttons;
        self
    }
}

impl<Tab> std::fmt::Debug for DockArea<'_, Tab> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockArea").finish_non_exhaustive()
    }
}
