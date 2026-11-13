use egui::{Id, Ui, WidgetText};

use crate::{NodePath, TabDestination, TabPath, TabStyle};

/// Defines how a tab should behave and be rendered inside a [`Tree`](crate::Tree).
pub trait TabViewer {
    /// The type of tab in which you can store state to be drawn in your tabs.
    type Tab;

    /// Unique ID for this tab.
    ///
    /// Assuming `Tab` implements `Hash`, you can simply do `Id::new(tab)`.
    /// If that is not possible, but tab names are guaranteed to be unique,
    /// one could also implement this as `Id::new(self.title(tab).text())`.
    fn id(&mut self, tab: &mut Self::Tab) -> Id;

    /// The title to be displayed in the tab bar.
    fn title(&mut self, tab: &mut Self::Tab) -> WidgetText;

    /// Actual tab content.
    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab);

    /// Content inside the context menu shown when the tab is right-clicked.
    ///
    /// `_path` is where the tab is: its [`Surface`](crate::Surface), [`Node`](crate::Node) and
    /// index, so a menu item can name it to the tab actions of [`DockState`](crate::DockState):
    /// [`close_tab`](crate::DockState::close_tab), [`eject_tab`](crate::DockState::eject_tab),
    /// [`focus_tab`](crate::DockState::focus_tab) and [`move_tab`](crate::DockState::move_tab).
    fn context_menu(&mut self, _ui: &mut Ui, _tab: &mut Self::Tab, _path: TabPath) {}

    /// Called after each tab button is shown, so you can add a tooltip, check for clicks, etc.
    fn on_tab_button(&mut self, _tab: &mut Self::Tab, _response: &egui::Response) {}

    /// This is called when the `_tab` gets closed by the user.
    ///
    /// Returns an `OnCloseResponse` which determines what happens to the tab after this function gets called.
    fn on_close(&mut self, _tab: &mut Self::Tab) -> OnCloseResponse {
        OnCloseResponse::Close
    }

    /// Returns `true` if the user of your app should be able to close a given `_tab`.
    ///
    /// By default, `true` is always returned.
    fn is_closeable(&self, _tab: &Self::Tab) -> bool {
        true
    }

    /// Tabs of a bar change places while one is dragged along it only within one group.
    fn tab_group(&self, _tab: &Self::Tab) -> u8 {
        0
    }

    /// Returns `false` when [`TabViewer::context_menu`] already offers closing `_tab`.
    fn close_in_tab_menu(&self, _tab: &Self::Tab) -> bool {
        true
    }

    /// Returns `true` if the user of your app should be able to close a given `_tab`.
    ///
    /// By default, `true` is always returned.
    #[deprecated = "Use the `TabViewer::is_closeable` function instead."]
    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }

    /// This is called every frame after [`ui`](Self::ui) is called, if the `_tab` is active.
    ///
    /// Returns `true` if the tab should be forced to close, `false` otherwise.
    ///
    /// In the event this function returns true the tab will be removed without calling `on_close`.
    fn force_close(&mut self, _tab: &mut Self::Tab) -> bool {
        false
    }

    /// This is called when the add button is pressed.
    ///
    /// `_path` specifies which [`Surface`](crate::Surface) and on which
    /// [`Node`](crate::Node) this particular add button was pressed.
    fn on_add(&mut self, _path: NodePath) {}

    /// Called when the rectangle of the tab content changes.
    ///
    /// This can happen when the window is resized, panels are docked or undocked,
    /// or when the layout of the dock area is changed in any way that affects
    /// the available space for the tab content.
    ///
    /// This is useful for tabs that need to adjust their content based on the
    /// available space.
    fn on_rect_changed(&mut self, _tab: &mut Self::Tab) {}

    /// Content of the popup under the add button. Useful for selecting what type of tab to add.
    ///
    /// This requires that [`DockArea::show_add_buttons`](crate::DockArea::show_add_buttons) and
    /// [`DockArea::show_add_popup`](crate::DockArea::show_add_popup) are set to `true`.
    fn add_popup(&mut self, _ui: &mut Ui, _path: NodePath) {}

    /// Sets custom style for given tab.
    fn tab_style_override(&self, _tab: &Self::Tab, _global_style: &TabStyle) -> Option<TabStyle> {
        None
    }

    /// Specifies a tab's ability to be shown in a window.
    ///
    /// Returns `false` if this tab should never be turned into a window.
    fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool {
        true
    }

    /// Whether `tab` may be dropped at `destination`.
    ///
    /// Asked every frame while `tab` is dragged over a destination the dock would otherwise accept
    /// (windows refused by [`allowed_in_windows`](Self::allowed_in_windows) are never offered).
    /// A [`DropVerdict::Refuse`] is shown on the drop overlay, with its reason if it has one, and
    /// releasing the tab there leaves the layout unchanged.
    ///
    /// By default every destination is allowed.
    fn drop_allowed(&self, _tab: &Self::Tab, _destination: &TabDestination) -> DropVerdict {
        DropVerdict::Allow
    }

    /// Whether `tab` may be dropped at `drop.destination`, knowing the docks of a
    /// [`DockSpace`](crate::DockSpace) it would leave and enter.
    ///
    /// Asked as [`drop_allowed`](Self::drop_allowed) is, by every [`DockArea`](crate::DockArea),
    /// inside one dock and between the docks of a space alike; outside a space, both docks are
    /// `None`. By default it returns what [`drop_allowed`](Self::drop_allowed) does, so a viewer
    /// overrides one or the other.
    fn dock_drop_allowed(&self, tab: &Self::Tab, drop: &DockDrop<'_>) -> DropVerdict {
        self.drop_allowed(tab, drop.destination)
    }

    /// Whether the tab body will be cleared with the color specified in
    /// [`TabBarStyle::bg_fill`](crate::TabBarStyle::bg_fill).
    fn clear_background(&self, _tab: &Self::Tab) -> bool {
        true
    }

    /// Returns `true` if the horizontal and vertical scroll bars will be shown for `tab`.
    ///
    /// By default, both scroll bars are shown.
    fn scroll_bars(&self, _tab: &Self::Tab) -> [bool; 2] {
        [true, true]
    }
}

/// Whether a dragged tab may be dropped at a destination, from [`TabViewer::drop_allowed`].
#[derive(Clone, Debug, Default)]
pub enum DropVerdict {
    /// The tab may be dropped there.
    #[default]
    Allow,
    /// The tab may not be dropped there. The reason, if any, is shown next to the pointer.
    Refuse(Option<WidgetText>),
}

/// Where a dragged tab would be dropped, for [`TabViewer::dock_drop_allowed`].
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct DockDrop<'a> {
    /// Where the tab would go.
    pub destination: &'a TabDestination,
    /// The name of the dock the tab leaves, `None` for a floating window or outside a
    /// [`DockSpace`](crate::DockSpace).
    pub from: Option<&'a str>,
    /// The name of the dock the tab enters, `None` for a floating window (a new one included) or
    /// outside a [`DockSpace`](crate::DockSpace).
    pub to: Option<&'a str>,
}

/// Determines what happens to a tab when a user attempts to close it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OnCloseResponse {
    /// Closes the tab.
    Close,
    /// Focuses on the tab.
    Focus,
    /// Ignores the close request.
    Ignore,
}
