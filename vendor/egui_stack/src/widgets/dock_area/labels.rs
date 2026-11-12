use std::borrow::Cow;

/// Accessible names of the [`DockArea`](crate::DockArea)'s buttons and separators, of the
/// boundaries of a [`Sections`](crate::sections::Sections) stack, of the More button of a
/// [`ToolRow`](crate::toolbar::ToolRow), and of the sashes and strip grips of a
/// [`LayoutArea`](crate::layout::LayoutArea).
///
/// Screen readers and test harnesses (through AccessKit) find these controls by their names. The
/// defaults are English; set your own with [`DockArea::labels`](crate::DockArea::labels) to
/// translate them:
///
/// ```rust
/// # use egui_stack::Labels;
/// let mut labels = Labels::default();
/// labels.close_tab = "Fermer l'onglet".into();
/// labels.add_tab = "Ajouter un onglet".into();
/// ```
///
/// Text shown on screen (menus and tooltips) is set by [`Translations`](crate::Translations).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(default)
)]
#[non_exhaustive]
pub struct Labels {
    /// The close button of a tab.
    pub close_tab: Cow<'static, str>,
    /// The button that adds a tab to a tab bar.
    pub add_tab: Cow<'static, str>,
    /// The button that closes every tab of a tab bar.
    pub close_all_tabs: Cow<'static, str>,
    /// The close-all button while it closes the whole window (with the secondary modifier).
    pub close_window: Cow<'static, str>,
    /// The button that collapses or expands a tab bar's body.
    pub collapse: Cow<'static, str>,
    /// The collapse button while it minimizes the whole window (with the secondary modifier).
    pub minimize_window: Cow<'static, str>,
    /// The button that restores a minimized window.
    pub restore_window: Cow<'static, str>,
    /// The button that shows a hidden tab bar.
    pub show_tab_bar: Cow<'static, str>,
    /// A separator between two nodes, which resizes them.
    pub separator: Cow<'static, str>,
    /// A boundary between two sections of a [`Sections`](crate::sections::Sections) stack,
    /// which resizes the section above it: `{section}` is replaced by that section's title.
    pub resize_section: Cow<'static, str>,
    /// The More button of a [`ToolRow`](crate::toolbar::ToolRow), which opens the tools that
    /// do not fit, when the row has no title.
    pub more_tools: Cow<'static, str>,
    /// The same button in a row with a title ([`ToolRow::title`](crate::toolbar::ToolRow::title)):
    /// `{toolbar}` is replaced by it, so two rows never have two controls of one name.
    pub more_tools_in: Cow<'static, str>,
    /// A sash of a [`LayoutArea`](crate::layout::LayoutArea), which resizes a zone: `{zone}`
    /// is replaced by the zone's title ([`ZoneViewer::title`](crate::layout::ZoneViewer::title)).
    pub resize_zone: Cow<'static, str>,
    /// A folded zone's grip, which unfolds it: `{zone}` is replaced by the zone's title.
    pub unfold_zone: Cow<'static, str>,
    /// The grip of a zone lying over its neighbour, which folds it back: `{zone}` is replaced
    /// by the zone's title.
    pub fold_zone: Cow<'static, str>,
    /// A flyout's free edge, which resizes it: `{flyout}` is replaced by the flyout's title
    /// ([`ZoneViewer::flyout_title`](crate::layout::ZoneViewer::flyout_title)).
    pub resize_flyout: Cow<'static, str>,
}

impl Labels {
    /// The default English labels.
    pub const ENGLISH: Self = Self {
        close_tab: Cow::Borrowed("Close tab"),
        add_tab: Cow::Borrowed("Add tab"),
        close_all_tabs: Cow::Borrowed("Close all tabs"),
        close_window: Cow::Borrowed("Close window"),
        collapse: Cow::Borrowed("Collapse"),
        minimize_window: Cow::Borrowed("Minimize window"),
        restore_window: Cow::Borrowed("Restore window"),
        show_tab_bar: Cow::Borrowed("Show tab bar"),
        separator: Cow::Borrowed("Resize"),
        resize_section: Cow::Borrowed("Resize {section}"),
        more_tools: Cow::Borrowed("More tools"),
        more_tools_in: Cow::Borrowed("More tools: {toolbar}"),
        resize_zone: Cow::Borrowed("Resize {zone}"),
        unfold_zone: Cow::Borrowed("Unfold {zone}"),
        fold_zone: Cow::Borrowed("Fold {zone}"),
        resize_flyout: Cow::Borrowed("Resize {flyout}"),
    };
}

impl Default for Labels {
    fn default() -> Self {
        Self::ENGLISH
    }
}
