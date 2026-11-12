//! A [`Layout`] and the [`DockSpace`] whose docks it places, kept together: see [`Docked`].

use crate::DockSpace;
use crate::layout::{Content, Layout, Zone, ZoneKind, ZoneName};

/// A layout and the dock space whose docks its zones hold, as one value: with the `serde`
/// feature, saved and loaded together.
///
/// The two are linked by name only (a leaf zone with [`Content::Dock`] shows the dock of that
/// name), so a pair saved by an older version, or edited by hand, may disagree: a zone naming a
/// dock the space lost, a dock no zone shows. [`Docked::repair`] makes them agree.
///
/// ```
/// use egui_stack::layout::{Content, Dir, Layout, Zone, ZoneName};
/// use egui_stack::{DockRules, DockSpace, Docked};
///
/// let mut space = DockSpace::new();
/// space.add_dock("centre", DockRules::default(), vec!["Editor".to_owned()]);
/// space.add_dock("log", DockRules::default(), vec!["Log".to_owned()]);
/// let layout = Layout::new(Zone::split(
///     "root",
///     Dir::Horizontal,
///     vec![
///         Zone::leaf("left", Content::Dock("files".into())),
///         Zone::leaf("centre", Content::Dock("centre".into())).with(|c| c.grow = true),
///     ],
/// ));
/// let mut docked = Docked::new(layout, space);
/// let repair = docked.repair();
/// // The space has no "files" dock: its zone goes. No zone shows "log": it gets a hidden one.
/// assert_eq!(repair.removed, [ZoneName::from("left")]);
/// assert_eq!(repair.added, [ZoneName::from("log")]);
/// assert!(docked.layout.zone("log").unwrap().state.hidden);
/// ```
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Docked<Tab> {
    /// Where the docks go.
    pub layout: Layout,
    /// The docks, their tabs and the floating windows.
    pub space: DockSpace<Tab>,
}

/// What [`Docked::repair`] changed in the layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DockedRepair {
    /// The zones dropped: they showed a dock the space has not.
    pub removed: Vec<ZoneName>,
    /// The hidden zones added, one per dock no zone showed, named after it.
    pub added: Vec<ZoneName>,
}

impl DockedRepair {
    /// Whether the layout and the space already agreed.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty()
    }
}

impl<Tab> Docked<Tab> {
    /// A pair of `layout` and `space`, as they are; see [`repair`](Self::repair).
    pub fn new(layout: Layout, space: DockSpace<Tab>) -> Self {
        Self { layout, space }
    }

    /// Makes the layout agree with the space: the leaf zones showing a dock the space has not
    /// are dropped (the root, if it is one, is emptied: [`Content::Empty`]), and each dock no
    /// zone shows gets a hidden leaf zone named after it, last under the root, so the
    /// application can show it again ([`ZoneState::hidden`](crate::layout::ZoneState::hidden)).
    /// A dock whose name a zone already has, or a root that is a leaf, gets no zone; the dock
    /// stays in the space.
    pub fn repair(&mut self) -> DockedRepair {
        let mut repair = DockedRepair::default();
        let space = &self.space;
        drop_unknown_docks(&mut self.layout.root, space, &mut repair.removed);
        if let ZoneKind::Leaf { content } = &mut self.layout.root.kind
            && let Content::Dock(name) = content
            && !space.has_dock(name)
        {
            *content = Content::Empty;
        }
        repair.added = add_unplaced_docks(&mut self.layout, space, true);
        repair
    }
}

impl Layout {
    /// A layout for a [`DockSpace`] an application placed with egui panels, so it moves onto
    /// the layout: `shape` is the tree the application wants (which dock is on the left, which
    /// one grows), its leaves holding [`Content::Dock`]. Each dock of the space that `shape`
    /// does not show gets a leaf of its own, named after it, last under the root, if the root
    /// is a split and has no zone of that name.
    ///
    /// ```
    /// use egui_stack::layout::{Content, Dir, Layout, Zone};
    /// use egui_stack::{DockRules, DockSpace};
    ///
    /// let mut space = DockSpace::<String>::new();
    /// space.add_dock("left", DockRules::default(), vec![]);
    /// space.add_dock("centre", DockRules::default(), vec![]);
    /// space.add_dock("bottom", DockRules::default(), vec![]);
    /// let shape = Zone::split(
    ///     "root",
    ///     Dir::Horizontal,
    ///     vec![
    ///         Zone::leaf("left", Content::Dock("left".into())),
    ///         Zone::leaf("centre", Content::Dock("centre".into())).with(|c| c.grow = true),
    ///     ],
    /// );
    /// let layout = Layout::from_space(&space, shape);
    /// let names: Vec<String> = layout.names().into_iter().map(|name| name.0).collect();
    /// assert_eq!(names, ["root", "left", "centre", "bottom"]);
    /// ```
    pub fn from_space<Tab>(space: &DockSpace<Tab>, shape: Zone) -> Self {
        let mut layout = Self::new(shape);
        add_unplaced_docks(&mut layout, space, false);
        layout
    }
}

/// The docks the leaves under `zone` show.
fn shown_docks(zone: &Zone) -> Vec<String> {
    let mut docks = Vec::new();
    zone.visit(&mut |zone| {
        if let ZoneKind::Leaf {
            content: Content::Dock(name),
        } = &zone.kind
        {
            docks.push(name.clone());
        }
    });
    docks
}

/// Drops from `zone`'s children, at any depth, the leaves showing a dock `space` has not.
fn drop_unknown_docks<Tab>(zone: &mut Zone, space: &DockSpace<Tab>, removed: &mut Vec<ZoneName>) {
    if let ZoneKind::Split { children, .. } = &mut zone.kind {
        children.retain(|child| match &child.kind {
            ZoneKind::Leaf {
                content: Content::Dock(name),
            } if !space.has_dock(name) => {
                removed.push(child.name.clone());
                false
            }
            _ => true,
        });
        for child in children {
            drop_unknown_docks(child, space, removed);
        }
    }
}

/// Adds a leaf, last under the root, for each dock of `space` no leaf shows; returns their
/// names.
fn add_unplaced_docks<Tab>(
    layout: &mut Layout,
    space: &DockSpace<Tab>,
    hidden: bool,
) -> Vec<ZoneName> {
    let shown = shown_docks(&layout.root);
    let names = layout.names();
    let mut added = Vec::new();
    let ZoneKind::Split { children, .. } = &mut layout.root.kind else {
        return added;
    };
    for dock in space.dock_names() {
        let name = ZoneName::from(dock);
        if shown.iter().any(|shown| shown == dock) || names.contains(&name) {
            continue;
        }
        children.push(
            Zone::leaf(name.clone(), Content::Dock(dock.to_owned()))
                .with_state(|state| state.hidden = hidden),
        );
        added.push(name);
    }
    added
}
